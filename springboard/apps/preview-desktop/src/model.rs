//! Viewer state that needs no drawing: zoom, quarter turns, page navigation, markup in page
//! coordinates, the sidecar file that keeps markup, and the burn-in used by export. Everything here
//! is plain data, so the tests run without a window.

use bifrost_app::kit::{pack, Image};
use std::rc::Rc;

/// The smallest and largest zoom factor the viewer goes to.
pub const ZOOM_MIN: f32 = 0.05;
pub const ZOOM_MAX: f32 = 8.0;
/// One Zoom In or Zoom Out step.
const ZOOM_STEP: f32 = 1.25;

/// How the picture is scaled into the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The whole page fits the canvas. Fit never enlarges a small picture past its own size.
    Fit,
    /// A fixed factor: 1.0 is actual size.
    Factor(f32),
}

impl Zoom {
    /// The scale factor for a `w` by `h` picture shown in a `view_w` by `view_h` canvas.
    pub fn scale(self, w: usize, h: usize, view_w: usize, view_h: usize) -> f32 {
        match self {
            Zoom::Factor(f) => f.clamp(ZOOM_MIN, ZOOM_MAX),
            Zoom::Fit => {
                if w == 0 || h == 0 || view_w == 0 || view_h == 0 {
                    return 1.0;
                }
                let s = (view_w as f32 / w as f32).min(view_h as f32 / h as f32);
                s.min(1.0).clamp(ZOOM_MIN, ZOOM_MAX)
            }
        }
    }

    /// The zoom one step in or out from the current scale.
    pub fn stepped(self, w: usize, h: usize, view_w: usize, view_h: usize, zoom_in: bool) -> Zoom {
        let current = self.scale(w, h, view_w, view_h);
        let next = if zoom_in { current * ZOOM_STEP } else { current / ZOOM_STEP };
        Zoom::Factor(next.clamp(ZOOM_MIN, ZOOM_MAX))
    }
}

/// Quarter turns clockwise, 0 to 3.
pub fn turn_size(w: usize, h: usize, quarters: u8) -> (usize, usize) {
    if quarters % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// The size of a `w` by `h` picture fitted inside a `max_w` by `max_h` box, aspect kept. A picture
/// already inside the box keeps its size; a zero side gives a box-sized result of at least one pixel.
pub fn fit_box(w: usize, h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    if w == 0 || h == 0 {
        return (max_w.max(1), max_h.max(1));
    }
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let s = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    let fw = ((w as f32 * s).round() as usize).clamp(1, max_w.max(1));
    let fh = ((h as f32 * s).round() as usize).clamp(1, max_h.max(1));
    (fw, fh)
}

/// Rotates `img` clockwise by `quarters` quarter turns (the count is taken modulo 4).
pub fn rotate(img: &Image, quarters: u8) -> Image {
    let q = quarters % 4;
    if q == 0 || img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let (w, h) = (img.w, img.h);
    let (ow, oh) = turn_size(w, h, q);
    let mut px = vec![0u32; ow * oh];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = match q {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            px[dy * ow + dx] = img.px[y * w + x];
        }
    }
    Image { w: ow, h: oh, px }
}

/// A rectangle in normalised page coordinates (0 to 1 across the unrotated page).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Norm {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Norm {
    /// The rectangle spanned by two corners, in any order.
    pub fn from_corners(a: (f32, f32), b: (f32, f32)) -> Norm {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Norm { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// Maps a point of the unrotated page to the same point after `quarters` clockwise turns.
pub fn rot_point(u: f32, v: f32, quarters: u8) -> (f32, f32) {
    match quarters % 4 {
        1 => (1.0 - v, u),
        2 => (1.0 - u, 1.0 - v),
        3 => (v, 1.0 - u),
        _ => (u, v),
    }
}

/// The inverse of `rot_point`: a point of the turned picture back on the unrotated page.
pub fn unrot_point(a: f32, b: f32, quarters: u8) -> (f32, f32) {
    match quarters % 4 {
        1 => (b, 1.0 - a),
        2 => (1.0 - a, 1.0 - b),
        3 => (1.0 - b, a),
        _ => (a, b),
    }
}

/// A source-page rectangle as it sits on the turned picture (its corners mapped, then re-spanned).
pub fn rot_rect(r: Norm, quarters: u8) -> Norm {
    let a = rot_point(r.x, r.y, quarters);
    let b = rot_point(r.x + r.w, r.y + r.h, quarters);
    Norm::from_corners(a, b)
}

/// The zoom factor and pan (canvas pixels) that fill a `view_w` by `view_h` canvas with the selected rectangle `sel`
/// (source-page coordinates) of a picture shown turned `quarters`, whose turned size at actual size is `rw` by `rh`.
/// The zoom is held to the viewer's range. None for an empty selection or canvas.
pub fn zoom_to_rect(sel: Norm, quarters: u8, rw: usize, rh: usize, view_w: usize, view_h: usize) -> Option<(f32, (f32, f32))> {
    let t = rot_rect(sel, quarters);
    let (sw, sh) = (t.w * rw as f32, t.h * rh as f32);
    if sw <= 0.0 || sh <= 0.0 || view_w == 0 || view_h == 0 {
        return None;
    }
    let scale = (view_w as f32 / sw).min(view_h as f32 / sh).clamp(ZOOM_MIN, ZOOM_MAX);
    // the picture is centred; moving it by minus the selection's offset from the middle brings the selection there
    let pan = (-(t.x + t.w / 2.0 - 0.5) * rw as f32 * scale, -(t.y + t.h / 2.0 - 0.5) * rh as f32 * scale);
    Some((scale, pan))
}

/// Which page is shown and how. The pan offset is in canvas pixels and resets on a page change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub page: usize,
    pub zoom: Zoom,
    pub quarters: u8,
    pub pan: (f32, f32),
    pub sidebar_scroll: f32,
}

impl Default for View {
    fn default() -> Self {
        View { page: 0, zoom: Zoom::Fit, quarters: 0, pan: (0.0, 0.0), sidebar_scroll: 0.0 }
    }
}

impl View {
    /// Shows page `page`, clamped to the page count. Returns whether the page changed.
    pub fn go(&mut self, page: usize, count: usize) -> bool {
        let last = count.saturating_sub(1);
        let next = page.min(last);
        let changed = next != self.page;
        self.page = next;
        if changed {
            self.pan = (0.0, 0.0);
        }
        changed
    }

    /// Moves by `delta` pages (negative goes back), clamped. Returns whether the page changed.
    pub fn step(&mut self, delta: i32, count: usize) -> bool {
        let target = (self.page as i64 + delta as i64).max(0) as usize;
        self.go(target, count)
    }

    /// Turns the picture by `delta` quarter turns (negative turns anticlockwise).
    pub fn turn(&mut self, delta: i32) {
        self.quarters = (self.quarters as i32 + delta).rem_euclid(4) as u8;
        self.pan = (0.0, 0.0);
    }
}

/// The colour adjustments of Adjust Color. The defaults leave the picture as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjust {
    /// Exposure in stops (EV), 0 is unchanged.
    pub exposure: f32,
    /// Saturation, 1 is unchanged.
    pub saturation: f32,
    /// Contrast, 1 is unchanged.
    pub contrast: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust { exposure: 0.0, saturation: 1.0, contrast: 1.0 }
    }
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Adjust::default()
    }

    /// The named filters to chain, each with its one input value, skipping the unchanged ones.
    pub fn settings(&self) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        if self.exposure != 0.0 {
            out.push(("exposure", self.exposure));
        }
        if self.saturation != 1.0 {
            out.push(("saturation", self.saturation));
        }
        if self.contrast != 1.0 {
            out.push(("contrast", self.contrast));
        }
        out
    }
}

/// What a markup mark is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Highlight,
    Note,
    Rect,
    Rounded,
    /// A freehand stroke: a signature or a drawing. Its points are in `Mark::ink`.
    Ink,
    /// An ellipse inside the rectangle.
    Oval,
    /// A straight line; its two ends are in `Mark::ink`.
    Line,
    /// A line with a head at its second end; the two ends are in `Mark::ink`.
    Arrow,
    /// A line along the bottom of the rectangle, for marking a line of text.
    Underline,
    /// A line through the middle of the rectangle, for striking out a line of text.
    Strike,
}

impl MarkKind {
    fn code(self) -> &'static str {
        match self {
            MarkKind::Highlight => "highlight",
            MarkKind::Note => "note",
            MarkKind::Rect => "rect",
            MarkKind::Rounded => "rounded",
            MarkKind::Ink => "ink",
            MarkKind::Oval => "oval",
            MarkKind::Line => "line",
            MarkKind::Arrow => "arrow",
            MarkKind::Underline => "underline",
            MarkKind::Strike => "strike",
        }
    }

    /// Whether the mark keeps its points in `Mark::ink`: a stroke, or the two ends of a line or an arrow.
    pub fn has_points(self) -> bool {
        matches!(self, MarkKind::Ink | MarkKind::Line | MarkKind::Arrow)
    }

    /// What the markup list in the sidebar calls the mark.
    pub fn label(self) -> &'static str {
        match self {
            MarkKind::Highlight => "Highlight",
            MarkKind::Note => "Note",
            MarkKind::Rect => "Rectangle",
            MarkKind::Rounded => "Rounded rectangle",
            MarkKind::Ink => "Drawing",
            MarkKind::Oval => "Oval",
            MarkKind::Line => "Line",
            MarkKind::Arrow => "Arrow",
            MarkKind::Underline => "Underline",
            MarkKind::Strike => "Strike-through",
        }
    }

    fn parse(s: &str) -> Option<MarkKind> {
        match s {
            "highlight" => Some(MarkKind::Highlight),
            "note" => Some(MarkKind::Note),
            "rect" => Some(MarkKind::Rect),
            "rounded" => Some(MarkKind::Rounded),
            "ink" => Some(MarkKind::Ink),
            "oval" => Some(MarkKind::Oval),
            "line" => Some(MarkKind::Line),
            "arrow" => Some(MarkKind::Arrow),
            "underline" => Some(MarkKind::Underline),
            "strike" => Some(MarkKind::Strike),
            _ => None,
        }
    }
}

/// One piece of markup. Its rectangle is in the unrotated page's normalised coordinates, so it
/// survives rotation and zoom.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub page: usize,
    pub rect: Norm,
    /// Packed colour (`pack`), straight alpha.
    pub color: u32,
    /// The note text. Empty for the other kinds.
    pub text: String,
    /// The points of a stroke, in the same normalised coordinates as `rect`. Empty for the other kinds.
    pub ink: Vec<(f32, f32)>,
}

/// A picture's pixels as an edit left them. Two values are equal when they are the same picture, not when every pixel
/// matches: an edit makes a new picture, so comparing the pointers tells whether anything changed without a pass over
/// the pixels.
#[derive(Clone, Debug)]
pub struct Pixels(pub Rc<Image>);

impl PartialEq for Pixels {
    fn eq(&self, other: &Pixels) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Everything an edit can change in a document: markup, page order and deletions, turns, colour adjustments and the
/// pixels of a picture (flip, crop, alpha). A PDF has no pixels of its own to edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub marks: Vec<Mark>,
    pub order: Vec<usize>,
    pub turns: Vec<u8>,
    pub adjust: Adjust,
    pub pixels: Option<Pixels>,
}

/// The most edits Undo goes back through.
const HISTORY_MAX: usize = 100;

/// The undo and redo stacks of a document's edits.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    /// Notes that an edit just changed the document from `before`. A new edit clears what Redo would have redone.
    pub fn record(&mut self, before: Snapshot) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// The state to go back to, given the `current` one (kept for Redo).
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        Some(previous)
    }

    /// The state to go forward to, given the `current` one (kept for Undo).
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// The pages visited, for Go > Back and Go > Forward.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trail {
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Trail {
    /// Notes that the view left page `from` for another page. Going somewhere new clears Forward.
    pub fn visit(&mut self, from: usize) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
        }
        if self.back.len() > HISTORY_MAX {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    /// The page Back goes to, given the `current` page (kept for Forward).
    pub fn back(&mut self, current: usize) -> Option<usize> {
        let page = self.back.pop()?;
        self.forward.push(current);
        Some(page)
    }

    /// The page Forward goes to, given the `current` page (kept for Back).
    pub fn forward(&mut self, current: usize) -> Option<usize> {
        let page = self.forward.pop()?;
        self.back.push(current);
        Some(page)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// The zero-based page for the text typed in Go to Page (pages count from 1), or `None` when it is not a page number
/// of a document of `count` pages.
pub fn parse_page(text: &str, count: usize) -> Option<usize> {
    let n: usize = text.trim().parse().ok()?;
    (n >= 1 && n <= count).then(|| n - 1)
}

/// The bookmarks file text: one source page per line.
pub fn to_bookmarks(pages: &[usize]) -> String {
    pages.iter().map(|p| format!("{p}\n")).collect()
}

/// Reads bookmarks back, in order and without repeats; lines that are not page numbers are skipped.
pub fn parse_bookmarks(text: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for page in text.lines().filter_map(|l| l.trim().parse::<usize>().ok()) {
        if !out.contains(&page) {
            out.push(page);
        }
    }
    out
}

/// Adds source page `page` to the bookmarks, or removes it when it is there. Returns whether it is bookmarked now.
pub fn toggle_bookmark(pages: &mut Vec<usize>, page: usize) -> bool {
    match pages.iter().position(|&p| p == page) {
        Some(i) => {
            pages.remove(i);
            false
        }
        None => {
            pages.push(page);
            true
        }
    }
}

/// The bookmarked shown page after (or before) `current`, from the shown positions of the bookmarks. It does not wrap.
pub fn neighbour(positions: &[usize], current: usize, forward: bool) -> Option<usize> {
    if forward {
        positions.iter().copied().filter(|&p| p > current).min()
    } else {
        positions.iter().copied().filter(|&p| p < current).max()
    }
}

/// A note's marker on an exported picture, in pixels.
const NOTE_PX: usize = 14;

/// Highlight colour: system yellow at about 45 per cent.
pub const HIGHLIGHT: u32 = pack(255, 214, 10, 115);
/// Note and shape colour: system blue.
pub const MARK_BLUE: u32 = pack(10, 132, 255, 255);
/// Strike-through colour: system red.
pub const MARK_RED: u32 = pack(255, 69, 58, 255);

const SIDECAR_HEADER: &str = "# viewer markup 1";

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The sidecar text for `marks`: a header line, then one tab-separated line per mark.
pub fn to_sidecar(marks: &[Mark]) -> String {
    let mut out = String::from(SIDECAR_HEADER);
    out.push('\n');
    for m in marks {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{:08x}\t{}\n",
            m.kind.code(),
            m.page,
            m.rect.x,
            m.rect.y,
            m.rect.w,
            m.rect.h,
            m.color,
            if m.kind.has_points() { encode_ink(&m.ink) } else { escape(&m.text) }
        ));
    }
    out
}

/// Reads sidecar text back. A missing header or a malformed line is skipped, not an error, so a
/// damaged file keeps whatever marks it still has.
pub fn parse_sidecar(text: &str) -> Vec<Mark> {
    let mut lines = text.lines();
    if lines.next() != Some(SIDECAR_HEADER) {
        return Vec::new();
    }
    lines.filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Mark> {
    let mut f = line.splitn(8, '\t');
    let kind = MarkKind::parse(f.next()?)?;
    let page = f.next()?.parse::<usize>().ok()?;
    let x = f.next()?.parse::<f32>().ok()?;
    let y = f.next()?.parse::<f32>().ok()?;
    let w = f.next()?.parse::<f32>().ok()?;
    let h = f.next()?.parse::<f32>().ok()?;
    let color = u32::from_str_radix(f.next()?, 16).ok()?;
    let field = f.next().unwrap_or("");
    let (text, ink) = if kind.has_points() { (String::new(), decode_ink(field)) } else { (unescape(field), Vec::new()) };
    if ![x, y, w, h].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Mark { kind, page, rect: Norm { x, y, w, h }, color, text, ink })
}

/// A stroke's points as text: `x,y` pairs separated by spaces. Numbers print in their shortest form, so they read back
/// exactly.
fn encode_ink(points: &[(f32, f32)]) -> String {
    points.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(" ")
}

/// Reads the points `encode_ink` wrote. Malformed pairs are skipped.
fn decode_ink(text: &str) -> Vec<(f32, f32)> {
    text.split_whitespace()
        .filter_map(|pair| {
            let (x, y) = pair.split_once(',')?;
            Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?))
        })
        .filter(|&(x, y)| x.is_finite() && y.is_finite())
        .collect()
}

/// The file name for a document's sidecar: a 64-bit FNV-1a hash of its path, so the same file
/// always finds its markup and two files never share a sidecar by name.
pub fn sidecar_name(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.markup")
}

/// Source-over of a straight-alpha colour `src` onto a premultiplied pixel `dst`.
fn over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xff;
    let inv = 255 - sa;
    let ch = |shift: u32| {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        ((s * sa + d * inv) / 255).min(255)
    };
    let da = (dst >> 24) & 0xff;
    let a = ((sa * 255 + da * inv) / 255).min(255);
    ch(0) | (ch(8) << 8) | (ch(16) << 16) | (a << 24)
}

/// The mark a drag from `a` to `b` (source-page points) makes. A line or an arrow keeps its two ends, in the order
/// they were dragged.
pub fn drag_mark(kind: MarkKind, page: usize, a: (f32, f32), b: (f32, f32), color: u32) -> Mark {
    let ink = if matches!(kind, MarkKind::Line | MarkKind::Arrow) { vec![a, b] } else { Vec::new() };
    Mark { kind, page, rect: Norm::from_corners(a, b), color, text: String::new(), ink }
}

/// Whether a drag of `moved` pixels that spans `rect` is big enough to keep as a mark of `kind`, on a page shown turned
/// `quarters`. A line may run straight across or down, and an underline or a strike-through has no height, but it needs
/// width as the page is shown (which is the page's height when it is turned a quarter).
pub fn drag_counts(kind: MarkKind, moved: f32, rect: Norm, quarters: u8) -> bool {
    if moved < 4.0 {
        return false;
    }
    match kind {
        MarkKind::Line | MarkKind::Arrow => true,
        MarkKind::Underline | MarkKind::Strike => (if quarters % 2 == 0 { rect.w } else { rect.h }) > 0.0,
        _ => rect.w > 0.0 && rect.h > 0.0,
    }
}

/// The two ends of the line an underline or a strike-through draws, as points of the unrotated page. The line follows
/// the bottom (underline) or the middle (strike-through) of the rectangle as the page is shown after `quarters`
/// clockwise turns, so a PDF reader that draws the saved annotation in the page's own orientation agrees with the window.
pub fn text_line_ends(kind: MarkKind, r: Norm, quarters: u8) -> ((f32, f32), (f32, f32)) {
    let (u0, v0, u1, v1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (um, vm) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
    let under = kind == MarkKind::Underline;
    match quarters % 4 {
        1 => {
            let u = if under { u1 } else { um };
            ((u, v0), (u, v1))
        }
        2 => {
            let v = if under { v0 } else { vm };
            ((u0, v), (u1, v))
        }
        3 => {
            let u = if under { u0 } else { um };
            ((u, v0), (u, v1))
        }
        _ => {
            let v = if under { v1 } else { vm };
            ((u0, v), (u1, v))
        }
    }
}

/// The two ends of an arrow head, for an arrow from `from` to `to`. The head is a quarter of the arrow long, between
/// 8 and 24 units (times `scale`), and opens 28 degrees each side. None for an arrow without length.
pub fn arrow_head(from: (f32, f32), to: (f32, f32), scale: f32) -> Option<[(f32, f32); 2]> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        return None;
    }
    let head = (len * 0.25).clamp(8.0 * scale, 24.0 * scale);
    // the unit vector from the tip back along the arrow, turned either way
    let (ux, uy) = (-dx / len, -dy / len);
    let (sin, cos) = 28.0f32.to_radians().sin_cos();
    let end = |side: f32| (to.0 + head * (ux * cos - side * uy * sin), to.1 + head * (uy * cos + side * ux * sin));
    Some([end(1.0), end(-1.0)])
}

/// The outline of the ellipse inside a rectangle, as a closed run of points.
pub fn ellipse_points(x: f32, y: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    const STEPS: usize = 48;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    (0..=STEPS)
        .map(|k| {
            let t = k as f32 / STEPS as f32 * std::f32::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// Draws the marks of `page` onto a picture that is `quarters` turns from the page. Highlights
/// are tinted, rectangles and rounded rectangles get a 2 pixel outline, notes get a small marker,
/// and strokes, lines, arrows, ovals, underlines and strike-throughs are drawn 2 pixels thick.
/// The picture is the unrotated page scaled to its own size, so this runs on the export copy.
pub fn burn(img: &mut Image, marks: &[Mark], page: usize, quarters: u8) {
    let (w, h) = (img.w, img.h);
    for m in marks.iter().filter(|m| m.page == page) {
        if m.kind.has_points() {
            burn_points_mark(img, m, quarters);
            continue;
        }
        let r = rot_rect(m.rect, quarters);
        let x0 = ((r.x * w as f32).round() as i64).clamp(0, w as i64) as usize;
        let y0 = ((r.y * h as f32).round() as i64).clamp(0, h as i64) as usize;
        let mut x1 = (((r.x + r.w) * w as f32).round() as i64).clamp(0, w as i64) as usize;
        let mut y1 = (((r.y + r.h) * h as f32).round() as i64).clamp(0, h as i64) as usize;
        if m.kind == MarkKind::Note {
            // a note is a small marker, whatever its zero-size anchor
            x1 = (x0 + NOTE_PX).min(w);
            y1 = (y0 + NOTE_PX).min(h);
        }
        let flat = matches!(m.kind, MarkKind::Underline | MarkKind::Strike);
        if x1 <= x0 || (y1 <= y0 && !flat) {
            continue;
        }
        match m.kind {
            MarkKind::Highlight => {
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y * w + x;
                        img.px[i] = over(img.px[i], m.color);
                    }
                }
            }
            MarkKind::Rect | MarkKind::Rounded | MarkKind::Note => {
                let c = if m.kind == MarkKind::Note { MARK_BLUE } else { m.color };
                let t = 2usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let edge = x < x0 + t || x + t >= x1 || y < y0 + t || y + t >= y1;
                        if edge {
                            let i = y * w + x;
                            img.px[i] = over(img.px[i], c);
                        }
                    }
                }
            }
            MarkKind::Oval => {
                let ring = ellipse_points(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32);
                burn_path(img, &ring, m.color);
            }
            MarkKind::Underline | MarkKind::Strike => {
                let y = if m.kind == MarkKind::Underline { y1 as f32 - 2.0 } else { (y0 + y1) as f32 / 2.0 };
                burn_path(img, &[(x0 as f32, y), (x1 as f32, y)], m.color);
            }
            MarkKind::Ink | MarkKind::Line | MarkKind::Arrow => {}
        }
    }
}

/// Draws a stroke, a line or an arrow onto a turned picture.
fn burn_points_mark(img: &mut Image, m: &Mark, quarters: u8) {
    let (w, h) = (img.w as f32, img.h as f32);
    let pts: Vec<(f32, f32)> = m
        .ink
        .iter()
        .map(|&(u, v)| {
            let (a, b) = rot_point(u, v, quarters);
            (a * w, b * h)
        })
        .collect();
    burn_path(img, &pts, m.color);
    if m.kind == MarkKind::Arrow {
        if let [.., from, to] = pts.as_slice() {
            if let Some([a, b]) = arrow_head(*from, *to, 1.0) {
                burn_path(img, &[a, *to, b], m.color);
            }
        }
    }
}

/// Draws a path of pixel points onto a picture: a 2 by 2 dot every pixel along each segment between its points.
fn burn_path(img: &mut Image, pts: &[(f32, f32)], color: u32) {
    for pair in pts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let x = (a.0 + (b.0 - a.0) * t).round() as i64;
            let y = (a.1 + (b.1 - a.1) * t).round() as i64;
            for dy in 0..2i64 {
                for dx in 0..2i64 {
                    let (px, py) = (x + dx, y + dy);
                    if px >= 0 && py >= 0 && (px as usize) < img.w && (py as usize) < img.h {
                        let i = py as usize * img.w + px as usize;
                        img.px[i] = over(img.px[i], color);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize) -> Image {
        // each pixel gets a distinct value so a rotation mistake shows up
        Image { w, h, px: (0..w * h).map(|i| pack(i as u8, 0, 0, 255)).collect() }
    }

    #[test]
    fn fit_scales_down_and_never_enlarges() {
        assert_eq!(Zoom::Fit.scale(2000, 1000, 1000, 1000), 0.5);
        assert_eq!(Zoom::Fit.scale(100, 100, 1000, 1000), 1.0);
        assert_eq!(Zoom::Fit.scale(0, 0, 10, 10), 1.0);
    }

    #[test]
    fn stepping_zoom_is_clamped() {
        let z = Zoom::Factor(ZOOM_MAX).stepped(10, 10, 10, 10, true);
        assert_eq!(z, Zoom::Factor(ZOOM_MAX));
        let z = Zoom::Factor(ZOOM_MIN).stepped(10, 10, 10, 10, false);
        assert_eq!(z, Zoom::Factor(ZOOM_MIN));
        let z = Zoom::Fit.stepped(100, 100, 50, 50, true);
        assert_eq!(z, Zoom::Factor(0.5 * ZOOM_STEP));
    }

    #[test]
    fn fit_box_keeps_aspect_and_never_enlarges() {
        assert_eq!(fit_box(200, 100, 100, 100), (100, 50));
        assert_eq!(fit_box(100, 300, 132, 264), (88, 264));
        assert_eq!(fit_box(50, 20, 132, 264), (50, 20));
        assert_eq!(fit_box(0, 0, 132, 264), (132, 264));
    }

    #[test]
    fn four_quarter_turns_return_the_original() {
        let src = img(3, 2);
        let mut cur = src.clone();
        for _ in 0..4 {
            cur = rotate(&cur, 1);
        }
        assert_eq!(cur, src);
    }

    #[test]
    fn quarter_turn_swaps_the_size_and_moves_the_top_left_pixel() {
        let src = img(2, 3);
        let out = rotate(&src, 1);
        assert_eq!((out.w, out.h), (3, 2));
        // the top-left source pixel (0,0) ends at the top-right corner after a clockwise turn
        assert_eq!(out.px[out.w - 1], src.px[0]);
        assert_eq!(rotate(&src, 5), rotate(&src, 1));
    }

    #[test]
    fn point_mapping_round_trips_for_every_turn() {
        for q in 0..4u8 {
            let (a, b) = rot_point(0.2, 0.7, q);
            let (u, v) = unrot_point(a, b, q);
            assert!((u - 0.2).abs() < 1e-6 && (v - 0.7).abs() < 1e-6, "turn {q}");
        }
    }

    #[test]
    fn rect_mapping_matches_the_rotated_picture() {
        let r = Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.25 };
        let turned = rot_rect(r, 1);
        assert!((turned.x - 0.75).abs() < 1e-6 && (turned.y - 0.0).abs() < 1e-6);
        assert!((turned.w - 0.25).abs() < 1e-6 && (turned.h - 0.5).abs() < 1e-6);
        let back = Norm::from_corners(unrot_point(turned.x, turned.y, 1), unrot_point(turned.x + turned.w, turned.y + turned.h, 1));
        assert!((back.x - 0.0).abs() < 1e-6 && (back.w - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zooming_to_a_selection_fits_it_to_the_canvas_and_centres_it() {
        let middle = Norm { x: 0.25, y: 0.25, w: 0.5, h: 0.5 };
        let (scale, pan) = zoom_to_rect(middle, 0, 1000, 500, 500, 250).expect("a zoom");
        assert!((scale - 1.0).abs() < 1e-6 && pan.0.abs() < 1e-3 && pan.1.abs() < 1e-3);
        let corner = Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 };
        let (scale, pan) = zoom_to_rect(corner, 0, 1000, 500, 500, 250).expect("a zoom");
        assert!((scale - 1.0).abs() < 1e-6);
        assert!((pan.0 - 250.0).abs() < 1e-3 && (pan.1 - 125.0).abs() < 1e-3, "{pan:?}");
        // a tiny selection is held to the largest zoom, and an empty one gives nothing
        let tiny = Norm { x: 0.5, y: 0.5, w: 0.0001, h: 0.0001 };
        assert_eq!(zoom_to_rect(tiny, 0, 1000, 500, 500, 250).map(|z| z.0), Some(ZOOM_MAX));
        assert_eq!(zoom_to_rect(Norm { x: 0.1, y: 0.1, w: 0.0, h: 0.2 }, 0, 10, 10, 10, 10), None);
        assert_eq!(zoom_to_rect(middle, 0, 10, 10, 0, 10), None);
        // turned a quarter, the selection's shown size swaps its sides
        let (scale, _) = zoom_to_rect(Norm { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }, 1, 500, 1000, 1000, 250).expect("a zoom");
        assert!((scale - 0.5).abs() < 1e-6, "{scale}");
    }

    #[test]
    fn navigation_clamps_and_resets_pan() {
        let mut v = View::default();
        v.pan = (5.0, 5.0);
        assert!(!v.step(-1, 3));
        assert!(v.step(10, 3));
        assert_eq!(v.page, 2);
        assert_eq!(v.pan, (0.0, 0.0));
        v.turn(-1);
        assert_eq!(v.quarters, 3);
    }

    #[test]
    fn default_adjust_adds_no_filters() {
        assert!(Adjust::default().is_identity());
        assert!(Adjust::default().settings().is_empty());
        let a = Adjust { exposure: 0.5, saturation: 1.0, contrast: 1.2 };
        assert_eq!(a.settings(), vec![("exposure", 0.5), ("contrast", 1.2)]);
    }

    #[test]
    fn sidecar_round_trips_text_with_tabs_and_newlines() {
        let marks = vec![
            Mark { kind: MarkKind::Note, page: 2, rect: Norm { x: 0.1, y: 0.2, w: 0.0, h: 0.0 }, color: MARK_BLUE, text: "a\tb\nc\\d".into(), ink: Vec::new() },
            Mark { kind: MarkKind::Highlight, page: 0, rect: Norm { x: 0.3, y: 0.4, w: 0.2, h: 0.05 }, color: HIGHLIGHT, text: String::new(), ink: Vec::new() },
        ];
        let text = to_sidecar(&marks);
        assert_eq!(parse_sidecar(&text), marks);
    }

    #[test]
    fn sidecar_skips_bad_lines_and_wrong_headers() {
        assert!(parse_sidecar("not a viewer file\nrect\t0\t0\t0\t1\t1\tff0000ff\t").is_empty());
        let text = format!("{SIDECAR_HEADER}\nbogus line\nrect\t0\tnope\t0\t1\t1\tff0000ff\t\nrect\t1\t0\t0\t1\t1\tff0000ff\tok\n");
        let got = parse_sidecar(&text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].page, 1);
    }

    #[test]
    fn sidecar_name_is_stable_and_distinct() {
        assert_eq!(sidecar_name("/a/b.png"), sidecar_name("/a/b.png"));
        assert_ne!(sidecar_name("/a/b.png"), sidecar_name("/a/c.png"));
        assert!(sidecar_name("/a/b.png").ends_with(".markup"));
    }

    #[test]
    fn burn_outlines_a_rectangle_and_tints_a_highlight() {
        let mut picture = Image { w: 10, h: 10, px: vec![pack(255, 255, 255, 255); 100] };
        let marks = vec![
            Mark { kind: MarkKind::Rect, page: 0, rect: Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }, color: MARK_BLUE, text: String::new(), ink: Vec::new() },
            Mark { kind: MarkKind::Highlight, page: 1, rect: Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, color: HIGHLIGHT, text: String::new(), ink: Vec::new() },
        ];
        burn(&mut picture, &marks, 0, 0);
        assert_ne!(picture.px[0], pack(255, 255, 255, 255));
        // the interior of the outline is untouched
        assert_eq!(picture.px[2 * 10 + 2], pack(255, 255, 255, 255));
        // the highlight on page 1 is not drawn on page 0
        assert_eq!(picture.px[99], pack(255, 255, 255, 255));
    }

    #[test]
    fn ink_strokes_round_trip_and_burn_in() {
        let stroke = Mark {
            kind: MarkKind::Ink,
            page: 0,
            rect: Norm { x: 0.1, y: 0.1, w: 0.8, h: 0.5 },
            color: MARK_BLUE,
            text: String::new(),
            ink: vec![(0.1, 0.1), (0.5, 0.6), (0.9, 0.2)],
        };
        assert_eq!(parse_sidecar(&to_sidecar(std::slice::from_ref(&stroke))), vec![stroke.clone()]);
        assert_eq!(decode_ink("0.5,0.25 x,y 0.75,1"), vec![(0.5, 0.25), (0.75, 1.0)]);
        let mut picture = Image { w: 10, h: 10, px: vec![pack(255, 255, 255, 255); 100] };
        burn(&mut picture, &[stroke], 0, 0);
        // the first point is drawn, and the picture away from the stroke is untouched
        assert_ne!(picture.px[1 * 10 + 1], pack(255, 255, 255, 255));
        assert_eq!(picture.px[9 * 10 + 9], pack(255, 255, 255, 255));
    }

    #[test]
    fn lines_and_arrows_keep_their_ends_and_round_trip() {
        let line = drag_mark(MarkKind::Arrow, 2, (0.8, 0.1), (0.2, 0.6), MARK_BLUE);
        assert_eq!(line.ink, vec![(0.8, 0.1), (0.2, 0.6)]);
        assert_eq!(line.rect, Norm::from_corners((0.8, 0.1), (0.2, 0.6)));
        assert_eq!(parse_sidecar(&to_sidecar(std::slice::from_ref(&line))), vec![line]);
        let oval = drag_mark(MarkKind::Oval, 0, (0.1, 0.1), (0.5, 0.4), MARK_BLUE);
        assert!(oval.ink.is_empty());
        for kind in [MarkKind::Oval, MarkKind::Underline, MarkKind::Strike, MarkKind::Line] {
            let m = drag_mark(kind, 1, (0.1, 0.2), (0.4, 0.3), MARK_RED);
            assert_eq!(parse_sidecar(&to_sidecar(std::slice::from_ref(&m))), vec![m], "{kind:?}");
        }
    }

    #[test]
    fn a_drag_is_kept_by_what_the_mark_needs() {
        let flat = Norm { x: 0.1, y: 0.5, w: 0.4, h: 0.0 };
        assert!(drag_counts(MarkKind::Line, 30.0, flat, 0));
        assert!(drag_counts(MarkKind::Underline, 30.0, flat, 0));
        assert!(!drag_counts(MarkKind::Rect, 30.0, flat, 0));
        assert!(!drag_counts(MarkKind::Arrow, 2.0, flat, 0));
        let tall = Norm { x: 0.1, y: 0.1, w: 0.0, h: 0.3 };
        assert!(!drag_counts(MarkKind::Strike, 30.0, tall, 0));
        // turned a quarter, a page's height is the width on screen
        assert!(drag_counts(MarkKind::Strike, 30.0, tall, 1));
        assert!(!drag_counts(MarkKind::Strike, 30.0, flat, 1));
    }

    #[test]
    fn a_text_line_lies_along_the_bottom_or_middle_as_the_page_is_shown() {
        let r = Norm { x: 0.2, y: 0.3, w: 0.4, h: 0.1 };
        for q in 0..4u8 {
            let shown = rot_rect(r, q);
            let ((ax, ay), (bx, by)) = text_line_ends(MarkKind::Underline, r, q);
            let (a, b) = (rot_point(ax, ay, q), rot_point(bx, by, q));
            assert!((a.1 - (shown.y + shown.h)).abs() < 1e-5 && (b.1 - (shown.y + shown.h)).abs() < 1e-5, "underline at turn {q}");
            assert!((a.0.min(b.0) - shown.x).abs() < 1e-5 && (a.0.max(b.0) - (shown.x + shown.w)).abs() < 1e-5, "underline span at turn {q}");
            let ((ax, ay), (bx, by)) = text_line_ends(MarkKind::Strike, r, q);
            let (a, b) = (rot_point(ax, ay, q), rot_point(bx, by, q));
            assert!((a.1 - (shown.y + shown.h / 2.0)).abs() < 1e-5 && (b.1 - a.1).abs() < 1e-5, "strike at turn {q}");
        }
    }

    #[test]
    fn an_arrow_head_sits_at_the_tip_and_opens_back_along_the_arrow() {
        let [a, b] = arrow_head((0.0, 0.0), (100.0, 0.0), 1.0).expect("a head");
        // a quarter of 100 is 25, held to 24, and the ends mirror each other across the arrow
        for p in [a, b] {
            let d = ((p.0 - 100.0).powi(2) + p.1.powi(2)).sqrt();
            assert!((d - 24.0).abs() < 1e-3, "{d}");
            assert!(p.0 < 100.0);
        }
        assert!((a.1 + b.1).abs() < 1e-4 && a.1.abs() > 1.0);
        assert_eq!(arrow_head((1.0, 1.0), (1.0, 1.0), 1.0), None);
        let [short, _] = arrow_head((0.0, 0.0), (10.0, 0.0), 1.0).expect("a head");
        assert!((((short.0 - 10.0).powi(2) + short.1.powi(2)).sqrt() - 8.0).abs() < 1e-3);
    }

    #[test]
    fn the_ellipse_outline_closes_and_stays_on_the_ellipse() {
        let pts = ellipse_points(10.0, 20.0, 40.0, 20.0);
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        assert!((first.0 - last.0).abs() < 1e-3 && (first.1 - last.1).abs() < 1e-3);
        for (x, y) in pts {
            let v = ((x - 30.0) / 20.0).powi(2) + ((y - 30.0) / 10.0).powi(2);
            assert!((v - 1.0).abs() < 1e-3, "{v}");
        }
    }

    #[test]
    fn the_new_shapes_burn_into_an_export() {
        let white = pack(255, 255, 255, 255);
        let blank = || Image { w: 20, h: 20, px: vec![white; 400] };
        let touched = |m: Mark| {
            let mut picture = blank();
            burn(&mut picture, &[m], 0, 0);
            picture.px.iter().filter(|&&p| p != white).count()
        };
        assert!(touched(drag_mark(MarkKind::Oval, 0, (0.1, 0.1), (0.9, 0.9), MARK_BLUE)) > 20);
        assert!(touched(drag_mark(MarkKind::Line, 0, (0.0, 0.0), (1.0, 1.0), MARK_BLUE)) > 20);
        assert!(touched(drag_mark(MarkKind::Arrow, 0, (0.0, 0.5), (1.0, 0.5), MARK_BLUE)) > touched(drag_mark(MarkKind::Line, 0, (0.0, 0.5), (1.0, 0.5), MARK_BLUE)));
        // an underline sits on the bottom edge of its rectangle, a strike-through in the middle
        let mut under = blank();
        burn(&mut under, &[drag_mark(MarkKind::Underline, 0, (0.0, 0.2), (1.0, 0.6), MARK_BLUE)], 0, 0);
        assert_ne!(under.px[10 * 20 + 5], white);
        assert_eq!(under.px[6 * 20 + 5], white);
        let mut strike = blank();
        burn(&mut strike, &[drag_mark(MarkKind::Strike, 0, (0.0, 0.2), (1.0, 0.6), MARK_RED)], 0, 0);
        assert_ne!(strike.px[8 * 20 + 5], white);
    }

    fn snap(n: usize) -> Snapshot {
        Snapshot { marks: Vec::new(), order: (0..n).collect(), turns: vec![0; n], adjust: Adjust::default(), pixels: None }
    }

    #[test]
    fn undo_and_redo_walk_the_edits_and_a_new_edit_clears_redo() {
        let mut h = History::default();
        assert!(!h.can_undo() && !h.can_redo());
        h.record(snap(1));
        h.record(snap(2));
        let back = h.undo(snap(3)).expect("an edit to undo");
        assert_eq!(back, snap(2));
        assert!(h.can_redo());
        assert_eq!(h.redo(snap(2)), Some(snap(3)));
        assert_eq!(h.undo(snap(3)), Some(snap(2)));
        h.record(snap(9));
        assert!(!h.can_redo());
        assert_eq!(h.undo(snap(10)), Some(snap(9)));
        assert_eq!(h.undo(snap(9)), Some(snap(1)));
        assert_eq!(h.undo(snap(1)), None);
    }

    #[test]
    fn history_keeps_a_bounded_number_of_edits() {
        let mut h = History::default();
        for i in 0..(HISTORY_MAX + 20) {
            h.record(snap(i + 1));
        }
        let mut n = 0;
        while h.undo(snap(1)).is_some() {
            n += 1;
        }
        assert_eq!(n, HISTORY_MAX);
    }

    #[test]
    fn the_trail_goes_back_and_forward_and_new_visits_clear_forward() {
        let mut t = Trail::default();
        assert!(!t.can_back() && !t.can_forward());
        t.visit(0);
        t.visit(4);
        assert_eq!(t.back(7), Some(4));
        assert_eq!(t.back(4), Some(0));
        assert_eq!(t.back(0), None);
        assert_eq!(t.forward(0), Some(4));
        t.visit(4);
        assert!(!t.can_forward());
    }

    #[test]
    fn go_to_page_text_is_one_based_and_checked() {
        assert_eq!(parse_page("1", 5), Some(0));
        assert_eq!(parse_page(" 5 ", 5), Some(4));
        assert_eq!(parse_page("6", 5), None);
        assert_eq!(parse_page("0", 5), None);
        assert_eq!(parse_page("x", 5), None);
        assert_eq!(parse_page("", 5), None);
    }

    #[test]
    fn bookmarks_toggle_round_trip_and_find_neighbours() {
        let mut pages = Vec::new();
        assert!(toggle_bookmark(&mut pages, 3));
        assert!(toggle_bookmark(&mut pages, 1));
        assert!(!toggle_bookmark(&mut pages, 3));
        assert_eq!(pages, vec![1]);
        assert_eq!(parse_bookmarks(&to_bookmarks(&[4, 2, 4, 9])), vec![4, 2, 9]);
        assert_eq!(parse_bookmarks("x\n7\n"), vec![7]);
        assert_eq!(neighbour(&[2, 5, 9], 5, true), Some(9));
        assert_eq!(neighbour(&[2, 5, 9], 5, false), Some(2));
        assert_eq!(neighbour(&[2, 5, 9], 9, true), None);
        assert_eq!(neighbour(&[], 0, false), None);
    }
}
