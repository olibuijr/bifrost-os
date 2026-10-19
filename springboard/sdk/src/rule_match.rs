//! The matching half of web content blockers (`ContentRuleList`): the URL filter pattern language, the rule model,
//! the verdict for one request, and the line format the engine reads.
//!
//! This file has no dependencies on purpose. The SDK compiles it as `rule_match` (see `content_rule_lists.rs` for
//! the JSON rule lists and the store) and the web engine process (`apps/browser/engine`) includes the same source
//! with `#[path]`, so the app and the engine cannot disagree about what a rule matches.
//!
//! ## Pattern language
//!
//! A `url-filter` is a small regular expression: literal characters, `.`, the classes `\d` `\w` `\s`, bracket
//! sets (`[a-z0-9]`, `[^/]`), groups `( )`, the quantifiers `*` `+` `?` `{n}` `{n,}` `{n,m}`, a leading `^` and a
//! trailing `$`. Alternation (`|`), lazy quantifiers, back references and look-around are refused when the list is
//! compiled, as they are in the reference. A pattern is searched anywhere in the URL unless it starts with `^`.
//! URLs are matched on their first [`MAX_URL_CHARS`] characters and a match attempt is cut after [`STEP_BUDGET`]
//! steps (no match), so a hostile pattern cannot stall the engine.

use std::cell::Cell;

/// Longest URL prefix a filter is run against.
pub const MAX_URL_CHARS: usize = 2048;
/// Matching steps one filter may take against one URL.
pub const STEP_BUDGET: u32 = 200_000;

#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Char(char),
    Any,
    Class { ranges: Vec<(char, char)>, negate: bool },
    Group(Vec<Piece>),
}

#[derive(Clone, Debug, PartialEq)]
struct Piece {
    atom: Atom,
    min: usize,
    max: usize,
}

/// A compiled `url-filter`.
#[derive(Clone, Debug, PartialEq)]
pub struct UrlFilter {
    source: String,
    pieces: Vec<Piece>,
    anchor_start: bool,
    anchor_end: bool,
    case_sensitive: bool,
}

fn class_escape(c: char) -> Option<(Vec<(char, char)>, bool)> {
    match c {
        'd' => Some((vec![('0', '9')], false)),
        'D' => Some((vec![('0', '9')], true)),
        'w' => Some((vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], false)),
        'W' => Some((vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], true)),
        's' => Some((vec![(' ', ' '), ('\t', '\r')], false)),
        'S' => Some((vec![(' ', ' '), ('\t', '\r')], true)),
        _ => None,
    }
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn sequence(&mut self, in_group: bool) -> Result<Vec<Piece>, String> {
        let mut out: Vec<Piece> = Vec::new();
        while let Some(c) = self.peek() {
            let atom = match c {
                ')' if in_group => return Ok(out),
                ')' => return Err("unmatched )".into()),
                '(' => {
                    self.pos += 1;
                    let inner = self.sequence(true)?;
                    if self.peek() != Some(')') {
                        return Err("unclosed group".into());
                    }
                    self.pos += 1;
                    Atom::Group(inner)
                }
                '|' => return Err("alternation is not supported".into()),
                '^' => return Err("^ is only allowed at the start".into()),
                '*' | '+' | '?' | '{' => return Err(format!("nothing to repeat before {c}")),
                '.' => {
                    self.pos += 1;
                    Atom::Any
                }
                '[' => self.bracket()?,
                '\\' => {
                    self.pos += 1;
                    let e = self.peek().ok_or("a pattern cannot end with a backslash")?;
                    self.pos += 1;
                    match class_escape(e) {
                        Some((ranges, negate)) => Atom::Class { ranges, negate },
                        None if e.is_ascii_digit() => return Err("back references are not supported".into()),
                        None if matches!(e, 'b' | 'B') => return Err("word boundaries are not supported".into()),
                        None => Atom::Char(e),
                    }
                }
                c => {
                    self.pos += 1;
                    Atom::Char(c)
                }
            };
            let (min, max) = self.quantifier()?;
            out.push(Piece { atom, min, max });
        }
        if in_group {
            return Err("unclosed group".into());
        }
        Ok(out)
    }

    fn quantifier(&mut self) -> Result<(usize, usize), String> {
        let q = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                let close = self.chars[self.pos..].iter().position(|c| *c == '}').ok_or("unclosed {")?;
                let body: String = self.chars[self.pos + 1..self.pos + close].iter().collect();
                let num = |s: &str| s.trim().parse::<usize>().map_err(|_| format!("bad repeat count {{{body}}}"));
                let (min, max) = match body.split_once(',') {
                    None => {
                        let n = num(&body)?;
                        (n, n)
                    }
                    Some((a, "")) => (num(a)?, usize::MAX),
                    Some((a, b)) => (num(a)?, num(b)?),
                };
                if min > max || max != usize::MAX && max > 1000 || min > 1000 {
                    return Err(format!("repeat count {{{body}}} is out of range"));
                }
                self.pos += close;
                (min, max)
            }
            _ => return Ok((1, 1)),
        };
        self.pos += 1;
        if matches!(self.peek(), Some('?' | '+')) {
            return Err("lazy and possessive quantifiers are not supported".into());
        }
        Ok(q)
    }

    fn bracket(&mut self) -> Result<Atom, String> {
        self.pos += 1;
        let negate = self.peek() == Some('^');
        if negate {
            self.pos += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or("unclosed [")?;
            self.pos += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.peek().ok_or("unclosed [")?;
                self.pos += 1;
                if let Some((r, false)) = class_escape(e) {
                    ranges.extend(r);
                    continue;
                }
                e
            } else {
                c
            };
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|n| *n != ']') {
                let hi = self.chars[self.pos + 1];
                self.pos += 2;
                if hi < lo {
                    return Err("a range in [] runs backwards".into());
                }
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
        Ok(Atom::Class { ranges, negate })
    }
}

struct Run<'a> {
    text: &'a [char],
    fold: bool,
    steps: Cell<u32>,
}

impl Run<'_> {
    fn tick(&self) -> bool {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        n <= STEP_BUDGET
    }

    fn eq(&self, a: char, b: char) -> bool {
        a == b || self.fold && a.to_ascii_lowercase() == b.to_ascii_lowercase()
    }

    fn in_class(&self, c: char, ranges: &[(char, char)], negate: bool) -> bool {
        let hit = |c: char| ranges.iter().any(|(lo, hi)| *lo <= c && c <= *hi);
        let found = hit(c) || self.fold && (hit(c.to_ascii_lowercase()) || hit(c.to_ascii_uppercase()));
        found != negate
    }

    fn atom(&self, atom: &Atom, pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        if !self.tick() {
            return false;
        }
        match atom {
            Atom::Char(c) => pos < self.text.len() && self.eq(self.text[pos], *c) && k(pos + 1),
            Atom::Any => pos < self.text.len() && k(pos + 1),
            Atom::Class { ranges, negate } => pos < self.text.len() && self.in_class(self.text[pos], ranges, *negate) && k(pos + 1),
            Atom::Group(inner) => self.seq(inner, pos, k),
        }
    }

    fn piece(&self, piece: &Piece, count: usize, pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        if count < piece.max {
            let more = self.atom(&piece.atom, pos, &mut |p| {
                if p == pos && count >= piece.min {
                    return false; // an empty repeat cannot make progress
                }
                self.piece(piece, count + 1, p, k)
            });
            if more {
                return true;
            }
        }
        count >= piece.min && k(pos)
    }

    fn seq(&self, pieces: &[Piece], pos: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        match pieces.split_first() {
            None => k(pos),
            Some((first, rest)) => self.piece(first, 0, pos, &mut |p| self.seq(rest, p, k)),
        }
    }
}

impl UrlFilter {
    /// Compiles `pattern`; `case_sensitive` is the rule's `url-filter-is-case-sensitive`.
    pub fn compile(pattern: &str, case_sensitive: bool) -> Result<UrlFilter, String> {
        if pattern.is_empty() {
            return Err("the url-filter is empty".into());
        }
        let mut chars: Vec<char> = pattern.chars().collect();
        let anchor_start = chars.first() == Some(&'^');
        if anchor_start {
            chars.remove(0);
        }
        // a trailing $ that is not escaped
        let mut backslashes = 0;
        for c in chars.iter().rev().skip(1) {
            if *c == '\\' {
                backslashes += 1;
            } else {
                break;
            }
        }
        let anchor_end = chars.last() == Some(&'$') && backslashes % 2 == 0;
        if anchor_end {
            chars.pop();
        }
        let mut parser = Parser { chars: &chars, pos: 0 };
        let pieces = parser.sequence(false)?;
        Ok(UrlFilter { source: pattern.to_string(), pieces, anchor_start, anchor_end, case_sensitive })
    }

    /// The pattern text as written.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// True when the filter matches `url`.
    pub fn matches(&self, url: &str) -> bool {
        let text: Vec<char> = url.chars().take(MAX_URL_CHARS).collect();
        let run = Run { text: &text, fold: !self.case_sensitive, steps: Cell::new(0) };
        let end = self.anchor_end;
        let len = text.len();
        let starts: Box<dyn Iterator<Item = usize>> = if self.anchor_start { Box::new(0..1) } else { Box::new(0..=len) };
        for start in starts {
            if run.seq(&self.pieces, start, &mut |p| !end || p == len) {
                return true;
            }
            if run.steps.get() > STEP_BUDGET {
                return false;
            }
        }
        false
    }
}

/// The kinds of resource a trigger can name (`resource-type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Image,
    StyleSheet,
    Script,
    Font,
    Raw,
    SvgDocument,
    Media,
    Popup,
    Ping,
    Fetch,
    Websocket,
    Other,
}

impl ResourceType {
    pub const ALL: [ResourceType; 13] = [
        ResourceType::Document,
        ResourceType::Image,
        ResourceType::StyleSheet,
        ResourceType::Script,
        ResourceType::Font,
        ResourceType::Raw,
        ResourceType::SvgDocument,
        ResourceType::Media,
        ResourceType::Popup,
        ResourceType::Ping,
        ResourceType::Fetch,
        ResourceType::Websocket,
        ResourceType::Other,
    ];

    /// The name a rule list uses.
    pub fn name(self) -> &'static str {
        match self {
            ResourceType::Document => "document",
            ResourceType::Image => "image",
            ResourceType::StyleSheet => "style-sheet",
            ResourceType::Script => "script",
            ResourceType::Font => "font",
            ResourceType::Raw => "raw",
            ResourceType::SvgDocument => "svg-document",
            ResourceType::Media => "media",
            ResourceType::Popup => "popup",
            ResourceType::Ping => "ping",
            ResourceType::Fetch => "fetch",
            ResourceType::Websocket => "websocket",
            ResourceType::Other => "other",
        }
    }

    pub fn from_name(name: &str) -> Option<ResourceType> {
        Self::ALL.iter().copied().find(|t| t.name() == name)
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// The type of a request from the engine's request destination (its `Debug` name: `Image`, `Script`, `Style`,
    /// `Font`, `Document`, `None` for fetch and XHR, and so on).
    pub fn from_destination(destination: &str) -> ResourceType {
        match destination {
            "Document" | "Iframe" | "Frame" | "Embed" | "Object" => ResourceType::Document,
            "Image" => ResourceType::Image,
            "Style" => ResourceType::StyleSheet,
            "Script" | "SharedWorker" | "Worker" | "ServiceWorker" | "AudioWorklet" | "PaintWorklet" => ResourceType::Script,
            "Font" => ResourceType::Font,
            "Audio" | "Video" | "Track" => ResourceType::Media,
            "None" => ResourceType::Raw,
            _ => ResourceType::Other,
        }
    }
}

/// `load-type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadType {
    FirstParty,
    ThirdParty,
}

/// What a rule does (`action.type`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Block,
    BlockCookies,
    /// `css-display-none` with its `selector`.
    CssDisplayNone(String),
    IgnorePreviousRules,
    MakeHttps,
    /// `notify`, with its `notification` text.
    Notify(String),
}

/// When a rule applies (`trigger`).
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub filter: UrlFilter,
    /// Lowercase domains; each also covers its subdomains. Empty means no restriction.
    pub if_domain: Vec<String>,
    pub unless_domain: Vec<String>,
    /// None means every type.
    pub resource_types: Option<Vec<ResourceType>>,
    pub load_type: Option<LoadType>,
    pub if_top_url: Vec<UrlFilter>,
    pub unless_top_url: Vec<UrlFilter>,
}

impl Trigger {
    pub fn new(filter: UrlFilter) -> Trigger {
        Trigger { filter, if_domain: Vec::new(), unless_domain: Vec::new(), resource_types: None, load_type: None, if_top_url: Vec::new(), unless_top_url: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub trigger: Trigger,
    pub action: Action,
}

/// One request to judge.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub url: &'a str,
    pub resource: ResourceType,
    /// The page the request belongs to. None for the top-level navigation itself.
    pub top_url: Option<&'a str>,
}

/// What the rules say about a request, after `ignore-previous-rules` has cancelled what came before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    pub block: bool,
    pub block_cookies: bool,
    pub make_https: bool,
    pub css_selectors: Vec<String>,
    pub notifications: Vec<String>,
}

impl Verdict {
    /// True when no rule asked for anything.
    pub fn is_empty(&self) -> bool {
        *self == Verdict::default()
    }
}

/// The host of a URL, lowercase, without credentials or port. Empty when there is none.
pub fn host_of(url: &str) -> String {
    let rest = match url.split_once("://") {
        Some((_, r)) => r,
        None => return String::new(),
    };
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit('@').next().unwrap_or("");
    let host = if let Some(stripped) = auth.strip_prefix('[') { stripped.split(']').next().unwrap_or("") } else { auth.split(':').next().unwrap_or("") };
    host.to_ascii_lowercase()
}

fn domain_covers(domain: &str, host: &str) -> bool {
    let d = domain.trim_start_matches('*').trim_start_matches('.');
    !d.is_empty() && (host == d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
}

/// Whether two hosts belong to the same site. Without a public suffix list this compares the last two labels
/// (three when the second to last is a short generic label such as `co` in `bbc.co.uk`).
pub fn same_site(a: &str, b: &str) -> bool {
    fn site(h: &str) -> String {
        let labels: Vec<&str> = h.split('.').collect();
        let n = labels.len();
        if n <= 2 || h.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':') {
            return h.to_string();
        }
        let generic = matches!(labels[n - 2], "co" | "com" | "org" | "net" | "gov" | "ac" | "edu") && labels[n - 1].len() == 2;
        let keep = if generic { 3 } else { 2 };
        labels[n.saturating_sub(keep)..].join(".")
    }
    site(a) == site(b)
}

impl Trigger {
    fn matches(&self, req: &Request) -> bool {
        if !self.filter.matches(req.url) {
            return false;
        }
        if let Some(types) = &self.resource_types {
            let mask = types.iter().fold(0u16, |m, t| m | t.bit());
            if mask & req.resource.bit() == 0 {
                return false;
            }
        }
        // domains are those of the page the request belongs to (the top-level navigation names itself)
        let page = req.top_url.unwrap_or(req.url);
        let page_host = host_of(page);
        if !self.if_domain.is_empty() && !self.if_domain.iter().any(|d| domain_covers(d, &page_host)) {
            return false;
        }
        if self.unless_domain.iter().any(|d| domain_covers(d, &page_host)) {
            return false;
        }
        if let Some(load) = self.load_type {
            let third = req.top_url.is_some_and(|t| !same_site(&host_of(t), &host_of(req.url)));
            if third != (load == LoadType::ThirdParty) {
                return false;
            }
        }
        if !self.if_top_url.is_empty() && !self.if_top_url.iter().any(|f| f.matches(page)) {
            return false;
        }
        if self.unless_top_url.iter().any(|f| f.matches(page)) {
            return false;
        }
        true
    }
}

/// An ordered list of rules (the compiled form of one `ContentRuleList`, or several joined).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> RuleSet {
        RuleSet { rules }
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Rules in order. A matching `ignore-previous-rules` forgets everything earlier rules asked for.
    pub fn evaluate(&self, req: &Request) -> Verdict {
        let mut v = Verdict::default();
        for rule in &self.rules {
            if !rule.trigger.matches(req) {
                continue;
            }
            match &rule.action {
                Action::Block => v.block = true,
                Action::BlockCookies => v.block_cookies = true,
                Action::MakeHttps => v.make_https = true,
                Action::CssDisplayNone(sel) => v.css_selectors.push(sel.clone()),
                Action::Notify(text) => v.notifications.push(text.clone()),
                Action::IgnorePreviousRules => v = Verdict::default(),
            }
        }
        v
    }
}

/// `make-https`: the URL with its scheme upgraded. A default port 80 is dropped, `http://localhost` and other
/// addresses with no dot are left alone. None when the URL is not plain `http` or must not be upgraded.
pub fn upgrade_to_https(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let host = host_of(url);
    if host.is_empty() || !host.contains('.') && !host.contains(':') || host == "localhost" {
        return None;
    }
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (auth, tail) = rest.split_at(auth_end);
    let auth = auth.strip_suffix(":80").unwrap_or(auth);
    Some(format!("https://{auth}{tail}"))
}

// ---- the line format the engine reads -------------------------------------------------------------------------
//
// One rule per line, nine fields separated by a tab:
//   ACTION  CASE  FILTER  IF_DOMAIN  UNLESS_DOMAIN  TYPES  LOAD  IF_TOP  UNLESS_TOP
// ACTION is block, cookies, hide:SELECTOR, ignore, https or notify:TEXT. CASE is 1 for a case-sensitive filter.
// IF_DOMAIN, UNLESS_DOMAIN and TYPES are comma separated; LOAD is first, third or empty; IF_TOP and UNLESS_TOP are
// filters joined by \u{2}, each written as CASE-digit then the pattern. Tabs and newlines inside a field are
// replaced by spaces when the line is written.

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

fn top_filters(list: &[UrlFilter]) -> String {
    list.iter().map(|f| format!("{}{}", if f.case_sensitive { 1 } else { 0 }, clean(&f.source))).collect::<Vec<_>>().join("\u{2}")
}

impl Rule {
    /// The engine line for this rule (no newline).
    pub fn to_line(&self) -> String {
        let action = match &self.action {
            Action::Block => "block".to_string(),
            Action::BlockCookies => "cookies".to_string(),
            Action::CssDisplayNone(s) => format!("hide:{}", clean(s)),
            Action::IgnorePreviousRules => "ignore".to_string(),
            Action::MakeHttps => "https".to_string(),
            Action::Notify(t) => format!("notify:{}", clean(t)),
        };
        let t = &self.trigger;
        let types = t.resource_types.as_ref().map(|v| v.iter().map(|r| r.name()).collect::<Vec<_>>().join(",")).unwrap_or_default();
        let load = match t.load_type {
            Some(LoadType::FirstParty) => "first",
            Some(LoadType::ThirdParty) => "third",
            None => "",
        };
        [
            action,
            (if t.filter.case_sensitive { "1" } else { "0" }).to_string(),
            clean(&t.filter.source),
            t.if_domain.join(","),
            t.unless_domain.join(","),
            types,
            load.to_string(),
            top_filters(&t.if_top_url),
            top_filters(&t.unless_top_url),
        ]
        .join("\t")
    }

    /// Reads a line written by `to_line`. None when it is malformed (the engine skips such lines).
    pub fn from_line(line: &str) -> Option<Rule> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 9 {
            return None;
        }
        let action = match f[0] {
            "block" => Action::Block,
            "cookies" => Action::BlockCookies,
            "ignore" => Action::IgnorePreviousRules,
            "https" => Action::MakeHttps,
            a => match a.split_once(':')? {
                ("hide", s) if !s.trim().is_empty() => Action::CssDisplayNone(s.to_string()),
                ("notify", t) => Action::Notify(t.to_string()),
                _ => return None,
            },
        };
        let filter = UrlFilter::compile(f[2], f[1] == "1").ok()?;
        let list = |s: &str| -> Vec<String> { s.split(',').filter(|d| !d.is_empty()).map(|d| d.to_string()).collect() };
        let resource_types = if f[5].is_empty() { None } else { Some(f[5].split(',').filter_map(ResourceType::from_name).collect()) };
        let load_type = match f[6] {
            "first" => Some(LoadType::FirstParty),
            "third" => Some(LoadType::ThirdParty),
            _ => None,
        };
        let tops = |s: &str| -> Option<Vec<UrlFilter>> {
            s.split('\u{2}').filter(|p| !p.is_empty()).map(|p| UrlFilter::compile(p.get(1..)?, p.starts_with('1')).ok()).collect()
        };
        Some(Rule {
            trigger: Trigger { filter, if_domain: list(f[3]), unless_domain: list(f[4]), resource_types, load_type, if_top_url: tops(f[7])?, unless_top_url: tops(f[8])? },
            action,
        })
    }
}

impl RuleSet {
    /// The engine text: one line per rule.
    pub fn to_wire(&self) -> String {
        self.rules.iter().map(Rule::to_line).collect::<Vec<_>>().join("\n")
    }

    /// Reads `to_wire` text; malformed lines are skipped.
    pub fn from_wire(text: &str) -> RuleSet {
        RuleSet { rules: text.lines().filter_map(Rule::from_line).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str) -> UrlFilter {
        UrlFilter::compile(p, false).unwrap()
    }

    #[test]
    fn literals_search_anywhere_and_fold_case() {
        assert!(f("ads").matches("https://example.com/ads/banner.png"));
        assert!(f("ADS").matches("https://example.com/ads/banner.png"));
        assert!(!UrlFilter::compile("ADS", true).unwrap().matches("https://example.com/ads/banner.png"));
        assert!(!f("tracker").matches("https://example.com/"));
    }

    #[test]
    fn anchors_pin_the_ends() {
        assert!(f("^https://example\\.com").matches("https://example.com/x"));
        assert!(!f("^example").matches("https://example.com/"));
        assert!(f("\\.png$").matches("https://a.b/c.png"));
        assert!(!f("\\.png$").matches("https://a.b/c.png?x=1"));
        assert!(f("a\\$").matches("cost a$ end"), "an escaped $ is a literal");
    }

    #[test]
    fn dots_and_quantifiers() {
        assert!(f("^https?://.*\\.example\\.com/").matches("http://ads.example.com/x"));
        assert!(f("^https?://.*\\.example\\.com/").matches("https://a.b.example.com/"));
        assert!(!f("^https?://.*\\.example\\.com/").matches("ftp://ads.example.com/"));
        assert!(f("colou?r").matches("color") && f("colou?r").matches("colour"));
        assert!(f("^a+b$").matches("aaab") && !f("^a+b$").matches("b"));
        assert!(f("^a{2,3}$").matches("aaa") && !f("^a{2,3}$").matches("aaaa") && !f("^a{2,3}$").matches("a"));
        assert!(f("^(ab)+$").matches("ababab") && !f("^(ab)+$").matches("aba"));
    }

    #[test]
    fn classes_and_sets() {
        assert!(f("^/\\d+/$").matches("/123/") && !f("^/\\d+/$").matches("/12a/"));
        assert!(f("^[a-c]+$").matches("abcabc") && !f("^[a-c]+$").matches("abd"));
        assert!(f("^[^/]+$").matches("abc") && !f("^[^/]+$").matches("a/b"));
        assert!(f("^[A-C]+$").matches("abc"), "sets fold case too");
        assert!(f("^\\w+$").matches("a_1") && !f("^\\w+$").matches("a-1"));
    }

    #[test]
    fn unsupported_syntax_is_refused_when_compiling() {
        for bad in ["a|b", "(a|b)", "*a", "a**", "a+?", "(ab", "ab)", "[a-", "\\1", "a\\", "x^y", "a{3,1}", "", "\\bfoo"] {
            assert!(UrlFilter::compile(bad, false).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_hostile_pattern_is_cut_off_instead_of_hanging() {
        let evil = f("(a*)*b");
        assert!(!evil.matches(&"a".repeat(40)));
        let evil2 = f("^(a+)+$");
        assert!(!evil2.matches(&format!("{}!", "a".repeat(60))));
    }

    fn rule(pattern: &str, action: Action) -> Rule {
        Rule { trigger: Trigger::new(f(pattern)), action }
    }

    #[test]
    fn rules_apply_in_order_and_ignore_previous_cancels() {
        let set = RuleSet::new(vec![
            rule("ads", Action::Block),
            rule("ads", Action::CssDisplayNone(".ad".into())),
            rule("ads/ok", Action::IgnorePreviousRules),
            rule("ads", Action::BlockCookies),
        ]);
        let ask = |u| set.evaluate(&Request { url: u, resource: ResourceType::Image, top_url: Some("https://site.example/") });
        let blocked = ask("https://x.example/ads/1.png");
        assert!(blocked.block && blocked.block_cookies && blocked.css_selectors == vec![".ad".to_string()]);
        let allowed = ask("https://x.example/ads/ok/1.png");
        assert!(!allowed.block, "ignore-previous-rules cancelled the block");
        assert!(allowed.block_cookies, "rules after the ignore still apply");
        assert!(allowed.css_selectors.is_empty());
        assert!(ask("https://x.example/clean").is_empty());
    }

    #[test]
    fn triggers_narrow_by_type_domain_and_load_type() {
        let mut t = Trigger::new(f("tracker"));
        t.resource_types = Some(vec![ResourceType::Script, ResourceType::Image]);
        t.if_domain = vec!["*news.example".into()];
        t.load_type = Some(LoadType::ThirdParty);
        let set = RuleSet::new(vec![Rule { trigger: t, action: Action::Block }]);
        let req = |url, rt, top| Request { url, resource: rt, top_url: Some(top) };
        assert!(set.evaluate(&req("https://tracker.example/t.js", ResourceType::Script, "https://www.news.example/a")).block);
        assert!(set.evaluate(&req("https://tracker.example/t.js", ResourceType::Script, "https://news.example/")).block);
        assert!(!set.evaluate(&req("https://tracker.example/t.css", ResourceType::StyleSheet, "https://news.example/")).block, "wrong type");
        assert!(!set.evaluate(&req("https://tracker.example/t.js", ResourceType::Script, "https://other.example/")).block, "wrong page domain");
        assert!(!set.evaluate(&req("https://cdn.news.example/tracker.js", ResourceType::Script, "https://www.news.example/")).block, "first party");
        let mut u = Trigger::new(f("tracker"));
        u.unless_domain = vec!["safe.example".into()];
        let set = RuleSet::new(vec![Rule { trigger: u, action: Action::Block }]);
        assert!(!set.evaluate(&req("https://tracker.example/x", ResourceType::Image, "https://a.safe.example/")).block);
        assert!(set.evaluate(&req("https://tracker.example/x", ResourceType::Image, "https://unsafe.example/")).block);
    }

    #[test]
    fn top_url_filters_and_the_navigation_itself() {
        let mut t = Trigger::new(f("^https://bad\\.example/"));
        t.unless_top_url = vec![f("^https://bad\\.example/allowed")];
        let set = RuleSet::new(vec![Rule { trigger: t, action: Action::Block }]);
        let nav = |u| set.evaluate(&Request { url: u, resource: ResourceType::Document, top_url: None });
        assert!(nav("https://bad.example/page").block);
        assert!(!nav("https://bad.example/allowed/page").block);
    }

    #[test]
    fn site_comparison_handles_two_part_suffixes() {
        assert!(same_site("www.example.com", "cdn.example.com"));
        assert!(!same_site("example.com", "example.org"));
        assert!(same_site("news.bbc.co.uk", "static.bbc.co.uk"));
        assert!(!same_site("a.co.uk", "b.co.uk"));
        assert!(same_site("127.0.0.1", "127.0.0.1"));
        assert_eq!(host_of("https://user:pw@Example.COM:8443/p?q#f"), "example.com");
        assert_eq!(host_of("https://[::1]:80/"), "::1");
    }

    #[test]
    fn https_upgrade_drops_port_80_and_spares_local_names() {
        assert_eq!(upgrade_to_https("http://example.com/a?b=1").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(upgrade_to_https("http://example.com:80/a").as_deref(), Some("https://example.com/a"));
        assert_eq!(upgrade_to_https("http://example.com:8080/a").as_deref(), Some("https://example.com:8080/a"));
        assert_eq!(upgrade_to_https("https://example.com/"), None);
        assert_eq!(upgrade_to_https("http://localhost/"), None);
        assert_eq!(upgrade_to_https("http://printer/"), None);
    }

    #[test]
    fn destinations_map_to_resource_types() {
        assert_eq!(ResourceType::from_destination("Image"), ResourceType::Image);
        assert_eq!(ResourceType::from_destination("Style"), ResourceType::StyleSheet);
        assert_eq!(ResourceType::from_destination("None"), ResourceType::Raw);
        assert_eq!(ResourceType::from_destination("Xslt"), ResourceType::Other);
        for t in ResourceType::ALL {
            assert_eq!(ResourceType::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn the_wire_format_round_trips_every_field() {
        let mut t = Trigger::new(UrlFilter::compile("^https://a\\.example/", true).unwrap());
        t.if_domain = vec!["one.example".into(), "*two.example".into()];
        t.unless_domain = vec!["three.example".into()];
        t.resource_types = Some(vec![ResourceType::Script, ResourceType::Font]);
        t.load_type = Some(LoadType::ThirdParty);
        t.if_top_url = vec![f("^https://top\\.example/")];
        t.unless_top_url = vec![UrlFilter::compile("skip", true).unwrap(), f("also")];
        let set = RuleSet::new(vec![
            Rule { trigger: t, action: Action::CssDisplayNone("div.ad, #banner".into()) },
            rule("x", Action::Block),
            rule("y", Action::IgnorePreviousRules),
            rule("z", Action::MakeHttps),
            rule("w", Action::Notify("seen".into())),
            rule("v", Action::BlockCookies),
        ]);
        let wire = set.to_wire();
        assert_eq!(wire.lines().count(), 6);
        assert_eq!(RuleSet::from_wire(&wire), set);
        let garbage = format!("{wire}\nnot a rule\nblock\t0\t(\t\t\t\t\t\t\nhide:\t0\ta\t\t\t\t\t\t");
        assert_eq!(RuleSet::from_wire(&garbage), set, "malformed lines are skipped");
    }

    #[test]
    fn tabs_in_a_selector_cannot_break_the_line() {
        let r = rule("a", Action::CssDisplayNone("a\tb\nc".into()));
        let back = Rule::from_line(&r.to_line()).unwrap();
        assert_eq!(back.action, Action::CssDisplayNone("a b c".into()));
    }
}
