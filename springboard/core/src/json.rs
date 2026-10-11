//! A small JSON reader/writer for the store catalogue and the installed-apps
//! registry (no serde in the shell core).

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(v) => v.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if *n >= 0.0 => Some(*n as u64),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
    pub fn string(&self) -> Option<&str> { self.as_str() }
    pub fn array(&self) -> Option<&[Json]> { self.as_array() }
    pub fn number(&self) -> Option<f64> {
        match self { Json::Num(n) => Some(*n), _ => None }
    }
    pub fn str_field(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Json::as_str).map(str::to_string)
    }
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = P { b: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err("trailing data after JSON".into());
    }
    Ok(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        self.depth += 1;
        if self.depth > 32 {
            return Err("JSON nested too deeply".into());
        }
        let r = match self.b.get(self.i) {
            None => Err("unexpected end of JSON".into()),
            Some(b'{') => {
                self.i += 1;
                let mut out = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.eat(b':')?;
                        out.push((k, self.value()?));
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
                        }
                    }
                }
                Ok(Json::Obj(out))
            }
            Some(b'[') => {
                self.i += 1;
                let mut out = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        out.push(self.value()?);
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
                        }
                    }
                }
                Ok(Json::Arr(out))
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse().ok()).map(Json::Num).ok_or_else(|| format!("bad value at byte {s}"))
            }
        };
        self.depth -= 1;
        r
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(format!("expected string at byte {}", self.i));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("unterminated escape")?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = self.b.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok()).ok_or("bad \\u escape")?;
                            self.i += 4;
                            char::from_u32(u32::from_str_radix(hex, 16).map_err(|_| "bad \\u escape")?).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| "string is not UTF-8".into())
    }
}

/// `s` as a quoted JSON string.
pub fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values_and_escapes() {
        let v = parse(r#" {"a":[1,2.5,"x\né"],"b":{"c":true,"d":null}} "#).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[2].as_str(), Some("x\n\u{e9}"));
        assert_eq!(v.get("b").unwrap().get("c"), Some(&Json::Bool(true)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{").is_err() && parse("[1,]").is_err() && parse("{} x").is_err());
        assert_eq!(parse(&quote("a\"b\\c\n")).unwrap().as_str(), Some("a\"b\\c\n"));
    }
}
