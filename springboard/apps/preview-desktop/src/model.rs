//! Viewer state that needs no drawing: zoom, quarter turns, page navigation, markup in page
//! coordinates, the sidecar file that keeps markup, and the burn-in used by export. Everything here
//! is plain data, so the tests run without a window.

use bifrost_app::kit::{pack, Image};

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

/// Orientation bits above the two quarter-turn bits: the picture is mirrored left to right, or top to bottom, after it
/// is turned. The orientation of a page is its turns plus these.
pub const FLIP_H: u8 = 4;
pub const FLIP_V: u8 = 8;

/// Maps a point of the unrotated page to the same point after the orientation `quarters` (turns, then flips).
pub fn rot_point(u: f32, v: f32, quarters: u8) -> (f32, f32) {
    let (a, b) = match quarters % 4 {
        1 => (1.0 - v, u),
        2 => (1.0 - u, 1.0 - v),
        3 => (v, 1.0 - u),
        _ => (u, v),
    };
    (if quarters & FLIP_H != 0 { 1.0 - a } else { a }, if quarters & FLIP_V != 0 { 1.0 - b } else { b })
}

/// The inverse of `rot_point`: a point of the oriented picture back on the unrotated page.
pub fn unrot_point(a: f32, b: f32, quarters: u8) -> (f32, f32) {
    let a = if quarters & FLIP_H != 0 { 1.0 - a } else { a };
    let b = if quarters & FLIP_V != 0 { 1.0 - b } else { b };
    match quarters % 4 {
        1 => (b, 1.0 - a),
        2 => (1.0 - a, 1.0 - b),
        3 => (1.0 - b, a),
        _ => (a, b),
    }
}

/// `img` turned and flipped to the orientation `quarters`.
pub fn orient(img: &Image, quarters: u8) -> Image {
    crate::picture::flip(&rotate(img, quarters), quarters & FLIP_H != 0, quarters & FLIP_V != 0)
}

/// A source-page rectangle as it sits on the turned picture (its corners mapped, then re-spanned).
pub fn rot_rect(r: Norm, quarters: u8) -> Norm {
    let a = rot_point(r.x, r.y, quarters);
    let b = rot_point(r.x + r.w, r.y + r.h, quarters);
    Norm::from_corners(a, b)
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
        let turns = (self.quarters as i32 + delta).rem_euclid(4) as u8;
        self.quarters = (self.quarters & !3) | turns;
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
    /// Colour temperature, -100 (cooler) to 100 (warmer), 0 is unchanged.
    pub temperature: f32,
    /// Midtone brightness as a gamma power, 1 is unchanged.
    pub midtones: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust { exposure: 0.0, saturation: 1.0, contrast: 1.0, temperature: 0.0, midtones: 1.0 }
    }
}

/// The rows of the Adjust Color panel: label, least, greatest. The order is the one `Adjust::get` and `with` use.
pub const ADJUST_ROWS: [(&str, f32, f32); 5] = [
    ("Exposure", -2.0, 2.0),
    ("Contrast", 0.5, 2.0),
    ("Saturation", 0.0, 2.0),
    ("Temperature", -100.0, 100.0),
    ("Midtones", 0.5, 2.0),
];

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
        if self.temperature != 0.0 {
            out.push(("temperatureTint", self.temperature));
        }
        if self.midtones != 1.0 {
            out.push(("gamma", self.midtones));
        }
        out
    }

    /// The value of panel row `row`, or 0 past the last row.
    pub fn get(&self, row: usize) -> f32 {
        match row {
            0 => self.exposure,
            1 => self.contrast,
            2 => self.saturation,
            3 => self.temperature,
            4 => self.midtones,
            _ => 0.0,
        }
    }

    /// This adjustment with panel row `row` set to `value`, kept inside the row's range.
    pub fn with(mut self, row: usize, value: f32) -> Adjust {
        let Some(&(_, lo, hi)) = ADJUST_ROWS.get(row) else { return self };
        let v = value.clamp(lo, hi);
        match row {
            0 => self.exposure = v,
            1 => self.contrast = v,
            2 => self.saturation = v,
            3 => self.temperature = v,
            _ => self.midtones = v,
        }
        self
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
    Oval,
    /// A straight line from the first to the last of `Mark::ink`.
    Line,
    /// A line with an arrow head at its last point.
    Arrow,
    Star,
    /// A speech bubble: a rounded box with a tail.
    Bubble,
    /// A line under the marked text.
    Underline,
    /// A line through the marked text.
    Strike,
    /// A text box; its text is `Mark::text`.
    Text,
    /// A black box that hides what is under it.
    Redact,
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
            MarkKind::Star => "star",
            MarkKind::Bubble => "bubble",
            MarkKind::Underline => "underline",
            MarkKind::Strike => "strike",
            MarkKind::Text => "text",
            MarkKind::Redact => "redact",
        }
    }

    fn parse(s: &str) -> Option<MarkKind> {
        const ALL: [MarkKind; 14] = [
            MarkKind::Highlight,
            MarkKind::Note,
            MarkKind::Rect,
            MarkKind::Rounded,
            MarkKind::Ink,
            MarkKind::Oval,
            MarkKind::Line,
            MarkKind::Arrow,
            MarkKind::Star,
            MarkKind::Bubble,
            MarkKind::Underline,
            MarkKind::Strike,
            MarkKind::Text,
            MarkKind::Redact,
        ];
        ALL.into_iter().find(|k| k.code() == s)
    }

    /// Whether the mark keeps its points in `Mark::ink` (a stroke, a line or an arrow) instead of text.
    pub fn has_points(self) -> bool {
        matches!(self, MarkKind::Ink | MarkKind::Line | MarkKind::Arrow)
    }

    /// Whether the mark is drawn as outlines from `outlines`.
    pub fn is_outline(self) -> bool {
        matches!(self, MarkKind::Oval | MarkKind::Line | MarkKind::Arrow | MarkKind::Star | MarkKind::Bubble | MarkKind::Underline | MarkKind::Strike)
    }

    /// The name shown in the notes list of the sidebar.
    pub fn label(self) -> &'static str {
        match self {
            MarkKind::Highlight => "Highlight",
            MarkKind::Note => "Note",
            MarkKind::Rect => "Rectangle",
            MarkKind::Rounded => "Rounded Rectangle",
            MarkKind::Ink => "Drawing",
            MarkKind::Oval => "Oval",
            MarkKind::Line => "Line",
            MarkKind::Arrow => "Arrow",
            MarkKind::Star => "Star",
            MarkKind::Bubble => "Speech Bubble",
            MarkKind::Underline => "Underline",
            MarkKind::Strike => "Strike Through",
            MarkKind::Text => "Text",
            MarkKind::Redact => "Redaction",
        }
    }
}

/// The colour of a redaction.
pub const REDACT: u32 = pack(0, 0, 0, 255);

/// The outlines a mark of an outline kind is drawn with, as polylines in the mark's page coordinates. A polyline that
/// ends where it starts is closed. Other kinds have none.
pub fn outlines(m: &Mark) -> Vec<Vec<(f32, f32)>> {
    let r = m.rect;
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    match m.kind {
        MarkKind::Oval => {
            let (cx, cy, rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
            let n = 48;
            let mut ring: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    (cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect();
            ring.push(ring[0]);
            vec![ring]
        }
        MarkKind::Line | MarkKind::Arrow => {
            let (a, b) = match (m.ink.first(), m.ink.last()) {
                (Some(&a), Some(&b)) if m.ink.len() >= 2 => (a, b),
                _ => ((x0, y0), (x1, y1)),
            };
            let mut out = vec![vec![a, b]];
            if m.kind == MarkKind::Arrow {
                out.push(arrow_head(a, b));
            }
            out
        }
        MarkKind::Star => {
            let (cx, cy, rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
            let mut pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let radius = if i % 2 == 0 { 1.0 } else { 0.4 };
                    let t = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    (cx + rx * radius * t.cos(), cy + ry * radius * t.sin())
                })
                .collect();
            pts.push(pts[0]);
            vec![pts]
        }
        MarkKind::Bubble => {
            // the box takes the upper four fifths; a tail hangs from its lower left
            let by = y0 + r.h * 0.8;
            vec![vec![(x0, y0), (x1, y0), (x1, by), (x0 + r.w * 0.45, by), (x0 + r.w * 0.2, y1), (x0 + r.w * 0.25, by), (x0, by), (x0, y0)]]
        }
        MarkKind::Underline => vec![vec![(x0, y1), (x1, y1)]],
        MarkKind::Strike => {
            let y = y0 + r.h / 2.0;
            vec![vec![(x0, y), (x1, y)]]
        }
        _ => Vec::new(),
    }
}

/// The strokes of an arrow head at `b` for a line from `a`, as one polyline: wing, tip, wing.
fn arrow_head(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= f32::EPSILON {
        return vec![b, b];
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (len * 0.3).min(0.04);
    // the direction back from the tip, turned 30 degrees either way
    let wing = |sign: f32| {
        let (c, s) = (0.866_025_4_f32, 0.5 * sign);
        let (bx, by) = (-ux, -uy);
        (b.0 + head * (bx * c - by * s), b.1 + head * (bx * s + by * c))
    };
    vec![wing(1.0), b, wing(-1.0)]
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

/// Everything an edit can change in a document: markup, page order and deletions, turns, flips, colour adjustments and
/// a picture's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub marks: Vec<Mark>,
    pub order: Vec<usize>,
    pub turns: Vec<u8>,
    /// The mirror bits of each source page (`FLIP_H`, `FLIP_V`).
    pub flips: Vec<u8>,
    pub adjust: Adjust,
    /// The picture of a picture document, for edits that change its pixels (crop, resize, mask).
    pub pic: PicState,
}

/// The pixels of a picture document at one moment. Two states are equal when they have the same revision, so the
/// history never compares pixels.
#[derive(Clone, Debug)]
pub struct PicState {
    pub rev: u32,
    pub image: Option<std::rc::Rc<Image>>,
}

impl PartialEq for PicState {
    fn eq(&self, other: &PicState) -> bool {
        self.rev == other.rev
    }
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

/// Draws the marks of `page` onto a picture that is `quarters` turns from the page. Highlights
/// are tinted, rectangles and rounded rectangles get a 2 pixel outline, notes get a small marker.
/// The picture is the unrotated page scaled to its own size, so this runs on the export copy.
pub fn burn(img: &mut Image, marks: &[Mark], page: usize, quarters: u8) {
    let (w, h) = (img.w, img.h);
    for m in marks.iter().filter(|m| m.page == page) {
        if m.kind == MarkKind::Ink {
            burn_polyline(img, &m.ink, m.color, quarters);
            continue;
        }
        if m.kind.is_outline() {
            for line in outlines(m) {
                burn_polyline(img, &line, m.color, quarters);
            }
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
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        match m.kind {
            MarkKind::Highlight | MarkKind::Redact => {
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
            // strokes and outlines were drawn above, and a text box has no text drawn into a picture
            MarkKind::Ink | MarkKind::Text => {}
            _ => {}
        }
    }
}

/// Draws a polyline of page coordinates onto a picture oriented by `quarters`: a 2 by 2 dot every pixel along each
/// segment between its points.
fn burn_polyline(img: &mut Image, points: &[(f32, f32)], color: u32, quarters: u8) {
    let (w, h) = (img.w as f32, img.h as f32);
    let pts: Vec<(f32, f32)> = points
        .iter()
        .map(|&(u, v)| {
            let (a, b) = rot_point(u, v, quarters);
            (a * w, b * h)
        })
        .collect();
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

/// What the Inspector shows about the open document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorInfo {
    pub name: String,
    pub kind: &'static str,
    /// The file's size in bytes.
    pub bytes: Option<u64>,
    /// The shown page's size in pixels (pictures).
    pub pixels: Option<(usize, usize)>,
    pub pages: usize,
    pub pdf_version: Option<String>,
    pub encrypted: Option<bool>,
    pub marks: usize,
    pub bookmarks: usize,
    /// The GPS position text, once it was asked for with Show Location Info.
    pub location: Option<String>,
}

/// A file size as text: bytes under a kilobyte, then one decimal of KB, MB or GB (1000 based).
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    if n < 1000 {
        return format!("{n} bytes");
    }
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The label and value lines of the Inspector.
pub fn inspector_lines(i: &InspectorInfo) -> Vec<(String, String)> {
    let mut out = vec![("Name".to_string(), i.name.clone()), ("Kind".to_string(), i.kind.to_string())];
    if let Some(bytes) = i.bytes {
        out.push(("Size".into(), format_bytes(bytes)));
    }
    if let Some((w, h)) = i.pixels {
        out.push(("Dimensions".into(), format!("{w} \u{d7} {h} pixels")));
    }
    out.push(("Pages".into(), i.pages.to_string()));
    if let Some(version) = &i.pdf_version {
        out.push(("PDF Version".into(), version.clone()));
    }
    if let Some(encrypted) = i.encrypted {
        out.push(("Encryption".into(), if encrypted { "Encrypted" } else { "None" }.into()));
    }
    out.push(("Annotations".into(), i.marks.to_string()));
    out.push(("Bookmarks".into(), i.bookmarks.to_string()));
    if let Some(location) = &i.location {
        out.push(("Location".into(), location.clone()));
    }
    out
}

/// The largest side Adjust Size writes.
pub const SIZE_MAX: usize = 16000;

/// The size typed for Adjust Size, for a `w` by `h` picture: `800` is a width (the height follows the ratio), `x600`
/// a height, and `800x600` both. None for text that is not a size or is out of range.
pub fn parse_size(text: &str, w: usize, h: usize) -> Option<(usize, usize)> {
    let text = text.trim().to_lowercase();
    let (a, b) = match text.split_once('x') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (text.as_str(), ""),
    };
    let width = if a.is_empty() { None } else { Some(a.parse::<usize>().ok()?) };
    let height = if b.is_empty() { None } else { Some(b.parse::<usize>().ok()?) };
    let (nw, nh) = match (width, height) {
        (Some(nw), Some(nh)) => (nw, nh),
        (Some(nw), None) => (nw, ((h as f64 * nw as f64 / w.max(1) as f64).round() as usize).max(1)),
        (None, Some(nh)) => (((w as f64 * nh as f64 / h.max(1) as f64).round() as usize).max(1), nh),
        (None, None) => return None,
    };
    ((1..=SIZE_MAX).contains(&nw) && (1..=SIZE_MAX).contains(&nh)).then_some((nw, nh))
}

/// `marks` of a picture after it is cropped to `crop`: each mark is moved into the new page and clamped to it, and a
/// mark that falls wholly outside is dropped.
pub fn crop_marks(marks: &[Mark], crop: Norm) -> Vec<Mark> {
    if crop.w <= 0.0 || crop.h <= 0.0 {
        return marks.to_vec();
    }
    let map = |(x, y): (f32, f32)| ((x - crop.x) / crop.w, (y - crop.y) / crop.h);
    marks
        .iter()
        .filter(|m| m.rect.x + m.rect.w >= crop.x && m.rect.x <= crop.x + crop.w && m.rect.y + m.rect.h >= crop.y && m.rect.y <= crop.y + crop.h)
        .map(|m| {
            let (x0, y0) = map((m.rect.x, m.rect.y));
            let (x1, y1) = map((m.rect.x + m.rect.w, m.rect.y + m.rect.h));
            let rect = Norm::from_corners((x0.clamp(0.0, 1.0), y0.clamp(0.0, 1.0)), (x1.clamp(0.0, 1.0), y1.clamp(0.0, 1.0)));
            let ink = m.ink.iter().map(|&p| map(p)).collect();
            Mark { rect, ink, ..m.clone() }
        })
        .collect()
}

/// The zoom factor and pan that show the page rectangle `sel` (page coordinates) filling a `canvas_w` by `canvas_h`
/// canvas. `page_w` and `page_h` are the oriented picture's size at actual size and `quarters` its orientation.
pub fn zoom_to_rect(sel: Norm, quarters: u8, page_w: usize, page_h: usize, canvas_w: usize, canvas_h: usize) -> (f32, (f32, f32)) {
    let r = rot_rect(sel, quarters);
    let (rw, rh) = (page_w as f32, page_h as f32);
    let sw = (r.w * rw).max(1.0);
    let sh = (r.h * rh).max(1.0);
    let scale = (canvas_w as f32 / sw).min(canvas_h as f32 / sh).clamp(ZOOM_MIN, ZOOM_MAX);
    let pan = ((0.5 - (r.x + r.w / 2.0)) * rw * scale, (0.5 - (r.y + r.h / 2.0)) * rh * scale);
    (scale, pan)
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

    fn snap(n: usize) -> Snapshot {
        Snapshot { marks: Vec::new(), order: (0..n).collect(), turns: vec![0; n], adjust: Adjust::default() }
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

    fn shape(kind: MarkKind, rect: Norm) -> Mark {
        Mark { kind, page: 0, rect, color: MARK_BLUE, text: String::new(), ink: Vec::new() }
    }

    #[test]
    fn flips_mirror_the_oriented_point_and_round_trip() {
        assert_eq!(rot_point(0.25, 0.5, FLIP_H), (0.75, 0.5));
        assert_eq!(rot_point(0.25, 0.5, FLIP_V), (0.25, 0.5));
        assert_eq!(rot_point(0.25, 0.75, FLIP_V), (0.25, 0.25));
        for q in 0..16u8 {
            let (a, b) = rot_point(0.2, 0.7, q);
            let (u, v) = unrot_point(a, b, q);
            assert!((u - 0.2).abs() < 1e-5 && (v - 0.7).abs() < 1e-5, "orientation {q}");
        }
        // flip bits do not change whether the picture is turned sideways
        assert_eq!(turn_size(4, 2, FLIP_H | 1), (2, 4));
        assert_eq!(turn_size(4, 2, FLIP_V), (4, 2));
    }

    #[test]
    fn orient_turns_then_mirrors() {
        let i = img(3, 2);
        let flipped = orient(&i, FLIP_H);
        assert_eq!(flipped.px[0], i.px[2]);
        assert_eq!(flipped.px[3], i.px[5]);
        assert_eq!(orient(&i, 0), i);
    }

    #[test]
    fn turning_keeps_the_flip_bits() {
        let mut v = View { quarters: FLIP_H | 3, ..View::default() };
        v.turn(1);
        assert_eq!(v.quarters, FLIP_H);
    }

    #[test]
    fn the_new_kinds_survive_the_sidecar() {
        let mut line = shape(MarkKind::Arrow, Norm { x: 0.1, y: 0.2, w: 0.3, h: 0.1 });
        line.ink = vec![(0.4, 0.3), (0.1, 0.2)];
        let mut text = shape(MarkKind::Text, Norm { x: 0.1, y: 0.2, w: 0.3, h: 0.05 });
        text.text = "hello\tthere".into();
        let marks = vec![line, text, shape(MarkKind::Oval, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }), shape(MarkKind::Redact, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.1 })];
        assert_eq!(parse_sidecar(&to_sidecar(&marks)), marks);
    }

    #[test]
    fn outlines_follow_the_mark_kind() {
        let r = Norm { x: 0.2, y: 0.2, w: 0.4, h: 0.2 };
        assert!(outlines(&shape(MarkKind::Rect, r)).is_empty());
        let oval = outlines(&shape(MarkKind::Oval, r));
        assert_eq!(oval.len(), 1);
        assert_eq!(oval[0].first(), oval[0].last());
        let star = outlines(&shape(MarkKind::Star, r));
        assert_eq!(star[0].len(), 11);
        let mut arrow = shape(MarkKind::Arrow, r);
        arrow.ink = vec![(0.0, 0.5), (0.5, 0.5)];
        let lines = outlines(&arrow);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], vec![(0.0, 0.5), (0.5, 0.5)]);
        // the head's wings sit behind the tip, on either side of the line
        assert_eq!(lines[1].len(), 3);
        assert_eq!(lines[1][1], (0.5, 0.5));
        assert!(lines[1][0].0 < 0.5 && lines[1][2].0 < 0.5);
        assert!(lines[1][0].1 != lines[1][2].1);
        let under = outlines(&shape(MarkKind::Underline, r));
        assert!((under[0][0].0 - 0.2).abs() < 1e-6 && (under[0][1].0 - 0.6).abs() < 1e-6);
        assert!((under[0][0].1 - 0.4).abs() < 1e-6 && under[0][0].1 == under[0][1].1);
        let strike = outlines(&shape(MarkKind::Strike, r));
        assert!((strike[0][0].1 - 0.3).abs() < 1e-6);
    }

    #[test]
    fn burn_draws_outline_kinds_and_redactions() {
        let mut i = Image { w: 20, h: 20, px: vec![pack(255, 255, 255, 255); 400] };
        burn(&mut i, &[shape(MarkKind::Underline, Norm { x: 0.0, y: 0.0, w: 1.0, h: 0.5 })], 0, 0);
        assert_ne!(i.px[10 * 20 + 5], pack(255, 255, 255, 255));
        assert_eq!(i.px[0], pack(255, 255, 255, 255));
        let mut j = Image { w: 10, h: 10, px: vec![pack(255, 255, 255, 255); 100] };
        let mut red = shape(MarkKind::Redact, Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 });
        red.color = REDACT;
        burn(&mut j, &[red], 0, 0);
        assert_eq!(j.px[0], pack(0, 0, 0, 255));
        assert_eq!(j.px[9 * 10 + 9], pack(255, 255, 255, 255));
    }

    #[test]
    fn adjust_rows_read_and_clamp() {
        let a = Adjust::default().with(0, 5.0).with(3, -250.0).with(9, 1.0);
        assert_eq!(a.exposure, 2.0);
        assert_eq!(a.temperature, -100.0);
        assert_eq!(a.get(0), 2.0);
        assert_eq!(a.get(9), 0.0);
        let names: Vec<_> = a.settings().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["exposure", "temperatureTint"]);
        assert!(Adjust::default().with(4, 1.0).is_identity());
    }

    #[test]
    fn sizes_are_typed_as_width_height_or_both() {
        assert_eq!(parse_size("800", 400, 200), Some((800, 400)));
        assert_eq!(parse_size("x100", 400, 200), Some((200, 100)));
        assert_eq!(parse_size("640 x 480", 400, 200), Some((640, 480)));
        assert_eq!(parse_size("", 400, 200), None);
        assert_eq!(parse_size("0", 400, 200), None);
        assert_eq!(parse_size("99999", 400, 200), None);
        assert_eq!(parse_size("wide", 400, 200), None);
    }

    #[test]
    fn cropping_moves_marks_into_the_new_page() {
        let crop = Norm { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
        let mut inside = shape(MarkKind::Rect, Norm { x: 0.6, y: 0.6, w: 0.2, h: 0.2 });
        inside.ink = vec![(0.75, 0.75)];
        let outside = shape(MarkKind::Rect, Norm { x: 0.0, y: 0.0, w: 0.2, h: 0.2 });
        let kept = crop_marks(&[inside, outside], crop);
        assert_eq!(kept.len(), 1);
        assert!((kept[0].rect.x - 0.2).abs() < 1e-5 && (kept[0].rect.w - 0.4).abs() < 1e-5);
        assert_eq!(kept[0].ink, vec![(0.5, 0.5)]);
    }

    #[test]
    fn zooming_to_a_rectangle_centres_it() {
        let (scale, pan) = zoom_to_rect(Norm { x: 0.25, y: 0.25, w: 0.5, h: 0.5 }, 0, 1000, 800, 500, 400);
        assert!((scale - 1.0).abs() < 1e-5);
        assert!(pan.0.abs() < 1e-3 && pan.1.abs() < 1e-3);
        // a rectangle in the top left pulls the page right and down to bring it to the middle
        let (_, pan) = zoom_to_rect(Norm { x: 0.0, y: 0.0, w: 0.25, h: 0.25 }, 0, 1000, 800, 500, 400);
        assert!(pan.0 > 0.0 && pan.1 > 0.0);
    }

    #[test]
    fn file_sizes_use_decimal_units() {
        assert_eq!(format_bytes(999), "999 bytes");
        assert_eq!(format_bytes(1500), "1.5 KB");
        assert_eq!(format_bytes(2_400_000), "2.4 MB");
        assert_eq!(format_bytes(3_000_000_000), "3.0 GB");
    }

    #[test]
    fn the_inspector_lists_only_what_it_knows() {
        let mut info = InspectorInfo { name: "a.png".into(), kind: "PNG image", bytes: Some(2048), pixels: Some((640, 480)), pages: 1, marks: 2, ..InspectorInfo::default() };
        let lines = inspector_lines(&info);
        let labels: Vec<&str> = lines.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["Name", "Kind", "Size", "Dimensions", "Pages", "Annotations", "Bookmarks"]);
        assert_eq!(lines[3].1, "640 \u{d7} 480 pixels");
        info.pdf_version = Some("1.7".into());
        info.encrypted = Some(false);
        info.location = Some("No location data".into());
        let lines = inspector_lines(&info);
        assert!(lines.iter().any(|(l, v)| l == "PDF Version" && v == "1.7"));
        assert!(lines.iter().any(|(l, v)| l == "Encryption" && v == "None"));
        assert_eq!(lines.last().map(|(l, _)| l.as_str()), Some("Location"));
    }
}
