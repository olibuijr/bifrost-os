//! TextKit 2 style layout (`NSTextContentStorage`, `NSTextLayoutManager`, `NSTextLayoutFragment`,
//! `NSTextLineFragment`, `NSTextSelection`, `NSTextSelectionNavigation`). A pure model on top of `textlayout`
//! (storage and container), `paragraph` (styles), `textlist` (markers) and `text_attachment`.
//!
//! * `TextContentStorage`: the text, a paragraph style and an optional list item per paragraph (kept as attribute runs
//!   of a `TextStorage`, so they shift with edits), and the attachments that stand at `U+FFFC` characters.
//! * `TextLayoutManager`: one `TextLayoutFragment` per paragraph, each with its `TextLineFragment`s. Layout is cached
//!   per paragraph, so an edit lays out only the paragraphs whose text, style, attachments or width changed
//!   (`relaid_paragraphs`).
//! * `TextSelection` and `Direction`/`Granularity`: moves by character, word, line and paragraph, and up and down
//!   keeping the x position.
//!
//! Tab stops left-align; right and centre tab stops are stored but laid out as left stops. A paragraph style is set on
//! a paragraph that has at least a character or a newline; the last empty paragraph cannot hold one. Measuring text is
//! the caller's: `measure` gives points for a string.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::paragraph::{LineBreakMode, ParagraphStyle, DEFAULT_TAB_INTERVAL};
use crate::text::{line_end, line_start, next_boundary, next_word_end, prev_boundary, prev_word_start};
use crate::text_attachment::{TextAttachment, ATTACHMENT_CHARACTER};
use crate::textlayout::{Attribute, EditedRange, TextContainer, TextStorage, NO_ATTRIBUTE};
use crate::textlist::List;

/// A list item of a paragraph: the list and the nesting level (0 is the outermost).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListItem {
    pub list: List,
    pub level: usize,
}

/// The paragraph attributes of a paragraph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphAttrs {
    pub style: ParagraphStyle,
    pub list: Option<ListItem>,
}

/// The text and its paragraph attributes and attachments (`NSTextContentStorage`).
#[derive(Debug)]
pub struct TextContentStorage {
    storage: TextStorage,
    /// Interned attributes; attribute id `n` is `table[n - 1]`.
    table: Vec<ParagraphAttrs>,
    /// Attachments by the byte offset of their `U+FFFC` character.
    attachments: BTreeMap<usize, TextAttachment>,
}

impl TextContentStorage {
    pub fn new(text: &str) -> Self {
        TextContentStorage { storage: TextStorage::new(text), table: Vec::new(), attachments: BTreeMap::new() }
    }

    pub fn text(&self) -> &str {
        self.storage.text()
    }

    pub fn generation(&self) -> u64 {
        self.storage.generation()
    }

    /// The pending edit (`processEditing`).
    pub fn process_editing(&mut self) -> Option<EditedRange> {
        self.storage.process_editing()
    }

    /// Replaces the bytes at `range`. Attachments inside the range are removed, those behind it move. False, and
    /// nothing changes, for an invalid range.
    pub fn replace_characters(&mut self, range: Range<usize>, new: &str) -> bool {
        let (a, b) = (range.start, range.end);
        if !self.storage.replace_characters(range, new) {
            return false;
        }
        let n = new.len();
        let old = std::mem::take(&mut self.attachments);
        for (k, v) in old {
            if k < a {
                self.attachments.insert(k, v);
            } else if k >= b {
                self.attachments.insert(k - (b - a) + n, v);
            }
        }
        true
    }

    /// Puts `attachment` at byte offset `at` as one `U+FFFC` character (`NSTextAttachment` in an attributed string).
    pub fn insert_attachment(&mut self, at: usize, attachment: TextAttachment) -> bool {
        let mut mark = [0u8; 4];
        if !self.replace_characters(at..at, ATTACHMENT_CHARACTER.encode_utf8(&mut mark)) {
            return false;
        }
        self.attachments.insert(at, attachment);
        true
    }

    pub fn attachment_at(&self, offset: usize) -> Option<&TextAttachment> {
        self.attachments.get(&offset)
    }

    /// The glyph sizes of the attachments inside `range`, with offsets relative to the range start, fitted to
    /// `max_width`.
    fn attachment_sizes(&self, range: Range<usize>, max_width: f32) -> Vec<(usize, (f32, f32))> {
        let start = range.start;
        self.attachments.range(range).map(|(k, a)| (k - start, a.bounds_for_container(max_width))).collect()
    }

    /// The paragraphs as byte ranges, without their newlines. There is one more than there are newlines.
    pub fn paragraphs(&self) -> Vec<Range<usize>> {
        let text = self.text();
        let mut out = Vec::new();
        let mut start = 0;
        for (i, c) in text.char_indices() {
            if c == '\n' {
                out.push(start..i);
                start = i + 1;
            }
        }
        out.push(start..text.len());
        out
    }

    /// Sets the attributes of every paragraph `range` touches, newline included. The default attributes clear them.
    pub fn set_paragraph_attrs(&mut self, range: Range<usize>, attrs: ParagraphAttrs) -> bool {
        let (start, end) = {
            let text = self.text();
            if range.start > range.end || range.end > text.len() {
                return false;
            }
            let end = line_end(text, range.end);
            (line_start(text, range.start), if end < text.len() { end + 1 } else { end })
        };
        let id: Attribute = if attrs == ParagraphAttrs::default() {
            NO_ATTRIBUTE
        } else {
            match self.table.iter().position(|a| *a == attrs) {
                Some(i) => (i + 1) as Attribute,
                None => {
                    self.table.push(attrs);
                    self.table.len() as Attribute
                }
            }
        };
        self.storage.set_attribute(start..end, id)
    }

    /// The attributes of the paragraph `r`: the first set attribute inside it, or on its newline.
    fn paragraph_attrs(&self, r: &Range<usize>) -> ParagraphAttrs {
        let id = self
            .storage
            .attribute_runs(r.clone())
            .into_iter()
            .map(|(_, a)| a)
            .find(|a| *a != NO_ATTRIBUTE)
            .unwrap_or_else(|| self.storage.attribute_at(r.end));
        if id == NO_ATTRIBUTE {
            ParagraphAttrs::default()
        } else {
            self.table[(id - 1) as usize].clone()
        }
    }

    /// The attributes of the paragraph that holds `offset`.
    pub fn attrs_at(&self, offset: usize) -> ParagraphAttrs {
        let text = self.text();
        let offset = offset.min(text.len());
        self.paragraph_attrs(&(line_start(text, offset)..line_end(text, offset)))
    }

    /// The list marker of every paragraph, `None` for a paragraph outside a list. Numbers restart when a list ends or
    /// changes, and a nested list counts from its starting item number under each parent item.
    pub fn list_markers(&self) -> Vec<Option<String>> {
        let mut counters: Vec<(List, usize)> = Vec::new();
        let mut last: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for r in self.paragraphs() {
            let Some(item) = self.paragraph_attrs(&r).list else {
                counters.clear();
                last.clear();
                out.push(None);
                continue;
            };
            let level = item.level;
            counters.truncate(level + 1);
            while counters.len() <= level {
                counters.push((item.list, 0));
            }
            if counters[level].0 != item.list {
                counters[level] = (item.list, 0);
            }
            let index = counters[level].1;
            counters[level].1 += 1;
            last.truncate(level);
            while last.len() < level {
                last.push(String::new());
            }
            let enclosing = if level > 0 { last.get(level - 1).map(|s| s.as_str()) } else { None };
            let marker = item.list.marker(index, enclosing);
            last.push(marker.clone());
            out.push(Some(marker));
        }
        out
    }
}

/// One laid out line (`NSTextLineFragment`). Offsets are in the text, `x` and `y` in the container.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLineFragment {
    /// The characters of the line, without the newline and without the spaces a wrap consumed.
    pub range: Range<usize>,
    /// The x of the line's first glyph, placed by the alignment.
    pub x: f32,
    pub y: f32,
    /// The width of the glyphs.
    pub width: f32,
    /// The line height including the line spacing.
    pub height: f32,
    pub first_in_paragraph: bool,
}

/// One laid out paragraph (`NSTextLayoutFragment`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayoutFragment {
    pub range: Range<usize>,
    pub y: f32,
    pub height: f32,
    /// The list marker drawn before the first line.
    pub marker: Option<String>,
    pub lines: Vec<TextLineFragment>,
}

#[derive(Clone, Debug, PartialEq)]
struct RelLine {
    start: usize,
    end: usize,
    x: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Debug)]
struct Cached {
    text: String,
    attrs: ParagraphAttrs,
    avail: f32,
    pad: f32,
    natural: f32,
    atts: Vec<(usize, (f32, f32))>,
    lines: Vec<RelLine>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Word,
    Space,
    Tab,
    Attach,
}

fn tokenize(text: &str) -> Vec<(Range<usize>, Kind)> {
    let mut out: Vec<(Range<usize>, Kind)> = Vec::new();
    for (i, c) in text.char_indices() {
        let kind = match c {
            ' ' => Kind::Space,
            '\t' => Kind::Tab,
            ATTACHMENT_CHARACTER => Kind::Attach,
            _ => Kind::Word,
        };
        let end = i + c.len_utf8();
        let merge = matches!(out.last(), Some((_, k)) if *k == kind && (kind == Kind::Word || kind == Kind::Space));
        if merge {
            if let Some(last) = out.last_mut() {
                last.0.end = end;
            }
        } else {
            out.push((i..end, kind));
        }
    }
    out
}

/// The x a tab at `x` moves to: the next explicit stop, else the next multiple of the default interval.
fn tab_advance(style: &ParagraphStyle, x: f32) -> f32 {
    if let Some(t) = style.next_tab(x) {
        return t.location;
    }
    let iv = if style.default_tab_interval > 0.0 { style.default_tab_interval } else { DEFAULT_TAB_INTERVAL };
    ((x / iv).floor() + 1.0) * iv
}

/// Everything `close_line` needs to place a line.
struct LineCtx<'a> {
    style: &'a ParagraphStyle,
    avail: f32,
    pad: f32,
    natural: f32,
}

fn close_line(lines: &mut Vec<RelLine>, c: &LineCtx, start: usize, end: usize, x: f32, att_h: f32) {
    let first = lines.is_empty();
    let width = (x - c.style.indent(first)).max(0.0);
    let lx = c.pad + c.style.line_x(first, c.avail, width);
    let height = c.style.line_height(c.natural.max(att_h)) + c.style.line_spacing;
    lines.push(RelLine { start, end, x: lx, width, height });
}

/// Breaks one paragraph into lines. Offsets in the result are relative to the paragraph.
fn layout_paragraph(
    text: &str,
    style: &ParagraphStyle,
    avail: f32,
    pad: f32,
    atts: &[(usize, (f32, f32))],
    natural: f32,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<RelLine> {
    let ctx = LineCtx { style, avail, pad, natural };
    let limit = style.trailing_edge(avail);
    let single = !matches!(style.line_break_mode, LineBreakMode::WordWrap | LineBreakMode::CharWrap);
    let char_wrap = style.line_break_mode == LineBreakMode::CharWrap;
    let att_size = |at: usize| atts.iter().find(|a| a.0 == at).map(|a| a.1).unwrap_or((0.0, 0.0));
    let mut lines: Vec<RelLine> = Vec::new();
    let (mut start, mut end) = (0usize, 0usize);
    let mut x = style.indent(true);
    let mut vis_x = x;
    let mut att_h = 0.0f32;
    for (r, kind) in tokenize(text) {
        match kind {
            Kind::Space => x += measure(&text[r.clone()]),
            Kind::Tab => {
                x = tab_advance(style, x);
                vis_x = x;
                end = r.end;
            }
            Kind::Word | Kind::Attach => {
                let (w, h) = if kind == Kind::Attach { att_size(r.start) } else { (measure(&text[r.clone()]), 0.0) };
                if !single && !char_wrap && x + w > limit && end > start {
                    close_line(&mut lines, &ctx, start, end, vis_x, att_h);
                    start = r.start;
                    end = start;
                    x = style.indent(false);
                    vis_x = x;
                    att_h = 0.0;
                }
                if !single && kind == Kind::Word && x + w > limit {
                    for (off, c) in text[r.clone()].char_indices() {
                        let (cs, ce) = (r.start + off, r.start + off + c.len_utf8());
                        let cw = measure(&text[cs..ce]);
                        if x + cw > limit && end > start {
                            close_line(&mut lines, &ctx, start, end, vis_x, att_h);
                            start = cs;
                            x = style.indent(false);
                            att_h = 0.0;
                        }
                        x += cw;
                        vis_x = x;
                        end = ce;
                    }
                } else {
                    x += w;
                    vis_x = x;
                    end = r.end;
                    att_h = att_h.max(h);
                }
            }
        }
    }
    close_line(&mut lines, &ctx, start, end, vis_x, att_h);
    lines
}

/// The x reached after `seg` (text starting at byte `base`) from `x0`, with tabs and attachments.
fn advance_x(
    style: &ParagraphStyle,
    seg: &str,
    base: usize,
    content: &TextContentStorage,
    avail: f32,
    x0: f32,
    measure: &dyn Fn(&str) -> f32,
) -> f32 {
    let mut x = x0;
    let mut run = 0usize;
    for (i, c) in seg.char_indices() {
        if c == '\t' || c == ATTACHMENT_CHARACTER {
            x += measure(&seg[run..i]);
            x = if c == '\t' {
                tab_advance(style, x)
            } else {
                x + content.attachment_at(base + i).map_or(0.0, |a| a.bounds_for_container(avail).0)
            };
            run = i + c.len_utf8();
        }
    }
    x + measure(&seg[run..])
}

/// The layout of a content storage in a container (`NSTextLayoutManager`).
#[derive(Debug)]
pub struct TextLayoutManager {
    natural_line_height: f32,
    cache: Vec<Cached>,
    fragments: Vec<TextLayoutFragment>,
    relaid: Vec<usize>,
    avail: f32,
    used_height: f32,
}

impl TextLayoutManager {
    /// A manager whose lines are `natural_line_height` points tall before the paragraph style changes them.
    pub fn new(natural_line_height: f32) -> Self {
        TextLayoutManager {
            natural_line_height,
            cache: Vec::new(),
            fragments: Vec::new(),
            relaid: Vec::new(),
            avail: 0.0,
            used_height: 0.0,
        }
    }

    pub fn fragments(&self) -> &[TextLayoutFragment] {
        &self.fragments
    }

    /// The height of all the text (`usageBoundsForTextContainer`).
    pub fn used_height(&self) -> f32 {
        self.used_height
    }

    /// The indexes of the paragraphs the last `ensure_layout` laid out again (the rest came from the cache).
    pub fn relaid_paragraphs(&self) -> &[usize] {
        &self.relaid
    }

    /// `ensureLayoutForRange` for the whole text. Paragraphs whose text, attributes, attachments and available width
    /// did not change keep their lines.
    pub fn ensure_layout(&mut self, content: &TextContentStorage, container: &TextContainer, measure: &dyn Fn(&str) -> f32) {
        let pad = container.line_fragment_padding.max(0.0);
        let avail = (container.width - 2.0 * pad).max(0.0);
        let natural = self.natural_line_height;
        self.avail = avail;
        let text = content.text();
        let paras = content.paragraphs();
        let markers = content.list_markers();
        let mut old = std::mem::take(&mut self.cache);
        let mut fresh: Vec<Cached> = Vec::with_capacity(paras.len());
        self.relaid.clear();
        self.fragments.clear();
        let mut y = 0.0f32;
        for (i, r) in paras.iter().enumerate() {
            let ptext = &text[r.clone()];
            let attrs = content.paragraph_attrs(r);
            let atts = content.attachment_sizes(r.clone(), avail);
            let same = |c: &Cached| {
                c.text == ptext && c.attrs == attrs && c.avail == avail && c.pad == pad && c.natural == natural && c.atts == atts
            };
            let cached = if let Some(c) = fresh.iter().find(|c| same(c)) {
                c.clone()
            } else if let Some(k) = old.iter().position(|c| same(c)) {
                old.remove(k)
            } else {
                self.relaid.push(i);
                let lines = layout_paragraph(ptext, &attrs.style, avail, pad, &atts, natural, measure);
                Cached { text: ptext.to_string(), attrs, avail, pad, natural, atts, lines }
            };
            let style = &cached.attrs.style;
            if i > 0 {
                y += style.paragraph_spacing_before;
            }
            let top = y;
            let mut lines = Vec::with_capacity(cached.lines.len());
            for (n, l) in cached.lines.iter().enumerate() {
                lines.push(TextLineFragment {
                    range: r.start + l.start..r.start + l.end,
                    x: l.x,
                    y,
                    width: l.width,
                    height: l.height,
                    first_in_paragraph: n == 0,
                });
                y += l.height;
            }
            self.fragments.push(TextLayoutFragment {
                range: r.clone(),
                y: top,
                height: y - top,
                marker: markers.get(i).cloned().flatten(),
                lines,
            });
            y += style.paragraph_spacing;
            fresh.push(cached);
        }
        self.used_height = y;
        self.cache = fresh;
    }

    fn flat_lines(&self) -> Vec<&TextLineFragment> {
        self.fragments.iter().flat_map(|f| f.lines.iter()).collect()
    }

    /// The index (counting all lines) of the line that holds `offset`: the last line that starts at or before it.
    pub fn line_index_for_offset(&self, offset: usize) -> usize {
        self.flat_lines().iter().rposition(|l| l.range.start <= offset).unwrap_or(0)
    }

    /// The line with flat index `index`.
    pub fn line(&self, index: usize) -> Option<&TextLineFragment> {
        self.flat_lines().get(index).copied()
    }

    pub fn line_count(&self) -> usize {
        self.fragments.iter().map(|f| f.lines.len()).sum()
    }

    fn x_on_line(&self, line: &TextLineFragment, content: &TextContentStorage, offset: usize, measure: &dyn Fn(&str) -> f32) -> f32 {
        let text = content.text();
        let off = offset.clamp(line.range.start, line.range.end);
        let style = content.attrs_at(line.range.start).style;
        let indent = style.indent(line.first_in_paragraph);
        let adv = advance_x(&style, &text[line.range.start..off], line.range.start, content, self.avail, indent, measure);
        line.x + (adv - indent)
    }

    /// The x of the caret at `offset` (`locationForCharacterAtOffset`).
    pub fn x_of(&self, content: &TextContentStorage, offset: usize, measure: &dyn Fn(&str) -> f32) -> f32 {
        let lines = self.flat_lines();
        match lines.get(self.line_index_for_offset(offset)) {
            Some(line) => self.x_on_line(line, content, offset, measure),
            None => 0.0,
        }
    }

    /// The character boundary on line `index` nearest to `x`. Ties go to the earlier boundary.
    pub fn offset_at_x(&self, content: &TextContentStorage, index: usize, x: f32, measure: &dyn Fn(&str) -> f32) -> usize {
        let lines = self.flat_lines();
        let Some(line) = lines.get(index) else { return 0 };
        let text = content.text();
        let (mut best, mut best_d) = (line.range.start, f32::MAX);
        let mut b = line.range.start;
        loop {
            let d = (self.x_on_line(line, content, b, measure) - x).abs();
            if d < best_d {
                best_d = d;
                best = b;
            }
            if b >= line.range.end {
                break;
            }
            b += text[b..line.range.end].chars().next().map_or(1, |c| c.len_utf8());
        }
        best
    }

    /// The offset under a point in the container: the line whose band holds `y` (the first or last line outside the
    /// text), then the nearest boundary to `x`.
    pub fn offset_at_point(&self, content: &TextContentStorage, x: f32, y: f32, measure: &dyn Fn(&str) -> f32) -> usize {
        let lines = self.flat_lines();
        if lines.is_empty() {
            return 0;
        }
        let index = lines.iter().rposition(|l| l.y <= y).unwrap_or(0);
        self.offset_at_x(content, index, x, measure)
    }
}

/// A move direction (`NSTextSelectionNavigation.Direction`, left to right text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// How far one move goes (`NSTextSelection.Granularity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Character,
    Word,
    Line,
    Paragraph,
}

/// A selection (`NSTextSelection`): the anchor, the moving head, and the x that up and down moves keep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSelection {
    pub anchor: usize,
    pub head: usize,
    pub goal_x: Option<f32>,
}

impl TextSelection {
    pub fn caret(at: usize) -> Self {
        TextSelection { anchor: at, head: at, goal_x: None }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// `destinationSelection`: the selection after moving `direction` by `granularity`. With `extend` the anchor
    /// stays; without it a non-empty selection moved by characters collapses to its edge.
    pub fn moved(
        &self,
        layout: &TextLayoutManager,
        content: &TextContentStorage,
        direction: Direction,
        granularity: Granularity,
        extend: bool,
        measure: &dyn Fn(&str) -> f32,
    ) -> TextSelection {
        let horizontal = matches!(direction, Direction::Left | Direction::Right);
        if !extend && !self.is_empty() && horizontal && granularity == Granularity::Character {
            let r = self.range();
            let p = if direction == Direction::Left { r.start } else { r.end };
            return TextSelection::caret(p);
        }
        let mut goal = self.goal_x;
        let head = destination(layout, content, self.head, direction, granularity, &mut goal, measure);
        TextSelection { anchor: if extend { self.anchor } else { head }, head, goal_x: goal }
    }
}

fn destination(
    layout: &TextLayoutManager,
    content: &TextContentStorage,
    from: usize,
    direction: Direction,
    granularity: Granularity,
    goal: &mut Option<f32>,
    measure: &dyn Fn(&str) -> f32,
) -> usize {
    let text = content.text();
    let from = from.min(text.len());
    let vertical = matches!(direction, Direction::Up | Direction::Down);
    if vertical && granularity != Granularity::Paragraph {
        let lines = layout.flat_lines();
        if lines.is_empty() {
            return from;
        }
        let index = layout.line_index_for_offset(from);
        let x = match *goal {
            Some(x) => x,
            None => layout.x_on_line(lines[index], content, from, measure),
        };
        *goal = Some(x);
        return if direction == Direction::Down {
            if index + 1 >= lines.len() {
                text.len()
            } else {
                layout.offset_at_x(content, index + 1, x, measure)
            }
        } else if index == 0 {
            0
        } else {
            layout.offset_at_x(content, index - 1, x, measure)
        };
    }
    *goal = None;
    let backward = matches!(direction, Direction::Left | Direction::Up);
    match granularity {
        Granularity::Character => {
            if backward {
                prev_boundary(text, from)
            } else {
                next_boundary(text, from)
            }
        }
        Granularity::Word => {
            if backward {
                prev_word_start(text, from)
            } else {
                next_word_end(text, from)
            }
        }
        Granularity::Line => {
            let lines = layout.flat_lines();
            match lines.get(layout.line_index_for_offset(from)) {
                Some(l) if backward => l.range.start,
                Some(l) => l.range.end,
                None => from,
            }
        }
        Granularity::Paragraph => {
            if backward {
                let start = line_start(text, from);
                if from > start {
                    start
                } else if from > 0 {
                    line_start(text, from - 1)
                } else {
                    0
                }
            } else {
                let end = line_end(text, from);
                if from < end {
                    end
                } else if from < text.len() {
                    line_end(text, from + 1)
                } else {
                    from
                }
            }
        }
    }
}
