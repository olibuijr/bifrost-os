//! Structured output while it is still being written, and the schema in the form an engine can enforce.
//!
//! - [`partial_parse`] reads the JSON a model has produced so far (a prefix of the reply, as streaming delivers it) into the
//!   [`Value`] it has so far: a string cut in the middle is the text so far, an array or object keeps the members that are
//!   whole, and a member whose name or value has not started is left out. A view binds to this to fill in a form or a card
//!   as the model writes (`PartiallyGenerated`). A prefix that cannot become valid JSON is an error, as for [`parse`].
//! - [`validate_partial`] checks that value against the schema without counting what has not arrived yet: a member not
//!   written yet is not "missing", a choice that is the start of an option is fine, and a number's range is left to the
//!   final [`read_reply`](crate::generation_schema::read_reply) because the number may still be growing.
//! - [`json_schema`] writes the schema as a JSON Schema document (`type`, `properties`, `required`, `enum`, `minimum` and
//!   `maximum`, `additionalProperties: false`), which an engine that constrains decoding takes as a response format.
//!
//! The partial reader does not guess: it never completes a number or a literal that is not whole (`tru` is nothing yet), and
//! a number that touches the end of the text is returned as written so far, since `12` may become `123`.

use crate::generation_schema::{Issue, Kind, ParseError, Property, Value};
use bifrost_core::json::Json;

/// Nesting deeper than this is refused, as in the strict parser.
const MAX_DEPTH: usize = 64;

/// Reads the JSON in `text` as far as it goes. `Ok(None)` when no value has started yet (empty text, an opening Markdown
/// fence, or a literal still being spelled). A Markdown code fence around the reply is skipped.
pub fn partial_parse(text: &str) -> Result<Option<Value>, ParseError> {
    let body = strip_open_fence(text);
    let mut p = Reader { src: body, bytes: body.as_bytes(), pos: 0 };
    let Some((value, _)) = p.value(0)? else { return Ok(None) };
    p.skip_space();
    // a closing fence after a finished value is fine
    let rest = &body[p.pos..];
    if !rest.is_empty() && rest.trim() != "```" {
        return Err(p.error("text after the value"));
    }
    Ok(Some(value))
}

fn strip_open_fence(text: &str) -> &str {
    let t = text.trim_start();
    let Some(rest) = t.strip_prefix("```") else { return t };
    match rest.find('\n') {
        Some(i) => &rest[i + 1..],
        // the fence line itself is still arriving
        None => "",
    }
}

struct Reader<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

type Step = Result<Option<(Value, bool)>, ParseError>;

impl Reader<'_> {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError { at: self.pos, message }
    }

    fn skip_space(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// A value and whether it is whole; `None` when nothing of it can be shown yet.
    fn value(&mut self, depth: usize) -> Step {
        self.skip_space();
        match self.bytes.get(self.pos) {
            None => Ok(None),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => {
                let (s, whole) = self.string()?;
                Ok(Some((Value::Text(s), whole)))
            }
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Step {
        let rest = &self.bytes[self.pos..];
        if rest.starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(Some((value, true)))
        } else if word.as_bytes().starts_with(rest) {
            // still being spelled
            self.pos = self.bytes.len();
            Ok(None)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn number(&mut self) -> Step {
        let start = self.pos;
        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')) {
            self.pos += 1;
        }
        let token = &self.src[start..self.pos];
        if self.eof() {
            // cut at the end: drop what cannot end a number, and show the rest as written so far
            let mut t = token;
            while !t.is_empty() {
                if let Some(n) = parse_number(t) {
                    return Ok(Some((Value::Number(n), false)));
                }
                t = &t[..t.len() - 1];
            }
            return Ok(None);
        }
        match parse_number(token) {
            Some(n) => Ok(Some((Value::Number(n), true))),
            None => {
                self.pos = start;
                Err(self.error("not a number"))
            }
        }
    }

    fn object(&mut self, depth: usize) -> Step {
        if depth >= MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.pos += 1;
        let mut members: Vec<(String, Value)> = Vec::new();
        let mut need_comma = false;
        loop {
            self.skip_space();
            match self.bytes.get(self.pos) {
                None => return Ok(Some((Value::Object(members), false))),
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Some((Value::Object(members), true)));
                }
                Some(b',') if need_comma => {
                    self.pos += 1;
                    need_comma = false;
                }
                Some(b'"') if !need_comma => {
                    let (key, whole) = self.string()?;
                    if !whole {
                        return Ok(Some((Value::Object(members), false)));
                    }
                    self.skip_space();
                    if self.eof() {
                        return Ok(Some((Value::Object(members), false)));
                    }
                    if self.bytes[self.pos] != b':' {
                        return Err(self.error("expected ':' after a member name"));
                    }
                    self.pos += 1;
                    match self.value(depth + 1)? {
                        None => return Ok(Some((Value::Object(members), false))),
                        Some((v, whole)) => {
                            members.push((key, v));
                            if !whole {
                                return Ok(Some((Value::Object(members), false)));
                            }
                            need_comma = true;
                        }
                    }
                }
                Some(_) => return Err(self.error(if need_comma { "expected ',' or '}'" } else { "expected a member name" })),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Step {
        if depth >= MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.pos += 1;
        let mut items: Vec<Value> = Vec::new();
        let mut need_comma = false;
        loop {
            self.skip_space();
            match self.bytes.get(self.pos) {
                None => return Ok(Some((Value::Array(items), false))),
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Some((Value::Array(items), true)));
                }
                Some(b',') if need_comma => {
                    self.pos += 1;
                    need_comma = false;
                }
                Some(_) if need_comma => return Err(self.error("expected ',' or ']'")),
                Some(_) => match self.value(depth + 1)? {
                    None => return Ok(Some((Value::Array(items), false))),
                    Some((v, whole)) => {
                        items.push(v);
                        if !whole {
                            return Ok(Some((Value::Array(items), false)));
                        }
                        need_comma = true;
                    }
                },
            }
        }
    }

    /// A string and whether its closing quote has arrived. A cut escape is dropped.
    fn string(&mut self) -> Result<(String, bool), ParseError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Ok((out, false)),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok((out, true));
                }
                Some(b'\\') => {
                    let Some(&code) = self.bytes.get(self.pos + 1) else {
                        self.pos = self.bytes.len();
                        return Ok((out, false));
                    };
                    let simple = match code {
                        b'"' => Some('"'),
                        b'\\' => Some('\\'),
                        b'/' => Some('/'),
                        b'b' => Some('\u{8}'),
                        b'f' => Some('\u{c}'),
                        b'n' => Some('\n'),
                        b'r' => Some('\r'),
                        b't' => Some('\t'),
                        _ => None,
                    };
                    if let Some(c) = simple {
                        out.push(c);
                        self.pos += 2;
                    } else if code == b'u' {
                        match self.unicode_escape()? {
                            Some(c) => out.push(c),
                            None => return Ok((out, false)),
                        }
                    } else {
                        self.pos += 1;
                        return Err(self.error("invalid escape in a string"));
                    }
                }
                Some(&b) if b < 0x20 => return Err(self.error("control character in a string")),
                Some(_) => {
                    let ch = self.src[self.pos..].chars().next().ok_or(self.error("unterminated string"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    /// Reads `\uXXXX` (and a following low surrogate) at `pos`; `None` when the text ends inside it.
    fn unicode_escape(&mut self) -> Result<Option<char>, ParseError> {
        let at = self.pos;
        let Some(high) = self.hex4(at + 2)? else {
            self.pos = self.bytes.len();
            return Ok(None);
        };
        if (0xDC00..0xE000).contains(&high) {
            self.pos = at + 2;
            return Err(self.error("lone surrogate in a string"));
        }
        if !(0xD800..0xDC00).contains(&high) {
            self.pos = at + 6;
            return char::from_u32(high).map(Some).ok_or(self.error("invalid code point"));
        }
        // a high surrogate needs `\uXXXX` after it
        let rest = &self.bytes[(at + 6).min(self.bytes.len())..];
        if rest.len() < 2 {
            if rest.is_empty() || rest == b"\\" {
                self.pos = self.bytes.len();
                return Ok(None);
            }
            self.pos = at + 6;
            return Err(self.error("lone surrogate in a string"));
        }
        if &rest[..2] != b"\\u" {
            self.pos = at + 6;
            return Err(self.error("lone surrogate in a string"));
        }
        let Some(low) = self.hex4(at + 8)? else {
            self.pos = self.bytes.len();
            return Ok(None);
        };
        if !(0xDC00..0xE000).contains(&low) {
            self.pos = at + 8;
            return Err(self.error("lone surrogate in a string"));
        }
        self.pos = at + 12;
        char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)).map(Some).ok_or(self.error("invalid code point"))
    }

    /// Four hex digits at `from`; `None` when the text ends first (and what is there is hex so far).
    fn hex4(&self, from: usize) -> Result<Option<u32>, ParseError> {
        let mut v = 0u32;
        for i in 0..4 {
            match self.bytes.get(from + i) {
                None => return Ok(None),
                Some(&b) => match (b as char).to_digit(16) {
                    Some(d) => v = v * 16 + d,
                    None => return Err(ParseError { at: from + i, message: "invalid \\u escape" }),
                },
            }
        }
        Ok(Some(v))
    }
}

/// A number in JSON form: no leading `+`, no leading zeros (`01`), digits on both sides of a `.`, digits after `e`.
fn parse_number(token: &str) -> Option<f64> {
    let t = token.strip_prefix('-').unwrap_or(token);
    let digits_end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let int = &t[..digits_end];
    if int.is_empty() || (int.len() > 1 && int.starts_with('0')) {
        return None;
    }
    let mut rest = &t[digits_end..];
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.find(|c: char| !c.is_ascii_digit()).unwrap_or(frac.len());
        if n == 0 {
            return None;
        }
        rest = &frac[n..];
    }
    if let Some(exp) = rest.strip_prefix(['e', 'E']) {
        let exp = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        if exp.is_empty() || !exp.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest = "";
    }
    if !rest.is_empty() {
        return None;
    }
    token.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Checks a value read by [`partial_parse`] against `schema`, counting only what is already wrong: a member that has not
/// arrived is not reported, a text that begins an option of a choice is accepted, and number ranges are left to the final
/// check. An empty result means the value so far can still become a reply that fits.
pub fn validate_partial(schema: &Kind, value: &Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    check(schema, value, "$", &mut issues);
    issues
}

fn problem(issues: &mut Vec<Issue>, path: &str, message: impl Into<String>) {
    issues.push(Issue { path: path.into(), message: message.into() });
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Text => "text",
        Kind::Bool => "boolean",
        Kind::Number(_) => "number",
        Kind::Choice(_) => "choice",
        Kind::Array(_) => "array",
        Kind::Object(_) => "object",
    }
}

fn value_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::Text(_) => "text",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn check(kind: &Kind, value: &Value, path: &str, issues: &mut Vec<Issue>) {
    match (kind, value) {
        (Kind::Text, Value::Text(_)) | (Kind::Bool, Value::Bool(_)) | (Kind::Number(_), Value::Number(_)) => {}
        (Kind::Choice(options), Value::Text(t)) => {
            if !options.iter().any(|o| o.starts_with(t.as_str())) {
                problem(issues, path, format!("\"{t}\" is not the start of any choice"));
            }
        }
        (Kind::Array(item), Value::Array(values)) => {
            for (i, v) in values.iter().enumerate() {
                check(item, v, &format!("{path}[{i}]"), issues);
            }
        }
        (Kind::Object(props), Value::Object(fields)) => {
            for (key, v) in fields {
                let here = format!("{path}.{key}");
                match props.iter().find(|p| p.name == *key) {
                    None => problem(issues, &here, "not in the schema"),
                    Some(Property { kind, optional, .. }) => match v {
                        Value::Null if *optional => {}
                        Value::Null => problem(issues, &here, "is null"),
                        v => check(kind, v, &here, issues),
                    },
                }
            }
        }
        (expected, found) => problem(issues, path, format!("expected {}, found {}", kind_name(expected), value_name(found))),
    }
}

/// The schema as a JSON Schema document, compact text. Members are `required` unless optional, no other members are
/// allowed, a choice is a string `enum`, and a number range is `minimum` and `maximum`.
pub fn json_schema(schema: &Kind) -> String {
    schema_node(schema).encode()
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn schema_node(kind: &Kind) -> Json {
    match kind {
        Kind::Text => obj(vec![("type", Json::Str("string".into()))]),
        Kind::Bool => obj(vec![("type", Json::Str("boolean".into()))]),
        Kind::Number(None) => obj(vec![("type", Json::Str("number".into()))]),
        Kind::Number(Some((low, high))) => obj(vec![("type", Json::Str("number".into())), ("minimum", Json::Num(*low)), ("maximum", Json::Num(*high))]),
        Kind::Choice(options) => obj(vec![("type", Json::Str("string".into())), ("enum", Json::Arr(options.iter().cloned().map(Json::Str).collect()))]),
        Kind::Array(item) => obj(vec![("type", Json::Str("array".into())), ("items", schema_node(item))]),
        Kind::Object(props) => {
            let properties = props
                .iter()
                .map(|p| {
                    let mut node = match schema_node(&p.kind) {
                        Json::Obj(fields) => fields,
                        other => vec![("schema".to_string(), other)],
                    };
                    if !p.description.is_empty() {
                        node.push(("description".to_string(), Json::Str(p.description.clone())));
                    }
                    (p.name.clone(), Json::Obj(node))
                })
                .collect();
            let required = props.iter().filter(|p| !p.optional).map(|p| Json::Str(p.name.clone())).collect();
            obj(vec![
                ("type", Json::Str("object".into())),
                ("properties", Json::Obj(properties)),
                ("required", Json::Arr(required)),
                ("additionalProperties", Json::Bool(false)),
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_schema::parse;

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn obj_of(members: Vec<(&str, Value)>) -> Value {
        Value::Object(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn read(s: &str) -> Option<Value> {
        partial_parse(s).unwrap()
    }

    #[test]
    fn nothing_is_shown_until_a_value_starts() {
        assert_eq!(read(""), None);
        assert_eq!(read("   \n"), None);
        assert_eq!(read("```json"), None);
        assert_eq!(read("```json\n"), None);
        assert_eq!(read("tr"), None, "a literal still being spelled");
        assert_eq!(read("-"), None);
    }

    #[test]
    fn a_cut_string_is_the_text_so_far() {
        assert_eq!(read("\"Hel"), Some(text("Hel")));
        assert_eq!(read("\"a\\nb"), Some(text("a\nb")));
        assert_eq!(read("\"cut escape\\"), Some(text("cut escape")), "a dangling backslash is dropped");
        assert_eq!(read("\"é ü"), Some(text("é ü")));
        assert_eq!(read("\"snow \\u26"), Some(text("snow ")), "a cut \\u escape is dropped");
        assert_eq!(read("\"snow \\u2603 man"), Some(text("snow \u{2603} man")));
        assert_eq!(read("\"emoji \\ud83d"), Some(text("emoji ")));
        assert_eq!(read("\"emoji \\ud83d\\ude0"), Some(text("emoji ")));
        assert_eq!(read("\"emoji \\ud83d\\ude00"), Some(text("emoji \u{1f600}")));
    }

    #[test]
    fn objects_keep_the_members_that_are_whole_and_the_text_of_the_one_in_progress() {
        assert_eq!(read("{"), Some(obj_of(vec![])));
        assert_eq!(read("{\"ti"), Some(obj_of(vec![])), "a member name still being written is not shown");
        assert_eq!(read("{\"title\""), Some(obj_of(vec![])));
        assert_eq!(read("{\"title\":"), Some(obj_of(vec![])));
        assert_eq!(read("{\"title\": \"Lunch w"), Some(obj_of(vec![("title", text("Lunch w"))])));
        assert_eq!(read("{\"title\":\"Lunch\",\"all_day\": tr"), Some(obj_of(vec![("title", text("Lunch"))])));
        assert_eq!(read("{\"title\":\"Lunch\",\"all_day\": true, \"n\": 1"), Some(obj_of(vec![("title", text("Lunch")), ("all_day", Value::Bool(true)), ("n", Value::Number(1.0))])));
        assert_eq!(read("{\"a\":1}"), Some(obj_of(vec![("a", Value::Number(1.0))])));
    }

    #[test]
    fn arrays_keep_whole_items_and_the_one_in_progress() {
        assert_eq!(read("[1, 2"), Some(Value::Array(vec![Value::Number(1.0), Value::Number(2.0)])));
        assert_eq!(read("[1, 2, "), Some(Value::Array(vec![Value::Number(1.0), Value::Number(2.0)])));
        assert_eq!(read("[\"a\", \"b"), Some(Value::Array(vec![text("a"), text("b")])));
        assert_eq!(read("[{\"x\":1},{\"x\""), Some(Value::Array(vec![obj_of(vec![("x", Value::Number(1.0))]), obj_of(vec![])])));
        assert_eq!(read("[[1],[2,"), Some(Value::Array(vec![Value::Array(vec![Value::Number(1.0)]), Value::Array(vec![Value::Number(2.0)])])));
    }

    #[test]
    fn a_number_at_the_end_is_shown_as_far_as_it_is_valid() {
        assert_eq!(read("[12"), Some(Value::Array(vec![Value::Number(12.0)])));
        assert_eq!(read("[1."), Some(Value::Array(vec![Value::Number(1.0)])));
        assert_eq!(read("[1.5e"), Some(Value::Array(vec![Value::Number(1.5)])));
        assert_eq!(read("[-"), Some(Value::Array(vec![])));
        assert_eq!(read("[-3.25e+2]"), Some(Value::Array(vec![Value::Number(-325.0)])));
        assert!(partial_parse("[01]").is_err(), "a leading zero is not JSON");
        assert!(partial_parse("[1.]").is_err(), "a finished number must be valid");
    }

    #[test]
    fn every_prefix_of_valid_json_reads_and_the_whole_equals_the_strict_parse() {
        let json = "{\"title\":\"Dinner \\u00e9 \\ud83d\\ude00\",\"guests\":[\"Ada\",\"Bob\"],\"n\":-12.5e1,\"ok\":true,\"x\":null,\"o\":{\"k\":[1,2,{\"z\":false}]}}";
        for end in 0..=json.len() {
            if !json.is_char_boundary(end) {
                continue;
            }
            assert!(partial_parse(&json[..end]).is_ok(), "prefix of {end} bytes");
        }
        assert_eq!(partial_parse(json).unwrap(), Some(parse(json).unwrap()));
    }

    #[test]
    fn prefixes_grow_monotonically_for_a_string_member() {
        let json = "{\"summary\":\"Meet at noon\"}";
        let mut last = String::new();
        for end in 0..=json.len() {
            if let Some(Value::Object(m)) = partial_parse(&json[..end]).unwrap() {
                if let Some((_, Value::Text(t))) = m.first() {
                    assert!(t.starts_with(&last), "{t:?} follows {last:?}");
                    last = t.clone();
                }
            }
        }
        assert_eq!(last, "Meet at noon");
    }

    #[test]
    fn a_prefix_that_cannot_become_json_is_an_error() {
        assert!(partial_parse("{\"a\" 1}").is_err());
        assert!(partial_parse("{\"a\":1 \"b\":2}").is_err());
        assert!(partial_parse("[1 2]").is_err());
        assert!(partial_parse("[,1]").is_err());
        assert!(partial_parse("{1:2}").is_err());
        assert!(partial_parse("nope").is_err());
        assert!(partial_parse("\"a\\q\"").is_err());
        assert!(partial_parse("\"\\ud83d x\"").is_err());
        assert!(partial_parse("\"\\ude00\"").is_err());
        assert!(partial_parse("\"\\u12g4\"").is_err());
        assert!(partial_parse("\"tab\there\"").is_err());
        assert!(partial_parse("1 2").is_err());
        assert!(partial_parse(&"[".repeat(65)).is_err(), "nesting is limited");
        assert!(partial_parse(&"[".repeat(60)).is_ok());
    }

    #[test]
    fn a_markdown_fence_is_skipped() {
        assert_eq!(read("```json\n{\"a\": 1"), Some(obj_of(vec![("a", Value::Number(1.0))])));
        assert_eq!(read("```json\n{\"a\": 1}\n```"), Some(obj_of(vec![("a", Value::Number(1.0))])));
        assert_eq!(read("```\n[1]"), Some(Value::Array(vec![Value::Number(1.0)])));
        assert!(partial_parse("{\"a\":1} trailing").is_err());
    }

    fn event_schema() -> Kind {
        Kind::Object(vec![
            Property::new("title", Kind::Text).described("what it is"),
            Property::new("day", Kind::choice(&["today", "tomorrow"])),
            Property::new("guests", Kind::array_of(Kind::Text)).optional(),
            Property::new("minutes", Kind::number_in(0.0, 1439.0)),
        ])
    }

    #[test]
    fn a_value_so_far_is_valid_while_it_can_still_fit() {
        let s = event_schema();
        for prefix in ["{", "{\"title\":\"Lu", "{\"title\":\"Lunch\",\"day\":\"tom", "{\"title\":\"Lunch\",\"day\":\"tomorrow\",\"minutes\":1", "{\"title\":\"Lunch\",\"guests\":[\"Ada\""] {
            let v = read(prefix).unwrap();
            assert_eq!(validate_partial(&s, &v), vec![], "{prefix}");
        }
    }

    #[test]
    fn a_value_that_can_no_longer_fit_is_reported_with_its_path() {
        let s = event_schema();
        let v = read("{\"title\":3,\"day\":\"yesterday\",\"guests\":[\"Ada\",4],\"extra\":1,\"minutes\":null").unwrap();
        let issues = validate_partial(&s, &v);
        let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["$.title", "$.day", "$.guests[1]", "$.extra", "$.minutes"]);
        assert_eq!(issues[1].message, "\"yesterday\" is not the start of any choice");
        assert_eq!(issues[4].message, "is null");
        assert_eq!(validate_partial(&Kind::Text, &Value::Bool(true))[0].message, "expected text, found boolean");
    }

    #[test]
    fn the_schema_exports_as_json_schema() {
        let out = json_schema(&event_schema());
        let j = bifrost_core::json::parse(&out).unwrap();
        assert_eq!(j.str_field("type").as_deref(), Some("object"));
        assert_eq!(j.get("additionalProperties"), Some(&Json::Bool(false)));
        let required: Vec<&str> = j.get("required").and_then(Json::as_array).unwrap().iter().filter_map(Json::as_str).collect();
        assert_eq!(required, ["title", "day", "minutes"], "the optional member is not required");
        let props = j.get("properties").unwrap();
        assert_eq!(props.get("title").unwrap().str_field("description").as_deref(), Some("what it is"));
        let day = props.get("day").unwrap();
        assert_eq!(day.str_field("type").as_deref(), Some("string"));
        assert_eq!(day.get("enum").and_then(Json::as_array).unwrap().len(), 2);
        let minutes = props.get("minutes").unwrap();
        assert_eq!((minutes.get("minimum").and_then(Json::number), minutes.get("maximum").and_then(Json::number)), (Some(0.0), Some(1439.0)));
        let guests = props.get("guests").unwrap();
        assert_eq!(guests.get("items").unwrap().str_field("type").as_deref(), Some("string"));
        assert_eq!(json_schema(&Kind::Bool), "{\"type\":\"boolean\"}");
        assert_eq!(json_schema(&Kind::number()), "{\"type\":\"number\"}");
    }
}
