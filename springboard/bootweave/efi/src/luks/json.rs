//! A small JSON reader for the LUKS2 JSON area. LUKS2 writes most numbers as strings; bare numbers are non-negative
//! integers, and anything else (fractions, negatives) is an error. Nesting is bounded, so hostile input cannot
//! exhaust the loader's stack.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

const MAX_DEPTH: u32 = 16;

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// A number, or a string holding a decimal number (how LUKS2 writes offsets and sizes).
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) => Some(*n),
            Json::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn entries(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(fields) => Some(fields),
            _ => None,
        }
    }
}

/// Parses one JSON value. Trailing NUL bytes and whitespace (the padding of the LUKS2 JSON area) are allowed;
/// anything else after the value is an error.
pub fn parse(text: &[u8]) -> Option<Json> {
    let mut p = Parser { s: text, i: 0 };
    let value = p.value(0)?;
    p.skip_space();
    if p.i != text.len() {
        return None;
    }
    Some(value)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while let Some(&c) = self.s.get(self.i) {
            if matches!(c, b' ' | b'\n' | b'\r' | b'\t' | 0) {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        self.skip_space();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Some(())
        } else {
            None
        }
    }

    fn word(&mut self, w: &[u8]) -> Option<()> {
        if self.s[self.i..].starts_with(w) {
            self.i += w.len();
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.skip_space();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.skip_space();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.skip_space();
                    let key = self.string()?;
                    self.eat(b':')?;
                    let value = self.value(depth + 1)?;
                    fields.push((key, value));
                    self.skip_space();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Json::Obj(fields));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.skip_space();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.skip_space();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Json::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.word(b"true").map(|_| Json::Bool(true)),
            b'f' => self.word(b"false").map(|_| Json::Bool(false)),
            b'n' => self.word(b"null").map(|_| Json::Null),
            b'0'..=b'9' => {
                let mut n: u64 = 0;
                while let Some(&c) = self.s.get(self.i) {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    n = n.checked_mul(10)?.checked_add((c - b'0') as u64)?;
                    self.i += 1;
                }
                Some(Json::Num(n))
            }
            _ => None,
        }
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return Some(out),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4)?;
                            let code = u32::from_str_radix(core::str::from_utf8(hex).ok()?, 16).ok()?;
                            self.i += 4;
                            out.push(char::from_u32(code)?);
                        }
                        _ => return None,
                    }
                }
                0x00..=0x1f => return None,
                0x20..=0x7f => out.push(c as char),
                _ => {
                    // A multi-byte UTF-8 sequence: copy its bytes whole.
                    let len = match c {
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf7 => 4,
                        _ => return None,
                    };
                    let start = self.i - 1;
                    let bytes = self.s.get(start..start + len)?;
                    out.push_str(core::str::from_utf8(bytes).ok()?);
                    self.i = start + len;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn parses_nested_objects_and_arrays() {
        let v = parse(br#"{"a":"1","b":[1,2,{"c":true}],"d":null}"#).unwrap();
        assert_eq!(v.get("a").and_then(Json::as_u64), Some(1));
        let b = v.get("b").and_then(Json::as_arr).unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!(b[2].get("c"), Some(&Json::Bool(true)));
        assert_eq!(v.get("d"), Some(&Json::Null));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn accepts_nul_padding_after_the_value() {
        let v = parse(b"{\"x\":\"y\"}\0\0\0\0").unwrap();
        assert_eq!(v.get("x").and_then(Json::as_str), Some("y"));
    }

    #[test]
    fn rejects_trailing_text_and_malformed_input() {
        assert_eq!(parse(b"{} x"), None);
        assert_eq!(parse(b"{\"a\":}"), None);
        assert_eq!(parse(b"[1,]"), None);
        assert_eq!(parse(b"{\"a\":1.5}"), None);
        assert_eq!(parse(b""), None);
    }

    #[test]
    fn decodes_escapes_and_unicode() {
        let v = parse(br#"{"k":"a\"b\n\u00e9"}"#).unwrap();
        assert_eq!(v.get("k").and_then(Json::as_str), Some("a\"b\n\u{e9}"));
        let utf8 = parse("\"\u{e9}\"".as_bytes()).unwrap();
        assert_eq!(utf8.as_str(), Some("\u{e9}"));
    }

    #[test]
    fn bounds_the_nesting_depth() {
        let deep: alloc::string::String = (0..40).map(|_| '[').chain((0..40).map(|_| ']')).collect();
        assert_eq!(parse(deep.as_bytes()), None);
        let shallow = "[[[]]]";
        assert!(parse(shallow.as_bytes()).is_some());
    }

    #[test]
    fn numbers_as_strings_and_numbers() {
        let v = parse(br#"{"n":"32768","m":32768,"bad":"x1"}"#).unwrap();
        assert_eq!(v.get("n").and_then(Json::as_u64), Some(32768));
        assert_eq!(v.get("m").and_then(Json::as_u64), Some(32768));
        assert_eq!(v.get("bad").and_then(Json::as_u64), None);
        let entries = v.entries().unwrap();
        assert_eq!(entries[0].0, "n".to_string());
    }
}
