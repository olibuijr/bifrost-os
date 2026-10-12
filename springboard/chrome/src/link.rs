//! The socket between the shell and the chrome clients: `$XDG_RUNTIME_DIR/bifrost-chrome.sock`, served by the shell
//! (`HostLink`) and joined by each client (`Link`), which says `FromChrome::Hello` first and then gets the current
//! models.
//!
//! The transport is a `SOCK_DGRAM` Unix socket rather than `SOCK_SEQPACKET`: std has no seqpacket socket, and this
//! crate has no libc, so the safe std API is the only option. Datagrams keep one `model` message per packet, as
//! seqpacket does. Each client binds its own socket path next to the host's (`bifrost-chrome-<client>-<pid>.sock`),
//! connects it to the host, and the host replies to a client by the path the message came from. A client that
//! exits removes its path; the host forgets a client whose path is gone or refuses. A model that finds a client's
//! queue full is kept (the newest of each kind) and sent again on the next `poll`.
//!
//! Owner: wave G2 agent `chrome-core`.

use crate::model::{self, FromChrome, ToChrome};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};

/// The socket path.
pub fn socket_path() -> PathBuf {
    socket_dir().join("bifrost-chrome.sock")
}

fn socket_dir() -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from("/tmp"),
    }
}

/// A client's own socket path: the client name keeps only letters, digits and '-', so it stays one path component.
fn client_path(dir: &Path, client: &str, pid: u32) -> PathBuf {
    let name: String = client.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    dir.join(format!("bifrost-chrome-{name}-{pid}.sock"))
}

/// A client's end.
pub struct Link {
    socket: UnixDatagram,
    own_path: PathBuf,
    buf: Vec<u8>,
}

impl Link {
    /// Connects and sends `Hello { client }`.
    pub fn connect(client: &str) -> io::Result<Link> {
        Link::connect_to(&socket_path(), client)
    }

    fn connect_to(server: &Path, client: &str) -> io::Result<Link> {
        let own_path = client_path(server.parent().unwrap_or(Path::new("/tmp")), client, std::process::id());
        let _ = std::fs::remove_file(&own_path);
        let socket = UnixDatagram::bind(&own_path)?;
        let joined = socket.connect(server).and_then(|_| socket.set_nonblocking(true));
        if let Err(e) = joined {
            let _ = std::fs::remove_file(&own_path);
            return Err(e);
        }
        let mut link = Link { socket, own_path, buf: vec![0; model::MAX_LEN + 1] };
        link.send(&FromChrome::Hello { client: client.to_string() })?;
        Ok(link)
    }

    /// Sends one message. `WouldBlock` means the shell's queue is full: the caller may retry.
    pub fn send(&mut self, m: &FromChrome) -> io::Result<()> {
        let bytes = model::encode_from_chrome(m);
        self.socket.send(&bytes).map(|_| ())
    }

    /// Every message waiting, without blocking. An error means the shell went away. Packets that do not decode
    /// (or that are too large) are dropped.
    pub fn recv(&mut self) -> io::Result<Vec<ToChrome>> {
        let mut out = Vec::new();
        loop {
            match self.socket.recv(&mut self.buf) {
                Ok(n) => {
                    // The buffer holds one byte more than the largest message, so a longer packet was truncated.
                    if n > model::MAX_LEN {
                        continue;
                    }
                    if let Ok(m) = model::decode_to_chrome(&self.buf[..n]) {
                        out.push(m);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(out),
                Err(e) => return Err(e),
            }
        }
    }

    /// For the client's poll loop (`Display::dispatch`'s `extra_fd`).
    pub fn fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.own_path);
    }
}

/// A connected client, as the shell sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u32);

/// The shell's end: listens, accepts, reads and broadcasts. Non-blocking; polled from the shell loop.
pub struct HostLink {
    socket: UnixDatagram,
    path: PathBuf,
    /// Clients by the path they send from, in the order they first wrote.
    clients: Vec<(ClientId, PathBuf)>,
    next_id: u32,
    buf: Vec<u8>,
    /// Models a full client queue refused: (client, message kind, bytes); the newest per kind and client.
    pending: Vec<(ClientId, u8, Vec<u8>)>,
}

impl HostLink {
    /// Binds the socket (replacing a stale one).
    pub fn bind() -> io::Result<HostLink> {
        HostLink::bind_at(&socket_path())
    }

    fn bind_at(path: &Path) -> io::Result<HostLink> {
        let _ = std::fs::remove_file(path);
        let socket = UnixDatagram::bind(path)?;
        socket.set_nonblocking(true)?;
        Ok(HostLink { socket, path: path.to_path_buf(), clients: Vec::new(), next_id: 1, buf: vec![0; model::MAX_LEN + 1], pending: Vec::new() })
    }

    /// Accepts new clients and returns every message waiting, with who sent it. A sender that has no path (it did
    /// not bind one) cannot be answered and its messages are dropped.
    pub fn poll(&mut self) -> Vec<(ClientId, FromChrome)> {
        self.retry_pending();
        let mut out = Vec::new();
        loop {
            let (n, addr) = match self.socket.recv_from(&mut self.buf) {
                Ok(v) => v,
                Err(_) => break,
            };
            let Some(from) = addr.as_pathname().map(Path::to_path_buf) else { continue };
            if n > model::MAX_LEN {
                continue;
            }
            let id = self.client_for(from);
            if let Ok(m) = model::decode_from_chrome(&self.buf[..n]) {
                out.push((id, m));
            }
        }
        out
    }

    fn client_for(&mut self, path: PathBuf) -> ClientId {
        if let Some((id, _)) = self.clients.iter().find(|(_, p)| *p == path) {
            return *id;
        }
        let id = ClientId(self.next_id);
        self.next_id += 1;
        self.clients.push((id, path));
        id
    }

    /// Sends to one client.
    pub fn send(&mut self, to: ClientId, m: &ToChrome) {
        let bytes = model::encode_to_chrome(m);
        if let Some(path) = self.path_of(to) {
            if !self.deliver(to, &path, bytes) {
                self.forget(to);
            }
        }
    }

    /// Sends to every client; the ones that cannot be reached are forgotten.
    pub fn broadcast(&mut self, m: &ToChrome) {
        let bytes = model::encode_to_chrome(m);
        let targets = self.clients.clone();
        for (id, path) in targets {
            if !self.deliver(id, &path, bytes.clone()) {
                self.forget(id);
            }
        }
    }

    /// The fds to poll: the host socket receives every client's messages.
    pub fn fds(&self) -> Vec<RawFd> {
        vec![self.socket.as_raw_fd()]
    }

    /// Clients the host currently sends to.
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    fn path_of(&self, id: ClientId) -> Option<PathBuf> {
        self.clients.iter().find(|(c, _)| *c == id).map(|(_, p)| p.clone())
    }

    fn forget(&mut self, id: ClientId) {
        self.clients.retain(|(c, _)| *c != id);
        self.pending.retain(|(c, _, _)| *c != id);
    }

    /// Sends again what a full queue refused earlier.
    fn retry_pending(&mut self) {
        for (id, _kind, bytes) in std::mem::take(&mut self.pending) {
            if let Some(path) = self.path_of(id) {
                if !self.deliver(id, &path, bytes) {
                    self.forget(id);
                }
            }
        }
    }

    /// Whether the client is still there. A full queue keeps the message as the newest of its kind (resent by
    /// `poll`); a message the transport refuses as too large is dropped but the client stays; only a path that is
    /// gone or refuses connections ends the client.
    fn deliver(&mut self, id: ClientId, path: &Path, bytes: Vec<u8>) -> bool {
        let kind = bytes.get(1).copied().unwrap_or(0);
        match self.socket.send_to(&bytes, path) {
            Ok(_) => {
                self.pending.retain(|(c, k, _)| !(*c == id && *k == kind));
                true
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.pending.retain(|(c, k, _)| !(*c == id && *k == kind));
                self.pending.push((id, kind, bytes));
                true
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => false,
            Err(_) => true,
        }
    }
}

impl Drop for HostLink {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DockModel, DockPosition, FromChrome, ToChrome};

    /// A fresh directory for one test's sockets (short names: Unix socket paths are limited to 108 bytes).
    fn test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bcs-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Polls the host until `want` messages arrive or a bound is reached (the datagrams are local, so this is quick).
    fn poll_until(host: &mut HostLink, want: usize) -> Vec<(ClientId, FromChrome)> {
        let mut got = Vec::new();
        for _ in 0..200 {
            got.extend(host.poll());
            if got.len() >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        got
    }

    /// Reads a client's models until one arrives or the bound is reached.
    fn recv_until(link: &mut Link, want: usize) -> Vec<ToChrome> {
        let mut got = Vec::new();
        for _ in 0..200 {
            got.extend(link.recv().expect("the host is up"));
            if got.len() >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn client_path_keeps_one_safe_component() {
        let dir = Path::new("/run/user/1000");
        assert_eq!(client_path(dir, "chrome-dock", 42), PathBuf::from("/run/user/1000/bifrost-chrome-chrome-dock-42.sock"));
        assert_eq!(client_path(dir, "../evil name", 1), PathBuf::from("/run/user/1000/bifrost-chrome-----evil-name-1.sock"));
    }

    #[test]
    fn socket_path_names_the_chrome_socket() {
        assert_eq!(socket_path().file_name().unwrap(), "bifrost-chrome.sock");
    }

    #[test]
    fn hello_then_models_round_trip_over_the_socket() {
        let dir = test_dir("hello");
        let server = dir.join("bifrost-chrome.sock");
        let mut host = HostLink::bind_at(&server).expect("bind the host");
        let mut link = Link::connect_to(&server, "chrome-dock").expect("connect");

        let hello = poll_until(&mut host, 1);
        assert_eq!(hello.len(), 1);
        assert_eq!(hello[0].1, FromChrome::Hello { client: "chrome-dock".into() });
        let id = hello[0].0;
        assert_eq!(host.client_count(), 1);

        let model = ToChrome::Dock(DockModel { items: vec![], size: 48.0, magnified: 0.0, position: DockPosition::Bottom, autohide: false });
        host.send(id, &model);
        assert_eq!(recv_until(&mut link, 1), vec![model]);

        link.send(&FromChrome::DockActivate { app_id: "files".into() }).expect("send");
        let got = poll_until(&mut host, 1);
        assert_eq!(got, vec![(id, FromChrome::DockActivate { app_id: "files".into() })]);
    }

    #[test]
    fn broadcast_reaches_every_client_and_a_gone_client_is_forgotten() {
        let dir = test_dir("bcast");
        let server = dir.join("bifrost-chrome.sock");
        let mut host = HostLink::bind_at(&server).expect("bind the host");
        let mut a = Link::connect_to(&server, "chrome-menubar").expect("connect a");
        let mut b = Link::connect_to(&server, "chrome-panels").expect("connect b");
        assert_eq!(poll_until(&mut host, 2).len(), 2);
        assert_eq!(host.client_count(), 2);

        host.broadcast(&ToChrome::ShowPanel("control".into()));
        assert_eq!(recv_until(&mut a, 1), vec![ToChrome::ShowPanel("control".into())]);
        assert_eq!(recv_until(&mut b, 1), vec![ToChrome::ShowPanel("control".into())]);

        // Dropping a client removes its path; the next send to it fails and the host forgets it.
        drop(b);
        host.broadcast(&ToChrome::ShowPanel(String::new()));
        assert_eq!(host.client_count(), 1);
        assert_eq!(recv_until(&mut a, 1), vec![ToChrome::ShowPanel(String::new())]);
    }

    #[test]
    fn a_full_client_queue_is_resent_and_a_huge_model_keeps_the_client() {
        let dir = test_dir("full");
        let server = dir.join("bifrost-chrome.sock");
        let mut host = HostLink::bind_at(&server).expect("bind the host");
        let mut link = Link::connect_to(&server, "chrome-panels").expect("connect");
        let id = poll_until(&mut host, 1)[0].0;
        let model = |n: u32| ToChrome::Dock(DockModel { items: vec![], size: n as f32, magnified: 0.0, position: DockPosition::Bottom, autohide: false });
        // the client never reads: publish until the queue refuses and a model is held back
        let mut n = 0;
        while host.pending.is_empty() {
            n += 1;
            host.send(id, &model(n));
            assert!(n < 100_000, "the queue must fill");
        }
        assert_eq!(host.client_count(), 1);
        // the newest model is the held one; drain the client, let the host resend
        let latest = model(n);
        let _ = link.recv().expect("the host is up");
        let mut got = Vec::new();
        for _ in 0..200 {
            host.poll();
            got.extend(link.recv().expect("the host is up"));
            if got.last() == Some(&latest) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.last(), Some(&latest), "the client ends with the newest model");
        // a model over the socket's send buffer is refused by the transport, not a reason to forget the client
        let big = ToChrome::Panels(crate::model::PanelsModel {
            notifications: (0..3000).map(|i| crate::model::Notification { id: i, body: "x".repeat(100), ..Default::default() }).collect(),
            controls: vec![],
        });
        host.send(id, &big);
        assert_eq!(host.client_count(), 1);
    }

    #[test]
    fn garbage_packets_are_dropped_and_the_host_keeps_serving() {
        let dir = test_dir("junk");
        let server = dir.join("bifrost-chrome.sock");
        let mut host = HostLink::bind_at(&server).expect("bind the host");
        let mut link = Link::connect_to(&server, "chrome-dock").expect("connect");
        assert_eq!(poll_until(&mut host, 1).len(), 1);

        // Raw junk from the client's socket: a bad version byte, then an unknown tag.
        link.socket.send(&[9, 9, 9]).expect("send junk");
        link.socket.send(&[model::VERSION, 99]).expect("send junk");
        link.send(&FromChrome::Hello { client: "again".into() }).expect("send");
        let got = poll_until(&mut host, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, FromChrome::Hello { client: "again".into() });
    }

    #[test]
    fn a_missing_host_is_an_error_for_the_client() {
        let dir = test_dir("nohost");
        let server = dir.join("absent.sock");
        assert!(Link::connect_to(&server, "chrome-dock").is_err());
    }

    #[test]
    fn host_bind_replaces_a_stale_socket_file() {
        let dir = test_dir("stale");
        let server = dir.join("bifrost-chrome.sock");
        std::fs::write(&server, b"stale").expect("write a stale file");
        let host = HostLink::bind_at(&server).expect("bind over the stale file");
        assert_eq!(host.fds().len(), 1);
        drop(host);
        assert!(!server.exists(), "the host removes its path on drop");
    }
}
