//! A multi-line text editor view: word-wrapped layout in SF Pro with a
//! larger first paragraph (the note's title), a blinking caret placed by
//! tap or arrow keys, insert/delete/replace with a 50-step undo history,
//! and a selection range the clipboard can read and replace.

use super::glyphs::{either, paint, segment};
use super::keyboard::Out;
use super::textfield::{draw_loupe, SELECT};
use super::md::{self, Block, Style};
use super::ui::{Ui, Weight, BLACK, SECONDARY};
use crate::gfx::{pack, rgba, Image, Surface};
use std::collections::HashMap;
use std::ops::Range;
use bifrost_ui::undo::UndoManager;

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
}

/// A run of chars (line-relative) drawn in one weight and colour. `kind` 1
/// is a bullet marker and 2 a checkbox, both drawn as shapes; 3, 4 and 5 are
/// struck-through, underlined and highlighted text, and 6 the bar of a quote.
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
        let mut ta = TextArea { text: String::new(), caret: 0, anchor: None, w, body_pt, title_pt, color, tint, lines: vec![], history: UndoManager::new(UNDO_DEPTH), typing: None, focused: false, blink_from: 0.0, markdown: false, images: HashMap::new(), highlights: vec![], current_hl: None, touch: None, last_tap: None, detect_links: false, links: vec![] };
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
        let (mut y, mut start) = (0.0, 0);
        let paragraphs: Vec<&str> = self.text.split('\n').collect();
        let last = paragraphs.len() - 1;
        for (pi, para) in paragraphs.into_iter().enumerate() {
            let chars: Vec<char> = para.chars().collect();
            if self.markdown && pi > 0 {
                if let Some(name) = md::image_ref(para).filter(|n| self.images.contains_key(*n)) {
                    let h = self.images[name].h as f32 + ui.pt(12.0);
                    let range = start..start + chars.len() + (pi != last) as usize;
                    lines.push(Line { range, text: para.to_string(), adv: vec![0.0; chars.len()], y, h, pt: self.body_pt, weight: Weight::Regular, img: None, segs: vec![], simgs: vec![], block: Block::Body, prefix: 0, indent: 0, first: true, para_start: start, image: Some(name.to_string()) });
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
            let h = self.line_h(ui, pt);
            let styles = if self.markdown { self.char_styles(para, chars.len(), block, prefix, indent, weight) } else { vec![] };
            let adv: Vec<f32> = if self.markdown {
                let mut buf = [0u8; 4];
                chars.iter().zip(&styles).map(|(c, st)| ui.measure(c.encode_utf8(&mut buf), pt, st.0)).collect()
            } else {
                advances(ui, para, pt, weight)
            };
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
                let mut line = Line { text, adv: adv[ls..end].to_vec(), range, y, h, pt, weight, img: None, segs, simgs: vec![], block, prefix, indent, first: ls == 0, para_start: start, image: None };
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
        best.0
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

    /// The selected char range, if any.
    pub fn selection(&self) -> Option<Range<usize>> {
        self.anchor.map(|a| a.min(self.caret)..a.max(self.caret))
    }

    pub fn selected_text(&self) -> String {
        self.selection().map(|r| self.text.chars().skip(r.start).take(r.len()).collect()).unwrap_or_default()
    }

    /// Replaces the selection (or inserts at the caret) with `s`.
    pub fn replace_selection(&mut self, ui: &Ui, s: &str, t: f64) {
        let range = self.selection().unwrap_or(self.caret..self.caret);
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
        self.blink_from = t;
        match out {
            Out::Text(s) => {
                let range = self.selection().unwrap_or(self.caret..self.caret);
                self.replace_range(ui, range, s, t, true);
                true
            }
            Out::Enter => {
                if self.markdown && self.selection().is_none() && self.continue_list(ui, t) {
                    return true;
                }
                let range = self.selection().unwrap_or(self.caret..self.caret);
                self.replace_range(ui, range, "\n", t, false);
                true
            }
            Out::Backspace => {
                let range = match self.selection() {
                    Some(r) => r,
                    None if self.caret > 0 => super::edit::prev_boundary(&self.text, self.caret)..self.caret,
                    None => return false,
                };
                self.replace_range(ui, range, "", t, true);
                true
            }
            Out::Named("Left") => {
                self.caret = self.selection().map_or_else(|| super::edit::prev_boundary(&self.text, self.caret), |r| r.start);
                self.anchor = None;
                false
            }
            Out::Named("Right") => {
                self.caret = self.selection().map_or_else(|| super::edit::next_boundary(&self.text, self.caret), |r| r.end);
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
            Out::Named("Cut") => match self.selection() {
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
            if ly + l.h <= top || ly >= bottom {
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
                    s.fill_rrect(x + x0, ly, (x1 - x0).max(ui.pt(2.0)), l.h, 0.0, rgba(255, 214, 10, 70));
                }
            }
            if let Some(name) = &l.image {
                if let Some(img) = self.images.get(name) {
                    s.blit(img, x as i32, (ly + ui.pt(6.0)) as i32, 255, 255);
                }
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
        if self.focused && self.caret_on(t) {
            let (cx, cy, cw, ch) = self.caret_rect(ui);
            s.fill_rrect(x + cx, y + cy, cw, ch, cw / 2.0, self.tint);
        }
        if let Some(hs) = self.handles(ui) {
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
            p.set_text(&ui, "T\n**bold** text");
            p.lines[1].adv.iter().sum::<f32>()
        };
        assert!(ta.lines[3].adv.iter().sum::<f32>() > plain, "bold chars are wider");
        assert!(ta.lines[3].segs.len() >= 3, "marker, bold and plain runs");
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
}
