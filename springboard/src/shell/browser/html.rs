//! A forgiving HTML tokenizer and tree builder: tags, attributes, comments,
//! raw-text elements, character references, void elements and the implied
//! end tags that real pages lean on (`<p>` closed by the next block, `<li>`
//! by the next `<li>`). It never fails; broken markup just yields a tree.

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    Element(Element),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Element {
    /// lower case
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class").is_some_and(|c| c.split_ascii_whitespace().any(|x| x == class))
    }
}

impl Node {
    pub fn element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        }
    }

    /// All text below this node, in document order, `<script>`/`<style>` excluded.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        match self {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if matches!(e.tag.as_str(), "script" | "style") => {}
            Node::Element(e) => e.children.iter().for_each(|c| c.collect_text(out)),
        }
    }
}

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "command", "keygen"];
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Tags whose opening ends an open `<p>`.
const CLOSES_P: &[&str] = &[
    "p", "div", "ul", "ol", "dl", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "blockquote", "table", "form", "hr", "section", "article", "header", "footer", "nav", "aside", "main", "figure", "address", "fieldset", "details",
];

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
    self_closing: bool,
}

pub fn parse(src: &str) -> Vec<Node> {
    let mut b = Builder { stack: vec![Element::default()] };
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] != b'<' {
            let next = src[i..].find('<').map_or(len, |p| i + p);
            b.text(&decode_entities(&src[i..next]));
            i = next;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(len, |p| i + p + 3);
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            i = rest.find('>').map_or(len, |p| i + p + 1);
        } else if let Some((tag, end)) = parse_tag(src, i) {
            i = end;
            if tag.closing {
                b.close(&tag.name);
                continue;
            }
            let raw = RAW_TEXT.contains(&tag.name.as_str()) && !tag.self_closing;
            let name = tag.name.clone();
            b.open(tag);
            if raw {
                let stop = find_closing_tag(&src[i..], &name).map_or(len, |p| i + p);
                let body = &src[i..stop];
                if name != "script" && !body.is_empty() {
                    let text = if name == "style" { body.to_string() } else { decode_entities(body) };
                    b.text(&text);
                }
                i = stop;
            }
        } else {
            b.text("<");
            i += 1;
        }
    }
    b.finish()
}

/// Offset of the first `</name` in `hay`, matched ASCII case-insensitively without copying `hay`.
fn find_closing_tag(hay: &str, name: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), name.as_bytes());
    let needle_len = n.len() + 2;
    (0..h.len().checked_sub(needle_len - 1)?)
        .find(|&i| h[i] == b'<' && h[i + 1] == b'/' && h[i + 2..i + needle_len].eq_ignore_ascii_case(n))
}

/// Parses the tag starting at `src[start] == '<'`; returns it and the index after its `>`.
fn parse_tag(src: &str, start: usize) -> Option<(Tag, usize)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = start + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name_start = i;
    while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' && bytes[i] != b'>' {
        i += 1;
    }
    let name = src[name_start..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < len && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            self_closing = bytes[i] == b'/';
            i += 1;
        }
        if i >= len {
            break;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        self_closing = false;
        let key_start = i;
        while i < len && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = src[key_start..i].to_ascii_lowercase();
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < len && bytes[i] == b'=' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                let v0 = i + 1;
                let v1 = bytes[v0..].iter().position(|&c| c == quote).map_or(len, |p| v0 + p);
                value = decode_entities(&src[v0..v1]);
                i = (v1 + 1).min(len);
            } else {
                let v0 = i;
                while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = decode_entities(&src[v0..i]);
            }
        }
        if !key.is_empty() && !attrs.iter().any(|(k, _): &(String, String)| *k == key) {
            attrs.push((key, value));
        }
    }
    Some((Tag { name, attrs, closing, self_closing }, i))
}

/// Deepest element nesting kept; deeper elements are appended flat.
const MAX_DEPTH: usize = 512;

struct Builder {
    /// stack[0] is the synthetic root
    stack: Vec<Element>,
}

impl Builder {
    fn text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        let top = self.stack.last_mut().unwrap();
        if let Some(Node::Text(prev)) = top.children.last_mut() {
            prev.push_str(t);
        } else {
            top.children.push(Node::Text(t.to_string()));
        }
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            let e = self.stack.pop().unwrap();
            self.stack.last_mut().unwrap().children.push(Node::Element(e));
        }
    }

    /// Pops down to the nearest open element named in `names`, unless a
    /// `barrier` element is met first.
    fn close_scoped(&mut self, names: &[&str], barrier: &[&str]) {
        for idx in (1..self.stack.len()).rev() {
            let tag = self.stack[idx].tag.as_str();
            if names.contains(&tag) {
                while self.stack.len() > idx {
                    self.pop();
                }
                return;
            }
            if barrier.contains(&tag) {
                return;
            }
        }
    }

    fn open(&mut self, tag: Tag) {
        let name = tag.name.as_str();
        if CLOSES_P.contains(&name) {
            self.close_scoped(&["p"], &["button", "td", "th", "li", "table", "blockquote"]);
        }
        match name {
            "li" => self.close_scoped(&["li"], &["ul", "ol"]),
            "dt" | "dd" => self.close_scoped(&["dt", "dd"], &["dl"]),
            "tr" => self.close_scoped(&["tr"], &["table"]),
            "td" | "th" => self.close_scoped(&["td", "th"], &["tr", "table"]),
            "option" => self.close_scoped(&["option"], &["select"]),
            _ => {}
        }
        let el = Element { tag: tag.name.clone(), attrs: tag.attrs, children: vec![] };
        if VOID.contains(&name) || tag.self_closing || self.stack.len() >= MAX_DEPTH {
            self.stack.last_mut().unwrap().children.push(Node::Element(el));
        } else {
            self.stack.push(el);
        }
    }

    fn close(&mut self, name: &str) {
        if VOID.contains(&name) {
            return;
        }
        if let Some(idx) = (1..self.stack.len()).rev().find(|&i| self.stack[i].tag == name) {
            while self.stack.len() > idx {
                self.pop();
            }
        }
    }

    fn finish(mut self) -> Vec<Node> {
        while self.stack.len() > 1 {
            self.pop();
        }
        self.stack.pop().unwrap().children
    }
}

/// Named references, sorted by name for binary search.
const NAMED: &[(&str, &str)] = &[
    ("Aacute", "\u{c1}"), ("Auml", "\u{c4}"), ("Eacute", "\u{c9}"), ("Ouml", "\u{d6}"), ("Uuml", "\u{dc}"), ("aacute", "\u{e1}"),
    ("aelig", "\u{e6}"), ("agrave", "\u{e0}"), ("amp", "&"), ("apos", "'"), ("auml", "\u{e4}"), ("bdquo", "\u{201e}"),
    ("bull", "\u{2022}"), ("ccedil", "\u{e7}"), ("cent", "\u{a2}"), ("copy", "\u{a9}"), ("dagger", "\u{2020}"), ("darr", "\u{2193}"),
    ("deg", "\u{b0}"), ("divide", "\u{f7}"), ("eacute", "\u{e9}"), ("egrave", "\u{e8}"), ("emsp", "\u{2003}"), ("ensp", "\u{2002}"),
    ("eth", "\u{f0}"), ("euro", "\u{20ac}"), ("frac12", "\u{bd}"), ("gt", ">"), ("hellip", "\u{2026}"), ("iacute", "\u{ed}"),
    ("iexcl", "\u{a1}"), ("iquest", "\u{bf}"), ("laquo", "\u{ab}"), ("larr", "\u{2190}"), ("ldquo", "\u{201c}"),
    ("lsquo", "\u{2018}"), ("lt", "<"), ("mdash", "\u{2014}"), ("micro", "\u{b5}"), ("middot", "\u{b7}"), ("minus", "\u{2212}"),
    ("nbsp", "\u{a0}"), ("ndash", "\u{2013}"), ("ntilde", "\u{f1}"), ("oacute", "\u{f3}"), ("ouml", "\u{f6}"), ("para", "\u{b6}"),
    ("permil", "\u{2030}"), ("plusmn", "\u{b1}"), ("pound", "\u{a3}"), ("prime", "\u{2032}"), ("quot", "\""), ("raquo", "\u{bb}"),
    ("rarr", "\u{2192}"), ("rdquo", "\u{201d}"), ("reg", "\u{ae}"), ("rsquo", "\u{2019}"), ("sbquo", "\u{201a}"), ("sect", "\u{a7}"),
    ("shy", "\u{ad}"), ("sup2", "\u{b2}"), ("sup3", "\u{b3}"), ("szlig", "\u{df}"), ("thinsp", "\u{2009}"), ("thorn", "\u{fe}"),
    ("times", "\u{d7}"), ("trade", "\u{2122}"), ("uacute", "\u{fa}"), ("uarr", "\u{2191}"), ("uuml", "\u{fc}"), ("yacute", "\u{fd}"),
    ("yen", "\u{a5}"), ("zwj", "\u{200d}"), ("zwnj", "\u{200c}"),
];

/// The Windows-1252 characters HTML substitutes for the C1 range `&#x80;`..`&#x9f;`.
const C1_MAP: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}',
    '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}',
    '\u{2013}', '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];

fn named(name: &str) -> Option<&'static str> {
    NAMED.binary_search_by(|(n, _)| (*n).cmp(name)).ok().map(|i| NAMED[i].1)
}

/// Legacy references that browsers also accept without the closing `;` (`&amp`, `&copy`, `&eacute`, ...).
fn legacy_named(name: &str) -> Option<&'static str> {
    let v = named(name)?;
    let ch = v.chars().next()?;
    (name != "apos" && ch <= '\u{ff}').then_some(v)
}

/// Decodes the number of `&#...` / `&#x...`; `None` when it has no digits.
fn numeric_char(num: &str) -> Option<char> {
    let code = match num.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => num.parse(),
    };
    // Overflowing numbers are still a numeric reference; they decode to U+FFFD.
    let code = code.unwrap_or(u32::MAX);
    Some(match code {
        0x80..=0x9f => C1_MAP[(code - 0x80) as usize],
        _ => char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}'),
    })
}

/// Decodes the reference at the start of `rest` (`rest` starts with `&`); returns the text and the bytes used.
fn decode_reference(rest: &str) -> Option<(String, usize)> {
    let run = rest[1..].find(|c: char| !c.is_ascii_alphanumeric() && c != '#').unwrap_or(rest.len() - 1);
    let name = &rest[1..1 + run];
    let semi = rest[1 + run..].starts_with(';') as usize;
    if let Some(num) = name.strip_prefix('#') {
        let digits = num.strip_prefix(['x', 'X']).unwrap_or(num);
        let valid = !digits.is_empty() && digits.bytes().all(|b| if num.len() != digits.len() { b.is_ascii_hexdigit() } else { b.is_ascii_digit() });
        return valid.then(|| numeric_char(num)).flatten().map(|c| (c.to_string(), 1 + run + semi));
    }
    if semi == 1 {
        if let Some(v) = named(name) {
            return Some((v.to_string(), 1 + run + 1));
        }
    }
    // No (known) `;`: the longest legacy reference that prefixes the name.
    (2..=name.len().min(8)).rev().find_map(|n| name.get(..n).and_then(legacy_named).map(|v| (v.to_string(), 1 + n)))
}

/// Resolves `&amp;`-style references; unknown ones stay as written.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        match decode_reference(rest) {
            Some((text, used)) => {
                out.push_str(&text);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every element named `tag` below `nodes`, in document order.
pub fn find_all<'a>(nodes: &'a [Node], tag: &str, out: &mut Vec<&'a Element>) {
    for n in nodes {
        if let Node::Element(e) = n {
            if e.tag == tag {
                out.push(e);
            }
            find_all(&e.children, tag, out);
        }
    }
}

/// The page title, whitespace collapsed.
pub fn title(nodes: &[Node]) -> Option<String> {
    let mut found = Vec::new();
    find_all(nodes, "title", &mut found);
    let text: String = found.first()?.children.iter().map(Node::text_content).collect();
    let t = text.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(n: &Node) -> &Element {
        n.element().expect("an element")
    }

    #[test]
    fn nests_elements_and_reads_attributes() {
        let t = parse(r#"<div id=main class="a b"><a href='/x?y=1&amp;z=2'>go</a></div>"#);
        let div = el(&t[0]);
        assert_eq!((div.tag.as_str(), div.attr("id")), ("div", Some("main")));
        assert!(div.has_class("b") && !div.has_class("c"));
        let a = el(&div.children[0]);
        assert_eq!(a.attr("href"), Some("/x?y=1&z=2"));
        assert_eq!(a.children, vec![Node::Text("go".into())]);
    }

    #[test]
    fn a_new_paragraph_closes_the_open_one() {
        let t = parse("<p>one<p>two<div>three</div>");
        assert_eq!(t.len(), 3);
        assert_eq!(el(&t[0]).children, vec![Node::Text("one".into())]);
        assert_eq!(el(&t[1]).tag, "p");
        assert_eq!(el(&t[2]).tag, "div");
    }

    #[test]
    fn list_items_close_each_other_but_not_across_nested_lists() {
        let t = parse("<ul><li>a<li>b<ul><li>c</ul><li>d</ul>");
        let ul = el(&t[0]);
        assert_eq!(ul.children.len(), 3);
        let b = el(&ul.children[1]);
        assert_eq!(el(&b.children[1]).tag, "ul");
    }

    #[test]
    fn void_elements_and_self_closing_tags_have_no_children() {
        let t = parse("a<br>b<img src=x.png><hr/>c");
        let tags: Vec<_> = t.iter().map(|n| n.element().map_or("#text", |e| e.tag.as_str())).collect();
        assert_eq!(tags, ["#text", "br", "#text", "img", "hr", "#text"]);
    }

    #[test]
    fn scripts_vanish_and_styles_keep_their_text() {
        let t = parse("<script>if (a<b) {}</script><style>p{color:red}</style><p>x");
        assert_eq!(t.len(), 3);
        assert!(el(&t[0]).children.is_empty());
        assert_eq!(el(&t[1]).children, vec![Node::Text("p{color:red}".into())]);
        assert_eq!(t[0].text_content() + &t[1].text_content() + &t[2].text_content(), "x");
    }

    #[test]
    fn comments_and_doctype_are_skipped() {
        let t = parse("<!DOCTYPE html><!-- hi <b>no</b> --><p>x</p>");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn character_references_decode() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &bogus; &"), "a & b <c> AB &bogus; &");
        assert_eq!(decode_entities("caf&eacute;&nbsp;x"), "caf\u{e9}\u{a0}x");
    }

    #[test]
    fn named_table_is_sorted_for_binary_search() {
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn legacy_references_decode_without_semicolon() {
        assert_eq!(decode_entities("a &amp b &copy 2026 &eacutex &lt&gt"), "a & b \u{a9} 2026 \u{e9}x <>");
        assert_eq!(decode_entities("&hellip &apos"), "&hellip &apos");
    }

    #[test]
    fn numeric_references_follow_the_html_spec() {
        assert_eq!(decode_entities("&#0;&#xD800;&#x110000;&#99999999999;"), "\u{fffd}\u{fffd}\u{fffd}\u{fffd}");
        assert_eq!(decode_entities("&#x80;&#150;&#x9f;"), "\u{20ac}\u{2013}\u{178}");
        assert_eq!(decode_entities("&#65 &#; &#x;"), "A &#; &#x;");
    }

    #[test]
    fn raw_text_end_tags_match_in_any_case() {
        let t = parse("<STYLE>p{}</Style><p>x</p><script>a</SCRIPT>y");
        assert_eq!(el(&t[0]).children, vec![Node::Text("p{}".into())]);
        assert_eq!(t[1].text_content() + &t[3].text_content(), "xy");
    }

    #[test]
    fn stray_angle_brackets_and_unclosed_tags_survive() {
        let t = parse("1 < 2 <b>bold <i>both");
        assert_eq!(t[0], Node::Text("1 < 2 ".into()));
        assert_eq!(t[1].text_content(), "bold both");
    }

    #[test]
    fn title_is_found_and_collapsed() {
        let t = parse("<html><head><title>  Hello\n  &amp; bye </title></head><body>x</body></html>");
        assert_eq!(title(&t).as_deref(), Some("Hello & bye"));
        assert_eq!(title(&parse("<p>x")), None);
    }

    #[test]
    fn mismatched_end_tags_are_ignored() {
        let t = parse("<div><span>a</div>b</span>");
        assert_eq!(t[0].text_content(), "a");
        assert_eq!(t[1], Node::Text("b".into()));
    }
}
