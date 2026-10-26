//! A multi-line text editor view: word-wrapped layout in SF Pro with a
//! larger first paragraph (the note's title), a blinking caret placed by
//! tap or arrow keys, insert/delete/replace with a 50-step undo history,
//! and a selection range the clipboard can read and replace.

use super::glyphs::{either, paint, segment};
use super::keyboard::Out;
use super::textfield::{draw_loupe, SELECT};

/// Opacity of the selection wash (the highlight colour under selected text).
const SELECTION_ALPHA: u32 = 90;
use super::md::{self, Block, Style};
use super::ui::{Ui, Weight, BLACK, SECONDARY};
use crate::gfx::{pack, rgba, Image, Surface};
use std::collections::HashMap;
use std::ops::Range;
use bifrost_ui::undo::UndoManager;
use bifrost_render::font::Style as FontStyle;
use bifrost_render::math::{self, layout::typeset_tex, Layout, Options as MathOptions};

/// Caret on/off period in ms (bifrost blinks at roughly 1 Hz).
const BLINK_MS: f64 = 500.0;
/// Line pitch as a multiple of the point size (bifrost body text: 17 pt on 22 pt).
const LINE_K: f32 = 1.3;
/// Edits kept for undo.
pub const UNDO_DEPTH: usize = 50;

/// One wrapped line: `range` is the char span it covers (including a
/// trailing newline or hanging spaces), `adv` the advance of each char
/// before the newline, in px.
struct Line {
    range: Range<usize>,
    text: String,
    adv: Vec<f32>,
    y: f32,
    h: f32,
    pt: f32,
    weight: Weight,
    img: Option<Image>,
    /// Markdown mode: styled runs of the line (empty in plain mode) and
    /// their rendered images.
    segs: Vec<Seg>,
    simgs: Vec<Option<Image>>,
    block: Block,
    /// chars of indent + marker at the start of the paragraph
    prefix: usize,
    indent: usize,
    /// first wrapped line of its paragraph
    first: bool,
    para_start: usize,
    /// an attached image shown instead of the text
    image: Option<String>,
    /// Inline math: each formula with the line-relative char where its `$...$` span starts. The span's
    /// source chars take the formula's width, and the formula is drawn there instead of the source.
    maths: Vec<(usize, Layout)>,
    /// A `$$...$$` paragraph: the formula is centred on the line and the source is not drawn.
    display: Option<Layout>,
}

/// The font size in px at which a line of `pt` points is set (the size `Ui::text` renders at).
fn line_px(ui: &Ui, pt: f32) -> f32 {
    ui.pt(pt * ui.text_scale)
}

/// Typesets the TeX source `src` at `px`, or `None` when it does not parse (the source is then shown as text).
fn formula(_ui: &Ui, src: &str, px: f32, display: bool) -> Option<Layout> {
    let metrics = math::system::metrics();
    let opts = if display { MathOptions::display(px) } else { MathOptions::text(px) };
    typeset_tex(src, opts, &metrics).ok()
}

/// Draws a typeset formula with its baseline origin at (`x`, `baseline`). With the installed math font the glyphs
/// are painted from that face; otherwise each symbol is an image of the UI font, placed by the same rule as the
/// math painter (the image's baseline sits `pad + ascent` below its top).
fn paint_formula(s: &mut Surface, ui: &Ui, lay: &Layout, x: f32, baseline: f32, color: u32) {
    let ink = if color == super::ui::WHITE { super::ui::LABEL.now() } else { color };
    if let Some(font) = math::system::math_font() {
        math::paint::paint(lay, s, (x, baseline), font.face, ink);
        return;
    }
    for g in &lay.glyphs {
        let asc = ui.font.line_metrics(g.size).ascent;
        let (gx, gy) = math::paint::run_origin(g, (x, baseline), asc);
        let st = FontStyle { px: g.size, embolden: 0.0, color: bifrost_render::appearance::contrast(color), shadow: false };
        let img = ui.font.render_var(&g.ch.to_string(), &st, &[]);
        blit_text(s, &img, gx.round() as i32, gy.round() as i32, color);
    }
    let rule = if color == super::ui::WHITE { super::ui::LABEL.now() } else { color };
    for r in &lay.rules {
        s.fill_rrect(x + r.x, baseline + r.y, r.w, r.h, 0.0, rule);
    }
}

/// A run of chars (line-relative) drawn in one weight and colour. `kind` 1
/// is a bullet marker and 2 a checkbox, both drawn as shapes; 3, 4 and 5 are
/// struck-through, underlined and highlighted text, and 6 the bar of a quote.
/// 7 draws nothing here: a formula is drawn over its source, and the "# " title marker of the first line and the
/// emphasis markers (`**`, `*`, `~~`, `++`, `==`, backticks) are hidden, so the note reads as rich text.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seg {
    a: usize,
    b: usize,
    weight: Weight,
    color: u32,
    kind: u8,
}

/// Draws text rendered in `color`. White text is the label colour: it is drawn in `LABEL.now()` so it follows Light
/// and Dark without re-rendering or rebuilding the area (`NativeTextView.textColor = .labelColor`; undo survives).
fn blit_text(s: &mut Surface, img: &crate::gfx::Image, x: i32, y: i32, color: u32) {
    if color == super::ui::WHITE {
        s.blit_colored(img, x, y, super::ui::LABEL.now(), 255);
    } else {
        s.blit(img, x, y, 255, 255);
    }
}

/// Inline code colour (systemOrange).
const CODE: u32 = pack(255, 159, 10, 255);

impl Line {
    /// Px from the line's left edge to the boundary before char `c`.
    fn x_at(&self, c: usize) -> f32 {
        self.adv.iter().take(c.saturating_sub(self.range.start)).sum()
    }

    fn key(&self) -> (String, u32, u8) {
        let mut text = self.text.clone();
        for g in &self.segs {
            text.push_str(&format!("\u{0}{}:{}:{}:{}:{}", g.a, g.b, g.weight as u8, g.color, g.kind));
        }
        (text, (self.pt * 10.0) as u32, self.weight as u8)
    }

}

/// One reversible edit: `inserted` replaced `removed` at char `at`.
#[derive(Clone, Debug, PartialEq)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    caret_before: usize,
}

/// The state an undo step works on: the text and the caret.
#[derive(Clone, Debug, Default, PartialEq)]
struct Doc {
    text: String,
    caret: usize,
}

/// Replaces `old` chars at char index `at` of `text` with `new`.
fn splice(text: &mut String, at: usize, old: usize, new: &str) {
    let a = text.char_indices().nth(at).map_or(text.len(), |(i, _)| i);
    let b = text.char_indices().nth(at + old).map_or(text.len(), |(i, _)| i);
    text.replace_range(a..b, new);
}

/// Reverses `e` on a document: the inserted text goes back to the removed text and the caret returns.
fn undo_step(e: &Edit) -> impl Fn(&mut Doc) + 'static {
    let e = e.clone();
    move |d| {
        splice(&mut d.text, e.at, e.inserted.chars().count(), &e.removed);
        d.caret = e.caret_before;
    }
}

/// Repeats `e` on a document.
fn redo_step(e: &Edit) -> impl Fn(&mut Doc) + 'static {
    let e = e.clone();
    move |d| {
        splice(&mut d.text, e.at, e.removed.chars().count(), &e.inserted);
        d.caret = e.at + e.inserted.chars().count();
    }
}

pub struct TextArea {
    text: String,
    /// insertion point as a char index
    caret: usize,
    /// the other end of the selection, when one exists
    anchor: Option<usize>,
    /// wrap width in px
    w: f32,
    body_pt: f32,
    title_pt: f32,
    color: u32,
    tint: u32,
    lines: Vec<Line>,
    /// Undo and redo (`UndoManager`); typing that is still coalescing sits in an open group.
    history: UndoManager<Doc>,
    /// The last edit in the open typing group, so the next keystrokes can merge into it.
    typing: Option<Edit>,
    pub focused: bool,
    blink_from: f64,
    /// render and edit Markdown (headings, emphasis, lists, checklists)
    markdown: bool,
    /// attached images by file name, already sized to fit the width
    images: HashMap<String, Image>,
    /// search matches to highlight (char ranges) and which one is current
    highlights: Vec<Range<usize>>,
    current_hl: Option<usize>,
    /// the finger on the text: where it went down, what it holds
    touch: Option<Touch>,
    /// the last tap: (x, y, t, taps in a row), for double and triple tap
    last_tap: Option<(f32, f32, f64, u8)>,
    /// find links in the text (opt-in: `set_detect_links`); char ranges, kept current by `layout`
    detect_links: bool,
    links: Vec<Range<usize>>,
    /// paragraphs (by index) drawn with no height: folded sections; empty for every other user
    hidden: Vec<Range<usize>>,
    /// a mouse drives the text (`set_pointer_mode`): no selection handles or loupe, and `press`/`drag_to` select
    pointer: bool,
    /// consecutive presses for double and triple click (`bifrost_ui::text::ClickCounter`)
    clicks: bifrost_ui::text::ClickCounter,
    /// the mouse button is down: the unit the press selected (an empty range at the caret for one click) and the
    /// click count, so a drag grows the selection by characters, words or paragraphs
    drag: Option<(Range<usize>, u8)>,
    /// an input method's composition in progress (`setMarkedText:`): the char range of the marked text, and the text it
    /// took the place of (the selection when the composition began), which comes back when it is dropped
    marked: Option<(Range<usize>, String)>,
    /// Markdown mode: the char ranges of the hidden emphasis markers. They take no width, and the caret steps, the
    /// deletes and the clicks pass over them as the reference's rich text has no such characters (`layout` keeps it)
    concealed: Vec<Range<usize>>,
    /// Markdown mode: the hidden block markers at the start of heading paragraphs (`## `, `### `, `# `, the title's
    /// included), also in `concealed`. A caret anywhere from a marker's start to its end is at the heading's first
    /// visible char: typing goes in after the marker, so the line stays a heading (`past_lead`, `settle`)
    leads: Vec<Range<usize>>,
}

/// What a finger down on the text is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Grab {
    /// a press that may still become a tap
    Tap,
    /// dragging the caret (after a long press), with the loupe up
    Caret,
    /// dragging the selection's start or end lollipop
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x: f32,
    y: f32,
    grab: Grab,
    moved: bool,
}

/// Taps this close in time and space chain into double and triple taps.
const MULTI_TAP_MS: f64 = 350.0;
const MULTI_TAP_PT: f32 = 20.0;
/// Finger travel that turns a press into a drag.
const SLOP_PT: f32 = 10.0;
/// Selection handle: a bar along the line with a dot at its free end.
const DOT_PT: f32 = 10.0;
const BAR_PT: f32 = 2.0;
const HANDLE_HIT_PT: f32 = 22.0;

/// Advance of every char of `text` at `pt` points, in px.
fn advances(ui: &Ui, text: &str, pt: f32, weight: Weight) -> Vec<f32> {
    let mut buf = [0u8; 4];
    text.chars().map(|c| ui.measure(c.encode_utf8(&mut buf), pt, weight)).collect()
}

/// The links in `text` as char ranges, from the `detect` kit (byte ranges converted for the caret).
fn detected_links(text: &str) -> Vec<Range<usize>> {
    use bifrost_ui::detect::{find, Kind};
    find(text, &[Kind::Link]).into_iter().map(|m| text[..m.range.start].chars().count()..text[..m.range.end].chars().count()).collect()
}

/// Greedy word wrap of one paragraph (no newlines) whose char advances are
/// `adv`: the char index where each line ends. Spaces hang past the width;
/// a word wider than the line breaks between chars.
pub fn wrap_breaks(chars: &[char], adv: &[f32], w: f32) -> Vec<usize> {
    let n = chars.len();
    let mut ends = vec![];
    let mut ls = 0;
    loop {
        let (mut i, mut x, mut last_space) = (ls, 0.0, None);
        let mut end = n;
        while i < n {
            if chars[i] != ' ' && x + adv[i] > w && i > ls {
                end = last_space.map_or(i, |s| s + 1);
                break;
            }
            x += adv[i];
            if chars[i] == ' ' {
                last_space = Some(i);
            }
            i += 1;
        }
        ends.push(end);
        if end >= n {
            return ends;
        }
        ls = end;
    }
}

impl TextArea {
    /// Wraps at `w` px; the first paragraph is set at `title_pt` bold, the
    /// rest at `body_pt` regular. `tint` colours the caret and selection.
    pub fn new(ui: &Ui, w: f32, body_pt: f32, title_pt: f32, color: u32, tint: u32) -> Self {
        let mut ta = TextArea { text: String::new(), caret: 0, anchor: None, w, body_pt, title_pt, color, tint, lines: vec![], history: UndoManager::new(UNDO_DEPTH), typing: None, focused: false, blink_from: 0.0, markdown: false, images: HashMap::new(), highlights: vec![], current_hl: None, touch: None, last_tap: None, detect_links: false, links: vec![], hidden: vec![], pointer: false, clicks: Default::default(), drag: None, marked: None, concealed: vec![], leads: vec![] };
        ta.layout(ui);
        ta
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn can_undo(&self) -> bool {
        self.typing.is_some() || self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The Edit menu titles (`undoMenuItemTitle`): an open run of typing is "Undo Typing".
    pub fn undo_titles(&self) -> (String, String) {
        let undo = if self.typing.is_some() { "Undo Typing".to_string() } else { self.history.undo_menu_item_title() };
        (undo, self.history.redo_menu_item_title())
    }

    /// The number of undo steps (a run of typing counts once), for tests.
    #[cfg(test)]
    fn steps(&self) -> usize {
        self.history.steps() + usize::from(self.typing.is_some())
    }

    /// Replaces the contents (caret at the end) and forgets the history.
    pub fn set_text(&mut self, ui: &Ui, text: &str) {
        self.marked = None;
        self.text = text.to_string();
        self.caret = self.len();
        self.anchor = None;
        self.end_typing();
        self.history.clear();
        self.layout(ui);
    }

    /// Re-wraps for a new width.
    pub fn set_width(&mut self, ui: &Ui, w: f32) {
        if w != self.w {
            self.w = w;
            self.layout(ui);
        }
    }

    /// Hides whole paragraphs (by paragraph index, end exclusive): they keep their place in the text and the undo
    /// history but take no height, so the lines below move up. Relayout only happens when the ranges change.
    pub fn set_hidden(&mut self, ui: &Ui, ranges: Vec<Range<usize>>) {
        if self.hidden != ranges {
            self.hidden = ranges;
            self.layout(ui);
        }
    }

    /// The y and height in px of the wrapped line holding char `c` (height 0 when its paragraph is hidden).
    pub fn line_span(&self, c: usize) -> (f32, f32) {
        self.lines.get(self.line_of(c)).map_or((0.0, 0.0), |l| (l.y, l.h))
    }

    /// Height of the laid-out text in px.
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.y + l.h)
    }

    /// Height of the first (title) line, for callers that size headers.
    pub fn line_h(&self, ui: &Ui, pt: f32) -> f32 {
        ui.pt(pt * LINE_K).round()
    }

    /// The text of every wrapped line, newlines stripped.
    pub fn line_texts(&self) -> Vec<&str> {
        self.lines.iter().map(|l| l.text.as_str()).collect()
    }

    /// Turns Markdown rendering and list continuation on or off.
    pub fn set_markdown(&mut self, ui: &Ui, on: bool) {
        if self.markdown != on {
            self.markdown = on;
            self.layout(ui);
        }
    }

    pub fn markdown(&self) -> bool {
        self.markdown
    }

    /// Image attachments by file name (pre-scaled to the wrap width).
    pub fn set_images(&mut self, ui: &Ui, images: HashMap<String, Image>) {
        self.images = images;
        self.layout(ui);
    }

    /// Highlights search matches; `current` is drawn stronger.
    /// Finds links in the text (`NativeDataDetector`): they are underlined in the tint and `link_at` reports them.
    pub fn set_detect_links(&mut self, on: bool) {
        self.detect_links = on;
        self.links = if on { detected_links(&self.text) } else { vec![] };
    }

    /// The link under the point (px from the text origin), as its text; None when there is no link there.
    pub fn link_at(&self, x: f32, y: f32) -> Option<String> {
        let c = self.position_at(x, y);
        let r = self.links.iter().find(|r| r.contains(&c))?;
        Some(self.text.chars().skip(r.start).take(r.len()).collect())
    }

    pub fn set_highlights(&mut self, ranges: Vec<Range<usize>>, current: Option<usize>) {
        self.highlights = ranges;
        self.current_hl = current;
    }

    /// Px from the text origin to the top of the line holding char `c`.
    pub fn y_of(&self, c: usize) -> f32 {
        self.lines.get(self.line_of(c)).map_or(0.0, |l| l.y)
    }

    /// Moves the caret (clamped) and drops the selection.
    pub fn set_caret(&mut self, c: usize) {
        self.caret = c.min(self.len());
        self.anchor = None;
    }

    /// The paragraph (char range without its newline) holding char `c`.
    pub fn paragraph_of(&self, c: usize) -> Range<usize> {
        let chars: Vec<char> = self.text.chars().collect();
        let c = c.min(chars.len());
        let start = chars[..c].iter().rposition(|&ch| ch == '\n').map_or(0, |p| p + 1);
        let end = chars[c..].iter().position(|&ch| ch == '\n').map_or(chars.len(), |p| c + p);
        start..end
    }

    /// The char index of the paragraph whose checkbox a tap at (x, y) px from
    /// the text origin lands on (a few px of slack for fingers).
    pub fn checkbox_at(&self, x: f32, y: f32) -> Option<usize> {
        let l = self.lines.iter().find(|l| y >= l.y && y < l.y + l.h)?;
        if !l.first || !matches!(l.block, Block::Check(_)) {
            return None;
        }
        let (a, b) = (l.x_at(l.range.start + l.indent), l.x_at(l.range.start + l.prefix));
        (x >= a - 8.0 && x < b + 8.0).then_some(l.para_start)
    }

    /// Per-char (weight, colour, kind) of a Markdown paragraph.
    fn char_styles(&self, para: &str, n: usize, block: Block, prefix: usize, indent: usize, base: Weight) -> Vec<(Weight, u32, u8)> {
        let mut v = vec![(base, self.color, 0u8); n];
        for (i, st) in v.iter_mut().enumerate().take(prefix) {
            st.1 = SECONDARY;
            if i >= indent {
                match block {
                    Block::Check(_) => st.2 = 2,
                    Block::Bullet if i == indent => st.2 = 1,
                    Block::Quote if i == indent => st.2 = 6,
                    _ => {}
                }
            }
        }
        match block {
            Block::Quote => v.iter_mut().skip(prefix).for_each(|st| st.1 = SECONDARY),
            Block::Mono => v.iter_mut().skip(prefix).for_each(|st| st.1 = CODE),
            _ => {}
        }
        let done = block == Block::Check(true);
        for sp in md::spans(para, prefix) {
            for st in v.iter_mut().skip(sp.range.start).take(sp.range.len()) {
                match sp.style {
                    Style::Bold => st.0 = Weight::Bold,
                    Style::Italic if st.0 == Weight::Regular => st.0 = Weight::Medium,
                    Style::Code => st.1 = CODE,
                    Style::Strike if !sp.marker => st.2 = 3,
                    Style::Underline if !sp.marker => st.2 = 4,
                    Style::Highlight if !sp.marker => st.2 = 5,
                    _ => {}
                }
                if sp.marker || done {
                    st.1 = SECONDARY;
                }
                // the emphasis markers are not shown: the text carries the style (a formula's dollars stay as source)
                if sp.marker && sp.style != Style::Math {
                    st.2 = 7;
                }
            }
        }
        v
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(b, _)| b)
    }

    fn layout(&mut self, ui: &Ui) {
        if self.detect_links {
            // byte ranges from the detector, as char ranges (the caret and the edits count chars)
            self.links = detected_links(&self.text);
        }
        let mut cache: HashMap<(String, u32, u8), (Option<Image>, Vec<Option<Image>>)> =
            self.lines.drain(..).filter(|l| l.img.is_some() || l.simgs.iter().any(Option::is_some)).map(|mut l| (l.key(), (l.img.take(), std::mem::take(&mut l.simgs)))).collect();
        let mut lines = vec![];
        let mut concealed: Vec<Range<usize>> = vec![];
        let mut leads: Vec<Range<usize>> = vec![];
        let (mut y, mut start) = (0.0, 0);
        let paragraphs: Vec<&str> = self.text.split('\n').collect();
        let last = paragraphs.len() - 1;
        for (pi, para) in paragraphs.into_iter().enumerate() {
            let chars: Vec<char> = para.chars().collect();
            let folded = self.hidden.iter().any(|r| r.contains(&pi));
            if self.markdown && pi > 0 {
                if let Some(name) = md::image_ref(para).filter(|n| self.images.contains_key(*n)) {
                    let h = if folded { 0.0 } else { self.images[name].h as f32 + ui.pt(12.0) };
                    let range = start..start + chars.len() + (pi != last) as usize;
                    lines.push(Line { range, text: para.to_string(), adv: vec![0.0; chars.len()], y, h, pt: self.body_pt, weight: Weight::Regular, img: None, segs: vec![], simgs: vec![], block: Block::Body, prefix: 0, indent: 0, first: true, para_start: start, image: Some(name.to_string()), maths: vec![], display: None });
                    y += h;
                    start += chars.len() + 1;
                    continue;
                }
            }
            let info = self.markdown.then(|| md::classify(para));
            let (pt, weight, block, prefix, indent) = match info {
                Some(i) if pi > 0 => {
                    let (pt, w) = match i.block {
                        Block::Title => (self.title_pt, Weight::Bold),
                        Block::Heading => (self.body_pt * 1.3, Weight::Bold),
                        Block::Subheading => (self.body_pt * 1.12, Weight::Semibold),
                        _ => (self.body_pt, Weight::Regular),
                    };
                    (pt, w, i.block, i.prefix, i.indent)
                }
                _ if pi == 0 => (self.title_pt, Weight::Bold, Block::Body, 0, 0),
                _ => (self.body_pt, Weight::Regular, Block::Body, 0, 0),
            };
            // `$$...$$` paragraph: display math; `$...$` spans: inline math, typeset at the body size
            let trimmed = para.trim();
            let display = if self.markdown && trimmed.len() > 4 && trimmed.starts_with("$$") && trimmed.ends_with("$$") {
                formula(ui, &trimmed[2..trimmed.len() - 2], line_px(ui, pt), true)
            } else {
                None
            };
            let h = if folded {
                0.0
            } else if let Some(d) = &display {
                self.line_h(ui, pt).max(d.ascent + d.depth + ui.pt(12.0))
            } else {
                self.line_h(ui, pt)
            };
            let mut styles = if self.markdown { self.char_styles(para, chars.len(), block, prefix, indent, weight) } else { vec![] };
            // a heading's marker (`## `, with a checkbox marker after it as older notes wrote it) is not drawn, as the
            // title's `# ` is not: the line already has the heading style, and the caret steps over the marker
            let lead = if self.markdown && pi > 0 { md::heading_marker(para) } else { 0 };
            for st in styles.iter_mut().take(lead) {
                st.2 = 7;
            }
            if lead > 0 && display.is_none() {
                leads.push(start..start + lead);
            }
            // the hidden emphasis markers, before the formulas and the title marker take kind 7 too
            if display.is_none() {
                for (i, st) in styles.iter().enumerate().filter(|(_, st)| st.2 == 7) {
                    match concealed.last_mut() {
                        Some(r) if r.end == start + i => r.end += 1,
                        _ => concealed.push(start + i..start + i + 1),
                    }
                }
            }
            let mut para_maths: Vec<(usize, usize, Layout)> = vec![];
            if self.markdown && display.is_none() {
                for sp in md::spans(para, prefix) {
                    if sp.style != Style::Math || sp.marker {
                        continue;
                    }
                    let src: String = chars[sp.range.clone()].iter().collect();
                    if let Some(lay) = formula(ui, &src, line_px(ui, pt), false) {
                        para_maths.push((sp.range.start, sp.range.end, lay));
                    }
                }
                for (a, b, _) in &para_maths {
                    for st in styles.iter_mut().take(*b).skip(*a) {
                        st.2 = 7;
                    }
                }
            }
            // the first line is the title; a "# " title marker on it is not shown (the line already has the title style)
            let hide = match info {
                Some(i) if pi == 0 && i.block == Block::Title => i.prefix.min(chars.len()),
                _ => 0,
            };
            for st in styles.iter_mut().take(hide) {
                st.2 = 7;
            }
            // the title's marker is a hidden lead like a heading's: a click or Cmd+Left at the line start puts the caret
            // after it, so a paste or typing there stays in the title instead of landing in front of the `# `
            if hide > 0 && display.is_none() {
                leads.push(start..start + hide);
                concealed.push(start..start + hide);
            }
            let mut adv: Vec<f32> = if display.is_some() {
                vec![0.0; chars.len()]
            } else if self.markdown {
                let mut buf = [0u8; 4];
                chars.iter().zip(&styles).map(|(c, st)| ui.measure(c.encode_utf8(&mut buf), pt, st.0)).collect()
            } else {
                advances(ui, para, pt, weight)
            };
            // hidden chars (the title marker, emphasis markers, a formula's source) take no width
            for (d, st) in adv.iter_mut().zip(&styles) {
                if st.2 == 7 {
                    *d = 0.0;
                }
            }
            // a formula takes its width on its first char; the rest of its source takes none
            for (a, b, lay) in &para_maths {
                adv[*a] = lay.width;
                for d in adv.iter_mut().take(*b).skip(*a + 1) {
                    *d = 0.0;
                }
            }
            let mut ls = 0;
            for end in wrap_breaks(&chars, &adv, self.w) {
                let text: String = chars[ls..end].iter().collect();
                let newline = (pi != last && end == chars.len()) as usize;
                let range = start + ls..start + end + newline;
                let mut segs: Vec<Seg> = vec![];
                if self.markdown {
                    for i in ls..end {
                        let (w, color, kind) = styles[i];
                        match segs.last_mut() {
                            Some(g) if g.weight == w && g.color == color && g.kind == kind && kind != 1 => g.b = i + 1 - ls,
                            _ => segs.push(Seg { a: i - ls, b: i + 1 - ls, weight: w, color, kind }),
                        }
                    }
                }
                let maths: Vec<(usize, Layout)> =
                    para_maths.iter().filter(|(a, _, _)| *a >= ls && *a < end).map(|(a, _, lay)| (a - ls, lay.clone())).collect();
                let line_display = if ls == 0 { display.clone() } else { None };
                let mut line = Line { text, adv: adv[ls..end].to_vec(), range, y, h, pt, weight, img: None, segs, simgs: vec![], block, prefix, indent, first: ls == 0, para_start: start, image: None, maths, display: line_display };
                if let Some((img, simgs)) = cache.remove(&line.key()) {
                    line.img = img;
                    line.simgs = simgs;
                }
                lines.push(line);
                y += h;
                ls = end;
            }
            start += chars.len() + 1;
        }
        self.lines = lines;
        self.concealed = concealed;
        self.leads = leads;
    }

    /// The hidden heading marker that boundary `c` is in or at either end of.
    fn lead_at(&self, c: usize) -> Option<Range<usize>> {
        self.leads.iter().find(|r| r.start <= c && c <= r.end).cloned()
    }

    /// Boundary `c` moved past a hidden heading marker it is inside of (or at the start of): the heading's first visible
    /// char, where typing keeps the line a heading.
    fn past_lead(&self, c: usize) -> usize {
        self.lead_at(c).map_or(c, |r| r.end)
    }

    /// A range about to be typed or pasted over that starts at or inside a heading's hidden marker (Shift+Cmd+Left from
    /// the end, a triple-click, a whole-line selection): the part after the marker, so the line keeps its heading style.
    /// Backspace at the start of an empty heading is the explicit way to take the marker away, as on the reference.
    fn keep_lead(&self, r: Range<usize>) -> Range<usize> {
        match self.leads.iter().find(|l| l.start <= r.start && r.start < l.end) {
            Some(l) => l.end..r.end.max(l.end),
            None => r,
        }
    }

    /// The selection as typing or pasting replaces it (`keep_lead`).
    fn selection_to_replace(&self) -> Option<Range<usize>> {
        self.selection().map(|r| self.keep_lead(r))
    }

    /// The selection as a delete removes it: the heading's marker stays (`keep_lead`), unless the selection holds
    /// nothing but the marker, which then goes as Backspace at an empty heading's start takes it.
    fn selection_to_delete(&self) -> Option<Range<usize>> {
        self.selection().map(|r| self.delete_keeping_lead(r))
    }

    /// `r` as a delete removes it (a selection, or Cmd+Backspace to the line start): `keep_lead`, or all of `r` when
    /// that would leave nothing to delete.
    fn delete_keeping_lead(&self, r: Range<usize>) -> Range<usize> {
        let kept = self.keep_lead(r.clone());
        if kept.is_empty() {
            r
        } else {
            kept
        }
    }

    /// True when char `c` is a hidden emphasis marker.
    fn concealed_at(&self, c: usize) -> bool {
        self.concealed.iter().any(|r| r.contains(&c))
    }

    /// The boundary `c` taken back over the hidden markers before it: the first of the boundaries that sit at the same
    /// place on screen, where a click puts the caret. Typing there takes the style of the char before it.
    fn settle(&self, mut c: usize) -> usize {
        // inside a heading's hidden marker: its end, never in front of it (typing there would end the heading)
        if let Some(r) = self.lead_at(c) {
            return r.end;
        }
        while c > 0 && self.concealed_at(c - 1) && !self.leads.iter().any(|r| r.end == c) {
            c -= 1;
        }
        c
    }

    /// The Left arrow from boundary `c`: one visible char back, over hidden markers.
    fn step_left(&self, c: usize) -> usize {
        let c = self.settle(c);
        // at a heading's first visible char the step goes over its hidden marker to the end of the line before
        let c = self.leads.iter().find(|r| r.end == c).map_or(c, |r| r.start);
        if c == 0 {
            return 0;
        }
        self.settle(super::edit::prev_boundary(&self.text, c))
    }

    /// The Right arrow from boundary `c`: past the next visible char, with the hidden markers in front of it.
    fn step_right(&self, c: usize) -> usize {
        let n = self.len();
        let mut to = super::edit::next_boundary(&self.text, c);
        while to < n && to > 0 && self.concealed_at(to - 1) {
            to = super::edit::next_boundary(&self.text, to);
        }
        self.past_lead(to)
    }

    /// A delete of the visible chars `r`: when they are all the text of one styled run (`md::spans`), its pair of hidden
    /// markers goes too, so no empty `****` is left to show. Equal markers of two runs on either side (the space in
    /// `**a** **b**`) are not a pair, and stay.
    fn with_empty_pair(&self, r: Range<usize>) -> Range<usize> {
        if !self.markdown || r.is_empty() {
            return r;
        }
        let para = self.paragraph_of(r.start);
        if r.end > para.end {
            return r;
        }
        let line: String = self.text.chars().skip(para.start).take(para.len()).collect();
        // the inline content starts after the block marker, except on the first paragraph (the title), as `layout` reads it
        let prefix = if para.start == 0 { 0 } else { md::classify(&line).prefix };
        let all = md::spans(&line, prefix);
        let inner = r.start - para.start..r.end - para.start;
        let Some(k) = all.iter().position(|sp| !sp.marker && sp.style != Style::Plain && sp.style != Style::Math && sp.range == inner) else { return r };
        match (k.checked_sub(1).and_then(|m| all.get(m)), all.get(k + 1)) {
            (Some(open), Some(close))
                if open.marker && close.marker && open.style == all[k].style && close.style == all[k].style && self.concealed_at(para.start + open.range.start) =>
            {
                para.start + open.range.start..para.start + close.range.end
            }
            _ => r,
        }
    }

    /// Backspace with nothing selected: the visible char before the caret (hidden markers are passed over).
    fn back_range(&self) -> Option<Range<usize>> {
        // at a heading's first visible char Backspace takes its hidden marker away: the line becomes body text
        if let Some(r) = self.lead_at(self.caret) {
            return Some(r);
        }
        let b = self.settle(self.caret);
        (b > 0).then(|| self.with_empty_pair(super::edit::prev_boundary(&self.text, b)..b))
    }

    /// Forward Delete with nothing selected: the visible char after the caret.
    fn forward_range(&self) -> Range<usize> {
        let n = self.len();
        let mut a = self.caret.min(n);
        while a < n && self.concealed_at(a) {
            a += 1;
        }
        if a >= n {
            return self.caret..self.caret;
        }
        self.with_empty_pair(a..super::edit::next_boundary(&self.text, a))
    }

    /// The line holding the boundary before char `c`.
    fn line_of(&self, c: usize) -> usize {
        self.lines.iter().position(|l| c < l.range.end).unwrap_or(self.lines.len() - 1)
    }

    /// The wrapped line the caret is on.
    pub fn caret_line(&self) -> usize {
        self.line_of(self.caret)
    }

    /// The caret's box as (x, y, w, h) relative to the text origin.
    pub fn caret_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let l = &self.lines[self.line_of(self.caret)];
        let ch = ui.pt(l.pt * 1.25);
        (l.x_at(self.caret), l.y + (l.h - ch) / 2.0, ui.pt(2.0), ch)
    }

    /// The char boundary nearest to (x, y) px from the text origin.
    pub fn position_at(&self, x: f32, y: f32) -> usize {
        let i = self.lines.iter().position(|l| y < l.y + l.h).unwrap_or(self.lines.len() - 1);
        let l = &self.lines[i];
        let last = l.range.end - if l.text.chars().count() < l.range.len() { 1 } else { 0 };
        let mut best = (l.range.start, f32::MAX);
        let mut px = 0.0;
        for c in l.range.start..=last {
            if (px - x).abs() < best.1 {
                best = (c, (px - x).abs());
            }
            if let Some(a) = l.adv.get(c - l.range.start) {
                px += a;
            }
        }
        // a soft-wrapped line's trailing space belongs to the next line
        if last == l.range.end && last < self.len() && best.0 == last && l.text.ends_with(' ') {
            return last;
        }
        self.past_lead(best.0)
    }

    /// Focuses and moves the caret to the boundary nearest the tap.
    pub fn tap(&mut self, x: f32, y: f32, t: f64) {
        self.focused = true;
        self.blink_from = t;
        self.anchor = None;
        self.caret = self.position_at(x, y);
    }

    /// Start and end lollipops of the selection as (bar top, bar bottom, x,
    /// dot centre y), px from the text origin.
    fn handles(&self, ui: &Ui) -> Option<[(f32, f32, f32, f32); 2]> {
        let r = self.selection()?;
        let (a, b) = (&self.lines[self.line_of(r.start)], &self.lines[self.line_of(r.end)]);
        let dot = ui.pt(DOT_PT);
        Some([(a.y, a.y + a.h, a.x_at(r.start), a.y - dot / 2.0), (b.y, b.y + b.h, b.x_at(r.end), b.y + b.h + dot / 2.0)])
    }

    fn grab_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Grab> {
        let hs = self.handles(ui)?;
        let reach = ui.pt(HANDLE_HIT_PT);
        let d = |h: (f32, f32, f32, f32)| (x - h.2).hypot(y - h.3);
        let (ds, de) = (d(hs[0]), d(hs[1]));
        (ds.min(de) <= reach).then_some(if ds <= de { Grab::Start } else { Grab::End })
    }

    /// A finger down at (x, y) px from the text origin: grabs a selection
    /// handle if one is there. True when the text takes the touch.
    pub fn touch_down(&mut self, ui: &Ui, x: f32, y: f32) -> bool {
        let grab = self.grab_at(ui, x, y).unwrap_or(Grab::Tap);
        self.touch = Some(Touch { x, y, grab, moved: false });
        true
    }

    /// The finger moved: drags a handle (ends never cross, one char at
    /// least stays selected) or, after a long press, the caret.
    pub fn touch_move(&mut self, ui: &Ui, x: f32, y: f32) {
        let Some(mut t) = self.touch else { return };
        if !t.moved && (x - t.x).hypot(y - t.y) > ui.pt(SLOP_PT) {
            t.moved = true;
        }
        let i = self.position_at(x, y);
        match t.grab {
            Grab::Start | Grab::End => {
                if let Some(r) = self.selection() {
                    let (s, e) = if t.grab == Grab::Start { (i.min(r.end - 1), r.end) } else { (r.start, i.max(r.start + 1)) };
                    self.anchor = Some(s);
                    self.caret = e;
                }
            }
            Grab::Caret => {
                self.anchor = None;
                self.caret = i;
            }
            Grab::Tap => {}
        }
        self.touch = Some(t);
    }

    /// A press held still long enough: the caret now follows the finger
    /// under the loupe. True when the screen changed.
    pub fn long_press(&mut self, x: f32, y: f32, t: f64) -> bool {
        match self.touch {
            Some(p) if p.grab == Grab::Tap && !p.moved => {
                self.touch = Some(Touch { grab: Grab::Caret, ..p });
                self.tap(x, y, t);
                true
            }
            _ => false,
        }
    }

    /// The finger lifted. A tap places the caret; a second tap selects the
    /// word, a third the paragraph. True when it was a tap (not a drag).
    pub fn touch_up(&mut self, ui: &Ui, x: f32, y: f32, t: f64) -> bool {
        let Some(p) = self.touch.take() else { return false };
        if p.grab != Grab::Tap || p.moved {
            return false;
        }
        let near = |l: Option<(f32, f32, f64, u8)>| l.filter(|&(lx, ly, lt, _)| t - lt <= MULTI_TAP_MS && (x - lx).hypot(y - ly) <= ui.pt(MULTI_TAP_PT));
        let n = near(self.last_tap).map_or(1, |l| l.3 + 1);
        self.last_tap = Some((x, y, t, n));
        let c = self.position_at(x, y);
        self.focused = true;
        self.blink_from = t;
        match n {
            1 => self.tap(x, y, t),
            2 => {
                let (a, b) = super::edit::word_at(&self.text, c);
                self.select(a..b);
            }
            _ => {
                let r = self.paragraph_of(c);
                self.select(r);
                self.last_tap = None;
            }
        }
        true
    }

    /// True while a finger drags the caret or a handle (the loupe is up).
    pub fn loupe_up(&self) -> bool {
        self.touch.is_some_and(|t| t.grab != Grab::Tap)
    }

    /// Selects `range` (char indices, clamped); the caret goes to its end.
    pub fn select(&mut self, range: Range<usize>) {
        let n = self.len();
        let (a, b) = (range.start.min(n), range.end.min(n));
        self.caret = b;
        self.anchor = (a != b).then_some(a);
    }

    // --- the desktop: mouse and hardware-keyboard selection (`set_pointer_mode`) ---

    /// Turns the mouse-and-keyboard behaviour on (a desktop window) or off (touch, the default): with it on, the
    /// selection has no lollipop handles and `press`, `drag_to`, `release` and `key_combo` drive the caret.
    pub fn set_pointer_mode(&mut self, on: bool) {
        self.pointer = on;
    }

    pub fn pointer_mode(&self) -> bool {
        self.pointer
    }

    /// The char index of byte offset `b` (clamped to the text).
    fn char_of_byte(&self, b: usize) -> usize {
        let b = bifrost_ui::text::floor_boundary(&self.text, b.min(self.text.len()));
        self.text[..b].chars().count()
    }

    /// The word a double click at char `c` selects (`TextBuffer::select_word_at`), as a char range.
    pub fn word_range(&self, c: usize) -> Range<usize> {
        let mut buf = bifrost_ui::text::TextBuffer::from_text(&self.text);
        buf.select_word_at(self.byte_at(c));
        let r = buf.selection();
        self.char_of_byte(r.start)..self.char_of_byte(r.end)
    }

    /// The paragraph a triple click at char `c` selects (`TextBuffer::select_line_at`), without its newline.
    pub fn line_range(&self, c: usize) -> Range<usize> {
        let mut buf = bifrost_ui::text::TextBuffer::from_text(&self.text);
        buf.select_line_at(self.byte_at(c));
        let r = buf.selection();
        self.char_of_byte(r.start)..self.char_of_byte(r.end)
    }

    /// The unit `clicks` presses at char `c` select: the caret, the word, or the paragraph.
    fn unit_at(&self, c: usize, clicks: u8) -> Range<usize> {
        match clicks {
            1 => c..c,
            2 => self.word_range(c),
            _ => self.line_range(c),
        }
    }

    /// Sets the selection from its fixed end `anchor` to the moving end `head` (no selection when they meet).
    fn set_ends(&mut self, anchor: usize, head: usize) {
        let n = self.len();
        let (anchor, head) = (anchor.min(n), head.min(n));
        self.caret = head;
        self.anchor = (anchor != head).then_some(anchor);
    }

    /// The mouse button went down at (x, y) px from the text origin at `t` ms: one click places the caret, a double
    /// click selects the word and a triple click the paragraph (`ClickCounter`). With `extend` (Shift held) a single
    /// click moves the selection's free end there instead. A drag that follows grows the selection by the same unit.
    pub fn press(&mut self, ui: &Ui, x: f32, y: f32, t: f64, extend: bool) {
        self.commit_marked(ui, t);
        self.focused = true;
        self.blink_from = t;
        self.end_typing();
        let k = ui.pt(1.0).max(f32::EPSILON);
        let clicks = self.clicks.press(t, x / k, y / k);
        let c = self.position_at(x, y);
        if extend && clicks == 1 {
            let anchor = self.anchor.unwrap_or(self.caret);
            self.set_ends(anchor, c);
            self.drag = Some((anchor..anchor, 1));
            return;
        }
        let unit = self.unit_at(c, clicks);
        if clicks == 1 {
            self.set_ends(c, c);
        } else {
            self.set_ends(unit.start, unit.end);
        }
        self.drag = Some((unit, clicks));
    }

    /// The mouse moved with the button down: the selection runs from where the press began to (x, y), by characters
    /// after one click, by whole words or paragraphs after two or three (`text::extend_unit`). False when no press
    /// is in progress.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some((origin, clicks)) = self.drag.clone() else { return false };
        let c = self.position_at(x, y);
        let (anchor, head) = if clicks == 1 { (origin.start, c) } else { bifrost_ui::text::extend_unit(origin, self.unit_at(c, clicks)) };
        self.set_ends(anchor, head);
        true
    }

    /// The mouse button came up; the selection stays.
    pub fn release(&mut self) {
        self.drag = None;
    }

    /// True while the mouse button is down on the text.
    pub fn pressed(&self) -> bool {
        self.drag.is_some()
    }

    /// The char where the wrapped line holding the caret starts, and where it ends (before its newline, or before the
    /// space it hangs at a soft wrap).
    fn visual_line(&self) -> (usize, usize) {
        let l = &self.lines[self.line_of(self.caret)];
        let mut end = l.range.end;
        if l.text.chars().count() < l.range.len() || (end < self.len() && l.text.ends_with(' ')) {
            end = end.saturating_sub(1).max(l.range.start);
        }
        (l.range.start, end)
    }

    /// Selects every character.
    pub fn select_all(&mut self) {
        self.set_ends(0, self.len());
    }

    /// A key combination from a hardware keyboard, as `bifrost_ui::text::binding` reads it: the arrows, Home, End and
    /// Page Up and Down move (Shift extends the selection, Option or Ctrl by word, Cmd to the line or text ends),
    /// Backspace and Delete delete (by word with Option, to the line start with Cmd), Cmd+A selects all, Cmd+Z and
    /// Shift+Cmd+Z undo and redo, and the emacs Ctrl keys work. `None` when the combination is not an editing key;
    /// `Some(true)` when the text changed.
    pub fn key_combo(&mut self, ui: &Ui, k: bifrost_ui::keys::KeyCombo, t: f64) -> Option<bool> {
        use bifrost_ui::text::{self as kit, EditCommand::*};
        let (cmd, extend) = kit::binding(k)?;
        self.commit_marked(ui, t);
        self.blink_from = t;
        let sel = self.selection();
        let head = self.caret;
        let byte = self.byte_at(head);
        let delete = |ta: &mut TextArea, range: Range<usize>| {
            let range = ta.selection_to_delete().unwrap_or_else(|| ta.delete_keeping_lead(range));
            if range.is_empty() {
                return false;
            }
            ta.replace_range(ui, range, "", t, false);
            true
        };
        let to = match cmd {
            MoveLeft if !extend && sel.is_some() => sel.as_ref().map_or(head, |r| r.start),
            MoveRight if !extend && sel.is_some() => sel.as_ref().map_or(head, |r| r.end),
            MoveLeft => self.step_left(head),
            MoveRight => self.step_right(head),
            MoveUp | MoveDown | MovePageUp | MovePageDown => {
                let down = matches!(cmd, MoveDown | MovePageDown);
                let steps = if matches!(cmd, MovePageUp | MovePageDown) { kit::PAGE_LINES } else { 1 };
                let anchor = self.anchor.unwrap_or(head);
                if !extend {
                    if let Some(r) = &sel {
                        self.caret = if down { r.end } else { r.start };
                    }
                }
                for _ in 0..steps {
                    self.vertical(down);
                }
                let to = self.caret;
                if extend {
                    self.set_ends(anchor, to);
                } else {
                    self.set_ends(to, to);
                }
                return Some(false);
            }
            MoveWordLeft => self.char_of_byte(kit::prev_word_start(&self.text, byte)),
            MoveWordRight => self.char_of_byte(kit::next_word_end(&self.text, byte)),
            MoveLineStart => self.visual_line().0,
            MoveLineEnd => self.visual_line().1,
            MoveDocumentStart => 0,
            MoveDocumentEnd => self.len(),
            SelectAll => {
                self.select_all();
                return Some(false);
            }
            Undo => return Some(self.undo(ui, t)),
            Redo => return Some(self.redo(ui, t)),
            DeleteBackward => return Some(self.edit(ui, &Out::Backspace, t)),
            DeleteForward => {
                let range = self.forward_range();
                return Some(delete(self, range));
            }
            DeleteWordBackward => {
                let range = self.char_of_byte(kit::prev_word_start(&self.text, byte))..head;
                return Some(delete(self, range));
            }
            DeleteWordForward => {
                let range = head..self.char_of_byte(kit::next_word_end(&self.text, byte));
                return Some(delete(self, range));
            }
            DeleteToLineStart => {
                let range = self.char_of_byte(kit::line_start(&self.text, byte))..head;
                return Some(delete(self, range));
            }
            KillLine => {
                let end = self.char_of_byte(kit::line_end(&self.text, byte));
                // at the end of a paragraph Ctrl+K joins the next one
                let end = if end == head { super::edit::next_boundary(&self.text, head) } else { end };
                return Some(delete(self, head..end));
            }
            OpenLine => {
                self.replace_range(ui, head..head, "\n", t, false);
                self.set_caret(head);
                return Some(true);
            }
            Transpose => {
                let n = self.len();
                if n < 2 || head == 0 {
                    return Some(false);
                }
                let at = if head >= n { n - 1 } else { head };
                let pair: String = self.text.chars().skip(at - 1).take(2).collect();
                let swapped: String = pair.chars().rev().collect();
                self.replace_range(ui, at - 1..at + 1, &swapped, t, false);
                return Some(true);
            }
            // a note keeps no kill ring
            Yank => return None,
        };
        if extend {
            let anchor = self.anchor.unwrap_or(head);
            self.set_ends(anchor, to);
        } else {
            self.set_ends(to, to);
        }
        Some(false)
    }

    /// The selected char range, if any.
    pub fn selection(&self) -> Option<Range<usize>> {
        self.anchor.map(|a| a.min(self.caret)..a.max(self.caret))
    }

    pub fn selected_text(&self) -> String {
        self.selection().map(|r| self.text.chars().skip(r.start).take(r.len()).collect()).unwrap_or_default()
    }

    // --- marked text (input methods) ------------------------------------------------------------------------------

    /// The char range of the marked text while an input method composes (`markedRange`).
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.as_ref().map(|(r, _)| r.clone())
    }

    /// The chars in `range`, as a string.
    fn slice(&self, range: Range<usize>) -> String {
        self.text.chars().skip(range.start).take(range.len()).collect()
    }

    /// Replaces chars `range` with `s` without recording anything (the composition is not an undo step until it is
    /// committed). The caller lays out.
    fn splice(&mut self, range: Range<usize>, s: &str) {
        let (a, b) = (self.byte_at(range.start), self.byte_at(range.end));
        self.text.replace_range(a..b, s);
    }

    /// `setMarkedText:selectedRange:` from an input method (`ToApp::SetMarkedText`): `text` replaces the marked text, or
    /// the selection when nothing is marked yet, and is drawn underlined. `sel_start` and `sel_len` select inside `text`
    /// in UTF-8 bytes (the caret when `sel_len` is 0). An empty `text` ends the composition and puts back what it
    /// replaced. Nothing is recorded for undo until `unmark_text` commits.
    pub fn set_marked_text(&mut self, ui: &Ui, text: &str, sel_start: usize, sel_len: usize, t: f64) {
        self.end_typing();
        self.drag = None;
        let (range, replaced) = match self.marked.take() {
            Some(m) => m,
            None => {
                let r = self.selection_to_replace().unwrap_or(self.caret..self.caret);
                let replaced = self.slice(r.clone());
                (r, replaced)
            }
        };
        let start = range.start;
        self.blink_from = t;
        if text.is_empty() {
            self.splice(range, &replaced);
            self.caret = start + replaced.chars().count();
            self.anchor = None;
            self.layout(ui);
            return;
        }
        self.splice(range, text);
        let n = text.chars().count();
        self.marked = Some((start..start + n, replaced));
        // the selection inside the marked text, bytes to chars (a cut inside a character rounds to the end)
        let chars_to = |b: usize| text.get(..b.min(text.len())).map_or(n, |p| p.chars().count());
        let (a, b) = (start + chars_to(sel_start), start + chars_to(sel_start.saturating_add(sel_len)));
        self.caret = b;
        self.anchor = (b > a).then_some(a);
        self.layout(ui);
    }

    /// `unmarkText` (`ToApp::UnmarkText`): the composition ends. With `commit` the marked text stays as typed text (one
    /// "Typing" undo step that puts back what it replaced); otherwise it is removed and the replaced text comes back.
    /// False when nothing was marked.
    pub fn unmark_text(&mut self, ui: &Ui, commit: bool, t: f64) -> bool {
        let Some((range, replaced)) = self.marked.take() else { return false };
        let marked = self.slice(range.clone());
        let start = range.start;
        // the text goes back to what it was before the composition, so the commit is an ordinary recorded edit
        self.splice(range, &replaced);
        let end = start + replaced.chars().count();
        self.caret = end;
        self.anchor = (end > start).then_some(start);
        if commit && !marked.is_empty() {
            self.replace_range(ui, start..end, &marked, t, true);
        } else {
            self.anchor = None;
            self.layout(ui);
        }
        self.blink_from = t;
        true
    }

    /// `delete_surrounding_text` (`ToApp::DeleteSurrounding`): deletes `before` UTF-8 bytes before and `after` bytes
    /// after the selection (rounded out to characters), one undo step. True when the text changed.
    pub fn delete_surrounding(&mut self, ui: &Ui, before: usize, after: usize, t: f64) -> bool {
        self.commit_marked(ui, t);
        let sel = self.selection().unwrap_or(self.caret..self.caret);
        let (a, b) = (self.byte_at(sel.start), self.byte_at(sel.end));
        let mut from = a.saturating_sub(before);
        while !self.text.is_char_boundary(from) {
            from -= 1;
        }
        let mut to = b.saturating_add(after).min(self.text.len());
        while !self.text.is_char_boundary(to) {
            to += 1;
        }
        let (from, to) = (self.text[..from].chars().count(), self.text[..to].chars().count());
        if from == sel.start && to == sel.end {
            return false;
        }
        let kept = self.slice(sel.clone());
        self.replace_range(ui, from..to, &kept, t, false);
        // the selection stays on the same text
        self.caret = from + kept.chars().count();
        self.anchor = (!kept.is_empty()).then_some(from);
        true
    }

    /// Commits a composition in progress before another edit, a click or a move (an input method's marked text is
    /// kept as typed text, as the reference does when the user acts elsewhere).
    fn commit_marked(&mut self, ui: &Ui, t: f64) {
        if self.marked.is_some() {
            self.unmark_text(ui, true, t);
        }
    }

    /// Replaces the selection (or inserts at the caret) with `s`.
    pub fn replace_selection(&mut self, ui: &Ui, s: &str, t: f64) {
        self.commit_marked(ui, t);
        let at = self.past_lead(self.caret);
        let range = self.selection_to_replace().unwrap_or(at..at);
        self.replace_range(ui, range, s, t, false);
    }

    /// Replaces `range` with `s`, records it for undo and relays out.
    /// `coalesce` merges contiguous typing into the previous undo step.
    pub fn replace_range(&mut self, ui: &Ui, range: Range<usize>, s: &str, t: f64, coalesce: bool) {
        let n = self.len();
        let range = range.start.min(n)..range.end.min(n);
        let removed: String = self.text.chars().skip(range.start).take(range.len()).collect();
        if removed.is_empty() && s.is_empty() {
            return;
        }
        let edit = Edit { at: range.start, removed, inserted: s.to_string(), caret_before: self.caret };
        self.apply(&edit, true, ui);
        let merged = coalesce && self.typing.as_mut().is_some_and(|prev| prev.merge(&edit));
        if merged {
            self.history.register("Typing", undo_step(&edit), redo_step(&edit));
        } else {
            self.end_typing();
            if coalesce {
                self.history.begin_group("Typing");
                self.typing = Some(edit.clone());
                self.history.register("Typing", undo_step(&edit), redo_step(&edit));
            } else {
                self.history.register("", undo_step(&edit), redo_step(&edit));
            }
        }
        self.blink_from = t;
    }

    /// Closes the open typing group, so the next edit starts a new undo step.
    fn end_typing(&mut self) {
        self.typing = None;
        self.history.end_group();
    }

    /// Applies (or reverts) an edit to the text and places the caret.
    fn apply(&mut self, e: &Edit, forward: bool, ui: &Ui) {
        let (old, new) = if forward { (&e.removed, &e.inserted) } else { (&e.inserted, &e.removed) };
        let (a, b) = (self.byte_at(e.at), self.byte_at(e.at + old.chars().count()));
        self.text.replace_range(a..b, new);
        self.caret = if forward { e.at + new.chars().count() } else { e.caret_before };
        self.anchor = None;
        self.layout(ui);
    }

    /// Reverts the last edit; false when there is none.
    pub fn undo(&mut self, ui: &Ui, t: f64) -> bool {
        self.step_history(ui, t, |h, d| h.undo(d).is_some())
    }

    pub fn redo(&mut self, ui: &Ui, t: f64) -> bool {
        self.step_history(ui, t, |h, d| h.redo(d).is_some())
    }

    /// Runs an undo or redo step on the text and caret; true when something changed.
    fn step_history(&mut self, ui: &Ui, t: f64, step: impl FnOnce(&mut UndoManager<Doc>, &mut Doc) -> bool) -> bool {
        self.commit_marked(ui, t);
        self.end_typing();
        let mut doc = Doc { text: std::mem::take(&mut self.text), caret: self.caret };
        let moved = step(&mut self.history, &mut doc);
        self.text = doc.text;
        self.caret = doc.caret;
        if moved {
            self.anchor = None;
            self.layout(ui);
            self.blink_from = t;
        }
        moved
    }

    /// Moves the caret a line up or down, keeping its x.
    fn vertical(&mut self, down: bool) {
        let i = self.line_of(self.caret);
        let x = self.lines[i].x_at(self.caret);
        let j = if down { i + 1 } else { i.wrapping_sub(1) };
        if let Some(l) = self.lines.get(j) {
            self.caret = self.position_at(x, l.y + l.h / 2.0);
        } else {
            self.caret = if down { self.len() } else { 0 };
        }
    }

    /// Enter at the end of a list item: starts the next item, or ends the
    /// list when the item is empty. One undo step. False when the caret is
    /// not at the end of a list line (a plain newline applies instead).
    fn continue_list(&mut self, ui: &Ui, t: f64) -> bool {
        let para = self.paragraph_of(self.caret);
        if self.caret != para.end {
            return false;
        }
        let line: String = self.text.chars().skip(para.start).take(para.len()).collect();
        match md::continue_list(&line) {
            md::Continue::None => false,
            md::Continue::Marker(m) => {
                self.replace_range(ui, self.caret..self.caret, &format!("\n{m}"), t, false);
                true
            }
            md::Continue::End(n) => {
                self.replace_range(ui, para.start..para.start + n, "", t, false);
                true
            }
        }
    }

    /// Applies a key from the on-screen keyboard; true when the text changed.
    pub fn edit(&mut self, ui: &Ui, out: &Out, t: f64) -> bool {
        self.commit_marked(ui, t);
        self.blink_from = t;
        match out {
            Out::Text(s) => {
                let at = self.past_lead(self.caret);
                let range = self.selection_to_replace().unwrap_or(at..at);
                self.replace_range(ui, range, s, t, true);
                true
            }
            Out::Enter => {
                if self.markdown && self.selection().is_none() && self.continue_list(ui, t) {
                    return true;
                }
                // Return at a heading's first visible char opens a line above it: the heading keeps its marker. An empty
                // heading (nothing after its marker) has its first visible char at its end: the new line goes below it.
                let at = self.lead_at(self.caret).map_or(self.caret, |r| if self.paragraph_of(r.end).end == r.end { r.end } else { r.start });
                let range = self.selection_to_replace().unwrap_or(at..at);
                self.replace_range(ui, range, "\n", t, false);
                true
            }
            Out::Backspace => {
                let range = match self.selection_to_delete() {
                    Some(r) => r,
                    None => match self.back_range() {
                        Some(r) => r,
                        None => return false,
                    },
                };
                self.replace_range(ui, range, "", t, true);
                true
            }
            Out::Named("Left" | "ShiftLeft") => {
                self.caret = self.selection().map_or_else(|| self.step_left(self.caret), |r| r.start);
                self.anchor = None;
                false
            }
            Out::Named("Right" | "ShiftRight") => {
                self.caret = self.selection().map_or_else(|| self.step_right(self.caret), |r| r.end);
                self.anchor = None;
                false
            }
            Out::Named("Up") => {
                self.anchor = None;
                self.vertical(false);
                false
            }
            Out::Named("Down") => {
                self.anchor = None;
                self.vertical(true);
                false
            }
            // hardware-keyboard shortcuts (Ctrl/Cmd + C, X, V, A, Z, Shift+Z)
            Out::Named("Copy") => {
                if self.selection().is_some() {
                    super::clipboard::set_text(&self.selected_text());
                }
                false
            }
            Out::Named("Cut") => match self.selection_to_delete() {
                Some(r) => {
                    super::clipboard::set_text(&self.selected_text());
                    self.replace_range(ui, r, "", t, false);
                    true
                }
                None => false,
            },
            Out::Named("Paste") => match super::clipboard::text() {
                Some(text) => {
                    self.replace_selection(ui, &text, t);
                    true
                }
                None => false,
            },
            Out::Named("SelectAll") => {
                self.select(0..self.len());
                false
            }
            Out::Named("Undo") => self.undo(ui, t),
            Out::Named("Redo") => self.redo(ui, t),
            Out::Hide | Out::Named(_) => false,
        }
    }

    pub fn caret_on(&self, t: f64) -> bool {
        (((t - self.blink_from) / BLINK_MS).floor() as i64) % 2 == 0
    }

    /// When the caret next toggles, for frame scheduling; None when unfocused.
    pub fn next_blink(&self, t: f64) -> Option<f64> {
        self.focused.then(|| self.blink_from + (((t - self.blink_from) / BLINK_MS).floor() + 1.0) * BLINK_MS)
    }

    /// Draws the lines that fall inside `s` with the text origin at (x, y),
    /// plus the selection and (when focused) the caret. Lines are rendered
    /// on first use and kept.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, x: f32, y: f32, t: f64) {
        let (top, bottom) = (s.y0 as f32, s.h as f32);
        let sel = self.selection();
        for l in &mut self.lines {
            let ly = y + l.y;
            if l.h <= 0.0 || ly + l.h <= top || ly >= bottom {
                continue;
            }
            let shown = l.range.start + l.text.chars().count();
            for r in &self.links {
                let (a, b) = (r.start.max(l.range.start), r.end.min(shown));
                if a < b {
                    let (x0, x1) = (l.x_at(a), l.x_at(b));
                    s.fill_rrect(x + x0, ly + l.h - ui.pt(3.0), (x1 - x0).max(ui.pt(2.0)), ui.pt(1.0), 0.0, self.tint);
                }
            }
            // the marked text of a composition: underlined in the text colour, thicker than a link
            if let Some((r, _)) = &self.marked {
                let (a, b) = (r.start.max(l.range.start), r.end.min(shown));
                if a < b {
                    let (x0, x1) = (l.x_at(a), l.x_at(b));
                    s.fill_rrect(x + x0, ly + l.h - ui.pt(3.0), (x1 - x0).max(ui.pt(2.0)), ui.pt(2.0), 0.0, self.color);
                }
            }
            for (hi, r) in self.highlights.iter().enumerate() {
                let (a, b) = (r.start.max(l.range.start), r.end.min(shown));
                if a < b {
                    let colour = if self.current_hl == Some(hi) { rgba(255, 159, 10, 150) } else { rgba(255, 214, 10, 80) };
                    let (x0, x1) = (l.x_at(a), l.x_at(b));
                    s.fill_rrect(x + x0, ly, (x1 - x0).max(ui.pt(2.0)), l.h, 0.0, colour);
                }
            }
            if let Some(r) = &sel {
                let (a, b) = (r.start.max(l.range.start), r.end.min(l.range.end));
                if a < b {
                    let (x0, x1) = (l.x_at(a), if b == l.range.end && l.text.chars().count() < l.range.len() { l.x_at(b - 1) + ui.pt(4.0) } else { l.x_at(b) });
                    // the selection: a wash of Settings > Appearance > Highlight Colour under the text
                    s.fill_rrect(x + x0, ly, (x1 - x0).max(ui.pt(2.0)), l.h, 0.0, crate::gfx::scale(bifrost_render::ui::highlight_now(), SELECTION_ALPHA));
                }
            }
            if let Some(name) = &l.image {
                if let Some(img) = self.images.get(name) {
                    s.blit(img, x as i32, (ly + ui.pt(6.0)) as i32, 255, 255);
                }
                continue;
            }
            if let Some(lay) = &l.display {
                let baseline = ly + (l.h - lay.ascent - lay.depth) / 2.0 + lay.ascent;
                let ox = x + (self.w - lay.width) / 2.0;
                paint_formula(s, ui, lay, ox, baseline, self.color);
                continue;
            }
            if l.text.is_empty() {
                continue;
            }
            if l.segs.is_empty() {
                let img = l.img.get_or_insert_with(|| ui.text(&l.text, l.pt, l.weight, self.color));
                blit_text(s, img, x as i32, (ly + (l.h - img.h as f32) / 2.0).round() as i32, self.color);
                continue;
            }
            if l.simgs.len() != l.segs.len() {
                l.simgs = vec![None; l.segs.len()];
            }
            let (pt, lh, done) = (l.pt, l.h, l.block == Block::Check(true));
            let chars: Vec<char> = l.text.chars().collect();
            for k in 0..l.segs.len() {
                let seg = l.segs[k];
                let (x0, x1) = (x + l.x_at(l.range.start + seg.a), x + l.x_at(l.range.start + seg.b));
                match seg.kind {
                    1 => {
                        let d = ui.pt(pt * 0.3);
                        s.fill_rrect(x0 + ((x1 - x0) - d) / 2.0, ly + (lh - d) / 2.0, d, d, d / 2.0, self.color);
                    }
                    2 => {
                        let side = ui.pt(pt * 1.1).min(x1 - x0);
                        let (bx, by, r) = (x0, ly + (lh - side) / 2.0, ui.pt(4.0));
                        if done {
                            s.fill_rrect(bx, by, side, side, r, self.tint);
                            let th = ui.pt(1.8);
                            let tick = either(segment((bx + side * 0.25, by + side * 0.52), (bx + side * 0.43, by + side * 0.7), th), segment((bx + side * 0.43, by + side * 0.7), (bx + side * 0.76, by + side * 0.3), th));
                            paint(s, (bx, by, side, side), BLACK, tick);
                        } else {
                            let b = ui.pt(1.5);
                            s.fill_rrect(bx, by, side, side, r, SECONDARY);
                            s.fill_rrect(bx + b, by + b, side - 2.0 * b, side - 2.0 * b, (r - b).max(0.0), BLACK);
                        }
                    }
                    6 => s.fill_rrect(x0 + ui.pt(1.0), ly, ui.pt(3.0), lh, ui.pt(1.5), SECONDARY),
                    // inline math: the formula is drawn below, over its reserved width
                    7 => {}
                    _ => {
                        if seg.kind == 5 {
                            s.fill_rrect(x0, ly, x1 - x0, lh, ui.pt(2.0), rgba(52, 199, 89, 100));
                        }
                        let text: String = chars[seg.a..seg.b].iter().collect();
                        let img = l.simgs[k].get_or_insert_with(|| ui.text(&text, pt, seg.weight, seg.color));
                        blit_text(s, img, x0 as i32, (ly + (lh - img.h as f32) / 2.0).round() as i32, seg.color);
                        // strike-through and underline are a rule in the text colour
                        let rule = if seg.color == super::ui::WHITE { super::ui::LABEL.now() } else { seg.color };
                        let thick = ui.pt(1.2).max(1.0);
                        match seg.kind {
                            3 => s.fill_rrect(x0, ly + lh / 2.0, x1 - x0, thick, 0.0, rule),
                            4 => s.fill_rrect(x0, ly + lh * 0.82, x1 - x0, thick, 0.0, rule),
                            _ => {}
                        }
                    }
                }
            }
        }
        // inline formulas: on the line's text baseline, at the place their source starts
        for l in &self.lines {
            if l.maths.is_empty() || l.h <= 0.0 {
                continue;
            }
            let lm = ui.font.line_metrics(line_px(ui, l.pt));
            let baseline = y + l.y + l.h / 2.0 + (lm.ascent - lm.descent) / 2.0;
            for (a, lay) in &l.maths {
                let x0 = x + l.x_at(l.range.start + a);
                paint_formula(s, ui, lay, x0, baseline, self.color);
            }
        }
        if self.focused && self.caret_on(t) {
            let (cx, cy, cw, ch) = self.caret_rect(ui);
            s.fill_rrect(x + cx, y + cy, cw, ch, cw / 2.0, self.tint);
        }
        // the lollipop handles are for fingers: a mouse-driven text shows the plain highlight
        if let Some(hs) = self.handles(ui).filter(|_| !self.pointer) {
            let (bar, dot) = (ui.pt(BAR_PT), ui.pt(DOT_PT));
            for (i, (top, bot, hx, dy)) in hs.into_iter().enumerate() {
                let (by, bh) = if i == 0 { (top - dot / 2.0, bot - top + dot / 2.0) } else { (top, bot - top + dot / 2.0) };
                s.fill_rrect(x + hx - bar / 2.0, y + by, bar, bh, 0.0, SELECT);
                s.fill_rrect(x + hx - dot / 2.0, y + dy - dot / 2.0, dot, dot, dot / 2.0, SELECT);
            }
        }
        if self.loupe_up() {
            let l = &self.lines[self.line_of(self.caret)];
            let chars: Vec<char> = l.text.chars().collect();
            let k = self.caret.saturating_sub(l.range.start).min(chars.len());
            let pre: String = chars[k.saturating_sub(8)..k].iter().collect();
            let post: String = chars[k..(k + 8).min(chars.len())].iter().collect();
            draw_loupe(s, ui, x + l.x_at(self.caret), y + l.y - ui.pt(44.0), &pre, &post, l.pt);
        }
    }
}

impl Edit {
    /// Absorbs `next` when it continues this edit: typing right after the
    /// last inserted char (stopping at a newline or at a word boundary) or
    /// backspacing right before the last removed char.
    fn merge(&mut self, next: &Edit) -> bool {
        let typed = self.removed.is_empty() && next.removed.is_empty() && !next.inserted.contains('\n') && !self.inserted.contains('\n');
        let contiguous = next.at == self.at + self.inserted.chars().count();
        let word = !self.inserted.ends_with(' ') || next.inserted.starts_with(' ');
        if typed && contiguous && word && !self.inserted.is_empty() {
            self.inserted.push_str(&next.inserted);
            return true;
        }
        let deleted = self.inserted.is_empty() && next.inserted.is_empty() && next.removed.chars().count() == 1;
        if deleted && next.at + 1 == self.at && !self.removed.is_empty() {
            self.removed.insert_str(0, &next.removed);
            self.at = next.at;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ui::WHITE;

    const TINT: u32 = 0xff0ad6ff;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn area(ui: &Ui, w: f32) -> TextArea {
        TextArea::new(ui, w, 17.0, 28.0, WHITE, TINT)
    }

    fn typed(ta: &mut TextArea, ui: &Ui, s: &str, t: f64) {
        for c in s.chars() {
            let out = if c == '\n' { Out::Enter } else { Out::Text(c.to_string()) };
            ta.edit(ui, &out, t);
        }
    }

    #[test]
    fn wraps_at_spaces_and_breaks_long_words() {
        let adv = |n: usize| vec![10.0; n];
        let chars: Vec<char> = "aa bb cc".chars().collect();
        assert_eq!(wrap_breaks(&chars, &adv(8), 55.0), vec![6, 8], "'aa bb ' then 'cc'");
        assert_eq!(wrap_breaks(&chars, &adv(8), 25.0), vec![3, 6, 8], "spaces hang past the width");
        let long: Vec<char> = "abcdefgh".chars().collect();
        assert_eq!(wrap_breaks(&long, &adv(8), 35.0), vec![3, 6, 8], "a long word breaks between chars");
        assert_eq!(wrap_breaks(&[], &[], 100.0), vec![0], "an empty paragraph is one empty line");
        let one: Vec<char> = "x".chars().collect();
        assert_eq!(wrap_breaks(&one, &adv(1), 1.0), vec![1], "never less than one char per line");
    }

    #[test]
    fn lays_out_paragraphs_with_a_bold_title() {
        let ui = ui();
        let mut ta = area(&ui, 300.0);
        ta.set_text(&ui, "Title\nthe quick brown fox jumps over the lazy dog\n\nend");
        let lines = ta.line_texts();
        assert_eq!(lines[0], "Title");
        assert!(lines.len() > 4, "{lines:?}");
        assert!(lines.contains(&""), "the blank paragraph keeps its line");
        assert_eq!(*lines.last().unwrap(), "end");
        let joined: String = ta.lines.iter().map(|l| ta.text.chars().skip(l.range.start).take(l.range.len()).collect::<String>()).collect();
        assert_eq!(joined, ta.text, "the line ranges tile the text");
        assert!(ta.lines[0].h > ta.lines[1].h, "title lines are taller");
        assert_eq!(ta.height(), ta.lines.iter().map(|l| l.h).sum::<f32>());
        for l in &ta.lines {
            let ink: f32 = l.adv.iter().take(l.text.trim_end().chars().count()).sum();
            assert!(ink <= 300.0, "{:?} fits", l.text);
        }
    }

    #[test]
    fn shift_with_an_arrow_moves_the_caret_like_the_plain_arrow() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        typed(&mut ta, &ui, "abc", 0.0);
        ta.edit(&ui, &Out::Named("ShiftLeft"), 1.0);
        assert_eq!(ta.caret(), 2);
        ta.edit(&ui, &Out::Named("ShiftRight"), 2.0);
        assert_eq!(ta.caret(), 3);
    }

    #[test]
    fn typing_moves_the_caret_and_backspace_deletes() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        assert!(ta.edit(&ui, &Out::Text("ab".into()), 0.0));
        ta.edit(&ui, &Out::Enter, 1.0);
        typed(&mut ta, &ui, "cd", 2.0);
        assert_eq!(ta.text(), "ab\ncd");
        assert_eq!(ta.caret(), 5);
        ta.edit(&ui, &Out::Named("Left"), 3.0);
        ta.edit(&ui, &Out::Named("Left"), 3.0);
        ta.edit(&ui, &Out::Backspace, 4.0);
        assert_eq!(ta.text(), "abcd");
        assert_eq!(ta.caret(), 2);
        ta.edit(&ui, &Out::Named("Right"), 5.0);
        ta.edit(&ui, &Out::Text("é".into()), 6.0);
        assert_eq!(ta.text(), "abcéd");
        for _ in 0..10 {
            ta.edit(&ui, &Out::Named("Left"), 7.0);
        }
        assert_eq!(ta.caret(), 0);
        assert!(!ta.edit(&ui, &Out::Backspace, 8.0), "nothing before the caret");
        assert!(!ta.edit(&ui, &Out::Hide, 8.0));
        assert!(!ta.edit(&ui, &Out::Named("Tab"), 8.0));
        assert_eq!(ta.text(), "abcéd");
    }

    #[test]
    fn up_and_down_keep_the_column_and_tap_places_the_caret() {
        let ui = ui();
        let mut ta = area(&ui, 2000.0);
        ta.set_text(&ui, "Title\nsecond line\nx");
        ta.select(17..17);
        ta.edit(&ui, &Out::Named("Up"), 0.0);
        assert_eq!(ta.caret(), 5, "the end of a long line maps to the end of the short title");
        ta.edit(&ui, &Out::Named("Down"), 0.0);
        let title_w = ta.lines[0].x_at(5);
        assert_eq!(ta.caret(), ta.position_at(title_w, ta.lines[1].y), "down keeps the title's x");
        assert!(ta.caret() > 6 && ta.caret() < 17);
        ta.select(0..0);
        ta.edit(&ui, &Out::Named("Up"), 0.0);
        assert_eq!(ta.caret(), 0, "up from the first line stays at the start");
        ta.edit(&ui, &Out::Named("Down"), 0.0);
        assert_eq!(ta.caret(), 6, "down keeps x = 0");
        ta.edit(&ui, &Out::Named("Down"), 0.0);
        assert_eq!(ta.caret(), 18);
        ta.edit(&ui, &Out::Named("Down"), 0.0);
        assert_eq!(ta.caret(), ta.len(), "down from the last line goes to the end");
        let l1 = &ta.lines[1];
        let x = l1.x_at(l1.range.start + 3) + 1.0;
        ta.tap(x, l1.y + 2.0, 100.0);
        assert_eq!(ta.caret(), 9);
        assert!(ta.focused);
        ta.tap(-50.0, -50.0, 100.0);
        assert_eq!(ta.caret(), 0);
        ta.tap(9999.0, 9999.0, 100.0);
        assert_eq!(ta.caret(), ta.len());
        let (cx, cy, _, ch) = ta.caret_rect(&ui);
        let last = ta.lines.last().unwrap();
        assert!(cy >= last.y && cy + ch <= last.y + last.h + 0.5 && cx > 0.0);
    }

    #[test]
    fn caret_at_a_soft_wrap_sits_on_the_next_line() {
        let ui = ui();
        let mut ta = area(&ui, 70.0);
        ta.set_text(&ui, "\naaa bbb ccc");
        assert!(ta.lines.len() >= 3, "{:?}", ta.line_texts());
        assert_eq!(ta.lines[1].text, "aaa ");
        let (end, y) = (ta.lines[1].range.end, ta.lines[1].y);
        let x = ta.lines[1].x_at(end - 1) + 2.0;
        ta.caret = end;
        assert_eq!(ta.line_of(ta.caret()), 2, "after the hanging space is the start of the next line");
        ta.tap(x, y + 1.0, 0.0);
        assert_eq!(ta.caret(), end - 1, "a tap at the space's left edge lands before it");
        ta.tap(500.0, y + 1.0, 0.0);
        assert_eq!(ta.line_of(ta.caret()), 2, "a tap past the hanging space goes to the next line");
    }

    #[test]
    fn redo_key_and_grapheme_backspace() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        typed(&mut ta, &ui, "a\u{1F1EE}\u{1F1F8}", 0.0);
        ta.edit(&ui, &Out::Backspace, 1.0);
        assert_eq!(ta.text(), "a", "a flag is deleted whole");
        ta.edit(&ui, &Out::Named("Undo"), 2.0);
        assert_eq!(ta.text(), "a\u{1F1EE}\u{1F1F8}");
        ta.edit(&ui, &Out::Named("Left"), 3.0);
        assert_eq!(ta.caret(), 1);
        ta.edit(&ui, &Out::Named("Right"), 4.0);
        assert_eq!(ta.caret(), 3);
        ta.edit(&ui, &Out::Named("Undo"), 5.0);
        ta.edit(&ui, &Out::Named("Redo"), 6.0);
        assert_eq!(ta.text(), "a\u{1F1EE}\u{1F1F8}");
    }

    #[test]
    fn typing_is_named_in_the_edit_menu_titles() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        assert_eq!(ta.undo_titles(), ("Undo".to_string(), "Redo".to_string()));
        typed(&mut ta, &ui, "hello", 0.0);
        assert_eq!(ta.undo_titles().0, "Undo Typing");
        ta.undo(&ui, 1.0);
        assert_eq!(ta.undo_titles(), ("Undo".to_string(), "Redo Typing".to_string()));
    }

    #[test]
    fn undo_groups_words_and_redo_replays() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        typed(&mut ta, &ui, "hello world", 0.0);
        assert_eq!(ta.steps(), 2, "a space ends a group");
        assert!(ta.undo(&ui, 1.0));
        assert_eq!(ta.text(), "hello ");
        assert!(ta.undo(&ui, 2.0));
        assert_eq!(ta.text(), "");
        assert!(!ta.undo(&ui, 3.0));
        assert!(ta.redo(&ui, 4.0) && ta.redo(&ui, 4.0));
        assert_eq!(ta.text(), "hello world");
        assert_eq!(ta.caret(), 11);
        assert!(!ta.redo(&ui, 5.0));
        ta.edit(&ui, &Out::Enter, 6.0);
        typed(&mut ta, &ui, "x", 6.0);
        assert_eq!(ta.steps(), 4, "a newline is its own step");
        ta.edit(&ui, &Out::Backspace, 7.0);
        ta.edit(&ui, &Out::Backspace, 7.0);
        ta.edit(&ui, &Out::Backspace, 7.0);
        assert_eq!(ta.text(), "hello worl");
        assert_eq!(ta.steps(), 5, "contiguous backspaces are one step");
        ta.undo(&ui, 8.0);
        assert_eq!(ta.text(), "hello world\nx");
        assert_eq!(ta.caret(), 13);
        typed(&mut ta, &ui, "y", 9.0);
        assert!(!ta.can_redo(), "a new edit drops the redo stack");
        for i in 0..UNDO_DEPTH + 10 {
            ta.edit(&ui, &Out::Enter, 10.0 + i as f64);
        }
        assert_eq!(ta.steps(), UNDO_DEPTH);
    }

    #[test]
    fn selection_can_be_read_replaced_and_typed_over() {
        let ui = ui();
        let mut ta = area(&ui, 400.0);
        ta.set_text(&ui, "Title\nabc def");
        ta.select(6..9);
        assert_eq!(ta.selected_text(), "abc");
        assert_eq!(ta.selection(), Some(6..9));
        ta.replace_selection(&ui, "XY", 0.0);
        assert_eq!(ta.text(), "Title\nXY def");
        assert_eq!(ta.selection(), None);
        assert_eq!(ta.caret(), 8);
        let past_end = ta.len() + 88;
        ta.select(past_end..3);
        assert_eq!(ta.selection(), Some(3..12), "clamped and ordered");
        ta.edit(&ui, &Out::Text("!".into()), 1.0);
        assert_eq!(ta.text(), "Tit!");
        ta.undo(&ui, 2.0);
        assert_eq!(ta.text(), "Title\nXY def");
        ta.select(0..2);
        ta.edit(&ui, &Out::Backspace, 3.0);
        assert_eq!(ta.text(), "tle\nXY def");
        ta.select(1..3);
        ta.edit(&ui, &Out::Named("Left"), 4.0);
        assert_eq!((ta.caret(), ta.selection()), (1, None));
        ta.select(1..3);
        ta.edit(&ui, &Out::Named("Right"), 4.0);
        assert_eq!(ta.caret(), 3);
        ta.select(2..2);
        assert_eq!(ta.selection(), None, "an empty range is no selection");
        let steps = ta.steps();
        ta.replace_selection(&ui, "", 5.0);
        assert_eq!(ta.text(), "tle\nXY def", "replacing nothing with nothing is a no-op");
        assert_eq!(ta.steps(), steps, "and records no step");
    }

    #[test]
    fn marked_text_composes_in_place_and_commits_as_one_typing_step() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\nk");
        let steps = ta.steps();
        // an input method marks "´", then replaces the marked text with the composed "á"
        ta.set_marked_text(&ui, "´", "´".len(), 0, 1.0);
        assert_eq!(ta.text(), "Title\nk´");
        assert_eq!(ta.marked_range(), Some(7..8));
        assert_eq!(ta.caret(), 8);
        assert_eq!(ta.steps(), steps, "marking records nothing");
        ta.set_marked_text(&ui, "á", "á".len(), 0, 2.0);
        assert_eq!(ta.text(), "Title\nká", "the new candidate replaces the marked text");
        assert!(ta.unmark_text(&ui, true, 3.0));
        assert_eq!(ta.text(), "Title\nká");
        assert_eq!(ta.marked_range(), None);
        assert_eq!(ta.caret(), 8);
        assert_eq!(ta.steps(), steps + 1, "the commit is one step");
        assert!(ta.undo(&ui, 4.0));
        assert_eq!(ta.text(), "Title\nk", "undo takes the committed text back out");
        assert!(!ta.unmark_text(&ui, true, 5.0), "nothing marked");
    }

    #[test]
    fn a_dropped_composition_puts_back_the_selection_it_replaced() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\nthe fox");
        ta.select(10..13);
        ta.set_marked_text(&ui, "にほん", 3, 3, 1.0);
        assert_eq!(ta.text(), "Title\nthe にほん");
        assert_eq!(ta.selection(), Some(11..12), "the selection inside the marked text, bytes to chars");
        assert!(ta.unmark_text(&ui, false, 2.0));
        assert_eq!(ta.text(), "Title\nthe fox");
        assert_eq!(ta.marked_range(), None);
        // an empty composition ends it the same way
        ta.set_marked_text(&ui, "x", 1, 0, 3.0);
        ta.set_marked_text(&ui, "", 0, 0, 4.0);
        assert_eq!(ta.text(), "Title\nthe fox");
        assert_eq!(ta.marked_range(), None);
    }

    #[test]
    fn typing_or_clicking_during_a_composition_commits_it_first() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\n");
        ta.set_marked_text(&ui, "ka", 2, 0, 1.0);
        assert!(ta.edit(&ui, &Out::Text("!".into()), 2.0));
        assert_eq!(ta.text(), "Title\nka!");
        assert_eq!(ta.marked_range(), None);
        ta.set_marked_text(&ui, "é", 2, 0, 3.0);
        ta.set_pointer_mode(true);
        ta.press(&ui, 0.0, 0.0, 4.0, false);
        ta.release();
        assert_eq!(ta.text(), "Title\nka!é", "a click keeps the marked text");
        assert_eq!(ta.marked_range(), None);
        // a new text from elsewhere forgets the composition
        ta.set_marked_text(&ui, "x", 1, 0, 5.0);
        ta.set_text(&ui, "other");
        assert_eq!(ta.marked_range(), None);
    }

    #[test]
    fn marked_text_is_drawn_underlined() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\nbody ");
        ta.set_marked_text(&ui, "ab", 2, 0, 0.0);
        let mut buf = vec![0u32; 600 * 300];
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
        let l1 = &ta.lines[1];
        let (x0, x1) = (l1.x_at(11), l1.x_at(13));
        let y = (10.0 + l1.y + l1.h - ui.pt(2.0)) as usize;
        let x = (20.0 + (x0 + x1) / 2.0) as usize;
        assert_eq!(buf[y * 600 + x], WHITE, "an underline in the text colour under the marked text");
    }

    #[test]
    fn delete_surrounding_removes_bytes_around_the_caret() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\nbaká");
        ta.set_caret(10);
        // "k" and "á" before the caret are 1 and 2 bytes
        assert!(ta.delete_surrounding(&ui, 3, 0, 1.0));
        assert_eq!(ta.text(), "Title\nba");
        assert_eq!(ta.caret(), 8);
        assert!(!ta.delete_surrounding(&ui, 0, 4, 2.0), "nothing after the caret");
        assert!(ta.undo(&ui, 3.0));
        assert_eq!(ta.text(), "Title\nbaká");
    }

    #[test]
    fn draws_text_selection_and_a_blinking_caret() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Title\nbody text here");
        ta.focused = true;
        let mut buf = vec![0u32; 600 * 300];
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
        assert!(buf.contains(&WHITE), "text drawn");
        assert!(buf.contains(&TINT), "caret visible right after an edit");
        assert!(ta.lines.iter().all(|l| l.img.is_some()), "visible lines are rendered once");
        buf.fill(0);
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 500.0);
        assert!(!buf.contains(&TINT), "caret off in the second half second");
        assert_eq!(ta.next_blink(600.0), Some(1000.0));
        ta.focused = false;
        assert_eq!(ta.next_blink(600.0), None);
        ta.select(6..10);
        buf.fill(0);
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
        let l1 = &ta.lines[1];
        let inside = buf[(10.0 + l1.y + 2.0) as usize * 600 + 21];
        assert!(inside >> 24 > 0 && inside != WHITE, "selection highlight under the text");
        // a band below the text draws nothing
        let mut band = vec![0u32; 600 * 300];
        let mut s = Surface { w: 600, h: 300, stride: 600, px: &mut band, y0: 0 };
        ta.draw(&mut s.rows(250, 300), &ui, 20.0, 10.0, 0.0);
        assert!(band.iter().all(|&p| p == 0));
    }

    #[test]
    fn set_width_rewraps_and_keeps_rendered_lines() {
        let ui = ui();
        let mut ta = area(&ui, 600.0);
        ta.set_text(&ui, "Title\none two three four five six seven eight nine ten");
        let mut buf = vec![0u32; 600 * 400];
        ta.draw(&mut Surface { w: 600, h: 400, stride: 600, px: &mut buf, y0: 0 }, &ui, 0.0, 0.0, 0.0);
        let before = ta.lines.len();
        ta.set_width(&ui, 200.0);
        assert!(ta.lines.len() > before);
        assert!(ta.lines[0].img.is_some(), "the unchanged title keeps its image");
        ta.set_width(&ui, 200.0);
        assert_eq!(ta.lines.len(), ta.line_texts().len());
    }

    fn md_area(ui: &Ui, text: &str) -> TextArea {
        let mut ta = area(ui, 500.0);
        ta.set_markdown(ui, true);
        ta.set_text(ui, text);
        ta
    }

    #[test]
    fn enter_continues_lists_only_in_markdown_mode() {
        let ui = ui();
        let mut ta = md_area(&ui, "T\n- milk");
        ta.edit(&ui, &Out::Enter, 0.0);
        assert_eq!(ta.text(), "T\n- milk\n- ");
        ta.edit(&ui, &Out::Enter, 1.0);
        assert_eq!(ta.text(), "T\n- milk\n", "an empty item ends the list");
        assert!(ta.undo(&ui, 2.0));
        assert_eq!(ta.text(), "T\n- milk\n- ", "the end of a list is one undo step");
        assert!(ta.undo(&ui, 3.0));
        assert_eq!(ta.text(), "T\n- milk", "and so is the continuation");
        let mut n = md_area(&ui, "T\n1. a");
        n.edit(&ui, &Out::Enter, 0.0);
        assert_eq!(n.text(), "T\n1. a\n2. ");
        let mut c = md_area(&ui, "T\n- [x] a");
        c.edit(&ui, &Out::Enter, 0.0);
        assert_eq!(c.text(), "T\n- [x] a\n- [ ] ");
        let mut mid = md_area(&ui, "T\n- ab");
        mid.set_caret(4);
        mid.edit(&ui, &Out::Enter, 0.0);
        assert_eq!(mid.text(), "T\n- \nab", "mid-line Enter is a plain newline");
        let mut plain = area(&ui, 500.0);
        plain.set_text(&ui, "T\n- milk");
        plain.edit(&ui, &Out::Enter, 0.0);
        assert_eq!(plain.text(), "T\n- milk\n");
    }

    #[test]
    fn inline_and_display_math_reserve_their_width() {
        let ui = ui();
        let ta = md_area(&ui, "title\nsee $x^2$ now\n$$\\frac{a}{b}$$");
        let inline = &ta.lines[1];
        assert_eq!(inline.maths.len(), 1, "one inline formula on the line");
        let (at, lay) = &inline.maths[0];
        // the opening dollar is a marker (drawn as source), so the formula's source starts after it
        assert_eq!(*at, 5, "the formula starts after the opening dollar");
        assert!((inline.adv[5] - lay.width).abs() < 0.01, "the formula takes its width on its first char");
        assert!(inline.adv[6..8].iter().all(|a| *a == 0.0), "the rest of its source takes no width");
        let display = &ta.lines[2];
        assert!(display.display.is_some(), "a $$ paragraph is display math");
        assert!(display.h >= display.display.as_ref().map_or(0.0, |d| d.ascent + d.depth));
    }

    #[test]
    fn checkbox_taps_resolve_to_their_paragraph() {
        let ui = ui();
        let ta = md_area(&ui, "Title\n- [ ] a\n- b\nplain");
        let y = |i: usize| ta.lines[i].y + 2.0;
        assert_eq!(ta.checkbox_at(2.0, y(1)), Some(6));
        assert_eq!(ta.checkbox_at(400.0, y(1)), None, "text area, not the box");
        assert_eq!(ta.checkbox_at(2.0, y(2)), None, "a bullet has no box");
        assert_eq!(ta.checkbox_at(2.0, y(0)), None);
        assert_eq!(ta.paragraph_of(8), 6..13);
        assert_eq!(ta.paragraph_of(0), 0..5);
        assert_eq!(ta.paragraph_of(ta.len()), 18..23);
    }

    #[test]
    fn headings_and_emphasis_change_line_height_and_width() {
        let ui = ui();
        let ta = md_area(&ui, "T\n## Heading\nbody\n**bold** text");
        assert!(ta.lines[1].h > ta.lines[2].h, "headings are taller than body");
        let plain = {
            let mut p = area(&ui, 500.0);
            p.set_text(&ui, "T\nbold text");
            p.lines[1].adv.iter().sum::<f32>()
        };
        assert!(ta.lines[3].adv.iter().sum::<f32>() > plain, "bold chars are wider (the markers take no room)");
        assert!(ta.lines[3].segs.len() >= 3, "marker, bold and plain runs");
    }

    #[test]
    fn emphasis_markers_are_hidden_and_the_caret_passes_over_them() {
        // E2E W2-2: Cmd+B shows bold text, not `**` around it
        let ui = ui();
        let mut ta = md_area(&ui, "T\na **b** c");
        let l = &ta.lines[1];
        assert!(l.adv[2..4].iter().chain(&l.adv[5..7]).all(|a| *a == 0.0), "the markers take no width");
        assert!(l.segs.iter().filter(|g| g.kind == 7).count() == 2, "and are not drawn");
        assert_eq!(ta.concealed, vec![4..6, 7..9]);
        // a click at the start of the bold word puts the caret before the markers, so typing there is plain
        let x = ta.lines[1].x_at(6);
        assert_eq!(ta.position_at(x, ta.lines[1].y + 2.0), 4);
        // the arrows step one visible char at a time
        ta.set_caret(4);
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Right, false, false, false), 0.0);
        assert_eq!(ta.caret(), 7, "past `**b`: typing goes on in bold");
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Right, false, false, false), 0.0);
        assert_eq!(ta.caret(), 10, "past `** `");
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Left, false, false, false), 0.0);
        assert_eq!(ta.caret(), 7, "back after the b");
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Left, false, false, false), 0.0);
        assert_eq!(ta.caret(), 4, "back before the markers");
        // deleting the only char between the markers takes the pair too
        ta.set_caret(9);
        assert!(ta.edit(&ui, &Out::Backspace, 1.0));
        assert_eq!(ta.text(), "T\na  c");
        assert!(ta.concealed.is_empty());
        // plain mode hides nothing
        let mut plain = area(&ui, 500.0);
        plain.set_text(&ui, "T\na **b** c");
        assert!(plain.concealed.is_empty() && plain.lines[1].adv.iter().all(|a| *a > 0.0));
    }

    /// E2E12 finding 3: the `##` of a heading line was drawn in grey; the reference's headings show no marker.
    #[test]
    fn heading_markers_are_hidden_and_typing_stays_in_the_heading() {
        let ui = ui();
        // "Note\n" is chars 0..5, "## two" 5..11 (its marker 5..8), "\n" 11, "body" 12..16
        let mut ta = md_area(&ui, "Note\n## two\nbody");
        let l = &ta.lines[1];
        assert_eq!(l.block, Block::Heading);
        assert!(l.adv[..3].iter().all(|&d| d == 0.0) && l.adv[3] > 0.0, "the marker takes no width");
        assert!(l.segs.iter().any(|g| g.kind == 7 && g.a == 0 && g.b == 3), "and is not drawn");
        assert_eq!(ta.leads, vec![5..8]);
        assert!(ta.concealed.contains(&(5..8)));
        // a click at the heading's left edge puts the caret after the marker
        let (x, y) = (ta.lines[1].x_at(5), ta.lines[1].y + 2.0);
        assert_eq!(ta.position_at(x, y), 8);
        // typing at the line start (Cmd+Left puts the caret before the marker) goes into the heading
        ta.set_caret(5);
        assert!(ta.edit(&ui, &Out::Text("A ".into()), 1.0));
        assert_eq!(ta.text(), "Note\n## A two\nbody");
        // the arrows step over the hidden marker: Left from the heading's first char goes to the end of "Note"
        ta.set_caret(8);
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Left, false, false, false), 2.0);
        assert_eq!(ta.caret(), 4);
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Right, false, false, false), 2.0);
        assert_eq!(ta.caret(), 8, "and Right comes back to after the marker");
        // Return at the heading's first char opens a line above it; the heading keeps its marker
        assert!(ta.edit(&ui, &Out::Enter, 3.0));
        assert_eq!(ta.text(), "Note\n\n## A two\nbody");
        // Backspace at the heading's first char takes the marker away: the line becomes body text
        let mut ta = md_area(&ui, "Note\n## two");
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Backspace, 4.0));
        assert_eq!(ta.text(), "Note\ntwo");
        assert!(ta.leads.is_empty());
        // emphasis right after the marker: the caret settles at the heading's first char, not before the marker
        let ta = md_area(&ui, "Note\n## **b**");
        assert_eq!(ta.settle(10), 8);
        assert_eq!(ta.settle(7), 8);
        // plain mode hides nothing
        let mut plain = area(&ui, 500.0);
        plain.set_text(&ui, "Note\n## two");
        assert!(plain.leads.is_empty() && plain.lines[1].adv.iter().all(|a| *a > 0.0));
    }

    /// E2E13: a selection that takes in a heading's hidden marker (Shift+Cmd+Left from the end, a triple-click, a whole
    /// line) and is typed over keeps the line a heading; Backspace at an empty heading's start still ends it.
    #[test]
    fn typing_over_a_selection_from_a_heading_start_keeps_the_heading() {
        let ui = ui();
        // "Note\n" is chars 0..5, "## two" 5..11 (its marker 5..8)
        let mut ta = md_area(&ui, "Note\n## two\nbody");
        ta.set_caret(11);
        ta.key_combo(&ui, combo(bifrost_ui::keys::Key::Left, true, false, true), 1.0);
        assert_eq!(ta.selection().map(|r| r.start), Some(5), "Shift+Cmd+Left selects from before the marker");
        assert!(ta.edit(&ui, &Out::Text("X".into()), 1.0));
        assert_eq!(ta.text(), "Note\n## X\nbody", "the marker stays");
        assert_eq!(ta.lines[1].block, Block::Heading);
        // a paragraph selection (as a triple-click makes) pasted over
        let mut ta = md_area(&ui, "Note\n## two\nbody");
        ta.select(5..11);
        ta.replace_selection(&ui, "Y", 2.0);
        assert_eq!(ta.text(), "Note\n## Y\nbody");
        // a selection starting inside the marker, deleted: the heading stays, now empty
        let mut ta = md_area(&ui, "Note\n## two\nbody");
        ta.select(6..11);
        assert!(ta.edit(&ui, &Out::Backspace, 3.0));
        assert_eq!(ta.text(), "Note\n## \nbody");
        assert_eq!(ta.leads, vec![5..8]);
        // Cmd+Backspace to the line start keeps the heading too
        let mut cmd = md_area(&ui, "Note\n## two\nbody");
        cmd.set_caret(11);
        cmd.key_combo(&ui, combo(bifrost_ui::keys::Key::Delete, false, false, true), 3.5);
        assert_eq!(cmd.text(), "Note\n## \nbody");
        // and Backspace at the empty heading's start is the way to end it
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Backspace, 4.0));
        assert_eq!(ta.text(), "Note\n\nbody");
        // a selection that starts on the line before takes the marker with the newline, as any join does
        let mut ta = md_area(&ui, "Note\n## two");
        ta.select(4..8);
        assert!(ta.edit(&ui, &Out::Text("Z".into()), 5.0));
        assert_eq!(ta.text(), "NoteZtwo");
    }

    /// E2E13: Return at the end of an empty heading (`## ` and nothing after it) puts the new line below the heading,
    /// not above it; a heading with text still opens the line above when Return is at its first char.
    #[test]
    fn return_in_an_empty_heading_opens_the_line_below() {
        let ui = ui();
        let mut ta = md_area(&ui, "Note\n## ");
        assert_eq!(ta.leads, vec![5..8]);
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Enter, 1.0));
        assert_eq!(ta.text(), "Note\n## \n");
        assert_eq!(ta.caret(), 9, "the caret is on the new line");
        // the same with a line after the empty heading
        let mut ta = md_area(&ui, "Note\n## \nbody");
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Enter, 2.0));
        assert_eq!(ta.text(), "Note\n## \n\nbody");
        let mut ta = md_area(&ui, "Note\n## two");
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Enter, 3.0));
        assert_eq!(ta.text(), "Note\n\n## two");
    }

    #[test]
    fn deleting_between_two_runs_keeps_their_markers() {
        // the space between two runs is not the text of an empty pair: only it goes
        let ui = ui();
        let mut ta = md_area(&ui, "T\n**a** *b*");
        // "**a**" is chars 2..7 and the space 7..8; Backspace after the space, then Delete before it
        ta.set_caret(8);
        assert!(ta.edit(&ui, &Out::Backspace, 1.0));
        assert_eq!(ta.text(), "T\n**a***b*", "the space alone goes");
        let mut ta = md_area(&ui, "T\n**a** **b**");
        ta.set_caret(7);
        let _ = ta.key_combo(&ui, combo(bifrost_ui::keys::Key::ForwardDelete, false, false, false), 2.0);
        assert_eq!(ta.text(), "T\n**a****b**", "the runs are not merged");
        // the only char of a run still takes its markers with it, on the title line too
        let mut ta = md_area(&ui, "**t**\nx *i* y");
        ta.set_caret(10);
        assert!(ta.edit(&ui, &Out::Backspace, 3.0));
        assert_eq!(ta.text(), "**t**\nx  y");
        ta.set_caret(4);
        assert!(ta.edit(&ui, &Out::Backspace, 4.0));
        assert_eq!(ta.text(), "\nx  y");
    }

    #[test]
    fn plain_mode_has_no_hidden_markers() {
        let ui = ui();
        let mut plain = area(&ui, 500.0);
        plain.set_text(&ui, "T\na **b** c");
        assert!(plain.concealed.is_empty() && plain.lines[1].adv.iter().all(|a| *a > 0.0));
    }

    #[test]
    fn image_paragraphs_become_tall_lines() {
        let ui = ui();
        let mut ta = md_area(&ui, "T\n![image](a.png)\nend");
        let before = ta.height();
        let mut images = HashMap::new();
        images.insert("a.png".to_string(), Image::new(40, 90));
        ta.set_images(&ui, images);
        assert_eq!(ta.lines[1].image.as_deref(), Some("a.png"));
        assert!(ta.height() > before + 60.0);
        assert_eq!(ta.lines[2].text, "end");
        let mut buf = vec![0u32; 600 * 400];
        ta.draw(&mut Surface { w: 600, h: 400, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
        ta.set_text(&ui, "T\n![image](missing.png)");
        assert!(ta.lines[1].image.is_none(), "unknown files stay as text");
    }

    #[test]
    fn links_are_found_only_when_asked_and_report_the_text_under_the_point() {
        let ui = ui();
        let mut ta = area(&ui, 500.0);
        ta.set_text(&ui, "Sjá https://bifrost.olibuijr.com/docs og fundur");
        assert!(ta.link_at(40.0, 10.0).is_none(), "detection is off by default (Contacts does not use it)");
        ta.set_detect_links(true);
        assert_eq!(ta.links.len(), 1);
        let start = ta.text.find("https").map(|b| ta.text[..b].chars().count()).unwrap();
        assert_eq!(ta.links[0], start..start + "https://bifrost.olibuijr.com/docs".chars().count());
        // a char is found through its x position: the first character of the link, at the start of its line
        let x = ta.lines[0].x_at(start) + 1.0;
        assert_eq!(ta.link_at(x, ta.lines[0].y + 1.0).as_deref(), Some("https://bifrost.olibuijr.com/docs"));
        // an edit before the link moves its range with it
        ta.set_caret(0);
        ta.replace_range(&ui, 0..0, "Hæ! ", 0.0, false);
        assert_eq!(ta.links[0].start, start + 4);
        // and it is drawn underlined: the pixels differ from the same text with detection off
        let mut with = vec![0u32; 600 * 120];
        ta.draw(&mut Surface { w: 600, h: 120, stride: 600, px: &mut with, y0: 0 }, &ui, 0.0, 0.0, 0.0);
        ta.set_detect_links(false);
        let mut without = vec![0u32; 600 * 120];
        ta.draw(&mut Surface { w: 600, h: 120, stride: 600, px: &mut without, y0: 0 }, &ui, 0.0, 0.0, 0.0);
        assert!(with != without, "the link underline changes the pixels");
    }

    #[test]
    fn markdown_draw_paints_boxes_highlights_and_text() {
        let ui = ui();
        let mut ta = md_area(&ui, "T\n- [x] done\n- [ ] todo\n- bullet **b** `c`");
        ta.set_highlights(vec![2..4, 10..12], Some(1));
        let mut buf = vec![0u32; 600 * 300];
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
        assert!(buf.contains(&TINT), "the ticked box is filled with the tint");
        assert!(buf.contains(&rgba(255, 159, 10, 150)) || buf.iter().any(|&p| p >> 24 > 0), "something painted");
        assert!(ta.lines.iter().filter(|l| l.block == Block::Check(false)).all(|l| l.simgs.iter().any(Option::is_some)));
        ta.edit(&ui, &Out::Text("x".into()), 1.0);
        ta.draw(&mut Surface { w: 600, h: 300, stride: 600, px: &mut buf, y0: 0 }, &ui, 20.0, 10.0, 0.0);
    }

    #[test]
    fn a_title_marker_on_the_first_line_takes_no_room_and_draws_nothing() {
        let ui = ui();
        let plain = md_area(&ui, "Shopping list\nMilk");
        let marked = md_area(&ui, "# Shopping list\nMilk");
        assert_eq!(marked.text(), "# Shopping list\nMilk", "the text keeps its marker");
        let (a, b) = (&plain.lines[0], &marked.lines[0]);
        assert_eq!(&b.adv[..2], &[0.0, 0.0]);
        assert_eq!(b.x_at(b.range.start + 2), 0.0);
        assert_eq!(a.x_at(a.range.end), b.x_at(b.range.end), "the title is as wide as the same title without the marker");
        assert!(b.pt == a.pt && b.weight == a.weight, "the first line has the title style either way");
        assert_eq!(b.segs[0].kind, 7, "the marker is not drawn");
        assert!(a.segs.iter().all(|g| g.kind != 7));
        // on a later line `# ` is a heading's marker, hidden as every heading marker is (E2E12 finding 3)
        let later = md_area(&ui, "Note\n# Heading");
        assert!(later.lines[1].adv[..2].iter().all(|&d| d == 0.0));
        assert_eq!(later.leads, vec![5..7]);
        // the title's marker is a lead too: a click at the line start, then a paste, lands after it (no stray `#`)
        assert_eq!(marked.leads, vec![0..2]);
        let mut empty = md_area(&ui, "# ");
        let (x, y) = (empty.lines[0].x_at(0), empty.lines[0].y + 2.0);
        assert_eq!(empty.position_at(x, y), 2);
        empty.set_caret(0);
        assert!(empty.edit(&ui, &Out::Text("QA copy test".into()), 1.0));
        assert_eq!(empty.text(), "# QA copy test");
    }

    /// E2E11 11: a heading line that also carries a checkbox marker draws as the heading alone, never the raw marker.
    #[test]
    fn a_checkbox_marker_after_a_heading_marker_is_not_drawn() {
        let ui = ui();
        let ta = md_area(&ui, "Note\n## - [ ] Item two\n## Item two");
        let (stray, plain) = (&ta.lines[1], &ta.lines[2]);
        assert_eq!(stray.block, Block::Heading, "a heading, not a checklist item");
        assert!(stray.pt == plain.pt && stray.weight == plain.weight);
        assert!(stray.adv[3..9].iter().all(|&d| d == 0.0), "the checkbox marker takes no room");
        assert!(stray.adv[..3].iter().all(|&d| d == 0.0), "the heading marker is hidden as on any heading");
        assert_eq!(stray.x_at(stray.range.start + 9), plain.x_at(plain.range.start + 3), "the text starts where a plain heading's does");
        assert!(stray.segs.iter().any(|g| g.kind == 7 && g.a == 0 && g.b == 9), "the whole marker is not drawn");
        assert_eq!(ta.checkbox_at(stray.x_at(stray.range.start + 3), stray.y + stray.h / 2.0), None, "nothing to tick");
    }

    #[test]
    fn whole_paragraph_replacement_is_one_undo_step() {
        let ui = ui();
        let mut ta = md_area(&ui, "T\n- [x] a\n- [ ] b");
        let n = ta.len();
        ta.replace_range(&ui, 0..n, &md::checked_to_bottom("T\n- [x] a\n- [ ] b"), 0.0, false);
        assert_eq!(ta.text(), "T\n- [ ] b\n- [x] a");
        assert!(ta.undo(&ui, 1.0));
        assert_eq!(ta.text(), "T\n- [x] a\n- [ ] b");
        assert!(!ta.can_undo());
    }

    /// The point (px from the text origin) in the middle of the line holding char `c`, at its left edge.
    fn at(ta: &TextArea, c: usize) -> (f32, f32) {
        let l = &ta.lines[ta.line_of(c)];
        (l.x_at(c), l.y + l.h / 2.0)
    }

    fn combo(key: bifrost_ui::keys::Key, shift: bool, alt: bool, cmd: bool) -> bifrost_ui::keys::KeyCombo {
        bifrost_ui::keys::KeyCombo { cmd, ctrl: false, alt, shift, key }
    }

    #[test]
    fn a_mouse_click_places_the_caret_and_a_drag_selects_by_characters() {
        let ui = ui();
        let mut ta = area(&ui, 2000.0);
        ta.set_pointer_mode(true);
        ta.set_text(&ui, "Title\nthe quick brown fox");
        let (x0, y0) = at(&ta, 10);
        ta.press(&ui, x0, y0, 0.0, false);
        assert_eq!((ta.caret(), ta.selection()), (10, None), "one click is a caret");
        let (x1, y1) = at(&ta, 15);
        assert!(ta.drag_to(x1, y1));
        ta.release();
        assert_eq!(ta.selection(), Some(10..15));
        assert_eq!(ta.selected_text(), "quick");
        // a Shift-click moves the free end and keeps the anchor
        let (x2, y2) = at(&ta, 21);
        ta.press(&ui, x2, y2, 2000.0, true);
        ta.release();
        assert_eq!(ta.selection(), Some(10..21));
        assert!(!ta.drag_to(x0, y0), "no drag after the release");
    }

    #[test]
    fn double_and_triple_clicks_select_the_word_and_the_paragraph() {
        let ui = ui();
        let mut ta = area(&ui, 2000.0);
        ta.set_pointer_mode(true);
        ta.set_text(&ui, "Title\nthe quick brown fox\nnext");
        let (x, y) = at(&ta, 12);
        ta.press(&ui, x, y, 0.0, false);
        ta.release();
        ta.press(&ui, x, y, 100.0, false);
        ta.release();
        assert_eq!(ta.selected_text(), "quick", "double click");
        ta.press(&ui, x, y, 200.0, false);
        assert_eq!(ta.selected_text(), "the quick brown fox", "triple click, without the newline");
        // dragging after a triple click grows by paragraphs
        let (xn, yn) = at(&ta, 27);
        ta.drag_to(xn, yn);
        ta.release();
        assert_eq!(ta.selected_text(), "the quick brown fox\nnext");
        // a slow second click starts over
        ta.press(&ui, x, y, 5000.0, false);
        assert_eq!(ta.selection(), None);
        // a double click then a drag grows by words
        ta.release();
        ta.press(&ui, x, y, 5100.0, false);
        let (xb, yb) = at(&ta, 17);
        ta.drag_to(xb, yb);
        assert_eq!(ta.selected_text(), "quick brown");
    }

    #[test]
    fn hardware_keys_move_extend_select_all_and_delete_by_word() {
        use bifrost_ui::keys::Key;
        let ui = ui();
        let mut ta = area(&ui, 2000.0);
        ta.set_pointer_mode(true);
        ta.set_text(&ui, "Title\nthe quick brown");
        // the caret is at the end: Shift+Option+Left takes the last word
        assert_eq!(ta.key_combo(&ui, combo(Key::Left, true, true, false), 0.0), Some(false));
        assert_eq!(ta.selected_text(), "brown");
        // Shift+Home extends to the start of the line
        assert_eq!(ta.key_combo(&ui, combo(Key::Home, true, false, false), 1.0), Some(false));
        assert_eq!(ta.selected_text(), "the quick brown");
        // a plain Right collapses to the end of the selection
        ta.key_combo(&ui, combo(Key::Right, false, false, false), 2.0);
        assert_eq!((ta.caret(), ta.selection()), (21, None));
        // Shift+Left twice, then Shift+Right once
        ta.key_combo(&ui, combo(Key::Left, true, false, false), 3.0);
        ta.key_combo(&ui, combo(Key::Left, true, false, false), 3.0);
        ta.key_combo(&ui, combo(Key::Right, true, false, false), 3.0);
        assert_eq!(ta.selected_text(), "n");
        // Cmd+A
        ta.key_combo(&ui, combo(Key::Char('a'), false, false, true), 4.0);
        assert_eq!(ta.selection(), Some(0..21));
        // Option+Backspace deletes the word before the caret
        ta.set_caret(21);
        assert_eq!(ta.key_combo(&ui, combo(Key::Delete, false, true, false), 5.0), Some(true));
        assert_eq!(ta.text(), "Title\nthe quick ");
        assert!(ta.undo(&ui, 6.0));
        assert_eq!(ta.text(), "Title\nthe quick brown");
        // Shift+Cmd+Up selects to the start of the text
        ta.set_caret(9);
        ta.key_combo(&ui, combo(Key::Up, true, false, true), 7.0);
        assert_eq!(ta.selection(), Some(0..9));
        // a letter on its own is not an editing key
        assert_eq!(ta.key_combo(&ui, combo(Key::Char('q'), false, false, false), 8.0), None);
    }

    #[test]
    fn shift_with_up_and_down_extends_by_lines() {
        use bifrost_ui::keys::Key;
        let ui = ui();
        let mut ta = area(&ui, 2000.0);
        ta.set_pointer_mode(true);
        ta.set_text(&ui, "Title\none\ntwo");
        ta.set_caret(8);
        ta.key_combo(&ui, combo(Key::Down, true, false, false), 0.0);
        assert_eq!(ta.selection().map(|r| r.start), Some(8), "the anchor stays");
        assert!(ta.caret() > 9, "the caret went down a line");
        ta.key_combo(&ui, combo(Key::Up, false, false, false), 1.0);
        assert_eq!(ta.selection(), None, "a plain arrow collapses");
    }
}
