//! Text storage, line layout and text containers (`NativeTextStorage`, `NativeLayoutManager`, `NativeTextContainer`, and
//! the line and run objects of the typesetter). A pure model: the caller supplies the font through `measure`, which
//! returns the width of a string in points, the same contract as `textfit`.
//!
//! * `TextStorage`: the text plus attribute runs (byte ranges with an attribute id, 0 for none). Every edit shifts the
//!   runs behind it, clips the runs it covers, and records the edited range and the change in length until
//!   `process_editing` takes them. Each edit bumps a generation counter, which layout uses to know it is stale.
//! * `TextContainer`: the region text is laid out in. Width, height (`f32::INFINITY` is unbounded), line fragment
//!   padding, a maximum line count, and exclusion rectangles that text flows around.
//! * `LayoutManager`: the line fragments of a storage in a container, cached until the storage or the container
//!   changes. It answers character to line, point to character, character range to rectangles, and line to runs.
//! * `suggest_line_break`: the typesetter's break suggestion. Breaks after spaces; a word wider than the line is split
//!   by characters; a newline always breaks. Trailing spaces hang past the width.
//!
//! Offsets are UTF-8 byte offsets. Not modelled: hyphenation, bidirectional runs, glyph-level positions and
//! kerning (the caller's `measure` decides widths), non-rectangular containers, and the empty line after a
//! trailing newline (a storage that ends in a newline gets no fragment for the empty last line).

use crate::Rect;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// An attribute id. Zero means no attribute.
pub type Attribute = u32;
/// The attribute of text that has none.
pub const NO_ATTRIBUTE: Attribute = 0;

/// Walks an offset through an edit that replaced `a..b` with `n` bytes. Offsets before the edit stay, offsets after
/// it move by the change in length, and an offset inside the replaced text goes to the start or the end of the new
/// text, depending on `start`.
fn map_pos(x: usize, a: usize, b: usize, n: usize, start: bool) -> usize {
    if x <= a {
        x
    } else if x >= b {
        x - (b - a) + n
    } else if start {
        a
    } else {
        a + n
    }
}

/// Removes the part of each run inside `a..b` and moves the runs behind it, for an edit that put `n` bytes there.
fn clip_out(runs: &[(Range<usize>, Attribute)], a: usize, b: usize, n: usize) -> Vec<(Range<usize>, Attribute)> {
    let mut out = Vec::with_capacity(runs.len() + 1);
    for (r, attr) in runs {
        let (s, e) = (r.start, r.end);
        if e <= a {
            out.push((s..e, *attr));
            continue;
        }
        if s >= b {
            out.push((map_pos(s, a, b, n, true)..map_pos(e, a, b, n, false), *attr));
            continue;
        }
        // The run overlaps the replaced text: keep what lies before it and what lies after it.
        if s < a {
            out.push((s..a, *attr));
        }
        if e > b {
            out.push(((a + n)..map_pos(e, a, b, n, false), *attr));
        }
    }
    out.retain(|(r, _)| r.start < r.end);
    out
}

/// The change to a storage's text reported by `TextStorage::process_editing`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditedRange {
    /// The edited text in the storage's current offsets.
    pub range: Range<usize>,
    /// The difference between the storage's length now and before the pending edits.
    pub change_in_length: isize,
}

static NEXT_STORAGE_ID: AtomicU64 = AtomicU64::new(1);

/// Text with attribute runs and edit tracking (`NativeTextStorage`).
#[derive(Debug)]
pub struct TextStorage {
    id: u64,
    text: String,
    /// Non-empty, sorted, non-overlapping runs of non-zero attributes.
    runs: Vec<(Range<usize>, Attribute)>,
    generation: u64,
    edited: Option<Range<usize>>,
    pending_change: isize,
}

impl TextStorage {
    pub fn new(text: &str) -> Self {
        Self {
            id: NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed),
            text: text.to_string(),
            runs: Vec::new(),
            generation: 0,
            edited: None,
            pending_change: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Changes on every edit, text or attribute. Layout caches compare it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Replaces the bytes at `range` with `new`. Returns false, and changes nothing, when the range is out of bounds
    /// or does not start and end on character boundaries.
    pub fn replace_characters(&mut self, range: Range<usize>, new: &str) -> bool {
        if !self.valid_range(&range) {
            return false;
        }
        let (a, b, n) = (range.start, range.end, new.len());
        self.text.replace_range(a..b, new);
        self.runs = clip_out(&self.runs, a, b, n);
        self.record(a, b, n);
        self.pending_change += n as isize - (b - a) as isize;
        true
    }

    /// Sets the attribute of every byte in `range`, replacing what was there. Zero clears it.
    pub fn set_attribute(&mut self, range: Range<usize>, attr: Attribute) -> bool {
        if !self.valid_range(&range) {
            return false;
        }
        let (a, b) = (range.start, range.end);
        let mut runs = clip_out(&self.runs, a, b, b - a);
        if attr != NO_ATTRIBUTE && a < b {
            runs.push((a..b, attr));
        }
        runs.sort_by_key(|(r, _)| r.start);
        self.runs = runs;
        self.record(a, b, b - a);
        true
    }

    /// The attribute of the byte at `index`, zero when none is set or the index is past the end.
    pub fn attribute_at(&self, index: usize) -> Attribute {
        self.runs
            .iter()
            .find(|(r, _)| r.start <= index && index < r.end)
            .map(|(_, attr)| *attr)
            .unwrap_or(NO_ATTRIBUTE)
    }

    /// The runs covering `range`, in order, with zero-attribute gaps filled in. Empty for an empty range.
    pub fn attribute_runs(&self, range: Range<usize>) -> Vec<(Range<usize>, Attribute)> {
        let mut out = Vec::new();
        let mut pos = range.start;
        for (r, attr) in &self.runs {
            if r.end <= pos || r.start >= range.end {
                continue;
            }
            let s = r.start.max(pos);
            let e = r.end.min(range.end);
            if s > pos {
                out.push((pos..s, NO_ATTRIBUTE));
            }
            out.push((s..e, *attr));
            pos = e;
        }
        if pos < range.end {
            out.push((pos..range.end, NO_ATTRIBUTE));
        }
        out
    }

    /// Takes the pending edit, if any, and resets the change in length (`processEditing`). `None` when nothing was
    /// edited since the last call.
    pub fn process_editing(&mut self) -> Option<EditedRange> {
        let range = self.edited.take()?;
        let change_in_length = std::mem::take(&mut self.pending_change);
        Some(EditedRange { range, change_in_length })
    }

    fn valid_range(&self, r: &Range<usize>) -> bool {
        r.start <= r.end
            && r.end <= self.text.len()
            && self.text.is_char_boundary(r.start)
            && self.text.is_char_boundary(r.end)
    }

    /// Notes that `a..b` became `n` bytes long: grows the pending edited range through the edit, then bumps the
    /// generation.
    fn record(&mut self, a: usize, b: usize, n: usize) {
        let new = a..a + n;
        self.edited = Some(match self.edited.take() {
            None => new,
            Some(p) => {
                let s = map_pos(p.start, a, b, n, true);
                let e = map_pos(p.end, a, b, n, false);
                s.min(new.start)..e.max(new.end)
            }
        });
        self.generation += 1;
    }
}

/// The break suggestion of the typesetter: the end of the line that starts at byte `start` and fits in `width`.
/// Returns an offset after the last space that fits, or the last character that fits when the line holds no space.
/// A newline ends the line after itself. The first character is always taken, so the result is always past `start`
/// when there is text left, and `start` only when there is none.
pub fn suggest_line_break(text: &str, start: usize, width: f32, measure: &dyn Fn(&str) -> f32) -> usize {
    let rest = &text[start..];
    let mut fit: Option<usize> = None;
    let mut brk: Option<usize> = None;
    let mut overflow = false;
    for (i, ch) in rest.char_indices() {
        let end = start + i + ch.len_utf8();
        if ch == '\n' {
            return end;
        }
        // Spaces hang past the edge; any other character that would overflow ends the line.
        if fit.is_some() && ch != ' ' && measure(&text[start..end]) > width {
            overflow = true;
            break;
        }
        fit = Some(end);
        if ch == ' ' {
            brk = Some(end);
        }
    }
    match (overflow, brk) {
        (true, Some(b)) => b,
        _ => fit.unwrap_or(start),
    }
}

/// The region text is laid out in (`NativeTextContainer`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextContainer {
    pub width: f32,
    /// `f32::INFINITY` for a container that grows downward without limit.
    pub height: f32,
    /// Inset applied on each side of every line fragment.
    pub line_fragment_padding: f32,
    /// Zero for no limit.
    pub maximum_lines: usize,
    /// Rectangles text flows around. A line whose band crosses one gets only the widest free gap beside it.
    pub exclusions: Vec<Rect>,
}

impl TextContainer {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height, line_fragment_padding: 0.0, maximum_lines: 0, exclusions: Vec::new() }
    }

    /// The horizontal extent `(x, width)` a line in the band `y..y+h` may use, inside the padding. The widest free gap
    /// between the exclusions that cross the band is used. `None` when no gap is wide enough for the padding.
    pub fn line_fragment_rect(&self, y: f32, h: f32) -> Option<(f32, f32)> {
        let pad = self.line_fragment_padding.max(0.0);
        let mut blocked: Vec<(f32, f32)> = self
            .exclusions
            .iter()
            .filter(|e| e.y < y + h && y < e.y + e.h && e.w > 0.0)
            .map(|e| (e.x.max(0.0), (e.x + e.w).min(self.width)))
            .collect();
        blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best = (0.0f32, 0.0f32);
        let mut cursor = 0.0f32;
        for (s, e) in blocked.into_iter().chain(std::iter::once((self.width, self.width))) {
            if s > cursor && s - cursor > best.1 - best.0 {
                best = (cursor, s);
            }
            cursor = cursor.max(e);
        }
        let (x0, x1) = best;
        let w = (x1 - x0) - 2.0 * pad;
        (w > 0.0).then_some((x0 + pad, w))
    }
}

/// One line of a laid-out storage (a line of the layout; a line fragment).
#[derive(Clone, Debug, PartialEq)]
pub struct LineFragment {
    /// The bytes on the line, including its trailing newline when it has one.
    pub chars: Range<usize>,
    /// The line box in container coordinates.
    pub rect: Rect,
    /// The width of the text on the line, without the newline.
    pub used_width: f32,
}

/// A run of one attribute on a line : its bytes, its attribute and its horizontal placement.
#[derive(Clone, Debug, PartialEq)]
pub struct LineRun {
    pub chars: Range<usize>,
    pub attr: Attribute,
    /// Left edge in container coordinates.
    pub x: f32,
    pub width: f32,
}

/// Bytes of a line that are drawn, without its trailing newline.
fn content_end(text: &str, chars: &Range<usize>) -> usize {
    if chars.end > chars.start && text.as_bytes()[chars.end - 1] == b'\n' {
        chars.end - 1
    } else {
        chars.end
    }
}

/// Lines that stop being laid out after this many bands blocked by exclusions, so a container that no text fits in
/// cannot loop forever.
const MAX_SKIPPED_BANDS: usize = 10_000;

/// The layout key: storage identity, storage generation, container, line height bits.
type LayoutKey = (u64, u64, TextContainer, u32);

/// Lays a storage out in a container and answers position queries (`NativeLayoutManager`, and the frame of the
/// typesetter). Queries read the last layout, so call `ensure_layout` first.
#[derive(Debug, Default)]
pub struct LayoutManager {
    /// The height of every line box.
    pub line_height: f32,
    lines: Vec<LineFragment>,
    key: Option<LayoutKey>,
}

impl LayoutManager {
    pub fn new(line_height: f32) -> Self {
        Self { line_height, lines: Vec::new(), key: None }
    }

    /// Lays out `storage` in `container`. Does nothing when the last layout used the same storage state, container and
    /// line height. A non-positive line height lays out nothing.
    pub fn ensure_layout(&mut self, storage: &TextStorage, container: &TextContainer, measure: &dyn Fn(&str) -> f32) {
        let key: LayoutKey = (storage.id, storage.generation, container.clone(), self.line_height.to_bits());
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.lines.clear();
        if !(self.line_height > 0.0) {
            return;
        }
        let text = storage.text();
        let max_lines = if container.maximum_lines == 0 { usize::MAX } else { container.maximum_lines };
        let (mut pos, mut y, mut skipped) = (0usize, 0.0f32, 0usize);
        while pos < text.len() && self.lines.len() < max_lines && y + self.line_height <= container.height {
            let (x, w) = match container.line_fragment_rect(y, self.line_height) {
                Some(v) => v,
                None => {
                    skipped += 1;
                    if skipped > MAX_SKIPPED_BANDS {
                        break;
                    }
                    y += self.line_height;
                    continue;
                }
            };
            let end = suggest_line_break(text, pos, w, measure);
            let used_width = measure(&text[pos..content_end(text, &(pos..end))]);
            self.lines.push(LineFragment {
                chars: pos..end,
                rect: Rect { x, y, w, h: self.line_height },
                used_width,
            });
            pos = end;
            y += self.line_height;
        }
    }

    /// The laid-out lines, top to bottom.
    pub fn lines(&self) -> &[LineFragment] {
        &self.lines
    }

    /// The bytes that fit in the container.
    pub fn visible_range(&self) -> Range<usize> {
        match (self.lines.first(), self.lines.last()) {
            (Some(a), Some(b)) => a.chars.start..b.chars.end,
            _ => 0..0,
        }
    }

    /// True when the storage holds text past the visible range.
    pub fn is_truncated(&self, storage: &TextStorage) -> bool {
        self.visible_range().end < storage.len()
    }

    /// The index of the line holding byte `index`.
    pub fn line_index_for_character(&self, index: usize) -> Option<usize> {
        self.lines.iter().position(|f| f.chars.start <= index && index < f.chars.end)
    }

    /// The byte boundary nearest to point `(x, y)` on the line under `y`. `None`
    /// when no line covers `y`.
    pub fn character_index_at(&self, storage: &TextStorage, measure: &dyn Fn(&str) -> f32, x: f32, y: f32) -> Option<usize> {
        let f = self.lines.iter().find(|f| y >= f.rect.y && y < f.rect.y + f.rect.h)?;
        let text = storage.text();
        let (s, ce) = (f.chars.start, content_end(text, &f.chars));
        let mut best: Option<(f32, usize)> = None;
        for b in s..=ce {
            if !text.is_char_boundary(b) {
                continue;
            }
            let bx = f.rect.x + measure(&text[s..b]);
            let d = (bx - x).abs();
            if best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, b));
            }
        }
        best.map(|(_, b)| b)
    }

    /// One rectangle per line that holds part of `range` (`usageBoundsForTextContainer` for a range). Empty when the
    /// range is not on character boundaries of the storage.
    pub fn rects_for_range(&self, storage: &TextStorage, measure: &dyn Fn(&str) -> f32, range: Range<usize>) -> Vec<Rect> {
        let text = storage.text();
        let mut out = Vec::new();
        if range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return out;
        }
        for f in &self.lines {
            let ce = content_end(text, &f.chars);
            let s = range.start.max(f.chars.start);
            let e = range.end.min(ce);
            if s >= e {
                continue;
            }
            let x0 = f.rect.x + measure(&text[f.chars.start..s]);
            let x1 = f.rect.x + measure(&text[f.chars.start..e]);
            out.push(Rect { x: x0, y: f.rect.y, w: x1 - x0, h: f.rect.h });
        }
        out
    }

    /// The attribute runs of line `line`, placed along it. Empty for an unknown line.
    pub fn runs_in_line(&self, storage: &TextStorage, measure: &dyn Fn(&str) -> f32, line: usize) -> Vec<LineRun> {
        let Some(f) = self.lines.get(line) else {
            return Vec::new();
        };
        let text = storage.text();
        let ce = content_end(text, &f.chars);
        let mut out = Vec::new();
        for (r, attr) in storage.attribute_runs(f.chars.start..ce) {
            let x = f.rect.x + measure(&text[f.chars.start..r.start]);
            let width = measure(&text[r.start..r.end]);
            out.push(LineRun { chars: r, attr, x, width });
        }
        out
    }

    /// The box around all text: each line from its left edge over its used width.
    pub fn used_rect(&self) -> Rect {
        self.lines
            .iter()
            .map(|f| Rect { x: f.rect.x, y: f.rect.y, w: f.used_width, h: f.rect.h })
            .reduce(|a, b| a.union(b))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten points per character.
    fn m(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn break_after_space_and_split_long_words() {
        // "aa bb " fits in 50 with its trailing space hanging; "aa bb c" does not.
        assert_eq!(suggest_line_break("aa bb cc dd", 0, 50.0, &m), 6);
        assert_eq!(suggest_line_break("aa bb cc dd", 6, 50.0, &m), 11);
        // No space: the word is split after the last character that fits.
        assert_eq!(suggest_line_break("abcdefg", 0, 30.0, &m), 3);
        assert_eq!(suggest_line_break("abcdefg", 3, 30.0, &m), 6);
        // A newline ends the line after itself, and an empty rest gives the start back.
        assert_eq!(suggest_line_break("ab\ncd", 0, 100.0, &m), 3);
        assert_eq!(suggest_line_break("ab", 2, 100.0, &m), 2);
    }

    #[test]
    fn storage_edits_shift_and_clip_attribute_runs() {
        let mut s = TextStorage::new("hello world");
        assert!(s.set_attribute(6..11, 7));
        assert_eq!(s.attribute_runs(0..11), vec![(0..6, 0), (6..11, 7)]);
        // Insert "+++" at the start: the run moves from 6..11 to 9..14.
        assert!(s.replace_characters(0..0, "+++"));
        assert_eq!(s.text(), "+++hello world");
        assert_eq!(s.attribute_at(8), NO_ATTRIBUTE);
        assert_eq!(s.attribute_at(9), 7);
        assert_eq!(s.attribute_at(13), 7);
        assert_eq!(s.attribute_at(14), NO_ATTRIBUTE);
        // Replacing "wor" (9..12) with "X" clears the bytes it replaced; the run keeps the bytes after it ("rld" -> "ld").
        assert!(s.replace_characters(9..12, "X"));
        assert_eq!(s.text(), "+++hello Xld");
        assert_eq!(s.attribute_runs(9..12), vec![(9..10, NO_ATTRIBUTE), (10..12, 7)]);
    }

    #[test]
    fn process_editing_unions_edits_and_sums_length() {
        let mut s = TextStorage::new("hello world");
        let g0 = s.generation();
        assert!(s.set_attribute(6..11, 7));
        assert_eq!(s.process_editing(), Some(EditedRange { range: 6..11, change_in_length: 0 }));
        assert_eq!(s.process_editing(), None);
        assert!(s.set_attribute(6..11, 7));
        assert!(s.replace_characters(0..0, "+++"));
        // The pending range 6..11 moves to 9..14 through the insert, then joins the new text at 0..3.
        assert_eq!(s.process_editing(), Some(EditedRange { range: 0..14, change_in_length: 3 }));
        assert!(s.generation() > g0);
        // Invalid edits change nothing: out of bounds and not on a character boundary.
        let before = s.generation();
        assert!(!s.replace_characters(0..99, "x"));
        assert_eq!(s.generation(), before);
        let mut u = TextStorage::new("é");
        assert!(!u.replace_characters(1..2, "x"));
    }

    #[test]
    fn exclusion_leaves_the_widest_gap() {
        let mut c = TextContainer::new(100.0, f32::INFINITY);
        c.exclusions.push(Rect { x: 0.0, y: 0.0, w: 40.0, h: 10.0 });
        // The band 0..10 is blocked on the left, so the free gap is 40..100.
        assert_eq!(c.line_fragment_rect(0.0, 10.0), Some((40.0, 60.0)));
        // The band 10..20 does not touch the exclusion.
        assert_eq!(c.line_fragment_rect(10.0, 10.0), Some((0.0, 100.0)));
        // Padding insets both ends of the gap.
        c.line_fragment_padding = 5.0;
        assert_eq!(c.line_fragment_rect(0.0, 10.0), Some((45.0, 50.0)));
        // A band blocked from edge to edge has no room.
        c.exclusions.push(Rect { x: 40.0, y: 0.0, w: 60.0, h: 10.0 });
        assert_eq!(c.line_fragment_rect(0.0, 10.0), None);
    }

    #[test]
    fn layout_wraps_limits_lines_and_answers_queries() {
        let s = TextStorage::new("aa bb cc dd");
        let mut lm = LayoutManager::new(10.0);
        lm.ensure_layout(&s, &TextContainer::new(50.0, f32::INFINITY), &m);
        // Lines "aa bb " (0..6) and "cc dd" (6..11).
        let l = lm.lines();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].chars, 0..6);
        // The trailing space of "aa bb " counts in the used width: 6 characters.
        assert_eq!(l[0].used_width, 60.0);
        assert_eq!(l[1].rect, Rect { x: 0.0, y: 10.0, w: 50.0, h: 10.0 });
        assert_eq!(lm.visible_range(), 0..11);
        assert_eq!(lm.line_index_for_character(7), Some(1));
        // The "bb" glyphs start after "aa " (30) and span 20 points on the first line.
        assert_eq!(lm.rects_for_range(&s, &m, 3..5), vec![Rect { x: 30.0, y: 0.0, w: 20.0, h: 10.0 }]);
        // Point (x=100, y=15) is past the end of "cc dd": the nearest boundary is 11.
        assert_eq!(lm.character_index_at(&s, &m, 100.0, 15.0), Some(11));
        assert_eq!(lm.character_index_at(&s, &m, 0.0, 5.0), Some(0));
        // Two lines of the hit test: x=0 on line 1 gives the start of "cc dd".
        assert_eq!(lm.character_index_at(&s, &m, 0.0, 12.0), Some(6));
        // A line cap truncates the layout.
        let mut capped = TextContainer::new(50.0, f32::INFINITY);
        capped.maximum_lines = 1;
        let mut lm2 = LayoutManager::new(10.0);
        lm2.ensure_layout(&s, &capped, &m);
        assert_eq!(lm2.visible_range(), 0..6);
        assert!(lm2.is_truncated(&s));
        // Runs along line 0: "aa " plain (0..3) and "bb" with attribute 2 (3..5) then " " plain.
        let mut s2 = TextStorage::new("aa bb cc dd");
        assert!(s2.set_attribute(3..5, 2));
        lm.ensure_layout(&s2, &TextContainer::new(50.0, f32::INFINITY), &m);
        let runs = lm.runs_in_line(&s2, &m, 0);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1], LineRun { chars: 3..5, attr: 2, x: 30.0, width: 20.0 });
        assert_eq!(runs[2].chars, 5..6);
        assert_eq!(runs[2].x, 50.0);
    }
}
