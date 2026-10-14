//! Structured output: a schema for the shape of a generated value, and a check of a value against it.
//! Objects have described, typed members, string choices and numeric ranges, as a generation schema does. Nothing here
//! constrains a model while it writes. An app uses `validate` to learn whether a reply fits, and `describe` to put the
//! shape in its prompt.

use std::fmt;

/// A value as a model returns it: the content of a structured response.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Value>),
    /// Members in the order they arrived.
    Object(Vec<(String, Value)>),
}

/// The type of a schema node.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text,
    /// A number, optionally limited to the inclusive range `low..=high`.
    Number(Option<(f64, f64)>),
    Bool,
    /// A string that must be one of the choices.
    Choice(Vec<String>),
    Array(Box<Kind>),
    Object(Vec<Property>),
}

/// A named member of an object schema.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub kind: Kind,
    /// What the member means. It is shown to the model with the schema.
    pub description: String,
    /// An optional member may be missing or null.
    pub optional: bool,
}

/// One problem found by `validate`: where it is and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl Kind {
    pub fn number() -> Kind {
        Kind::Number(None)
    }

    pub fn number_in(low: f64, high: f64) -> Kind {
        Kind::Number(Some((low, high)))
    }

    pub fn array_of(item: Kind) -> Kind {
        Kind::Array(Box::new(item))
    }

    pub fn choice(options: &[&str]) -> Kind {
        Kind::Choice(options.iter().map(|s| s.to_string()).collect())
    }

    /// A compact text form of the schema, for a prompt. It lists each member with its type, choices, range and
    /// description. Optional members have a `?` after the name.
    pub fn describe(&self) -> String {
        match self {
            Kind::Text => "text".into(),
            Kind::Bool => "boolean".into(),
            Kind::Number(None) => "number".into(),
            Kind::Number(Some((low, high))) => format!("number from {low} to {high}"),
            Kind::Choice(options) => format!("one of {}", quoted(options)),
            Kind::Array(item) => format!("array of {}", item.describe()),
            Kind::Object(props) => {
                let members: Vec<String> = props
                    .iter()
                    .map(|p| {
                        let mut line = format!("{}{}: {}", p.name, if p.optional { "?" } else { "" }, p.kind.describe());
                        if !p.description.is_empty() {
                            line.push_str(&format!(" ({})", p.description));
                        }
                        line
                    })
                    .collect();
                format!("object {{ {} }}", members.join(", "))
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Bool => "boolean",
            Kind::Number(_) => "number",
            Kind::Choice(_) => "choice",
            Kind::Array(_) => "array",
            Kind::Object(_) => "object",
        }
    }
}

impl Value {
    fn name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

impl Property {
    pub fn new(name: &str, kind: Kind) -> Self {
        Property { name: name.into(), kind, description: String::new(), optional: false }
    }

    pub fn described(mut self, text: &str) -> Self {
        self.description = text.into();
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

fn quoted(items: &[String]) -> String {
    items.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ")
}

/// Checks `value` against `schema`. An empty result means the value fits. Every problem is reported, members in schema
/// order first and then unknown members, so a caller can show them all at once.
pub fn validate(schema: &Kind, value: &Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    check(schema, value, "$", &mut issues);
    issues
}

fn problem(issues: &mut Vec<Issue>, path: &str, message: impl Into<String>) {
    issues.push(Issue { path: path.into(), message: message.into() });
}

fn check(kind: &Kind, value: &Value, path: &str, issues: &mut Vec<Issue>) {
    match (kind, value) {
        (Kind::Text, Value::Text(_)) | (Kind::Bool, Value::Bool(_)) => {}
        (Kind::Number(range), Value::Number(n)) => {
            if !n.is_finite() {
                problem(issues, path, "number is not finite");
            } else if let Some((low, high)) = range {
                if n < low || n > high {
                    problem(issues, path, format!("{n} is outside {low}..={high}"));
                }
            }
        }
        (Kind::Choice(options), Value::Text(t)) => {
            if !options.iter().any(|o| o == t) {
                problem(issues, path, format!("\"{t}\" is not one of {}", quoted(options)));
            }
        }
        (Kind::Array(item), Value::Array(values)) => {
            for (i, v) in values.iter().enumerate() {
                check(item, v, &format!("{path}[{i}]"), issues);
            }
        }
        (Kind::Object(props), Value::Object(fields)) => check_object(props, fields, path, issues),
        (expected, found) => {
            problem(issues, path, format!("expected {}, found {}", expected.name(), found.name()));
        }
    }
}

fn check_object(props: &[Property], fields: &[(String, Value)], path: &str, issues: &mut Vec<Issue>) {
    for prop in props {
        let here = format!("{path}.{}", prop.name);
        let found = fields.iter().find(|(k, _)| *k == prop.name).map(|(_, v)| v);
        match found {
            None | Some(Value::Null) if prop.optional => {}
            None => problem(issues, &here, "missing"),
            Some(Value::Null) => problem(issues, &here, "is null"),
            Some(v) => check(&prop.kind, v, &here, issues),
        }
    }
    for (key, _) in fields {
        if !props.iter().any(|p| p.name == *key) {
            problem(issues, &format!("{path}.{key}"), "not in the schema");
        }
    }
}

/// Why a text is not a JSON value: the byte offset where the reading stopped, and what was expected there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.at, self.message)
    }
}

/// Nesting deeper than this is refused, so a hostile reply cannot exhaust the stack.
const MAX_DEPTH: usize = 64;

/// Reads one JSON value from `text`. Surrounding white space is allowed; anything else after the value is an error.
/// Object members keep their order, and a repeated key is kept twice, as it arrived.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut p = Parser { src: text, bytes: text.as_bytes(), pos: 0 };
    p.skip_space();
    let value = p.value(0)?;
    p.skip_space();
    if p.pos != p.bytes.len() {
        return Err(p.error("text after the value"));
    }
    Ok(value)
}

/// The instruction an app puts in front of a model so that its reply has the shape of `schema`.
pub fn instructions(schema: &Kind) -> String {
    format!("Reply with only one JSON value of this shape: {}. Put no text before or after the JSON.", schema.describe())
}

/// Why a reply cannot be used: it is not JSON at all, or it is JSON that does not fit the schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyError {
    NotJson(ParseError),
    DoesNotFit(Vec<Issue>),
}

/// Reads a model's reply against `schema`. Models often wrap JSON in a Markdown code fence, so a fence around the
/// reply is removed first. The value is returned only when `validate` finds no issue.
pub fn read_reply(schema: &Kind, reply: &str) -> Result<Value, ReplyError> {
    let value = parse(unfence(reply)).map_err(ReplyError::NotJson)?;
    let issues = validate(schema, &value);
    if issues.is_empty() {
        Ok(value)
    } else {
        Err(ReplyError::DoesNotFit(issues))
    }
}

/// The text inside a Markdown code fence, or the trimmed reply when it has no fence.
fn unfence(reply: &str) -> &str {
    let trimmed = reply.trim();
    let Some(rest) = trimmed.strip_prefix("```") else { return trimmed };
    let Some(newline) = rest.find('\n') else { return trimmed };
    let body = rest[newline + 1..].trim_end();
    body.strip_suffix("```").unwrap_or(body).trim()
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError { at: self.pos, message }
    }

    fn skip_space(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        match self.bytes.get(self.pos) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::Text),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.pos += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.eat(b'}') {
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_space();
            if self.bytes.get(self.pos) != Some(&b'"') {
                return Err(self.error("expected a member name"));
            }
            let key = self.string()?;
            self.skip_space();
            if !self.eat(b':') {
                return Err(self.error("expected ':' after a member name"));
            }
            self.skip_space();
            members.push((key, self.value(depth + 1)?));
            self.skip_space();
            if self.eat(b'}') {
                return Ok(Value::Object(members));
            }
            if !self.eat(b',') {
                return Err(self.error("expected ',' or '}'"));
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth + 1)?);
            self.skip_space();
            if self.eat(b']') {
                return Ok(Value::Array(items));
            }
            if !self.eat(b',') {
                return Err(self.error("expected ',' or ']'"));
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.bytes.get(self.pos) {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    out.push(self.escape()?);
                }
                Some(&b) if b < 0x20 => return Err(self.error("control character in a string")),
                Some(_) => {
                    // The position is always on a character boundary here: multi-byte characters are copied whole.
                    let ch = self.src[self.pos..].chars().next().ok_or(self.error("unterminated string"))?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self) -> Result<char, ParseError> {
        let byte = self.bytes.get(self.pos).copied();
        self.pos += 1;
        match byte {
            Some(b'"') => Ok('"'),
            Some(b'\\') => Ok('\\'),
            Some(b'/') => Ok('/'),
            Some(b'b') => Ok('\u{8}'),
            Some(b'f') => Ok('\u{c}'),
            Some(b'n') => Ok('\n'),
            Some(b'r') => Ok('\r'),
            Some(b't') => Ok('\t'),
            Some(b'u') => {
                let high = self.hex4()?;
                if (0xD800..0xDC00).contains(&high) {
                    if !(self.eat(b'\\') && self.eat(b'u')) {
                        return Err(self.error("lone surrogate in a string"));
                    }
                    let low = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return Err(self.error("lone surrogate in a string"));
                    }
                    char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00))
                        .ok_or(self.error("invalid code point"))
                } else {
                    char::from_u32(high).ok_or(self.error("invalid code point"))
                }
            }
            _ => Err(self.error("invalid escape in a string")),
        }
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = self.bytes.get(self.pos).and_then(|&b| (b as char).to_digit(16));
            value = value * 16 + digit.ok_or(self.error("expected four hex digits"))?;
            self.pos += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        self.eat(b'-');
        if self.eat(b'0') {
            // A leading zero ends the integer part.
        } else if !self.digits() {
            return Err(self.error("expected a digit"));
        }
        if self.eat(b'.') && !self.digits() {
            return Err(self.error("expected a digit after '.'"));
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if !self.digits() {
                return Err(self.error("expected a digit in the exponent"));
            }
        }
        // Only ASCII bytes were consumed, so the slice is on character boundaries.
        let text = &self.src[start..self.pos];
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Value::Number(n)),
            _ => Err(ParseError { at: start, message: "number out of range" }),
        }
    }

    /// Consumes one or more ASCII digits. Returns false when there is none.
    fn digits(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos > start
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn objects_arrays_and_scalars_parse_in_order() {
        let v = parse(" { \"a\": [1, -0.5e2, true, null], \"b\": \"x\" } ").unwrap();
        assert_eq!(
            v,
            Value::Object(vec![
                ("a".into(), Value::Array(vec![Value::Number(1.0), Value::Number(-50.0), Value::Bool(true), Value::Null])),
                ("b".into(), Value::Text("x".into())),
            ])
        );
        assert_eq!(parse("[]").unwrap(), Value::Array(vec![]));
        assert_eq!(parse("{}").unwrap(), Value::Object(vec![]));
    }

    #[test]
    fn string_escapes_and_surrogate_pairs_decode() {
        assert_eq!(parse("\"a\\n\\\"b\\\\\"").unwrap(), Value::Text("a\n\"b\\".into()));
        assert_eq!(parse("\"\\u00f0\"").unwrap(), Value::Text("\u{f0}".into()));
        assert_eq!(parse("\"\\ud83d\\ude00\"").unwrap(), Value::Text("\u{1F600}".into()));
        assert_eq!(parse("\"\u{f0}\"").unwrap(), Value::Text("\u{f0}".into()), "non-ASCII is copied whole");
    }

    #[test]
    fn a_leading_zero_ends_the_number_and_the_rest_is_an_error() {
        let err = parse("[01]").unwrap_err();
        assert_eq!(err, ParseError { at: 2, message: "expected ',' or ']'" });
        assert_eq!(parse("01").unwrap_err().message, "text after the value");
    }

    #[test]
    fn malformed_text_reports_the_offset_of_the_problem() {
        assert_eq!(parse("{\"a\" 1}").unwrap_err(), ParseError { at: 5, message: "expected ':' after a member name" });
        assert_eq!(parse("[1, 2").unwrap_err(), ParseError { at: 5, message: "expected ',' or ']'" });
        assert_eq!(parse("\"abc").unwrap_err().message, "unterminated string");
        assert_eq!(parse("\"\\ud83d x\"").unwrap_err().message, "lone surrogate in a string");
        assert_eq!(parse("1e999").unwrap_err().message, "number out of range");
        assert_eq!(parse("").unwrap_err(), ParseError { at: 0, message: "expected a value" });
    }

    #[test]
    fn nesting_beyond_the_limit_is_refused() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_ok());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert_eq!(parse(&deep).unwrap_err().message, "nesting is too deep");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Kind {
        Kind::Object(vec![
            Property::new("title", Kind::Text).described("the dish name"),
            Property::new("minutes", Kind::number_in(1.0, 240.0)),
            Property::new("difficulty", Kind::choice(&["easy", "medium", "hard"])),
            Property::new("tags", Kind::array_of(Kind::Text)).optional(),
        ])
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn obj(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn a_value_that_matches_the_schema_has_no_issues() {
        let v = obj(vec![("title", text("Soup")), ("minutes", Value::Number(25.0)), ("difficulty", text("easy"))]);
        assert!(validate(&recipe(), &v).is_empty());
        let with_tags = obj(vec![
            ("title", text("Soup")),
            ("minutes", Value::Number(25.0)),
            ("difficulty", text("easy")),
            ("tags", Value::Array(vec![text("warm")])),
        ]);
        assert!(validate(&recipe(), &with_tags).is_empty());
    }

    #[test]
    fn a_missing_required_member_is_reported_and_an_optional_one_may_be_null() {
        let v = obj(vec![("minutes", Value::Number(5.0)), ("difficulty", text("hard"))]);
        assert_eq!(validate(&recipe(), &v), vec![Issue { path: "$.title".into(), message: "missing".into() }]);
        let null_tags = obj(vec![
            ("title", text("x")),
            ("minutes", Value::Number(5.0)),
            ("difficulty", text("hard")),
            ("tags", Value::Null),
        ]);
        assert!(validate(&recipe(), &null_tags).is_empty());
        let null_title = obj(vec![("title", Value::Null), ("minutes", Value::Number(5.0)), ("difficulty", text("hard"))]);
        assert_eq!(validate(&recipe(), &null_title), vec![Issue { path: "$.title".into(), message: "is null".into() }]);
    }

    #[test]
    fn wrong_types_are_reported_with_the_path_to_the_member() {
        let v = obj(vec![
            ("title", Value::Number(3.0)),
            ("minutes", Value::Number(5.0)),
            ("difficulty", text("easy")),
            ("tags", Value::Array(vec![text("ok"), Value::Bool(true)])),
        ]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0], Issue { path: "$.title".into(), message: "expected text, found number".into() });
        assert_eq!(issues[1], Issue { path: "$.tags[1]".into(), message: "expected text, found boolean".into() });
    }

    #[test]
    fn a_choice_and_a_numeric_range_are_enforced() {
        let v = obj(vec![("title", text("x")), ("minutes", Value::Number(300.0)), ("difficulty", text("extreme"))]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].message, "300 is outside 1..=240");
        assert_eq!(issues[1].message, "\"extreme\" is not one of \"easy\", \"medium\", \"hard\"");
    }

    #[test]
    fn unknown_members_and_non_finite_numbers_are_refused() {
        let v = obj(vec![
            ("title", text("x")),
            ("minutes", Value::Number(f64::NAN)),
            ("difficulty", text("easy")),
            ("author", text("me")),
        ]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].message, "number is not finite");
        assert_eq!(issues[1], Issue { path: "$.author".into(), message: "not in the schema".into() });
    }

    #[test]
    fn describe_lists_members_types_choices_ranges_and_descriptions() {
        assert_eq!(
            recipe().describe(),
            "object { title: text (the dish name), minutes: number from 1 to 240, \
             difficulty: one of \"easy\", \"medium\", \"hard\", tags?: array of text }"
        );
    }
}
