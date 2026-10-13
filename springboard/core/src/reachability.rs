//! Reachability for apps: whether this machine has a default route, and whether a host accepts a TCP connection on a
//! port.
//!
//! [`has_default_route`] reads the kernel's routing table text (`/proc/net/route` on Linux). It says the machine has a
//! way out, not that any particular host is reachable. [`probe`] opens a TCP connection to the host and closes it at
//! once. It is a test of one host and one port, not a copy of the system's route flags: it blocks for up to the timeout
//! while the name is resolved and the connection is attempted, so call it off the main thread.

use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The kernel's IPv4 routing table, one route per line after a header.
pub const ROUTE_TABLE: &str = "/proc/net/route";

/// The outcome of a [`probe`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The connection opened. The address is the one that answered.
    Reachable(SocketAddr),
    /// The name did not resolve to any address.
    NoAddress,
    /// The host answered and refused the connection: nothing listens on that port.
    Refused,
    /// No answer before the timeout.
    TimedOut,
    /// Another failure, such as no route to the host, with the system's message.
    Failed(String),
}

/// True when the routing table holds a default route: a destination and a mask that are both zero. The header line
/// and lines too short to hold a mask are ignored.
pub fn has_default_route(table: &str) -> bool {
    table.lines().skip(1).any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() >= 8 && f[1] == "00000000" && f[7] == "00000000"
    })
}

/// Whether the machine has a default route, read from [`ROUTE_TABLE`]. An unreadable table counts as no route.
pub fn default_route_present() -> bool {
    std::fs::read_to_string(ROUTE_TABLE).map(|t| has_default_route(&t)).unwrap_or(false)
}

/// Maps a failed connection to its outcome.
pub fn classify(err: &io::Error) -> Reach {
    match err.kind() {
        io::ErrorKind::ConnectionRefused => Reach::Refused,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Reach::TimedOut,
        _ => Reach::Failed(err.to_string()),
    }
}

/// Opens a TCP connection to `host:port`, trying each resolved address in turn, and closes it again. The first address
/// that accepts wins. A name that does not resolve is [`Reach::NoAddress`].
pub fn probe(host: &str, port: u16, timeout: Duration) -> Reach {
    let addrs: Vec<SocketAddr> = match (host, port).to_socket_addrs() {
        Ok(found) => found.collect(),
        Err(_) => return Reach::NoAddress,
    };
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_stream) => return Reach::Reachable(addr),
            Err(err) => last = Some(err),
        }
    }
    match last {
        Some(err) => classify(&err),
        None => Reach::NoAddress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const TABLE: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";

    #[test]
    fn a_zero_destination_and_mask_is_a_default_route() {
        assert!(has_default_route(TABLE));
    }

    #[test]
    fn a_table_with_only_subnet_routes_has_no_default_route() {
        let subnet_only = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert!(!has_default_route(subnet_only));
        assert!(!has_default_route(""));
    }

    #[test]
    fn refused_and_timed_out_connections_are_told_apart_from_other_failures() {
        assert_eq!(classify(&io::Error::from(io::ErrorKind::ConnectionRefused)), Reach::Refused);
        assert_eq!(classify(&io::Error::from(io::ErrorKind::TimedOut)), Reach::TimedOut);
        assert!(matches!(classify(&io::Error::from(io::ErrorKind::PermissionDenied)), Reach::Failed(_)));
    }

    #[test]
    fn a_listening_loopback_port_is_reachable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let got = probe("127.0.0.1", port, Duration::from_secs(2));
        assert_eq!(got, Reach::Reachable(SocketAddr::from(([127, 0, 0, 1], port))));
    }

    #[test]
    fn a_closed_loopback_port_is_refused() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(probe("127.0.0.1", port, Duration::from_secs(2)), Reach::Refused);
    }
}
