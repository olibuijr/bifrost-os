//! Arena DOM built by html5ever. Node 0 is the Document; nodes never move
//! and removed nodes stay allocated (detached). Every mutation bumps a
//! generation counter so caches can tell when to rebuild.

use std::cell::{Ref, RefCell};
use std::collections::HashMap;

use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{ns, parse_document, parse_fragment, Attribute, LocalName, ParseOpts, QualName};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Document,
    Element { tag: String, attrs: Vec<(String, String)> },
    Text(String),
    Comment(String),
}

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    parent: Option<NodeId>,
    first: Option<NodeId>,
    last: Option<NodeId>,
    prev: Option<NodeId>,
    next: Option<NodeId>,
}

#[derive(Clone, Debug)]
pub struct Dom {
    nodes: Vec<Node>,
    generation: u64,
}

impl Default for Dom {
    fn default() -> Self {
        Dom::new()
    }
}

pub struct Children<'a> {
    dom: &'a Dom,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let cur = self.next?;
        self.next = self.dom.next_sibling(cur);
        Some(cur)
    }
}

pub struct Descendants<'a> {
    dom: &'a Dom,
    root: NodeId,
    cur: NodeId,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        if let Some(c) = self.dom.first_child(self.cur) {
            self.cur = c;
            return Some(c);
        }
        loop {
            if self.cur == self.root {
                return None;
            }
            if let Some(s) = self.dom.next_sibling(self.cur) {
                self.cur = s;
                return Some(s);
            }
            self.cur = self.dom.parent(self.cur)?;
        }
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr", "basefont", "bgsound", "frame", "keygen",
];
const RAW: &[&str] = &[
    "script", "style", "xmp", "iframe", "noembed", "noframes", "plaintext",
];

impl Dom {
    pub fn new() -> Dom {
        let mut d = Dom { nodes: Vec::new(), generation: 0 };
        d.alloc(NodeKind::Document);
        d
    }

    fn alloc(&mut self, kind: NodeKind) -> NodeId {
        self.nodes.push(Node { kind, parent: None, first: None, last: None, prev: None, next: None });
        NodeId(self.nodes.len() as u32 - 1)
    }

    fn n(&self, n: NodeId) -> &Node {
        &self.nodes[n.0 as usize]
    }

    fn nm(&mut self, n: NodeId) -> &mut Node {
        &mut self.nodes[n.0 as usize]
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }
    pub fn kind(&self, n: NodeId) -> &NodeKind {
        &self.n(n).kind
    }
    pub fn parent(&self, n: NodeId) -> Option<NodeId> {
        self.n(n).parent
    }
    pub fn first_child(&self, n: NodeId) -> Option<NodeId> {
        self.n(n).first
    }
    pub fn last_child(&self, n: NodeId) -> Option<NodeId> {
        self.n(n).last
    }
    pub fn next_sibling(&self, n: NodeId) -> Option<NodeId> {
        self.n(n).next
    }
    pub fn prev_sibling(&self, n: NodeId) -> Option<NodeId> {
        self.n(n).prev
    }
    pub fn children(&self, n: NodeId) -> Children<'_> {
        Children { dom: self, next: self.first_child(n) }
    }
    pub fn descendants(&self, n: NodeId) -> Descendants<'_> {
        Descendants { dom: self, root: n, cur: n }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.generation += 1;
        self.alloc(NodeKind::Element { tag: tag.to_ascii_lowercase(), attrs: Vec::new() })
    }
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.generation += 1;
        self.alloc(NodeKind::Text(text.to_string()))
    }
    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.generation += 1;
        self.alloc(NodeKind::Comment(text.to_string()))
    }

    fn detach(&mut self, c: NodeId) {
        let (p, pr, nx) = {
            let node = self.n(c);
            (node.parent, node.prev, node.next)
        };
        if let Some(pr) = pr {
            self.nm(pr).next = nx;
        } else if let Some(p) = p {
            self.nm(p).first = nx;
        }
        if let Some(nx) = nx {
            self.nm(nx).prev = pr;
        } else if let Some(p) = p {
            self.nm(p).last = pr;
        }
        let node = self.nm(c);
        node.parent = None;
        node.prev = None;
        node.next = None;
    }

    fn is_inclusive_ancestor(&self, anc: NodeId, mut n: NodeId) -> bool {
        loop {
            if n == anc {
                return true;
            }
            match self.parent(n) {
                Some(p) => n = p,
                None => return false,
            }
        }
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.insert_before(parent, child, None);
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        if self.is_inclusive_ancestor(child, parent) || child == self.root() {
            return;
        }
        if before == Some(child) {
            return;
        }
        self.generation += 1;
        self.detach(child);
        let before = before.filter(|b| self.parent(*b) == Some(parent));
        let prev = match before {
            Some(b) => self.n(b).prev,
            None => self.n(parent).last,
        };
        {
            let c = self.nm(child);
            c.parent = Some(parent);
            c.prev = prev;
            c.next = before;
        }
        match prev {
            Some(p) => self.nm(p).next = Some(child),
            None => self.nm(parent).first = Some(child),
        }
        match before {
            Some(b) => self.nm(b).prev = Some(child),
            None => self.nm(parent).last = Some(child),
        }
    }

    pub fn remove(&mut self, n: NodeId) {
        self.generation += 1;
        self.detach(n);
    }

    pub fn tag(&self, n: NodeId) -> Option<&str> {
        match &self.n(n).kind {
            NodeKind::Element { tag, .. } => Some(tag),
            _ => None,
        }
    }

    pub fn attr(&self, n: NodeId, name: &str) -> Option<&str> {
        self.attrs(n)
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, n: NodeId, name: &str, value: &str) {
        let name = name.to_ascii_lowercase();
        if let NodeKind::Element { attrs, .. } = &mut self.nm(n).kind {
            match attrs.iter_mut().find(|(k, _)| *k == name) {
                Some(a) => a.1 = value.to_string(),
                None => attrs.push((name, value.to_string())),
            }
            self.generation += 1;
        }
    }

    pub fn remove_attr(&mut self, n: NodeId, name: &str) {
        if let NodeKind::Element { attrs, .. } = &mut self.nm(n).kind {
            let before = attrs.len();
            attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
            if attrs.len() != before {
                self.generation += 1;
            }
        }
    }

    pub fn attrs(&self, n: NodeId) -> &[(String, String)] {
        match &self.n(n).kind {
            NodeKind::Element { attrs, .. } => attrs,
            _ => &[],
        }
    }

    pub fn id(&self, n: NodeId) -> Option<&str> {
        self.attr(n, "id")
    }

    pub fn has_class(&self, n: NodeId, class: &str) -> bool {
        self.classes(n).any(|c| c == class)
    }

    pub fn classes(&self, n: NodeId) -> impl Iterator<Item = &str> {
        self.attr(n, "class").unwrap_or("").split_ascii_whitespace()
    }

    pub fn text(&self, n: NodeId) -> Option<&str> {
        match &self.n(n).kind {
            NodeKind::Text(t) | NodeKind::Comment(t) => Some(t),
            _ => None,
        }
    }

    pub fn set_text(&mut self, n: NodeId, text: &str) {
        if let NodeKind::Text(t) | NodeKind::Comment(t) = &mut self.nm(n).kind {
            *t = text.to_string();
            self.generation += 1;
        }
    }

    pub fn text_content(&self, n: NodeId) -> String {
        if let NodeKind::Text(t) = self.kind(n) {
            return t.clone();
        }
        let mut out = String::new();
        for d in self.descendants(n) {
            if let NodeKind::Text(t) = self.kind(d) {
                out.push_str(t);
            }
        }
        out
    }

    fn clear_children(&mut self, n: NodeId) {
        while let Some(c) = self.first_child(n) {
            self.remove(c);
        }
    }

    pub fn set_text_content(&mut self, n: NodeId, text: &str) {
        match self.kind(n) {
            NodeKind::Text(_) | NodeKind::Comment(_) => self.set_text(n, text),
            _ => {
                self.clear_children(n);
                if !text.is_empty() {
                    let t = self.create_text(text);
                    self.append_child(n, t);
                }
                self.generation += 1;
            }
        }
    }

    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.descendants(self.root())
            .find(|&d| self.tag(d).is_some() && self.id(d) == Some(id))
    }

    pub fn elements_by_tag(&self, tag: &str) -> Vec<NodeId> {
        let any = tag == "*";
        self.descendants(self.root())
            .filter(|&d| match self.tag(d) {
                Some(t) => any || t.eq_ignore_ascii_case(tag),
                None => false,
            })
            .collect()
    }

    pub fn document_element(&self) -> Option<NodeId> {
        self.children(self.root()).find(|&c| self.tag(c).is_some())
    }

    fn html_child(&self, tag: &str) -> Option<NodeId> {
        let html = self.document_element()?;
        self.children(html).find(|&c| self.tag(c) == Some(tag))
    }
    pub fn head(&self) -> Option<NodeId> {
        self.html_child("head")
    }
    pub fn body(&self) -> Option<NodeId> {
        self.html_child("body")
    }

    pub fn inner_html(&self, n: NodeId) -> String {
        let mut out = String::new();
        let raw = self.tag(n).is_some_and(|t| RAW.contains(&t));
        for c in self.children(n) {
            self.write_node(c, raw, &mut out);
        }
        out
    }

    pub fn outer_html(&self, n: NodeId) -> String {
        let mut out = String::new();
        self.write_node(n, false, &mut out);
        out
    }

    fn write_node(&self, n: NodeId, raw_parent: bool, out: &mut String) {
        match self.kind(n) {
            NodeKind::Document => {
                for c in self.children(n) {
                    self.write_node(c, false, out);
                }
            }
            NodeKind::Text(t) => {
                if raw_parent {
                    out.push_str(t);
                } else {
                    escape(t, false, out);
                }
            }
            NodeKind::Comment(t) => {
                out.push_str("<!--");
                out.push_str(t);
                out.push_str("-->");
            }
            NodeKind::Element { tag, attrs } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    escape(v, true, out);
                    out.push('"');
                }
                out.push('>');
                if VOID.contains(&tag.as_str()) {
                    return;
                }
                let raw = RAW.contains(&tag.as_str());
                for c in self.children(n) {
                    self.write_node(c, raw, out);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        }
    }

    /// Replace the children of `n` with the fragment `html` parsed in the
    /// context of `n`.
    pub fn set_inner_html(&mut self, n: NodeId, html: &str) {
        let ctx = self.tag(n).unwrap_or("body").to_string();
        let frag = parse_fragment_dom(html, &ctx);
        self.clear_children(n);
        if let Some(html_root) = frag.first_child(frag.root()) {
            let kids: Vec<NodeId> = frag.children(html_root).collect();
            for k in kids {
                let c = self.import(&frag, k);
                self.append_child(n, c);
            }
        }
        self.generation += 1;
    }

    fn import(&mut self, src: &Dom, s: NodeId) -> NodeId {
        let id = self.alloc(src.kind(s).clone());
        for c in src.children(s) {
            let ic = self.import(src, c);
            self.append_child(id, ic);
        }
        id
    }
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// TreeSink over the arena. Names live beside the arena because
/// `elem_name` must hand out a borrowed `QualName`.
struct Sink {
    dom: RefCell<Dom>,
    names: RefCell<Vec<Option<QualName>>>,
    templates: RefCell<HashMap<NodeId, NodeId>>,
}

impl Sink {
    fn new() -> Sink {
        Sink {
            dom: RefCell::new(Dom::new()),
            names: RefCell::new(vec![None]),
            templates: RefCell::new(HashMap::new()),
        }
    }

    fn sync(&self, id: NodeId, name: Option<QualName>) {
        let mut names = self.names.borrow_mut();
        names.resize(id.0 as usize + 1, None);
        names[id.0 as usize] = name;
    }

    fn append_text(&self, parent: NodeId, before: Option<NodeId>, text: &StrTendril) {
        let mut dom = self.dom.borrow_mut();
        let prev = match before {
            Some(b) => dom.prev_sibling(b),
            None => dom.last_child(parent),
        };
        if let Some(p) = prev {
            if let NodeKind::Text(t) = &mut dom.nm(p).kind {
                t.push_str(text);
                return;
            }
        }
        let t = dom.create_text(text);
        dom.insert_before(parent, t, before);
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Dom;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Dom {
        let mut dom = self.dom.into_inner();
        for (tpl, contents) in self.templates.into_inner() {
            let kids: Vec<NodeId> = dom.children(contents).collect();
            for k in kids {
                dom.append_child(tpl, k);
            }
        }
        dom.generation = 0;
        dom
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        NodeId(0)
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
        Ref::map(self.names.borrow(), |v| v[target.0 as usize].as_ref().expect("element"))
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let id = {
            let mut dom = self.dom.borrow_mut();
            let id = dom.create_element(&name.local);
            if let NodeKind::Element { attrs: a, .. } = &mut dom.nm(id).kind {
                for at in attrs {
                    a.push((at.name.local.to_string(), at.value.to_string()));
                }
            }
            id
        };
        self.sync(id, Some(name));
        if flags.template {
            let c = self.dom.borrow_mut().alloc(NodeKind::Document);
            self.sync(c, None);
            self.templates.borrow_mut().insert(id, c);
        }
        id
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        let id = self.dom.borrow_mut().create_comment(&text);
        self.sync(id, None);
        id
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> NodeId {
        self.create_comment(format!("?{target} {data}").into())
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(c) => self.dom.borrow_mut().append_child(*parent, c),
            NodeOrText::AppendText(t) => self.append_text(*parent, None, &t),
        }
    }

    fn append_based_on_parent_node(&self, element: &NodeId, prev_element: &NodeId, child: NodeOrText<NodeId>) {
        if self.dom.borrow().parent(*element).is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _name: StrTendril, _public_id: StrTendril, _system_id: StrTendril) {}

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        self.templates.borrow()[target]
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        let Some(parent) = self.dom.borrow().parent(*sibling) else { return };
        match new_node {
            NodeOrText::AppendNode(c) => self.dom.borrow_mut().insert_before(parent, c, Some(*sibling)),
            NodeOrText::AppendText(t) => self.append_text(parent, Some(*sibling), &t),
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let mut dom = self.dom.borrow_mut();
        for at in attrs {
            let name = at.name.local.to_string();
            if dom.attr(*target, &name).is_none() {
                dom.set_attr(*target, &name, &at.value);
            }
        }
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.dom.borrow_mut().remove(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut dom = self.dom.borrow_mut();
        let kids: Vec<NodeId> = dom.children(*node).collect();
        for k in kids {
            dom.append_child(*new_parent, k);
        }
    }
}

pub fn parse_html(src: &str) -> Dom {
    parse_document(Sink::new(), ParseOpts::default()).one(src)
}

fn parse_fragment_dom(src: &str, context_tag: &str) -> Dom {
    let ctx = QualName::new(None, ns!(html), LocalName::from(context_tag));
    parse_fragment(Sink::new(), ParseOpts::default(), ctx, Vec::new(), false).one(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(d: &Dom, n: NodeId) -> Vec<String> {
        d.children(n).filter_map(|c| d.tag(c).map(str::to_string)).collect()
    }

    #[test]
    fn new_dom_has_only_document() {
        let d = Dom::new();
        assert_eq!(d.root(), NodeId(0));
        assert_eq!(*d.kind(d.root()), NodeKind::Document);
        assert!(d.first_child(d.root()).is_none());
        assert_eq!(d.generation(), 0);
    }

    #[test]
    fn implied_html_head_body() {
        let d = parse_html("<p>hi");
        let html = d.document_element().unwrap();
        assert_eq!(d.tag(html), Some("html"));
        assert_eq!(tags(&d, html), ["head", "body"]);
        let body = d.body().unwrap();
        assert_eq!(tags(&d, body), ["p"]);
        assert_eq!(d.text_content(body), "hi");
        assert!(d.head().is_some());
    }

    #[test]
    fn full_document_and_doctype() {
        let d = parse_html("<!DOCTYPE html><html><head><title>T</title></head><body><h1 id=a>X</h1></body></html>");
        assert_eq!(d.elements_by_tag("title").len(), 1);
        let h = d.get_element_by_id("a").unwrap();
        assert_eq!(d.tag(h), Some("h1"));
        assert_eq!(d.parent(h), d.body());
    }

    #[test]
    fn attributes_and_classes() {
        let d = parse_html(r#"<div id="x" class="  a  b c " data-k='v&amp;w'></div>"#);
        let n = d.get_element_by_id("x").unwrap();
        assert_eq!(d.attr(n, "data-k"), Some("v&w"));
        assert_eq!(d.attr(n, "DATA-K"), Some("v&w"));
        assert_eq!(d.classes(n).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert!(d.has_class(n, "b"));
        assert!(!d.has_class(n, "d"));
        assert_eq!(d.id(n), Some("x"));
        assert_eq!(d.attrs(n).len(), 3);
    }

    #[test]
    fn uppercase_tags_lowercased() {
        let d = parse_html("<DIV ID=q><SPAN>x</SPAN></DIV>");
        let n = d.get_element_by_id("q").unwrap();
        assert_eq!(d.tag(n), Some("div"));
    }

    #[test]
    fn entities_decoded() {
        let d = parse_html("<p>a &lt; b &amp; c &copy; &#65;&#x42;&nbsp;</p>");
        let p = d.elements_by_tag("p")[0];
        assert_eq!(d.text_content(p), "a < b & c \u{a9} AB\u{a0}");
    }

    #[test]
    fn adjacent_text_merges() {
        let d = parse_html("<p>a&amp;b</p>");
        let p = d.elements_by_tag("p")[0];
        assert_eq!(d.children(p).count(), 1);
    }

    #[test]
    fn comments_kept() {
        let d = parse_html("<body><!-- hi --><b>x</b></body>");
        let body = d.body().unwrap();
        let c = d.first_child(body).unwrap();
        assert_eq!(*d.kind(c), NodeKind::Comment(" hi ".into()));
        assert_eq!(d.text(c), Some(" hi "));
        assert_eq!(d.text_content(body), "x");
    }

    #[test]
    fn script_and_style_text_raw() {
        let d = parse_html("<script>if (a < b && c) {}</script><style>p > a { }</style>");
        let s = d.elements_by_tag("script")[0];
        assert_eq!(d.text_content(s), "if (a < b && c) {}");
        let st = d.elements_by_tag("style")[0];
        assert_eq!(d.text_content(st), "p > a { }");
        assert_eq!(d.outer_html(s), "<script>if (a < b && c) {}</script>");
    }

    #[test]
    fn table_gets_tbody_and_foster_parenting() {
        let d = parse_html("<table><tr><td>1<td>2</table>");
        let t = d.elements_by_tag("table")[0];
        assert_eq!(tags(&d, t), ["tbody"]);
        assert_eq!(d.elements_by_tag("td").len(), 2);
        let d = parse_html("<table>oops<tr><td>x</td></tr></table>");
        let body = d.body().unwrap();
        assert_eq!(d.text(d.first_child(body).unwrap()), Some("oops"));
    }

    #[test]
    fn malformed_recovery() {
        let d = parse_html("<p>one<p>two<b>bold<i>both</b>it</i>");
        assert_eq!(d.elements_by_tag("p").len(), 2);
        let d = parse_html("<div><span>unclosed<div>x");
        assert_eq!(d.text_content(d.body().unwrap()), "unclosedx");
        let d = parse_html("<ul><li>a<li>b</ul>");
        assert_eq!(d.elements_by_tag("li").len(), 2);
    }

    #[test]
    fn void_elements_have_no_children() {
        let d = parse_html("<p>a<br>b<img src=x>c</p>");
        let p = d.elements_by_tag("p")[0];
        assert_eq!(tags(&d, p), ["br", "img"]);
        assert_eq!(d.inner_html(p), "a<br>b<img src=\"x\">c");
    }

    #[test]
    fn template_contents_become_children() {
        let d = parse_html("<template><p>x</p></template>");
        let t = d.elements_by_tag("template")[0];
        assert_eq!(tags(&d, t), ["p"]);
    }

    #[test]
    fn navigation_and_descendants() {
        let d = parse_html("<body><a><b></b></a><c></c></body>");
        let body = d.body().unwrap();
        let names: Vec<_> = d.descendants(body).filter_map(|n| d.tag(n)).collect();
        assert_eq!(names, ["a", "b", "c"]);
        let a = d.first_child(body).unwrap();
        let c = d.last_child(body).unwrap();
        assert_eq!(d.next_sibling(a), Some(c));
        assert_eq!(d.prev_sibling(c), Some(a));
        assert_eq!(d.prev_sibling(a), None);
        assert_eq!(d.descendants(c).count(), 0);
    }

    #[test]
    fn mutation_and_generation() {
        let mut d = parse_html("<body><p>x</p></body>");
        assert_eq!(d.generation(), 0);
        let body = d.body().unwrap();
        let g = d.generation();
        let e = d.create_element("DIV");
        assert_eq!(d.tag(e), Some("div"));
        d.append_child(body, e);
        assert!(d.generation() > g);
        assert_eq!(d.last_child(body), Some(e));
        let g = d.generation();
        d.set_attr(e, "Class", "k");
        assert!(d.generation() > g);
        assert!(d.has_class(e, "k"));
        d.set_attr(e, "class", "z");
        assert_eq!(d.attrs(e).len(), 1);
        let g = d.generation();
        d.remove_attr(e, "class");
        assert!(d.generation() > g);
        assert_eq!(d.attr(e, "class"), None);
        let g = d.generation();
        d.remove(e);
        assert!(d.generation() > g);
        assert_eq!(d.parent(e), None);
        assert_eq!(d.get_element_by_id("none"), None);
        assert_eq!(d.last_child(body), d.first_child(body));
    }

    #[test]
    fn append_moves_and_insert_before_orders() {
        let mut d = Dom::new();
        let r = d.root();
        let a = d.create_element("a");
        let b = d.create_element("b");
        let c = d.create_element("c");
        d.append_child(r, a);
        d.append_child(r, c);
        d.insert_before(r, b, Some(c));
        assert_eq!(tags(&d, r), ["a", "b", "c"]);
        d.append_child(r, a);
        assert_eq!(tags(&d, r), ["b", "c", "a"]);
        d.insert_before(r, a, Some(b));
        assert_eq!(tags(&d, r), ["a", "b", "c"]);
        d.append_child(b, c);
        assert_eq!(tags(&d, r), ["a", "b"]);
        // cycle is refused
        d.append_child(c, b);
        assert_eq!(d.parent(b), Some(r));
    }

    #[test]
    fn detached_nodes_not_found() {
        let mut d = parse_html("<p id=q>x</p>");
        let p = d.get_element_by_id("q").unwrap();
        d.remove(p);
        assert_eq!(d.get_element_by_id("q"), None);
        assert_eq!(d.id(p), Some("q"));
    }

    #[test]
    fn text_helpers() {
        let mut d = parse_html("<div id=x>a<b>b</b>c</div>");
        let x = d.get_element_by_id("x").unwrap();
        assert_eq!(d.text_content(x), "abc");
        d.set_text_content(x, "new <b>");
        assert_eq!(d.children(x).count(), 1);
        assert_eq!(d.inner_html(x), "new &lt;b&gt;");
        let t = d.first_child(x).unwrap();
        d.set_text(t, "z");
        assert_eq!(d.text(t), Some("z"));
        d.set_text_content(x, "");
        assert_eq!(d.children(x).count(), 0);
    }

    #[test]
    fn serialization_escaping() {
        let d = parse_html("<p title=\"a&quot;b&amp;c<d>\">1 &lt; 2 &amp; 3 &gt; 0&nbsp;!</p>");
        let p = d.elements_by_tag("p")[0];
        assert_eq!(
            d.outer_html(p),
            "<p title=\"a&quot;b&amp;c<d>\">1 &lt; 2 &amp; 3 &gt; 0&nbsp;!</p>"
        );
        assert_eq!(d.inner_html(p), "1 &lt; 2 &amp; 3 &gt; 0&nbsp;!");
    }

    #[test]
    fn serialization_comment() {
        let d = parse_html("<body><!--c--><i>x</i></body>");
        assert_eq!(d.inner_html(d.body().unwrap()), "<!--c--><i>x</i>");
    }

    #[test]
    fn round_trip() {
        let src = "<div class=\"a\"><p>x &amp; y</p><br><ul><li>1</li><li>2</li></ul></div>";
        let d = parse_html(src);
        let body = d.body().unwrap();
        let once = d.inner_html(body);
        assert_eq!(once, src);
        let d2 = parse_html(&once);
        assert_eq!(d2.inner_html(d2.body().unwrap()), once);
    }

    #[test]
    fn set_inner_html_replaces_children() {
        let mut d = parse_html("<div id=x><i>old</i></div>");
        let x = d.get_element_by_id("x").unwrap();
        let g = d.generation();
        d.set_inner_html(x, "a<b class=k>b</b><p>c");
        assert!(d.generation() > g);
        assert_eq!(d.inner_html(x), "a<b class=\"k\">b</b><p>c</p>");
        assert_eq!(tags(&d, x), ["b", "p"]);
        let b = d.elements_by_tag("b")[0];
        assert_eq!(d.parent(b), Some(x));
        assert!(d.elements_by_tag("i").is_empty());
    }

    #[test]
    fn fragment_context_matters() {
        let mut d = parse_html("<table id=t></table><select id=s></select>");
        let t = d.get_element_by_id("t").unwrap();
        d.set_inner_html(t, "<tr><td>1</td></tr>");
        assert_eq!(tags(&d, t), ["tbody"]);
        let s = d.get_element_by_id("s").unwrap();
        d.set_inner_html(s, "<option>a<option>b");
        assert_eq!(d.children(s).count(), 2);
    }

    #[test]
    fn fragment_script_text_and_empty() {
        let mut d = parse_html("<div id=x>keep</div>");
        let x = d.get_element_by_id("x").unwrap();
        d.set_inner_html(x, "<script>a<b</script>");
        assert_eq!(d.inner_html(x), "<script>a<b</script>");
        d.set_inner_html(x, "");
        assert_eq!(d.children(x).count(), 0);
    }
}
