//! Text selection over recognised text in an image: the model behind a system image-analysis overlay's
//! text selection, in pure Rust. The caller recognises the text and lays it out as
//! blocks in reading order, each with its bounds in the image view. This module owns hit testing, the
//! selection range across blocks, the copied text, and data detection (links, dates, addresses, phone numbers)
//! inside the selection. Character positions come from the caller's glyph layout; nothing here measures text.

use std::ops::Range;

use crate::detect::{self, Kind, Match};
use crate::Rect;

/// One run of recognised text and where it sits in the view.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub text: String,
    pub bounds: Rect,
}

/// A place in the text: a block index and a byte offset that is a char boundary inside that block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub block: usize,
    pub offset: usize,
}

/// A selected range, always stored with `start <= end` in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

/// Recognised text blocks with a selection. Blocks keep the order the caller gave them.
#[derive(Debug, Default)]
pub struct Overlay {
    blocks: Vec<TextBlock>,
    selection: Option<Selection>,
}

/// The byte offset of char `n` in `text`, or the end of the text when `n` is past its last char.
fn byte_at(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(b, _)| b)
}

/// Char classes for word selection: a run of one class is selected together.
fn class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

impl Overlay {
    pub fn new(blocks: Vec<TextBlock>) -> Self {
        Self { blocks, selection: None }
    }

    pub fn blocks(&self) -> &[TextBlock] {
        &self.blocks
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    /// The topmost block under the point (the later block wins when two overlap), if any.
    pub fn block_at(&self, x: f32, y: f32) -> Option<usize> {
        self.blocks.iter().rposition(|b| b.bounds.contains(x, y))
    }

    /// Selects all text in the overlay. An overlay without text has no selection.
    pub fn select_all(&mut self) {
        self.selection = self.blocks.last().map(|last| Selection {
            start: Position { block: 0, offset: 0 },
            end: Position { block: self.blocks.len() - 1, offset: last.text.len() },
        });
    }

    /// Selects the run of same-class characters (word, space or punctuation) around char `index` of `block`.
    /// Returns false when the block or the char does not exist.
    pub fn select_word(&mut self, block: usize, index: usize) -> bool {
        let Some(b) = self.blocks.get(block) else {
            return false;
        };
        let chars: Vec<char> = b.text.chars().collect();
        let Some(&c) = chars.get(index) else {
            return false;
        };
        let k = class(c);
        let mut from = index;
        while from > 0 && class(chars[from - 1]) == k {
            from -= 1;
        }
        let mut to = index + 1;
        while to < chars.len() && class(chars[to]) == k {
            to += 1;
        }
        let start = Position { block, offset: byte_at(&b.text, from) };
        let end = Position { block, offset: byte_at(&b.text, to) };
        self.selection = Some(Selection { start, end });
        true
    }

    /// Selects from char `from` of block `a` to char `to` of block `b`, in either order. Char indexes past the
    /// end of a block are clamped to its end. Returns false when either block does not exist.
    pub fn select_range(&mut self, a: (usize, usize), b: (usize, usize)) -> bool {
        let (Some(ba), Some(bb)) = (self.blocks.get(a.0), self.blocks.get(b.0)) else {
            return false;
        };
        let p = Position { block: a.0, offset: byte_at(&ba.text, a.1) };
        let q = Position { block: b.0, offset: byte_at(&bb.text, b.1) };
        self.selection = Some(if p <= q { Selection { start: p, end: q } } else { Selection { start: q, end: p } });
        true
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The byte ranges of the selection inside each block it touches, in reading order.
    fn segments(&self) -> Vec<(usize, Range<usize>)> {
        let Some(sel) = self.selection else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            if i < sel.start.block || i > sel.end.block {
                continue;
            }
            let from = if i == sel.start.block { sel.start.offset } else { 0 };
            let to = if i == sel.end.block { sel.end.offset } else { b.text.len() };
            if from < to {
                out.push((i, from..to));
            }
        }
        out
    }

    /// The selected text. Blocks are joined with a newline, as text in separate lines is copied.
    pub fn selected_text(&self) -> String {
        self.segments()
            .into_iter()
            .map(|(i, r)| self.blocks[i].text[r].to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Data detections inside the selection, as (block index, match). Match ranges are byte ranges into the
    /// block's full text, so they can be drawn over the recognised text. Detection runs per selected segment,
    /// so a link cannot span two blocks.
    pub fn data_matches(&self, kinds: &[Kind]) -> Vec<(usize, Match)> {
        let mut out = Vec::new();
        for (block, range) in self.segments() {
            let base = range.start;
            let segment = &self.blocks[block].text[range];
            for m in detect::find(segment, kinds) {
                out.push((block, Match { kind: m.kind, range: base + m.range.start..base + m.range.end }));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str, x: f32, y: f32) -> TextBlock {
        TextBlock { text: text.to_string(), bounds: Rect { x, y, w: 100.0, h: 20.0 } }
    }

    fn sample() -> Overlay {
        Overlay::new(vec![block("Call 555-0100 today", 0.0, 0.0), block("see https://example.com/a", 0.0, 30.0)])
    }

    #[test]
    fn hit_testing_prefers_the_later_block_when_blocks_overlap() {
        let o = Overlay::new(vec![block("under", 0.0, 0.0), block("over", 0.0, 0.0)]);
        assert_eq!(o.block_at(5.0, 5.0), Some(1));
        assert_eq!(o.block_at(5.0, 25.0), None);
    }

    #[test]
    fn select_all_copies_every_block_on_its_own_line() {
        let mut o = sample();
        o.select_all();
        assert_eq!(o.selected_text(), "Call 555-0100 today\nsee https://example.com/a");
        assert_eq!(Overlay::new(Vec::new()).selection(), None);
    }

    #[test]
    fn a_range_across_blocks_is_ordered_whichever_end_is_given_first() {
        let mut o = sample();
        // Given backwards: the selection still starts at block 0, char 5 and ends at block 1, char 4.
        assert!(o.select_range((1, 4), (0, 5)));
        assert_eq!(o.selected_text(), "555-0100 today\nsee ");
        assert!(!o.select_range((9, 0), (0, 0)), "missing block");
    }

    #[test]
    fn a_word_selection_expands_over_one_character_class() {
        let mut o = sample();
        // "Call 555-0100 today": char 6 is a digit; the run stops at the dash (another class).
        assert!(o.select_word(0, 6));
        assert_eq!(o.selected_text(), "555");
    }

    #[test]
    fn the_word_selection_stops_at_the_class_boundary() {
        let mut o = Overlay::new(vec![block("hello, world", 0.0, 0.0)]);
        assert!(o.select_word(0, 1));
        assert_eq!(o.selected_text(), "hello");
        assert!(o.select_word(0, 5));
        assert_eq!(o.selected_text(), ",");
        assert!(!o.select_word(0, 99));
    }

    #[test]
    fn data_detection_runs_inside_the_selection_only() {
        let mut o = sample();
        // Select only the second block's text: the phone number in the first block is outside it.
        assert!(o.select_range((1, 0), (1, 40)));
        let found = o.data_matches(&[Kind::Link, Kind::Phone]);
        assert_eq!(found.len(), 1);
        let (block, m) = &found[0];
        assert_eq!(*block, 1);
        assert_eq!(m.kind, Kind::Link);
        assert_eq!(&o.blocks()[1].text[m.range.clone()], "https://example.com/a");
    }

    #[test]
    fn detections_map_back_to_full_block_offsets_when_the_selection_starts_mid_block() {
        let mut o = sample();
        assert!(o.select_range((0, 5), (0, 19)));
        let found = o.data_matches(&[Kind::Phone]);
        assert_eq!(found.len(), 1);
        let (block, m) = &found[0];
        assert_eq!(*block, 0);
        assert_eq!(&o.blocks()[0].text[m.range.clone()], "555-0100");
    }
}
