//! IPP/2.0 binary codec (RFC 8010 framing, RFC 8011 attributes), the subset the printer client needs.
//!
//! [`Request`] encodes an operation (header, attribute groups, end tag, optional document bytes). [`Response::decode`]
//! reads a response into [`Group`]s of [`Attribute`]s. Values keep their IPP tag so a caller can tell an enum from an
//! integer. Pure byte code: no sockets here, so every rule is covered by unit tests in this file.

use std::fmt;

pub const VERSION_MAJOR: u8 = 2;
pub const VERSION_MINOR: u8 = 0;

/// Operation identifiers (RFC 8011 section 4.4 and the CUPS extensions).
pub mod op {
    pub const PRINT_JOB: u16 = 0x0002;
    pub const CANCEL_JOB: u16 = 0x0008;
    pub const GET_JOBS: u16 = 0x000A;
    pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
    pub const PAUSE_PRINTER: u16 = 0x0010;
    pub const RESUME_PRINTER: u16 = 0x0011;
    pub const CUPS_GET_DEFAULT: u16 = 0x4001;
    pub const CUPS_GET_PRINTERS: u16 = 0x4002;
    pub const CUPS_ADD_MODIFY_PRINTER: u16 = 0x4003;
    pub const CUPS_DELETE_PRINTER: u16 = 0x4004;
    pub const CUPS_SET_DEFAULT: u16 = 0x4005;
}

/// Group delimiter tags (RFC 8010 section 3.5.1).
pub mod group {
    pub const OPERATION: u8 = 0x01;
    pub const JOB: u8 = 0x02;
    pub const PRINTER: u8 = 0x04;
    pub const UNSUPPORTED: u8 = 0x05;
}

pub const END_OF_ATTRIBUTES: u8 = 0x03;

/// Value tags (RFC 8010 section 3.5.2). Tags below 0x10 are delimiters, so value tags start at 0x10.
pub mod tag {
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const OCTET_STRING: u8 = 0x30;
    pub const DATE_TIME: u8 = 0x31;
    pub const RESOLUTION: u8 = 0x32;
    pub const RANGE_OF_INTEGER: u8 = 0x33;
    pub const TEXT_WITH_LANGUAGE: u8 = 0x35;
    pub const NAME_WITH_LANGUAGE: u8 = 0x36;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const URI_SCHEME: u8 = 0x46;
    pub const CHARSET: u8 = 0x47;
    pub const NATURAL_LANGUAGE: u8 = 0x48;
    pub const MIME_MEDIA_TYPE: u8 = 0x49;
}

/// Status codes at or above this value are errors (RFC 8011 section 13).
pub const CLIENT_ERROR_MIN: u16 = 0x0400;
pub const STATUS_NOT_FOUND: u16 = 0x0406;

pub fn is_success(status: u16) -> bool {
    status < CLIENT_ERROR_MIN
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IppError {
    /// The buffer ended inside a header, attribute or value.
    Truncated,
    /// The bytes break the framing rules.
    Malformed(&'static str),
}

impl fmt::Display for IppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IppError::Truncated => write!(f, "IPP message truncated"),
            IppError::Malformed(why) => write!(f, "IPP message malformed: {why}"),
        }
    }
}

impl std::error::Error for IppError {}

/// One attribute value. Text-like tags (keyword, uri, name, text, charset, language, mime) share [`Value::Text`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    Text { tag: u8, text: String },
    /// Any other value (dateTime, resolution, octetString, ...), kept as the raw bytes.
    Other { tag: u8, bytes: Vec<u8> },
}

impl Value {
    pub fn keyword(s: &str) -> Value {
        Value::Text { tag: tag::KEYWORD, text: s.to_string() }
    }

    pub fn uri(s: &str) -> Value {
        Value::Text { tag: tag::URI, text: s.to_string() }
    }

    pub fn name(s: &str) -> Value {
        Value::Text { tag: tag::NAME, text: s.to_string() }
    }

    pub fn text(s: &str) -> Value {
        Value::Text { tag: tag::TEXT, text: s.to_string() }
    }

    pub fn charset(s: &str) -> Value {
        Value::Text { tag: tag::CHARSET, text: s.to_string() }
    }

    pub fn natural_language(s: &str) -> Value {
        Value::Text { tag: tag::NATURAL_LANGUAGE, text: s.to_string() }
    }

    pub fn mime(s: &str) -> Value {
        Value::Text { tag: tag::MIME_MEDIA_TYPE, text: s.to_string() }
    }

    pub fn tag(&self) -> u8 {
        match self {
            Value::Integer(_) => tag::INTEGER,
            Value::Boolean(_) => tag::BOOLEAN,
            Value::Enum(_) => tag::ENUM,
            Value::Text { tag, .. } | Value::Other { tag, .. } => *tag,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }

    /// Integers and enums both read as a number (job-state and printer-state are enums).
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Integer(v) | Value::Enum(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    fn encode_bytes(&self) -> Vec<u8> {
        match self {
            Value::Integer(v) | Value::Enum(v) => v.to_be_bytes().to_vec(),
            Value::Boolean(b) => vec![u8::from(*b)],
            Value::Text { text, .. } => text.as_bytes().to_vec(),
            Value::Other { bytes, .. } => bytes.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub tag: u8,
    pub attributes: Vec<Attribute>,
}

impl Group {
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// First value of the named attribute.
    pub fn value(&self, name: &str) -> Option<&Value> {
        self.attribute(name).and_then(|a| a.values.first())
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        self.value(name).and_then(Value::as_str)
    }

    pub fn int(&self, name: &str) -> Option<i32> {
        self.value(name).and_then(Value::as_int)
    }

    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.value(name).and_then(Value::as_bool)
    }
}

/// An IPP request. The operation group is always first; [`Request::with`] adds attributes to a group.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub operation: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
    /// Document bytes sent after the end-of-attributes tag (Print-Job).
    pub document: Vec<u8>,
}

impl Request {
    pub fn new(operation: u16) -> Request {
        Request {
            operation,
            request_id: 1,
            groups: vec![Group { tag: group::OPERATION, attributes: Vec::new() }],
            document: Vec::new(),
        }
    }

    /// Adds one attribute with the given values to the group (created after the existing ones if missing).
    pub fn with(mut self, group_tag: u8, name: &str, values: Vec<Value>) -> Request {
        let index = match self.groups.iter().position(|g| g.tag == group_tag) {
            Some(i) => i,
            None => {
                self.groups.push(Group { tag: group_tag, attributes: Vec::new() });
                self.groups.len() - 1
            }
        };
        self.groups[index].attributes.push(Attribute { name: name.to_string(), values });
        self
    }

    pub fn document(mut self, bytes: Vec<u8>) -> Request {
        self.document = bytes;
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![VERSION_MAJOR, VERSION_MINOR];
        out.extend_from_slice(&self.operation.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        for g in &self.groups {
            out.push(g.tag);
            for attr in &g.attributes {
                for (i, value) in attr.values.iter().enumerate() {
                    // Additional values of one attribute have an empty name (RFC 8010 section 3.1.3).
                    let name: &[u8] = if i == 0 { attr.name.as_bytes() } else { &[] };
                    let bytes = value.encode_bytes();
                    out.push(value.tag());
                    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
                    out.extend_from_slice(name);
                    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
                    out.extend_from_slice(&bytes);
                }
            }
        }
        out.push(END_OF_ATTRIBUTES);
        out.extend_from_slice(&self.document);
        out
    }
}

/// A decoded IPP response. For a response `status` is the status code; the operation field holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub version: (u8, u8),
    pub status: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
}

impl Response {
    pub fn decode(buf: &[u8]) -> Result<Response, IppError> {
        let mut r = Reader { buf, pos: 0 };
        let major = r.u8()?;
        let minor = r.u8()?;
        let status = r.u16()?;
        let request_id = r.u32()?;
        let mut groups: Vec<Group> = Vec::new();
        loop {
            let t = r.u8()?;
            if t == END_OF_ATTRIBUTES {
                break;
            }
            if t == 0 {
                return Err(IppError::Malformed("zero tag"));
            }
            if t < 0x10 {
                groups.push(Group { tag: t, attributes: Vec::new() });
                continue;
            }
            let name_len = r.u16()? as usize;
            let name = String::from_utf8_lossy(r.take(name_len)?).into_owned();
            let value_len = r.u16()? as usize;
            let value = decode_value(t, r.take(value_len)?)?;
            let group = groups.last_mut().ok_or(IppError::Malformed("attribute before a group"))?;
            if name.is_empty() {
                let attr = group
                    .attributes
                    .last_mut()
                    .ok_or(IppError::Malformed("additional value without an attribute"))?;
                attr.values.push(value);
            } else {
                group.attributes.push(Attribute { name, values: vec![value] });
            }
        }
        Ok(Response { version: (major, minor), status, request_id, groups })
    }

    pub fn is_success(&self) -> bool {
        is_success(self.status)
    }

    /// All groups with the given delimiter tag, in order (for example every printer-attributes group).
    pub fn groups_with(&self, group_tag: u8) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |g| g.tag == group_tag)
    }
}

fn is_text_tag(t: u8) -> bool {
    matches!(
        t,
        tag::TEXT
            | tag::NAME
            | tag::KEYWORD
            | tag::URI
            | tag::URI_SCHEME
            | tag::CHARSET
            | tag::NATURAL_LANGUAGE
            | tag::MIME_MEDIA_TYPE
    )
}

fn decode_value(t: u8, raw: &[u8]) -> Result<Value, IppError> {
    match t {
        tag::INTEGER => Ok(Value::Integer(i32::from_be_bytes(fixed::<4>(raw)?))),
        tag::ENUM => Ok(Value::Enum(i32::from_be_bytes(fixed::<4>(raw)?))),
        tag::BOOLEAN => Ok(Value::Boolean(fixed::<1>(raw)?[0] != 0)),
        tag::TEXT_WITH_LANGUAGE | tag::NAME_WITH_LANGUAGE => {
            // language-length, language, text-length, text (RFC 8010 section 3.9).
            let mut r = Reader { buf: raw, pos: 0 };
            let lang_len = r.u16()? as usize;
            r.take(lang_len)?;
            let text_len = r.u16()? as usize;
            let text = String::from_utf8_lossy(r.take(text_len)?).into_owned();
            Ok(Value::Text { tag: t, text })
        }
        _ if is_text_tag(t) => Ok(Value::Text { tag: t, text: String::from_utf8_lossy(raw).into_owned() }),
        _ => Ok(Value::Other { tag: t, bytes: raw.to_vec() }),
    }
}

fn fixed<const N: usize>(raw: &[u8]) -> Result<[u8; N], IppError> {
    <[u8; N]>::try_from(raw).map_err(|_| IppError::Malformed("value has the wrong length"))
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], IppError> {
        if self.buf.len() - self.pos < n {
            return Err(IppError::Truncated);
        }
        let buf: &'a [u8] = self.buf;
        let slice = &buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, IppError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, IppError> {
        Ok(u16::from_be_bytes(fixed::<2>(self.take(2)?)?))
    }

    fn u32(&mut self) -> Result<u32, IppError> {
        Ok(u32::from_be_bytes(fixed::<4>(self.take(4)?)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_operation_group_are_encoded_big_endian() {
        let req = Request::new(op::GET_PRINTER_ATTRIBUTES).with(group::OPERATION, "attributes-charset", vec![Value::charset("utf-8")]);
        let bytes = req.encode();
        assert_eq!(&bytes[..8], &[2, 0, 0x00, 0x0B, 0, 0, 0, 1]);
        assert_eq!(bytes[8], group::OPERATION);
        assert_eq!(bytes[9], tag::CHARSET);
        let name_len = u16::from_be_bytes([bytes[10], bytes[11]]) as usize;
        assert_eq!(name_len, "attributes-charset".len());
        assert_eq!(&bytes[12..12 + name_len], b"attributes-charset");
        let value_at = 12 + name_len;
        assert_eq!(&bytes[value_at..value_at + 2], &[0, 5]);
        assert_eq!(&bytes[value_at + 2..value_at + 7], b"utf-8");
        assert_eq!(*bytes.last().unwrap(), END_OF_ATTRIBUTES);
    }

    #[test]
    fn end_tag_precedes_document_bytes() {
        let bytes = Request::new(op::PRINT_JOB).document(b"%PDF".to_vec()).encode();
        let end = bytes.len() - 4 - 1;
        assert_eq!(bytes[end], END_OF_ATTRIBUTES);
        assert_eq!(&bytes[end + 1..], b"%PDF");
    }

    #[test]
    fn request_round_trips_through_decode() {
        let req = Request::new(op::CUPS_GET_PRINTERS)
            .with(group::OPERATION, "requested-attributes", vec![Value::keyword("printer-name"), Value::keyword("printer-state")])
            .with(group::OPERATION, "limit", vec![Value::Integer(-7)])
            .with(group::OPERATION, "ok", vec![Value::Boolean(true)])
            .with(group::JOB, "job-state", vec![Value::Enum(5)]);
        let decoded = Response::decode(&req.encode()).unwrap();
        assert_eq!(decoded.status, op::CUPS_GET_PRINTERS);
        assert_eq!(decoded.request_id, 1);
        let op_group = &decoded.groups[0];
        assert_eq!(op_group.tag, group::OPERATION);
        let requested = op_group.attribute("requested-attributes").unwrap();
        assert_eq!(requested.values, vec![Value::keyword("printer-name"), Value::keyword("printer-state")]);
        assert_eq!(op_group.int("limit"), Some(-7));
        assert_eq!(op_group.boolean("ok"), Some(true));
        assert_eq!(decoded.groups[1].int("job-state"), Some(5));
    }

    #[test]
    fn additional_values_join_their_attribute() {
        let mut bytes = vec![2, 0, 0, 0, 0, 0, 0, 7, group::PRINTER];
        for (i, v) in ["a", "bb"].iter().enumerate() {
            bytes.push(tag::KEYWORD);
            let name: &[u8] = if i == 0 { b"x" } else { b"" };
            bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&(v.len() as u16).to_be_bytes());
            bytes.extend_from_slice(v.as_bytes());
        }
        bytes.push(END_OF_ATTRIBUTES);
        let resp = Response::decode(&bytes).unwrap();
        assert_eq!(resp.request_id, 7);
        let attr = resp.groups[0].attribute("x").unwrap();
        assert_eq!(attr.values, vec![Value::keyword("a"), Value::keyword("bb")]);
    }

    #[test]
    fn truncated_input_is_reported() {
        let full = Request::new(op::GET_JOBS).with(group::OPERATION, "printer-uri", vec![Value::uri("ipp://localhost/printers/x")]).encode();
        for cut in [0, 3, 9, full.len() - 1] {
            assert_eq!(Response::decode(&full[..cut]).err(), Some(IppError::Truncated), "cut at {cut}");
        }
    }

    #[test]
    fn attribute_before_group_is_malformed() {
        let bytes = [2, 0, 0, 0, 0, 0, 0, 1, tag::KEYWORD, 0, 1, b'x', 0, 0, END_OF_ATTRIBUTES];
        assert_eq!(Response::decode(&bytes).err(), Some(IppError::Malformed("attribute before a group")));
    }

    #[test]
    fn integer_with_wrong_length_is_malformed() {
        let bytes = [2, 0, 0, 0, 0, 0, 0, 1, group::PRINTER, tag::INTEGER, 0, 1, b'n', 0, 2, 0, 1, END_OF_ATTRIBUTES];
        assert!(matches!(Response::decode(&bytes), Err(IppError::Malformed(_))));
    }

    #[test]
    fn success_is_below_client_error_range() {
        assert!(is_success(0x0000));
        assert!(is_success(0x0001));
        assert!(!is_success(STATUS_NOT_FOUND));
        assert!(!is_success(0x0500));
    }
}
