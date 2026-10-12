//! The control-socket connection: SEQPACKET framing with SCM_RIGHTS, a reader thread that splits replies from
//! notifications, and synchronous requests. Not real time.

use bifrost_audio_types::control::{decode_reply, encode_request, Reply, Request, MAX_LEN};
use bifrost_audio_types::{AudioError, ObjectId, PropertyAddress};
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a request waits for its reply.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// The most fds one packet may carry.
const MAX_FDS: usize = 4;

pub(crate) type ListenerFn = dyn Fn(ObjectId, PropertyAddress) + Send + Sync;
pub(crate) type Msg = (Reply, Vec<OwnedFd>);

/// State the reader thread shares with the callers.
pub(crate) struct Shared {
    pub listeners: Mutex<HashMap<(ObjectId, PropertyAddress), Vec<(u64, Arc<ListenerFn>)>>>,
    pub overloads: Mutex<HashMap<u32, u32>>,
    pub dead: AtomicBool,
}

pub(crate) struct Conn {
    pub sock: Arc<OwnedFd>,
    rx: Mutex<Receiver<Msg>>,
    pub shared: Arc<Shared>,
}

/// Sends one packet with optional fds.
pub(crate) fn send_packet(fd: RawFd, data: &[u8], fds: &[RawFd]) -> io::Result<()> {
    if fds.len() > MAX_FDS {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    // SAFETY: CMSG_SPACE is a pure size computation.
    let space = unsafe { libc::CMSG_SPACE((fds.len() * 4) as u32) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8).max(1)];
    // SAFETY: msghdr is plain data; all-zero is a valid empty value.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = space as _;
        // SAFETY: msg_control points at `space` zeroed, 8-aligned bytes, enough for one SCM_RIGHTS header and
        // `fds.len()` ints, so the first header and its data area are in bounds.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN((fds.len() * 4) as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, libc::CMSG_DATA(c), fds.len() * 4);
        }
    }
    loop {
        // SAFETY: `msg` and the buffers it points at outlive the call.
        let n = unsafe { libc::sendmsg(fd, &msg, libc::MSG_NOSIGNAL) };
        if n >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Receives one packet and the fds that came with it. Ok(None) at end of stream.
pub(crate) fn recv_packet(fd: RawFd, buf: &mut [u8]) -> io::Result<Option<(usize, Vec<OwnedFd>)>> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    // SAFETY: pure size computation.
    let space = unsafe { libc::CMSG_SPACE((MAX_FDS * 4) as u32) } as usize;
    let mut cbuf = vec![0u64; space.div_ceil(8)];
    // SAFETY: all-zero msghdr is valid.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = space as _;
    let n = loop {
        // SAFETY: `msg` and its buffers outlive the call.
        let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut fds = Vec::new();
    // SAFETY: the kernel filled the control buffer; the CMSG macros walk it within `msg_controllen`, and every
    // fd read here is owned by this process from now on (wrapped at once so it closes on any early return).
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let len = ((*c).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                for i in 0..len / 4 {
                    let raw = std::ptr::read_unaligned(libc::CMSG_DATA(c).add(i * 4) as *const i32);
                    fds.push(OwnedFd::from_raw_fd(raw));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    if n == 0 && fds.is_empty() {
        return Ok(None);
    }
    Ok(Some((n, fds)))
}

impl Conn {
    /// Wraps a connected SEQPACKET socket and starts the reader (notification) thread.
    pub fn new(sock: OwnedFd) -> Arc<Conn> {
        let sock = Arc::new(sock);
        let shared = Arc::new(Shared { listeners: Mutex::new(HashMap::new()), overloads: Mutex::new(HashMap::new()), dead: AtomicBool::new(false) });
        let (tx, rx) = channel::<Msg>();
        let (s2, sh2) = (sock.clone(), shared.clone());
        std::thread::Builder::new()
            .name("audio-notify".into())
            .spawn(move || {
                let mut buf = vec![0u8; MAX_LEN];
                loop {
                    let (n, fds) = match recv_packet(s2.as_raw_fd(), &mut buf) {
                        Ok(Some(x)) => x,
                        Ok(None) | Err(_) => break,
                    };
                    let reply = match decode_reply(&buf[..n]) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[is.olibuijr.audio/hal] undecodable reply: {e:?}");
                            continue;
                        }
                    };
                    match reply {
                        Reply::Changed(list) => {
                            for key in list {
                                let fs: Vec<Arc<ListenerFn>> = sh2.listeners.lock().unwrap().get(&key).map(|v| v.iter().map(|(_, f)| f.clone()).collect()).unwrap_or_default();
                                for f in fs {
                                    f(key.0, key.1);
                                }
                            }
                        }
                        Reply::Overload { id, count } => {
                            *sh2.overloads.lock().unwrap().entry(id).or_insert(0) += count;
                        }
                        other => {
                            let _ = tx.send((other, fds));
                        }
                    }
                }
                sh2.dead.store(true, Ordering::Release);
                // dropping `tx` wakes a waiting request with a disconnect
            })
            .expect("spawn audio-notify");
        Arc::new(Conn { sock, rx: Mutex::new(rx), shared })
    }

    /// One request, one reply (and the fds that came with it). `Reply::Error` becomes `Err`.
    pub fn request(&self, req: &Request, fds: &[RawFd]) -> Result<Msg, AudioError> {
        let rx = self.rx.lock().unwrap();
        if self.shared.dead.load(Ordering::Acquire) {
            return Err(AudioError::NoServer);
        }
        while rx.try_recv().is_ok() {}
        send_packet(self.sock.as_raw_fd(), &encode_request(req), fds).map_err(|_| AudioError::NoServer)?;
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok((Reply::Error(e), _)) => Err(e),
            Ok(m) => Ok(m),
            Err(RecvTimeoutError::Timeout) => Err(AudioError::Io),
            Err(RecvTimeoutError::Disconnected) => Err(AudioError::NoServer),
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: shutdown on a valid socket fd ends the reader thread's recvmsg.
        unsafe { libc::shutdown(self.sock.as_raw_fd(), libc::SHUT_RDWR) };
    }
}

#[cfg(test)]
pub(crate) fn socket_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0i32; 2];
    // SAFETY: fds has room for two ints.
    let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) };
    assert_eq!(r, 0);
    // SAFETY: socketpair returned two fresh fds.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_keeps_its_bytes_and_boundaries() {
        let (a, b) = socket_pair();
        send_packet(a.as_raw_fd(), &[1, 2, 3], &[]).unwrap();
        send_packet(a.as_raw_fd(), &[4, 5], &[]).unwrap();
        let mut buf = [0u8; 64];
        let (n, fds) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((&buf[..n], fds.len()), (&[1u8, 2, 3][..], 0));
        let (n, _) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], &[4, 5]);
    }

    #[test]
    fn two_fds_arrive_with_the_packet_and_work() {
        let (a, b) = socket_pair();
        let (x, y) = socket_pair();
        send_packet(a.as_raw_fd(), &[9], &[x.as_raw_fd(), y.as_raw_fd()]).unwrap();
        let mut buf = [0u8; 8];
        let (n, fds) = recv_packet(b.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((n, fds.len()), (1, 2));
        // fds[0] is a dup of x; what it sends, y (the other end) reads.
        send_packet(fds[0].as_raw_fd(), &[7], &[]).unwrap();
        let (n, _) = recv_packet(y.as_raw_fd(), &mut buf).unwrap().unwrap();
        assert_eq!((n, buf[0]), (1, 7));
    }

    #[test]
    fn end_of_stream_is_none() {
        let (a, b) = socket_pair();
        drop(a);
        let mut buf = [0u8; 8];
        assert!(recv_packet(b.as_raw_fd(), &mut buf).unwrap().is_none());
    }

    #[test]
    fn more_than_four_fds_is_refused() {
        let (a, _b) = socket_pair();
        let fd = a.as_raw_fd();
        assert!(send_packet(fd, &[0], &[fd; 5]).is_err());
    }
}
