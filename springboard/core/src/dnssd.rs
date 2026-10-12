//! DNS-SD domain enumeration, DNS message wire format and record data (std only).
//!
//! This is the pure model behind a service discovery API. It enumerates the browse and
//! registration domains from a stream of replies, encodes and decodes DNS messages (the
//! wire format shared by unicast DNS and multicast DNS), builds the record data for
//! PTR, SRV, TXT and address records, and follows a CNAME chain to find addresses.
//! It opens no socket and sends no query: the daemon layer hands it the bytes it
//! received and sends the bytes this module encodes.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::txt_record::TxtRecord;

/// Reply flag: more replies follow in this batch.
pub const FLAG_MORE_COMING: u32 = 0x1;
/// Reply flag: the named item was added. Without it the item was removed.
pub const FLAG_ADD: u32 = 0x2;
/// Reply flag: the item is the default of its kind.
pub const FLAG_DEFAULT: u32 = 0x4;
/// Reply flag: the domain is a browse domain.
pub const FLAG_BROWSE_DOMAINS: u32 = 0x40;
/// Reply flag: the domain is a registration domain.
pub const FLAG_REGISTRATION_DOMAINS: u32 = 0x80;

/// Interface index that lets the system choose the interface.
pub const INTERFACE_ANY: u32 = 0;
/// Interface index that limits a lookup to the local host.
pub const INTERFACE_LOCAL_ONLY: u32 = u32::MAX;
/// Interface index that asks for a unicast lookup.
pub const INTERFACE_UNICAST: u32 = u32::MAX - 1;

/// Longest label (one length byte, so at most 63 bytes).
const MAX_LABEL: usize = 63;
/// Longest encoded name, including the root byte.
const MAX_NAME: usize = 255;
/// CNAME hops followed before giving up.
const MAX_CNAME_HOPS: usize = 8;
/// Pointer jumps allowed while reading one name.
const MAX_JUMPS: usize = 32;
/// Top bit of a response flags word.
const FLAG_RESPONSE: u16 = 0x8000;
/// Top bit of a record class: the cache-flush bit.
const CACHE_FLUSH: u16 = 0x8000;
/// Class IN, the only class used for DNS-SD.
pub const CLASS_IN: u16 = 1;

/// Record types used by DNS-SD.
pub mod rtype {
    pub const A: u16 = 1;
    pub const NS: u16 = 2;
    pub const CNAME: u16 = 5;
    pub const PTR: u16 = 12;
    pub const TXT: u16 = 16;
    pub const AAAA: u16 = 28;
    pub const SRV: u16 = 33;
    pub const ANY: u16 = 255;
}

/// Error codes the DNS-SD interface reports, by their C names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsServiceError {
    AwdlTimeout,
    AlreadyRegistered,
    BadFlags,
    BadInterfaceIndex,
    BadKey,
    BadParam,
    BadReference,
    BadSig,
    BadState,
    BadTime,
    DefunctConnection,
    DoubleNAT,
    Firewall,
    Incompatible,
    Invalid,
    NatPortMappingDisabled,
    NatPortMappingUnsupported,
    NatTraversal,
    NameConflict,
    NoAuth,
    NoError,
    NoMemory,
    NoRouter,
    NoSuchKey,
    NoSuchName,
    NoSuchRecord,
    NotInitialized,
    NotPermitted,
    PolicyDenied,
    PollingMode,
    Refused,
    ServiceNotRunning,
    StaleData,
    Timeout,
    Transient,
    Unknown,
    Unsupported,
}

impl DnsServiceError {
    /// Every error code, in the order the reference lists them.
    pub const ALL: [DnsServiceError; 37] = [
        Self::AwdlTimeout,
        Self::AlreadyRegistered,
        Self::BadFlags,
        Self::BadInterfaceIndex,
        Self::BadKey,
        Self::BadParam,
        Self::BadReference,
        Self::BadSig,
        Self::BadState,
        Self::BadTime,
        Self::DefunctConnection,
        Self::DoubleNAT,
        Self::Firewall,
        Self::Incompatible,
        Self::Invalid,
        Self::NatPortMappingDisabled,
        Self::NatPortMappingUnsupported,
        Self::NatTraversal,
        Self::NameConflict,
        Self::NoAuth,
        Self::NoError,
        Self::NoMemory,
        Self::NoRouter,
        Self::NoSuchKey,
        Self::NoSuchName,
        Self::NoSuchRecord,
        Self::NotInitialized,
        Self::NotPermitted,
        Self::PolicyDenied,
        Self::PollingMode,
        Self::Refused,
        Self::ServiceNotRunning,
        Self::StaleData,
        Self::Timeout,
        Self::Transient,
        Self::Unknown,
        Self::Unsupported,
    ];

    /// The part of the C constant name after `kDNSServiceErr_`.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::AwdlTimeout => "AWDLTimeout",
            Self::AlreadyRegistered => "AlreadyRegistered",
            Self::BadFlags => "BadFlags",
            Self::BadInterfaceIndex => "BadInterfaceIndex",
            Self::BadKey => "BadKey",
            Self::BadParam => "BadParam",
            Self::BadReference => "BadReference",
            Self::BadSig => "BadSig",
            Self::BadState => "BadState",
            Self::BadTime => "BadTime",
            Self::DefunctConnection => "DefunctConnection",
            Self::DoubleNAT => "DoubleNAT",
            Self::Firewall => "Firewall",
            Self::Incompatible => "Incompatible",
            Self::Invalid => "Invalid",
            Self::NatPortMappingDisabled => "NATPortMappingDisabled",
            Self::NatPortMappingUnsupported => "NATPortMappingUnsupported",
            Self::NatTraversal => "NATTraversal",
            Self::NameConflict => "NameConflict",
            Self::NoAuth => "NoAuth",
            Self::NoError => "NoError",
            Self::NoMemory => "NoMemory",
            Self::NoRouter => "NoRouter",
            Self::NoSuchKey => "NoSuchKey",
            Self::NoSuchName => "NoSuchName",
            Self::NoSuchRecord => "NoSuchRecord",
            Self::NotInitialized => "NotInitialized",
            Self::NotPermitted => "NotPermitted",
            Self::PolicyDenied => "PolicyDenied",
            Self::PollingMode => "PollingMode",
            Self::Refused => "Refused",
            Self::ServiceNotRunning => "ServiceNotRunning",
            Self::StaleData => "StaleData",
            Self::Timeout => "Timeout",
            Self::Transient => "Transient",
            Self::Unknown => "Unknown",
            Self::Unsupported => "Unsupported",
        }
    }
}

impl fmt::Display for DnsServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "kDNSServiceErr_{}", self.suffix())
    }
}

/// Which list a domain belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainKind {
    Browse,
    Registration,
}

/// One domain in the enumerated lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainEntry {
    pub kind: DomainKind,
    pub name: String,
    pub is_default: bool,
}

/// The browse and registration domains built up from enumeration replies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainEnumeration {
    entries: Vec<DomainEntry>,
    more_coming: bool,
}

impl DomainEnumeration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one reply. Returns false when the reply names neither list.
    /// An added default replaces the default of the same kind.
    pub fn apply(&mut self, flags: u32, domain: &str) -> bool {
        let kind = if flags & FLAG_BROWSE_DOMAINS != 0 {
            DomainKind::Browse
        } else if flags & FLAG_REGISTRATION_DOMAINS != 0 {
            DomainKind::Registration
        } else {
            return false;
        };
        self.more_coming = flags & FLAG_MORE_COMING != 0;
        let is_default = flags & FLAG_DEFAULT != 0;
        let pos = self
            .entries
            .iter()
            .position(|e| e.kind == kind && same_name(&e.name, domain));
        if flags & FLAG_ADD != 0 {
            if is_default {
                for e in self.entries.iter_mut().filter(|e| e.kind == kind) {
                    e.is_default = false;
                }
            }
            match pos {
                Some(i) => self.entries[i].is_default = is_default,
                None => self.entries.push(DomainEntry {
                    kind,
                    name: domain.to_string(),
                    is_default,
                }),
            }
        } else if let Some(i) = pos {
            self.entries.remove(i);
        }
        true
    }

    /// Names of one kind, in the order they were added.
    pub fn domains(&self, kind: DomainKind) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.name.as_str())
            .collect()
    }

    /// The default domain of one kind, if one has been added.
    pub fn default_domain(&self, kind: DomainKind) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.kind == kind && e.is_default)
            .map(|e| e.name.as_str())
    }

    /// True once the last reply of the batch has arrived.
    pub fn is_complete(&self) -> bool {
        !self.more_coming
    }
}

/// Why a name, message or record could not be encoded or decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// An empty label, a label over 63 bytes, or a name over 255 bytes.
    BadName,
    /// The data ends before the field does.
    Truncated,
    /// A compression pointer is out of range, reserved, or loops.
    BadPointer,
    /// A count or record length does not fit its 16-bit field.
    TooLong,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            WireError::BadName => "bad domain name",
            WireError::Truncated => "message is truncated",
            WireError::BadPointer => "bad name compression pointer",
            WireError::TooLong => "value does not fit its field",
        };
        f.write_str(text)
    }
}

impl std::error::Error for WireError {}

/// A question: the name, record type and class being asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
}

/// A resource record. `rdata` is self-contained: decoded names are not compressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
    pub cache_flush: bool,
    pub ttl: u32,
    pub rdata: Vec<u8>,
}

impl Record {
    /// A class IN record with no cache-flush bit.
    pub fn new(name: &str, rtype: u16, ttl: u32, rdata: Vec<u8>) -> Self {
        Record {
            name: name.to_string(),
            rtype,
            class: CLASS_IN,
            cache_flush: false,
            ttl,
            rdata,
        }
    }
}

/// A whole DNS message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authority: Vec<Record>,
    pub additional: Vec<Record>,
}

impl Message {
    pub fn is_response(&self) -> bool {
        self.flags & FLAG_RESPONSE != 0
    }

    /// The low four bits of the flags word (0 is no error).
    pub fn rcode(&self) -> u8 {
        (self.flags & 0xF) as u8
    }

    /// Encodes the message. Names are written uncompressed.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Vec::with_capacity(512);
        for v in [
            self.id,
            self.flags,
            count(self.questions.len())?,
            count(self.answers.len())?,
            count(self.authority.len())?,
            count(self.additional.len())?,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        for q in &self.questions {
            encode_name(&q.name, &mut out)?;
            out.extend_from_slice(&q.rtype.to_be_bytes());
            out.extend_from_slice(&q.class.to_be_bytes());
        }
        for r in self
            .answers
            .iter()
            .chain(self.authority.iter())
            .chain(self.additional.iter())
        {
            encode_record(r, &mut out)?;
        }
        Ok(out)
    }

    /// Parses a message. Compressed names are expanded and rdata is made self-contained.
    pub fn decode(msg: &[u8]) -> Result<Message, WireError> {
        let id = read_u16(msg, 0)?;
        let flags = read_u16(msg, 2)?;
        let qd = usize::from(read_u16(msg, 4)?);
        let an = usize::from(read_u16(msg, 6)?);
        let ns = usize::from(read_u16(msg, 8)?);
        let ar = usize::from(read_u16(msg, 10)?);
        let mut pos = 12;
        let mut questions = Vec::new();
        for _ in 0..qd {
            let (name, next) = read_name(msg, pos)?;
            let rtype = read_u16(msg, next)?;
            let class = read_u16(msg, next + 2)?;
            pos = next + 4;
            questions.push(Question { name, rtype, class });
        }
        let answers = read_records(msg, &mut pos, an)?;
        let authority = read_records(msg, &mut pos, ns)?;
        let additional = read_records(msg, &mut pos, ar)?;
        Ok(Message {
            id,
            flags,
            questions,
            answers,
            authority,
            additional,
        })
    }
}

/// Builds a one-question query for a name and record type, class IN.
pub fn query(id: u16, name: &str, rtype: u16) -> Result<Vec<u8>, WireError> {
    Message {
        id,
        questions: vec![Question {
            name: name.to_string(),
            rtype,
            class: CLASS_IN,
        }],
        ..Message::default()
    }
    .encode()
}

/// Answers in a message that match a name and record type, in message order.
pub fn answers_for<'a>(msg: &'a Message, name: &str, rtype: u16) -> Vec<&'a Record> {
    msg.answers
        .iter()
        .filter(|r| r.rtype == rtype && same_name(&r.name, name))
        .collect()
}

/// Record data for an A or AAAA record.
pub fn address_rdata(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(a) => a.octets().to_vec(),
        IpAddr::V6(a) => a.octets().to_vec(),
    }
}

/// The address in A or AAAA record data, or None if the length does not match.
pub fn parse_address(rtype: u16, rdata: &[u8]) -> Option<IpAddr> {
    match rtype {
        rtype::A if rdata.len() == 4 => {
            let mut b = [0u8; 4];
            b.copy_from_slice(rdata);
            Some(IpAddr::V4(Ipv4Addr::from(b)))
        }
        rtype::AAAA if rdata.len() == 16 => {
            let mut b = [0u8; 16];
            b.copy_from_slice(rdata);
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}

/// Record data for a PTR (or CNAME, NS) record: one uncompressed name.
pub fn name_rdata(name: &str) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::new();
    encode_name(name, &mut out)?;
    Ok(out)
}

/// The name in PTR, CNAME or NS record data.
pub fn parse_name_rdata(rdata: &[u8]) -> Option<String> {
    read_name(rdata, 0).ok().map(|(name, _)| name)
}

/// A parsed SRV record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// Record data for an SRV record.
pub fn srv_rdata(priority: u16, weight: u16, port: u16, target: &str) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::new();
    out.extend_from_slice(&priority.to_be_bytes());
    out.extend_from_slice(&weight.to_be_bytes());
    out.extend_from_slice(&port.to_be_bytes());
    encode_name(target, &mut out)?;
    Ok(out)
}

/// Parses SRV record data.
pub fn parse_srv(rdata: &[u8]) -> Option<Srv> {
    let priority = read_u16(rdata, 0).ok()?;
    let weight = read_u16(rdata, 2).ok()?;
    let port = read_u16(rdata, 4).ok()?;
    let (target, _) = read_name(rdata, 6).ok()?;
    Some(Srv {
        priority,
        weight,
        port,
        target,
    })
}

/// Record data for a TXT record.
pub fn txt_rdata(txt: &TxtRecord) -> Vec<u8> {
    txt.to_bytes()
}

/// Parses TXT record data.
pub fn parse_txt(rdata: &[u8]) -> Option<TxtRecord> {
    TxtRecord::from_bytes(rdata).ok()
}

/// The addresses a name resolves to, following CNAME records in the answers.
/// Returns an empty list when the chain ends without an address or runs too long.
pub fn resolve_addresses(records: &[Record], name: &str) -> Vec<IpAddr> {
    let mut target = name.to_string();
    for _ in 0..MAX_CNAME_HOPS {
        let addresses: Vec<IpAddr> = records
            .iter()
            .filter(|r| same_name(&r.name, &target))
            .filter_map(|r| parse_address(r.rtype, &r.rdata))
            .collect();
        if !addresses.is_empty() {
            return addresses;
        }
        let next = records
            .iter()
            .find(|r| r.rtype == rtype::CNAME && same_name(&r.name, &target))
            .and_then(|r| parse_name_rdata(&r.rdata));
        match next {
            Some(n) => target = n,
            None => return Vec::new(),
        }
    }
    Vec::new()
}

/// Two names are equal when they differ only in case and a trailing dot.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim_end_matches('.')
        .eq_ignore_ascii_case(b.trim_end_matches('.'))
}

fn count(n: usize) -> Result<u16, WireError> {
    u16::try_from(n).map_err(|_| WireError::TooLong)
}

fn read_u16(data: &[u8], pos: usize) -> Result<u16, WireError> {
    let b = data.get(pos..pos + 2).ok_or(WireError::Truncated)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn read_u32(data: &[u8], pos: usize) -> Result<u32, WireError> {
    let b = data.get(pos..pos + 4).ok_or(WireError::Truncated)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Writes a name as length-prefixed labels ending in the root byte.
fn encode_name(name: &str, out: &mut Vec<u8>) -> Result<(), WireError> {
    let start = out.len();
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    if !trimmed.is_empty() {
        for label in trimmed.split('.') {
            let bytes = label.as_bytes();
            if bytes.is_empty() || bytes.len() > MAX_LABEL {
                out.truncate(start);
                return Err(WireError::BadName);
            }
            out.push(bytes.len() as u8);
            out.extend_from_slice(bytes);
        }
    }
    out.push(0);
    if out.len() - start > MAX_NAME {
        out.truncate(start);
        return Err(WireError::BadName);
    }
    Ok(())
}

/// Reads a name at `start`, following compression pointers. Returns the name with a
/// trailing dot and the offset just past the name's bytes at `start`.
fn read_name(msg: &[u8], start: usize) -> Result<(String, usize), WireError> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = start;
    let mut end: Option<usize> = None;
    let mut jumps = 0;
    loop {
        let len = usize::from(*msg.get(pos).ok_or(WireError::Truncated)?);
        if len == 0 {
            if end.is_none() {
                end = Some(pos + 1);
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            let lo = usize::from(*msg.get(pos + 1).ok_or(WireError::Truncated)?);
            if end.is_none() {
                end = Some(pos + 2);
            }
            jumps += 1;
            if jumps > MAX_JUMPS {
                return Err(WireError::BadPointer);
            }
            pos = ((len & 0x3F) << 8) | lo;
            continue;
        }
        if len & 0xC0 != 0 {
            return Err(WireError::BadPointer);
        }
        let label = msg
            .get(pos + 1..pos + 1 + len)
            .ok_or(WireError::Truncated)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        pos += 1 + len;
    }
    let mut name = String::new();
    for label in &labels {
        name.push_str(label);
        name.push('.');
    }
    if name.is_empty() {
        name.push('.');
    }
    Ok((name, end.unwrap_or(start)))
}

fn encode_record(r: &Record, out: &mut Vec<u8>) -> Result<(), WireError> {
    encode_name(&r.name, out)?;
    out.extend_from_slice(&r.rtype.to_be_bytes());
    let class = (r.class & 0x7FFF) | (if r.cache_flush { CACHE_FLUSH } else { 0 });
    out.extend_from_slice(&class.to_be_bytes());
    out.extend_from_slice(&r.ttl.to_be_bytes());
    let len = u16::try_from(r.rdata.len()).map_err(|_| WireError::TooLong)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&r.rdata);
    Ok(())
}

fn read_records(msg: &[u8], pos: &mut usize, count: usize) -> Result<Vec<Record>, WireError> {
    let mut out = Vec::new();
    for _ in 0..count {
        let (name, next) = read_name(msg, *pos)?;
        let rtype = read_u16(msg, next)?;
        let raw_class = read_u16(msg, next + 2)?;
        let ttl = read_u32(msg, next + 4)?;
        let rdlen = usize::from(read_u16(msg, next + 8)?);
        let start = next + 10;
        let end = start + rdlen;
        if end > msg.len() {
            return Err(WireError::Truncated);
        }
        let rdata = normalize_rdata(msg, rtype, start, end)?;
        *pos = end;
        out.push(Record {
            name,
            rtype,
            class: raw_class & 0x7FFF,
            cache_flush: raw_class & CACHE_FLUSH != 0,
            ttl,
            rdata,
        });
    }
    Ok(out)
}

/// Copies record data, expanding any compressed names it holds into plain labels.
fn normalize_rdata(msg: &[u8], rtype: u16, start: usize, end: usize) -> Result<Vec<u8>, WireError> {
    match rtype {
        rtype::NS | rtype::PTR | rtype::CNAME => {
            let (name, _) = read_name(msg, start)?;
            name_rdata(&name)
        }
        rtype::SRV => {
            let head = msg.get(start..start + 6).ok_or(WireError::Truncated)?;
            let (target, _) = read_name(msg, start + 6)?;
            let mut out = head.to_vec();
            encode_name(&target, &mut out)?;
            Ok(out)
        }
        _ => Ok(msg[start..end].to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_round_trips_as_labels() {
        let mut wire = Vec::new();
        encode_name("_http._tcp.local.", &mut wire).unwrap();
        // 1+5 for "_http", 1+4 for "_tcp", 1+5 for "local", then the root byte: 18 bytes.
        assert_eq!(wire.len(), 18);
        assert_eq!(wire[0], 5);
        assert_eq!(&wire[1..6], b"_http");
        assert_eq!(wire[17], 0);
        let (name, next) = read_name(&wire, 0).unwrap();
        assert_eq!(name, "_http._tcp.local.");
        assert_eq!(next, 18);
    }

    #[test]
    fn compression_pointer_resolves_to_an_earlier_name() {
        // "local." at offset 0, then "x" followed by a pointer back to offset 0.
        let msg = [5, b'l', b'o', b'c', b'a', b'l', 0, 1, b'x', 0xC0, 0];
        let (name, next) = read_name(&msg, 7).unwrap();
        assert_eq!(name, "x.local.");
        assert_eq!(next, 11);
    }

    #[test]
    fn a_pointer_that_loops_is_refused() {
        let msg = [0xC0, 0];
        assert_eq!(read_name(&msg, 0), Err(WireError::BadPointer));
    }

    #[test]
    fn over_long_label_is_refused() {
        let long = "a".repeat(64);
        let mut wire = Vec::new();
        assert_eq!(encode_name(&format!("{long}.local."), &mut wire), Err(WireError::BadName));
        assert!(wire.is_empty());
    }

    #[test]
    fn query_encodes_header_and_question_and_decodes_back() {
        let bytes = query(0x1234, "_ipp._tcp.local.", rtype::PTR).unwrap();
        assert_eq!(&bytes[0..2], &[0x12, 0x34]);
        assert_eq!(&bytes[2..4], &[0, 0]);
        assert_eq!(&bytes[4..6], &[0, 1]);
        let msg = Message::decode(&bytes).unwrap();
        assert!(!msg.is_response());
        assert_eq!(msg.questions.len(), 1);
        assert_eq!(msg.questions[0].name, "_ipp._tcp.local.");
        assert_eq!(msg.questions[0].rtype, rtype::PTR);
        assert_eq!(msg.questions[0].class, CLASS_IN);
    }

    #[test]
    fn truncated_message_is_refused() {
        assert_eq!(Message::decode(&[0u8; 5]), Err(WireError::Truncated));
    }

    #[test]
    fn cname_chain_resolves_to_an_address() {
        let records = vec![
            Record::new("www.example.test.", rtype::CNAME, 60, name_rdata("host.example.test.").unwrap()),
            Record::new("HOST.example.test", rtype::A, 60, address_rdata("192.0.2.7".parse().unwrap())),
        ];
        let got = resolve_addresses(&records, "www.example.test.");
        assert_eq!(got, vec!["192.0.2.7".parse::<IpAddr>().unwrap()]);
        assert!(resolve_addresses(&records, "other.example.test.").is_empty());
    }

    #[test]
    fn srv_record_round_trips_through_a_message() {
        let rdata = srv_rdata(0, 0, 631, "printer.local.").unwrap();
        let msg = Message {
            id: 7,
            flags: FLAG_RESPONSE,
            answers: vec![Record::new("_ipp._tcp.local.", rtype::SRV, 120, rdata)],
            ..Message::default()
        };
        let bytes = msg.encode().unwrap();
        let back = Message::decode(&bytes).unwrap();
        assert!(back.is_response());
        let srv = parse_srv(&back.answers[0].rdata).unwrap();
        assert_eq!(srv.port, 631);
        assert_eq!(srv.target, "printer.local.");
    }

    #[test]
    fn txt_rdata_keeps_its_keys() {
        let mut txt = TxtRecord::new();
        txt.set("txtvers", Some(&b"1"[..])).unwrap();
        txt.set("flags", None).unwrap();
        let parsed = parse_txt(&txt_rdata(&txt)).unwrap();
        assert!(parsed.contains_key("txtvers"));
        assert!(parsed.contains_key("flags"));
        assert!(!parsed.contains_key("missing"));
    }

    #[test]
    fn enumeration_adds_removes_and_marks_the_default() {
        let mut e = DomainEnumeration::new();
        assert!(e.apply(FLAG_BROWSE_DOMAINS | FLAG_ADD | FLAG_DEFAULT | FLAG_MORE_COMING, "local."));
        assert!(!e.is_complete());
        assert!(e.apply(FLAG_BROWSE_DOMAINS | FLAG_ADD, "example.com."));
        assert!(e.apply(FLAG_REGISTRATION_DOMAINS | FLAG_ADD | FLAG_DEFAULT, "local."));
        assert!(e.is_complete());
        // Removing a browse domain drops it from that list only.
        assert!(e.apply(FLAG_BROWSE_DOMAINS, "EXAMPLE.com."));
        assert_eq!(e.domains(DomainKind::Browse), vec!["local."]);
        assert_eq!(e.domains(DomainKind::Registration), vec!["local."]);
        assert_eq!(e.default_domain(DomainKind::Browse), Some("local."));
        assert_eq!(e.default_domain(DomainKind::Registration), Some("local."));
        // A reply that names neither list is ignored.
        assert!(!e.apply(FLAG_ADD, "ignored.test."));
    }

    #[test]
    fn a_new_default_replaces_the_old_one_of_its_kind() {
        let mut e = DomainEnumeration::new();
        e.apply(FLAG_BROWSE_DOMAINS | FLAG_ADD | FLAG_DEFAULT, "a.test.");
        e.apply(FLAG_BROWSE_DOMAINS | FLAG_ADD | FLAG_DEFAULT, "b.test.");
        assert_eq!(e.default_domain(DomainKind::Browse), Some("b.test."));
        assert_eq!(e.domains(DomainKind::Browse), vec!["a.test.", "b.test."]);
    }

    #[test]
    fn error_names_follow_the_c_constants() {
        assert_eq!(DnsServiceError::NameConflict.to_string(), "kDNSServiceErr_NameConflict");
        assert_eq!(DnsServiceError::NatTraversal.suffix(), "NATTraversal");
        assert_eq!(DnsServiceError::ALL.len(), 37);
        assert!(DnsServiceError::ALL.contains(&DnsServiceError::NoError));
        assert_eq!(INTERFACE_ANY, 0);
        assert_eq!(INTERFACE_LOCAL_ONLY, 0xFFFF_FFFF);
        assert_eq!(INTERFACE_UNICAST, 0xFFFF_FFFE);
    }
}
