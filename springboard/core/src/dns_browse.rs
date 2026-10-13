//! DNS-SD service types and the service browser (std only).
//!
//! A service type such as `_http._tcp` names a kind of service on the local network. A browser
//! for one type keeps the instances the network has announced (added or removed one reply at a
//! time) and records each instance's host, port and TXT record when it is resolved. This module
//! sends no query and reads no socket. The daemon layer feeds it the replies it received, and the
//! names it builds are the ones it would query for.

use crate::net_connection::Transport;
use crate::txt_record::TxtRecord;

/// Longest service name, without the leading underscore (RFC 6335).
const MAX_SERVICE_NAME: usize = 15;
/// Longest instance label, one length byte (RFC 1035).
const MAX_INSTANCE: usize = 63;
/// Domain used when none is given.
const DEFAULT_DOMAIN: &str = "local";

/// Why a service type string was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceTypeError {
    /// The string is not `_name._proto`: a missing underscore or dot.
    Format,
    /// The service name is empty, too long, or breaks the character rules.
    BadName,
    /// The protocol label is not `_tcp` or `_udp`.
    BadTransport,
}

impl std::fmt::Display for ServiceTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            ServiceTypeError::Format => "service type must look like _name._tcp",
            ServiceTypeError::BadName => "service name must be 1 to 15 letters, digits or hyphens",
            ServiceTypeError::BadTransport => "service protocol must be _tcp or _udp",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ServiceTypeError {}

/// A validated service type, stored in lower case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceType {
    name: String,
    transport: Transport,
}

impl ServiceType {
    /// Parses `_name._tcp` or `_name._udp`. The name is 1 to 15 bytes of letters, digits and
    /// hyphens. It may not start or end with a hyphen, contain two hyphens in a row, or lack a letter.
    pub fn parse(s: &str) -> Result<ServiceType, ServiceTypeError> {
        let (name, proto) = s.split_once('.').ok_or(ServiceTypeError::Format)?;
        let name = name.strip_prefix('_').ok_or(ServiceTypeError::Format)?;
        let transport = match proto {
            "_tcp" => Transport::Tcp,
            "_udp" => Transport::Udp,
            _ => return Err(ServiceTypeError::BadTransport),
        };
        let b = name.as_bytes();
        if b.is_empty() || b.len() > MAX_SERVICE_NAME {
            return Err(ServiceTypeError::BadName);
        }
        if !b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-') {
            return Err(ServiceTypeError::BadName);
        }
        if b[0] == b'-' || b[b.len() - 1] == b'-' || name.contains("--") {
            return Err(ServiceTypeError::BadName);
        }
        if !b.iter().any(|c| c.is_ascii_alphabetic()) {
            return Err(ServiceTypeError::BadName);
        }
        Ok(ServiceType { name: name.to_ascii_lowercase(), transport })
    }

    /// The service name without its underscore, in lower case.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The transport protocol the service uses.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// The DNS name the service type lives under, such as `_http._tcp.local.`.
    /// An empty domain means `local`, and a trailing dot on the domain is optional.
    pub fn dns_name(&self, domain: &str) -> String {
        let proto = match self.transport {
            Transport::Tcp => "_tcp",
            Transport::Udp => "_udp",
        };
        let d = domain.trim_end_matches('.');
        let d = if d.is_empty() { DEFAULT_DOMAIN } else { d };
        format!("_{}.{}.{}.", self.name, proto, d)
    }
}

/// One service instance the browser knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// The instance name as announced.
    pub name: String,
    /// The host the instance resolved to. `None` until it is resolved.
    pub host: Option<String>,
    /// The port the instance resolved to. `None` until it is resolved.
    pub port: Option<u16>,
    /// The TXT record of the instance. Empty until it is resolved.
    pub txt: TxtRecord,
}

impl Instance {
    /// True when both the host and the port are known.
    pub fn is_resolved(&self) -> bool {
        self.host.is_some() && self.port.is_some()
    }
}

/// A change to the set of instances, from one browse reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// An instance appeared. Carries its name as announced.
    Added(String),
    /// An instance went away. Carries the name it was stored under.
    Removed(String),
}

/// Why a browse reply was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseError {
    /// The instance name is empty.
    EmptyName,
    /// The instance name is longer than 63 bytes.
    NameTooLong,
}

/// Tracks the instances of one service type in one domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Browser {
    kind: ServiceType,
    domain: String,
    instances: Vec<Instance>,
}

impl Browser {
    /// A browser for `kind` in `domain`. An empty domain means `local`.
    pub fn new(kind: ServiceType, domain: &str) -> Self {
        let d = domain.trim_end_matches('.');
        let domain = if d.is_empty() { DEFAULT_DOMAIN.to_string() } else { d.to_string() };
        Browser { kind, domain, instances: Vec::new() }
    }

    /// The service type being browsed.
    pub fn kind(&self) -> &ServiceType {
        &self.kind
    }

    /// The domain being browsed, without a trailing dot.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The full DNS name of an instance, such as `Kitchen._http._tcp.local.`.
    pub fn full_name(&self, instance: &str) -> String {
        format!("{}.{}", instance, self.kind.dns_name(&self.domain))
    }

    /// Applies one browse reply. `added` is true when the instance appeared and false when it
    /// went away. Instance names compare without regard to case. A repeated add, or a removal of
    /// an unknown name, returns `Ok(None)` because nothing changed.
    pub fn browse_reply(&mut self, added: bool, name: &str) -> Result<Option<Change>, BrowseError> {
        if name.is_empty() {
            return Err(BrowseError::EmptyName);
        }
        if name.len() > MAX_INSTANCE {
            return Err(BrowseError::NameTooLong);
        }
        let pos = self.instances.iter().position(|i| i.name.eq_ignore_ascii_case(name));
        if added {
            if pos.is_some() {
                return Ok(None);
            }
            self.instances.push(Instance {
                name: name.to_string(),
                host: None,
                port: None,
                txt: TxtRecord::new(),
            });
            return Ok(Some(Change::Added(name.to_string())));
        }
        match pos {
            Some(i) => {
                let gone = self.instances.remove(i);
                Ok(Some(Change::Removed(gone.name)))
            }
            None => Ok(None),
        }
    }

    /// Applies a resolve reply: the host, port and TXT record of a known instance.
    /// Returns false when the instance is not in the browser.
    pub fn resolve_reply(&mut self, name: &str, host: &str, port: u16, txt: TxtRecord) -> bool {
        match self.instances.iter_mut().find(|i| i.name.eq_ignore_ascii_case(name)) {
            Some(i) => {
                i.host = Some(host.trim_end_matches('.').to_string());
                i.port = Some(port);
                i.txt = txt;
                true
            }
            None => false,
        }
    }

    /// The instance with this name, compared without regard to case.
    pub fn instance(&self, name: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.name.eq_ignore_ascii_case(name))
    }

    /// All known instances, sorted by name without regard to case.
    pub fn instances(&self) -> Vec<&Instance> {
        let mut v: Vec<&Instance> = self.instances.iter().collect();
        v.sort_by_key(|a| a.name.to_ascii_lowercase());
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_type_builds_its_dns_name() {
        let t = ServiceType::parse("_http._tcp").unwrap();
        assert_eq!(t.dns_name("local."), "_http._tcp.local.");
        assert_eq!(t.dns_name(""), "_http._tcp.local.");
        assert_eq!(t.transport(), Transport::Tcp);
        let u = ServiceType::parse("_HTTP._udp").unwrap();
        assert_eq!(u.name(), "http");
        assert_eq!(u.dns_name("example.test"), "_http._udp.example.test.");
    }

    #[test]
    fn service_type_refuses_bad_strings() {
        assert_eq!(ServiceType::parse("http._tcp"), Err(ServiceTypeError::Format));
        assert_eq!(ServiceType::parse("_http"), Err(ServiceTypeError::Format));
        assert_eq!(ServiceType::parse("_http.tcp"), Err(ServiceTypeError::BadTransport));
        assert_eq!(ServiceType::parse("_a--b._tcp"), Err(ServiceTypeError::BadName));
        assert_eq!(ServiceType::parse("_-x._tcp"), Err(ServiceTypeError::BadName));
        assert_eq!(ServiceType::parse("_1234._tcp"), Err(ServiceTypeError::BadName));
        assert_eq!(ServiceType::parse("_abcdefghijklmnop._tcp"), Err(ServiceTypeError::BadName));
    }

    #[test]
    fn browse_add_duplicate_and_remove() {
        let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "");
        assert_eq!(b.browse_reply(true, "Kitchen"), Ok(Some(Change::Added("Kitchen".to_string()))));
        // The same name in another case is already known.
        assert_eq!(b.browse_reply(true, "kitchen"), Ok(None));
        assert_eq!(b.browse_reply(false, "KITCHEN"), Ok(Some(Change::Removed("Kitchen".to_string()))));
        assert_eq!(b.browse_reply(false, "KITCHEN"), Ok(None));
        assert!(b.instances().is_empty());
    }

    #[test]
    fn browse_refuses_empty_and_long_names() {
        let mut b = Browser::new(ServiceType::parse("_ipp._tcp").unwrap(), "local");
        assert_eq!(b.browse_reply(true, ""), Err(BrowseError::EmptyName));
        let long = "x".repeat(64);
        assert_eq!(b.browse_reply(true, &long), Err(BrowseError::NameTooLong));
        let max = "x".repeat(63);
        assert_eq!(b.browse_reply(true, &max), Ok(Some(Change::Added(max.clone()))));
    }

    #[test]
    fn resolve_fills_host_port_and_txt() {
        let mut b = Browser::new(ServiceType::parse("_ipp._tcp").unwrap(), "local.");
        b.browse_reply(true, "Printer").unwrap();
        let mut txt = TxtRecord::new();
        txt.set("ty", Some("Laser".as_bytes())).unwrap();
        assert!(b.resolve_reply("printer", "printer.local.", 631, txt.clone()));
        let p = b.instance("PRINTER").unwrap();
        assert!(p.is_resolved());
        assert_eq!(p.host.as_deref(), Some("printer.local"));
        assert_eq!(p.port, Some(631));
        assert_eq!(p.txt, txt);
        // A resolve for an instance that was never browsed changes nothing.
        assert!(!b.resolve_reply("Scanner", "scanner.local.", 80, TxtRecord::new()));
        assert!(b.instance("Scanner").is_none());
    }

    #[test]
    fn instances_sort_without_regard_to_case_and_full_name_is_built() {
        let mut b = Browser::new(ServiceType::parse("_http._tcp").unwrap(), "local");
        b.browse_reply(true, "beta").unwrap();
        b.browse_reply(true, "Alpha").unwrap();
        b.browse_reply(true, "gamma").unwrap();
        let names: Vec<&str> = b.instances().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "beta", "gamma"]);
        assert_eq!(b.full_name("Alpha"), "Alpha._http._tcp.local.");
        assert_eq!(b.domain(), "local");
    }
}
