//! Markup documents (`NativeMarkupDocument`, `NativeMarkupOrderedSet`, `NativeMarkupID`, the element kinds and
//! `NativeMarkupError`): an ordered set of vector elements with stable ids, undo and redo, and a text form that saves
//! and reloads. This is the pure data model. Drawing, pointer input and the markup toolbar belong to the shell and apps.
use crate::Rect;

/// How many edits undo keeps. Redo can never hold more steps than undo did.
const HISTORY_LIMIT: usize = 100;
/// First line of a saved document, followed by the next free id.
const FORMAT: &str = "bifrost-markup 1";

/// Opaque element id (`NativeMarkupID`). Ids are never reused inside one document, so an id still names the same
/// element after undo, redo and a reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MarkupId(u32);

impl MarkupId {
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// The kinds of shape a `ShapeMarkup` can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
    /// A text box: its words are `ShapeMarkup::text`.
    TextBox,
}

impl ShapeKind {
    fn name(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "rectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Line => "line",
            ShapeKind::TextBox => "text",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "rectangle" => Some(ShapeKind::Rectangle),
            "ellipse" => Some(ShapeKind::Ellipse),
            "line" => Some(ShapeKind::Line),
            "text" => Some(ShapeKind::TextBox),
            _ => None,
        }
    }
}

/// A shape or text box (`NativeShapeMarkup`). Colours are packed 0xAARRGGBB; `fill` of `None` is no fill.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeMarkup {
    pub kind: ShapeKind,
    pub frame: Rect,
    pub stroke: u32,
    pub fill: Option<u32>,
    pub line_width: f32,
    /// The words of a text box; empty for the other shapes.
    pub text: String,
}

/// A picture placed on the canvas (`NativeImageMarkup`). `source` names the asset to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageMarkup {
    pub frame: Rect,
    pub source: String,
}

/// A tappable link on the canvas (`NativeLinkMarkup`).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkMarkup {
    pub frame: Rect,
    pub url: String,
}

/// A loupe that magnifies the content under it (`NativeLoupeMarkup`). `magnification` is at least 1.
#[derive(Clone, Debug, PartialEq)]
pub struct LoupeMarkup {
    pub frame: Rect,
    pub magnification: f32,
}

/// One element of a markup document (`NativeMarkup`).
#[derive(Clone, Debug, PartialEq)]
pub enum MarkupElement {
    Shape(ShapeMarkup),
    Image(ImageMarkup),
    Link(LinkMarkup),
    Loupe(LoupeMarkup),
}

impl MarkupElement {
    pub fn frame(&self) -> Rect {
        match self {
            MarkupElement::Shape(s) => s.frame,
            MarkupElement::Image(i) => i.frame,
            MarkupElement::Link(l) => l.frame,
            MarkupElement::Loupe(l) => l.frame,
        }
    }

    /// The name of the feature that allows this element, as used in `MarkupError::FeatureDisabled`.
    pub fn feature_name(&self) -> &'static str {
        match self {
            MarkupElement::Shape(s) if s.kind == ShapeKind::TextBox => "text boxes",
            MarkupElement::Shape(_) => "shapes",
            MarkupElement::Image(_) => "images",
            MarkupElement::Link(_) => "links",
            MarkupElement::Loupe(_) => "loupes",
        }
    }
}

/// The element kinds a canvas accepts (`FeatureSet`). Strokes are not elements, so drawing is not part of this set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureSet(u8);

impl FeatureSet {
    pub const NONE: FeatureSet = FeatureSet(0);
    pub const SHAPES: FeatureSet = FeatureSet(1);
    pub const TEXT_BOXES: FeatureSet = FeatureSet(2);
    pub const IMAGES: FeatureSet = FeatureSet(4);
    pub const LINKS: FeatureSet = FeatureSet(8);
    pub const LOUPES: FeatureSet = FeatureSet(16);
    pub const ALL: FeatureSet = FeatureSet(31);

    pub fn union(self, other: FeatureSet) -> FeatureSet {
        FeatureSet(self.0 | other.0)
    }

    /// True when every feature in `other` is also in this set.
    pub fn contains(self, other: FeatureSet) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when the set allows an element of this kind.
    pub fn permits(self, element: &MarkupElement) -> bool {
        self.contains(element_feature(element))
    }
}

fn element_feature(element: &MarkupElement) -> FeatureSet {
    match element {
        MarkupElement::Shape(s) if s.kind == ShapeKind::TextBox => FeatureSet::TEXT_BOXES,
        MarkupElement::Shape(_) => FeatureSet::SHAPES,
        MarkupElement::Image(_) => FeatureSet::IMAGES,
        MarkupElement::Link(_) => FeatureSet::LINKS,
        MarkupElement::Loupe(_) => FeatureSet::LOUPES,
    }
}

/// Items of the insertion menu (`MarkupEditViewController`, `MarkupToolbarViewController`): what a person can add to a
/// canvas, in menu order (see [`Insertion::MENU`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insertion {
    Rectangle,
    Ellipse,
    Line,
    TextBox,
    Image,
    Link,
    Loupe,
}

/// Default ink for inserted shapes: opaque black, two points wide. Text boxes are one point wide.
const DEFAULT_STROKE: u32 = 0xff00_0000;
const DEFAULT_LINE_WIDTH: f32 = 2.0;
const DEFAULT_TEXT_LINE_WIDTH: f32 = 1.0;
/// Default magnification of an inserted loupe.
const DEFAULT_MAGNIFICATION: f32 = 2.0;

impl Insertion {
    pub const MENU: [Insertion; 7] = [
        Insertion::Rectangle,
        Insertion::Ellipse,
        Insertion::Line,
        Insertion::TextBox,
        Insertion::Image,
        Insertion::Link,
        Insertion::Loupe,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Insertion::Rectangle => "Rectangle",
            Insertion::Ellipse => "Ellipse",
            Insertion::Line => "Line",
            Insertion::TextBox => "Text Box",
            Insertion::Image => "Image",
            Insertion::Link => "Link",
            Insertion::Loupe => "Loupe",
        }
    }

    /// The feature a menu item needs before it can be inserted.
    pub fn feature(self) -> FeatureSet {
        match self {
            Insertion::Rectangle | Insertion::Ellipse | Insertion::Line => FeatureSet::SHAPES,
            Insertion::TextBox => FeatureSet::TEXT_BOXES,
            Insertion::Image => FeatureSet::IMAGES,
            Insertion::Link => FeatureSet::LINKS,
            Insertion::Loupe => FeatureSet::LOUPES,
        }
    }

    /// The element a menu item inserts with its top left corner at `origin`, at its default size. `payload` is the
    /// words of a text box, the asset name of an image or the URL of a link; the other items ignore it. The value is
    /// not checked here: `MarkupDocument::insert` refuses an empty image source or link URL.
    pub fn element(self, origin: (f32, f32), payload: &str) -> MarkupElement {
        let (x, y) = origin;
        let shape = |kind: ShapeKind, w: f32, h: f32, fill: Option<u32>, line_width: f32, text: String| {
            MarkupElement::Shape(ShapeMarkup {
                kind,
                frame: Rect { x, y, w, h },
                stroke: DEFAULT_STROKE,
                fill,
                line_width,
                text,
            })
        };
        match self {
            Insertion::Rectangle => shape(ShapeKind::Rectangle, 120.0, 80.0, None, DEFAULT_LINE_WIDTH, String::new()),
            Insertion::Ellipse => shape(ShapeKind::Ellipse, 80.0, 80.0, None, DEFAULT_LINE_WIDTH, String::new()),
            Insertion::Line => shape(ShapeKind::Line, 120.0, 0.0, None, DEFAULT_LINE_WIDTH, String::new()),
            Insertion::TextBox => {
                shape(ShapeKind::TextBox, 160.0, 40.0, Some(0xffff_ffff), DEFAULT_TEXT_LINE_WIDTH, payload.to_string())
            }
            Insertion::Image => MarkupElement::Image(ImageMarkup {
                frame: Rect { x, y, w: 160.0, h: 120.0 },
                source: payload.to_string(),
            }),
            Insertion::Link => MarkupElement::Link(LinkMarkup {
                frame: Rect { x, y, w: 120.0, h: 20.0 },
                url: payload.to_string(),
            }),
            Insertion::Loupe => MarkupElement::Loupe(LoupeMarkup {
                frame: Rect { x, y, w: 120.0, h: 120.0 },
                magnification: DEFAULT_MAGNIFICATION,
            }),
        }
    }
}

/// Automatic sizing of a canvas (`MarkupAutoresizing`). A flexible axis grows to reach the far edge of the content, so
/// the canvas stays large enough to show every element. A canvas never shrinks below its requested size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Autoresizing {
    pub flexible_width: bool,
    pub flexible_height: bool,
}

impl Autoresizing {
    pub const FIXED: Autoresizing = Autoresizing { flexible_width: false, flexible_height: false };

    /// The canvas size for `width` by `height` once the content of `doc` is laid out.
    pub fn fitted_size(self, doc: &MarkupDocument, width: f32, height: f32) -> (f32, f32) {
        let (content_w, content_h) = doc.content_extent();
        let w = if self.flexible_width { width.max(content_w) } else { width };
        let h = if self.flexible_height { height.max(content_h) } else { height };
        (w, h)
    }
}

/// Errors from editing and from reading a saved document (`NativeMarkupError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupError {
    UnknownId(MarkupId),
    /// The frame has a non-finite value or a negative size.
    InvalidFrame,
    /// A field that cannot hold this value (named by the field).
    InvalidValue(&'static str),
    /// The canvas's feature set does not allow this kind of element (named by the kind, see `MarkupElement::feature_name`).
    FeatureDisabled(&'static str),
    /// A saved document that cannot be read. `line` is 1-based.
    Decode { line: usize, reason: &'static str },
}

impl std::fmt::Display for MarkupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkupError::UnknownId(id) => write!(f, "no markup element with id {}", id.0),
            MarkupError::InvalidFrame => write!(f, "frame must be finite with a non-negative size"),
            MarkupError::InvalidValue(field) => write!(f, "invalid {field}"),
            MarkupError::FeatureDisabled(kind) => write!(f, "the feature set does not allow {kind}"),
            MarkupError::Decode { line, reason } => write!(f, "line {line}: {reason}"),
        }
    }
}

impl std::error::Error for MarkupError {}

type Elements = Vec<(MarkupId, MarkupElement)>;

/// A markup document (`NativeMarkupDocument`): elements in stacking order (index 0 is the back), each with a stable id,
/// plus undo and redo over the edits made in this session. Saving keeps the elements and ids, not the history.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkupDocument {
    elements: Elements,
    /// The id the next insert receives. It only grows, so ids of removed elements are never handed out again.
    next: u32,
    undo: Vec<Elements>,
    redo: Vec<Elements>,
}

impl Default for MarkupDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkupDocument {
    pub fn new() -> Self {
        MarkupDocument { elements: Vec::new(), next: 1, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Elements from back to front, with their ids.
    pub fn iter(&self) -> impl Iterator<Item = (MarkupId, &MarkupElement)> + '_ {
        self.elements.iter().map(|(id, el)| (*id, el))
    }

    pub fn get(&self, id: MarkupId) -> Option<&MarkupElement> {
        self.elements.iter().find(|(i, _)| *i == id).map(|(_, el)| el)
    }

    /// Stacking position of an element, 0 being the back.
    pub fn index_of(&self, id: MarkupId) -> Option<usize> {
        self.elements.iter().position(|(i, _)| *i == id)
    }

    /// The far corner of all elements (right and bottom edges): the smallest canvas that holds the content from the
    /// origin. Zero for an empty document.
    pub fn content_extent(&self) -> (f32, f32) {
        self.elements.iter().fold((0.0_f32, 0.0_f32), |(w, h), (_, el)| {
            let f = el.frame();
            (w.max(f.x + f.w), h.max(f.y + f.h))
        })
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Adds an element on top of the stack and returns its new id.
    pub fn insert(&mut self, element: MarkupElement) -> Result<MarkupId, MarkupError> {
        check(&element)?;
        let id = MarkupId(self.next);
        self.next += 1;
        self.edit();
        self.elements.push((id, element));
        Ok(id)
    }

    /// Takes an element out of the document. Its id stays retired, so undo brings the same element back under the
    /// same id.
    pub fn remove(&mut self, id: MarkupId) -> Result<MarkupElement, MarkupError> {
        let at = self.index_of(id).ok_or(MarkupError::UnknownId(id))?;
        self.edit();
        Ok(self.elements.remove(at).1)
    }

    /// Replaces an element's content and keeps its place in the stack.
    pub fn replace(&mut self, id: MarkupId, element: MarkupElement) -> Result<(), MarkupError> {
        check(&element)?;
        let at = self.index_of(id).ok_or(MarkupError::UnknownId(id))?;
        self.edit();
        self.elements[at].1 = element;
        Ok(())
    }

    /// Moves an element to stacking position `index`; positions past the top clamp to the top. Moving an element to
    /// the place it already holds is not an edit.
    pub fn reorder(&mut self, id: MarkupId, index: usize) -> Result<(), MarkupError> {
        let from = self.index_of(id).ok_or(MarkupError::UnknownId(id))?;
        let to = index.min(self.elements.len() - 1);
        if from == to {
            return Ok(());
        }
        self.edit();
        let item = self.elements.remove(from);
        self.elements.insert(to, item);
        Ok(())
    }

    /// Reverses the last edit. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(before) => {
                self.redo.push(std::mem::replace(&mut self.elements, before));
                true
            }
            None => false,
        }
    }

    /// Repeats the last undone edit. Returns false when there is nothing to redo. A new edit clears redo.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(after) => {
                self.undo.push(std::mem::replace(&mut self.elements, after));
                true
            }
            None => false,
        }
    }

    /// Records the state before an edit. Call it only once the edit is known to succeed.
    fn edit(&mut self) {
        self.undo.push(self.elements.clone());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The saved text: a header with the next free id, then one line per element, back to front.
    /// Fields are tab-separated; text fields escape backslash, tab, newline and carriage return.
    pub fn encode(&self) -> String {
        let mut out = format!("{FORMAT} {}\n", self.next);
        for (id, el) in &self.elements {
            let fields: Vec<String> = match el {
                MarkupElement::Shape(s) => vec![
                    "shape".into(),
                    s.kind.name().into(),
                    rect_fields(s.frame),
                    format!("{:08x}", s.stroke),
                    s.fill.map_or("-".into(), |c| format!("{c:08x}")),
                    format!("{}", s.line_width),
                    escape(&s.text),
                ],
                MarkupElement::Image(i) => vec!["image".into(), rect_fields(i.frame), escape(&i.source)],
                MarkupElement::Link(l) => vec!["link".into(), rect_fields(l.frame), escape(&l.url)],
                MarkupElement::Loupe(l) => {
                    vec!["loupe".into(), rect_fields(l.frame), format!("{}", l.magnification)]
                }
            };
            out.push_str(&format!("{}\t{}\n", id.0, fields.join("\t")));
        }
        out
    }

    /// Reads a document saved by `encode`. The result has no undo history. Every element is checked the same way
    /// an edit is, and ids must be unique and below the next free id.
    pub fn decode(text: &str) -> Result<Self, MarkupError> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("");
        let next = header
            .strip_prefix(FORMAT)
            .and_then(|rest| rest.trim().parse::<u32>().ok())
            .filter(|n| *n >= 1)
            .ok_or(MarkupError::Decode { line: 1, reason: "not a markup document" })?;
        let mut elements: Elements = Vec::new();
        for (n, line) in lines.enumerate() {
            let line_no = n + 2;
            let fail = |reason| MarkupError::Decode { line: line_no, reason };
            let mut parts = line.split('\t');
            let id = parts
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .ok_or_else(|| fail("bad element id"))?;
            let kind = parts.next().ok_or_else(|| fail("missing element kind"))?;
            let fields: Vec<&str> = parts.collect();
            let element = parse_element(kind, &fields).map_err(|reason| fail(reason))?;
            check(&element).map_err(|_| fail("invalid element"))?;
            if id >= next {
                return Err(fail("id beyond the next free id"));
            }
            if elements.iter().any(|(i, _)| i.0 == id) {
                return Err(fail("duplicate element id"));
            }
            elements.push((MarkupId(id), element));
        }
        Ok(MarkupDocument { elements, next, undo: Vec::new(), redo: Vec::new() })
    }
}

/// Rejects frames with non-finite values or negative sizes, and element fields that cannot hold their value.
fn check(element: &MarkupElement) -> Result<(), MarkupError> {
    let f = element.frame();
    if ![f.x, f.y, f.w, f.h].iter().all(|v| v.is_finite()) || f.w < 0.0 || f.h < 0.0 {
        return Err(MarkupError::InvalidFrame);
    }
    match element {
        MarkupElement::Shape(s) if !(s.line_width.is_finite() && s.line_width >= 0.0) => {
            Err(MarkupError::InvalidValue("line width"))
        }
        MarkupElement::Image(i) if i.source.is_empty() => Err(MarkupError::InvalidValue("image source")),
        MarkupElement::Link(l) if l.url.is_empty() => Err(MarkupError::InvalidValue("link url")),
        MarkupElement::Loupe(l) if !(l.magnification.is_finite() && l.magnification >= 1.0) => {
            Err(MarkupError::InvalidValue("magnification"))
        }
        _ => Ok(()),
    }
}

fn rect_fields(r: Rect) -> String {
    format!("{}\t{}\t{}\t{}", r.x, r.y, r.w, r.h)
}

fn parse_element(kind: &str, f: &[&str]) -> Result<MarkupElement, &'static str> {
    match (kind, f.len()) {
        ("shape", 9) => Ok(MarkupElement::Shape(ShapeMarkup {
            kind: ShapeKind::from_name(f[0]).ok_or("unknown shape kind")?,
            frame: parse_rect(&f[1..5])?,
            stroke: u32::from_str_radix(f[5], 16).map_err(|_| "bad stroke colour")?,
            fill: match f[6] {
                "-" => None,
                hex => Some(u32::from_str_radix(hex, 16).map_err(|_| "bad fill colour")?),
            },
            line_width: parse_number(f[7])?,
            text: unescape(f[8])?,
        })),
        ("image", 5) => Ok(MarkupElement::Image(ImageMarkup { frame: parse_rect(&f[0..4])?, source: unescape(f[4])? })),
        ("link", 5) => Ok(MarkupElement::Link(LinkMarkup { frame: parse_rect(&f[0..4])?, url: unescape(f[4])? })),
        ("loupe", 5) => Ok(MarkupElement::Loupe(LoupeMarkup {
            frame: parse_rect(&f[0..4])?,
            magnification: parse_number(f[4])?,
        })),
        ("shape" | "image" | "link" | "loupe", _) => Err("wrong number of fields"),
        _ => Err("unknown element kind"),
    }
}

fn parse_number(s: &str) -> Result<f32, &'static str> {
    s.parse::<f32>().map_err(|_| "bad number")
}

fn parse_rect(f: &[&str]) -> Result<Rect, &'static str> {
    Ok(Rect { x: parse_number(f[0])?, y: parse_number(f[1])?, w: parse_number(f[2])?, h: parse_number(f[3])? })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> Result<String, &'static str> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('\\') => '\\',
            Some('t') => '\t',
            Some('n') => '\n',
            Some('r') => '\r',
            _ => return Err("bad escape in text"),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn rectangle(x: f32) -> MarkupElement {
        MarkupElement::Shape(ShapeMarkup {
            kind: ShapeKind::Rectangle,
            frame: rect(x, 0.0, 10.0, 10.0),
            stroke: 0xff00_00ff,
            fill: None,
            line_width: 2.0,
            text: String::new(),
        })
    }

    fn text_box(words: &str) -> MarkupElement {
        MarkupElement::Shape(ShapeMarkup {
            kind: ShapeKind::TextBox,
            frame: rect(4.0, 6.0, 120.0, 40.0),
            stroke: 0xff00_0000,
            fill: Some(0x80ff_ffff),
            line_width: 1.0,
            text: words.into(),
        })
    }

    fn order(doc: &MarkupDocument) -> Vec<u32> {
        doc.iter().map(|(id, _)| id.raw()).collect()
    }

    #[test]
    fn ids_are_never_reused_and_undo_brings_an_element_back_under_its_id() {
        let mut doc = MarkupDocument::new();
        let a = doc.insert(rectangle(0.0)).unwrap();
        let b = doc.insert(rectangle(20.0)).unwrap();
        assert_eq!((a.raw(), b.raw()), (1, 2));
        let removed = doc.remove(b).unwrap();
        assert_eq!(removed, rectangle(20.0));
        let c = doc.insert(rectangle(40.0)).unwrap();
        assert_eq!(c.raw(), 3, "a removed id stays retired");
        assert!(doc.undo(), "undo the insert of c");
        assert!(doc.undo(), "undo the removal of b");
        assert_eq!(order(&doc), vec![1, 2], "b is back in its place under the same id");
        assert_eq!(doc.get(b), Some(&rectangle(20.0)));
    }

    #[test]
    fn undo_and_redo_walk_the_history_and_a_new_edit_clears_redo() {
        let mut doc = MarkupDocument::new();
        let a = doc.insert(rectangle(0.0)).unwrap();
        doc.insert(rectangle(20.0)).unwrap();
        assert!(doc.undo());
        assert!(doc.can_redo());
        assert_eq!(doc.len(), 1);
        assert!(doc.redo());
        assert_eq!(doc.len(), 2);
        assert!(doc.undo());
        doc.replace(a, rectangle(5.0)).unwrap();
        assert!(!doc.can_redo(), "an edit after undo drops the redo history");
        assert!(!doc.redo());
        assert_eq!(doc.get(a), Some(&rectangle(5.0)));
        assert!(doc.undo());
        assert_eq!(doc.get(a), Some(&rectangle(0.0)));
        assert!(doc.undo());
        assert!(doc.is_empty());
        assert!(!doc.undo(), "nothing left to undo");
    }

    #[test]
    fn reorder_moves_in_the_stack_clamps_at_the_top_and_ignores_no_op_moves() {
        let mut doc = MarkupDocument::new();
        let a = doc.insert(rectangle(0.0)).unwrap();
        doc.insert(rectangle(20.0)).unwrap();
        let c = doc.insert(rectangle(40.0)).unwrap();
        doc.reorder(a, 10).unwrap();
        assert_eq!(order(&doc), vec![2, 3, 1], "a goes to the top and b, c move down");
        assert_eq!(doc.index_of(a), Some(2));
        doc.reorder(c, 0).unwrap();
        assert_eq!(order(&doc), vec![3, 2, 1]);
        assert!(doc.undo(), "the move is one undo step");
        assert_eq!(order(&doc), vec![2, 3, 1]);
        assert!(doc.can_redo());
        doc.reorder(c, 1).unwrap();
        assert!(doc.can_redo(), "moving an element to the place it holds is not an edit");
        assert!(doc.redo());
        assert_eq!(order(&doc), vec![3, 2, 1]);
    }

    #[test]
    fn invalid_elements_and_unknown_ids_are_rejected_without_history() {
        let mut doc = MarkupDocument::new();
        let negative = MarkupElement::Image(ImageMarkup { frame: rect(0.0, 0.0, -1.0, 4.0), source: "shot".into() });
        assert_eq!(doc.insert(negative), Err(MarkupError::InvalidFrame));
        let nan = MarkupElement::Loupe(LoupeMarkup { frame: rect(f32::NAN, 0.0, 4.0, 4.0), magnification: 2.0 });
        assert_eq!(doc.insert(nan), Err(MarkupError::InvalidFrame));
        let no_url = MarkupElement::Link(LinkMarkup { frame: rect(0.0, 0.0, 4.0, 4.0), url: String::new() });
        assert_eq!(doc.insert(no_url), Err(MarkupError::InvalidValue("link url")));
        let shrink = MarkupElement::Loupe(LoupeMarkup { frame: rect(0.0, 0.0, 4.0, 4.0), magnification: 0.5 });
        assert_eq!(doc.insert(shrink), Err(MarkupError::InvalidValue("magnification")));
        assert!(!doc.can_undo(), "failed edits leave no history");
        assert_eq!(doc.remove(MarkupId(9)), Err(MarkupError::UnknownId(MarkupId(9))));
        assert_eq!(doc.reorder(MarkupId(9), 0), Err(MarkupError::UnknownId(MarkupId(9))));
    }

    #[test]
    fn save_and_reload_keep_ids_order_and_escaped_text() {
        let mut doc = MarkupDocument::new();
        let note = doc.insert(text_box("two\tlines\nand a \\ slash")).unwrap();
        doc.insert(MarkupElement::Link(LinkMarkup { frame: rect(1.5, 2.25, 30.0, 8.0), url: "https://example.org/a?b=c".into() })).unwrap();
        doc.insert(MarkupElement::Loupe(LoupeMarkup { frame: rect(0.1, 0.2, 50.0, 50.0), magnification: 2.5 })).unwrap();
        doc.insert(MarkupElement::Image(ImageMarkup { frame: rect(0.0, 0.0, 9.0, 9.0), source: "capture-1.png".into() })).unwrap();
        doc.remove(note).unwrap();
        doc.insert(text_box("again")).unwrap();

        let saved = doc.encode();
        let loaded = MarkupDocument::decode(&saved).unwrap();
        assert_eq!(loaded.len(), doc.len());
        assert_eq!(order(&loaded), order(&doc));
        assert_eq!(loaded.iter().map(|(_, el)| el.clone()).collect::<Vec<_>>(), doc.iter().map(|(_, el)| el.clone()).collect::<Vec<_>>());
        assert!(!loaded.can_undo(), "saving keeps the elements, not the history");
        let mut reloaded = loaded;
        let next = reloaded.insert(rectangle(0.0)).unwrap();
        assert_eq!(next.raw(), 6, "the next free id survives a save");
        assert_eq!(reloaded.encode().lines().next(), Some("bifrost-markup 1 7"));
    }

    #[test]
    fn decode_rejects_documents_it_cannot_trust() {
        let header = "bifrost-markup 1 5\n";
        assert_eq!(
            MarkupDocument::decode("bifrost-markup 2 5\n"),
            Err(MarkupError::Decode { line: 1, reason: "not a markup document" })
        );
        let duplicate = format!("{header}1\timage\t0\t0\t4\t4\tone\n1\timage\t0\t0\t4\t4\ttwo\n");
        assert_eq!(
            MarkupDocument::decode(&duplicate),
            Err(MarkupError::Decode { line: 3, reason: "duplicate element id" })
        );
        let beyond = format!("{header}7\timage\t0\t0\t4\t4\tone\n");
        assert_eq!(
            MarkupDocument::decode(&beyond),
            Err(MarkupError::Decode { line: 2, reason: "id beyond the next free id" })
        );
        let unknown = format!("{header}1\tpolygon\t0\t0\t4\t4\tone\n");
        assert_eq!(MarkupDocument::decode(&unknown), Err(MarkupError::Decode { line: 2, reason: "unknown element kind" }));
        let bad_escape = format!("{header}1\timage\t0\t0\t4\t4\tbad\\q\n");
        assert_eq!(MarkupDocument::decode(&bad_escape), Err(MarkupError::Decode { line: 2, reason: "bad escape in text" }));
        let short = format!("{header}1\tlink\t0\t0\t4\t4\n");
        assert_eq!(MarkupDocument::decode(&short), Err(MarkupError::Decode { line: 2, reason: "wrong number of fields" }));
    }

    #[test]
    fn feature_sets_combine_and_permit_elements_by_kind() {
        let shapes_and_images = FeatureSet::SHAPES.union(FeatureSet::IMAGES);
        assert!(shapes_and_images.contains(FeatureSet::IMAGES));
        assert!(!shapes_and_images.contains(FeatureSet::ALL), "ALL needs every bit");
        assert!(shapes_and_images.permits(&rectangle(0.0)));
        assert!(!shapes_and_images.permits(&text_box("note")), "a text box needs the text box feature, not shapes");
        assert!(FeatureSet::ALL.contains(FeatureSet::NONE));
        let loupe = MarkupElement::Loupe(LoupeMarkup { frame: rect(0.0, 0.0, 4.0, 4.0), magnification: 2.0 });
        assert!(!FeatureSet::NONE.permits(&loupe));
        assert_eq!(loupe.feature_name(), "loupes");
        assert_eq!(text_box("x").feature_name(), "text boxes");
    }

    #[test]
    fn insertion_menu_items_build_their_default_elements() {
        assert_eq!(Insertion::MENU.len(), 7);
        assert_eq!(Insertion::TextBox.feature(), FeatureSet::TEXT_BOXES);
        assert_eq!(Insertion::Ellipse.feature(), FeatureSet::SHAPES);
        let loupe = Insertion::Loupe.element((10.0, 20.0), "");
        assert_eq!(loupe, MarkupElement::Loupe(LoupeMarkup { frame: rect(10.0, 20.0, 120.0, 120.0), magnification: 2.0 }));
        let note = Insertion::TextBox.element((4.0, 6.0), "hello");
        assert_eq!(note.frame(), rect(4.0, 6.0, 160.0, 40.0));
        match note {
            MarkupElement::Shape(s) => assert_eq!((s.kind, s.text.as_str()), (ShapeKind::TextBox, "hello")),
            other => panic!("expected a shape, got {other:?}"),
        }
    }

    #[test]
    fn content_extent_is_the_far_corner_of_all_elements() {
        let mut doc = MarkupDocument::new();
        assert_eq!(doc.content_extent(), (0.0, 0.0));
        doc.insert(rectangle(250.0)).unwrap();
        doc.insert(MarkupElement::Image(ImageMarkup { frame: rect(0.0, 90.0, 40.0, 30.0), source: "a".into() })).unwrap();
        assert_eq!(doc.content_extent(), (260.0, 120.0), "right edge 250 + 10, bottom edge 90 + 30");
    }

    #[test]
    fn autoresizing_grows_flexible_axes_to_the_content_and_never_shrinks() {
        let mut doc = MarkupDocument::new();
        doc.insert(rectangle(250.0)).unwrap();
        let wide = Autoresizing { flexible_width: true, flexible_height: false };
        assert_eq!(wide.fitted_size(&doc, 300.0, 200.0), (300.0, 200.0), "content fits inside 300 wide");
        assert_eq!(wide.fitted_size(&doc, 200.0, 200.0), (260.0, 200.0), "width grows to the far edge");
        assert_eq!(wide.fitted_size(&doc, 500.0, 200.0), (500.0, 200.0), "a larger width is kept");
        assert_eq!(Autoresizing::FIXED.fitted_size(&doc, 200.0, 200.0), (200.0, 200.0));
        let both = Autoresizing { flexible_width: true, flexible_height: true };
        assert_eq!(both.fitted_size(&doc, 200.0, 4.0), (260.0, 10.0), "height grows to 10 as well");
    }

    #[test]
    fn insertion_items_are_checked_like_any_other_element() {
        let mut doc = MarkupDocument::new();
        assert_eq!(doc.insert(Insertion::Image.element((0.0, 0.0), "")), Err(MarkupError::InvalidValue("image source")));
        assert_eq!(doc.insert(Insertion::Link.element((0.0, 0.0), "")), Err(MarkupError::InvalidValue("link url")));
        assert!(doc.insert(Insertion::Image.element((0.0, 0.0), "shot.png")).is_ok());
        assert_eq!(doc.len(), 1, "refused items leave the document unchanged");
        assert_eq!(MarkupError::FeatureDisabled("links").to_string(), "the feature set does not allow links");
    }
}
