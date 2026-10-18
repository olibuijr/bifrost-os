//! XML documents and path queries (std only).
//!
//! [`Document::parse`] reads well-formed XML into a node tree: elements with attributes, text (character data and
//! CDATA sections, with the five named entities and numeric references decoded) and nothing else; comments,
//! processing instructions and the document type declaration are skipped. A document has exactly one root element.
//! Namespaces are not resolved: a prefix stays part of the name, and a name test without a prefix also matches the
//! local part of a prefixed name.
//!
//! [`Document::query`] evaluates a path expression over it. The supported language is the path subset of XPath that
//! XQuery builds on:
//!
//! - steps `name`, `*`, `@name`, `@*`, `text()`, `node()`, `.` and `..`, joined by `/` and `//`, absolute (`/a/b`) or
//!   relative to the root element;
//! - predicates `[n]`, `[last()]`, `[@a]`, `[@a='v']`, `[child]`, `[child>3]`, `[.='v']`, `[text()='v']`, `and`, `or`,
//!   parentheses and the comparisons `= != < <= > >=`;
//! - the functions `count`, `string`, `exists`, `not`, `contains`, `starts-with`, `name`, `position`, `last`, `true`
//!   and `false`.
//!
//! Arithmetic, `for`/`let`/`where`/`return` expressions, element constructors, axes written with `::` and user
//! functions are not supported and are refused with a [`QueryError`].
//!
//! Parsing and querying are iterative where a document's depth could exhaust the stack, nesting is limited to
//! [`MAX_DEPTH`] and the tree to [`MAX_NODES`], so hostile input fails cleanly.

use std::cmp::Ordering;
use std::fmt;

/// Deepest element nesting accepted.
pub const MAX_DEPTH: usize = 256;
/// Most nodes (elements and text runs) in one document.
pub const MAX_NODES: usize = 1_000_000;
/// Deepest nesting of parentheses and predicates in a query.
pub const MAX_QUERY_DEPTH: usize = 64;

/// Why a document could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XmlError {
    Syntax { offset: usize, message: String },
    TooDeep,
    TooLarge,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmlError::Syntax { offset, message } => write!(f, "xml error at byte {offset}: {message}"),
            XmlError::TooDeep => write!(f, "xml is nested too deeply"),
            XmlError::TooLarge => write!(f, "xml has too many nodes"),
        }
    }
}

impl std::error::Error for XmlError {}

/// Why a query could not be read or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError(pub String);

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query error: {}", self.0)
    }
}

impl std::error::Error for QueryError {}

fn qerr<T>(m: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError(m.into()))
}

fn xerr<T>(offset: usize, m: &str) -> Result<T, XmlError> {
    Err(XmlError::Syntax { offset, message: m.to_string() })
}

// ---- document -------------------------------------------------------------------------------------------------

/// A node's address inside its document.
pub type NodeId = usize;

#[derive(Clone, Debug)]
enum Kind {
    Root,
    Element { name: String, attrs: Vec<(String, String)> },
    Text(String),
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

/// A parsed XML document.
#[derive(Clone, Debug)]
pub struct Document {
    nodes: Vec<Node>,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn decode(s: &str, base: usize) -> Result<String, XmlError> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let Some(end) = after.find(';').filter(|e| *e <= 10) else { return xerr(base + s.len() - rest.len() + p, "an & must start a reference ending in ;") };
        let name = &after[..end];
        let ch = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => return xerr(base + s.len() - rest.len() + p, "unknown reference"),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

impl Document {
    /// Reads `src` as XML.
    pub fn parse(src: &str) -> Result<Document, XmlError> {
        let b = src.as_bytes();
        let mut doc = Document { nodes: vec![Node { kind: Kind::Root, parent: None, children: Vec::new() }] };
        let mut stack: Vec<NodeId> = vec![0];
        let mut seen_root = false;
        let mut i = 0usize;
        while i < b.len() {
            if b[i] != b'<' {
                let end = src[i..].find('<').map_or(b.len(), |p| i + p);
                let raw = &src[i..end];
                if stack.len() == 1 {
                    if !raw.bytes().all(is_space) {
                        return xerr(i, "text outside the root element");
                    }
                } else {
                    let text = decode(raw, i)?;
                    doc.add(*stack.last().unwrap_or(&0), Kind::Text(text))?;
                }
                i = end;
                continue;
            }
            let rest = &src[i..];
            if rest.starts_with("<!--") {
                let Some(p) = rest[4..].find("-->") else { return xerr(i, "comment is not closed") };
                i += 4 + p + 3;
            } else if rest.starts_with("<![CDATA[") {
                let Some(p) = rest[9..].find("]]>") else { return xerr(i, "CDATA section is not closed") };
                if stack.len() == 1 {
                    return xerr(i, "CDATA outside the root element");
                }
                let text = rest[9..9 + p].to_string();
                doc.add(*stack.last().unwrap_or(&0), Kind::Text(text))?;
                i += 9 + p + 3;
            } else if rest.starts_with("<?") {
                let Some(p) = rest[2..].find("?>") else { return xerr(i, "processing instruction is not closed") };
                i += 2 + p + 2;
            } else if rest.starts_with("<!") {
                // Document type declaration: skip to the closing > outside any [ ... ] subset.
                let mut depth = 0i32;
                let mut j = i + 2;
                loop {
                    let Some(&c) = b.get(j) else { return xerr(i, "declaration is not closed") };
                    match c {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
            } else if rest.starts_with("</") {
                let Some(p) = rest.find('>') else { return xerr(i, "end tag is not closed") };
                let name = rest[2..p].trim();
                let top = *stack.last().unwrap_or(&0);
                match &doc.nodes[top].kind {
                    Kind::Element { name: open, .. } if stack.len() > 1 && open == name => {}
                    _ => return xerr(i, "end tag does not match the open element"),
                }
                stack.pop();
                i += p + 1;
            } else {
                i = doc.start_tag(src, i, &mut stack, &mut seen_root)?;
            }
        }
        if stack.len() != 1 {
            return xerr(b.len(), "an element is not closed");
        }
        if !seen_root {
            return xerr(0, "no root element");
        }
        Ok(doc)
    }

    fn add(&mut self, parent: NodeId, kind: Kind) -> Result<NodeId, XmlError> {
        if self.nodes.len() >= MAX_NODES {
            return Err(XmlError::TooLarge);
        }
        let id = self.nodes.len();
        self.nodes.push(Node { kind, parent: Some(parent), children: Vec::new() });
        self.nodes[parent].children.push(id);
        Ok(id)
    }

    /// Reads a start tag at `i` and returns the offset after it.
    fn start_tag(&mut self, src: &str, start: usize, stack: &mut Vec<NodeId>, seen_root: &mut bool) -> Result<usize, XmlError> {
        let b = src.as_bytes();
        let name_end = |from: usize| -> usize {
            let mut j = from;
            while j < b.len() && !is_space(b[j]) && !matches!(b[j], b'/' | b'>' | b'=' | b'<' | b'"' | b'\'') {
                j += 1;
            }
            j
        };
        let mut i = start + 1;
        let ne = name_end(i);
        if ne == i {
            return xerr(start, "element has no name");
        }
        let name = src[i..ne].to_string();
        i = ne;
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut self_closing = false;
        loop {
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            match b.get(i) {
                None => return xerr(start, "start tag is not closed"),
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(b'/') if b.get(i + 1) == Some(&b'>') => {
                    self_closing = true;
                    i += 2;
                    break;
                }
                Some(_) => {}
            }
            let ae = name_end(i);
            if ae == i {
                return xerr(i, "attribute has no name");
            }
            let aname = src[i..ae].to_string();
            i = ae;
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                return xerr(i, "attribute needs = and a quoted value");
            }
            i += 1;
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            let Some(&q) = b.get(i) else { return xerr(i, "attribute value is missing") };
            if q != b'"' && q != b'\'' {
                return xerr(i, "attribute value must be quoted");
            }
            let Some(len) = src[i + 1..].find(q as char) else { return xerr(i, "attribute value is not closed") };
            let value = decode(&src[i + 1..i + 1 + len], i + 1)?;
            if attrs.iter().any(|(n, _)| *n == aname) {
                return xerr(i, "attribute is given twice");
            }
            attrs.push((aname, value));
            i += len + 2;
        }
        if stack.len() == 1 {
            if *seen_root {
                return xerr(start, "a document has one root element");
            }
            *seen_root = true;
        }
        let id = self.add(*stack.last().unwrap_or(&0), Kind::Element { name, attrs })?;
        if !self_closing {
            if stack.len() > MAX_DEPTH {
                return Err(XmlError::TooDeep);
            }
            stack.push(id);
        }
        Ok(i)
    }

    /// The root element.
    pub fn root_element(&self) -> NodeId {
        self.nodes[0].children.iter().copied().find(|c| matches!(self.nodes[*c].kind, Kind::Element { .. })).unwrap_or(0)
    }

    /// The element's name (with any prefix), or `None` for text and the document node.
    pub fn name(&self, id: NodeId) -> Option<&str> {
        match &self.nodes.get(id)?.kind {
            Kind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    /// An attribute's value.
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes.get(id)?.kind {
            Kind::Element { attrs, .. } => attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    /// The child nodes of `id`, in order.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id).map_or(&[][..], |n| n.children.as_slice())
    }

    /// The text of a node: a text node's own text, otherwise the text of everything under it in order.
    pub fn text(&self, id: NodeId) -> String {
        let mut out = String::new();
        let mut todo = vec![id];
        while let Some(n) = todo.pop() {
            match self.nodes.get(n).map(|x| &x.kind) {
                Some(Kind::Text(t)) => out.push_str(t),
                Some(_) => todo.extend(self.nodes[n].children.iter().rev()),
                None => {}
            }
        }
        out
    }

    // ---- queries ----------------------------------------------------------------------------------------------

    /// Evaluates `expr`.
    pub fn query(&self, expr: &str) -> Result<Value, QueryError> {
        let e = QueryParser::new(expr).parse_all()?;
        let ctx = Ctx { item: Item::Node(self.root_element()), pos: 1, size: 1 };
        self.eval(&e, &ctx)
    }

    /// The text of every node or attribute the path selects, in document order.
    pub fn select_text(&self, expr: &str) -> Result<Vec<String>, QueryError> {
        match self.query(expr)? {
            Value::Nodes(items) => Ok(items.iter().map(|i| self.item_text(*i)).collect()),
            _ => qerr("the expression does not select nodes"),
        }
    }

    /// How many nodes the path selects.
    pub fn count(&self, expr: &str) -> Result<usize, QueryError> {
        match self.query(expr)? {
            Value::Nodes(items) => Ok(items.len()),
            _ => qerr("the expression does not select nodes"),
        }
    }

    /// The text of one selected item.
    pub fn item_text(&self, item: Item) -> String {
        match item {
            Item::Node(n) => self.text(n),
            Item::Attr(n, k) => match &self.nodes[n].kind {
                Kind::Element { attrs, .. } => attrs.get(k).map(|a| a.1.clone()).unwrap_or_default(),
                _ => String::new(),
            },
        }
    }

    fn item_name(&self, item: Item) -> String {
        match item {
            Item::Node(n) => self.name(n).unwrap_or("").to_string(),
            Item::Attr(n, k) => match &self.nodes[n].kind {
                Kind::Element { attrs, .. } => attrs.get(k).map(|a| a.0.clone()).unwrap_or_default(),
                _ => String::new(),
            },
        }
    }

    fn axis(&self, axis: Axis, item: Item) -> Vec<Item> {
        match (axis, item) {
            (Axis::Self_, i) => vec![i],
            (Axis::Child, Item::Node(n)) => self.nodes[n].children.iter().map(|c| Item::Node(*c)).collect(),
            (Axis::Child, Item::Attr(..)) => Vec::new(),
            (Axis::Parent, Item::Node(n)) => self.nodes[n].parent.map(Item::Node).into_iter().collect(),
            (Axis::Parent, Item::Attr(n, _)) => vec![Item::Node(n)],
            (Axis::Attribute, Item::Node(n)) => match &self.nodes[n].kind {
                Kind::Element { attrs, .. } => (0..attrs.len()).map(|k| Item::Attr(n, k)).collect(),
                _ => Vec::new(),
            },
            (Axis::Attribute, Item::Attr(..)) => Vec::new(),
            (Axis::DescendantOrSelf, Item::Attr(n, k)) => vec![Item::Attr(n, k)],
            (Axis::DescendantOrSelf, Item::Node(n)) => {
                let mut out = Vec::new();
                let mut todo = vec![n];
                while let Some(x) = todo.pop() {
                    out.push(Item::Node(x));
                    todo.extend(self.nodes[x].children.iter().rev());
                }
                out
            }
        }
    }

    fn matches(&self, axis: Axis, test: &Test, item: Item) -> bool {
        let same = |want: &str, have: &str| have == want || have.rsplit(':').next() == Some(want);
        match (test, item) {
            (Test::Node, _) => true,
            (Test::Name(w), Item::Node(n)) => axis != Axis::Attribute && self.name(n).is_some_and(|h| same(w, h)),
            (Test::Name(w), Item::Attr(..)) => axis == Axis::Attribute && same(w, &self.item_name(item)),
            (Test::Any, Item::Node(n)) => axis != Axis::Attribute && self.name(n).is_some(),
            (Test::Any, Item::Attr(..)) => axis == Axis::Attribute,
            (Test::Text, Item::Node(n)) => matches!(self.nodes[n].kind, Kind::Text(_)),
            (Test::Text, Item::Attr(..)) => false,
        }
    }

    fn eval_path(&self, p: &Path, ctx: &Ctx) -> Result<Vec<Item>, QueryError> {
        let mut cur = vec![if p.absolute { Item::Node(0) } else { ctx.item }];
        for s in &p.steps {
            let mut next: Vec<Item> = Vec::new();
            for item in &cur {
                let mut cand: Vec<Item> = self.axis(s.axis, *item).into_iter().filter(|c| self.matches(s.axis, &s.test, *c)).collect();
                for pred in &s.preds {
                    let size = cand.len();
                    let mut kept = Vec::new();
                    for (k, c) in cand.iter().enumerate() {
                        let v = self.eval(pred, &Ctx { item: *c, pos: k + 1, size })?;
                        if self.keep(&v, k + 1) {
                            kept.push(*c);
                        }
                    }
                    cand = kept;
                }
                next.extend(cand);
            }
            next.sort();
            next.dedup();
            cur = next;
        }
        Ok(cur)
    }

    /// A predicate keeps an item when its value is true; a bare number is a position.
    fn keep(&self, v: &Value, pos: usize) -> bool {
        match v {
            Value::Nodes(n) => !n.is_empty(),
            Value::Number(x) => *x == pos as f64,
            Value::Text(s) => !s.is_empty(),
            Value::Bool(b) => *b,
        }
    }

    fn truth(&self, v: &Value) -> bool {
        match v {
            Value::Nodes(n) => !n.is_empty(),
            Value::Number(x) => *x != 0.0,
            Value::Text(s) => !s.is_empty(),
            Value::Bool(b) => *b,
        }
    }

    fn string_of(&self, v: &Value) -> String {
        match v {
            Value::Nodes(n) => n.first().map(|i| self.item_text(*i)).unwrap_or_default(),
            Value::Number(x) => {
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    format!("{}", *x as i64)
                } else {
                    x.to_string()
                }
            }
            Value::Text(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
        }
    }

    fn atoms(&self, v: &Value) -> Vec<Atom> {
        match v {
            Value::Nodes(n) => n.iter().map(|i| Atom::Str(self.item_text(*i))).collect(),
            Value::Number(x) => vec![Atom::Num(*x)],
            Value::Text(s) => vec![Atom::Str(s.clone())],
            Value::Bool(b) => vec![Atom::Num(if *b { 1.0 } else { 0.0 })],
        }
    }

    fn compare(&self, a: &Value, op: CmpOp, b: &Value) -> bool {
        let (la, lb) = (self.atoms(a), self.atoms(b));
        la.iter().any(|x| lb.iter().any(|y| atom_cmp(x, op, y)))
    }

    fn eval(&self, e: &Expr, ctx: &Ctx) -> Result<Value, QueryError> {
        Ok(match e {
            Expr::Or(a, b) => Value::Bool(self.truth_at(a, ctx)? || self.truth_at(b, ctx)?),
            Expr::And(a, b) => Value::Bool(self.truth_at(a, ctx)? && self.truth_at(b, ctx)?),
            Expr::Cmp(a, op, b) => {
                let (va, vb) = (self.eval(a, ctx)?, self.eval(b, ctx)?);
                Value::Bool(self.compare(&va, *op, &vb))
            }
            Expr::Path(p) => Value::Nodes(self.eval_path(p, ctx)?),
            Expr::Str(s) => Value::Text(s.clone()),
            Expr::Num(x) => Value::Number(*x),
            Expr::Call(name, args) => self.call(name, args, ctx)?,
        })
    }

    /// Truth of a sub-expression inside `and`, `or` and `not`: a number is a position test there too.
    fn truth_at(&self, e: &Expr, ctx: &Ctx) -> Result<bool, QueryError> {
        let v = self.eval(e, ctx)?;
        Ok(match v {
            Value::Number(x) => x == ctx.pos as f64,
            other => self.truth(&other),
        })
    }

    fn call(&self, name: &str, args: &[Expr], ctx: &Ctx) -> Result<Value, QueryError> {
        let want = |n: usize| -> Result<(), QueryError> {
            if args.len() == n {
                Ok(())
            } else {
                qerr(format!("{name}() takes {n} argument(s)"))
            }
        };
        Ok(match name {
            "true" => {
                want(0)?;
                Value::Bool(true)
            }
            "false" => {
                want(0)?;
                Value::Bool(false)
            }
            "last" => {
                want(0)?;
                Value::Number(ctx.size as f64)
            }
            "position" => {
                want(0)?;
                Value::Number(ctx.pos as f64)
            }
            "count" => {
                want(1)?;
                match self.eval(&args[0], ctx)? {
                    Value::Nodes(n) => Value::Number(n.len() as f64),
                    _ => return qerr("count() needs a path"),
                }
            }
            "exists" => {
                want(1)?;
                match self.eval(&args[0], ctx)? {
                    Value::Nodes(n) => Value::Bool(!n.is_empty()),
                    _ => return qerr("exists() needs a path"),
                }
            }
            "not" => {
                want(1)?;
                Value::Bool(!self.truth_at(&args[0], ctx)?)
            }
            "string" => match args.len() {
                0 => Value::Text(self.item_text(ctx.item)),
                1 => Value::Text(self.string_of(&self.eval(&args[0], ctx)?)),
                _ => return qerr("string() takes at most one argument"),
            },
            "name" => match args.len() {
                0 => Value::Text(self.item_name(ctx.item)),
                1 => match self.eval(&args[0], ctx)? {
                    Value::Nodes(n) => Value::Text(n.first().map(|i| self.item_name(*i)).unwrap_or_default()),
                    _ => return qerr("name() needs a path"),
                },
                _ => return qerr("name() takes at most one argument"),
            },
            "contains" | "starts-with" => {
                want(2)?;
                let a = self.string_of(&self.eval(&args[0], ctx)?);
                let b = self.string_of(&self.eval(&args[1], ctx)?);
                Value::Bool(if name == "contains" { a.contains(&b) } else { a.starts_with(&b) })
            }
            other => return qerr(format!("unknown function {other}()")),
        })
    }
}

/// What a query returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nodes(Vec<Item>),
    Number(f64),
    Text(String),
    Bool(bool),
}

/// One selected node or attribute. Items order by position in the document, an element before its attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Node(NodeId),
    /// An element's attribute: the element and the attribute's index.
    Attr(NodeId, usize),
}

impl Item {
    fn key(self) -> (NodeId, usize) {
        match self {
            Item::Node(n) => (n, 0),
            Item::Attr(n, k) => (n, k + 1),
        }
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

enum Atom {
    Str(String),
    Num(f64),
}

fn atom_cmp(a: &Atom, op: CmpOp, b: &Atom) -> bool {
    let num = |x: &Atom| match x {
        Atom::Num(n) => Some(*n),
        Atom::Str(s) => s.trim().parse::<f64>().ok(),
    };
    if matches!(a, Atom::Num(_)) || matches!(b, Atom::Num(_)) || !matches!(op, CmpOp::Eq | CmpOp::Ne) {
        return match (num(a), num(b)) {
            (Some(x), Some(y)) => match op {
                CmpOp::Eq => x == y,
                CmpOp::Ne => x != y,
                CmpOp::Lt => x < y,
                CmpOp::Le => x <= y,
                CmpOp::Gt => x > y,
                CmpOp::Ge => x >= y,
            },
            _ => op == CmpOp::Ne,
        };
    }
    let (Atom::Str(x), Atom::Str(y)) = (a, b) else { return false };
    if op == CmpOp::Eq {
        x == y
    } else {
        x != y
    }
}

// ---- query syntax ---------------------------------------------------------------------------------------------

struct Ctx {
    item: Item,
    pos: usize,
    size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Child,
    DescendantOrSelf,
    Self_,
    Parent,
    Attribute,
}

#[derive(Clone, Debug, PartialEq)]
enum Test {
    Name(String),
    Any,
    Text,
    Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
struct Step {
    axis: Axis,
    test: Test,
    preds: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
struct Path {
    absolute: bool,
    steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
    Path(Path),
    Str(String),
    Num(f64),
    Call(String, Vec<Expr>),
}

struct QueryParser {
    c: Vec<char>,
    i: usize,
}

fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

impl QueryParser {
    fn new(s: &str) -> QueryParser {
        QueryParser { c: s.chars().collect(), i: 0 }
    }

    fn ws(&mut self) {
        while self.i < self.c.len() && self.c[self.i].is_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn at(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, ch)| self.c.get(self.i + k) == Some(&ch))
    }

    fn word(&mut self, w: &str) -> bool {
        self.ws();
        let n = w.chars().count();
        if self.at(w) && !self.c.get(self.i + n).is_some_and(|c| name_char(*c)) {
            self.i += n;
            true
        } else {
            false
        }
    }

    fn parse_all(mut self) -> Result<Expr, QueryError> {
        let e = self.expr(0)?;
        self.ws();
        if self.i < self.c.len() {
            return qerr(format!("unexpected {:?} at {}", self.c[self.i], self.i));
        }
        Ok(e)
    }

    fn expr(&mut self, depth: usize) -> Result<Expr, QueryError> {
        if depth > MAX_QUERY_DEPTH {
            return qerr("expression is nested too deeply");
        }
        let mut left = self.and(depth)?;
        while self.word("or") {
            let right = self.and(depth)?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let mut left = self.compare(depth)?;
        while self.word("and") {
            let right = self.compare(depth)?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn compare(&mut self, depth: usize) -> Result<Expr, QueryError> {
        let left = self.primary(depth)?;
        self.ws();
        let op = if self.at("!=") {
            self.i += 2;
            CmpOp::Ne
        } else if self.at("<=") {
            self.i += 2;
            CmpOp::Le
        } else if self.at(">=") {
            self.i += 2;
            CmpOp::Ge
        } else if self.at("=") {
            self.i += 1;
            CmpOp::Eq
        } else if self.at("<") {
            self.i += 1;
            CmpOp::Lt
        } else if self.at(">") {
            self.i += 1;
            CmpOp::Gt
        } else {
            return Ok(left);
        };
        let right = self.primary(depth)?;
        Ok(Expr::Cmp(Box::new(left), op, Box::new(right)))
    }

    fn primary(&mut self, depth: usize) -> Result<Expr, QueryError> {
        self.ws();
        let Some(ch) = self.peek() else { return qerr("the expression ends too soon") };
        match ch {
            '(' => {
                self.i += 1;
                let e = self.expr(depth + 1)?;
                self.ws();
                if self.peek() != Some(')') {
                    return qerr("expected )");
                }
                self.i += 1;
                Ok(e)
            }
            '\'' | '"' => {
                self.i += 1;
                let start = self.i;
                while self.i < self.c.len() && self.c[self.i] != ch {
                    self.i += 1;
                }
                if self.i >= self.c.len() {
                    return qerr("text is not closed");
                }
                let s: String = self.c[start..self.i].iter().collect();
                self.i += 1;
                Ok(Expr::Str(s))
            }
            c if c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.c.len() && (self.c[self.i].is_ascii_digit() || self.c[self.i] == '.') {
                    self.i += 1;
                }
                let s: String = self.c[start..self.i].iter().collect();
                match s.parse::<f64>() {
                    Ok(x) => Ok(Expr::Num(x)),
                    Err(_) => qerr(format!("{s} is not a number")),
                }
            }
            c if name_start(c) => {
                // A function call is a name followed by ( that is not a node test.
                let mut j = self.i;
                while j < self.c.len() && name_char(self.c[j]) {
                    j += 1;
                }
                let name: String = self.c[self.i..j].iter().collect();
                if self.c.get(j) == Some(&'(') && name != "text" && name != "node" {
                    self.i = j + 1;
                    let mut args = Vec::new();
                    self.ws();
                    if self.peek() == Some(')') {
                        self.i += 1;
                    } else {
                        loop {
                            args.push(self.expr(depth + 1)?);
                            self.ws();
                            match self.peek() {
                                Some(',') => self.i += 1,
                                Some(')') => {
                                    self.i += 1;
                                    break;
                                }
                                _ => return qerr("expected , or )"),
                            }
                        }
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Path(self.path(depth)?))
                }
            }
            _ => Ok(Expr::Path(self.path(depth)?)),
        }
    }

    fn path(&mut self, depth: usize) -> Result<Path, QueryError> {
        let mut steps = Vec::new();
        let mut absolute = false;
        let dos = Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: Vec::new() };
        if self.at("//") {
            absolute = true;
            self.i += 2;
            steps.push(dos.clone());
        } else if self.at("/") {
            absolute = true;
            self.i += 1;
            // A lone "/" is the document node.
            match self.peek() {
                Some(c) if name_start(c) || matches!(c, '*' | '@' | '.') => {}
                _ => return Ok(Path { absolute, steps }),
            }
        }
        loop {
            steps.push(self.step(depth)?);
            if self.at("//") {
                self.i += 2;
                steps.push(dos.clone());
            } else if self.at("/") {
                self.i += 1;
            } else {
                break;
            }
        }
        Ok(Path { absolute, steps })
    }

    fn step(&mut self, depth: usize) -> Result<Step, QueryError> {
        let (axis, test) = if self.at("..") {
            self.i += 2;
            (Axis::Parent, Test::Node)
        } else if self.at(".") {
            self.i += 1;
            (Axis::Self_, Test::Node)
        } else if self.at("@") {
            self.i += 1;
            if self.at("*") {
                self.i += 1;
                (Axis::Attribute, Test::Any)
            } else {
                (Axis::Attribute, Test::Name(self.name()?))
            }
        } else if self.at("*") {
            self.i += 1;
            (Axis::Child, Test::Any)
        } else {
            let n = self.name()?;
            if self.at("::") {
                return qerr("axes written with :: are not supported");
            }
            if self.at("()") && (n == "text" || n == "node") {
                self.i += 2;
                (Axis::Child, if n == "text" { Test::Text } else { Test::Node })
            } else {
                (Axis::Child, Test::Name(n))
            }
        };
        let mut preds = Vec::new();
        while self.peek() == Some('[') {
            self.i += 1;
            preds.push(self.expr(depth + 1)?);
            self.ws();
            if self.peek() != Some(']') {
                return qerr("expected ]");
            }
            self.i += 1;
        }
        Ok(Step { axis, test, preds })
    }

    fn name(&mut self) -> Result<String, QueryError> {
        match self.peek() {
            Some(c) if name_start(c) => {}
            _ => return qerr(format!("expected a name at {}", self.i)),
        }
        let start = self.i;
        // A name stops before `::` (an axis, which is refused) but keeps a single `:` of a prefix.
        while self.i < self.c.len() && name_char(self.c[self.i]) && !self.at("::") {
            self.i += 1;
        }
        Ok(self.c[start..self.i].iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#"<?xml version="1.0"?>
<!DOCTYPE library [ <!ENTITY x "y"> ]>
<!-- the shelves -->
<library name="City &amp; Co">
  <book id="1" lang="en"><title>Alpha</title><price>12.5</price></book>
  <book id="2" lang="is"><title>Beta &lt;2&gt;</title><price>7</price></book>
  <book id="3" lang="en"><title><![CDATA[Gamma <raw>]]></title><price>30</price><tag/></book>
  <dc:note xmlns:dc="urn:x">hello</dc:note>
</library>"#;

    fn doc() -> Document {
        Document::parse(LIBRARY).unwrap()
    }

    #[test]
    fn parses_elements_attributes_text_and_entities() {
        let d = doc();
        let root = d.root_element();
        assert_eq!(d.name(root), Some("library"));
        assert_eq!(d.attr(root, "name"), Some("City & Co"));
        assert_eq!(d.select_text("/library/book[2]/title").unwrap(), ["Beta <2>"]);
        assert_eq!(d.select_text("/library/book[3]/title").unwrap(), ["Gamma <raw>"]);
    }

    #[test]
    fn character_references_decode() {
        let d = Document::parse("<a>&#65;&#x42;&quot;&apos;</a>").unwrap();
        assert_eq!(d.text(d.root_element()), "AB\"'");
    }

    #[test]
    fn child_descendant_and_attribute_paths() {
        let d = doc();
        assert_eq!(d.count("/library/book").unwrap(), 3);
        assert_eq!(d.count("//price").unwrap(), 3);
        assert_eq!(d.select_text("//book/@id").unwrap(), ["1", "2", "3"]);
        assert_eq!(d.select_text("//@lang").unwrap(), ["en", "is", "en"]);
        assert_eq!(d.count("/library/*").unwrap(), 4);
        assert_eq!(d.count("//@*").unwrap(), 8);
    }

    #[test]
    fn predicates_filter_by_position_attribute_and_child() {
        let d = doc();
        assert_eq!(d.select_text("//book[1]/title").unwrap(), ["Alpha"]);
        assert_eq!(d.select_text("//book[last()]/@id").unwrap(), ["3"]);
        assert_eq!(d.select_text("//book[@lang='en']/@id").unwrap(), ["1", "3"]);
        assert_eq!(d.select_text("//book[price>10]/@id").unwrap(), ["1", "3"]);
        assert_eq!(d.select_text("//book[price<=12.5 and @lang='en']/title").unwrap(), ["Alpha"]);
        assert_eq!(d.select_text("//book[tag]/@id").unwrap(), ["3"]);
        assert_eq!(d.select_text("//book[not(tag)]/@id").unwrap(), ["1", "2"]);
        assert_eq!(d.select_text("//book[title='Alpha' or @id=2]/@id").unwrap(), ["1", "2"]);
        assert_eq!(d.select_text("//title[contains(., 'amm')]").unwrap(), ["Gamma <raw>"]);
        assert_eq!(d.select_text("//book[position()<3]/@id").unwrap(), ["1", "2"]);
    }

    #[test]
    fn parent_self_text_and_prefixed_names() {
        let d = doc();
        assert_eq!(d.select_text("//price[.='7']/../@id").unwrap(), ["2"]);
        assert_eq!(d.select_text("//title/text()").unwrap().len(), 3);
        assert_eq!(d.select_text("//note").unwrap(), ["hello"]);
        assert_eq!(d.select_text("//dc:note").unwrap(), ["hello"]);
    }

    #[test]
    fn functions_return_values() {
        let d = doc();
        assert_eq!(d.query("count(//book)").unwrap(), Value::Number(3.0));
        assert_eq!(d.query("exists(//nothing)").unwrap(), Value::Bool(false));
        assert_eq!(d.query("string(//book[2]/price)").unwrap(), Value::Text("7".into()));
        assert_eq!(d.query("name(/library/*[1])").unwrap(), Value::Text("book".into()));
        assert_eq!(d.query("starts-with('abc', 'ab')").unwrap(), Value::Bool(true));
        assert_eq!(d.query("count(//book[@lang='en']) = 2").unwrap(), Value::Bool(true));
    }

    #[test]
    fn malformed_xml_is_refused() {
        for bad in ["", "<a>", "<a></b>", "<a/><b/>", "text", "<a b=1/>", "<a b='1' b='2'/>", "<a>&nope;</a>", "<a><!-- x </a>", "<a x='1/>"] {
            assert!(Document::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unsupported_queries_are_refused() {
        let d = doc();
        for bad in ["for $b in //book return $b", "child::book", "//book[", "//book[1 + 1]", "frobnicate(1)", "", "//", "count('x')"] {
            assert!(d.query(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn deep_documents_and_queries_fail_cleanly() {
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 10), "</a>".repeat(MAX_DEPTH + 10));
        assert_eq!(Document::parse(&deep).unwrap_err(), XmlError::TooDeep);
        let ok_depth = 200;
        let okay = format!("{}x{}", "<a>".repeat(ok_depth), "</a>".repeat(ok_depth));
        let d = Document::parse(&okay).unwrap();
        assert_eq!(d.count("//a").unwrap(), ok_depth);
        assert_eq!(d.text(d.root_element()), "x");
        let q = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        assert!(d.query(&q).is_err());
    }
}
