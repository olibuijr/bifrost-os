//! A small CSS selector matcher over `Dom`: type, universal, class, id,
//! attribute operators, the four combinators, `:not`, `:is`, `:where`,
//! `:first-child`, `:last-child`, `:only-child`, `:nth-child`, `:nth-last-child`,
//! the `-of-type` forms, `:empty`, `:root`, `:checked` and `:disabled`.
//! It serves `querySelector[All]`, `matches` and `closest`.

use crate::shell::browser::dom::{Dom, NodeId};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comb {
    None,
    Descendant,
    Child,
    Adjacent,
    General,
}

#[derive(Clone, Debug, PartialEq)]
enum AttrOp {
    Exists,
    Eq,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substr,
}

#[derive(Clone, Debug)]
struct AttrSel {
    name: String,
    op: AttrOp,
    value: String,
    ci: bool,
}

#[derive(Clone, Debug)]
enum Pseudo {
    Not(SelectorList),
    Is(SelectorList),
    Nth { a: i32, b: i32, last: bool, of_type: bool },
    Empty,
    Root,
    Checked,
    Disabled,
    Enabled,
}

#[derive(Clone, Debug, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSel>,
    pseudos: Vec<Pseudo>,
}

#[derive(Clone, Debug)]
struct Complex {
    parts: Vec<(Comb, Compound)>,
}

/// A parsed comma separated selector list.
#[derive(Clone, Debug, Default)]
pub struct SelectorList {
    list: Vec<Complex>,
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
        self.i > start
    }

    fn ident(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '\\' {
                self.i += 1;
                if let Some(n) = self.peek() {
                    out.push(n);
                    self.i += 1;
                }
            } else if is_ident(c) {
                out.push(c);
                self.i += 1;
            } else {
                break;
            }
        }
        out
    }

    fn list(&mut self, nested: bool) -> Result<SelectorList, String> {
        let mut list = Vec::new();
        loop {
            self.skip_ws();
            list.push(self.complex()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some(')') if nested => break,
                None if !nested => break,
                Some(c) => return Err(format!("unexpected '{c}' in selector")),
                None => return Err("unclosed parenthesis in selector".into()),
            }
        }
        Ok(SelectorList { list })
    }

    fn complex(&mut self) -> Result<Complex, String> {
        let mut parts = Vec::new();
        let mut comb = Comb::None;
        loop {
            let compound = self.compound()?;
            parts.push((comb, compound));
            let had_ws = self.skip_ws();
            match self.peek() {
                None | Some(',') | Some(')') => break,
                Some('>') => {
                    self.i += 1;
                    comb = Comb::Child;
                }
                Some('+') => {
                    self.i += 1;
                    comb = Comb::Adjacent;
                }
                Some('~') => {
                    self.i += 1;
                    comb = Comb::General;
                }
                Some(_) if had_ws => comb = Comb::Descendant,
                Some(c) => return Err(format!("unexpected '{c}' in selector")),
            }
            self.skip_ws();
        }
        Ok(Complex { parts })
    }

    fn compound(&mut self) -> Result<Compound, String> {
        let mut c = Compound::default();
        let mut any = false;
        if self.peek() == Some('*') {
            self.i += 1;
            any = true;
        } else if self.peek().is_some_and(|ch| is_ident(ch) || ch == '\\') {
            c.tag = Some(self.ident().to_ascii_lowercase());
            any = true;
        }
        loop {
            match self.peek() {
                Some('#') => {
                    self.i += 1;
                    c.id = Some(self.ident());
                }
                Some('.') => {
                    self.i += 1;
                    let n = self.ident();
                    if n.is_empty() {
                        return Err("empty class selector".into());
                    }
                    c.classes.push(n);
                }
                Some('[') => {
                    self.i += 1;
                    c.attrs.push(self.attr()?);
                }
                Some(':') => {
                    self.i += 1;
                    if self.peek() == Some(':') {
                        return Err("pseudo-elements are not selectable".into());
                    }
                    c.pseudos.push(self.pseudo()?);
                }
                _ => break,
            }
            any = true;
        }
        if !any {
            return Err("empty selector".into());
        }
        Ok(c)
    }

    fn attr(&mut self) -> Result<AttrSel, String> {
        self.skip_ws();
        let name = self.ident().to_ascii_lowercase();
        if name.is_empty() {
            return Err("empty attribute name".into());
        }
        self.skip_ws();
        let mut op = AttrOp::Exists;
        let mut value = String::new();
        let mut ci = false;
        if self.peek() != Some(']') {
            let first = self.peek().ok_or("unclosed attribute selector")?;
            op = match first {
                '=' => {
                    self.i += 1;
                    AttrOp::Eq
                }
                '~' | '|' | '^' | '$' | '*' => {
                    self.i += 1;
                    if self.peek() != Some('=') {
                        return Err("bad attribute operator".into());
                    }
                    self.i += 1;
                    match first {
                        '~' => AttrOp::Includes,
                        '|' => AttrOp::Dash,
                        '^' => AttrOp::Prefix,
                        '$' => AttrOp::Suffix,
                        _ => AttrOp::Substr,
                    }
                }
                _ => return Err("bad attribute operator".into()),
            };
            self.skip_ws();
            match self.peek() {
                Some(q) if q == '"' || q == '\'' => {
                    self.i += 1;
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == q {
                            break;
                        }
                        if c == '\\' {
                            if let Some(n) = self.peek() {
                                value.push(n);
                                self.i += 1;
                            }
                        } else {
                            value.push(c);
                        }
                    }
                }
                _ => value = self.ident(),
            }
            self.skip_ws();
            if matches!(self.peek(), Some('i') | Some('I')) {
                ci = true;
                self.i += 1;
                self.skip_ws();
            }
        }
        if self.peek() != Some(']') {
            return Err("unclosed attribute selector".into());
        }
        self.i += 1;
        Ok(AttrSel { name, op, value, ci })
    }

    fn paren_arg(&mut self) -> Result<String, String> {
        if self.peek() != Some('(') {
            return Err("expected '('".into());
        }
        self.i += 1;
        let mut depth = 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                }
                _ => {}
            }
            out.push(c);
        }
        Err("unclosed parenthesis".into())
    }

    fn pseudo(&mut self) -> Result<Pseudo, String> {
        let name = self.ident().to_ascii_lowercase();
        let nth = |a, b, last, of_type| Pseudo::Nth { a, b, last, of_type };
        Ok(match name.as_str() {
            "not" | "is" | "where" | "matches" => {
                let arg = self.paren_arg()?;
                let inner = parse(&arg)?;
                if name == "not" {
                    Pseudo::Not(inner)
                } else {
                    Pseudo::Is(inner)
                }
            }
            "first-child" => nth(0, 1, false, false),
            "last-child" => nth(0, 1, true, false),
            "first-of-type" => nth(0, 1, false, true),
            "last-of-type" => nth(0, 1, true, true),
            "only-child" => {
                // Expressed as :is(:first-child):is(:last-child) by the caller.
                Pseudo::Is(SelectorList {
                    list: vec![Complex {
                        parts: vec![(
                            Comb::None,
                            Compound {
                                pseudos: vec![nth(0, 1, false, false), nth(0, 1, true, false)],
                                ..Compound::default()
                            },
                        )],
                    }],
                })
            }
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                let arg = self.paren_arg()?;
                let (a, b) = parse_nth(&arg)?;
                nth(a, b, name.contains("last"), name.ends_with("of-type"))
            }
            "empty" => Pseudo::Empty,
            "root" => Pseudo::Root,
            "checked" => Pseudo::Checked,
            "disabled" => Pseudo::Disabled,
            "enabled" => Pseudo::Enabled,
            other => return Err(format!("unsupported pseudo-class :{other}")),
        })
    }
}

fn parse_nth(arg: &str) -> Result<(i32, i32), String> {
    let t: String = arg.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    let bad = || format!("bad nth expression '{arg}'");
    match t.as_str() {
        "odd" => return Ok((2, 1)),
        "even" => return Ok((2, 0)),
        "" => return Err(bad()),
        _ => {}
    }
    if let Some(pos) = t.find('n') {
        let a = match &t[..pos] {
            "" | "+" => 1,
            "-" => -1,
            s => s.parse::<i32>().map_err(|_| bad())?,
        };
        let rest = &t[pos + 1..];
        let b = if rest.is_empty() { 0 } else { rest.trim_start_matches('+').parse::<i32>().map_err(|_| bad())? };
        Ok((a, b))
    } else {
        Ok((0, t.trim_start_matches('+').parse::<i32>().map_err(|_| bad())?))
    }
}

/// Parses a selector list; the error is a human readable reason.
pub fn parse(sel: &str) -> Result<SelectorList, String> {
    let mut p = Parser { s: sel.chars().collect(), i: 0 };
    if sel.trim().is_empty() {
        return Err("empty selector".into());
    }
    p.list(false)
}

fn is_element(dom: &Dom, n: NodeId) -> bool {
    dom.tag(n).is_some()
}

fn prev_element(dom: &Dom, n: NodeId) -> Option<NodeId> {
    let mut cur = dom.prev_sibling(n);
    while let Some(c) = cur {
        if is_element(dom, c) {
            return Some(c);
        }
        cur = dom.prev_sibling(c);
    }
    None
}

fn next_element(dom: &Dom, n: NodeId) -> Option<NodeId> {
    let mut cur = dom.next_sibling(n);
    while let Some(c) = cur {
        if is_element(dom, c) {
            return Some(c);
        }
        cur = dom.next_sibling(c);
    }
    None
}

fn position(dom: &Dom, n: NodeId, last: bool, of_type: bool) -> i32 {
    let tag = dom.tag(n).map(str::to_owned);
    let mut pos = 1;
    let mut cur = if last { next_element(dom, n) } else { prev_element(dom, n) };
    while let Some(c) = cur {
        if !of_type || dom.tag(c).map(str::to_owned) == tag {
            pos += 1;
        }
        cur = if last { next_element(dom, c) } else { prev_element(dom, c) };
    }
    pos
}

fn nth_matches(a: i32, b: i32, pos: i32) -> bool {
    if a == 0 {
        return pos == b;
    }
    let d = pos - b;
    d % a == 0 && d / a >= 0
}

fn attr_matches(dom: &Dom, n: NodeId, a: &AttrSel) -> bool {
    let Some(v) = dom.attr(n, &a.name) else { return false };
    if a.op == AttrOp::Exists {
        return true;
    }
    let (v, want) = if a.ci { (v.to_lowercase(), a.value.to_lowercase()) } else { (v.to_owned(), a.value.clone()) };
    match a.op {
        AttrOp::Exists => true,
        AttrOp::Eq => v == want,
        AttrOp::Includes => !want.is_empty() && v.split_whitespace().any(|w| w == want),
        AttrOp::Dash => v == want || v.starts_with(&format!("{want}-")),
        AttrOp::Prefix => !want.is_empty() && v.starts_with(&want),
        AttrOp::Suffix => !want.is_empty() && v.ends_with(&want),
        AttrOp::Substr => !want.is_empty() && v.contains(&want),
    }
}

fn pseudo_matches(dom: &Dom, n: NodeId, p: &Pseudo) -> bool {
    match p {
        Pseudo::Not(l) => !l.matches(dom, n),
        Pseudo::Is(l) => l.matches(dom, n),
        Pseudo::Nth { a, b, last, of_type } => nth_matches(*a, *b, position(dom, n, *last, *of_type)),
        Pseudo::Empty => dom.children(n).all(|c| match dom.kind(c) {
            crate::shell::browser::dom::NodeKind::Text(t) => t.is_empty(),
            crate::shell::browser::dom::NodeKind::Comment(_) => true,
            _ => false,
        }),
        Pseudo::Root => dom.document_element() == Some(n),
        Pseudo::Checked => {
            dom.attr(n, "checked").is_some() && matches!(dom.tag(n), Some("input"))
                || dom.attr(n, "selected").is_some() && matches!(dom.tag(n), Some("option"))
        }
        Pseudo::Disabled => dom.attr(n, "disabled").is_some(),
        Pseudo::Enabled => {
            matches!(dom.tag(n), Some("input" | "button" | "select" | "textarea")) && dom.attr(n, "disabled").is_none()
        }
    }
}

fn compound_matches(dom: &Dom, n: NodeId, c: &Compound) -> bool {
    let Some(tag) = dom.tag(n) else { return false };
    if let Some(t) = &c.tag {
        if t != tag {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if dom.id(n) != Some(id.as_str()) {
            return false;
        }
    }
    c.classes.iter().all(|k| dom.has_class(n, k))
        && c.attrs.iter().all(|a| attr_matches(dom, n, a))
        && c.pseudos.iter().all(|p| pseudo_matches(dom, n, p))
}

fn complex_matches(dom: &Dom, n: NodeId, parts: &[(Comb, Compound)], idx: usize) -> bool {
    let (comb, compound) = &parts[idx];
    if !compound_matches(dom, n, compound) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    let up = idx - 1;
    match comb {
        Comb::None => true,
        Comb::Child => dom
            .parent(n)
            .is_some_and(|p| is_element(dom, p) && complex_matches(dom, p, parts, up)),
        Comb::Descendant => {
            let mut cur = dom.parent(n);
            while let Some(p) = cur {
                if is_element(dom, p) && complex_matches(dom, p, parts, up) {
                    return true;
                }
                cur = dom.parent(p);
            }
            false
        }
        Comb::Adjacent => prev_element(dom, n).is_some_and(|p| complex_matches(dom, p, parts, up)),
        Comb::General => {
            let mut cur = prev_element(dom, n);
            while let Some(p) = cur {
                if complex_matches(dom, p, parts, up) {
                    return true;
                }
                cur = prev_element(dom, p);
            }
            false
        }
    }
}

impl SelectorList {
    /// Whether the element `n` matches any selector of the list.
    pub fn matches(&self, dom: &Dom, n: NodeId) -> bool {
        self.list
            .iter()
            .any(|c| !c.parts.is_empty() && complex_matches(dom, n, &c.parts, c.parts.len() - 1))
    }
}

/// Elements below `root` (excluding it) matching the list, in document order.
pub fn query_all(dom: &Dom, root: NodeId, list: &SelectorList, first_only: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    for n in dom.descendants(root) {
        if list.matches(dom, n) {
            out.push(n);
            if first_only {
                break;
            }
        }
    }
    out
}

/// The nearest inclusive ancestor of `n` matching the list.
pub fn closest(dom: &Dom, n: NodeId, list: &SelectorList) -> Option<NodeId> {
    let mut cur = Some(n);
    while let Some(c) = cur {
        if is_element(dom, c) && list.matches(dom, c) {
            return Some(c);
        }
        cur = dom.parent(c);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::browser::dom::Dom;

    /// html > body > div#a.box.big[data-x="1"] > (p.one, p.two, span, p.three)
    fn sample() -> (Dom, Vec<NodeId>) {
        let mut d = Dom::new();
        let html = d.create_element("html");
        let body = d.create_element("body");
        let div = d.create_element("div");
        d.append_child(d.root(), html);
        d.append_child(html, body);
        d.append_child(body, div);
        d.set_attr(div, "id", "a");
        d.set_attr(div, "class", "box big");
        d.set_attr(div, "data-x", "1");
        let mut kids = vec![div];
        for (tag, class) in [("p", "one"), ("p", "two"), ("span", ""), ("p", "three")] {
            let e = d.create_element(tag);
            if !class.is_empty() {
                d.set_attr(e, "class", class);
            }
            d.append_child(div, e);
            kids.push(e);
        }
        (d, kids)
    }

    fn count(d: &Dom, sel: &str) -> usize {
        query_all(d, d.root(), &parse(sel).unwrap(), false).len()
    }

    #[test]
    fn type_class_id() {
        let (d, _) = sample();
        assert_eq!(count(&d, "p"), 3);
        assert_eq!(count(&d, ".box"), 1);
        assert_eq!(count(&d, "div.box.big"), 1);
        assert_eq!(count(&d, "#a"), 1);
        assert_eq!(count(&d, "*"), 7);
        assert_eq!(count(&d, "P"), 3);
    }

    #[test]
    fn attribute_operators() {
        let (d, _) = sample();
        assert_eq!(count(&d, "[data-x]"), 1);
        assert_eq!(count(&d, "[data-x=\"1\"]"), 1);
        assert_eq!(count(&d, "[data-x=2]"), 0);
        assert_eq!(count(&d, "[class~=big]"), 1);
        assert_eq!(count(&d, "[class^=bo]"), 1);
        assert_eq!(count(&d, "[class$=ig]"), 1);
        assert_eq!(count(&d, "[class*=\"x b\"]"), 1);
        assert_eq!(count(&d, "[data-x=\"1\" i]"), 1);
    }

    #[test]
    fn combinators() {
        let (d, _) = sample();
        assert_eq!(count(&d, "body p"), 3);
        assert_eq!(count(&d, "body > p"), 0);
        assert_eq!(count(&d, "div > p"), 3);
        assert_eq!(count(&d, ".one + p"), 1);
        assert_eq!(count(&d, ".one ~ p"), 2);
        assert_eq!(count(&d, ".one + span"), 0);
        assert_eq!(count(&d, "html > body > div p.two"), 1);
    }

    #[test]
    fn pseudo_classes() {
        let (d, _) = sample();
        assert_eq!(count(&d, "p:not(.one)"), 2);
        assert_eq!(count(&d, "p:first-child"), 1);
        assert_eq!(count(&d, "p:last-child"), 1);
        assert_eq!(count(&d, "div > :nth-child(2)"), 1);
        assert_eq!(count(&d, "div > :nth-child(odd)"), 2);
        assert_eq!(count(&d, "div > :nth-child(2n+2)"), 2);
        assert_eq!(count(&d, "div > :nth-child(-n+2)"), 2);
        assert_eq!(count(&d, "div > :nth-last-child(1)"), 1);
        assert_eq!(count(&d, "p:nth-of-type(2)"), 1);
        assert_eq!(count(&d, "span:empty"), 1);
        assert_eq!(count(&d, ":root"), 1);
        assert_eq!(count(&d, ":is(span, .two)"), 2);
        assert_eq!(count(&d, "html:only-child"), 1);
    }

    #[test]
    fn lists_and_scoping() {
        let (d, kids) = sample();
        assert_eq!(count(&d, "span, .one"), 2);
        let l = parse("p").unwrap();
        assert!(query_all(&d, kids[1], &l, false).is_empty());
        assert_eq!(query_all(&d, kids[0], &l, true).len(), 1);
    }

    #[test]
    fn matches_and_closest() {
        let (d, kids) = sample();
        assert!(parse(".one").unwrap().matches(&d, kids[1]));
        assert!(!parse(".two").unwrap().matches(&d, kids[1]));
        assert_eq!(closest(&d, kids[1], &parse(".box").unwrap()), Some(kids[0]));
        assert_eq!(closest(&d, kids[1], &parse("p").unwrap()), Some(kids[1]));
        assert_eq!(closest(&d, kids[1], &parse("table").unwrap()), None);
    }

    #[test]
    fn invalid_selectors_error() {
        assert!(parse("").is_err());
        assert!(parse("p >").is_err());
        assert!(parse("[a").is_err());
        assert!(parse(":nope").is_err());
        assert!(parse("p::before").is_err());
        assert!(parse("a,,b").is_err());
    }
}
