//! A multi-line text view model: wrapped text in a scroll region, with a caret, a selection, vertical movement that keeps
//! a goal column in points, hit testing from a point, and scrolling that keeps the caret on screen. Editing goes through
//! `text::TextEditor`, so typing, undo and the key bindings are the ones the single-line field already uses.
//!
//! Layout is greedy word wrap against a caller-supplied `measure` (points for a string), the typesetter's rule (`typeset::Typesetter::suggest_line_break`),
//! but each line is kept as a byte range of the text so that carets and hit tests map back to offsets. A line excludes its
//! newline and the space that a wrap consumed. Positions are UTF-8 byte offsets, as in `text::TextBuffer`.
//!
//! Layout is recomputed on each query and nothing is cached, so a change of text or width needs no invalidation. The
//! model has no drawing: a caller draws `visible_lines`, the caret from `caret_point` and the `Highlight`s.

use std::ops::Range;
use std::rc::Rc;

use crate::text::TextEditor;
use crate::textfind::{Options as FindOptions, TextFinder};
use crate::textmetrics::{CharTable, FontMetrics, GlyphSource};
use crate::typeset::{Typesetter, TypesetterOptions};

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
    rich_text: bool,
    uses_find_bar: bool,
    find_bar_visible: bool,
    find_session: TextFinder,
    /// The spelling hook (`SpellChecker`): the misspelled byte ranges of a text.
    spell_check: Option<SpellHook>,
}

/// A spelling check: the misspelled byte ranges of the text it is given.
pub type SpellHook = Box<dyn Fn(&str) -> Vec<Range<usize>>>;

/// Word wrap of the whole text with the typesetter (line break opportunities, hanging spaces, long words split between
/// characters). Returns one byte range per visual line, in order. An empty paragraph is an empty line. A line excludes
/// its newline and, when it was wrapped, its trailing spaces.
pub fn layout(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<Range<usize>> {
    // One font, measured by the caller: the metrics only need to be positive.
    let metrics = FontMetrics { ascent: 1.0, descent: 0.0, leading: 0.0, underline_position: 0.0, underline_thickness: 0.0 };
    let source: Rc<dyn GlyphSource> = Rc::new(CharTable::measure(text, measure, metrics));
    let ts = Typesetter::new(text, &[], &[], TypesetterOptions::default(), source);
    // Byte offset of every UTF-16 offset (the low half of a pair maps to the next boundary).
    let mut byte_at_u16: Vec<usize> = Vec::with_capacity(ts.len() + 1);
    for (b, c) in text.char_indices() {
        byte_at_u16.push(b);
        if c.len_utf16() == 2 {
            byte_at_u16.push(b + c.len_utf8());
        }
    }
    byte_at_u16.push(text.len());
    let mut lines = Vec::new();
    let mut pos = 0usize;
    while pos < ts.len() {
        let n = ts.suggest_line_break(pos, width).max(1);
        let (s, e) = (byte_at_u16[pos], byte_at_u16[(pos + n).min(ts.len())]);
        let line = &text[s..e];
        let end = if let Some(stripped) = line.strip_suffix('\n') {
            s + stripped.len()
        } else if e < text.len() {
            s + line.trim_end_matches(' ').len()
        } else {
            e
        };
        lines.push(s..end);
        pos += n;
    }
    if text.is_empty() || text.ends_with('\n') {
        lines.push(text.len()..text.len());
    }
    lines
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
        TextView { editor: TextEditor::with_text(text, UNDO_LEVELS), width, line_height, viewport, scroll_y: 0.0,
            goal_x: None,
            rich_text: false,
            uses_find_bar: true,
            find_bar_visible: false,
            find_session: TextFinder::new(FindOptions::default()),
            spell_check: None,
        }
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

    /// Types `s` over the selection, through the compose method (an apostrophe and a letter open the accent list).
    /// True when the text changed.
    pub fn insert(&mut self, s: &str, measure: &dyn Fn(&str) -> f32) -> bool {
        self.goal_x = None;
        let changed = self.editor.type_text(s);
        self.scroll_to_caret(measure);
        changed
    }

    /// The accent candidates of an open composition (empty when none is open), for the caller to draw at the caret.
    pub fn candidates(&self) -> &[String] {
        self.editor.candidates()
    }

    /// Deletes the selection, or the character before the caret. True when the text changed.
    pub fn delete_backward(&mut self, measure: &dyn Fn(&str) -> f32) -> bool {
        self.goal_x = None;
        let changed = self.editor.perform(crate::text::EditCommand::DeleteBackward, false);
        self.scroll_to_caret(measure);
        changed
    }

    /// `isRichText`: whether attributes (paragraph styles, lists, attachments) apply. A plain view ignores them.
    pub fn is_rich_text(&self) -> bool {
        self.rich_text
    }

    pub fn set_rich_text(&mut self, on: bool) {
        self.rich_text = on;
    }

    /// `allowsUndo`.
    pub fn allows_undo(&self) -> bool {
        self.editor.allows_undo()
    }

    pub fn set_allows_undo(&mut self, on: bool) {
        self.editor.set_allows_undo(on);
    }

    /// `usesFindBar`: the find bar (on) or the find panel (off).
    pub fn uses_find_bar(&self) -> bool {
        self.uses_find_bar
    }

    pub fn set_uses_find_bar(&mut self, on: bool) {
        self.uses_find_bar = on;
        if !on {
            self.find_bar_visible = false;
        }
    }

    pub fn find_bar_visible(&self) -> bool {
        self.find_bar_visible
    }

    /// Shows the find bar (`performFindPanelAction:` with show). False when the view does not use the find bar.
    pub fn show_find_bar(&mut self) -> bool {
        if !self.uses_find_bar {
            return false;
        }
        self.find_bar_visible = true;
        true
    }

    pub fn hide_find_bar(&mut self) {
        self.find_bar_visible = false;
    }

    /// The find bar's model: query, matches, counter.
    pub fn find_session(&self) -> &TextFinder {
        &self.find_session
    }

    pub fn set_find_options(&mut self, options: FindOptions) {
        let text = self.editor.buf.text().to_string();
        self.find_session.set_options(&text, options);
    }

    /// Incremental search: sets the query and selects the first match at or after the start of the selection.
    pub fn set_find_query(&mut self, query: &str, measure: &dyn Fn(&str) -> f32) -> Option<Range<usize>> {
        let text = self.editor.buf.text().to_string();
        self.find_session.set_query(&text, query);
        let from = self.editor.buf.selection().start;
        let r = self.find_session.find_next(from);
        self.select_found(r, measure)
    }

    /// Next match after the selection, wrapping at the end.
    pub fn find_next(&mut self, measure: &dyn Fn(&str) -> f32) -> Option<Range<usize>> {
        let text = self.editor.buf.text().to_string();
        self.find_session.refresh(&text);
        let from = self.editor.buf.selection().end;
        let r = self.find_session.find_next(from);
        self.select_found(r, measure)
    }

    /// Previous match before the selection, wrapping at the start.
    pub fn find_previous(&mut self, measure: &dyn Fn(&str) -> f32) -> Option<Range<usize>> {
        let text = self.editor.buf.text().to_string();
        self.find_session.refresh(&text);
        let from = self.editor.buf.selection().start;
        let r = self.find_session.find_previous(from);
        self.select_found(r, measure)
    }

    fn select_found(&mut self, r: Option<Range<usize>>, measure: &dyn Fn(&str) -> f32) -> Option<Range<usize>> {
        let r = r?;
        self.editor.buf.set_selection(r.start, r.end);
        self.goal_x = None;
        self.scroll_to_caret(measure);
        Some(r)
    }

    /// Replaces the selected match with `with` (one undo step). False when the selection is not the current match.
    pub fn replace_current(&mut self, with: &str) -> bool {
        let sel = self.editor.buf.selection();
        if self.find_session.selected() != Some(sel.clone()) {
            return false;
        }
        let done = self.editor.replace_range(sel, with, "Replace");
        let text = self.editor.buf.text().to_string();
        self.find_session.refresh(&text);
        done
    }

    /// Replaces every match with `with` (one undo step). Returns how many were replaced.
    pub fn replace_all(&mut self, with: &str) -> usize {
        let text = self.editor.buf.text().to_string();
        self.find_session.refresh(&text);
        let ranges = self.find_session.matches().to_vec();
        let n = self.editor.replace_ranges(&ranges, with, "Replace All");
        let text = self.editor.buf.text().to_string();
        self.find_session.refresh(&text);
        n
    }

    /// Installs the spelling hook (`SpellChecker` integration).
    pub fn set_spell_checker(&mut self, hook: Option<SpellHook>) {
        self.spell_check = hook;
    }

    /// The misspelled ranges of the text, empty without a hook.
    pub fn misspelled_ranges(&self) -> Vec<Range<usize>> {
        self.spell_check.as_ref().map(|h| h(self.editor.buf.text())).unwrap_or_default()
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
        // a new view's caret is at the end of its text; start from the top
        v.editor.buf.set_selection(0, 0);
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
