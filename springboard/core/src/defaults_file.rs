//! The file behind an app's `UserDefaults` and the `defaults` command: one line per key, `kind<TAB>key<TAB>value`.
//!
//! Kinds: `s` string, `b` bool (`1` or `0`), `i` integer, `d` double (its bits in hex), `x` data (hex), and `p` a
//! property-list value (arrays and dictionaries) held as its XML text. Tabs, newlines and backslashes in keys and
//! strings are escaped. A line that does not parse is skipped, so one bad line never loses the rest.

use std::collections::BTreeMap;

use crate::plist::{self, Plist};

/// The file name inside an app's data directory (`<data>/appdata/<domain>/`).
pub const FILE: &str = "defaults.txt";

/// One stored value. Scalars keep their own kinds; arrays and dictionaries are `Plist`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    Data(Vec<u8>),
    Plist(Plist),
}

impl Value {
    /// The value as a property-list value.
    pub fn to_plist(&self) -> Plist {
        match self {
            Value::Str(s) => Plist::Str(s.clone()),
            Value::Bool(b) => Plist::Bool(*b),
            Value::Int(n) => Plist::Int(*n),
            Value::Double(d) => Plist::Real(*d),
            Value::Data(d) => Plist::Data(d.clone()),
            Value::Plist(p) => p.clone(),
        }
    }

    /// The scalar kind for a property-list value, so a value is stored the same way however it was made.
    pub fn from_plist(p: Plist) -> Value {
        match p {
            Plist::Str(s) => Value::Str(s),
            Plist::Bool(b) => Value::Bool(b),
            Plist::Int(n) => Value::Int(n),
            Plist::Real(d) => Value::Double(d),
            Plist::Data(d) => Value::Data(d),
            other => Value::Plist(other),
        }
    }
}

/// Writes every value, one line per key, in key order.
pub fn serialise(values: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    for (key, v) in values {
        let (kind, value) = match v {
            Value::Str(s) => ("s", escape(s)),
            Value::Bool(b) => ("b", if *b { "1" } else { "0" }.to_string()),
            Value::Int(n) => ("i", n.to_string()),
            Value::Double(d) => ("d", format!("{:016x}", d.to_bits())),
            Value::Data(d) => ("x", plist::hex_encode(d)),
            Value::Plist(p) => ("p", escape(&plist::to_xml(p))),
        };
        out.push_str(&format!("{kind}\t{}\t{value}\n", escape(key)));
    }
    out
}

/// Reads the lines `serialise` wrote. A later line for a key replaces an earlier one.
pub fn parse(text: &str) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(kind), Some(key), Some(raw)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let key = unescape(key);
        let value = match kind {
            "s" => Value::Str(unescape(raw)),
            "b" => match raw {
                "1" => Value::Bool(true),
                "0" => Value::Bool(false),
                _ => continue,
            },
            "i" => match raw.parse() {
                Ok(n) => Value::Int(n),
                Err(_) => continue,
            },
            "d" => match u64::from_str_radix(raw, 16) {
                Ok(bits) => Value::Double(f64::from_bits(bits)),
                Err(_) => continue,
            },
            "x" => match plist::hex_decode(raw) {
                Some(d) => Value::Data(d),
                None => continue,
            },
            "p" => match plist::parse_xml(&unescape(raw)) {
                Ok(p) => Value::from_plist(p),
                Err(_) => continue,
            },
            _ => continue,
        };
        values.insert(key, value);
    }
    values
}

/// Escapes a key or string so it fits on one line with its tabs intact.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// Reverses `escape`. A trailing lone backslash is dropped.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_with_awkward_text() {
        let mut v = BTreeMap::new();
        v.insert("a\tb\nc".to_string(), Value::Str("line one\nline\\two\tx".into()));
        v.insert("flag".to_string(), Value::Bool(true));
        v.insert("n".to_string(), Value::Int(-12));
        v.insert("d".to_string(), Value::Double(0.25));
        v.insert("blob".to_string(), Value::Data(vec![0, 171, 255]));
        v.insert("list".to_string(), Value::Plist(Plist::Array(vec![Plist::Str("x\ny".into()), Plist::Int(2)])));
        let text = serialise(&v);
        assert_eq!(text.lines().count(), 6, "every key stays on one line: {text}");
        assert_eq!(parse(&text), v);
    }

    #[test]
    fn bad_lines_are_skipped_and_later_lines_win() {
        let text = "s\tk\tfirst\nq\tbad\tzz\nb\tf\tmaybe\ni\tn\tnot-a-number\nx\tz\tabc\np\tp\tnot xml\ns\tk\tsecond\n";
        let v = parse(text);
        assert_eq!(v.len(), 1);
        assert_eq!(v.get("k"), Some(&Value::Str("second".into())));
    }

    #[test]
    fn plist_values_normalise_to_scalar_kinds() {
        assert_eq!(Value::from_plist(Plist::Real(1.5)), Value::Double(1.5));
        assert_eq!(Value::from_plist(Plist::Date("2026-01-01T00:00:00Z".into())), Value::Plist(Plist::Date("2026-01-01T00:00:00Z".into())));
        assert_eq!(Value::Double(2.0).to_plist(), Plist::Real(2.0));
        let text = serialise(&BTreeMap::from([("when".to_string(), Value::Plist(Plist::Str("plain".into())))]));
        assert_eq!(parse(&text).get("when"), Some(&Value::Str("plain".into())));
    }

    #[test]
    fn escape_and_unescape_invert_each_other() {
        let s = "a\\b\tc\nd\re";
        assert_eq!(unescape(&escape(s)), s);
        assert_eq!(unescape("tail\\"), "tail");
    }
}
