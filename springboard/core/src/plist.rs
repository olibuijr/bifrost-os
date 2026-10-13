//! Property lists: one value model with the text forms the command-line tools read and write (XML, the older ASCII
//! form, and JSON out), the key-path edits `plutil` makes, and the base64 that data values use.
//!
//! Rules of this module:
//! - Dictionaries keep their key order as read. Only the human-readable and JSON pretty forms sort keys.
//! - A key path is dot-separated. A component names a dictionary key, or a whole number that indexes an array.
//! - Dates are kept as their text (for example `2026-10-13T08:00:00Z`) and are not checked as calendar values.
//! - Binary property lists are read by [`crate::plist_binary`] through [`parse_bytes`]. They are not written yet.

use std::fmt::Write as _;

/// One property-list value.
#[derive(Clone, Debug, PartialEq)]
pub enum Plist {
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    Data(Vec<u8>),
    Date(String),
    Array(Vec<Plist>),
    Dict(Vec<(String, Plist)>),
}

impl Plist {
    /// The type name `plutil -type` prints and `-expect` accepts.
    pub fn type_name(&self) -> &'static str {
        match self {
            Plist::Bool(_) => "bool",
            Plist::Int(_) => "integer",
            Plist::Real(_) => "float",
            Plist::Str(_) => "string",
            Plist::Data(_) => "data",
            Plist::Date(_) => "date",
            Plist::Array(_) => "array",
            Plist::Dict(_) => "dictionary",
        }
    }
}

/// Reads a property list from bytes: binary (`bplist00`), XML or the ASCII form, detected from the first bytes.
pub fn parse_bytes(bytes: &[u8]) -> Result<Plist, String> {
    if bytes.starts_with(b"bplist") {
        return crate::plist_binary::parse(bytes);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not text".to_string())?;
    parse_any(text)
}

/// Reads XML when the text starts with `<`, else the ASCII form.
pub fn parse_any(text: &str) -> Result<Plist, String> {
    if text.trim_start().starts_with('<') {
        parse_xml(text)
    } else {
        parse_ascii(text)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// XML

/// Reads one XML value. A `plist` wrapper is accepted and its single value returned, so a whole file or a bare
/// fragment such as `<integer>3</integer>` both work.
pub fn parse_xml(text: &str) -> Result<Plist, String> {
    let mut p = Xml { s: text.as_bytes(), pos: 0 };
    p.skip_misc()?;
    let v = p.value()?;
    p.skip_misc()?;
    if p.pos != p.s.len() {
        return Err(format!("unexpected text after the value at byte {}", p.pos));
    }
    Ok(v)
}

struct Xml<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Xml<'_> {
    fn starts(&self, pat: &str) -> bool {
        self.s[self.pos..].starts_with(pat.as_bytes())
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.pos))
    }

    fn skip_past(&mut self, end: &str) -> Result<(), String> {
        match find(&self.s[self.pos..], end.as_bytes()) {
            Some(i) => {
                self.pos += i + end.len();
                Ok(())
            }
            None => self.err(&format!("missing {end}")),
        }
    }

    /// Whitespace, comments, processing instructions and the document type, which carry no values.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.starts("<!--") {
                self.pos += 4;
                self.skip_past("-->")?;
            } else if self.starts("<?") {
                self.pos += 2;
                self.skip_past("?>")?;
            } else if self.starts("<!") {
                self.skip_past(">")?;
            } else {
                return Ok(());
            }
        }
    }

    /// Reads `<name attributes>` or `<name/>`, returning the name and whether the element is empty.
    fn start(&mut self) -> Result<(String, bool), String> {
        if self.pos >= self.s.len() || self.s[self.pos] != b'<' {
            return self.err("expected an element");
        }
        self.pos += 1;
        let begin = self.pos;
        while self.pos < self.s.len() && is_name_byte(self.s[self.pos]) {
            self.pos += 1;
        }
        if self.pos == begin {
            return self.err("expected an element name");
        }
        let name = String::from_utf8_lossy(&self.s[begin..self.pos]).into_owned();
        let mut quoted = false;
        loop {
            let Some(&c) = self.s.get(self.pos) else { return self.err("unterminated tag") };
            self.pos += 1;
            match c {
                b'"' => quoted = !quoted,
                b'>' if !quoted => return Ok((name, self.s[self.pos - 2] == b'/')),
                _ => {}
            }
        }
    }

    /// The text up to the closing tag of `name`, entity-decoded. Consumes the closing tag.
    fn text_until_close(&mut self, name: &str) -> Result<String, String> {
        let Some(i) = find(&self.s[self.pos..], b"</") else { return self.err("missing a closing tag") };
        let raw = String::from_utf8_lossy(&self.s[self.pos..self.pos + i]).into_owned();
        self.pos += i;
        let close = format!("</{name}");
        if !self.starts(&close) {
            return self.err(&format!("expected {close}>"));
        }
        self.pos += close.len();
        self.skip_past(">")?;
        decode_entities(&raw)
    }

    /// Consumes `</name>` if it comes next (after misc items). Returns false, consuming nothing more, otherwise.
    fn close_follows(&mut self, name: &str) -> Result<bool, String> {
        self.skip_misc()?;
        let close = format!("</{name}");
        if self.starts(&close) {
            self.pos += close.len();
            self.skip_past(">")?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn value(&mut self) -> Result<Plist, String> {
        self.skip_misc()?;
        let (name, empty) = self.start()?;
        if name == "plist" {
            if empty {
                return self.err("the plist has no value");
            }
            let v = self.value()?;
            if !self.close_follows("plist")? {
                return self.err("expected </plist>");
            }
            return Ok(v);
        }
        match name.as_str() {
            "dict" => {
                let mut items = Vec::new();
                if !empty {
                    while !self.close_follows("dict")? {
                        let (key_name, key_empty) = self.start()?;
                        if key_name != "key" {
                            return self.err("expected <key>");
                        }
                        let key = if key_empty { String::new() } else { self.text_until_close("key")? };
                        let v = self.value()?;
                        items.push((key, v));
                    }
                }
                Ok(Plist::Dict(items))
            }
            "array" => {
                let mut items = Vec::new();
                if !empty {
                    while !self.close_follows("array")? {
                        items.push(self.value()?);
                    }
                }
                Ok(Plist::Array(items))
            }
            "true" | "false" => {
                if !empty {
                    self.text_until_close(&name)?;
                }
                Ok(Plist::Bool(name == "true"))
            }
            "string" | "integer" | "real" | "date" | "data" => {
                let text = if empty { String::new() } else { self.text_until_close(&name)? };
                match name.as_str() {
                    "string" => Ok(Plist::Str(text)),
                    "date" => Ok(Plist::Date(text.trim().to_string())),
                    "data" => base64_decode(&text).map(Plist::Data).ok_or_else(|| "invalid base64 in <data>".to_string()),
                    "integer" => text.trim().parse().map(Plist::Int).map_err(|_| format!("invalid integer {:?}", text.trim())),
                    _ => text.trim().parse().map(Plist::Real).map_err(|_| format!("invalid real {:?}", text.trim())),
                }
            }
            other => Err(format!("unknown element <{other}>")),
        }
    }
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.' | b'-')
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn decode_entities(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail.find(';') else { return Err("unterminated entity".into()) };
        let name = &tail[1..semi];
        let ch = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => name.strip_prefix('#').and_then(|d| d.parse().ok()),
                };
                code.and_then(char::from_u32).ok_or_else(|| format!("unknown entity &{name};"))?
            }
        };
        out.push(ch);
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// The XML form (`xml1`), tab-indented, with the XML declaration.
pub fn to_xml(v: &Plist) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n");
    write_xml(v, 0, &mut out);
    out.push_str("</plist>\n");
    out
}

fn write_xml(v: &Plist, depth: usize, out: &mut String) {
    let pad = "\t".repeat(depth);
    match v {
        Plist::Bool(b) => {
            let _ = writeln!(out, "{pad}<{}/>", if *b { "true" } else { "false" });
        }
        Plist::Int(n) => {
            let _ = writeln!(out, "{pad}<integer>{n}</integer>");
        }
        Plist::Real(f) => {
            let _ = writeln!(out, "{pad}<real>{f}</real>");
        }
        Plist::Str(s) => {
            let _ = writeln!(out, "{pad}<string>{}</string>", escape_xml(s));
        }
        Plist::Data(d) => {
            let _ = writeln!(out, "{pad}<data>{}</data>", base64_encode(d));
        }
        Plist::Date(d) => {
            let _ = writeln!(out, "{pad}<date>{}</date>", escape_xml(d));
        }
        Plist::Array(items) if items.is_empty() => {
            let _ = writeln!(out, "{pad}<array/>");
        }
        Plist::Array(items) => {
            let _ = writeln!(out, "{pad}<array>");
            for item in items {
                write_xml(item, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</array>");
        }
        Plist::Dict(items) if items.is_empty() => {
            let _ = writeln!(out, "{pad}<dict/>");
        }
        Plist::Dict(items) => {
            let _ = writeln!(out, "{pad}<dict>");
            for (k, item) in items {
                let _ = writeln!(out, "{pad}\t<key>{}</key>", escape_xml(k));
                write_xml(item, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</dict>");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// ASCII (the older text form that `defaults` prints and accepts)

/// Reads the ASCII form: `{ key = value; }` dictionaries, `( a, b )` arrays, `<hex>` data, and quoted or bare strings.
/// A bare word is always a string, so `1` is the text "1".
pub fn parse_ascii(text: &str) -> Result<Plist, String> {
    let mut p = Ascii { s: text.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.pos != p.s.len() {
        return Err(format!("unexpected text after the value at byte {}", p.pos));
    }
    Ok(v)
}

struct Ascii<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Ascii<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.pos))
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if self.s[self.pos..].starts_with(b"//") {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }

    fn value(&mut self) -> Result<Plist, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        break;
                    }
                    let key = self.string_or_word()?;
                    self.expect(b'=')?;
                    let v = self.value()?;
                    self.expect(b';')?;
                    items.push((key, v));
                }
                Ok(Plist::Dict(items))
            }
            Some(b'(') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b')') {
                        self.pos += 1;
                        break;
                    }
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b')') => {}
                        _ => return self.err("expected ',' or ')'"),
                    }
                }
                Ok(Plist::Array(items))
            }
            Some(b'<') => {
                self.pos += 1;
                let begin = self.pos;
                while self.peek().is_some_and(|c| c != b'>') {
                    self.pos += 1;
                }
                if self.peek() != Some(b'>') {
                    return self.err("unterminated <data>");
                }
                let hex: String = String::from_utf8_lossy(&self.s[begin..self.pos]).chars().filter(|c| !c.is_whitespace()).collect();
                self.pos += 1;
                hex_decode(&hex).map(Plist::Data).ok_or_else(|| "invalid hex in <data>".to_string())
            }
            _ => Ok(Plist::Str(self.string_or_word()?)),
        }
    }

    fn string_or_word(&mut self) -> Result<String, String> {
        self.skip_ws();
        if self.peek() == Some(b'"') {
            self.pos += 1;
            let mut out = Vec::new();
            loop {
                match self.peek() {
                    None => return self.err("unterminated string"),
                    Some(b'"') => {
                        self.pos += 1;
                        break;
                    }
                    Some(b'\\') => {
                        let e = self.s.get(self.pos + 1).copied();
                        self.pos += 2;
                        out.push(match e {
                            Some(b'n') => b'\n',
                            Some(b't') => b'\t',
                            Some(b'r') => b'\r',
                            Some(other) => other,
                            None => return self.err("unterminated string"),
                        });
                    }
                    Some(c) => {
                        out.push(c);
                        self.pos += 1;
                    }
                }
            }
            return String::from_utf8(out).map_err(|_| "the string is not UTF-8".to_string());
        }
        let begin = self.pos;
        while self.peek().is_some_and(is_word_byte) {
            self.pos += 1;
        }
        if self.pos == begin {
            return self.err("expected a value");
        }
        Ok(String::from_utf8_lossy(&self.s[begin..self.pos]).into_owned())
    }
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'/' | b':' | b'.' | b'-' | b'+')
}

/// The ASCII form as `defaults read` prints it: `=` between key and value, keys sorted.
pub fn to_ascii(v: &Plist) -> String {
    let mut out = String::new();
    write_ascii(v, 0, " = ", true, &mut out);
    out
}

/// The form `plutil -p` prints: `=>` between key and value, keys sorted.
pub fn to_print(v: &Plist) -> String {
    let mut out = String::new();
    write_ascii(v, 0, " => ", true, &mut out);
    out
}

fn write_ascii(v: &Plist, depth: usize, sep: &str, sorted: bool, out: &mut String) {
    let pad = "    ".repeat(depth);
    match v {
        Plist::Dict(items) if items.is_empty() => out.push_str("{}"),
        Plist::Dict(items) => {
            out.push_str("{\n");
            let mut refs: Vec<&(String, Plist)> = items.iter().collect();
            if sorted {
                refs.sort_by(|a, b| a.0.cmp(&b.0));
            }
            for (k, item) in refs {
                let _ = write!(out, "{pad}    {}{sep}", quote_word(k));
                write_ascii(item, depth + 1, sep, sorted, out);
                out.push_str(";\n");
            }
            let _ = write!(out, "{pad}}}");
        }
        Plist::Array(items) if items.is_empty() => out.push_str("()"),
        Plist::Array(items) => {
            out.push_str("(\n");
            for item in items {
                let _ = write!(out, "{pad}    ");
                write_ascii(item, depth + 1, sep, sorted, out);
                out.push_str(",\n");
            }
            let _ = write!(out, "{pad})");
        }
        Plist::Bool(b) => out.push_str(if *b { "1" } else { "0" }),
        Plist::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Plist::Real(f) => {
            let _ = write!(out, "{f}");
        }
        Plist::Str(s) => out.push_str(&quote_word(s)),
        Plist::Data(d) => {
            let _ = write!(out, "<{}>", hex_encode(d));
        }
        Plist::Date(d) => out.push_str(&quote_word(d)),
    }
}

/// The text as a bare word when it reads back the same, else as a quoted string.
fn quote_word(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(is_word_byte) {
        return s.to_string();
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// JSON (output only: `plutil -convert json`, `-extract json`)

/// The JSON form. Dates have no JSON form and are refused. Data is base64 text. `pretty` indents and sorts keys.
pub fn to_json(v: &Plist, pretty: bool) -> Result<String, String> {
    let mut out = String::new();
    write_json(v, pretty, 0, &mut out)?;
    if pretty {
        out.push('\n');
    }
    Ok(out)
}

fn write_json(v: &Plist, pretty: bool, depth: usize, out: &mut String) -> Result<(), String> {
    let (nl, pad, inner) = if pretty {
        ("\n".to_string(), "  ".repeat(depth), "  ".repeat(depth + 1))
    } else {
        (String::new(), String::new(), String::new())
    };
    match v {
        Plist::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Plist::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Plist::Real(f) => {
            if !f.is_finite() {
                return Err("a non-finite real has no JSON form".into());
            }
            let _ = write!(out, "{f:?}");
        }
        Plist::Str(s) => out.push_str(&json_quote(s)),
        Plist::Date(_) => return Err("dates have no JSON form".into()),
        Plist::Data(d) => out.push_str(&json_quote(&base64_encode(d))),
        Plist::Array(items) if items.is_empty() => out.push_str("[]"),
        Plist::Array(items) => {
            out.push('[');
            out.push_str(&nl);
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                    out.push_str(&nl);
                }
                out.push_str(&inner);
                write_json(item, pretty, depth + 1, out)?;
            }
            out.push_str(&nl);
            out.push_str(&pad);
            out.push(']');
        }
        Plist::Dict(items) if items.is_empty() => out.push_str("{}"),
        Plist::Dict(items) => {
            let mut refs: Vec<&(String, Plist)> = items.iter().collect();
            if pretty {
                refs.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            out.push_str(&nl);
            for (i, (k, item)) in refs.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                    out.push_str(&nl);
                }
                out.push_str(&inner);
                out.push_str(&json_quote(k));
                out.push_str(if pretty { ": " } else { ":" });
                write_json(item, pretty, depth + 1, out)?;
            }
            out.push_str(&nl);
            out.push_str(&pad);
            out.push('}');
        }
    }
    Ok(())
}

fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Converts a parsed JSON fragment (`plutil -insert -json`). Numbers with no fraction become integers.
pub fn from_json(j: &crate::json::Json) -> Result<Plist, String> {
    use crate::json::Json;
    Ok(match j {
        Json::Null => return Err("null has no property-list form".into()),
        Json::Bool(b) => Plist::Bool(*b),
        Json::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Plist::Int(*n as i64),
        Json::Num(n) => Plist::Real(*n),
        Json::Str(s) => Plist::Str(s.clone()),
        Json::Arr(items) => Plist::Array(items.iter().map(from_json).collect::<Result<_, _>>()?),
        Json::Obj(items) => {
            Plist::Dict(items.iter().map(|(k, v)| from_json(v).map(|p| (k.clone(), p))).collect::<Result<_, _>>()?)
        }
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Key paths

/// The components of a key path. The empty path names the whole value.
pub fn components(path: &str) -> Vec<&str> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split('.').collect()
    }
}

fn index_of(seg: &str) -> Option<usize> {
    seg.parse::<usize>().ok()
}

fn dict_slot<'a>(items: &'a mut [(String, Plist)], key: &str) -> Option<&'a mut Plist> {
    items.iter_mut().find(|e| e.0 == key).map(|e| &mut e.1)
}

/// The value at a key path.
pub fn get<'a>(root: &'a Plist, path: &str) -> Result<&'a Plist, String> {
    let mut cur = root;
    for seg in components(path) {
        cur = match cur {
            Plist::Dict(items) => items.iter().find(|e| e.0 == seg).map(|e| &e.1),
            Plist::Array(items) => index_of(seg).and_then(|i| items.get(i)),
            _ => return Err(format!("{seg}: the value is not a dictionary or an array")),
        }
        .ok_or_else(|| format!("no value at key path {path}"))?;
    }
    Ok(cur)
}

fn get_mut<'a>(root: &'a mut Plist, segs: &[&str], path: &str) -> Result<&'a mut Plist, String> {
    let mut cur = root;
    for seg in segs {
        cur = match cur {
            Plist::Dict(items) => dict_slot(items, seg),
            Plist::Array(items) => index_of(seg).and_then(move |i| items.get_mut(i)),
            _ => return Err(format!("{seg}: the value is not a dictionary or an array")),
        }
        .ok_or_else(|| format!("no value at key path {path}"))?;
    }
    Ok(cur)
}

/// Replaces the value at an existing key path.
pub fn replace(root: &mut Plist, path: &str, value: Plist) -> Result<(), String> {
    let segs = components(path);
    let (last, parent) = segs.split_last().ok_or("the key path is empty")?;
    let target = get_mut(root, parent, path)?;
    let slot = match target {
        Plist::Dict(items) => dict_slot(items, last),
        Plist::Array(items) => index_of(last).and_then(move |i| items.get_mut(i)),
        _ => return Err(format!("{last}: the value is not a dictionary or an array")),
    };
    *slot.ok_or_else(|| format!("no value at key path {path}"))? = value;
    Ok(())
}

/// Inserts a value. A dictionary key must be new. An array index inserts before the element at that index, and
/// with `append` the value is added to the end of the array at the path.
pub fn insert(root: &mut Plist, path: &str, value: Plist, append: bool) -> Result<(), String> {
    if append {
        return match get_mut(root, &components(path), path)? {
            Plist::Array(items) => {
                items.push(value);
                Ok(())
            }
            _ => Err("-append needs a key path to an array".into()),
        };
    }
    let segs = components(path);
    let (last, parent) = segs.split_last().ok_or("the key path is empty")?;
    match get_mut(root, parent, path)? {
        Plist::Dict(items) => {
            if items.iter().any(|e| e.0 == *last) {
                return Err(format!("{last}: a value already exists at this key path"));
            }
            items.push((last.to_string(), value));
            Ok(())
        }
        Plist::Array(items) => {
            let i = index_of(last).ok_or_else(|| format!("{last}: an array index must be a whole number"))?;
            if i > items.len() {
                return Err(format!("{last}: index is past the end of the array"));
            }
            items.insert(i, value);
            Ok(())
        }
        _ => Err(format!("{last}: the parent is not a dictionary or an array")),
    }
}

/// Removes the value at a key path.
pub fn remove(root: &mut Plist, path: &str) -> Result<(), String> {
    let segs = components(path);
    let (last, parent) = segs.split_last().ok_or("the key path is empty")?;
    match get_mut(root, parent, path)? {
        Plist::Dict(items) => {
            let at = items.iter().position(|e| e.0 == *last).ok_or_else(|| format!("no value at key path {path}"))?;
            items.remove(at);
            Ok(())
        }
        Plist::Array(items) => {
            let i = index_of(last).filter(|&i| i < items.len()).ok_or_else(|| format!("no value at key path {path}"))?;
            items.remove(i);
            Ok(())
        }
        _ => Err(format!("{last}: the parent is not a dictionary or an array")),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// base64 and hex

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decodes base64, ignoring white space and padding. None for any other character.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        if c.is_ascii_whitespace() || c == b'=' {
            continue;
        }
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes an even-length hex string. None for an odd length or any other character.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Plist {
        Plist::Dict(vec![
            ("Name".into(), Plist::Str("Tom & \"Jerry\" <3".into())),
            ("Count".into(), Plist::Int(-7)),
            ("Ratio".into(), Plist::Real(1.5)),
            ("On".into(), Plist::Bool(true)),
            ("Blob".into(), Plist::Data(vec![0, 1, 2, 254, 255])),
            ("When".into(), Plist::Date("2026-10-13T08:00:00Z".into())),
            ("Empty".into(), Plist::Dict(vec![])),
            ("List".into(), Plist::Array(vec![Plist::Str("a".into()), Plist::Array(vec![])])),
        ])
    }

    #[test]
    fn xml_round_trips_every_type() {
        let v = sample();
        let text = to_xml(&v);
        assert!(text.starts_with("<?xml version=\"1.0\""));
        assert_eq!(parse_xml(&text), Ok(v));
    }

    #[test]
    fn xml_fragments_and_entities() {
        assert_eq!(parse_xml("<integer>42</integer>"), Ok(Plist::Int(42)));
        assert_eq!(parse_xml("<string>a&amp;b &#65;&#x42;</string>"), Ok(Plist::Str("a&b AB".into())));
        assert_eq!(parse_xml("<!DOCTYPE plist><!-- c --><plist version=\"1.0\"><true/></plist>"), Ok(Plist::Bool(true)));
        assert_eq!(parse_xml("<array/>"), Ok(Plist::Array(vec![])));
        assert!(parse_xml("<string>open").is_err());
        assert!(parse_xml("<integer>x</integer>").is_err());
        assert!(parse_xml("<bogus/>").is_err());
    }

    #[test]
    fn ascii_round_trips_and_reads_the_old_syntax() {
        let v = Plist::Dict(vec![
            ("Default Color".into(), Plist::Array(vec![Plist::Str("255".into()), Plist::Str("0".into())])),
            ("Default Font".into(), Plist::Str("Helvetica Neue".into())),
            ("Empty".into(), Plist::Str(String::new())),
        ]);
        let text = to_ascii(&v);
        assert!(text.contains("\"Default Color\" = ("), "{text}");
        assert_eq!(parse_ascii(&text), Ok(v));
        assert_eq!(parse_ascii("(255, 0, 0)"), Ok(Plist::Array(vec![Plist::Str("255".into()), Plist::Str("0".into()), Plist::Str("0".into())])));
        assert_eq!(parse_ascii("<00ff 10>"), Ok(Plist::Data(vec![0, 255, 16])));
        assert_eq!(parse_ascii("{ a = b; // note\n c = \"d\\\"e\"; }"), Ok(Plist::Dict(vec![("a".into(), Plist::Str("b".into())), ("c".into(), Plist::Str("d\"e".into()))])));
        assert!(parse_ascii("{ a = b }").is_err());
        assert!(parse_ascii("(a b)").is_err());
        assert!(parse_ascii("a b").is_err());
    }

    #[test]
    fn print_form_uses_arrows_and_sorted_keys() {
        let v = Plist::Dict(vec![("b".into(), Plist::Bool(false)), ("a".into(), Plist::Int(1))]);
        assert_eq!(to_print(&v), "{\n    a => 1;\n    b => 0;\n}");
    }

    #[test]
    fn json_output_and_its_limits() {
        let v = Plist::Dict(vec![("z".into(), Plist::Real(2.0)), ("a".into(), Plist::Data(b"hi".to_vec()))]);
        assert_eq!(to_json(&v, false), Ok("{\"z\":2.0,\"a\":\"aGk=\"}".into()));
        assert_eq!(to_json(&v, true), Ok("{\n  \"a\": \"aGk=\",\n  \"z\": 2.0\n}\n".into()));
        assert!(to_json(&Plist::Date("x".into()), false).is_err());
        assert!(to_json(&Plist::Real(f64::NAN), false).is_err());
        assert_eq!(to_json(&Plist::Str("a\n\"\u{1}".into()), false), Ok("\"a\\n\\\"\\u0001\"".into()));
    }

    #[test]
    fn json_fragments_become_plist_values() {
        let j = crate::json::parse("{\"n\": 3, \"f\": 2.5, \"ok\": true, \"s\": [\"x\"]}").unwrap();
        assert_eq!(
            from_json(&j),
            Ok(Plist::Dict(vec![
                ("n".into(), Plist::Int(3)),
                ("f".into(), Plist::Real(2.5)),
                ("ok".into(), Plist::Bool(true)),
                ("s".into(), Plist::Array(vec![Plist::Str("x".into())])),
            ]))
        );
    }

    #[test]
    fn base64_round_trips_and_matches_the_standard() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        for n in 0..40u8 {
            let data: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64_encode(&data)), Some(data));
        }
        assert_eq!(base64_decode("TW Fu\n"), Some(b"Man".to_vec()));
        assert_eq!(base64_decode("%%"), None);
    }

    #[test]
    fn binary_input_is_refused_by_name() {
        assert!(parse_bytes(b"bplist00xxxx").is_err());
        assert_eq!(parse_bytes(&[0xff, 0xfe]), Err("the file is not text".into()));
        assert_eq!(parse_bytes(b"  <integer>1</integer>"), Ok(Plist::Int(1)));
        assert_eq!(parse_bytes(b"{ k = v; }"), Ok(Plist::Dict(vec![("k".into(), Plist::Str("v".into()))])));
    }

    #[test]
    fn key_paths_read_and_write_dictionaries_and_arrays() {
        let mut v = Plist::Dict(vec![("list".into(), Plist::Array(vec![Plist::Int(1), Plist::Int(2)]))]);
        assert_eq!(get(&v, "list.1"), Ok(&Plist::Int(2)));
        assert_eq!(get(&v, ""), Ok(&v.clone()));
        assert!(get(&v, "list.9").is_err());
        assert!(get(&v, "list.x").is_err());
        assert!(get(&v, "list.0.deeper").is_err());

        replace(&mut v, "list.0", Plist::Int(10)).unwrap();
        assert_eq!(get(&v, "list.0"), Ok(&Plist::Int(10)));
        assert!(replace(&mut v, "missing", Plist::Int(0)).is_err());

        insert(&mut v, "list.1", Plist::Str("mid".into()), false).unwrap();
        insert(&mut v, "list", Plist::Str("end".into()), true).unwrap();
        assert!(insert(&mut v, "list.99", Plist::Int(0), false).is_err());
        assert_eq!(get(&v, "list.1"), Ok(&Plist::Str("mid".into())));
        assert_eq!(get(&v, "list.3"), Ok(&Plist::Str("end".into())));

        insert(&mut v, "added", Plist::Bool(false), false).unwrap();
        assert!(insert(&mut v, "added", Plist::Bool(true), false).is_err());
        assert!(insert(&mut v, "list", Plist::Int(1), true).is_ok());
        assert!(insert(&mut v, "added", Plist::Int(1), true).is_err());

        remove(&mut v, "list.0").unwrap();
        remove(&mut v, "added").unwrap();
        assert!(remove(&mut v, "added").is_err());
        assert!(remove(&mut v, "").is_err());
        assert_eq!(get(&v, "list.0"), Ok(&Plist::Str("mid".into())));
    }

    #[test]
    fn type_names_match_the_tool_vocabulary() {
        assert_eq!(Plist::Real(0.0).type_name(), "float");
        assert_eq!(Plist::Dict(vec![]).type_name(), "dictionary");
        assert_eq!(Plist::Bool(true).type_name(), "bool");
    }
}
