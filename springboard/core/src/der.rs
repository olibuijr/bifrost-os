//! ASN.1 DER (ITU-T X.690 distinguished encoding rules): a strict reader and a writer (std only).
//!
//! The reader accepts only the canonical encodings a certificate, a key or a signed structure carries: single-byte
//! tags, definite lengths in the shortest form, minimal integers, booleans of 0x00 or 0xFF, and bit strings whose
//! unused bits are in range. Everything it hands back borrows from the input. Nesting is bounded so a hostile blob
//! cannot recurse without limit. The writer builds the same encodings. Object identifiers are `crate::oid::Oid`;
//! times are Unix seconds (UTC) and the writer picks UTCTime or GeneralizedTime by year as RFC 5280 does.
//! This is the codec behind `crate::x509` and `crate::crlite`; it opens no file and checks no signature.

use crate::oid::{Oid, OidError};
use crate::snapshot_backup::{civil, from_civil};
use std::fmt;

pub const BOOLEAN: u8 = 0x01;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OBJECT_IDENTIFIER: u8 = 0x06;
pub const UTF8_STRING: u8 = 0x0c;
pub const PRINTABLE_STRING: u8 = 0x13;
pub const TELETEX_STRING: u8 = 0x14;
pub const IA5_STRING: u8 = 0x16;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;
pub const VISIBLE_STRING: u8 = 0x1a;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;

/// The deepest nesting a reader will descend into.
pub const MAX_DEPTH: u8 = 32;

/// The tag byte of a context-specific value `[n]` (n below 31). Constructed values are the explicit tags.
pub fn context_tag(n: u8, constructed: bool) -> u8 {
    0x80 | (n & 0x1f) | if constructed { 0x20 } else { 0 }
}

/// Why a value was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerError {
    /// The input ends inside a value.
    Truncated,
    /// A length that could have been written shorter.
    NonMinimalLength,
    /// The indefinite length form (0x80), which DER forbids.
    IndefiniteLength,
    /// A length that needs more than four bytes.
    LengthTooLarge,
    /// A tag number of 31 or more (multi-byte tag).
    HighTagNumber,
    /// Bytes remain after the last value.
    TrailingData,
    /// The value has another tag than the one asked for.
    UnexpectedTag { expected: u8, found: u8 },
    /// An integer with a redundant leading byte.
    NonMinimalInteger,
    /// A negative integer where an unsigned one is needed.
    NegativeInteger,
    /// An integer too large for the type asked for.
    IntegerTooLarge,
    /// A boolean that is not 0x00 or 0xFF.
    InvalidBoolean,
    /// A bit string with no unused-bits byte, or unused bits out of range.
    InvalidBitString,
    /// A string with bytes its type does not allow.
    InvalidString,
    /// A time that is malformed, impossible or before 1970.
    InvalidTime,
    /// Nesting beyond `MAX_DEPTH`.
    TooDeep,
    /// A bad object identifier.
    Oid(OidError),
}

impl fmt::Display for DerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DerError::Truncated => f.write_str("value is cut off"),
            DerError::NonMinimalLength => f.write_str("length is not in the shortest form"),
            DerError::IndefiniteLength => f.write_str("indefinite length is not allowed in DER"),
            DerError::LengthTooLarge => f.write_str("length needs more than four bytes"),
            DerError::HighTagNumber => f.write_str("multi-byte tags are not supported"),
            DerError::TrailingData => f.write_str("bytes follow the last value"),
            DerError::UnexpectedTag { expected, found } => write!(f, "expected tag {expected:#04x}, found {found:#04x}"),
            DerError::NonMinimalInteger => f.write_str("integer has a redundant leading byte"),
            DerError::NegativeInteger => f.write_str("integer is negative"),
            DerError::IntegerTooLarge => f.write_str("integer is too large"),
            DerError::InvalidBoolean => f.write_str("boolean is neither 0x00 nor 0xff"),
            DerError::InvalidBitString => f.write_str("bit string is malformed"),
            DerError::InvalidString => f.write_str("string holds bytes its type does not allow"),
            DerError::InvalidTime => f.write_str("time is malformed"),
            DerError::TooDeep => f.write_str("nesting is too deep"),
            DerError::Oid(e) => write!(f, "object identifier: {e}"),
        }
    }
}

impl std::error::Error for DerError {}

impl From<OidError> for DerError {
    fn from(e: OidError) -> Self {
        DerError::Oid(e)
    }
}

/// One tag-length-value, borrowed from the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    /// The content octets, without tag and length.
    pub content: &'a [u8],
    /// The whole encoding, tag and length included (what a signature covers).
    pub raw: &'a [u8],
    depth: u8,
}

/// Reads consecutive values from a byte slice.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    depth: u8,
}

/// The tag, header length and content length at the start of `data`.
fn parse_header(data: &[u8]) -> Result<(u8, usize, usize), DerError> {
    let tag = *data.first().ok_or(DerError::Truncated)?;
    if tag & 0x1f == 0x1f {
        return Err(DerError::HighTagNumber);
    }
    let first = *data.get(1).ok_or(DerError::Truncated)?;
    if first < 0x80 {
        return Ok((tag, 2, usize::from(first)));
    }
    if first == 0x80 {
        return Err(DerError::IndefiniteLength);
    }
    let n = usize::from(first & 0x7f);
    if n > 4 {
        return Err(DerError::LengthTooLarge);
    }
    let bytes = data.get(2..2 + n).ok_or(DerError::Truncated)?;
    if bytes[0] == 0 {
        return Err(DerError::NonMinimalLength);
    }
    let len = bytes.iter().fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
    if len < 0x80 {
        return Err(DerError::NonMinimalLength);
    }
    Ok((tag, 2 + n, len))
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0, depth: 0 }
    }

    /// True when every byte has been read.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// The tag of the next value, without reading it.
    pub fn peek_tag(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Reads the next value of any tag.
    pub fn read(&mut self) -> Result<Tlv<'a>, DerError> {
        let data: &'a [u8] = self.data;
        let rest = data.get(self.pos..).unwrap_or(&[]);
        let (tag, header, len) = parse_header(rest)?;
        let end = header.checked_add(len).ok_or(DerError::LengthTooLarge)?;
        if end > rest.len() {
            return Err(DerError::Truncated);
        }
        self.pos += end;
        Ok(Tlv { tag, content: &rest[header..end], raw: &rest[..end], depth: self.depth })
    }

    /// Reads the next value, which must have `tag`.
    pub fn expect(&mut self, tag: u8) -> Result<Tlv<'a>, DerError> {
        let tlv = self.read()?;
        if tlv.tag != tag {
            return Err(DerError::UnexpectedTag { expected: tag, found: tlv.tag });
        }
        Ok(tlv)
    }

    /// Reads the next value when it has `tag`; leaves the reader alone otherwise (an OPTIONAL field).
    pub fn optional(&mut self, tag: u8) -> Result<Option<Tlv<'a>>, DerError> {
        if self.peek_tag() == Some(tag) {
            self.read().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reads a SEQUENCE and opens its contents.
    pub fn sequence(&mut self) -> Result<Reader<'a>, DerError> {
        self.expect(SEQUENCE)?.reader()
    }

    /// Fails unless every byte was read.
    pub fn finish(&self) -> Result<(), DerError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(DerError::TrailingData)
        }
    }
}

impl<'a> Tlv<'a> {
    /// True for a constructed encoding (bit 6 of the tag).
    pub fn is_constructed(&self) -> bool {
        self.tag & 0x20 != 0
    }

    /// Opens the content as a run of values, one level deeper.
    pub fn reader(&self) -> Result<Reader<'a>, DerError> {
        if self.depth + 1 > MAX_DEPTH {
            return Err(DerError::TooDeep);
        }
        Ok(Reader { data: self.content, pos: 0, depth: self.depth + 1 })
    }

    fn check(&self, tag: u8) -> Result<(), DerError> {
        if self.tag == tag {
            Ok(())
        } else {
            Err(DerError::UnexpectedTag { expected: tag, found: self.tag })
        }
    }

    pub fn as_bool(&self) -> Result<bool, DerError> {
        self.check(BOOLEAN)?;
        match self.content {
            [0x00] => Ok(false),
            [0xff] => Ok(true),
            _ => Err(DerError::InvalidBoolean),
        }
    }

    /// The magnitude of a non-negative INTEGER, big-endian, without a leading zero byte (zero is `[0]`).
    pub fn as_unsigned_bytes(&self) -> Result<&'a [u8], DerError> {
        self.check(INTEGER)?;
        let c = self.content;
        if c.is_empty() {
            return Err(DerError::Truncated);
        }
        if c.len() > 1 && ((c[0] == 0x00 && c[1] & 0x80 == 0) || (c[0] == 0xff && c[1] & 0x80 != 0)) {
            return Err(DerError::NonMinimalInteger);
        }
        if c[0] & 0x80 != 0 {
            return Err(DerError::NegativeInteger);
        }
        Ok(if c.len() > 1 && c[0] == 0 { &c[1..] } else { c })
    }

    pub fn as_u64(&self) -> Result<u64, DerError> {
        let m = self.as_unsigned_bytes()?;
        if m.len() > 8 {
            return Err(DerError::IntegerTooLarge);
        }
        Ok(m.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
    }

    pub fn as_oid(&self) -> Result<Oid, DerError> {
        self.check(OBJECT_IDENTIFIER)?;
        Ok(Oid::from_der_content(self.content)?)
    }

    pub fn as_octets(&self) -> Result<&'a [u8], DerError> {
        self.check(OCTET_STRING)?;
        Ok(self.content)
    }

    pub fn as_null(&self) -> Result<(), DerError> {
        self.check(NULL)?;
        if self.content.is_empty() {
            Ok(())
        } else {
            Err(DerError::TrailingData)
        }
    }

    /// A BIT STRING as `(unused bits in the last byte, the bytes)`.
    pub fn as_bit_string(&self) -> Result<(u8, &'a [u8]), DerError> {
        self.check(BIT_STRING)?;
        let (&unused, bytes) = self.content.split_first().ok_or(DerError::InvalidBitString)?;
        if unused > 7 || (bytes.is_empty() && unused != 0) {
            return Err(DerError::InvalidBitString);
        }
        Ok((unused, bytes))
    }

    /// Text of a UTF8String, PrintableString, IA5String, VisibleString or TeletexString (read as Latin-1 is not
    /// attempted: Teletex must be ASCII here).
    pub fn as_string(&self) -> Result<&'a str, DerError> {
        let ascii_only = |c: &[u8]| c.iter().all(|b| b.is_ascii());
        match self.tag {
            UTF8_STRING => {}
            PRINTABLE_STRING | IA5_STRING | VISIBLE_STRING | TELETEX_STRING if ascii_only(self.content) => {}
            PRINTABLE_STRING | IA5_STRING | VISIBLE_STRING | TELETEX_STRING => return Err(DerError::InvalidString),
            found => return Err(DerError::UnexpectedTag { expected: UTF8_STRING, found }),
        }
        std::str::from_utf8(self.content).map_err(|_| DerError::InvalidString)
    }

    /// A UTCTime (`YYMMDDHHMMSSZ`, years 1950 to 2049) or GeneralizedTime (`YYYYMMDDHHMMSSZ`) as Unix seconds.
    pub fn as_time(&self) -> Result<u64, DerError> {
        let c = self.content;
        let digits = |s: &[u8]| -> Result<u32, DerError> {
            if s.iter().all(u8::is_ascii_digit) {
                Ok(s.iter().fold(0u32, |a, &b| a * 10 + u32::from(b - b'0')))
            } else {
                Err(DerError::InvalidTime)
            }
        };
        let (year, rest) = match self.tag {
            UTC_TIME if c.len() == 13 => {
                let yy = digits(&c[0..2])?;
                (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &c[2..])
            }
            GENERALIZED_TIME if c.len() == 15 => (digits(&c[0..4])?, &c[4..]),
            UTC_TIME | GENERALIZED_TIME => return Err(DerError::InvalidTime),
            found => return Err(DerError::UnexpectedTag { expected: UTC_TIME, found }),
        };
        if rest[10] != b'Z' {
            return Err(DerError::InvalidTime);
        }
        let (month, day, hour, minute, second) = (digits(&rest[0..2])?, digits(&rest[2..4])?, digits(&rest[4..6])?, digits(&rest[6..8])?, digits(&rest[8..10])?);
        if day == 0 || day > days_in_month(i64::from(year), month) {
            return Err(DerError::InvalidTime);
        }
        from_civil(i64::from(year), month, day, hour, minute, second).ok_or(DerError::InvalidTime)
    }
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Reads exactly one value from `data`, which must have no bytes after it.
pub fn parse_one(data: &[u8]) -> Result<Tlv<'_>, DerError> {
    let mut r = Reader::new(data);
    let tlv = r.read()?;
    r.finish()?;
    Ok(tlv)
}

// -- Writing ------------------------------------------------------------------------------------------------

fn push_length(out: &mut Vec<u8>, n: usize) {
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = n.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

/// One value: `tag`, the length of `content`, then `content`.
pub fn encode(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    push_length(&mut out, content.len());
    out.extend_from_slice(content);
    out
}

/// An INTEGER from unsigned big-endian bytes (leading zeros dropped, a zero byte added when the top bit is set).
pub fn integer_unsigned(be: &[u8]) -> Vec<u8> {
    let skip = be.iter().take_while(|b| **b == 0).count();
    let mag = &be[skip..];
    let mut content = Vec::with_capacity(mag.len() + 1);
    if mag.is_empty() || mag[0] & 0x80 != 0 {
        content.push(0);
    }
    content.extend_from_slice(mag);
    encode(INTEGER, &content)
}

pub fn integer_u64(v: u64) -> Vec<u8> {
    integer_unsigned(&v.to_be_bytes())
}

pub fn boolean(v: bool) -> Vec<u8> {
    encode(BOOLEAN, &[if v { 0xff } else { 0x00 }])
}

pub fn null() -> Vec<u8> {
    encode(NULL, &[])
}

pub fn oid(o: &Oid) -> Result<Vec<u8>, DerError> {
    Ok(encode(OBJECT_IDENTIFIER, &o.to_der_content()?))
}

pub fn octet_string(bytes: &[u8]) -> Vec<u8> {
    encode(OCTET_STRING, bytes)
}

pub fn bit_string(unused: u8, bytes: &[u8]) -> Vec<u8> {
    let mut content = Vec::with_capacity(bytes.len() + 1);
    content.push(unused);
    content.extend_from_slice(bytes);
    encode(BIT_STRING, &content)
}

pub fn utf8_string(s: &str) -> Vec<u8> {
    encode(UTF8_STRING, s.as_bytes())
}

pub fn sequence(parts: &[Vec<u8>]) -> Vec<u8> {
    encode(SEQUENCE, &parts.concat())
}

pub fn set(parts: &[Vec<u8>]) -> Vec<u8> {
    encode(SET, &parts.concat())
}

/// An explicit context tag `[n]` around an encoded value.
pub fn explicit(n: u8, inner: &[u8]) -> Vec<u8> {
    encode(context_tag(n, true), inner)
}

/// A time for a validity field: UTCTime for 1950 to 2049, GeneralizedTime after, as RFC 5280 requires.
pub fn time(unix: u64) -> Vec<u8> {
    let (y, mo, d, h, mi, s) = civil(unix);
    if y < 2050 {
        encode(UTC_TIME, format!("{:02}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z", y % 100).as_bytes())
    } else {
        encode(GENERALIZED_TIME, format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z").as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_long_lengths_round_trip() {
        let short = encode(OCTET_STRING, &[1, 2, 3]);
        assert_eq!(short, vec![0x04, 3, 1, 2, 3]);
        let big = vec![7u8; 300];
        let long = encode(OCTET_STRING, &big);
        assert_eq!(&long[..4], &[0x04, 0x82, 0x01, 0x2c]);
        assert_eq!(parse_one(&long).unwrap().content, &big[..]);
        assert_eq!(parse_one(&short).unwrap().as_octets().unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn non_canonical_lengths_are_refused() {
        // 0x81 0x05: a long form for a value that fits the short form.
        assert_eq!(parse_one(&[0x04, 0x81, 0x05, 1, 2, 3, 4, 5]).unwrap_err(), DerError::NonMinimalLength);
        // leading zero length byte
        let mut v = vec![0x04, 0x82, 0x00, 0x80];
        v.extend(vec![0u8; 128]);
        assert_eq!(parse_one(&v).unwrap_err(), DerError::NonMinimalLength);
        assert_eq!(parse_one(&[0x30, 0x80, 0x00, 0x00]).unwrap_err(), DerError::IndefiniteLength);
        assert_eq!(parse_one(&[0x04, 0x85, 1, 0, 0, 0, 0]).unwrap_err(), DerError::LengthTooLarge);
        assert_eq!(parse_one(&[0x1f, 0x01, 0x00]).unwrap_err(), DerError::HighTagNumber);
    }

    #[test]
    fn truncated_and_trailing_input_is_refused() {
        assert_eq!(parse_one(&[0x04, 0x05, 1, 2]).unwrap_err(), DerError::Truncated);
        assert_eq!(parse_one(&[0x04]).unwrap_err(), DerError::Truncated);
        assert_eq!(parse_one(&[0x05, 0x00, 0x00]).unwrap_err(), DerError::TrailingData);
    }

    #[test]
    fn integers_are_minimal_and_unsigned_where_asked() {
        assert_eq!(integer_u64(0), vec![0x02, 0x01, 0x00]);
        assert_eq!(integer_u64(127), vec![0x02, 0x01, 0x7f]);
        assert_eq!(integer_u64(128), vec![0x02, 0x02, 0x00, 0x80]);
        assert_eq!(integer_unsigned(&[0, 0, 0xff, 0x01]), vec![0x02, 0x03, 0x00, 0xff, 0x01]);
        for v in [0u64, 1, 127, 128, 255, 256, 65_535, u64::MAX] {
            assert_eq!(parse_one(&integer_u64(v)).unwrap().as_u64().unwrap(), v);
        }
        assert_eq!(parse_one(&[0x02, 0x02, 0x00, 0x01]).unwrap().as_u64().unwrap_err(), DerError::NonMinimalInteger);
        assert_eq!(parse_one(&[0x02, 0x01, 0x80]).unwrap().as_u64().unwrap_err(), DerError::NegativeInteger);
        assert_eq!(parse_one(&[0x02, 0x00]).unwrap().as_u64().unwrap_err(), DerError::Truncated);
        let nine = integer_unsigned(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(parse_one(&nine).unwrap().as_u64().unwrap_err(), DerError::IntegerTooLarge);
    }

    #[test]
    fn booleans_null_and_bit_strings() {
        assert_eq!(parse_one(&boolean(true)).unwrap().as_bool(), Ok(true));
        assert_eq!(parse_one(&boolean(false)).unwrap().as_bool(), Ok(false));
        assert_eq!(parse_one(&[0x01, 0x01, 0x01]).unwrap().as_bool().unwrap_err(), DerError::InvalidBoolean);
        assert_eq!(parse_one(&null()).unwrap().as_null(), Ok(()));
        let bits = bit_string(3, &[0b1010_0000]);
        assert_eq!(parse_one(&bits).unwrap().as_bit_string().unwrap(), (3, &[0b1010_0000][..]));
        assert_eq!(parse_one(&[0x03, 0x01, 0x04]).unwrap().as_bit_string().unwrap_err(), DerError::InvalidBitString);
        assert_eq!(parse_one(&[0x03, 0x02, 0x08, 0x00]).unwrap().as_bit_string().unwrap_err(), DerError::InvalidBitString);
        assert_eq!(parse_one(&[0x03, 0x00]).unwrap().as_bit_string().unwrap_err(), DerError::InvalidBitString);
    }

    #[test]
    fn object_identifiers_and_strings() {
        let o = Oid::parse("2.5.29.19").unwrap();
        let der = oid(&o).unwrap();
        assert_eq!(der, vec![0x06, 0x03, 0x55, 0x1d, 0x13]);
        assert_eq!(parse_one(&der).unwrap().as_oid().unwrap(), o);
        assert_eq!(parse_one(&utf8_string("Þór")).unwrap().as_string().unwrap(), "Þór");
        assert_eq!(parse_one(&encode(IA5_STRING, b"a.example")).unwrap().as_string().unwrap(), "a.example");
        assert_eq!(parse_one(&encode(IA5_STRING, &[0xc3, 0xa9])).unwrap().as_string().unwrap_err(), DerError::InvalidString);
        assert_eq!(parse_one(&[0x0c, 0x01, 0xff]).unwrap().as_string().unwrap_err(), DerError::InvalidString);
        assert!(matches!(parse_one(&null()).unwrap().as_string(), Err(DerError::UnexpectedTag { .. })));
    }

    #[test]
    fn times_round_trip_and_pick_the_right_form() {
        // 2026-10-17 12:30:45 UTC
        let t = from_civil(2026, 10, 17, 12, 30, 45).unwrap();
        let der = time(t);
        assert_eq!(der[0], UTC_TIME);
        assert_eq!(&der[2..], b"261017123045Z");
        assert_eq!(parse_one(&der).unwrap().as_time().unwrap(), t);
        let far = from_civil(2051, 1, 2, 3, 4, 5).unwrap();
        let der = time(far);
        assert_eq!(der[0], GENERALIZED_TIME);
        assert_eq!(parse_one(&der).unwrap().as_time().unwrap(), far);
        // two-digit years: 49 is 2049, 50 is 1950 (before 1970 is refused)
        assert!(parse_one(&encode(UTC_TIME, b"490101000000Z")).unwrap().as_time().is_ok());
        assert_eq!(parse_one(&encode(UTC_TIME, b"500101000000Z")).unwrap().as_time().unwrap_err(), DerError::InvalidTime);
    }

    #[test]
    fn impossible_times_are_refused() {
        for bad in [&b"260230000000Z"[..], b"261301000000Z", b"261017240000Z", b"261017126000Z", b"2610171230450", b"26101712304Z", b"26ab17123045Z"] {
            let got = parse_one(&encode(UTC_TIME, bad)).unwrap().as_time();
            assert_eq!(got, Err(DerError::InvalidTime), "{}", String::from_utf8_lossy(bad));
        }
        // 2024 is a leap year, 2100 (generalized) is not
        assert!(parse_one(&encode(UTC_TIME, b"240229000000Z")).unwrap().as_time().is_ok());
        assert_eq!(parse_one(&encode(GENERALIZED_TIME, b"21000229000000Z")).unwrap().as_time(), Err(DerError::InvalidTime));
    }

    #[test]
    fn sequences_read_in_order_with_optional_fields() {
        let der = sequence(&[explicit(0, &integer_u64(2)), integer_u64(9), set(&[boolean(true)])]);
        let mut top = Reader::new(&der);
        let mut seq = top.sequence().unwrap();
        top.finish().unwrap();
        let version = seq.optional(context_tag(0, true)).unwrap().unwrap();
        assert_eq!(version.reader().unwrap().expect(INTEGER).unwrap().as_u64().unwrap(), 2);
        assert_eq!(seq.optional(context_tag(1, true)).unwrap(), None);
        assert_eq!(seq.expect(INTEGER).unwrap().as_u64().unwrap(), 9);
        let inner = seq.expect(SET).unwrap();
        assert!(inner.is_constructed());
        assert_eq!(inner.reader().unwrap().expect(BOOLEAN).unwrap().as_bool(), Ok(true));
        seq.finish().unwrap();
        assert_eq!(seq.expect(INTEGER).unwrap_err(), DerError::Truncated);
    }

    #[test]
    fn an_unexpected_tag_is_named() {
        let der = integer_u64(1);
        let mut r = Reader::new(&der);
        assert_eq!(r.expect(SEQUENCE).unwrap_err(), DerError::UnexpectedTag { expected: SEQUENCE, found: INTEGER });
    }

    #[test]
    fn nesting_is_bounded() {
        let mut der = null();
        for _ in 0..(MAX_DEPTH as usize + 4) {
            der = sequence(&[der]);
        }
        let mut tlv = parse_one(&der).unwrap();
        let mut levels = 0;
        let err = loop {
            match tlv.reader().and_then(|mut r| r.read()) {
                Ok(next) => {
                    tlv = next;
                    levels += 1;
                }
                Err(e) => break e,
            }
        };
        assert_eq!(err, DerError::TooDeep);
        assert_eq!(levels, MAX_DEPTH as usize);
    }
}
