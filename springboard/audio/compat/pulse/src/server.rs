//! The socket side: one non-blocking poll loop over the listening socket and every client connection. Each client
//! is a [`Session`]; the loop feeds it frames, ticks it every few milliseconds while it has streams, and writes the
//! frames it returns. It runs on its own thread in the compat process, never on a real-time thread.

use crate::backend::Backend;
use crate::proto::FrameBuf;
use crate::session::Session;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The poll interval while any stream is active (flow control and capture delivery granularity).
pub const TICK_MS: i32 = 5;
/// A client that lets this much output pile up is disconnected.
const MAX_PENDING: usize = 32 << 20;

/// Logs under subsystem `is.olibuijr.audio`, category `client` (stderr until the logger is reachable from here).
macro_rules! log_client {
    ($($arg:tt)*) => { eprintln!("is.olibuijr.audio/client: {}", format!($($arg)*)) };
}

/// `$XDG_RUNTIME_DIR/pulse/native`.
pub fn default_socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe_uid())));
    dir.join("pulse").join("native")
}

fn unsafe_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Binds the socket, creating its directory (mode 0700) and replacing a stale socket file.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(ErrorKind::AddrInUse, "another pulse server is running"));
        }
        std::fs::remove_file(path)?;
    }
    UnixListener::bind(path)
}

/// The listening socket systemd passed (socket activation: fd 3 when `LISTEN_PID` is this process).
pub fn inherited_listener() -> Option<UnixListener> {
    let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    let fds: u32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    if pid != std::process::id() || fds < 1 {
        return None;
    }
    // SAFETY: systemd passes an open listening socket as fd 3 (SD_LISTEN_FDS_START) and gives this process
    // ownership of it; nothing else in the process uses that descriptor.
    Some(unsafe { UnixListener::from_raw_fd(3) })
}

struct Conn {
    stream: UnixStream,
    frames: FrameBuf,
    wbuf: Vec<u8>,
    session: Session,
    dead: bool,
}

pub struct Server {
    listener: UnixListener,
    backend: Box<dyn Backend>,
    conns: Vec<Conn>,
    next_client: u32,
}

impl Server {
    pub fn new(listener: UnixListener, backend: Box<dyn Backend>) -> std::io::Result<Server> {
        listener.set_nonblocking(true)?;
        Ok(Server { listener, backend, conns: Vec::new(), next_client: 1 })
    }

    pub fn clients(&self) -> usize {
        self.conns.len()
    }

    /// One loop iteration: waits up to `timeout_ms` (-1 forever) for activity, then services everything.
    pub fn step(&mut self, timeout_ms: i32) -> std::io::Result<()> {
        let active = self.conns.iter().any(|c| c.session.has_streams());
        let timeout = if active { if timeout_ms < 0 { TICK_MS } else { timeout_ms.min(TICK_MS) } } else { timeout_ms };
        let mut fds: Vec<libc::pollfd> = Vec::with_capacity(self.conns.len() + 1);
        fds.push(libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        for c in &self.conns {
            let mut events = libc::POLLIN;
            if !c.wbuf.is_empty() {
                events |= libc::POLLOUT;
            }
            fds.push(libc::pollfd { fd: c.stream.as_raw_fd() as RawFd, events, revents: 0 });
        }
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd structs for the duration of the call.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(e);
            }
        }
        if fds[0].revents & libc::POLLIN != 0 {
            self.accept();
        }
        for (i, pfd) in fds.iter().enumerate().skip(1) {
            if pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                if let Some(c) = self.conns.get_mut(i - 1) {
                    Self::read_conn(c, self.backend.as_mut());
                }
            }
        }
        for c in &mut self.conns {
            if c.session.has_streams() && !c.dead {
                let out = c.session.tick();
                for f in out {
                    c.wbuf.extend_from_slice(&f);
                }
            }
            Self::flush(c);
        }
        let before = self.conns.len();
        self.conns.retain(|c| !(c.dead || (c.session.closed && c.wbuf.is_empty())));
        if self.conns.len() != before {
            log_client!("{} client(s) connected", self.conns.len());
        }
        Ok(())
    }

    fn accept(&mut self) {
        while let Ok((stream, _)) = self.listener.accept() {
            if stream.set_nonblocking(true).is_err() {
                continue;
            }
            let index = self.next_client;
            self.next_client += 1;
            self.conns.push(Conn { stream, frames: FrameBuf::default(), wbuf: Vec::new(), session: Session::new(index), dead: false });
            log_client!("client {} connected", index);
        }
    }

    fn read_conn(c: &mut Conn, backend: &mut dyn Backend) {
        let mut buf = [0u8; 65536];
        loop {
            match c.stream.read(&mut buf) {
                Ok(0) => {
                    c.dead = true;
                    return;
                }
                Ok(n) => c.frames.push(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    c.dead = true;
                    return;
                }
            }
        }
        loop {
            match c.frames.next() {
                Ok(Some((d, payload))) => {
                    for f in c.session.handle(backend, &d, &payload) {
                        c.wbuf.extend_from_slice(&f);
                    }
                }
                Ok(None) => break,
                Err(()) => {
                    log_client!("oversized frame; dropping the client");
                    c.dead = true;
                    return;
                }
            }
        }
    }

    fn flush(c: &mut Conn) {
        while !c.wbuf.is_empty() {
            match c.stream.write(&c.wbuf) {
                Ok(0) => {
                    c.dead = true;
                    return;
                }
                Ok(n) => {
                    c.wbuf.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    c.dead = true;
                    return;
                }
            }
        }
        if c.wbuf.len() > MAX_PENDING {
            c.dead = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;
    use crate::proto::{cmd, command_frame, Descriptor, DESCRIPTOR_LEN};
    use crate::tagstruct::{Reader, Writer};

    fn socket_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bifrost-pulse-test-{}-{}", std::process::id(), tag)).join("pulse").join("native")
    }

    fn read_frame(s: &mut UnixStream) -> Vec<u8> {
        let mut h = [0u8; DESCRIPTOR_LEN];
        s.read_exact(&mut h).unwrap();
        let d = Descriptor::parse(&h);
        let mut p = vec![0u8; d.length as usize];
        s.read_exact(&mut p).unwrap();
        p
    }

    #[test]
    fn auth_and_server_info_over_a_real_socket() {
        let path = socket_path("auth");
        let listener = bind(&path).unwrap();
        let mut server = Server::new(listener, Box::new(MockBackend::standard())).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        client.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let mut w = Writer::new();
        w.u32(cmd::AUTH).u32(1).u32(35).arbitrary(&[0; 256]);
        client.write_all(&command_frame(&w.buf)).unwrap();
        for _ in 0..3 {
            server.step(20).unwrap();
        }
        assert_eq!(server.clients(), 1);
        let reply = read_frame(&mut client);
        let mut rd = Reader::new(&reply);
        assert_eq!((rd.u32(), rd.u32(), rd.u32()), (Ok(cmd::REPLY), Ok(1), Ok(35)));
        // a frame split across two writes still works
        let mut w = Writer::new();
        w.u32(cmd::STAT).u32(2);
        let f = command_frame(&w.buf);
        client.write_all(&f[..7]).unwrap();
        server.step(20).unwrap();
        client.write_all(&f[7..]).unwrap();
        for _ in 0..3 {
            server.step(20).unwrap();
        }
        let reply = read_frame(&mut client);
        assert_eq!(Reader::new(&reply).u32(), Ok(cmd::REPLY));
        // closing the socket removes the client
        drop(client);
        for _ in 0..3 {
            server.step(20).unwrap();
        }
        assert_eq!(server.clients(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_live_socket_is_not_taken_over_but_a_stale_one_is() {
        let path = socket_path("stale");
        let first = bind(&path).unwrap();
        assert_eq!(bind(&path).err().map(|e| e.kind()), Some(ErrorKind::AddrInUse));
        drop(first);
        // the file is still there but nobody listens
        assert!(path.exists());
        assert!(bind(&path).is_ok());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn an_oversized_frame_drops_only_that_client() {
        use crate::proto::MAX_FRAME;
        let path = socket_path("oversized");
        let mut server = Server::new(bind(&path).unwrap(), Box::new(MockBackend::standard())).unwrap();
        let mut good = UnixStream::connect(&path).unwrap();
        let mut bad = UnixStream::connect(&path).unwrap();
        good.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        server.step(20).unwrap();
        assert_eq!(server.clients(), 2);
        let d = Descriptor { length: MAX_FRAME as u32 + 1, channel: 0, offset: 0, flags: 0 };
        bad.write_all(&d.encode()).unwrap();
        for _ in 0..3 {
            server.step(20).unwrap();
        }
        assert_eq!(server.clients(), 1);
        let mut w = Writer::new();
        w.u32(cmd::AUTH).u32(1).u32(35).arbitrary(&[0; 256]);
        good.write_all(&command_frame(&w.buf)).unwrap();
        for _ in 0..3 {
            server.step(20).unwrap();
        }
        assert_eq!(Reader::new(&read_frame(&mut good)).u32(), Ok(cmd::REPLY));
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn an_idle_step_returns_after_its_timeout() {
        let path = socket_path("idle");
        let mut server = Server::new(bind(&path).unwrap(), Box::new(MockBackend::standard())).unwrap();
        let t0 = std::time::Instant::now();
        server.step(30).unwrap();
        let dt = t0.elapsed();
        assert!(dt >= std::time::Duration::from_millis(25) && dt < std::time::Duration::from_secs(2), "{dt:?}");
        assert_eq!(server.clients(), 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn the_socket_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_path("mode");
        let _l = bind(&path).unwrap();
        let mode = std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
