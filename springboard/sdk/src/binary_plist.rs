//! Binary property lists (`bplist00`): the compact on-disk form of a property list, read and written.
//!
//! The value model keeps what the format can hold: null, booleans, integers (one to eight bytes, and sixteen on
//! read), reals, dates (seconds since 2001-01-01, the reference date), data, strings, `Uid` references (used by
//! keyed archives), arrays and dictionaries with string keys. Sets read back as arrays.
//!
//! Strings are written as ASCII when every character is ASCII and as UTF-16 big endian otherwise. Integers use the
//! smallest unsigned width that fits; a negative integer is always eight bytes. Object references and the offset
//! table use the smallest width that fits. The top object is written first, then children in order, so a small
//! dictionary comes out byte for byte as the reference writer lays it out.
//!
//! The reader checks every offset and length against the file, refuses nesting deeper than `MAX_DEPTH` (which also
//! stops reference cycles) and never allocates from an unchecked count.

use crate::coding::Value;

const MAGIC: &[u8] = b"bplist00";
const TRAILER: usize = 32;
/// Deepest nesting the reader follows.
pub const MAX_DEPTH: usize = 512;
/// Seconds from the Unix epoch to 2001-01-01T00:00:00Z, the date origin of the format.
pub const REFERENCE_DATE_OFFSET: f64 = 978_307_200.0;

/// One property-list value.
#[derive(Clone, Debug, PartialEq)]
pub enum BValue {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// Seconds since 2001-01-01.
    Date(f64),
    Data(Vec<u8>),
    Str(String),
    /// A reference into a keyed archive's object table.
    Uid(u64),
    Array(Vec<BValue>),
    Dict(Vec<(String, BValue)>),
}

impl BValue {
    /// A date from seconds since 1970.
    pub fn date_from_unix(secs: f64) -> BValue {
        BValue::Date(secs - REFERENCE_DATE_OFFSET)
    }

    /// Seconds since 1970 for a `Date`.
    pub fn unix_seconds(&self) -> Option<f64> {
        if let BValue::Date(d) = self {
            Some(d + REFERENCE_DATE_OFFSET)
        } else {
            None
        }
    }

    /// The entry named `key` of a dictionary.
    pub fn get(&self, key: &str) -> Option<&BValue> {
        match self {
            BValue::Dict(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BplistError(pub String);

impl std::fmt::Display for BplistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "binary property list: {}", self.0)
    }
}

impl std::error::Error for BplistError {}

fn bad<T>(m: &str) -> Result<T, BplistError> {
    Err(BplistError(m.to_string()))
}

// ---- writing ----

enum Node {
    Leaf(Vec<u8>),
    Array(Vec<usize>),
    Dict(Vec<usize>, Vec<usize>),
}

/// Serialises `root` as a binary property list.
pub fn write(root: &BValue) -> Vec<u8> {
    let mut nodes = Vec::new();
    flatten(root, &mut nodes);
    let count = nodes.len();
    let ref_size = byte_width((count - 1) as u64);
    let mut out = MAGIC.to_vec();
    let mut offsets = Vec::with_capacity(count);
    for node in &nodes {
        offsets.push(out.len() as u64);
        match node {
            Node::Leaf(bytes) => out.extend_from_slice(bytes),
            Node::Array(items) => {
                marker(&mut out, 0xA0, items.len());
                for &i in items {
                    push_be(&mut out, i as u64, ref_size);
                }
            }
            Node::Dict(keys, values) => {
                marker(&mut out, 0xD0, keys.len());
                for &i in keys.iter().chain(values.iter()) {
                    push_be(&mut out, i as u64, ref_size);
                }
            }
        }
    }
    let table_offset = out.len() as u64;
    let offset_size = byte_width(table_offset);
    for o in offsets {
        push_be(&mut out, o, offset_size);
    }
    out.extend_from_slice(&[0u8; 6]); // five unused bytes and the sort version
    out.push(offset_size as u8);
    out.push(ref_size as u8);
    out.extend_from_slice(&(count as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes()); // the top object is object 0
    out.extend_from_slice(&table_offset.to_be_bytes());
    out
}

fn flatten(v: &BValue, nodes: &mut Vec<Node>) -> usize {
    let idx = nodes.len();
    match v {
        BValue::Array(items) => {
            nodes.push(Node::Array(Vec::new()));
            let refs: Vec<usize> = items.iter().map(|i| flatten(i, nodes)).collect();
            nodes[idx] = Node::Array(refs);
        }
        BValue::Dict(entries) => {
            nodes.push(Node::Dict(Vec::new(), Vec::new()));
            let mut keys = Vec::with_capacity(entries.len());
            for (k, _) in entries {
                keys.push(nodes.len());
                nodes.push(Node::Leaf(string_bytes(k)));
            }
            let values: Vec<usize> = entries.iter().map(|(_, v)| flatten(v, nodes)).collect();
            nodes[idx] = Node::Dict(keys, values);
        }
        leaf => nodes.push(Node::Leaf(leaf_bytes(leaf))),
    }
    idx
}

fn leaf_bytes(v: &BValue) -> Vec<u8> {
    match v {
        BValue::Null => vec![0x00],
        BValue::Bool(false) => vec![0x08],
        BValue::Bool(true) => vec![0x09],
        BValue::Int(i) => int_bytes(*i),
        BValue::Real(r) => [&[0x23u8][..], &r.to_be_bytes()].concat(),
        BValue::Date(d) => [&[0x33u8][..], &d.to_be_bytes()].concat(),
        BValue::Data(d) => {
            let mut o = Vec::new();
            marker(&mut o, 0x40, d.len());
            o.extend_from_slice(d);
            o
        }
        BValue::Str(s) => string_bytes(s),
        BValue::Uid(u) => {
            let w = byte_width(*u);
            let mut o = vec![0x80 | (w as u8 - 1)];
            push_be(&mut o, *u, w);
            o
        }
        BValue::Array(_) | BValue::Dict(_) => Vec::new(),
    }
}

fn int_bytes(i: i64) -> Vec<u8> {
    if i < 0 {
        return [&[0x13u8][..], &i.to_be_bytes()].concat();
    }
    let w = byte_width(i as u64);
    let mut o = vec![0x10 | w.trailing_zeros() as u8];
    push_be(&mut o, i as u64, w);
    o
}

fn string_bytes(s: &str) -> Vec<u8> {
    let mut o = Vec::new();
    if s.is_ascii() {
        marker(&mut o, 0x50, s.len());
        o.extend_from_slice(s.as_bytes());
    } else {
        let units: Vec<u16> = s.encode_utf16().collect();
        marker(&mut o, 0x60, units.len());
        for u in units {
            o.extend_from_slice(&u.to_be_bytes());
        }
    }
    o
}

/// A type marker with the length in its low nibble, or `0xF` followed by an integer object when it is 15 or more.
fn marker(out: &mut Vec<u8>, base: u8, len: usize) {
    if len < 15 {
        out.push(base | len as u8);
    } else {
        out.push(base | 0x0F);
        out.extend_from_slice(&int_bytes(len as i64));
    }
}

/// The smallest of 1, 2, 4 and 8 bytes that holds `n`.
fn byte_width(n: u64) -> usize {
    match n {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFFFF_FFFF => 4,
        _ => 8,
    }
}

fn push_be(out: &mut Vec<u8>, v: u64, width: usize) {
    out.extend_from_slice(&v.to_be_bytes()[8 - width..]);
}

// ---- reading ----

/// Parses a binary property list.
pub fn read(bytes: &[u8]) -> Result<BValue, BplistError> {
    if bytes.len() < MAGIC.len() + TRAILER || &bytes[..MAGIC.len()] != MAGIC {
        return bad("not a binary property list");
    }
    let limit = bytes.len() - TRAILER;
    let t = &bytes[limit..];
    let offset_size = t[6] as usize;
    let ref_size = t[7] as usize;
    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
        return bad("bad offset or reference size");
    }
    let count = be(&t[8..16]);
    let top = be(&t[16..24]);
    let table = be(&t[24..32]);
    let table_end = count.checked_mul(offset_size as u64).and_then(|l| l.checked_add(table));
    if count == 0 || top >= count || table < MAGIC.len() as u64 || table_end.map_or(true, |e| e > limit as u64) {
        return bad("the offset table is out of range");
    }
    let reader = Reader { b: &bytes[..limit], table: table as usize, offset_size, ref_size, count: count as usize };
    reader.object(top as usize, 0)
}

fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64)
}

struct Reader<'a> {
    b: &'a [u8],
    table: usize,
    offset_size: usize,
    ref_size: usize,
    count: usize,
}

impl Reader<'_> {
    fn slice(&self, start: usize, len: usize) -> Result<&[u8], BplistError> {
        match start.checked_add(len) {
            Some(end) if end <= self.b.len() => Ok(&self.b[start..end]),
            _ => bad("an object runs past the end of the file"),
        }
    }

    /// The marker's length: in the low nibble, or in an integer object that follows. Returns the length and where
    /// the payload starts.
    fn length(&self, off: usize, lo: u8) -> Result<(usize, usize), BplistError> {
        if lo != 0x0F {
            return Ok((lo as usize, off + 1));
        }
        let m = self.slice(off + 1, 1)?[0];
        if m >> 4 != 0x1 || (m & 0x0F) > 3 {
            return bad("a length is not an integer");
        }
        let n = 1usize << (m & 0x0F);
        let v = be(self.slice(off + 2, n)?);
        match usize::try_from(v) {
            Ok(len) => Ok((len, off + 2 + n)),
            Err(_) => bad("a length is too large"),
        }
    }

    /// `len` references starting at `start`, checked to lie inside the file.
    fn refs_len(&self, len: usize, per_item: usize) -> Result<usize, BplistError> {
        match len.checked_mul(per_item) {
            Some(n) => Ok(n),
            None => bad("a length overflows"),
        }
    }

    fn reference(&self, at: usize) -> Result<usize, BplistError> {
        let r = be(self.slice(at, self.ref_size)?) as usize;
        if r >= self.count {
            return bad("a reference names no object");
        }
        Ok(r)
    }

    fn object(&self, idx: usize, depth: usize) -> Result<BValue, BplistError> {
        if depth > MAX_DEPTH {
            return bad("nested too deeply");
        }
        let entry = self.table + idx * self.offset_size;
        let off = be(self.slice(entry, self.offset_size)?) as usize;
        if off < MAGIC.len() {
            return bad("an object starts inside the header");
        }
        let m = self.slice(off, 1)?[0];
        let (hi, lo) = (m >> 4, m & 0x0F);
        match hi {
            0x0 => match lo {
                0x0 => Ok(BValue::Null),
                0x8 => Ok(BValue::Bool(false)),
                0x9 => Ok(BValue::Bool(true)),
                _ => bad("an unknown simple value"),
            },
            0x1 => {
                if lo > 4 {
                    return bad("an integer is too wide");
                }
                let n = 1usize << lo;
                let raw = self.slice(off + 1, n)?;
                Ok(BValue::Int(match n {
                    8 => i64::from_be_bytes(raw.try_into().unwrap()),
                    16 => i64::from_be_bytes(raw[8..].try_into().unwrap()),
                    _ => be(raw) as i64,
                }))
            }
            0x2 => match lo {
                2 => Ok(BValue::Real(f32::from_be_bytes(self.slice(off + 1, 4)?.try_into().unwrap()) as f64)),
                3 => Ok(BValue::Real(f64::from_be_bytes(self.slice(off + 1, 8)?.try_into().unwrap()))),
                _ => bad("an unknown real width"),
            },
            0x3 if lo == 3 => Ok(BValue::Date(f64::from_be_bytes(self.slice(off + 1, 8)?.try_into().unwrap()))),
            0x4 => {
                let (len, start) = self.length(off, lo)?;
                Ok(BValue::Data(self.slice(start, len)?.to_vec()))
            }
            0x5 | 0x7 => {
                let (len, start) = self.length(off, lo)?;
                let raw = self.slice(start, len)?;
                match String::from_utf8(raw.to_vec()) {
                    Ok(s) => Ok(BValue::Str(s)),
                    Err(_) => bad("a string is not valid text"),
                }
            }
            0x6 => {
                let (len, start) = self.length(off, lo)?;
                let raw = self.slice(start, self.refs_len(len, 2)?)?;
                let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                match String::from_utf16(&units) {
                    Ok(s) => Ok(BValue::Str(s)),
                    Err(_) => bad("a string is not valid text"),
                }
            }
            0x8 => Ok(BValue::Uid(be(self.slice(off + 1, lo as usize + 1)?))),
            0xA | 0xC => {
                let (len, start) = self.length(off, lo)?;
                self.slice(start, self.refs_len(len, self.ref_size)?)?;
                let mut items = Vec::with_capacity(len.min(1024));
                for i in 0..len {
                    items.push(self.object(self.reference(start + i * self.ref_size)?, depth + 1)?);
                }
                Ok(BValue::Array(items))
            }
            0xD => {
                let (len, start) = self.length(off, lo)?;
                self.slice(start, self.refs_len(len, 2 * self.ref_size)?)?;
                let mut entries = Vec::with_capacity(len.min(1024));
                for i in 0..len {
                    let key = self.object(self.reference(start + i * self.ref_size)?, depth + 1)?;
                    let value = self.object(self.reference(start + (len + i) * self.ref_size)?, depth + 1)?;
                    match key {
                        BValue::Str(k) => entries.push((k, value)),
                        _ => return bad("a dictionary key is not a string"),
                    }
                }
                Ok(BValue::Dict(entries))
            }
            _ => bad("an unknown object type"),
        }
    }
}

// ---- Codable bridge ----

/// The Codable value tree as a property list value. `Value::Date` is seconds since 1970.
pub fn from_value(v: &Value) -> BValue {
    match v {
        Value::Null => BValue::Null,
        Value::Bool(b) => BValue::Bool(*b),
        Value::Int(i) => BValue::Int(*i),
        Value::Num(n) => BValue::Real(*n),
        Value::Str(s) => BValue::Str(s.clone()),
        Value::Date(d) => BValue::date_from_unix(*d),
        Value::Arr(a) => BValue::Array(a.iter().map(from_value).collect()),
        Value::Dict(d) => BValue::Dict(d.iter().map(|(k, v)| (k.clone(), from_value(v))).collect()),
    }
}

/// The reverse of [`from_value`]. Data and `Uid` have no Codable counterpart and are refused.
pub fn to_value(v: &BValue) -> Result<Value, BplistError> {
    Ok(match v {
        BValue::Null => Value::Null,
        BValue::Bool(b) => Value::Bool(*b),
        BValue::Int(i) => Value::Int(*i),
        BValue::Real(r) => Value::Num(*r),
        BValue::Str(s) => Value::Str(s.clone()),
        BValue::Date(d) => Value::Date(d + REFERENCE_DATE_OFFSET),
        BValue::Array(a) => Value::Arr(a.iter().map(to_value).collect::<Result<Vec<_>, _>>()?),
        BValue::Dict(d) => {
            Value::Dict(d.iter().map(|(k, v)| to_value(v).map(|v| (k.clone(), v))).collect::<Result<Vec<_>, _>>()?)
        }
        BValue::Data(_) | BValue::Uid(_) => return bad("data and uid values have no Codable form"),
    })
}
