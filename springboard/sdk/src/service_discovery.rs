//! Local service discovery (DNS-SD, RFC 6763, over multicast DNS, RFC 6762): browse for services and advertise one.
//!
//! `ServiceBrowser` builds the PTR query for a service type and folds the replies it is handed into a list of
//! instances. `ServiceAdvertiser` builds the reply that publishes one instance and tells whether a query asks for it.
//! Pure std and no socket: the caller owns the UDP socket on port 5353, sends the bytes `query` and `response` return,
//! and passes each received message to `handle_message` or `matches_query`. A record with a zero TTL is a goodbye and
//! removes its instance. Caching and repeated queries are the caller's job.

use bifrost_core::dnssd::{self, rtype, Message, Record, WireError};
use bifrost_core::txt_record::TxtRecord;
use std::net::IpAddr;

/// The domain used when the caller passes an empty one.
pub const DEFAULT_DOMAIN: &str = "local";
/// Flags of a reply: response and authoritative answer.
const RESPONSE_FLAGS: u16 = 0x8400;
/// Time to live of published records, in seconds (RFC 6762 section 10 gives 120 for host and service records).
const DEFAULT_TTL: u32 = 120;

/// The DNS name of a service type in a domain. `kind` is two labels: a service starting with an underscore, then
/// `_tcp` or `_udp`, such as `_http._tcp`. An empty domain means the local domain. Returns `None` for any other shape.
pub fn service_type_name(kind: &str, domain: &str) -> Option<String> {
    let kind = kind.trim_end_matches('.');
    let labels: Vec<&str> = kind.split('.').collect();
    if labels.len() != 2 || labels[0].len() < 2 || !labels[0].starts_with('_') {
        return None;
    }
    if labels[1] != "_tcp" && labels[1] != "_udp" {
        return None;
    }
    let domain = domain.trim_matches('.');
    let domain = if domain.is_empty() { DEFAULT_DOMAIN } else { domain };
    Some(format!("{kind}.{domain}"))
}

/// One service found by a browse. `name` is the instance label ("Den"), `full_name` the whole name ("Den._http._tcp.local").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceInstance {
    pub name: String,
    pub full_name: String,
    /// The service type it was found under, such as `_http._tcp.local`.
    pub service: String,
    /// The host the service runs on, from its SRV record.
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Addresses of the host, from address records in the same reply (CNAME chains followed).
    pub addresses: Vec<IpAddr>,
    pub txt: TxtRecord,
}

/// Copies the SRV, TXT and address details that `records` hold for an instance. Returns whether anything changed,
/// so applying the same reply twice reports no change the second time.
fn apply_details(inst: &mut ServiceInstance, records: &[Record]) -> bool {
    let before = inst.clone();
    let full = inst.full_name.clone();
    if let Some(srv) = records
        .iter()
        .find(|r| r.rtype == rtype::SRV && dnssd::same_name(&r.name, &full))
        .and_then(|r| dnssd::parse_srv(&r.rdata))
    {
        inst.port = Some(srv.port);
        inst.host = Some(srv.target.trim_end_matches('.').to_string());
    }
    if let Some(txt) = records
        .iter()
        .find(|r| r.rtype == rtype::TXT && dnssd::same_name(&r.name, &full))
        .and_then(|r| dnssd::parse_txt(&r.rdata))
    {
        inst.txt = txt;
    }
    if let Some(host) = inst.host.clone() {
        inst.addresses = dnssd::resolve_addresses(records, &host);
    }
    *inst != before
}

/// Browse of one service type. Feed it every reply; read `instances`.
#[derive(Clone, Debug)]
pub struct ServiceBrowser {
    service: String,
    instances: Vec<ServiceInstance>,
}

impl ServiceBrowser {
    /// Browses `kind` (such as `_http._tcp`) in `domain` (empty for the local domain). `None` for a bad type.
    pub fn new(kind: &str, domain: &str) -> Option<Self> {
        Some(ServiceBrowser { service: service_type_name(kind, domain)?, instances: Vec::new() })
    }

    /// The service type being browsed, such as `_http._tcp.local`.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The services found so far, in the order they first appeared.
    pub fn instances(&self) -> &[ServiceInstance] {
        &self.instances
    }

    /// The query packet that asks for every instance of the type: one PTR question with `id`.
    pub fn query(&self, id: u16) -> Result<Vec<u8>, WireError> {
        dnssd::query(id, &self.service, rtype::PTR)
    }

    /// Folds one received message into the results and returns how many instances were added, changed or removed.
    /// Queries are ignored. A PTR answer adds an instance (or removes it, with a zero TTL). SRV, TXT and address
    /// records in the answers or the additional section fill in the details of every known instance.
    pub fn handle_message(&mut self, msg: &Message) -> usize {
        if !msg.is_response() {
            return 0;
        }
        let mut changed = 0;
        let service = self.service.clone();
        let records: Vec<Record> = msg.answers.iter().chain(msg.additional.iter()).cloned().collect();
        for ptr in dnssd::answers_for(msg, &service, rtype::PTR) {
            let Some(full) = dnssd::parse_name_rdata(&ptr.rdata) else { continue };
            if ptr.ttl == 0 {
                let before = self.instances.len();
                self.instances.retain(|i| !dnssd::same_name(&i.full_name, &full));
                changed += before - self.instances.len();
                continue;
            }
            if self.instances.iter().any(|i| dnssd::same_name(&i.full_name, &full)) {
                continue;
            }
            let trimmed = full.trim_end_matches('.');
            let name = trimmed
                .strip_suffix(service.as_str())
                .and_then(|s| s.strip_suffix('.'))
                .unwrap_or(trimmed)
                .to_string();
            let mut inst = ServiceInstance {
                name,
                full_name: trimmed.to_string(),
                service: service.clone(),
                host: None,
                port: None,
                addresses: Vec::new(),
                txt: TxtRecord::new(),
            };
            apply_details(&mut inst, &records);
            self.instances.push(inst);
            changed += 1;
        }
        for inst in self.instances.iter_mut() {
            if apply_details(inst, &records) {
                changed += 1;
            }
        }
        changed
    }
}

/// Publication of one instance: the reply that announces it, and the test for a query that asks for it.
#[derive(Clone, Debug)]
pub struct ServiceAdvertiser {
    service: String,
    instance: String,
    host: String,
    port: u16,
    txt: TxtRecord,
    addresses: Vec<IpAddr>,
    ttl: u32,
}

impl ServiceAdvertiser {
    /// Publishes `label` (such as "Den") as the service `kind` in `domain`, on `host` (such as "den.local") at `port`.
    /// `None` for a bad type, an empty label or an empty host.
    pub fn new(label: &str, kind: &str, domain: &str, host: &str, port: u16, txt: TxtRecord, addresses: Vec<IpAddr>) -> Option<Self> {
        let service = service_type_name(kind, domain)?;
        let host = host.trim_end_matches('.');
        if label.is_empty() || host.is_empty() {
            return None;
        }
        Some(ServiceAdvertiser {
            instance: format!("{label}.{service}"),
            service,
            host: host.to_string(),
            port,
            txt,
            addresses,
            ttl: DEFAULT_TTL,
        })
    }

    /// The full instance name, such as `Den._http._tcp.local`.
    pub fn instance_name(&self) -> &str {
        &self.instance
    }

    /// The instance label alone, such as `Den`.
    pub fn label(&self) -> &str {
        self.instance.strip_suffix(&format!(".{}", self.service)).unwrap_or(&self.instance)
    }

    /// The same publication under another instance label, for a conflict rename. Everything else is kept.
    pub fn with_label(&self, label: &str) -> ServiceAdvertiser {
        let mut renamed = self.clone();
        renamed.instance = format!("{label}.{}", self.service);
        renamed
    }

    /// Sets the time to live of the published records, in seconds.
    pub fn set_ttl(&mut self, ttl: u32) {
        self.ttl = ttl;
    }

    /// The reply that announces the instance: a shared PTR in the answers, and the SRV, TXT and address records
    /// (unique, so with the cache-flush bit set) in the additional section.
    pub fn response(&self) -> Result<Vec<u8>, WireError> {
        let ptr = Record::new(&self.service, rtype::PTR, self.ttl, dnssd::name_rdata(&self.instance)?);
        let mut srv = Record::new(&self.instance, rtype::SRV, self.ttl, dnssd::srv_rdata(0, 0, self.port, &self.host)?);
        let mut txt = Record::new(&self.instance, rtype::TXT, self.ttl, dnssd::txt_rdata(&self.txt));
        srv.cache_flush = true;
        txt.cache_flush = true;
        let mut additional = vec![srv, txt];
        for ip in &self.addresses {
            let kind = if ip.is_ipv4() { rtype::A } else { rtype::AAAA };
            let mut addr = Record::new(&self.host, kind, self.ttl, dnssd::address_rdata(*ip));
            addr.cache_flush = true;
            additional.push(addr);
        }
        Message { flags: RESPONSE_FLAGS, answers: vec![ptr], additional, ..Message::default() }.encode()
    }

    /// True when a query asks for something this instance answers: its service type (PTR), the instance (SRV, TXT),
    /// or its host (A, AAAA). `ANY` questions match each of the three. Responses never match.
    pub fn matches_query(&self, msg: &Message) -> bool {
        if msg.is_response() {
            return false;
        }
        msg.questions.iter().any(|q| {
            let any = q.rtype == rtype::ANY;
            (dnssd::same_name(&q.name, &self.service) && (q.rtype == rtype::PTR || any))
                || (dnssd::same_name(&q.name, &self.instance) && (q.rtype == rtype::SRV || q.rtype == rtype::TXT || any))
                || (dnssd::same_name(&q.name, &self.host) && (q.rtype == rtype::A || q.rtype == rtype::AAAA || any))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn den_txt() -> TxtRecord {
        let mut txt = TxtRecord::new();
        txt.set("path", Some(&b"/x"[..])).unwrap();
        txt
    }

    fn den_addr() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))
    }

    /// A reply for `Den._http._tcp.local` on `den.local:8080`, with the PTR as an answer and the rest as additional.
    fn den_reply(ptr_ttl: u32) -> Message {
        Message {
            flags: 0x8400,
            answers: vec![Record::new("_http._tcp.local", rtype::PTR, ptr_ttl, dnssd::name_rdata("Den._http._tcp.local").unwrap())],
            additional: vec![
                Record::new("Den._http._tcp.local", rtype::SRV, 120, dnssd::srv_rdata(0, 0, 8080, "den.local").unwrap()),
                Record::new("Den._http._tcp.local", rtype::TXT, 120, dnssd::txt_rdata(&den_txt())),
                Record::new("den.local", rtype::A, 120, dnssd::address_rdata(den_addr())),
            ],
            ..Message::default()
        }
    }

    #[test]
    fn service_type_names_are_two_labels() {
        assert_eq!(service_type_name("_http._tcp", ""), Some("_http._tcp.local".to_string()));
        assert_eq!(service_type_name("_ipp._tcp.", "lab"), Some("_ipp._tcp.lab".to_string()));
        assert_eq!(service_type_name("http._tcp", ""), None);
        assert_eq!(service_type_name("_http._sctp", ""), None);
        assert_eq!(service_type_name("_http", ""), None);
        assert_eq!(service_type_name("_a._tcp._x", ""), None);
    }

    #[test]
    fn browse_query_asks_for_the_pointer_records_of_the_type() {
        let browser = ServiceBrowser::new("_http._tcp", "").unwrap();
        let msg = Message::decode(&browser.query(7).unwrap()).unwrap();
        assert_eq!(msg.id, 7);
        assert!(!msg.is_response());
        assert_eq!(msg.questions.len(), 1);
        // the wire message keeps the fully qualified name; the browser's own fields drop the root dot
        assert_eq!(msg.questions[0].name, "_http._tcp.local.");
        assert_eq!(msg.questions[0].rtype, rtype::PTR);
        assert_eq!(msg.questions[0].class, dnssd::CLASS_IN);
    }

    #[test]
    fn reply_fills_in_the_instance_details() {
        let mut browser = ServiceBrowser::new("_http._tcp", "").unwrap();
        assert_eq!(browser.handle_message(&den_reply(4500)), 1);
        let inst = &browser.instances()[0];
        assert_eq!(inst.name, "Den");
        assert_eq!(inst.full_name, "Den._http._tcp.local");
        assert_eq!(inst.host.as_deref(), Some("den.local"));
        assert_eq!(inst.port, Some(8080));
        assert_eq!(inst.addresses, vec![den_addr()]);
        assert_eq!(inst.txt, den_txt());
        // The same reply again changes nothing.
        assert_eq!(browser.handle_message(&den_reply(4500)), 0);
        assert_eq!(browser.instances().len(), 1);
    }

    #[test]
    fn a_zero_ttl_pointer_is_a_goodbye() {
        let mut browser = ServiceBrowser::new("_http._tcp", "").unwrap();
        browser.handle_message(&den_reply(4500));
        assert_eq!(browser.handle_message(&den_reply(0)), 1);
        assert!(browser.instances().is_empty());
    }

    #[test]
    fn queries_are_not_browse_results() {
        let mut browser = ServiceBrowser::new("_http._tcp", "").unwrap();
        let query = Message::decode(&browser.query(1).unwrap()).unwrap();
        assert_eq!(browser.handle_message(&query), 0);
        assert!(browser.instances().is_empty());
    }

    #[test]
    fn advertised_reply_is_found_by_a_browser() {
        let adv = ServiceAdvertiser::new("Den", "_http._tcp", "", "den.local.", 8080, den_txt(), vec![den_addr()]).unwrap();
        assert_eq!(adv.instance_name(), "Den._http._tcp.local");
        let msg = Message::decode(&adv.response().unwrap()).unwrap();
        assert!(msg.is_response());
        assert_eq!(msg.answers.len(), 1);
        assert_eq!(msg.additional.len(), 3);
        assert!(msg.additional[0].cache_flush);
        let mut browser = ServiceBrowser::new("_http._tcp", "local").unwrap();
        assert_eq!(browser.handle_message(&msg), 1);
        let inst = &browser.instances()[0];
        assert_eq!(inst.name, "Den");
        assert_eq!(inst.port, Some(8080));
        assert_eq!(inst.host.as_deref(), Some("den.local"));
        assert_eq!(inst.addresses, vec![den_addr()]);
        assert_eq!(inst.txt, den_txt());
    }

    #[test]
    fn advertiser_answers_matching_questions_only() {
        let adv = ServiceAdvertiser::new("Den", "_http._tcp", "", "den.local", 8080, den_txt(), vec![den_addr()]).unwrap();
        let ask = |name: &str, kind: u16| Message::decode(&dnssd::query(1, name, kind).unwrap()).unwrap();
        assert!(adv.matches_query(&ask("_http._tcp.local", rtype::PTR)));
        assert!(adv.matches_query(&ask("Den._http._tcp.local", rtype::SRV)));
        assert!(adv.matches_query(&ask("den.local", rtype::ANY)));
        assert!(!adv.matches_query(&ask("_ipp._tcp.local", rtype::PTR)));
        assert!(!adv.matches_query(&ask("_http._tcp.local", rtype::SRV)));
        let mut reply = ask("_http._tcp.local", rtype::PTR);
        reply.flags = 0x8400;
        assert!(!adv.matches_query(&reply));
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(ServiceAdvertiser::new("", "_http._tcp", "", "den.local", 80, TxtRecord::new(), vec![]).is_none());
        assert!(ServiceAdvertiser::new("Den", "_http._tcp", "", "", 80, TxtRecord::new(), vec![]).is_none());
        assert!(ServiceAdvertiser::new("Den", "http", "", "den.local", 80, TxtRecord::new(), vec![]).is_none());
        assert!(ServiceBrowser::new("_http", "").is_none());
    }

    #[test]
    fn registry_renames_a_conflicting_label() {
        let mut registry = ServiceRegistry::new();
        let first = ServiceAdvertiser::new("Den", "_http._tcp", "", "den.local", 80, TxtRecord::new(), vec![]).unwrap();
        assert_eq!(registry.register("app.one", first.clone()), "Den._http._tcp.local");
        assert_eq!(registry.register("app.two", first.clone()), "Den (2)._http._tcp.local");
        assert_eq!(registry.register("app.two", first), "Den (3)._http._tcp.local");
        assert_eq!(registry.entries().len(), 3);
    }

    #[test]
    fn registry_removes_one_owner_and_keeps_the_rest() {
        let mut registry = ServiceRegistry::new();
        let a = ServiceAdvertiser::new("Printer", "_ipp._tcp", "", "p.local", 631, TxtRecord::new(), vec![]).unwrap();
        let b = ServiceAdvertiser::new("Share", "_smb._tcp", "", "p.local", 445, TxtRecord::new(), vec![]).unwrap();
        registry.register("app.one", a);
        registry.register("app.two", b);
        assert_eq!(registry.for_owner("app.one").len(), 1);
        assert_eq!(registry.unregister_owner("app.one"), 1);
        assert!(registry.for_owner("app.one").is_empty());
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].1.label(), "Share");
    }

    #[test]
    fn with_label_keeps_the_service_type() {
        let a = ServiceAdvertiser::new("Den", "_http._tcp", "", "den.local", 80, TxtRecord::new(), vec![]).unwrap();
        let renamed = a.with_label("Den (2)");
        assert_eq!(renamed.label(), "Den (2)");
        assert_eq!(renamed.instance_name(), "Den (2)._http._tcp.local");
    }
}

/// Every service the shell publishes for the apps on this machine, one registry for the whole session. Each entry
/// records the app that owns it, so an app's services leave with that app. A label that another entry already holds
/// is renamed with a number, as RFC 6762 section 9 asks ("Den (2)"), so two publications never share a name.
#[derive(Clone, Debug, Default)]
pub struct ServiceRegistry {
    entries: Vec<(String, ServiceAdvertiser)>,
}

impl ServiceRegistry {
    /// An empty registry.
    pub fn new() -> ServiceRegistry {
        ServiceRegistry::default()
    }

    /// Publishes `advertiser` for `owner`. Returns the instance name it was published under, which may carry a number.
    pub fn register(&mut self, owner: &str, advertiser: ServiceAdvertiser) -> String {
        let base = advertiser.label().to_string();
        let mut published = advertiser.clone();
        let mut n = 1;
        while self.entries.iter().any(|(_, a)| a.instance_name() == published.instance_name()) {
            n += 1;
            published = advertiser.with_label(&format!("{base} ({n})"));
        }
        let name = published.instance_name().to_string();
        self.entries.push((owner.to_string(), published));
        name
    }

    /// Removes every service `owner` published. Returns how many were removed.
    pub fn unregister_owner(&mut self, owner: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(o, _)| o != owner);
        before - self.entries.len()
    }

    /// All published entries with their owners, in registration order.
    pub fn entries(&self) -> &[(String, ServiceAdvertiser)] {
        &self.entries
    }

    /// The publications one owner holds, in registration order.
    pub fn for_owner(&self, owner: &str) -> Vec<&ServiceAdvertiser> {
        self.entries.iter().filter(|(o, _)| o == owner).map(|(_, a)| a).collect()
    }

    /// The publications that answer a query message, for the caller to send their replies.
    pub fn answers(&self, query: &Message) -> Vec<&ServiceAdvertiser> {
        self.entries.iter().map(|(_, a)| a).filter(|a| a.matches_query(query)).collect()
    }
}

