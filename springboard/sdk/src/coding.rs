//! Codable parity: `JSONEncoder`/`JSONDecoder`, `PropertyListEncoder`/`PropertyListDecoder` (XML plist; binary through `binary_plist`).
//!
//! No serde and no proc-macro: types implement [`Encodable`]/[`Decodable`] over the [`Value`] tree, by hand or with
//! the [`codable!`](crate::codable) macro for plain structs. `Option` fields are left out when `None` and decode
//! as `None` when the key is missing or null. JSON parsing reuses `bifrost_core::json`.

use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    /// Seconds since 1970 (`Date.timeIntervalSince1970`); written per the encoder's date strategy.
    Date(f64),
    Arr(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodingError(pub String);
impl std::fmt::Display for CodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CodingError {}
fn err<T>(m: impl Into<String>) -> Result<T, CodingError> {
    Err(CodingError(m.into()))
}

pub trait Encodable {
    fn encode(&self) -> Value;
    /// `true` leaves a struct key out (Swift's `encodeIfPresent`).
    fn is_absent(&self) -> bool {
        false
    }
}
pub trait Decodable: Sized {
    fn decode(v: &Value) -> Result<Self, CodingError>;
    /// Value used when a struct key is missing (`Option` gives `None`).
    fn missing() -> Option<Self> {
        None
    }
}

/// A point in time, encoded per the coder's date strategy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Date(pub f64);

macro_rules! int_codable { ($($t:ty),*) => {$(
    impl Encodable for $t { fn encode(&self) -> Value { Value::Int(*self as i64) } }
    impl Decodable for $t { fn decode(v: &Value) -> Result<Self, CodingError> {
        match v { Value::Int(i) => <$t>::try_from(*i).or_else(|_| err("integer out of range")),
                  Value::Num(n) if n.fract() == 0.0 => Ok(*n as $t), _ => err(concat!("expected ", stringify!($t))) } } }
)*} }
int_codable!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

macro_rules! float_codable { ($($t:ty),*) => {$(
    impl Encodable for $t { fn encode(&self) -> Value { Value::Num(*self as f64) } }
    impl Decodable for $t { fn decode(v: &Value) -> Result<Self, CodingError> {
        match v { Value::Int(i) => Ok(*i as $t), Value::Num(n) => Ok(*n as $t), _ => err("expected number") } } }
)*} }
float_codable!(f32, f64);

impl Encodable for bool {
    fn encode(&self) -> Value {
        Value::Bool(*self)
    }
}
impl Decodable for bool {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if let Value::Bool(b) = v { Ok(*b) } else { err("expected bool") }
    }
}
impl Encodable for String {
    fn encode(&self) -> Value {
        Value::Str(self.clone())
    }
}
impl Decodable for String {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if let Value::Str(s) = v { Ok(s.clone()) } else { err("expected string") }
    }
}
impl Encodable for crate::value_types::Uuid {
    fn encode(&self) -> Value {
        Value::Str(self.uuid_string())
    }
}
impl Decodable for crate::value_types::Uuid {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        match v {
            Value::Str(s) => crate::value_types::Uuid::parse(s).ok_or_else(|| CodingError("bad UUID string".into())),
            _ => err("expected UUID string"),
        }
    }
}
impl Encodable for crate::decimal::Decimal {
    fn encode(&self) -> Value {
        Value::Str(self.to_string())
    }
}
impl Decodable for crate::decimal::Decimal {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        match v {
            Value::Str(s) => crate::decimal::Decimal::parse(s).ok_or_else(|| CodingError("bad decimal string".into())),
            Value::Int(i) => Ok(crate::decimal::Decimal::from_i64(*i)),
            _ => err("expected decimal string"),
        }
    }
}
impl Encodable for Date {
    fn encode(&self) -> Value {
        Value::Date(self.0)
    }
}
impl Decodable for Date {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        match v {
            Value::Date(d) => Ok(Date(*d)),
            Value::Int(i) => Ok(Date(*i as f64)),
            Value::Num(n) => Ok(Date(*n)),
            Value::Str(s) => parse_iso8601(s).map(Date).ok_or_else(|| CodingError("bad ISO 8601 date".into())),
            _ => err("expected date"),
        }
    }
}
impl<T: Encodable> Encodable for Option<T> {
    fn encode(&self) -> Value {
        self.as_ref().map_or(Value::Null, |v| v.encode())
    }
    fn is_absent(&self) -> bool {
        self.is_none()
    }
}
impl<T: Decodable> Decodable for Option<T> {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if *v == Value::Null { Ok(None) } else { T::decode(v).map(Some) }
    }
    fn missing() -> Option<Self> {
        Some(None)
    }
}
impl<T: Encodable> Encodable for Vec<T> {
    fn encode(&self) -> Value {
        Value::Arr(self.iter().map(|v| v.encode()).collect())
    }
}
impl<T: Decodable> Decodable for Vec<T> {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if let Value::Arr(a) = v { a.iter().map(T::decode).collect() } else { err("expected array") }
    }
}
impl<T: Encodable> Encodable for HashMap<String, T> {
    fn encode(&self) -> Value {
        Value::Dict(self.iter().map(|(k, v)| (k.clone(), v.encode())).collect())
    }
}
impl<T: Decodable> Decodable for HashMap<String, T> {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if let Value::Dict(d) = v { d.iter().map(|(k, v)| Ok((k.clone(), T::decode(v)?))).collect() } else { err("expected dictionary") }
    }
}
impl<T: Encodable> Encodable for BTreeMap<String, T> {
    fn encode(&self) -> Value {
        Value::Dict(self.iter().map(|(k, v)| (k.clone(), v.encode())).collect())
    }
}
impl<T: Decodable> Decodable for BTreeMap<String, T> {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        if let Value::Dict(d) = v { d.iter().map(|(k, v)| Ok((k.clone(), T::decode(v)?))).collect() } else { err("expected dictionary") }
    }
}

/// Reads `key` from a dictionary value (used by [`codable!`](crate::codable)).
pub fn field<T: Decodable>(v: &Value, key: &str) -> Result<T, CodingError> {
    let Value::Dict(d) = v else { return err("expected dictionary") };
    match d.iter().find(|(k, _)| k == key) {
        Some((_, x)) => T::decode(x).map_err(|e| CodingError(format!("{key}: {}", e.0))),
        None => T::missing().ok_or_else(|| CodingError(format!("missing key {key}"))),
    }
}

/// Implements [`Encodable`] and [`Decodable`] for a plain struct: `codable!(Message { user: bool, text: String });`
/// Keys are the field names. Every field type must itself be codable.
#[macro_export]
macro_rules! codable {
    // `field: Type = default` decodes a missing key as `default` (Swift's `decodeIfPresent(_:forKey:) ?? default`)
    ($name:ident { $($f:ident : $t:ty $(= $d:expr)?),* $(,)? }) => {
        impl $crate::coding::Encodable for $name {
            fn encode(&self) -> $crate::coding::Value {
                #[allow(unused_mut)]
                let mut d = Vec::new();
                $( if !$crate::coding::Encodable::is_absent(&self.$f) { d.push((stringify!($f).to_string(), $crate::coding::Encodable::encode(&self.$f))); } )*
                $crate::coding::Value::Dict(d)
            }
        }
        impl $crate::coding::Decodable for $name {
            fn decode(v: &$crate::coding::Value) -> Result<Self, $crate::coding::CodingError> {
                Ok($name { $( $f: $crate::codable!(@field v, $f, $t $(, $d)?) ),* })
            }
        }
    };
    (@field $v:ident, $f:ident, $t:ty) => { $crate::coding::field::<$t>($v, stringify!($f))? };
    (@field $v:ident, $f:ident, $t:ty, $d:expr) => {
        match $crate::coding::field::<Option<$t>>($v, stringify!($f))? { Some(x) => x, None => $d }
    };
}

// Dates -----------------------------------------------------------------------------------------------------------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - (m <= 2) as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as i64 + 2) / 5 + d as i64 - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}
/// `1970-01-01T00:00:00Z` form (whole seconds, UTC), as `ISO8601DateFormatter` writes.
pub fn format_iso8601(secs: f64) -> String {
    let s = secs.floor() as i64;
    let (y, mo, d) = civil_from_days(s.div_euclid(86400));
    let t = s.rem_euclid(86400);
    format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t / 60 % 60, t % 60)
}
pub fn parse_iso8601(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let mut rest = &s[19..];
    let mut frac = 0.0;
    if let Some(r) = rest.strip_prefix('.') {
        let end = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
        frac = format!("0.{}", &r[..end]).parse().ok()?;
        rest = &r[end..];
    }
    let off = match rest {
        "Z" => 0,
        r if r.len() == 6 && (r.starts_with('+') || r.starts_with('-')) => {
            let o = r[1..3].parse::<i64>().ok()? * 3600 + r[4..6].parse::<i64>().ok()? * 60;
            if r.starts_with('-') { -o } else { o }
        }
        _ => return None,
    };
    Some((days_from_civil(y, mo as u32, d as u32) * 86400 + h * 3600 + mi * 60 + se - off) as f64 + frac)
}

// JSON ------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DateEncodingStrategy {
    #[default]
    SecondsSince1970,
    Iso8601,
}

#[derive(Clone, Debug, Default)]
pub struct JSONEncoder {
    pub pretty_printed: bool,
    pub sorted_keys: bool,
    pub date_encoding_strategy: DateEncodingStrategy,
}

impl JSONEncoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn encode<T: Encodable>(&self, v: &T) -> String {
        let mut o = String::new();
        self.write(&v.encode(), 0, &mut o);
        o
    }
    fn write(&self, v: &Value, depth: usize, o: &mut String) {
        let nl = |o: &mut String, d: usize| {
            if self.pretty_printed {
                o.push('\n');
                o.push_str(&"  ".repeat(d));
            }
        };
        match v {
            Value::Null => o.push_str("null"),
            Value::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => o.push_str(&i.to_string()),
            Value::Num(n) if n.is_finite() => o.push_str(&n.to_string()),
            Value::Num(_) => o.push_str("null"),
            Value::Str(s) => o.push_str(&bifrost_core::json::quote(s)),
            Value::Date(d) => match self.date_encoding_strategy {
                DateEncodingStrategy::SecondsSince1970 => o.push_str(&d.to_string()),
                DateEncodingStrategy::Iso8601 => o.push_str(&bifrost_core::json::quote(&format_iso8601(*d))),
            },
            Value::Arr(a) if a.is_empty() => o.push_str("[]"),
            Value::Arr(a) => {
                o.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, depth + 1);
                    self.write(x, depth + 1, o);
                }
                nl(o, depth);
                o.push(']');
            }
            Value::Dict(d) if d.is_empty() => o.push_str("{}"),
            Value::Dict(d) => {
                let mut items: Vec<&(String, Value)> = d.iter().collect();
                if self.sorted_keys {
                    items.sort_by(|a, b| a.0.cmp(&b.0));
                }
                o.push('{');
                for (i, (k, x)) in items.into_iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    nl(o, depth + 1);
                    o.push_str(&bifrost_core::json::quote(k));
                    o.push_str(if self.pretty_printed { " : " } else { ":" });
                    self.write(x, depth + 1, o);
                }
                nl(o, depth);
                o.push('}');
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct JSONDecoder;

impl JSONDecoder {
    pub fn new() -> Self {
        JSONDecoder
    }
    /// Dates decode from seconds (number) or an ISO 8601 string, whichever the document holds.
    pub fn decode<T: Decodable>(&self, text: &str) -> Result<T, CodingError> {
        T::decode(&from_json(&bifrost_core::json::parse(text).map_err(CodingError)?))
    }
}

fn from_json(j: &bifrost_core::json::Json) -> Value {
    use bifrost_core::json::Json;
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Value::Int(*n as i64),
        Json::Num(n) => Value::Num(*n),
        Json::Str(s) => Value::Str(s.clone()),
        Json::Arr(a) => Value::Arr(a.iter().map(from_json).collect()),
        Json::Obj(o) => Value::Dict(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect()),
    }
}

// Property list (XML) ---------------------------------------------------------------------------------------------

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
fn xml_unesc(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[derive(Clone, Debug, Default)]
pub struct PropertyListEncoder;

impl PropertyListEncoder {
    pub fn new() -> Self {
        PropertyListEncoder
    }
    pub fn encode<T: Encodable>(&self, v: &T) -> String {
        let mut o = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n");
        Self::write(&v.encode(), 0, &mut o);
        o.push_str("</plist>\n");
        o
    }
    /// Encodes as a binary property list (`bplist00`, see `binary_plist`). Dates keep their exact value.
    pub fn encode_binary<T: Encodable>(&self, v: &T) -> Vec<u8> {
        crate::binary_plist::write(&crate::binary_plist::from_value(&v.encode()))
    }

    fn write(v: &Value, depth: usize, o: &mut String) {
        let pad = "\t".repeat(depth);
        match v {
            Value::Null => o.push_str(&format!("{pad}<string></string>\n")),
            Value::Bool(b) => o.push_str(&format!("{pad}<{b}/>\n")),
            Value::Int(i) => o.push_str(&format!("{pad}<integer>{i}</integer>\n")),
            Value::Num(n) => o.push_str(&format!("{pad}<real>{n}</real>\n")),
            Value::Str(s) => o.push_str(&format!("{pad}<string>{}</string>\n", xml_esc(s))),
            Value::Date(d) => o.push_str(&format!("{pad}<date>{}</date>\n", format_iso8601(*d))),
            Value::Arr(a) => {
                o.push_str(&format!("{pad}<array>\n"));
                a.iter().for_each(|x| Self::write(x, depth + 1, o));
                o.push_str(&format!("{pad}</array>\n"));
            }
            Value::Dict(d) => {
                o.push_str(&format!("{pad}<dict>\n"));
                for (k, x) in d {
                    o.push_str(&format!("{pad}\t<key>{}</key>\n", xml_esc(k)));
                    Self::write(x, depth + 1, o);
                }
                o.push_str(&format!("{pad}</dict>\n"));
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PropertyListDecoder;

impl PropertyListDecoder {
    pub fn new() -> Self {
        PropertyListDecoder
    }
    pub fn decode<T: Decodable>(&self, text: &str) -> Result<T, CodingError> {
        let body = text.find("<plist").and_then(|i| text[i..].find('>').map(|j| &text[i + j + 1..])).ok_or_else(|| CodingError("not a plist".into()))?;
        let mut p = Plist { s: body, pos: 0 };
        T::decode(&p.value()?)
    }

    /// Decodes a binary property list written by `encode_binary` (or any writer), keeping dates exact.
    pub fn decode_binary<T: Decodable>(&self, bytes: &[u8]) -> Result<T, CodingError> {
        let parsed = crate::binary_plist::read(bytes).map_err(|e| CodingError(e.0))?;
        T::decode(&crate::binary_plist::to_value(&parsed).map_err(|e| CodingError(e.0))?)
    }

    /// Decodes binary (`bplist00`) or text (XML or ASCII) property-list bytes. Binary dates decode like XML dates;
    /// data values have no [`Value`] form yet, so a binary file holding one is refused.
    pub fn decode_bytes<T: Decodable>(&self, bytes: &[u8]) -> Result<T, CodingError> {
        if bytes.starts_with(b"bplist") {
            let p = bifrost_core::plist::parse_bytes(bytes).map_err(CodingError)?;
            return T::decode(&plist_value(p)?);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| CodingError("the property list is not text".into()))?;
        self.decode(text)
    }
}

/// Maps a parsed property list onto the Codable value tree (binary reads).
fn plist_value(p: bifrost_core::plist::Plist) -> Result<Value, CodingError> {
    use bifrost_core::plist::Plist as P;
    Ok(match p {
        P::Bool(b) => Value::Bool(b),
        P::Int(i) => Value::Int(i),
        P::Real(r) => Value::Num(r),
        P::Str(s) => Value::Str(s),
        P::Date(text) => Value::Date(parse_iso8601(&text).ok_or_else(|| CodingError("bad date".into()))?),
        P::Data(_) => return err("data values are not decoded through Codable yet"),
        P::Array(a) => Value::Arr(a.into_iter().map(plist_value).collect::<Result<Vec<_>, _>>()?),
        P::Dict(d) => Value::Dict(
            d.into_iter().map(|(k, v)| plist_value(v).map(|v| (k, v))).collect::<Result<Vec<_>, _>>()?,
        ),
    })
}

struct Plist<'a> {
    s: &'a str,
    pos: usize,
}

impl Plist<'_> {
    /// Next tag as `(name, self_closing, is_close)`; skips comments and `<?...?>`.
    fn tag(&mut self) -> Result<(String, bool, bool), CodingError> {
        loop {
            let r = &self.s[self.pos..];
            let i = r.find('<').ok_or_else(|| CodingError("unexpected end of plist".into()))?;
            let j = r[i..].find('>').ok_or_else(|| CodingError("unterminated tag".into()))? + i;
            self.pos += j + 1;
            let t = &r[i + 1..j];
            if t.starts_with('?') || t.starts_with('!') {
                continue;
            }
            let close = t.starts_with('/');
            let selfc = t.ends_with('/');
            let name = t.trim_matches('/').split_whitespace().next().unwrap_or("").to_string();
            return Ok((name, selfc, close));
        }
    }
    fn text(&mut self, name: &str) -> Result<String, CodingError> {
        let r = &self.s[self.pos..];
        let end = r.find(&format!("</{name}>")).ok_or_else(|| CodingError(format!("missing </{name}>")))?;
        self.pos += end + name.len() + 3;
        Ok(xml_unesc(&r[..end]))
    }
    fn value(&mut self) -> Result<Value, CodingError> {
        let (name, selfc, close) = self.tag()?;
        if close {
            return err("unexpected closing tag");
        }
        Ok(match (name.as_str(), selfc) {
            ("true", _) => Value::Bool(true),
            ("false", _) => Value::Bool(false),
            ("string", true) => Value::Str(String::new()),
            ("string", _) => Value::Str(self.text("string")?),
            ("integer", _) => Value::Int(self.text("integer")?.trim().parse().map_err(|_| CodingError("bad integer".into()))?),
            ("real", _) => Value::Num(self.text("real")?.trim().parse().map_err(|_| CodingError("bad real".into()))?),
            ("date", _) => Value::Date(parse_iso8601(self.text("date")?.trim()).ok_or_else(|| CodingError("bad date".into()))?),
            ("array", true) => Value::Arr(Vec::new()),
            ("array", _) => {
                let mut a = Vec::new();
                loop {
                    let save = self.pos;
                    let (n, _, c) = self.tag()?;
                    if c && n == "array" {
                        break;
                    }
                    self.pos = save;
                    a.push(self.value()?);
                }
                Value::Arr(a)
            }
            ("dict", true) => Value::Dict(Vec::new()),
            ("dict", _) => {
                let mut d = Vec::new();
                loop {
                    let (n, _, c) = self.tag()?;
                    if c && n == "dict" {
                        break;
                    }
                    if n != "key" {
                        return err("expected <key>");
                    }
                    let k = self.text("key")?;
                    d.push((k, self.value()?));
                }
                Value::Dict(d)
            }
            (n, _) => return err(format!("unsupported plist element <{n}>")),
        })
    }
}

#[cfg(test)]
mod binary_tests {
    use super::*;
    use std::collections::HashMap;

    /// `{ a = b; }` as a binary property list.
    fn dict_of_strings() -> Vec<u8> {
        let mut b = b"bplist00".to_vec();
        b.extend([0xD1, 1, 2, 0x51, b'a', 0x51, b'b']); // dict at 8, then "a" at 11 and "b" at 13
        b.extend([8, 11, 13]); // offset table at 15
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]);
        b.extend(3u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(15u64.to_be_bytes());
        b
    }

    #[test]
    fn decodes_binary_dictionary_into_a_map() {
        let m: HashMap<String, String> = PropertyListDecoder::new().decode_bytes(&dict_of_strings()).unwrap();
        assert_eq!(m.get("a").map(String::as_str), Some("b"));
    }

    #[test]
    fn text_bytes_still_decode() {
        let text = b"<plist version=\"1.0\"><dict><key>a</key><string>b</string></dict></plist>";
        let m: HashMap<String, String> = PropertyListDecoder::new().decode_bytes(text).unwrap();
        assert_eq!(m.get("a").map(String::as_str), Some("b"));
    }

    #[test]
    fn binary_data_values_are_refused_until_codable_has_data() {
        let mut b = b"bplist00".to_vec();
        b.extend([0xD1, 1, 2, 0x51, b'a', 0x41, 1]); // "a" maps to one data byte
        b.extend([8, 11, 13]);
        b.extend([0, 0, 0, 0, 0, 0, 1, 1]);
        b.extend(3u64.to_be_bytes());
        b.extend(0u64.to_be_bytes());
        b.extend(15u64.to_be_bytes());
        assert!(PropertyListDecoder::new().decode_bytes::<HashMap<String, String>>(&b).is_err());
    }
}

#[cfg(test)]
mod uuid_decimal_tests {
    use super::*;
    use crate::decimal::Decimal;
    use crate::value_types::Uuid;

    #[test]
    fn uuid_round_trips_as_a_json_string() {
        let u = Uuid::version4([9; 16]);
        let text = JSONEncoder::new().encode(&u);
        assert_eq!(text, format!("\"{u}\""));
        assert_eq!(JSONDecoder::new().decode::<Uuid>(&text).unwrap(), u);
        assert!(JSONDecoder::new().decode::<Uuid>("\"nope\"").is_err());
    }

    #[test]
    fn decimal_round_trips_as_a_json_string() {
        let d = Decimal::parse("12.50").unwrap();
        let text = JSONEncoder::new().encode(&d);
        assert_eq!(text, "\"12.5\"");
        assert_eq!(JSONDecoder::new().decode::<Decimal>(&text).unwrap(), d);
    }
}
