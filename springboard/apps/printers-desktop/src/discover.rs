//! Printers on the local network: a DNS-SD browse for `_ipps._tcp` and `_ipp._tcp` over multicast DNS.
//!
//! The browse itself is the SDK's `ServiceBrowser` (a pure model). This module owns the one socket: a non-blocking UDP
//! socket on an ephemeral port that sends a PTR query for each service type to the mDNS group every few seconds and
//! reads the replies waiting. A query from a port other than 5353 is answered unicast to that port (RFC 6762
//! section 6.7), so no privileged port is needed. The socket is opened when the Add Printer pane opens and closed when
//! it closes. The TXT record of a printer carries its resource path (`rp`), model (`ty`) and location (`note`), which
//! become the address, the model line and the location of the queue that gets added.

use bifrost_app::controls::cups::Printer;
use bifrost_app::service_discovery::{ServiceBrowser, ServiceInstance};
use bifrost_core::dnssd::Message;
use bifrost_core::txt_record::{Presence, TxtRecord};
use std::net::{IpAddr, SocketAddr, UdpSocket};

/// The service types browsed, the secure one first, with the address scheme each one gives.
const SERVICES: [(&str, &str); 2] = [("_ipps._tcp", "ipps"), ("_ipp._tcp", "ipp")];
/// Milliseconds between two queries (the app's tick clock).
const QUERY_EVERY_MS: f64 = 3000.0;
/// The largest datagram read.
const MAX_DATAGRAM: usize = 9000;
/// The resource path of a printer whose TXT record names none.
const DEFAULT_PATH: &str = "ipp/print";

/// A printer the network announced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The name the printer advertises, such as "Front Office".
    pub name: String,
    /// The address to add: `ipps://192.168.1.40:631/ipp/print`.
    pub device_uri: String,
    /// The model line from the TXT record (`ty`), empty when absent.
    pub model: String,
    /// The location from the TXT record (`note`), empty when absent.
    pub location: String,
}

/// The text of a TXT key, trimmed; empty when the key is absent or holds no text.
fn txt_text(txt: &TxtRecord, key: &str) -> String {
    match txt.get(key) {
        Presence::Value(bytes) => String::from_utf8_lossy(bytes).trim().to_string(),
        _ => String::new(),
    }
}

/// The host part of an address: an IPv4 address first, then an IPv6 one in brackets, then the host name. `None` when
/// the reply named no host yet.
fn host_part(addresses: &[IpAddr], host: Option<&str>) -> Option<String> {
    if let Some(v4) = addresses.iter().find(|a| a.is_ipv4()) {
        return Some(v4.to_string());
    }
    if let Some(v6) = addresses.first() {
        return Some(format!("[{v6}]"));
    }
    host.map(|h| h.trim_end_matches('.').to_string()).filter(|h| !h.is_empty())
}

/// The printer an instance stands for, or `None` while its host or port is not resolved yet.
pub fn found_from(instance: &ServiceInstance, scheme: &str) -> Option<Found> {
    let port = instance.port?;
    let host = host_part(&instance.addresses, instance.host.as_deref())?;
    let path = txt_text(&instance.txt, "rp");
    let path = path.trim_matches('/');
    let path = if path.is_empty() { DEFAULT_PATH } else { path };
    Some(Found {
        name: instance.name.clone(),
        device_uri: format!("{scheme}://{host}:{port}/{path}"),
        model: txt_text(&instance.txt, "ty"),
        location: txt_text(&instance.txt, "note"),
    })
}

/// The printers of every browse, in the order of `SERVICES`. A printer announced under both types is listed once, under
/// the first.
pub fn merge(lists: &[(&str, &[ServiceInstance])]) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for (scheme, instances) in lists {
        for instance in *instances {
            if let Some(printer) = found_from(instance, scheme) {
                if !found.iter().any(|f| f.name == printer.name) {
                    found.push(printer);
                }
            }
        }
    }
    found
}

/// True when a queue for this printer exists already: the same address, or the same host and path under another scheme.
pub fn is_added(found: &Found, printers: &[Printer]) -> bool {
    let key = |uri: &str| uri.split_once("://").map_or(uri.to_string(), |(_, rest)| rest.to_string());
    let wanted = key(&found.device_uri);
    printers.iter().any(|p| !p.device_uri.is_empty() && (p.device_uri == found.device_uri || key(&p.device_uri) == wanted))
}

/// The second line of a row: model, location and address, whichever the printer gave.
pub fn detail_line(found: &Found) -> String {
    [found.model.as_str(), found.location.as_str(), found.device_uri.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ")
}

fn mdns_group() -> SocketAddr {
    SocketAddr::from(([224, 0, 0, 251], 5353))
}

/// The browse state the window keeps while the Add Printer pane is open.
pub struct Discover {
    browsers: Vec<(&'static str, ServiceBrowser)>,
    socket: Option<UdpSocket>,
    /// The socket could not be opened: the list stays empty and nothing is retried until the pane opens again.
    failed: bool,
    last_query: Option<f64>,
    query_id: u16,
    /// The printers found so far.
    pub found: Vec<Found>,
}

impl Default for Discover {
    fn default() -> Self {
        Self::new()
    }
}

impl Discover {
    pub fn new() -> Discover {
        let browsers = SERVICES.iter().filter_map(|(kind, scheme)| ServiceBrowser::new(kind, "").map(|b| (*scheme, b))).collect();
        Discover { browsers, socket: None, failed: false, last_query: None, query_id: 1, found: Vec::new() }
    }

    /// True when the socket could not be opened.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Starts a fresh browse: forgets what was found and asks again at the next poll.
    pub fn restart(&mut self) {
        *self = Discover::new();
    }

    /// Ends the browse and closes the socket.
    pub fn stop(&mut self) {
        self.socket = None;
        self.last_query = None;
    }

    /// Sends the queries when due and reads every reply waiting. `now` is the tick clock in milliseconds. True when
    /// the list of printers changed.
    pub fn poll(&mut self, now: f64) -> bool {
        if self.failed {
            return false;
        }
        if self.socket.is_none() {
            match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.set_nonblocking(true).map(|()| s)) {
                Ok(s) => self.socket = Some(s),
                Err(_) => {
                    self.failed = true;
                    return false;
                }
            }
        }
        let Some(socket) = self.socket.as_ref() else { return false };
        if self.last_query.map_or(true, |t| now - t >= QUERY_EVERY_MS) {
            for (_, browser) in &self.browsers {
                if let Ok(query) = browser.query(self.query_id) {
                    let _ = socket.send_to(&query, mdns_group());
                }
                self.query_id = self.query_id.wrapping_add(1);
            }
            self.last_query = Some(now);
        }
        let mut changed = 0;
        let mut buf = vec![0u8; MAX_DATAGRAM];
        while let Ok((n, _)) = socket.recv_from(&mut buf) {
            if let Ok(message) = Message::decode(&buf[..n]) {
                for (_, browser) in &mut self.browsers {
                    changed += browser.handle_message(&message);
                }
            }
        }
        if changed == 0 {
            return false;
        }
        let lists: Vec<(&str, &[ServiceInstance])> = self.browsers.iter().map(|(scheme, b)| (*scheme, b.instances())).collect();
        let found = merge(&lists);
        let differs = found != self.found;
        self.found = found;
        differs
    }
}

/// The label, information and location a new queue gets from what the network announced: the queue name is made from
/// the advertised name (see `model::queue_name_for`), the description is the model line, the location as announced.
pub fn queue_details(found: &Found) -> (String, String, String) {
    let info = if found.model.is_empty() { found.name.clone() } else { found.model.clone() };
    (found.name.clone(), info, found.location.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;
    use bifrost_app::controls::cups::PrinterState;

    fn instance(name: &str, host: Option<&str>, port: Option<u16>, addresses: Vec<IpAddr>, txt: &[(&str, &str)]) -> ServiceInstance {
        let mut record = TxtRecord::new();
        for (k, v) in txt {
            record.set(k, Some(v.as_bytes())).unwrap();
        }
        ServiceInstance {
            name: name.into(),
            full_name: format!("{name}._ipp._tcp.local"),
            service: "_ipp._tcp.local".into(),
            host: host.map(str::to_string),
            port,
            addresses,
            txt: record,
        }
    }

    fn printer_at(device_uri: &str) -> Printer {
        Printer {
            name: "Office".into(),
            uri: String::new(),
            state: PrinterState::Idle,
            state_message: String::new(),
            accepting_jobs: true,
            is_default: false,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: device_uri.into(),
            queued_jobs: 0,
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    #[test]
    fn a_resolved_instance_becomes_an_address_with_its_model_and_location() {
        let v4: IpAddr = "192.168.1.40".parse().unwrap();
        let i = instance("Front Office", Some("front.local."), Some(631), vec![v4], &[("rp", "ipp/print"), ("ty", "Acme Laser 5"), ("note", "Floor 2")]);
        let f = found_from(&i, "ipps").unwrap();
        assert_eq!(f.device_uri, "ipps://192.168.1.40:631/ipp/print");
        assert_eq!((f.model.as_str(), f.location.as_str(), f.name.as_str()), ("Acme Laser 5", "Floor 2", "Front Office"));
        assert!(bifrost_app::controls::cups::valid_device_uri(&f.device_uri));
    }

    #[test]
    fn the_address_prefers_ipv4_then_ipv6_then_the_host_name() {
        let v4: IpAddr = "10.0.0.7".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        let both = instance("A", Some("a.local"), Some(631), vec![v6, v4], &[("rp", "/printers/a/")]);
        assert_eq!(found_from(&both, "ipp").unwrap().device_uri, "ipp://10.0.0.7:631/printers/a");
        let only6 = instance("A", Some("a.local"), Some(631), vec![v6], &[]);
        assert_eq!(found_from(&only6, "ipp").unwrap().device_uri, "ipp://[fe80::1]:631/ipp/print");
        let named = instance("A", Some("a.local."), Some(8631), vec![], &[]);
        assert_eq!(found_from(&named, "ipp").unwrap().device_uri, "ipp://a.local:8631/ipp/print");
    }

    #[test]
    fn an_instance_that_is_not_resolved_yet_is_not_listed() {
        assert_eq!(found_from(&instance("A", None, None, vec![], &[]), "ipp"), None);
        assert_eq!(found_from(&instance("A", Some("a.local"), None, vec![], &[]), "ipp"), None, "no port yet");
        assert_eq!(found_from(&instance("A", None, Some(631), vec![], &[]), "ipp"), None, "no host yet");
    }

    #[test]
    fn a_printer_announced_under_both_types_is_listed_once_under_the_secure_one() {
        let v4: IpAddr = "10.0.0.7".parse().unwrap();
        let secure = vec![instance("Den", Some("den.local"), Some(443), vec![v4], &[])];
        let plain = vec![instance("Den", Some("den.local"), Some(631), vec![v4], &[]), instance("Lab", Some("lab.local"), Some(631), vec![v4], &[])];
        let list = merge(&[("ipps", secure.as_slice()), ("ipp", plain.as_slice())]);
        assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Den", "Lab"]);
        assert!(list[0].device_uri.starts_with("ipps://"));
    }

    #[test]
    fn a_printer_with_a_queue_already_is_marked_added_whatever_the_scheme() {
        let f = Found { name: "Den".into(), device_uri: "ipps://10.0.0.7:631/ipp/print".into(), model: String::new(), location: String::new() };
        assert!(is_added(&f, &[printer_at("ipps://10.0.0.7:631/ipp/print")]));
        assert!(is_added(&f, &[printer_at("ipp://10.0.0.7:631/ipp/print")]));
        assert!(!is_added(&f, &[printer_at("ipp://10.0.0.8:631/ipp/print"), printer_at("")]));
        assert!(!is_added(&f, &[]));
    }

    #[test]
    fn the_row_detail_joins_what_is_known() {
        let f = Found { name: "Den".into(), device_uri: "ipp://10.0.0.7:631/ipp/print".into(), model: "Acme".into(), location: String::new() };
        assert_eq!(detail_line(&f), "Acme, ipp://10.0.0.7:631/ipp/print");
        let (label, info, location) = queue_details(&f);
        assert_eq!((label.as_str(), info.as_str(), location.as_str()), ("Den", "Acme", ""));
        let bare = Found { model: String::new(), ..f };
        assert_eq!(queue_details(&bare).1, "Den", "without a model line the description is the name");
        assert_eq!(model::queue_name_for(&bare.name, &[]), "Den");
    }

    #[test]
    fn a_fresh_browse_asks_for_both_service_types_and_finds_nothing() {
        let d = Discover::new();
        assert_eq!(d.browsers.len(), 2);
        assert_eq!(d.browsers[0].1.service(), "_ipps._tcp.local");
        assert_eq!(d.browsers[1].1.service(), "_ipp._tcp.local");
        assert!(d.found.is_empty() && !d.failed());
    }
}
