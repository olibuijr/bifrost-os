//! Text storage and text containers (`NativeTextStorage`, `NativeTextContainer`). A pure model.
//!
//! * `TextStorage`: the text plus attribute runs (byte ranges with an attribute id, 0 for none). Every edit shifts the
//!   runs behind it, clips the runs it covers, and records the edited range and the change in length until
//!   `process_editing` takes them. Each edit bumps a generation counter, which layout uses to know it is stale.
//! * `TextContainer`: the region text is laid out in. Width, height (`f32::INFINITY` is unbounded), line fragment
//!   padding, a maximum line count, and exclusion rectangles that text flows around.
//!
//! Line layout, line breaking and position queries are in `typeset` (the typesetter, framesetter, frame and line
//! objects). Offsets here are UTF-8 byte offsets.

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

    /// Identifies the storage; unique per storage.
    pub fn id(&self) -> u64 {
        self.id
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
