//! DNS message wire format for multicast and unicast DNS-SD (std only).
//!
//! Encodes and decodes DNS messages in the layout RFC 1035 section 4 defines, with the class
//! bit of RFC 6762 (cache-flush on answers, unicast-response on questions) and the PTR, SRV,
//! TXT, A and AAAA record data of RFC 6763. Names are written without compression and read
//! with compression pointers. This module opens no socket: the service layer sends the bytes
//! it encodes and hands it the bytes it receives.

use std::fmt;

/// The Internet class.
pub const CLASS_IN: u16 = 1;
/// Top bit of a class: cache-flush on an answer, unicast-response on a question.
const TOP_BIT: u16 = 0x8000;
/// Header flag: the message is a response (QR).
const FLAG_RESPONSE: u16 = 0x8000;
/// Header flag: the responder is authoritative for the name (AA).
const FLAG_AUTHORITATIVE: u16 = 0x0400;
/// Longest label a length byte can describe.
const MAX_LABEL: usize = 63;
/// Longest name on the wire, including the terminating zero byte.
const MAX_NAME: usize = 255;
/// Longest pointer chain accepted while reading one name.
const MAX_POINTER_HOPS: usize = 64;

/// Why a message could not be encoded or decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsError {
    /// A label is empty or longer than 63 bytes, or the name is longer than 255 bytes.
    BadName,
    /// The message ends before a field is complete.
    Truncated,
    /// A compression pointer points forward or to itself, or it loops.
    BadPointer,
    /// A record length does not match its data, or TXT data is not length-prefixed strings.
    BadLength,
    /// A section holds more than 65535 entries.
    TooMany,
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DnsError::BadName => "DNS name is malformed or too long",
            DnsError::Truncated => "DNS message ends inside a field",
            DnsError::BadPointer => "DNS name compression pointer is invalid",
            DnsError::BadLength => "DNS record length does not match its data",
            DnsError::TooMany => "DNS section holds more than 65535 entries",
        })
    }
}

impl std::error::Error for DnsError {}

/// A record or question type. Types this module does not interpret are kept by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RType {
    A,
    Ptr,
    Txt,
    Aaaa,
    Srv,
    Other(u16),
}

impl RType {
    /// The 16-bit type number on the wire.
    pub fn code(self) -> u16 {
        match self {
            RType::A => 1,
            RType::Ptr => 12,
            RType::Txt => 16,
            RType::Aaaa => 28,
            RType::Srv => 33,
            RType::Other(code) => code,
        }
    }

    /// The type for a 16-bit type number.
    pub fn from_code(code: u16) -> RType {
        match code {
            1 => RType::A,
            12 => RType::Ptr,
            16 => RType::Txt,
            28 => RType::Aaaa,
            33 => RType::Srv,
            other => RType::Other(other),
        }
    }
}

/// The data of one resource record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RData {
    /// An IPv4 address.
    A([u8; 4]),
    /// An IPv6 address.
    Aaaa([u8; 16]),
    /// A pointer to another name, such as a service instance.
    Ptr(String),
    /// A service location: priority, weight, port and the host name.
    Srv {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    /// The raw TXT strings: each is a length byte followed by that many bytes.
    Txt(Vec<u8>),
    /// Any other type, kept as the raw record data.
    Raw { rtype: RType, bytes: Vec<u8> },
}

impl RData {
    /// The record type that this data belongs to.
    pub fn rtype(&self) -> RType {
        match self {
            RData::A(_) => RType::A,
            RData::Aaaa(_) => RType::Aaaa,
            RData::Ptr(_) => RType::Ptr,
            RData::Srv { .. } => RType::Srv,
            RData::Txt(_) => RType::Txt,
            RData::Raw { rtype, .. } => *rtype,
        }
    }
}

/// One resource record in the answer, authority or additional section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    /// The class without the top bit.
    pub class: u16,
    /// Set on records that replace older cached records with the same name and type.
    pub cache_flush: bool,
    pub ttl: u32,
    pub data: RData,
}

/// One entry of the question section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub name: String,
    pub rtype: RType,
    /// The class without the top bit.
    pub class: u16,
    /// Set when the querier asks for a unicast answer.
    pub unicast_response: bool,
}

/// A whole DNS message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: u16,
    pub response: bool,
    pub authoritative: bool,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub authorities: Vec<Record>,
    pub additionals: Vec<Record>,
}

/// Appends `name` to `out` as a sequence of labels ending in a zero byte.
/// A trailing dot is allowed. An empty name is the root.
pub fn encode_name(name: &str, out: &mut Vec<u8>) -> Result<(), DnsError> {
    let start = out.len();
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    if !trimmed.is_empty() {
        for label in trimmed.split('.') {
            if label.is_empty() || label.len() > MAX_LABEL {
                out.truncate(start);
                return Err(DnsError::BadName);
            }
            out.push(label.len() as u8);
            out.extend_from_slice(label.as_bytes());
        }
    }
    out.push(0);
    if out.len() - start > MAX_NAME {
        out.truncate(start);
        return Err(DnsError::BadName);
    }
    Ok(())
}

/// Reads the name at `pos` and returns it with the position just after it in the message.
/// Compression pointers are followed; each must point to bytes before itself.
pub fn decode_name(msg: &[u8], mut pos: usize) -> Result<(String, usize), DnsError> {
    let mut labels: Vec<String> = Vec::new();
    let mut end: Option<usize> = None;
    let mut hops = 0usize;
    let mut total = 0usize;
    loop {
        let len = usize::from(*msg.get(pos).ok_or(DnsError::Truncated)?);
        if len & 0xC0 == 0xC0 {
            let low = usize::from(*msg.get(pos + 1).ok_or(DnsError::Truncated)?);
            let target = ((len & 0x3F) << 8) | low;
            if end.is_none() {
                end = Some(pos + 2);
            }
            if target >= pos {
                return Err(DnsError::BadPointer);
            }
            hops += 1;
            if hops > MAX_POINTER_HOPS {
                return Err(DnsError::BadPointer);
            }
            pos = target;
        } else if len & 0xC0 != 0 {
            return Err(DnsError::BadName);
        } else if len == 0 {
            let after = end.unwrap_or(pos + 1);
            return Ok((labels.join("."), after));
        } else {
            let start = pos + 1;
            let bytes = msg.get(start..start + len).ok_or(DnsError::Truncated)?;
            total += len + 1;
            if total > MAX_NAME {
                return Err(DnsError::BadName);
            }
            labels.push(String::from_utf8_lossy(bytes).into_owned());
            pos = start + len;
        }
    }
}

/// True when `bytes` is a run of length-prefixed strings that ends exactly at its last byte.
pub fn txt_is_valid(bytes: &[u8]) -> bool {
    let mut i = 0usize;
    while i < bytes.len() {
        i += 1 + usize::from(bytes[i]);
    }
    i == bytes.len()
}

/// Encodes a message. Names are written without compression.
pub fn encode(msg: &Message) -> Result<Vec<u8>, DnsError> {
    let counts = [
        msg.questions.len(),
        msg.answers.len(),
        msg.authorities.len(),
        msg.additionals.len(),
    ];
    if counts.iter().any(|&n| n > usize::from(u16::MAX)) {
        return Err(DnsError::TooMany);
    }
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(&msg.id.to_be_bytes());
    let mut flags = 0u16;
    if msg.response {
        flags |= FLAG_RESPONSE;
    }
    if msg.authoritative {
        flags |= FLAG_AUTHORITATIVE;
    }
    out.extend_from_slice(&flags.to_be_bytes());
    for n in counts {
        out.extend_from_slice(&(n as u16).to_be_bytes());
    }
    for q in &msg.questions {
        encode_name(&q.name, &mut out)?;
        out.extend_from_slice(&q.rtype.code().to_be_bytes());
        let mut class = q.class & !TOP_BIT;
        if q.unicast_response {
            class |= TOP_BIT;
        }
        out.extend_from_slice(&class.to_be_bytes());
    }
    for r in msg.answers.iter().chain(&msg.authorities).chain(&msg.additionals) {
        encode_record(r, &mut out)?;
    }
    Ok(out)
}

fn encode_record(r: &Record, out: &mut Vec<u8>) -> Result<(), DnsError> {
    encode_name(&r.name, out)?;
    out.extend_from_slice(&r.data.rtype().code().to_be_bytes());
    let mut class = r.class & !TOP_BIT;
    if r.cache_flush {
        class |= TOP_BIT;
    }
    out.extend_from_slice(&class.to_be_bytes());
    out.extend_from_slice(&r.ttl.to_be_bytes());
    let len_at = out.len();
    out.extend_from_slice(&[0, 0]);
    let body_start = out.len();
    match &r.data {
        RData::A(addr) => out.extend_from_slice(&addr[..]),
        RData::Aaaa(addr) => out.extend_from_slice(&addr[..]),
        RData::Ptr(target) => encode_name(target, out)?,
        RData::Srv {
            priority,
            weight,
            port,
            target,
        } => {
            out.extend_from_slice(&priority.to_be_bytes());
            out.extend_from_slice(&weight.to_be_bytes());
            out.extend_from_slice(&port.to_be_bytes());
            encode_name(target, out)?;
        }
        RData::Txt(bytes) => {
            if !txt_is_valid(bytes) {
                return Err(DnsError::BadLength);
            }
            out.extend_from_slice(bytes);
        }
        RData::Raw { bytes, .. } => out.extend_from_slice(bytes),
    }
    let len = out.len() - body_start;
    if len > usize::from(u16::MAX) {
        return Err(DnsError::BadLength);
    }
    out[len_at..len_at + 2].copy_from_slice(&(len as u16).to_be_bytes());
    Ok(())
}

/// Decodes a message. Bytes after the last record are ignored.
pub fn decode(msg: &[u8]) -> Result<Message, DnsError> {
    let id = u16_at(msg, 0)?;
    let flags = u16_at(msg, 2)?;
    let qd = usize::from(u16_at(msg, 4)?);
    let an = usize::from(u16_at(msg, 6)?);
    let ns = usize::from(u16_at(msg, 8)?);
    let ar = usize::from(u16_at(msg, 10)?);
    let mut pos = 12usize;
    let mut questions = Vec::new();
    for _ in 0..qd {
        let (name, p) = decode_name(msg, pos)?;
        let code = u16_at(msg, p)?;
        let class = u16_at(msg, p + 2)?;
        questions.push(Question {
            name,
            rtype: RType::from_code(code),
            class: class & !TOP_BIT,
            unicast_response: class & TOP_BIT != 0,
        });
        pos = p + 4;
    }
    let answers = read_records(msg, &mut pos, an)?;
    let authorities = read_records(msg, &mut pos, ns)?;
    let additionals = read_records(msg, &mut pos, ar)?;
    Ok(Message {
        id,
        response: flags & FLAG_RESPONSE != 0,
        authoritative: flags & FLAG_AUTHORITATIVE != 0,
        questions,
        answers,
        authorities,
        additionals,
    })
}

fn read_records(msg: &[u8], pos: &mut usize, count: usize) -> Result<Vec<Record>, DnsError> {
    let mut out = Vec::new();
    for _ in 0..count {
        let (record, next) = decode_record(msg, *pos)?;
        out.push(record);
        *pos = next;
    }
    Ok(out)
}

fn decode_record(msg: &[u8], pos: usize) -> Result<(Record, usize), DnsError> {
    let (name, p) = decode_name(msg, pos)?;
    let code = u16_at(msg, p)?;
    let class = u16_at(msg, p + 2)?;
    let ttl = u32_at(msg, p + 4)?;
    let rdlen = usize::from(u16_at(msg, p + 8)?);
    let body_at = p + 10;
    let body = msg
        .get(body_at..body_at + rdlen)
        .ok_or(DnsError::Truncated)?;
    let rtype = RType::from_code(code);
    let data = match rtype {
        RType::A => {
            if body.len() != 4 {
                return Err(DnsError::BadLength);
            }
            RData::A([body[0], body[1], body[2], body[3]])
        }
        RType::Aaaa => {
            if body.len() != 16 {
                return Err(DnsError::BadLength);
            }
            let mut addr = [0u8; 16];
            addr.copy_from_slice(body);
            RData::Aaaa(addr)
        }
        RType::Ptr => {
            let (target, end) = decode_name(msg, body_at)?;
            if end != body_at + rdlen {
                return Err(DnsError::BadLength);
            }
            RData::Ptr(target)
        }
        RType::Srv => {
            if body.len() < 7 {
                return Err(DnsError::BadLength);
            }
            let priority = u16::from_be_bytes([body[0], body[1]]);
            let weight = u16::from_be_bytes([body[2], body[3]]);
            let port = u16::from_be_bytes([body[4], body[5]]);
            let (target, end) = decode_name(msg, body_at + 6)?;
            if end != body_at + rdlen {
                return Err(DnsError::BadLength);
            }
            RData::Srv {
                priority,
                weight,
                port,
                target,
            }
        }
        RType::Txt => {
            if !txt_is_valid(body) {
                return Err(DnsError::BadLength);
            }
            RData::Txt(body.to_vec())
        }
        other => RData::Raw {
            rtype: other,
            bytes: body.to_vec(),
        },
    };
    let record = Record {
        name,
        class: class & !TOP_BIT,
        cache_flush: class & TOP_BIT != 0,
        ttl,
        data,
    };
    Ok((record, body_at + rdlen))
}

fn u16_at(msg: &[u8], pos: usize) -> Result<u16, DnsError> {
    let b = msg.get(pos..pos + 2).ok_or(DnsError::Truncated)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn u32_at(msg: &[u8], pos: usize) -> Result<u32, DnsError> {
    let b = msg.get(pos..pos + 4).ok_or(DnsError::Truncated)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr_record(instance: &str) -> Record {
        Record {
            name: "_ipp._tcp.local".to_string(),
            class: CLASS_IN,
            cache_flush: false,
            ttl: 4500,
            data: RData::Ptr(instance.to_string()),
        }
    }

    #[test]
    fn name_is_length_prefixed_labels_with_zero_end() {
        let mut out = Vec::new();
        encode_name("_ipp._tcp.local", &mut out).unwrap();
        // 1+4 for "_ipp", 1+4 for "_tcp", 1+5 for "local", then the zero byte: 17 bytes.
        assert_eq!(out.len(), 17);
        assert_eq!(out[0], 4);
        assert_eq!(out[5], 4);
        assert_eq!(out[10], 5);
        assert_eq!(out[16], 0);
        // A trailing dot names the same thing.
        let mut dotted = Vec::new();
        encode_name("_ipp._tcp.local.", &mut dotted).unwrap();
        assert_eq!(out, dotted);
    }

    #[test]
    fn label_longer_than_63_bytes_is_refused_and_leaves_no_bytes() {
        let mut out = vec![9u8];
        let long = "a".repeat(64);
        assert_eq!(encode_name(&long, &mut out), Err(DnsError::BadName));
        assert_eq!(out, vec![9u8]);
        // An empty label in the middle is refused too.
        assert_eq!(encode_name("a..b", &mut out), Err(DnsError::BadName));
    }

    #[test]
    fn compression_pointer_is_followed_and_the_name_ends_after_it() {
        // "foo.local" at offset 0, then "bar" followed by a pointer to offset 0.
        let mut msg = vec![3, b'f', b'o', b'o', 5, b'l', b'o', b'c', b'a', b'l', 0];
        msg.extend_from_slice(&[3, b'b', b'a', b'r', 0xC0, 0x00]);
        let (name, end) = decode_name(&msg, 11).unwrap();
        assert_eq!(name, "bar.foo.local");
        // 1 + 3 for "bar", then the 2-byte pointer: the name ends at 11 + 6 = 17.
        assert_eq!(end, 17);
    }

    #[test]
    fn forward_or_self_pointer_is_refused() {
        // The pointer at offset 0 targets offset 5, which is after it.
        let msg = vec![0xC0, 0x05, 0, 0, 0, 0];
        assert_eq!(decode_name(&msg, 0), Err(DnsError::BadPointer));
        // A pointer to its own position would loop forever.
        let self_loop = vec![0xC0, 0x00];
        assert_eq!(decode_name(&self_loop, 0), Err(DnsError::BadPointer));
    }

    #[test]
    fn header_flags_and_counts_are_written_big_endian() {
        let msg = Message {
            id: 0x1234,
            response: true,
            authoritative: true,
            questions: vec![],
            answers: vec![],
            authorities: vec![],
            additionals: vec![],
        };
        let bytes = encode(&msg).unwrap();
        // Flags 0x8000 | 0x0400 = 0x8400; four zero counts follow the flags.
        assert_eq!(&bytes[..], &[0x12, 0x34, 0x84, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn ptr_answer_round_trips_with_its_ttl_and_class() {
        let msg = Message {
            id: 7,
            response: true,
            authoritative: true,
            questions: vec![],
            answers: vec![ptr_record("Printer._ipp._tcp.local")],
            authorities: vec![],
            additionals: vec![],
        };
        let bytes = encode(&msg).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back, msg);
        assert_eq!(back.answers[0].ttl, 4500);
        assert_eq!(back.answers[0].data, RData::Ptr("Printer._ipp._tcp.local".into()));
    }

    #[test]
    fn srv_and_a_records_round_trip_with_cache_flush_bit_on_the_wire() {
        let srv = Record {
            name: "Printer._ipp._tcp.local".to_string(),
            class: CLASS_IN,
            cache_flush: true,
            ttl: 120,
            data: RData::Srv {
                priority: 0,
                weight: 0,
                port: 631,
                target: "box.local".to_string(),
            },
        };
        let host = Record {
            name: "box.local".to_string(),
            class: CLASS_IN,
            cache_flush: true,
            ttl: 120,
            data: RData::A([192, 168, 1, 20]),
        };
        let msg = Message {
            id: 0,
            response: true,
            authoritative: true,
            questions: vec![],
            answers: vec![srv.clone()],
            authorities: vec![],
            additionals: vec![host.clone()],
        };
        let bytes = encode(&msg).unwrap();
        // The message has no questions, so the SRV record starts at offset 12. Its name is
        // "Printer" (1+7), "_ipp" (1+4), "_tcp" (1+4), "local" (1+5) and the zero byte: 25 bytes.
        // The 2-byte type comes next, so the class starts at 12 + 25 + 2 = 39.
        let name_len = 8 + 5 + 5 + 6 + 1;
        let class_at = 12 + name_len + 2;
        assert_eq!(&bytes[class_at..class_at + 2], &[0x80, 0x01]);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.answers, vec![srv]);
        assert_eq!(back.additionals, vec![host]);
    }

    #[test]
    fn question_with_unicast_bit_round_trips() {
        let q = Question {
            name: "box.local".to_string(),
            rtype: RType::Aaaa,
            class: CLASS_IN,
            unicast_response: true,
        };
        let msg = Message {
            id: 1,
            response: false,
            authoritative: false,
            questions: vec![q.clone()],
            answers: vec![],
            authorities: vec![],
            additionals: vec![],
        };
        let back = decode(&encode(&msg).unwrap()).unwrap();
        assert_eq!(back.questions, vec![q]);
        assert!(!back.response);
    }

    #[test]
    fn truncated_header_and_bad_txt_are_refused() {
        assert_eq!(decode(&[0u8; 11]), Err(DnsError::Truncated));
        // "3 a" claims three bytes but holds one.
        assert!(!txt_is_valid(&[3, b'a']));
        assert!(txt_is_valid(&[3, b'a', b'=', b'1']));
        let bad = Record {
            name: "x.local".to_string(),
            class: CLASS_IN,
            cache_flush: false,
            ttl: 1,
            data: RData::Txt(vec![3, b'a']),
        };
        let msg = Message {
            id: 0,
            response: true,
            authoritative: false,
            questions: vec![],
            answers: vec![bad],
            authorities: vec![],
            additionals: vec![],
        };
        assert_eq!(encode(&msg), Err(DnsError::BadLength));
    }
}
