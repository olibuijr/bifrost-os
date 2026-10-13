//! RFB (VNC) client protocol, pure: the handshake, the messages a viewer sends, the server messages it reads, and the
//! framebuffer those messages update. There are no sockets and no threads here; the Screen Sharing app owns the
//! connection and calls these functions over any `Read` or `Write`.
//!
//! Supported: RFB 3.3, 3.7 and 3.8; security None (1) only; true-colour pixel formats of 8, 16 and 32 bits; Raw and
//! CopyRect rectangles, plus the DesktopSize pseudo-encoding for resizes. Password (VNC) authentication is refused
//! with `Error::Unsupported`, and no other encodings are requested, so a server must send Raw rectangles.

pub mod framebuffer;
pub mod handshake;
pub mod keysym;
pub mod message;
pub mod pixel;

pub use framebuffer::Framebuffer;
pub use handshake::{ServerInit, Version};
pub use message::{Payload, Rect, ServerMessage};
pub use pixel::PixelFormat;

use std::fmt;

/// Why a connection cannot continue: the server broke the protocol, asked for something this viewer lacks, or the
/// socket failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Protocol(String),
    Unsupported(String),
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Protocol(s) => write!(f, "protocol error: {s}"),
            Error::Unsupported(s) => write!(f, "not supported: {s}"),
            Error::Io(s) => write!(f, "connection error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// The default TCP port of a VNC server (display 0).
pub const DEFAULT_PORT: u16 = 5900;

/// Adds `DEFAULT_PORT` to an address that has no port. An IPv6 literal (`[::1]`) or a `host:port` is returned as is.
pub fn with_default_port(addr: &str) -> String {
    let addr = addr.trim();
    if addr.starts_with('[') {
        return if addr.contains("]:") { addr.to_string() } else { format!("{addr}:{DEFAULT_PORT}") };
    }
    match addr.rfind(':') {
        Some(i) if !addr[..i].contains(':') && addr[i + 1..].parse::<u16>().is_ok() => addr.to_string(),
        _ if addr.contains(':') => addr.to_string(),
        _ => format!("{addr}:{DEFAULT_PORT}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_added_only_when_missing() {
        assert_eq!(with_default_port("192.168.1.20"), "192.168.1.20:5900");
        assert_eq!(with_default_port(" host.local "), "host.local:5900");
        assert_eq!(with_default_port("192.168.1.20:5901"), "192.168.1.20:5901");
        assert_eq!(with_default_port("[::1]"), "[::1]:5900");
        assert_eq!(with_default_port("[::1]:5902"), "[::1]:5902");
    }

    #[test]
    fn error_messages_name_the_class() {
        assert_eq!(Error::Protocol("bad".into()).to_string(), "protocol error: bad");
        assert_eq!(Error::Unsupported("vnc auth".into()).to_string(), "not supported: vnc auth");
    }
}
