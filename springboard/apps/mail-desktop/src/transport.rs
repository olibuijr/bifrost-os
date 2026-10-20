//! Connections for the mail protocols. Every connection is made by the shell's TLS transport
//! (`springboard::deildu::http`), so certificates are checked against the same roots as the rest of the desktop.

use bifrost_app::controls::internet_accounts::{Security, Server};
use springboard::deildu::http::{Stream, Tls, Transport};
use std::io;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::Arc;

/// The shared transport. Built once; each worker thread gets a clone of the `Arc`.
pub fn shared() -> Arc<dyn Transport> {
    Arc::new(Tls::new())
}

/// Opens the TCP connection for `server`: TLS from the first byte for `Security::Tls`, plain otherwise. A STARTTLS
/// server is upgraded by its protocol code once the server has agreed to it. A `Security::Plain` server never gets a
/// password in the clear across the internet: it is connected only when its name resolves to this computer or a private
/// network, and the connection goes to the address that was checked.
pub fn open(transport: &dyn Transport, server: &Server) -> io::Result<Box<dyn Stream>> {
    if server.security == Security::Plain {
        let ip = local_address(&server.host, server.port)?;
        return transport.connect(&ip.to_string(), server.port, false);
    }
    transport.connect(&server.host, server.port, server.security == Security::Tls)
}

/// True for an address on this computer or a private or link-local network, where an unencrypted connection stays off
/// the internet.
pub fn is_local(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// The IPv4 address of `host` for an unencrypted connection: every address it resolves to must be local.
fn local_address(host: &str, port: u16) -> io::Result<Ipv4Addr> {
    let found: Vec<Ipv4Addr> = (host, port)
        .to_socket_addrs()?
        .filter_map(|a| match a.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .collect();
    match found.first() {
        Some(first) if found.iter().all(|ip| is_local(*ip)) => Ok(*first),
        _ => Err(io::Error::other(format!(
            "{host} is not on a local network, so Mail will not send the password unencrypted. Choose SSL/TLS or STARTTLS for this account"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_addresses_may_be_unencrypted() {
        for ip in [[127, 0, 0, 1], [10, 1, 2, 3], [192, 168, 20, 140], [172, 16, 0, 1], [169, 254, 1, 1]] {
            assert!(is_local(Ipv4Addr::from(ip)), "{ip:?}");
        }
        for ip in [[8, 8, 8, 8], [93, 184, 216, 34], [172, 32, 0, 1]] {
            assert!(!is_local(Ipv4Addr::from(ip)), "{ip:?}");
        }
    }

    #[test]
    fn a_public_address_is_refused_before_any_connection() {
        let refused = local_address("8.8.8.8", 143).unwrap_err().to_string();
        assert!(refused.contains("not on a local network"), "{refused}");
        assert_eq!(local_address("127.0.0.1", 143).unwrap(), Ipv4Addr::LOCALHOST);
    }
}
