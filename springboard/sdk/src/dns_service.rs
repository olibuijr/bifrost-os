//! Service names and TXT records for DNS-based service discovery (RFC 6763).
//!
//! A `ServiceType` such as `_http._tcp` names a kind of service. A `ServiceInstance` is one named copy
//! of it, with a port and TXT key/value pairs. This module builds the full names and reads and writes
//! the TXT wire format. It sends and receives no packets: the multicast transport that carries these
//! records is not part of this model.

use std::fmt;

/// The domain that local services are published in.
pub const DEFAULT_DOMAIN: &str = "local.";
/// Longest instance name in bytes.
pub const MAX_INSTANCE_LEN: usize = 63;
/// Longest TXT entry (`key=value`) in bytes.
pub const MAX_TXT_ENTRY: usize = 255;
/// Longest TXT key in bytes.
pub const MAX_TXT_KEY: usize = 9;

/// A name or entry that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    BadServiceType(String),
    BadInstanceName,
    BadKey(String),
    EntryTooLong(String),
    Truncated,
    BadUtf8,
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::BadServiceType(t) => write!(f, "service type \"{t}\" is not valid"),
            ServiceError::BadInstanceName => write!(f, "instance name must be 1 to {MAX_INSTANCE_LEN} bytes"),
            ServiceError::BadKey(k) => write!(f, "TXT key \"{k}\" is not valid"),
            ServiceError::EntryTooLong(k) => write!(f, "TXT entry \"{k}\" is longer than {MAX_TXT_ENTRY} bytes"),
            ServiceError::Truncated => write!(f, "TXT data ends inside an entry"),
            ServiceError::BadUtf8 => write!(f, "TXT value is not UTF-8"),
        }
    }
}

/// The transport part of a service type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Tcp,
    Udp,
}

impl Transport {
    /// The label as it appears in a name, such as `_tcp`.
    pub fn label(self) -> &'static str {
        match self {
            Transport::Tcp => "_tcp",
            Transport::Udp => "_udp",
        }
    }
}

/// A label of a service name: 1 to 15 bytes of letters, digits and single hyphens, with at least one
/// letter and no leading or trailing hyphen.
fn valid_service_label(label: &str) -> bool {
    let b = label.as_bytes();
    !b.is_empty()
        && b.len() <= 15
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
        && !label.contains("--")
        && b.iter().any(|c| c.is_ascii_alphabetic())
}

fn bad_type(text: &str) -> ServiceError {
    ServiceError::BadServiceType(text.to_string())
}

/// A kind of service, such as `_http._tcp`. Names are stored lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceType {
    name: String,
    transport: Transport,
}

impl ServiceType {
    /// Reads `_name._tcp` or `_name._udp`. A trailing dot is allowed.
    pub fn parse(text: &str) -> Result<ServiceType, ServiceError> {
        let t = text.trim().trim_end_matches('.');
        let mut parts = t.split('.');
        let label = parts
            .next()
            .and_then(|p| p.strip_prefix('_'))
            .ok_or_else(|| bad_type(text))?;
        if !valid_service_label(label) {
            return Err(bad_type(text));
        }
        let transport_text = parts.next().map(|p| p.to_ascii_lowercase());
        let transport = match transport_text.as_deref() {
            Some("_tcp") => Transport::Tcp,
            Some("_udp") => Transport::Udp,
            _ => return Err(bad_type(text)),
        };
        if parts.next().is_some() {
            return Err(bad_type(text));
        }
        Ok(ServiceType {
            name: format!("_{}", label.to_ascii_lowercase()),
            transport,
        })
    }

    /// The service label, with its underscore, such as `_http`.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// The full name in a domain, such as `_http._tcp.local.`.
    pub fn domain_name(&self, domain: &str) -> String {
        format!("{}.{}.{}.", self.name, self.transport.label(), domain.trim_end_matches('.'))
    }
}

impl fmt::Display for ServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.name, self.transport.label())
    }
}

/// An instance name is 1 to 63 bytes and not only spaces.
fn validate_instance_name(name: &str) -> Result<(), ServiceError> {
    if name.trim().is_empty() || name.len() > MAX_INSTANCE_LEN {
        return Err(ServiceError::BadInstanceName);
    }
    Ok(())
}

/// Dots and backslashes in an instance name are escaped with a backslash in the full name.
fn escape_label(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c == '.' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A TXT key: 1 to 9 printable ASCII bytes, without `=`.
fn valid_key_bytes(key: &[u8]) -> bool {
    !key.is_empty() && key.len() <= MAX_TXT_KEY && key.iter().all(|&b| (b' '..=b'~').contains(&b) && b != b'=')
}

/// Flattens key/value pairs into TXT data: one length byte and `key=value` per entry. Empty input
/// gives the single zero byte that means an empty record.
pub fn encode_txt(pairs: &[(&str, &str)]) -> Result<Vec<u8>, ServiceError> {
    if pairs.is_empty() {
        return Ok(vec![0]);
    }
    let mut out = Vec::new();
    for &(key, value) in pairs {
        if !valid_key_bytes(key.as_bytes()) {
            return Err(ServiceError::BadKey(key.to_string()));
        }
        let len = key.len() + 1 + value.len();
        if len > MAX_TXT_ENTRY {
            return Err(ServiceError::EntryTooLong(key.to_string()));
        }
        out.push(len as u8);
        out.extend_from_slice(key.as_bytes());
        out.push(b'=');
        out.extend_from_slice(value.as_bytes());
    }
    Ok(out)
}

/// Reads TXT data into `(key, value)` pairs. A key with no `=` has no value (`None`). Entries with an
/// invalid key are skipped, and a repeated key keeps its first entry, ignoring case.
pub fn decode_txt(data: &[u8]) -> Result<Vec<(String, Option<String>)>, ServiceError> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let len = data[i] as usize;
        i += 1;
        if i + len > data.len() {
            return Err(ServiceError::Truncated);
        }
        let entry = &data[i..i + len];
        i += len;
        if entry.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match entry.iter().position(|&b| b == b'=') {
            Some(p) => (&entry[..p], Some(&entry[p + 1..])),
            None => (entry, None),
        };
        if !valid_key_bytes(raw_key) {
            continue;
        }
        let key = String::from_utf8_lossy(raw_key).into_owned();
        let value = match raw_value {
            Some(v) => Some(String::from_utf8(v.to_vec()).map_err(|_| ServiceError::BadUtf8)?),
            None => None,
        };
        if out.iter().any(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            continue;
        }
        out.push((key, value));
    }
    Ok(out)
}

/// One named service on the network: a type, a port and TXT pairs, in the default domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstance {
    name: String,
    service: ServiceType,
    domain: String,
    port: u16,
    txt: Vec<(String, Option<String>)>,
}

impl ServiceInstance {
    pub fn new(name: &str, service: ServiceType, port: u16) -> Result<ServiceInstance, ServiceError> {
        validate_instance_name(name)?;
        Ok(ServiceInstance {
            name: name.to_string(),
            service,
            domain: DEFAULT_DOMAIN.to_string(),
            port,
            txt: Vec::new(),
        })
    }

    /// Replaces the TXT pairs. They go through the wire format, so the stored pairs are what a
    /// reader of the record would see.
    pub fn set_txt(&mut self, pairs: &[(&str, &str)]) -> Result<(), ServiceError> {
        let wire = encode_txt(pairs)?;
        self.txt = decode_txt(&wire)?;
        Ok(())
    }

    /// The full name, such as `Office._http._tcp.local.`, with dots in the instance name escaped.
    pub fn fqdn(&self) -> String {
        format!("{}.{}", escape_label(&self.name), self.service.domain_name(&self.domain))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn service(&self) -> &ServiceType {
        &self.service
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn txt(&self) -> &[(String, Option<String>)] {
        &self.txt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http() -> ServiceType {
        ServiceType::parse("_http._tcp").unwrap()
    }

    #[test]
    fn a_service_type_is_read_lowercase_with_its_transport() {
        let t = ServiceType::parse("_HTTP._TCP.").unwrap();
        assert_eq!(t.name(), "_http");
        assert_eq!(t.transport(), Transport::Tcp);
        assert_eq!(t.to_string(), "_http._tcp");
        assert_eq!(t.domain_name("local"), "_http._tcp.local.");
        assert_eq!(t.domain_name("local."), "_http._tcp.local.");
        assert_eq!(ServiceType::parse("_dns._udp").unwrap().transport(), Transport::Udp);
    }

    #[test]
    fn a_service_type_rejects_bad_labels_transports_and_extra_parts() {
        for bad in ["http._tcp", "_http._sctp", "_a-._tcp", "_a--b._tcp", "_123._tcp", "_http._tcp.local", "_http", ""] {
            assert!(ServiceType::parse(bad).is_err(), "{bad} should be refused");
        }
        assert!(ServiceType::parse("_abcdefghijklmnop._tcp").is_err(), "16 bytes is over the 15 limit");
    }

    #[test]
    fn txt_encodes_length_prefixed_entries_in_order() {
        let wire = encode_txt(&[("txtvers", "1"), ("path", "/x")]).unwrap();
        let mut want = vec![9u8];
        want.extend_from_slice(b"txtvers=1");
        want.push(7);
        want.extend_from_slice(b"path=/x");
        assert_eq!(wire, want);
        assert_eq!(encode_txt(&[]).unwrap(), vec![0u8]);
    }

    #[test]
    fn txt_decodes_flag_keys_and_skips_empty_and_invalid_entries() {
        // "a=b", "x", an empty entry, "=q" (no key, skipped), then "e=z"
        let data = [3u8, b'a', b'=', b'b', 1, b'x', 0, 2, b'=', b'q', 3, b'e', b'=', b'z'];
        let got = decode_txt(&data).unwrap();
        assert_eq!(
            got,
            vec![("a".to_string(), Some("b".to_string())), ("x".to_string(), None), ("e".to_string(), Some("z".to_string()))]
        );
        assert!(decode_txt(&[0]).unwrap().is_empty());
    }

    #[test]
    fn txt_keeps_the_first_of_a_repeated_key_ignoring_case() {
        let data = [3u8, b'a', b'=', b'1', 3, b'A', b'=', b'2'];
        assert_eq!(decode_txt(&data).unwrap(), vec![("a".to_string(), Some("1".to_string()))]);
    }

    #[test]
    fn txt_refuses_bad_keys_long_entries_and_truncated_data() {
        assert_eq!(encode_txt(&[("a=b", "1")]), Err(ServiceError::BadKey("a=b".to_string())));
        assert_eq!(encode_txt(&[("toolongkey1", "1")]), Err(ServiceError::BadKey("toolongkey1".to_string())));
        let value254 = "v".repeat(254);
        assert_eq!(encode_txt(&[("k", value254.as_str())]), Err(ServiceError::EntryTooLong("k".to_string())));
        let value253 = "v".repeat(253);
        assert!(encode_txt(&[("k", value253.as_str())]).is_ok());
        assert_eq!(decode_txt(&[5, b'a']), Err(ServiceError::Truncated));
    }

    #[test]
    fn an_instance_has_an_escaped_full_name_in_the_default_domain() {
        let office = ServiceInstance::new("Office.v2", http(), 631).unwrap();
        assert_eq!(office.fqdn(), "Office\\.v2._http._tcp.local.");
        assert_eq!(office.domain(), "local.");
        assert_eq!(office.port(), 631);
        assert_eq!(ServiceInstance::new("", http(), 1), Err(ServiceError::BadInstanceName));
        assert_eq!(ServiceInstance::new(&"n".repeat(64), http(), 1), Err(ServiceError::BadInstanceName));
        assert!(ServiceInstance::new(&"n".repeat(63), http(), 1).is_ok());
    }

    #[test]
    fn an_instance_stores_its_txt_as_the_wire_format_reads_it() {
        let mut printer = ServiceInstance::new("Hall", http(), 80).unwrap();
        printer.set_txt(&[("Path", "/ipp"), ("path", "/other"), ("ready", "")]).unwrap();
        assert_eq!(
            printer.txt(),
            &[("Path".to_string(), Some("/ipp".to_string())), ("ready".to_string(), Some(String::new()))]
        );
        assert!(printer.set_txt(&[("bad=key", "1")]).is_err());
    }
}
