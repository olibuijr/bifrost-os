//! Pattern rules: a small rule language for classifying bytes by what they contain (std only).
//!
//! A rule names some patterns (text, with `nocase`, `wide`, `ascii` and `fullword`, or hex bytes with `??`
//! wildcards) and a condition over them. A rule set is compiled from source text and scanned over a byte slice;
//! the result lists the rules whose condition held, with their metadata and where each pattern matched.
//!
//! ```text
//! rule sample_dropper : tag1 {
//!     meta:   family = "Sample"  action = "block"
//!     strings:
//!         $a = "curl | sh" nocase
//!         $b = { 7F 45 4C 46 ?? 01 }
//!     condition: $b at 0 and ($a or #a > 2) and filesize < 10000
//! }
//! ```
//!
//! Conditions: `true`, `false`, `$id`, `$id at N`, `#id <cmp> N`, `filesize <cmp> N`, `any|all|N of them`,
//! `any|all|N of ($a, $b, $p*)`, `not`, `and`, `or` and parentheses. Numbers are decimal or `0x` hex with an
//! optional `KB` or `MB` suffix. Jumps inside hex strings, regular expressions and modules are not supported and
//! are refused with a syntax error rather than ignored.
//!
//! The scanner is deliberately plain: a pattern is found by direct comparison, at most [`MAX_HITS`] hits are kept per
//! pattern, and the nesting of a condition is limited to [`MAX_DEPTH`], so a hostile rule file or input cannot
//! exhaust the stack or the memory.

use std::collections::BTreeMap;
use std::fmt;

/// Most hits kept for one pattern.
pub const MAX_HITS: usize = 1000;
/// Deepest nesting of parentheses and `not` in a condition.
pub const MAX_DEPTH: usize = 64;

/// Why rule text could not be compiled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleError {
    Syntax { line: usize, message: String },
    DuplicateRule(String),
    DuplicateString { rule: String, id: String },
    UnknownString { rule: String, id: String },
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleError::Syntax { line, message } => write!(f, "line {line}: {message}"),
            RuleError::DuplicateRule(r) => write!(f, "rule {r} is defined twice"),
            RuleError::DuplicateString { rule, id } => write!(f, "rule {rule}: pattern {id} is defined twice"),
            RuleError::UnknownString { rule, id } => write!(f, "rule {rule}: condition names unknown pattern {id}"),
        }
    }
}

impl std::error::Error for RuleError {}

fn syntax<T>(line: usize, message: impl Into<String>) -> Result<T, RuleError> {
    Err(RuleError::Syntax { line, message: message.into() })
}

// ---- lexer ----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// `$name` (the name without the `$`; empty for a bare `$`).
    StrId(String),
    /// `#name`.
    CountId(String),
    /// `$name*` in a pattern set.
    StrPrefix(String),
    Number(u64),
    Text(Vec<u8>),
    Hex(Vec<Option<u8>>),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Colon,
    Comma,
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
}

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
    line: usize,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'a> Lexer<'a> {
    fn skip_space(&mut self) -> Result<(), RuleError> {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'\n' {
                self.line += 1;
                self.i += 1;
            } else if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b'/' && self.b.get(self.i + 1) == Some(&b'/') {
                while self.i < self.b.len() && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if c == b'/' && self.b.get(self.i + 1) == Some(&b'*') {
                let start = self.line;
                self.i += 2;
                loop {
                    if self.i + 1 >= self.b.len() {
                        return syntax(start, "comment is not closed");
                    }
                    if self.b[self.i] == b'*' && self.b[self.i + 1] == b'/' {
                        self.i += 2;
                        break;
                    }
                    if self.b[self.i] == b'\n' {
                        self.line += 1;
                    }
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn word(&mut self) -> String {
        let s = self.i;
        while self.i < self.b.len() && is_ident(self.b[self.i]) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.i]).into_owned()
    }

    fn lex(mut self) -> Result<Vec<(Tok, usize)>, RuleError> {
        let mut out: Vec<(Tok, usize)> = Vec::new();
        loop {
            self.skip_space()?;
            if self.i >= self.b.len() {
                return Ok(out);
            }
            let line = self.line;
            let c = self.b[self.i];
            let next = self.b.get(self.i + 1).copied();
            let tok = match c {
                b'{' => {
                    // A brace right after `=` opens a hex string; elsewhere it opens a rule body.
                    let after_eq = matches!(out.last(), Some((Tok::Eq, _)));
                    if after_eq {
                        self.i += 1;
                        Tok::Hex(self.hex(line)?)
                    } else {
                        self.i += 1;
                        Tok::LBrace
                    }
                }
                b'}' => {
                    self.i += 1;
                    Tok::RBrace
                }
                b'(' => {
                    self.i += 1;
                    Tok::LParen
                }
                b')' => {
                    self.i += 1;
                    Tok::RParen
                }
                b':' => {
                    self.i += 1;
                    Tok::Colon
                }
                b',' => {
                    self.i += 1;
                    Tok::Comma
                }
                b'=' if next == Some(b'=') => {
                    self.i += 2;
                    Tok::EqEq
                }
                b'=' => {
                    self.i += 1;
                    Tok::Eq
                }
                b'!' if next == Some(b'=') => {
                    self.i += 2;
                    Tok::Ne
                }
                b'<' if next == Some(b'=') => {
                    self.i += 2;
                    Tok::Le
                }
                b'<' => {
                    self.i += 1;
                    Tok::Lt
                }
                b'>' if next == Some(b'=') => {
                    self.i += 2;
                    Tok::Ge
                }
                b'>' => {
                    self.i += 1;
                    Tok::Gt
                }
                b'"' => {
                    self.i += 1;
                    Tok::Text(self.quoted(line)?)
                }
                b'$' => {
                    self.i += 1;
                    let w = self.word();
                    if self.b.get(self.i) == Some(&b'*') {
                        self.i += 1;
                        Tok::StrPrefix(w)
                    } else {
                        Tok::StrId(w)
                    }
                }
                b'#' => {
                    self.i += 1;
                    Tok::CountId(self.word())
                }
                c if c.is_ascii_digit() => Tok::Number(self.number(line)?),
                c if is_ident(c) => Tok::Ident(self.word()),
                other => return syntax(line, format!("unexpected character {:?}", other as char)),
            };
            out.push((tok, line));
        }
    }

    fn number(&mut self, line: usize) -> Result<u64, RuleError> {
        let hex = self.b[self.i] == b'0' && matches!(self.b.get(self.i + 1), Some(b'x') | Some(b'X'));
        if hex {
            self.i += 2;
        }
        let s = self.i;
        while self.i < self.b.len() && (if hex { self.b[self.i].is_ascii_hexdigit() } else { self.b[self.i].is_ascii_digit() }) {
            self.i += 1;
        }
        let digits = String::from_utf8_lossy(&self.b[s..self.i]).into_owned();
        let Ok(mut n) = u64::from_str_radix(&digits, if hex { 16 } else { 10 }) else {
            return syntax(line, "number is missing or too large");
        };
        let rest = &self.b[self.i..];
        let mult: u64 = if rest.starts_with(b"KB") {
            self.i += 2;
            1024
        } else if rest.starts_with(b"MB") {
            self.i += 2;
            1024 * 1024
        } else {
            1
        };
        n = match n.checked_mul(mult) {
            Some(v) => v,
            None => return syntax(line, "number is too large"),
        };
        Ok(n)
    }

    fn quoted(&mut self, line: usize) -> Result<Vec<u8>, RuleError> {
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return syntax(line, "text is not closed") };
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\n' => return syntax(line, "text is not closed"),
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else { return syntax(line, "text is not closed") };
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'x' => {
                            let h = self.b.get(self.i..self.i + 2).and_then(|s| std::str::from_utf8(s).ok());
                            match h.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                                Some(v) => {
                                    out.push(v);
                                    self.i += 2;
                                }
                                None => return syntax(line, "\\x needs two hex digits"),
                            }
                        }
                        _ => return syntax(line, "unknown escape in text"),
                    }
                }
                c => out.push(c),
            }
        }
    }

    /// Reads hex bytes up to the closing brace; the opening brace is already consumed.
    fn hex(&mut self, line: usize) -> Result<Vec<Option<u8>>, RuleError> {
        let mut out = Vec::new();
        loop {
            self.skip_space()?;
            let Some(&c) = self.b.get(self.i) else { return syntax(line, "hex string is not closed") };
            if c == b'}' {
                self.i += 1;
                break;
            }
            if c == b'[' || c == b'(' || c == b'|' || c == b'~' {
                return syntax(self.line, "jumps and alternatives in hex strings are not supported");
            }
            let pair = self.b.get(self.i..self.i + 2);
            match pair {
                Some(b"??") => out.push(None),
                Some(p) if p[0].is_ascii_hexdigit() && p[1].is_ascii_hexdigit() => {
                    let s = std::str::from_utf8(p).unwrap_or("00");
                    out.push(Some(u8::from_str_radix(s, 16).unwrap_or(0)));
                }
                _ => return syntax(self.line, "hex string holds something other than byte pairs and ??"),
            }
            self.i += 2;
        }
        if out.is_empty() {
            return syntax(line, "hex string is empty");
        }
        Ok(out)
    }
}

// ---- model ----------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cmp {
    fn test(self, a: u64, b: u64) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Quant {
    Any,
    All,
    N(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Bool(bool),
    Has(String),
    At(String, u64),
    Count(String, Cmp, u64),
    FileSize(Cmp, u64),
    Of(Quant, Vec<SetItem>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SetItem {
    Id(String),
    Prefix(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Body {
    /// Bytes compared directly; `fold` ignores ASCII case.
    Text { bytes: Vec<u8>, fold: bool },
    Hex(Vec<Option<u8>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Variant {
    body: Body,
    fullword: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pattern {
    id: String,
    variants: Vec<Variant>,
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    name: String,
    private: bool,
    tags: Vec<String>,
    meta: BTreeMap<String, String>,
    patterns: Vec<Pattern>,
    condition: Expr,
}

/// One rule that held for the scanned bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub rule: String,
    pub tags: Vec<String>,
    pub meta: BTreeMap<String, String>,
    /// Pattern id (with `$`) and the offsets where it matched, for the patterns that matched at all.
    pub hits: Vec<(String, Vec<usize>)>,
}

/// A compiled set of rules.
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    rules: Vec<Rule>,
}

// ---- parser ---------------------------------------------------------------------------------------------------

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    rule: String,
}

impl Parser {
    fn line(&self) -> usize {
        self.toks.get(self.pos).or_else(|| self.toks.last()).map(|t| t.1).unwrap_or(1)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.0.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, want: Tok, what: &str) -> Result<(), RuleError> {
        let line = self.line();
        match self.next() {
            Some(t) if t == want => Ok(()),
            _ => syntax(line, format!("expected {what}")),
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, RuleError> {
        let line = self.line();
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            _ => syntax(line, format!("expected {what}")),
        }
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s == w)
    }

    fn rules(&mut self) -> Result<Vec<Rule>, RuleError> {
        let mut out: Vec<Rule> = Vec::new();
        while self.peek().is_some() {
            let r = self.rule()?;
            if out.iter().any(|o| o.name == r.name) {
                return Err(RuleError::DuplicateRule(r.name));
            }
            out.push(r);
        }
        Ok(out)
    }

    fn rule(&mut self) -> Result<Rule, RuleError> {
        let mut private = false;
        if self.is_word("private") {
            self.pos += 1;
            private = true;
        }
        if !self.is_word("rule") {
            return syntax(self.line(), "expected rule");
        }
        self.pos += 1;
        let name = self.ident("a rule name")?;
        self.rule = name.clone();
        let mut tags = Vec::new();
        if self.peek() == Some(&Tok::Colon) {
            self.pos += 1;
            while matches!(self.peek(), Some(Tok::Ident(_))) {
                tags.push(self.ident("a tag")?);
            }
        }
        self.expect(Tok::LBrace, "{")?;
        let mut meta = BTreeMap::new();
        let mut patterns: Vec<Pattern> = Vec::new();
        if self.is_word("meta") {
            self.pos += 1;
            self.expect(Tok::Colon, ":")?;
            while matches!(self.peek(), Some(Tok::Ident(s)) if s != "strings" && s != "condition") {
                let key = self.ident("a metadata key")?;
                self.expect(Tok::Eq, "=")?;
                let line = self.line();
                let value = match self.next() {
                    Some(Tok::Text(t)) => String::from_utf8_lossy(&t).into_owned(),
                    Some(Tok::Number(n)) => n.to_string(),
                    Some(Tok::Ident(s)) if s == "true" || s == "false" => s,
                    _ => return syntax(line, "metadata value must be text, a number, true or false"),
                };
                meta.insert(key, value);
            }
        }
        if self.is_word("strings") {
            self.pos += 1;
            self.expect(Tok::Colon, ":")?;
            while matches!(self.peek(), Some(Tok::StrId(_))) {
                let p = self.pattern()?;
                if patterns.iter().any(|o| o.id == p.id) {
                    return Err(RuleError::DuplicateString { rule: name, id: format!("${}", p.id) });
                }
                patterns.push(p);
            }
        }
        if !self.is_word("condition") {
            return syntax(self.line(), "expected condition");
        }
        self.pos += 1;
        self.expect(Tok::Colon, ":")?;
        let condition = self.expr(0)?;
        self.expect(Tok::RBrace, "}")?;
        self.check_names(&condition, &patterns)?;
        Ok(Rule { name, private, tags, meta, patterns, condition })
    }

    fn pattern(&mut self) -> Result<Pattern, RuleError> {
        let Some(Tok::StrId(id)) = self.next() else { return syntax(self.line(), "expected a pattern name") };
        self.expect(Tok::Eq, "=")?;
        let line = self.line();
        match self.next() {
            Some(Tok::Text(bytes)) => {
                if bytes.is_empty() {
                    return syntax(line, "pattern text is empty");
                }
                let (mut nocase, mut wide, mut ascii, mut fullword) = (false, false, false, false);
                while let Some(Tok::Ident(m)) = self.peek().cloned() {
                    match m.as_str() {
                        "nocase" => nocase = true,
                        "wide" => wide = true,
                        "ascii" => ascii = true,
                        "fullword" => fullword = true,
                        "condition" | "strings" => break,
                        _ => return syntax(self.line(), format!("unknown modifier {m}")),
                    }
                    self.pos += 1;
                }
                let mut variants = Vec::new();
                if ascii || !wide {
                    variants.push(Variant { body: Body::Text { bytes: bytes.clone(), fold: nocase }, fullword });
                }
                if wide {
                    let mut w = Vec::with_capacity(bytes.len() * 2);
                    for b in &bytes {
                        w.push(*b);
                        w.push(0);
                    }
                    variants.push(Variant { body: Body::Text { bytes: w, fold: nocase }, fullword });
                }
                Ok(Pattern { id, variants })
            }
            Some(Tok::Hex(h)) => Ok(Pattern { id, variants: vec![Variant { body: Body::Hex(h), fullword: false }] }),
            _ => syntax(line, "a pattern is text or a hex string"),
        }
    }

    fn expr(&mut self, depth: usize) -> Result<Expr, RuleError> {
        if depth > MAX_DEPTH {
            return syntax(self.line(), "condition is nested too deeply");
        }
        let mut left = self.and(depth)?;
        while self.is_word("or") {
            self.pos += 1;
            let right = self.and(depth)?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self, depth: usize) -> Result<Expr, RuleError> {
        let mut left = self.unary(depth)?;
        while self.is_word("and") {
            self.pos += 1;
            let right = self.unary(depth)?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self, depth: usize) -> Result<Expr, RuleError> {
        if self.is_word("not") {
            if depth + 1 > MAX_DEPTH {
                return syntax(self.line(), "condition is nested too deeply");
            }
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.unary(depth + 1)?)));
        }
        self.primary(depth)
    }

    fn cmp(&mut self) -> Result<Cmp, RuleError> {
        let line = self.line();
        match self.next() {
            Some(Tok::Lt) => Ok(Cmp::Lt),
            Some(Tok::Le) => Ok(Cmp::Le),
            Some(Tok::Gt) => Ok(Cmp::Gt),
            Some(Tok::Ge) => Ok(Cmp::Ge),
            Some(Tok::EqEq) => Ok(Cmp::Eq),
            Some(Tok::Ne) => Ok(Cmp::Ne),
            _ => syntax(line, "expected a comparison"),
        }
    }

    fn number(&mut self) -> Result<u64, RuleError> {
        let line = self.line();
        match self.next() {
            Some(Tok::Number(n)) => Ok(n),
            _ => syntax(line, "expected a number"),
        }
    }

    fn primary(&mut self, depth: usize) -> Result<Expr, RuleError> {
        let line = self.line();
        match self.next() {
            Some(Tok::LParen) => {
                let e = self.expr(depth + 1)?;
                self.expect(Tok::RParen, ")")?;
                Ok(e)
            }
            Some(Tok::StrId(id)) => {
                if self.is_word("at") {
                    self.pos += 1;
                    Ok(Expr::At(id, self.number()?))
                } else {
                    Ok(Expr::Has(id))
                }
            }
            Some(Tok::CountId(id)) => {
                let c = self.cmp()?;
                Ok(Expr::Count(id, c, self.number()?))
            }
            Some(Tok::Ident(w)) => match w.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "filesize" => {
                    let c = self.cmp()?;
                    Ok(Expr::FileSize(c, self.number()?))
                }
                "any" | "all" => {
                    let q = if w == "any" { Quant::Any } else { Quant::All };
                    self.of(q)
                }
                _ => syntax(line, format!("unexpected word {w} in condition")),
            },
            Some(Tok::Number(n)) => {
                if !self.is_word("of") {
                    return syntax(line, "a number in a condition must be followed by of");
                }
                self.of(Quant::N(n))
            }
            _ => syntax(line, "expected a condition"),
        }
    }

    fn of(&mut self, q: Quant) -> Result<Expr, RuleError> {
        if !self.is_word("of") {
            return syntax(self.line(), "expected of");
        }
        self.pos += 1;
        if self.is_word("them") {
            self.pos += 1;
            return Ok(Expr::Of(q, Vec::new()));
        }
        self.expect(Tok::LParen, "( or them")?;
        let mut items = Vec::new();
        loop {
            let line = self.line();
            match self.next() {
                Some(Tok::StrId(id)) => items.push(SetItem::Id(id)),
                Some(Tok::StrPrefix(p)) => items.push(SetItem::Prefix(p)),
                _ => return syntax(line, "expected a pattern name"),
            }
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RParen) => break,
                _ => return syntax(line, "expected , or )"),
            }
        }
        Ok(Expr::Of(q, items))
    }

    fn check_names(&self, e: &Expr, patterns: &[Pattern]) -> Result<(), RuleError> {
        let known = |id: &str| patterns.iter().any(|p| p.id == id);
        let bad = |id: &str| RuleError::UnknownString { rule: self.rule.clone(), id: format!("${id}") };
        match e {
            Expr::Has(id) | Expr::At(id, _) | Expr::Count(id, _, _) => {
                if known(id) {
                    Ok(())
                } else {
                    Err(bad(id))
                }
            }
            Expr::Of(_, items) => {
                for it in items {
                    match it {
                        SetItem::Id(id) if !known(id) => return Err(bad(id)),
                        SetItem::Prefix(p) if !patterns.iter().any(|x| x.id.starts_with(p.as_str())) => return Err(bad(p)),
                        _ => {}
                    }
                }
                Ok(())
            }
            Expr::Not(a) => self.check_names(a, patterns),
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.check_names(a, patterns)?;
                self.check_names(b, patterns)
            }
            Expr::Bool(_) | Expr::FileSize(_, _) => Ok(()),
        }
    }
}

// ---- matching -------------------------------------------------------------------------------------------------

fn byte_eq(a: u8, b: u8, fold: bool) -> bool {
    if fold {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

fn word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn find_variant(data: &[u8], v: &Variant, hits: &mut Vec<usize>) {
    let n = match &v.body {
        Body::Text { bytes, .. } => bytes.len(),
        Body::Hex(h) => h.len(),
    };
    if n == 0 || n > data.len() {
        return;
    }
    for start in 0..=(data.len() - n) {
        if hits.len() >= MAX_HITS {
            return;
        }
        let window = &data[start..start + n];
        let ok = match &v.body {
            Body::Text { bytes, fold } => window.iter().zip(bytes).all(|(a, b)| byte_eq(*a, *b, *fold)),
            Body::Hex(h) => window.iter().zip(h).all(|(a, b)| b.map_or(true, |b| *a == b)),
        };
        if !ok {
            continue;
        }
        if v.fullword {
            let before = start > 0 && word_char(data[start - 1]);
            let after = start + n < data.len() && word_char(data[start + n]);
            if before || after {
                continue;
            }
        }
        hits.push(start);
    }
}

fn find_pattern(data: &[u8], p: &Pattern) -> Vec<usize> {
    let mut hits = Vec::new();
    for v in &p.variants {
        find_variant(data, v, &mut hits);
    }
    hits.sort_unstable();
    hits.dedup();
    hits.truncate(MAX_HITS);
    hits
}

struct Eval<'a> {
    size: u64,
    hits: &'a BTreeMap<String, Vec<usize>>,
}

impl Eval<'_> {
    fn hit_count(&self, id: &str) -> u64 {
        self.hits.get(id).map_or(0, |h| h.len() as u64)
    }

    fn eval(&self, e: &Expr) -> bool {
        match e {
            Expr::Bool(b) => *b,
            Expr::Has(id) => self.hit_count(id) > 0,
            Expr::At(id, off) => self.hits.get(id).map_or(false, |h| h.contains(&(*off as usize))),
            Expr::Count(id, c, n) => c.test(self.hit_count(id), *n),
            Expr::FileSize(c, n) => c.test(self.size, *n),
            Expr::Not(a) => !self.eval(a),
            Expr::And(a, b) => self.eval(a) && self.eval(b),
            Expr::Or(a, b) => self.eval(a) || self.eval(b),
            Expr::Of(q, items) => {
                let ids: Vec<&String> = if items.is_empty() {
                    self.hits.keys().collect()
                } else {
                    self.hits
                        .keys()
                        .filter(|k| {
                            items.iter().any(|it| match it {
                                SetItem::Id(id) => id == *k,
                                SetItem::Prefix(p) => k.starts_with(p.as_str()),
                            })
                        })
                        .collect()
                };
                let total = ids.len() as u64;
                let matched = ids.iter().filter(|k| self.hit_count(k) > 0).count() as u64;
                match q {
                    Quant::Any => matched >= 1,
                    Quant::All => total > 0 && matched == total,
                    Quant::N(n) => matched >= *n,
                }
            }
        }
    }
}

impl RuleSet {
    /// Compiles rule text. Fails on the first error and names its line or rule.
    pub fn compile(source: &str) -> Result<RuleSet, RuleError> {
        let toks = Lexer { b: source.as_bytes(), i: 0, line: 1 }.lex()?;
        let mut p = Parser { toks, pos: 0, rule: String::new() };
        Ok(RuleSet { rules: p.rules()? })
    }

    /// How many rules the set holds.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The names of the rules, in source order.
    pub fn names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }

    /// Scans `data` and returns the rules that held, in source order. Private rules are evaluated but not reported.
    pub fn scan(&self, data: &[u8]) -> Vec<Match> {
        let mut out = Vec::new();
        for r in &self.rules {
            if r.private {
                continue;
            }
            let hits: BTreeMap<String, Vec<usize>> = r.patterns.iter().map(|p| (p.id.clone(), find_pattern(data, p))).collect();
            let ev = Eval { size: data.len() as u64, hits: &hits };
            if ev.eval(&r.condition) {
                out.push(Match {
                    rule: r.name.clone(),
                    tags: r.tags.clone(),
                    meta: r.meta.clone(),
                    hits: hits.into_iter().filter(|(_, h)| !h.is_empty()).map(|(id, h)| (format!("${id}"), h)).collect(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(m: &[Match]) -> Vec<&str> {
        m.iter().map(|m| m.rule.as_str()).collect()
    }

    #[test]
    fn text_pattern_matches_and_reports_offsets() {
        let rs = RuleSet::compile(r#"rule r { strings: $a = "abc" condition: $a }"#).unwrap();
        let m = rs.scan(b"xxabcxabc");
        assert_eq!(names(&m), ["r"]);
        assert_eq!(m[0].hits, vec![("$a".to_string(), vec![2, 6])]);
        assert!(rs.scan(b"nothing here").is_empty());
    }

    #[test]
    fn nocase_wide_and_fullword_modifiers() {
        let rs = RuleSet::compile(
            r#"rule n { strings: $a = "Hello" nocase condition: $a }
               rule w { strings: $a = "hi" wide condition: $a }
               rule f { strings: $a = "cat" fullword condition: $a }"#,
        )
        .unwrap();
        assert_eq!(names(&rs.scan(b"say HELLO")), ["n"]);
        assert_eq!(names(&rs.scan(b"h\0i\0")), ["w"]);
        assert!(rs.scan(b"hi").is_empty());
        assert_eq!(names(&rs.scan(b"a cat sat")), ["f"]);
        assert!(rs.scan(b"concatenate").is_empty());
    }

    #[test]
    fn hex_patterns_support_wildcards() {
        let rs = RuleSet::compile("rule h { strings: $m = { 7F 45 ?? 46 } condition: $m at 0 }").unwrap();
        assert_eq!(names(&rs.scan(&[0x7F, 0x45, 0x99, 0x46, 0])), ["h"]);
        assert!(rs.scan(&[0, 0x7F, 0x45, 0x99, 0x46]).is_empty());
    }

    #[test]
    fn conditions_combine_counts_sizes_and_sets() {
        let src = r#"
            rule c {
                meta: family = "Demo" level = 3
                strings: $a = "aa" $b = "bb" $p1 = "x1" $p2 = "x2"
                condition: #a >= 2 and filesize < 1KB and not $b and 1 of ($p*)
            }
            rule all3 { strings: $a = "aa" $b = "bb" condition: all of them }
            rule two { strings: $a = "aa" $b = "bb" $c = "cc" condition: 2 of them }
        "#;
        let rs = RuleSet::compile(src).unwrap();
        let m = rs.scan(b"aa aa x2");
        assert_eq!(names(&m), ["c"]);
        assert_eq!(m[0].meta["family"], "Demo");
        assert_eq!(m[0].meta["level"], "3");
        assert_eq!(names(&rs.scan(b"aa bb")), ["all3", "two"]);
        assert!(rs.scan(b"aa aa x2 bb").iter().all(|m| m.rule != "c"));
    }

    #[test]
    fn private_rules_are_not_reported_and_tags_are_kept() {
        let rs = RuleSet::compile("private rule hidden { condition: true } rule shown : bad worm { condition: true }").unwrap();
        let m = rs.scan(b"");
        assert_eq!(names(&m), ["shown"]);
        assert_eq!(m[0].tags, ["bad", "worm"]);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs.names(), ["hidden", "shown"]);
    }

    #[test]
    fn comments_escapes_and_hex_numbers_are_read() {
        let src = "// head\nrule e { /* mid */ strings: $a = \"a\\x41\\n\" condition: $a and filesize > 0x2 }";
        let rs = RuleSet::compile(src).unwrap();
        assert_eq!(names(&rs.scan(b"aA\n")), ["e"]);
        assert!(rs.scan(b"aA").is_empty());
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(matches!(RuleSet::compile("rule a { condition: true } rule a { condition: true }"), Err(RuleError::DuplicateRule(_))));
        assert!(matches!(
            RuleSet::compile(r#"rule a { strings: $x = "1" $x = "2" condition: $x }"#),
            Err(RuleError::DuplicateString { .. })
        ));
        assert!(matches!(RuleSet::compile("rule a { condition: $nope }"), Err(RuleError::UnknownString { .. })));
        match RuleSet::compile("rule a {\n condition: ) }") {
            Err(RuleError::Syntax { line, .. }) => assert_eq!(line, 2),
            other => panic!("{other:?}"),
        }
        assert!(RuleSet::compile("rule a { strings: $h = { 41 [2-4] 42 } condition: $h }").is_err());
        assert!(RuleSet::compile(r#"rule a { strings: $h = "x" regex condition: $h }"#).is_err());
        assert!(RuleSet::compile("rule a { condition: true").is_err());
    }

    #[test]
    fn deep_nesting_is_refused_instead_of_overflowing() {
        let src = format!("rule a {{ condition: {}true{} }}", "(".repeat(5000), ")".repeat(5000));
        assert!(RuleSet::compile(&src).is_err());
        let nots = format!("rule a {{ condition: {}true }}", "not ".repeat(5000));
        assert!(RuleSet::compile(&nots).is_err());
    }

    #[test]
    fn hits_per_pattern_are_capped() {
        let rs = RuleSet::compile(r#"rule r { strings: $a = "a" condition: #a >= 1000 }"#).unwrap();
        let m = rs.scan(&vec![b'a'; 5000]);
        assert_eq!(m[0].hits[0].1.len(), MAX_HITS);
    }
}
