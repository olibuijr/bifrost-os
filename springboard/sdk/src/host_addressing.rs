//! Host name addressing: the app-facing model for resolving a host name to its addresses.
//!
//! An app builds a `HostLookup` for a name and an address family. A numeric address resolves at once,
//! with no query. A name starts `Idle`; the caller starts it, asks its resolver, and reports the answer
//! through `finish` or `fail`. The model is pure std and sends no queries itself. The shell's resolver
//! stays internal (springboard/src/deildu/dns.rs) until the shell feeds answers in through the SDK.

use std::fmt;
use std::net::IpAddr;

/// Longest host name in bytes, without the trailing dot.
pub const MAX_NAME_LEN: usize = 253;
/// Longest single label of a host name in bytes.
pub const MAX_LABEL_LEN: usize = 63;

const NO_ADDRESS: &str = "no address of the requested family";

/// Which address families an answer may contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Any,
    Ipv4,
    Ipv6,
}

/// A name that cannot be looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    Empty,
    TooLong,
    BadLabel(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Empty => write!(f, "host name is empty"),
            HostError::TooLong => write!(f, "host name is longer than {MAX_NAME_LEN} bytes"),
            HostError::BadLabel(label) => write!(f, "host name label \"{label}\" is not valid"),
        }
    }
}

/// Trims spaces, drops one trailing dot and lowercases a name. Each label must be 1 to 63 bytes of
/// letters, digits, hyphen or underscore.
pub fn normalize_name(name: &str) -> Result<String, HostError> {
    let trimmed = name.trim();
    let body = trimmed.strip_suffix('.').unwrap_or(trimmed);
    if body.is_empty() {
        return Err(HostError::Empty);
    }
    if body.len() > MAX_NAME_LEN {
        return Err(HostError::TooLong);
    }
    let lower = body.to_ascii_lowercase();
    for label in lower.split('.') {
        let ok = !label.is_empty()
            && label.len() <= MAX_LABEL_LEN
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(HostError::BadLabel(label.to_string()));
        }
    }
    Ok(lower)
}

/// The address written as a number, such as `192.0.2.1` or `2001:db8::1` (brackets allowed for IPv6).
/// Returns `None` for a name, which needs a lookup.
pub fn literal(name: &str) -> Option<IpAddr> {
    let t = name.trim();
    let t = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(t);
    t.parse::<IpAddr>().ok()
}

/// The addresses of one family, IPv4 first, without repeats. Input order is kept inside each family.
pub fn order(addrs: &[IpAddr], family: Family) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::with_capacity(addrs.len());
    for &want_v4 in &[true, false] {
        for ip in addrs {
            let family_ok = match family {
                Family::Any => true,
                Family::Ipv4 => ip.is_ipv4(),
                Family::Ipv6 => ip.is_ipv6(),
            };
            if family_ok && ip.is_ipv4() == want_v4 && !out.contains(ip) {
                out.push(*ip);
            }
        }
    }
    out
}

/// Where a lookup stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Resolving,
    Resolved,
    Failed(String),
}

/// One lookup of a host name, or of a numeric address that needs no query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLookup {
    name: String,
    family: Family,
    state: State,
    addresses: Vec<IpAddr>,
}

impl HostLookup {
    /// A numeric address resolves at once. A name is normalized and starts `Idle`.
    pub fn new(name: &str, family: Family) -> Result<HostLookup, HostError> {
        if let Some(ip) = literal(name) {
            let addresses = order(&[ip], family);
            let state = if addresses.is_empty() {
                State::Failed(NO_ADDRESS.to_string())
            } else {
                State::Resolved
            };
            return Ok(HostLookup { name: ip.to_string(), family, state, addresses });
        }
        Ok(HostLookup {
            name: normalize_name(name)?,
            family,
            state: State::Idle,
            addresses: Vec::new(),
        })
    }

    /// `Idle` to `Resolving`. Returns false in any other state.
    pub fn start(&mut self) -> bool {
        if self.state != State::Idle {
            return false;
        }
        self.state = State::Resolving;
        true
    }

    /// `Resolving` to `Resolved` with the addresses of the family, or to `Failed` when none match.
    pub fn finish(&mut self, found: &[IpAddr]) -> bool {
        if self.state != State::Resolving {
            return false;
        }
        self.addresses = order(found, self.family);
        self.state = if self.addresses.is_empty() {
            State::Failed(NO_ADDRESS.to_string())
        } else {
            State::Resolved
        };
        true
    }

    /// `Resolving` to `Failed` with the reason the resolver gave.
    pub fn fail(&mut self, reason: &str) -> bool {
        if self.state != State::Resolving {
            return false;
        }
        self.addresses.clear();
        self.state = State::Failed(reason.to_string());
        true
    }

    /// `Resolving` back to `Idle`, dropping any partial answer.
    pub fn cancel(&mut self) -> bool {
        if self.state != State::Resolving {
            return false;
        }
        self.addresses.clear();
        self.state = State::Idle;
        true
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn family(&self) -> Family {
        self.family
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn addresses(&self) -> &[IpAddr] {
        &self.addresses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn normalize_trims_drops_the_trailing_dot_and_lowercases() {
        assert_eq!(normalize_name("  Example.COM. ").unwrap(), "example.com");
        assert_eq!(normalize_name("a_b-1.example").unwrap(), "a_b-1.example");
        assert_eq!(normalize_name(""), Err(HostError::Empty));
        assert_eq!(normalize_name("."), Err(HostError::Empty));
        assert_eq!(normalize_name("a..b"), Err(HostError::BadLabel(String::new())));
        assert_eq!(normalize_name("bad label.com"), Err(HostError::BadLabel("bad label".to_string())));
    }

    #[test]
    fn normalize_limits_label_and_name_length() {
        let label64 = "a".repeat(64);
        assert_eq!(normalize_name(&label64), Err(HostError::BadLabel(label64.clone())));
        let label63 = "a".repeat(63);
        assert_eq!(normalize_name(&label63).unwrap(), label63);
        // four 63-byte labels plus dots is 255 bytes, over the 253 limit
        let long = [label63.as_str(), label63.as_str(), label63.as_str(), label63.as_str()].join(".");
        assert_eq!(normalize_name(&long), Err(HostError::TooLong));
    }

    #[test]
    fn literal_reads_numbers_and_leaves_names_alone() {
        assert_eq!(literal("192.0.2.1"), Some(ip("192.0.2.1")));
        assert_eq!(literal("[2001:db8::1]"), Some(ip("2001:db8::1")));
        assert_eq!(literal("example.com"), None);
        assert_eq!(literal("256.0.0.1"), None);
    }

    #[test]
    fn order_puts_ipv4_first_removes_repeats_and_filters_by_family() {
        let found = [ip("2001:db8::1"), ip("192.0.2.1"), ip("192.0.2.1"), ip("2001:db8::1"), ip("192.0.2.9")];
        assert_eq!(
            order(&found, Family::Any),
            vec![ip("192.0.2.1"), ip("192.0.2.9"), ip("2001:db8::1")]
        );
        assert_eq!(order(&found, Family::Ipv6), vec![ip("2001:db8::1")]);
        assert_eq!(order(&found, Family::Ipv4), vec![ip("192.0.2.1"), ip("192.0.2.9")]);
        assert!(order(&[ip("192.0.2.1")], Family::Ipv6).is_empty());
    }

    #[test]
    fn a_numeric_address_resolves_at_once_or_fails_for_the_wrong_family() {
        let lookup = HostLookup::new("10.0.0.5", Family::Any).unwrap();
        assert_eq!(lookup.state(), &State::Resolved);
        assert_eq!(lookup.addresses(), &[ip("10.0.0.5")]);
        let wrong = HostLookup::new("10.0.0.5", Family::Ipv6).unwrap();
        assert_eq!(wrong.state(), &State::Failed(NO_ADDRESS.to_string()));
        assert!(wrong.addresses().is_empty());
    }

    #[test]
    fn a_name_moves_from_idle_through_resolving_to_resolved() {
        let mut lookup = HostLookup::new("Example.org.", Family::Any).unwrap();
        assert_eq!(lookup.name(), "example.org");
        assert_eq!(lookup.state(), &State::Idle);
        assert!(!lookup.finish(&[ip("192.0.2.7")]), "finish needs a started lookup");
        assert!(lookup.start());
        assert!(!lookup.start(), "a second start is refused");
        assert!(lookup.finish(&[ip("192.0.2.7")]));
        assert_eq!(lookup.state(), &State::Resolved);
        assert_eq!(lookup.addresses(), &[ip("192.0.2.7")]);
        assert!(!lookup.cancel(), "a finished lookup cannot be cancelled");
    }

    #[test]
    fn an_empty_answer_fails_and_a_cancel_returns_to_idle() {
        let mut empty = HostLookup::new("example.org", Family::Ipv6).unwrap();
        assert!(empty.start());
        assert!(empty.finish(&[ip("192.0.2.7")]));
        assert_eq!(empty.state(), &State::Failed(NO_ADDRESS.to_string()));

        let mut cancelled = HostLookup::new("example.org", Family::Any).unwrap();
        assert!(cancelled.start());
        assert!(cancelled.cancel());
        assert_eq!(cancelled.state(), &State::Idle);

        let mut failed = HostLookup::new("example.org", Family::Any).unwrap();
        assert!(failed.start());
        assert!(failed.fail("timed out"));
        assert_eq!(failed.state(), &State::Failed("timed out".to_string()));
    }
}
