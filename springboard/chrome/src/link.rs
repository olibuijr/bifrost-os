//! The socket between the shell and the chrome clients: `$XDG_RUNTIME_DIR/bifrost-chrome.sock` (`SOCK_SEQPACKET`,
//! one `model` message per packet), served by the shell (`HostLink`) and joined by each client (`Link`), which says
//! `FromChrome::Hello` first and then gets the current models.
//!
//! Owner: wave G2 agent `chrome-core`.

use crate::model::{FromChrome, ToChrome};
use std::os::fd::RawFd;
use std::path::PathBuf;

/// The socket path.
pub fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("bifrost-chrome.sock")
}

/// A client's end.
pub struct Link {}

impl Link {
    /// Connects and sends `Hello { client }`.
    pub fn connect(client: &str) -> std::io::Result<Link> {
        let _ = client;
        todo!("chrome-core")
    }

    pub fn send(&mut self, m: &FromChrome) -> std::io::Result<()> {
        let _ = m;
        todo!("chrome-core")
    }

    /// Every message waiting, without blocking. An error means the shell went away.
    pub fn recv(&mut self) -> std::io::Result<Vec<ToChrome>> {
        todo!("chrome-core")
    }

    /// For the client's poll loop (`Display::dispatch`'s `extra_fd`).
    pub fn fd(&self) -> RawFd {
        todo!("chrome-core")
    }
}

/// A connected client, as the shell sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u32);

/// The shell's end: listens, accepts, reads and broadcasts. Non-blocking; polled from the shell loop.
pub struct HostLink {}

impl HostLink {
    /// Binds the socket (replacing a stale one).
    pub fn bind() -> std::io::Result<HostLink> {
        todo!("chrome-core")
    }

    /// Accepts new clients and returns every message waiting, with who sent it; dropped clients are forgotten.
    pub fn poll(&mut self) -> Vec<(ClientId, FromChrome)> {
        todo!("chrome-core")
    }

    /// Sends to one client.
    pub fn send(&mut self, to: ClientId, m: &ToChrome) {
        let _ = (to, m);
        todo!("chrome-core")
    }

    /// Sends to every client.
    pub fn broadcast(&mut self, m: &ToChrome) {
        let _ = m;
        todo!("chrome-core")
    }

    /// The fds to poll (listener and clients).
    pub fn fds(&self) -> Vec<RawFd> {
        todo!("chrome-core")
    }
}
