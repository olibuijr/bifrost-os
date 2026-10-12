//! Nearby remote computers: a DNS-SD browse for `_bifrost-remote._tcp` over multicast DNS.
//!
//! The browse itself is the SDK's `ServiceBrowser` (pure model). This module owns the one socket: a
//! non-blocking UDP socket on an ephemeral port that sends a PTR query to the mDNS group every few
//! seconds and reads whatever replies are waiting. Replies to a query from a non-5353 port come back
//! unicast to that port (RFC 6762 section 6.7), so the socket needs no privileged port. This app does
//! not advertise its own service: binding port 5353 next to a running mDNS responder needs
//! SO_REUSEADDR, which std cannot set (see docs/parity/wire-app-remote.md).

use bifrost_app::service_discovery::ServiceBrowser;
use bifrost_core::dnssd::Message;
use std::net::{IpAddr, SocketAddr, UdpSocket};

/// The service type a BifrOSt computer would advertise for remote sharing.
pub const SERVICE: &str = "_bifrost-remote._tcp";
/// Time between two queries, in milliseconds (the app's tick clock).
const QUERY_EVERY_MS: f64 = 5000.;
/// Largest datagram read; mDNS messages stay under the 9000-byte jumbo limit used by responders.
const MAX_DATAGRAM: usize = 9000;

/// One line of the nearby list: the instance name and its address, or `Resolving...` until the reply carries one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub detail: String,
}

/// What the list needs from one browsed instance. Built from `ServiceInstance` by `entries`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub addresses: Vec<IpAddr>,
}

/// Turns browsed entries into list rows. An IPv4 address is preferred; an IPv6 address is bracketed in the URL.
pub fn rows(entries: &[Entry]) -> Vec<Row> {
    entries
        .iter()
        .map(|e| {
            let detail = match (preferred(&e.addresses), e.port) {
                (Some(ip), Some(port)) => format!("http://{}", SocketAddr::new(ip, port)),
                _ => match (e.host.as_deref(), e.port) {
                    (Some(host), Some(port)) => format!("http://{host}:{port}"),
                    _ => "Resolving\u{2026}".to_string(),
                },
            };
            Row { title: e.name.clone(), detail }
        })
        .collect()
}

fn preferred(addrs: &[IpAddr]) -> Option<IpAddr> {
    addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied()
}

fn mdns_group() -> SocketAddr {
    SocketAddr::from(([224, 0, 0, 251], 5353))
}

/// The browse state the app keeps between ticks.
pub struct Nearby {
    browser: Option<ServiceBrowser>,
    socket: Option<UdpSocket>,
    /// The socket could not be opened: the list stays empty and nothing is retried.
    failed: bool,
    last_query: Option<f64>,
    query_id: u16,
    pub rows: Vec<Row>,
}

impl Default for Nearby {
    fn default() -> Self {
        Self::new()
    }
}

impl Nearby {
    pub fn new() -> Self {
        Nearby { browser: ServiceBrowser::new(SERVICE, ""), socket: None, failed: false, last_query: None, query_id: 1, rows: Vec::new() }
    }

    /// Sends a query when one is due and reads every reply that is waiting. `now` is the tick clock in milliseconds.
    /// Returns true when the list of nearby computers changed.
    pub fn poll(&mut self, now: f64) -> bool {
        if self.failed {
            return false;
        }
        let Some(browser) = self.browser.as_mut() else {
            return false;
        };
        if self.socket.is_none() {
            match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.set_nonblocking(true).map(|_| s)) {
                Ok(s) => self.socket = Some(s),
                Err(_) => {
                    self.failed = true;
                    return false;
                }
            }
        }
        let Some(socket) = self.socket.as_ref() else {
            return false;
        };
        let due = self.last_query.map_or(true, |t| now - t >= QUERY_EVERY_MS);
        if due {
            if let Ok(q) = browser.query(self.query_id) {
                let _ = socket.send_to(&q, mdns_group());
            }
            self.query_id = self.query_id.wrapping_add(1);
            self.last_query = Some(now);
        }
        let mut changed = 0;
        let mut buf = vec![0u8; MAX_DATAGRAM];
        while let Ok((n, _)) = socket.recv_from(&mut buf) {
            if let Ok(msg) = Message::decode(&buf[..n]) {
                changed += browser.handle_message(&msg);
            }
        }
        if changed == 0 {
            return false;
        }
        let entries: Vec<Entry> = browser
            .instances()
            .iter()
            .map(|i| Entry { name: i.name.clone(), host: i.host.clone(), port: i.port, addresses: i.addresses.clone() })
            .collect();
        self.rows = rows(&entries);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    fn entry(name: &str, host: Option<&str>, port: Option<u16>, addrs: Vec<IpAddr>) -> Entry {
        Entry { name: name.to_string(), host: host.map(str::to_string), port, addresses: addrs }
    }

    #[test]
    fn service_type_is_a_valid_dns_sd_browse() {
        let b = ServiceBrowser::new(SERVICE, "").expect("valid service type");
        assert_eq!(b.service(), "_bifrost-remote._tcp.local");
        assert!(b.instances().is_empty());
    }

    #[test]
    fn no_instances_gives_no_rows() {
        assert!(rows(&[]).is_empty());
    }

    #[test]
    fn resolved_instance_shows_its_ipv4_url() {
        let v4: IpAddr = "192.168.1.20".parse().unwrap();
        let v6: IpAddr = Ipv6Addr::LOCALHOST.into();
        let r = rows(&[entry("Studio", Some("studio.local"), Some(12000), vec![v6, v4])]);
        assert_eq!(r, vec![Row { title: "Studio".into(), detail: "http://192.168.1.20:12000".into() }]);
    }

    #[test]
    fn ipv6_only_instance_brackets_the_address() {
        let v6: IpAddr = "fe80::1".parse().unwrap();
        let r = rows(&[entry("Den", Some("den.local"), Some(12000), vec![v6])]);
        assert_eq!(r[0].detail, "http://[fe80::1]:12000");
    }

    #[test]
    fn host_without_address_falls_back_to_the_host_name() {
        let r = rows(&[entry("Den", Some("den.local"), Some(12000), Vec::new())]);
        assert_eq!(r[0].detail, "http://den.local:12000");
    }

    #[test]
    fn unresolved_instance_says_resolving() {
        let r = rows(&[entry("Den", None, None, Vec::new())]);
        assert_eq!(r[0].detail, "Resolving\u{2026}");
    }

    #[test]
    fn a_new_browse_starts_with_no_rows() {
        let n = Nearby::new();
        assert!(n.rows.is_empty());
        assert!(!n.failed);
    }
}
