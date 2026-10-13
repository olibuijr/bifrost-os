//! Connections for the mail protocols. Every connection is made by the shell's TLS transport
//! (`springboard::deildu::http`), so certificates are checked against the same roots as the rest of the desktop.

use bifrost_app::controls::internet_accounts::{Security, Server};
use springboard::deildu::http::{Stream, Tls, Transport};
use std::io;
use std::sync::Arc;

/// The shared transport. Built once; each worker thread gets a clone of the `Arc`.
pub fn shared() -> Arc<dyn Transport> {
    Arc::new(Tls::new())
}

/// Opens the TCP connection for `server`: TLS from the first byte for `Security::Tls`, plain otherwise. A STARTTLS
/// server is upgraded by its protocol code once the server has agreed to it.
pub fn open(transport: &dyn Transport, server: &Server) -> io::Result<Box<dyn Stream>> {
    transport.connect(&server.host, server.port, server.security == Security::Tls)
}
