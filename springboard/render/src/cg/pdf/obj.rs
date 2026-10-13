//! PDF objects and their lexer/parser, shared by the file structure code
//! and the content-stream interpreter.

use std::collections::HashMap;
use std::rc::Rc;

pub type Dict = HashMap<String, Obj>;

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    /// the raw (still filtered) bytes
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Obj>),
    Dict(Dict),
    Stream(Rc<Stream>),
    Ref(u32, u16),
    /// a bare keyword: `obj`, `stream`, or a content-stream operator
    Op(String),
}

impl Obj {
    pub fn num(&self) -> Option<f64> {
        match self {
            Obj::Int(i) => Some(*i as f64),
            Obj::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Obj::Int(i) => Some(*i),
            Obj::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The dictionary of a dictionary or a stream.
    pub fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            Obj::Stream(s) => Some(&s.dict),
            _ => None,
        }
    }

    pub fn is_op(&self, op: &str) -> bool {
        matches!(self, Obj::Op(o) if o == op)
    }
}

pub fn is_ws(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

pub fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

pub struct Parser<'a> {
    pub d: &'a [u8],
    pub pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(d: &'a [u8], pos: usize) -> Self {
        Parser { d, pos }
    }

    pub fn eof(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.d.len()
    }

    pub fn peek(&self) -> Option<u8> {
        self.d.get(self.pos).copied()
    }

    pub fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if is_ws(c) {
                self.pos += 1;
            } else if c == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn token(&mut self) -> &'a [u8] {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_ws(c) || is_delim(c) {
                break;
            }
            self.pos += 1;
        }
        &self.d[start..self.pos]
    }

    /// Parses one object; `refs` enables `N G R` references. None at the end
    /// of the data.
    pub fn parse(&mut self, refs: bool) -> Option<Obj> {
        self.skip_ws();
        let c = self.peek()?;
        match c {
            b'/' => {
                self.pos += 1;
                Some(Obj::Name(self.name()))
            }
            b'(' => Some(self.string()),
            b'<' => {
                if self.d.get(self.pos + 1) == Some(&b'<') {
                    self.pos += 2;
                    Some(self.dict(refs))
                } else {
                    Some(self.hex())
                }
            }
            b'[' => {
                self.pos += 1;
                let mut a = vec![];
                loop {
                    self.skip_ws();
                    match self.peek() {
                        None => break,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        Some(_) => match self.parse(refs) {
                            Some(o) => a.push(o),
                            None => break,
                        },
                    }
                }
                Some(Obj::Array(a))
            }
            b')' | b']' | b'>' | b'{' | b'}' => {
                self.pos += 1;
                Some(Obj::Op((c as char).to_string()))
            }
            _ => {
                let tok = self.token();
                if tok.is_empty() {
                    self.pos += 1;
                    return Some(Obj::Null);
                }
                Some(self.word(tok, refs))
            }
        }
    }

    fn word(&mut self, tok: &[u8], refs: bool) -> Obj {
        let s = String::from_utf8_lossy(tok);
        let numeric = tok.iter().all(|&c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.')) && tok.iter().any(|c| c.is_ascii_digit());
        if numeric {
            if let Ok(i) = s.parse::<i64>() {
                if refs && i >= 0 {
                    let save = self.pos;
                    self.skip_ws();
                    let genr = self.token();
                    if !genr.is_empty() && genr.iter().all(|c| c.is_ascii_digit()) {
                        self.skip_ws();
                        if self.peek() == Some(b'R') && self.d.get(self.pos + 1).map_or(true, |&n| is_ws(n) || is_delim(n)) {
                            self.pos += 1;
                            let g = std::str::from_utf8(genr).ok().and_then(|g| g.parse().ok()).unwrap_or(0);
                            return Obj::Ref(i as u32, g);
                        }
                    }
                    self.pos = save;
                }
                return Obj::Int(i);
            }
            // forms like "--5" or "4.5.6": keep what parses
            let cleaned: String = s.trim_start_matches(['+', '-']).to_string();
            let neg = s.starts_with('-');
            let v = cleaned.parse::<f64>().or_else(|_| cleaned.split('.').take(2).collect::<Vec<_>>().join(".").parse::<f64>()).unwrap_or(0.0);
            return Obj::Real(if neg { -v } else { v });
        }
        match &*s {
            "true" => Obj::Bool(true),
            "false" => Obj::Bool(false),
            "null" => Obj::Null,
            other => Obj::Op(other.to_string()),
        }
    }

    fn name(&mut self) -> String {
        let raw = self.token();
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'#' {
                let hex = raw.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = hex {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(raw[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn string(&mut self) -> Obj {
        self.pos += 1;
        let mut out = vec![];
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                _ => out.push(c),
            }
        }
        Obj::Str(out)
    }

    fn hex(&mut self) -> Obj {
        self.pos += 1;
        let mut out = vec![];
        let mut hi: Option<u8> = None;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'>' {
                break;
            }
            let Some(v) = (c as char).to_digit(16) else { continue };
            match hi.take() {
                Some(h) => out.push(h << 4 | v as u8),
                None => hi = Some(v as u8),
            }
        }
        if let Some(h) = hi {
            out.push(h << 4);
        }
        Obj::Str(out)
    }

    fn dict(&mut self, refs: bool) -> Obj {
        let mut d = Dict::new();
        loop {
            self.skip_ws();
            if self.d[self.pos.min(self.d.len())..].starts_with(b">>") {
                self.pos += 2;
                break;
            }
            let Some(k) = self.parse(refs) else { break };
            let key = match k {
                Obj::Name(key) => key,
                // a missing `>>` shows up as one of these keywords
                Obj::Op(o) if o == "endobj" || o == "stream" => break,
                _ => continue,
            };
            match self.parse(refs) {
                Some(Obj::Op(o)) if o == ">" => break,
                Some(v) => {
                    d.insert(key, v);
                }
                None => break,
            }
        }
        Obj::Dict(d)
    }
}

/// A number the way PDF writes it: no exponent, trimmed zeros.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// Writes a name with `#xx` escapes for bytes a name cannot hold raw.
pub fn write_name(name: &str, out: &mut Vec<u8>) {
    out.push(b'/');
    for &b in name.as_bytes() {
        if b <= 32 || b >= 127 || is_delim(b) || b == b'#' {
            out.extend_from_slice(format!("#{b:02X}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

impl Obj {
    /// Serialises the object as PDF syntax. Streams write their dictionary,
    /// then `stream`, the bytes and `endstream`, with `/Length` set.
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Obj::Null => out.extend_from_slice(b"null"),
            Obj::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Obj::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
            Obj::Real(r) => out.extend_from_slice(fmt_num(*r).as_bytes()),
            Obj::Str(s) => {
                out.push(b'<');
                for b in s {
                    out.extend_from_slice(format!("{b:02X}").as_bytes());
                }
                out.push(b'>');
            }
            Obj::Name(n) => write_name(n, out),
            Obj::Array(a) => {
                out.push(b'[');
                for (i, o) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b' ');
                    }
                    o.write(out);
                }
                out.push(b']');
            }
            Obj::Dict(d) => write_dict(d, out),
            Obj::Stream(s) => {
                let mut d = s.dict.clone();
                d.insert("Length".into(), Obj::Int(s.data.len() as i64));
                write_dict(&d, out);
                out.extend_from_slice(b"\nstream\n");
                out.extend_from_slice(&s.data);
                out.extend_from_slice(b"\nendstream");
            }
            Obj::Ref(n, g) => out.extend_from_slice(format!("{n} {g} R").as_bytes()),
            Obj::Op(o) => out.extend_from_slice(o.as_bytes()),
        }
    }
}

fn write_dict(d: &Dict, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    let mut keys: Vec<&String> = d.keys().collect();
    keys.sort();
    for k in keys {
        write_name(k, out);
        out.push(b' ');
        d[k].write(out);
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str, refs: bool) -> Obj {
        Parser::new(s.as_bytes(), 0).parse(refs).unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(parse("42", false), Obj::Int(42));
        assert_eq!(parse("-3.5", false), Obj::Real(-3.5));
        assert_eq!(parse(".25", false), Obj::Real(0.25));
        assert_eq!(parse("true", false), Obj::Bool(true));
        assert_eq!(parse("null", false), Obj::Null);
        assert_eq!(parse("/A#20B", false), Obj::Name("A B".into()));
        assert_eq!(parse("Tj", false), Obj::Op("Tj".into()));
    }

    #[test]
    fn strings_hex_and_escapes() {
        assert_eq!(parse(r"(a\(b\) (nested) \101\n)", false), Obj::Str(b"a(b) (nested) A\n".to_vec()));
        assert_eq!(parse("<48 65 6c6C6f7>", false), Obj::Str(b"Hellop".to_vec()));
        assert_eq!(parse("(line\\\nsplice)", false), Obj::Str(b"linesplice".to_vec()));
    }

    #[test]
    fn arrays_dicts_and_references() {
        let o = parse("<< /Type /Page /Kids [1 0 R 2 0 R] /N 7 /Sub << /X (y) >> >>", true);
        let d = o.dict().unwrap();
        assert_eq!(d["Type"], Obj::Name("Page".into()));
        assert_eq!(d["Kids"], Obj::Array(vec![Obj::Ref(1, 0), Obj::Ref(2, 0)]));
        assert_eq!(d["N"], Obj::Int(7));
        assert_eq!(d["Sub"].dict().unwrap()["X"], Obj::Str(b"y".to_vec()));
        // without reference parsing the numbers stay numbers
        assert_eq!(parse("[1 0 R]", false), Obj::Array(vec![Obj::Int(1), Obj::Int(0), Obj::Op("R".into())]));
    }

    #[test]
    fn comments_and_truncation_do_not_loop() {
        assert_eq!(parse("% comment\n 5", false), Obj::Int(5));
        assert!(matches!(parse("<< /A [1 2", true), Obj::Dict(_)));
        assert!(Parser::new(b"   ", 0).parse(false).is_none());
    }

    #[test]
    fn written_objects_parse_back() {
        let mut d = Dict::new();
        d.insert("Kids".into(), Obj::Array(vec![Obj::Ref(3, 0), Obj::Real(1.5), Obj::Int(-2)]));
        d.insert("T".into(), Obj::Name("A B".into()));
        d.insert("S".into(), Obj::Str(b"hi (there)".to_vec()));
        let o = Obj::Dict(d);
        let mut out = vec![];
        o.write(&mut out);
        assert_eq!(Parser::new(&out, 0).parse(true).unwrap(), o);
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(0.25), "0.25");
        assert_eq!(fmt_num(-0.00001), "0");
    }
}
