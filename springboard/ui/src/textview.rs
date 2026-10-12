//! A multi-line text view model: wrapped text in a scroll region, with a caret, a selection, vertical movement that keeps
//! a goal column in points, hit testing from a point, and scrolling that keeps the caret on screen. Editing goes through
//! `text::TextEditor`, so typing, undo and the key bindings are the ones the single-line field already uses.
//!
//! Layout is greedy word wrap against a caller-supplied `measure` (points for a string), the same rule as `textfit::wrap`,
//! but each line is kept as a byte range of the text so that carets and hit tests map back to offsets. A line excludes its
//! newline and the space that a wrap consumed. Positions are UTF-8 byte offsets, as in `text::TextBuffer`.
//!
//! Layout is recomputed on each query and nothing is cached, so a change of text or width needs no invalidation. The
//! model has no drawing: a caller draws `visible_lines`, the caret from `caret_point` and the `Highlight`s.

use std::ops::Range;

use crate::text::TextEditor;

/// Undo levels kept for the view's editor.
const UNDO_LEVELS: usize = 100;

/// One highlighted piece of a line: the selected part from `x0` to `x1` points, on line index `line`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub line: usize,
    pub x0: f32,
    pub x1: f32,
}

pub struct TextView {
    pub editor: TextEditor,
    width: f32,
    line_height: f32,
    viewport: f32,
    scroll_y: f32,
    /// The x (points) that vertical movement tries to keep. Cleared by any move or edit that is not vertical.
    goal_x: Option<f32>,
}

/// Greedy word wrap of the whole text. Returns one byte range per visual line, in order. An empty paragraph is an empty
/// line, and a word wider than `width` is split by characters.
pub fn layout(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut base = 0usize;
    for para in text.split('\n') {
        wrap_paragraph(para, base, width, measure, &mut lines);
        base += para.len() + 1;
    }
    lines
}

fn wrap_paragraph(para: &str, base: usize, width: f32, measure: &dyn Fn(&str) -> f32, out: &mut Vec<Range<usize>>) {
    if para.is_empty() {
        out.push(base..base);
        return;
    }
    // Offsets are relative to `para`; `base` is added when a line is pushed.
    let mut start = 0usize;
    let mut i = 0usize;
    // Offset just after the most recent space on the current line, where a word wrap can break.
    let mut after_space: Option<usize> = None;
    while i < para.len() {
        let ch = para[i..].chars().next().unwrap_or(' ');
        let next = i + ch.len_utf8();
        if i > start && measure(&para[start..next]) > width {
            if ch == ' ' {
                // The space itself is the break: the line ends before it and the space is consumed.
                out.push(base + start..base + i);
                start = next;
                after_space = None;
                i = next;
                continue;
            }
            match after_space.filter(|&b| b > start) {
                Some(b) => {
                    // Break after the last space; that space (one byte) is consumed.
                    out.push(base + start..base + b - 1);
                    start = b;
                }
                None => {
                    // No space to break at: split the word before this character.
                    out.push(base + start..base + i);
                    start = i;
                }
            }
            after_space = None;
            continue;
        }
        if ch == ' ' {
            after_space = Some(next);
        }
        i = next;
    }
    out.push(base + start..base + para.len());
}

/// The index of the visual line that holds byte offset `byte`: the last line that starts at or before it.
pub fn line_of(lines: &[Range<usize>], byte: usize) -> usize {
    lines.iter().rposition(|l| l.start <= byte).unwrap_or(0)
}

/// The x position (points) of byte offset `byte` on `line`. An offset outside the line is clamped to it.
pub fn x_of(text: &str, line: &Range<usize>, byte: usize, measure: &dyn Fn(&str) -> f32) -> f32 {
    let b = byte.clamp(line.start, line.end);
    measure(&text[line.start..b])
}

/// The character boundary on `line` whose x is nearest to `x`. Ties go to the earlier boundary.
pub fn byte_at(text: &str, line: &Range<usize>, x: f32, measure: &dyn Fn(&str) -> f32) -> usize {
    let mut best = line.start;
    let mut best_d = f32::MAX;
    let mut b = line.start;
    loop {
        let d = (measure(&text[line.start..b]) - x).abs();
        if d < best_d {
            best_d = d;
            best = b;
        }
        if b >= line.end {
            break;
        }
        b += text[b..line.end].chars().next().map_or(1, |c| c.len_utf8());
    }
    best
}

impl TextView {
    /// A view of `text`, wrapped to `width` points, with lines `line_height` points apart and `viewport` points tall.
    pub fn new(text: &str, width: f32, line_height: f32, viewport: f32) -> Self {
        TextView { editor: TextEditor::with_text(text, UNDO_LEVELS), width, line_height, viewport, scroll_y: 0.0, goal_x: None }
    }

    pub fn text(&self) -> &str {
        self.editor.buf.text()
    }

    /// The visual lines of the current text at the current width.
    pub fn lines(&self, measure: &dyn Fn(&str) -> f32) -> Vec<Range<usize>> {
        layout(self.text(), self.width, measure)
    }

    /// Changes the wrap width and the visible height; the scroll position is kept within range.
    pub fn resize(&mut self, width: f32, viewport: f32, measure: &dyn Fn(&str) -> f32) {
        self.width = width;
        self.viewport = viewport;
        self.set_scroll(self.scroll_y, measure);
    }

    pub fn content_height(&self, measure: &dyn Fn(&str) -> f32) -> f32 {
        self.lines(measure).len() as f32 * self.line_height
    }

    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    fn max_scroll(&self, measure: &dyn Fn(&str) -> f32) -> f32 {
        (self.content_height(measure) - self.viewport).max(0.0)
    }

    /// Scrolls to `y`, kept between the top and the last full screen of text.
    pub fn set_scroll(&mut self, y: f32, measure: &dyn Fn(&str) -> f32) {
        self.scroll_y = y.clamp(0.0, self.max_scroll(measure));
    }

    /// The range of visual lines that fall in the viewport (partly or fully).
    pub fn visible_lines(&self, measure: &dyn Fn(&str) -> f32) -> Range<usize> {
        let n = self.lines(measure).len();
        let first = (self.scroll_y / self.line_height).floor() as usize;
        let last = ((self.scroll_y + self.viewport) / self.line_height).ceil() as usize;
        first.min(n)..last.min(n)
    }

    /// The caret's top-left point in content coordinates (before scrolling).
    pub fn caret_point(&self, measure: &dyn Fn(&str) -> f32) -> (f32, f32) {
        let lines = self.lines(measure);
        let caret = self.editor.buf.caret();
        let l = line_of(&lines, caret);
        (x_of(self.text(), &lines[l], caret, measure), l as f32 * self.line_height)
    }

    /// Scrolls the least amount that puts the caret line fully in the viewport.
    pub fn scroll_to_caret(&mut self, measure: &dyn Fn(&str) -> f32) {
        let (_, top) = self.caret_point(measure);
        let bottom = top + self.line_height;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + self.viewport {
            self.scroll_y = bottom - self.viewport;
        }
        self.set_scroll(self.scroll_y, measure);
    }

    /// The line and byte offset under a point given in viewport coordinates.
    pub fn hit_test(&self, x: f32, y: f32, measure: &dyn Fn(&str) -> f32) -> usize {
        let lines = self.lines(measure);
        let row = ((y + self.scroll_y) / self.line_height).floor().max(0.0) as usize;
        let l = row.min(lines.len() - 1);
        byte_at(self.text(), &lines[l], x, measure)
    }

    /// Puts the caret at `byte`, or extends the selection to it from the anchor.
    pub fn place_caret(&mut self, byte: usize, extend: bool) {
        let anchor = if extend { self.editor.buf.anchor() } else { byte };
        self.editor.buf.set_selection(anchor, byte);
        self.goal_x = None;
    }

    /// Moves the caret one visual line up or down, keeping the goal x. False at the first or last line.
    pub fn move_vertical(&mut self, down: bool, extend: bool, measure: &dyn Fn(&str) -> f32) -> bool {
        let lines = self.lines(measure);
        let caret = self.editor.buf.caret();
        let l = line_of(&lines, caret);
        let cached = self.goal_x;
        let x = match cached {
            Some(x) => x,
            None => {
                let x = x_of(self.text(), &lines[l], caret, measure);
                self.goal_x = Some(x);
                x
            }
        };
        let target = if down {
            if l + 1 >= lines.len() {
                return false;
            }
            l + 1
        } else {
            match l.checked_sub(1) {
                Some(t) => t,
                None => return false,
            }
        };
        let byte = byte_at(self.text(), &lines[target], x, measure);
        let anchor = if extend { self.editor.buf.anchor() } else { byte };
        self.editor.buf.set_selection(anchor, byte);
        self.goal_x = Some(x);
        self.scroll_to_caret(measure);
        true
    }

    /// Moves the caret one character left or right, collapsing or extending the selection.
    pub fn move_horizontal(&mut self, right: bool, extend: bool, measure: &dyn Fn(&str) -> f32) {
        self.editor.buf.move_char(right, extend);
        self.goal_x = None;
        self.scroll_to_caret(measure);
    }

    /// Types `s` over the selection. True when the text changed.
    pub fn insert(&mut self, s: &str, measure: &dyn Fn(&str) -> f32) -> bool {
        self.goal_x = None;
        let changed = self.editor.insert_text(s);
        self.scroll_to_caret(measure);
        changed
    }

    /// Deletes the selection, or the character before the caret. True when the text changed.
    pub fn delete_backward(&mut self, measure: &dyn Fn(&str) -> f32) -> bool {
        self.goal_x = None;
        let changed = self.editor.perform(crate::text::EditCommand::DeleteBackward, false);
        self.scroll_to_caret(measure);
        changed
    }

    /// The selection as highlight pieces, one per visual line the selection touches. A wrap's consumed space is not
    /// highlighted, and a newline adds no width.
    pub fn highlights(&self, measure: &dyn Fn(&str) -> f32) -> Vec<Highlight> {
        let sel = self.editor.buf.selection();
        let lines = self.lines(measure);
        let text = self.text();
        let mut out = Vec::new();
        if sel.start == sel.end {
            return out;
        }
        for (l, r) in lines.iter().enumerate() {
            let s = sel.start.max(r.start);
            let e = sel.end.min(r.end);
            if s < e {
                out.push(Highlight { line: l, x0: x_of(text, r, s, measure), x1: x_of(text, r, e, measure) });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monospace: 10 points per character.
    fn m(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn wraps_at_spaces_and_consumes_the_space() {
        // Five characters fit on a 50 point line.
        assert_eq!(layout("aa bb cc dd", 50.0, &m), vec![0..5, 6..11]);
        assert_eq!(layout("aa bb cc dd ee", 50.0, &m), vec![0..5, 6..11, 12..14]);
    }

    #[test]
    fn newlines_and_empty_paragraphs_are_lines() {
        assert_eq!(layout("a\n\nb", 100.0, &m), vec![0..1, 2..2, 3..4]);
        assert_eq!(layout("", 100.0, &m), vec![0..0]);
    }

    #[test]
    fn long_word_splits_by_character() {
        // Three characters fit on a 30 point line.
        assert_eq!(layout("abcdefg", 30.0, &m), vec![0..3, 3..6, 6..7]);
    }

    #[test]
    fn caret_maps_to_line_and_x() {
        let text = "aa bb cc dd";
        let lines = layout(text, 50.0, &m);
        // A caret at the wrap point belongs to the next line.
        assert_eq!(line_of(&lines, 6), 1);
        // A caret before the consumed space stays on the first line, at its end.
        assert_eq!(line_of(&lines, 5), 0);
        assert_eq!(x_of(text, &lines[0], 5, &m), 50.0);
        assert_eq!(x_of(text, &lines[1], 11, &m), 50.0);
    }

    #[test]
    fn hit_test_picks_the_nearest_boundary() {
        let text = "aa bb cc dd";
        let lines = layout(text, 50.0, &m);
        // On the first line x = 26 is nearest to boundary 3 (x 30) rather than 2 (x 20).
        assert_eq!(byte_at(text, &lines[0], 26.0, &m), 3);
        // On the second line x = 20 is exactly byte 8.
        assert_eq!(byte_at(text, &lines[1], 20.0, &m), 8);
    }

    #[test]
    fn vertical_moves_keep_the_goal_column() {
        let mut v = TextView::new("aa bb cc dd", 50.0, 20.0, 40.0);
        assert!(v.move_vertical(true, false, &m));
        assert_eq!(v.editor.buf.caret(), 6);
        assert_eq!(v.caret_point(&m), (0.0, 20.0));
        // Already on the last line: no move.
        assert!(!v.move_vertical(true, false, &m));
        assert!(v.move_vertical(false, false, &m));
        assert_eq!(v.editor.buf.caret(), 0);
        assert!(!v.move_vertical(false, false, &m));
    }

    #[test]
    fn scroll_keeps_the_caret_visible() {
        // Three lines of 20 points in a 40 point viewport: the caret at the end needs a 20 point scroll.
        let mut v = TextView::new("aa bb cc dd ee", 50.0, 20.0, 40.0);
        assert_eq!(v.lines(&m).len(), 3);
        assert_eq!(v.visible_lines(&m), 0..2);
        v.place_caret(14, false);
        v.scroll_to_caret(&m);
        assert_eq!(v.scroll_y(), 20.0);
        assert_eq!(v.visible_lines(&m), 1..3);
        // The scroll range is limited to the content.
        v.set_scroll(500.0, &m);
        assert_eq!(v.scroll_y(), 20.0);
    }

    #[test]
    fn selection_highlights_each_line() {
        let mut v = TextView::new("aa bb cc dd", 50.0, 20.0, 100.0);
        // Select bytes 4..8 ("b cc") across the wrap. Line 0 is "aa bb": 4 to 5 is x 40 to 50. Line 1 is "cc dd": 6 to 8 is x 0 to 20.
        v.place_caret(4, false);
        v.place_caret(8, true);
        let h = v.highlights(&m);
        assert_eq!(h, vec![Highlight { line: 0, x0: 40.0, x1: 50.0 }, Highlight { line: 1, x0: 0.0, x1: 20.0 }]);
    }
}
