//! Selecting text with the pointer and the keyboard: where a press lands in the page, words and paragraphs for a double
//! and a triple click, how a drag grows the selection by those units, and where the caret goes for the arrow keys, Home
//! and End with Shift (extend), Alt or Ctrl (by word) and Cmd (by line or to the ends of the text). Pure functions of the
//! cells and the layout, so the rules are tested without a window.

use crate::layout::{Layout, Measure};
use crate::text::{is_break, Cell};
use crate::view::Geometry;
use std::ops::Range;

/// Presses this close together in time (ms) and place (points) are a double or a triple click.
pub const MULTI_CLICK_MS: f64 = 500.0;
pub const MULTI_CLICK_SLOP_PT: f32 = 4.0;

/// What a press selects: a caret, a word (double click) or a paragraph (triple click).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Char,
    Word,
    Paragraph,
}

/// The unit a click count selects: 2 a word, 3 a paragraph, anything else a caret.
pub fn unit_of(clicks: u8) -> Unit {
    match clicks {
        2 => Unit::Word,
        3 => Unit::Paragraph,
        _ => Unit::Char,
    }
}

/// Counts the presses of one click series: 1, 2, 3 and then 1 again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clicks {
    last: Option<(f64, f32, f32)>,
    count: u8,
}

impl Clicks {
    /// A press at (`x`, `y`) points at time `t` ms; returns how many presses the series has now.
    pub fn press(&mut self, t: f64, x: f32, y: f32) -> u8 {
        let again = self.last.is_some_and(|(t0, x0, y0)| t >= t0 && t - t0 <= MULTI_CLICK_MS && (x - x0).abs() <= MULTI_CLICK_SLOP_PT && (y - y0).abs() <= MULTI_CLICK_SLOP_PT);
        self.count = if again { self.count % 3 + 1 } else { 1 };
        self.last = Some((t, x, y));
        self.count
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
    Break,
}

fn class(ch: char) -> Class {
    if ch == '\n' || is_break(ch) {
        Class::Break
    } else if ch.is_whitespace() {
        Class::Space
    } else if ch.is_alphanumeric() || ch == '_' || ch == '\'' || ch == '\u{2019}' {
        Class::Word
    } else {
        Class::Other
    }
}

/// The word that holds the character at `index`: its run of letters and digits, a run of blanks, or a run of
/// punctuation. A break (the end of a line) holds nothing, so a double click on an empty line selects nothing. Past the
/// end of the text it is the last character's word.
pub fn word_range(cells: &[Cell], index: usize) -> Range<usize> {
    if cells.is_empty() {
        return 0..0;
    }
    let mut at = index.min(cells.len() - 1);
    // a point just past the last character of a line lands on its break: the word before it is meant
    if class(cells[at].ch) == Class::Break && at > 0 && class(cells[at - 1].ch) != Class::Break {
        at -= 1;
    }
    let kind = class(cells[at].ch);
    if kind == Class::Break {
        return at..at;
    }
    let mut start = at;
    while start > 0 && class(cells[start - 1].ch) == kind {
        start -= 1;
    }
    let mut end = at + 1;
    while end < cells.len() && class(cells[end].ch) == kind {
        end += 1;
    }
    start..end
}

/// The paragraph that holds the character at `index`, without its newline.
pub fn paragraph_range(cells: &[Cell], index: usize) -> Range<usize> {
    let index = index.min(cells.len());
    let start = cells[..index].iter().rposition(|c| c.ch == '\n').map_or(0, |i| i + 1);
    let end = cells[index..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| index + i);
    start..end
}

/// The range a press selects: the caret (`caret`), the word or the paragraph under the pointer (`under`).
pub fn unit_range(cells: &[Cell], unit: Unit, caret: usize, under: usize) -> Range<usize> {
    match unit {
        Unit::Char => caret..caret,
        Unit::Word => word_range(cells, under),
        Unit::Paragraph => paragraph_range(cells, under),
    }
}

/// Where the pointer is in the page: the caret position nearest to it and the character under it, from a point in
/// window points. The page scrolls under the pointer by `scroll` and starts at the text column of `geom`.
pub fn locate(geom: &Geometry, scroll: f32, layout: &Layout, cells: &[Cell], measure: &Measure, x: f32, y: f32) -> (usize, usize) {
    let at = x - geom.text_x;
    let down = y - geom.body_top + scroll;
    (layout.index_at(cells, at, down, measure), layout.char_at(cells, at, down, measure))
}

/// The selection (anchor, head) of a drag that began on `origin`: with a caret it runs from there to the pointer, with a
/// word or a paragraph it holds the origin's unit and the unit under the pointer and everything between.
pub fn drag_selection(cells: &[Cell], origin: &Range<usize>, unit: Unit, caret: usize, under: usize) -> (usize, usize) {
    if unit == Unit::Char {
        return (origin.start, caret);
    }
    let now = unit_range(cells, unit, caret, under);
    if now.start < origin.start {
        (origin.end, now.start)
    } else {
        (origin.start, now.end.max(origin.end))
    }
}

/// A key that moves the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// The modifiers that change a move: Shift extends the selection, Alt or Ctrl move by word, Cmd by line (and to the
/// ends of the text for Up and Down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavMods {
    pub shift: bool,
    pub word: bool,
    pub line: bool,
}

/// The evdev key codes of the navigation keys, which a chord names (`KeyInput::chord`).
pub fn nav_of_evdev(key: u32) -> Option<Nav> {
    Some(match key {
        105 => Nav::Left,
        106 => Nav::Right,
        103 => Nav::Up,
        108 => Nav::Down,
        102 => Nav::Home,
        107 => Nav::End,
        _ => return None,
    })
}

/// The navigation key a plain key code stands for (the codes the shell protocol sends).
pub fn nav_of_code(code: u32) -> Option<Nav> {
    Some(match code {
        37 => Nav::Left,
        38 => Nav::Up,
        39 => Nav::Right,
        40 => Nav::Down,
        36 => Nav::Home,
        35 => Nav::End,
        _ => return None,
    })
}

/// Where the caret goes. `head` is the end of the selection that moves, `sel` the selected range: without Shift a
/// plain Left or Right collapses to the near edge of a selection instead of stepping.
pub fn target(nav: Nav, mods: NavMods, cells: &[Cell], layout: &Layout, head: usize, sel: &Range<usize>, measure: &Measure) -> usize {
    let collapse = !mods.shift && !sel.is_empty();
    match nav {
        Nav::Left if mods.line => layout.line_start(head),
        Nav::Left if mods.word => word_left(cells, head),
        Nav::Left if collapse => sel.start,
        Nav::Left => head.saturating_sub(1),
        Nav::Right if mods.line => layout.line_end(cells, head),
        Nav::Right if mods.word => word_right(cells, head),
        Nav::Right if collapse => sel.end,
        Nav::Right => (head + 1).min(cells.len()),
        Nav::Up if mods.line => 0,
        Nav::Down if mods.line => cells.len(),
        // the first line has nothing above it: Up goes to the start of the text, and Down on the last line to its end
        Nav::Up => {
            let to = layout.move_vertical(cells, head, false, measure);
            if to == head && layout.line_of(head) == 0 {
                0
            } else {
                to
            }
        }
        Nav::Down => {
            let to = layout.move_vertical(cells, head, true, measure);
            if to == head && layout.line_of(head) + 1 >= layout.lines.len() {
                cells.len()
            } else {
                to
            }
        }
        Nav::Home if mods.word || mods.line => 0,
        Nav::Home => layout.line_start(head),
        Nav::End if mods.word || mods.line => cells.len(),
        Nav::End => layout.line_end(cells, head),
    }
}

/// The start of the word before `from`: the blanks and breaks before the caret are skipped, then the run of one kind.
pub fn word_left(cells: &[Cell], from: usize) -> usize {
    let mut at = from.min(cells.len());
    while at > 0 && matches!(class(cells[at - 1].ch), Class::Space | Class::Break) {
        at -= 1;
    }
    if at == 0 {
        return 0;
    }
    let kind = class(cells[at - 1].ch);
    while at > 0 && class(cells[at - 1].ch) == kind {
        at -= 1;
    }
    at
}

/// The end of the word after `from`: the blanks and breaks after the caret are skipped, then the run of one kind.
pub fn word_right(cells: &[Cell], from: usize) -> usize {
    let mut at = from.min(cells.len());
    while at < cells.len() && matches!(class(cells[at].ch), Class::Space | Class::Break) {
        at += 1;
    }
    if at >= cells.len() {
        return cells.len();
    }
    let kind = class(cells[at].ch);
    while at < cells.len() && class(cells[at].ch) == kind {
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout;
    use crate::text::Style;

    fn cells(s: &str) -> Vec<Cell> {
        s.chars().map(|ch| Cell { ch, style: Style::default() }).collect()
    }

    fn one(_: char, _: &Style) -> f32 {
        1.0
    }

    #[test]
    fn presses_in_one_place_count_up_to_three_and_start_over() {
        let mut c = Clicks::default();
        assert_eq!(c.press(0.0, 10.0, 10.0), 1);
        assert_eq!(c.press(200.0, 11.0, 10.0), 2);
        assert_eq!(c.press(400.0, 10.0, 12.0), 3);
        assert_eq!(c.press(600.0, 10.0, 10.0), 1, "a fourth press begins a new series");
        assert_eq!(unit_of(1), Unit::Char);
        assert_eq!(unit_of(2), Unit::Word);
        assert_eq!(unit_of(3), Unit::Paragraph);
    }

    #[test]
    fn a_late_or_distant_press_is_a_new_click() {
        let mut c = Clicks::default();
        c.press(0.0, 10.0, 10.0);
        assert_eq!(c.press(900.0, 10.0, 10.0), 1, "too slow");
        c.press(1000.0, 10.0, 10.0);
        assert_eq!(c.press(1100.0, 40.0, 10.0), 1, "too far");
        assert_eq!(c.press(1050.0, 40.0, 10.0), 1, "time going backwards is not a double click");
    }

    #[test]
    fn a_word_is_a_run_of_letters_blanks_or_punctuation() {
        let c = cells("Hello, big  world");
        assert_eq!(word_range(&c, 0), 0..5);
        assert_eq!(word_range(&c, 4), 0..5);
        assert_eq!(word_range(&c, 5), 5..6, "the comma");
        assert_eq!(word_range(&c, 6), 6..7, "one blank");
        assert_eq!(word_range(&c, 10), 10..12, "a run of blanks");
        assert_eq!(word_range(&c, 14), 12..17);
        assert_eq!(word_range(&c, 99), 12..17, "past the end is the last word");
        assert_eq!(word_range(&[], 0), 0..0);
        assert_eq!(word_range(&cells("don't"), 2), 0..5, "an apostrophe stays in the word");
    }

    #[test]
    fn a_word_never_takes_a_line_break_and_an_empty_line_holds_nothing() {
        let c = cells("ab\n\ncd");
        assert_eq!(word_range(&c, 2), 0..2, "a point on the newline after a word means that word");
        assert_eq!(word_range(&c, 3), 3..3, "an empty line");
        assert_eq!(word_range(&c, 4), 4..6);
    }

    #[test]
    fn a_paragraph_stops_at_its_newlines() {
        let c = cells("one two\nthree\n\nfour");
        assert_eq!(paragraph_range(&c, 2), 0..7);
        assert_eq!(paragraph_range(&c, 7), 0..7, "a point on the newline is the paragraph it ends");
        assert_eq!(paragraph_range(&c, 8), 8..13);
        assert_eq!(paragraph_range(&c, 14), 14..14, "an empty paragraph");
        assert_eq!(paragraph_range(&c, 99), 15..19);
    }

    #[test]
    fn a_press_selects_by_its_unit() {
        let c = cells("hello world");
        assert_eq!(unit_range(&c, Unit::Char, 3, 2), 3..3);
        assert_eq!(unit_range(&c, Unit::Word, 6, 6), 6..11);
        assert_eq!(unit_range(&c, Unit::Paragraph, 6, 6), 0..11);
    }

    #[test]
    fn a_drag_grows_by_the_unit_in_either_direction() {
        let c = cells("aa bb cc dd");
        // dragging from a caret runs from the press to the pointer
        assert_eq!(drag_selection(&c, &(4..4), Unit::Char, 9, 8), (4, 9));
        assert_eq!(drag_selection(&c, &(4..4), Unit::Char, 1, 1), (4, 1));
        // dragging from the word "bb" (3..5) to a point in "dd" takes whole words to its end
        assert_eq!(drag_selection(&c, &(3..5), Unit::Word, 10, 9), (3, 11));
        // and back to a point in "aa" runs from the end of "bb" to the start of "aa"
        assert_eq!(drag_selection(&c, &(3..5), Unit::Word, 1, 1), (5, 0));
        // staying on the word keeps it selected
        assert_eq!(drag_selection(&c, &(3..5), Unit::Word, 4, 4), (3, 5));
    }

    #[test]
    fn the_page_is_hit_from_window_points_and_the_scroll() {
        let geom = crate::view::geometry(800.0, 600.0, false, false);
        let c = cells("hello world");
        let l = layout(&c, geom.text_w, &one);
        // the point at the left edge of the column on the first line is before the first character
        let (index, under) = locate(&geom, 0.0, &l, &c, &one, geom.text_x + 2.2, geom.body_top + 3.0);
        assert_eq!((index, under), (2, 2));
        // the right half of a character: the caret goes after it, the character is still the one under the pointer
        let (index, under) = locate(&geom, 0.0, &l, &c, &one, geom.text_x + 2.8, geom.body_top + 3.0);
        assert_eq!((index, under), (3, 2));
        // the page scrolled by a line's height: the same window point is on the second line when there is one
        let two = cells("hello\nworld");
        let l2 = layout(&two, geom.text_w, &one);
        let height = l2.lines[0].height;
        let (_, under) = locate(&geom, height, &l2, &two, &one, geom.text_x + 0.5, geom.body_top + 1.0);
        assert_eq!(under, 6, "scrolling by one line puts the second line under the first row");
        // a point above or left of the page clamps to its start
        let (index, _) = locate(&geom, 0.0, &l, &c, &one, 0.0, geom.body_top + 1.0);
        assert_eq!(index, 0);
    }

    #[test]
    fn words_move_the_caret_by_word_both_ways() {
        let c = cells("one  two, three");
        assert_eq!(word_right(&c, 0), 3);
        assert_eq!(word_right(&c, 3), 8, "blanks are skipped, then the word");
        assert_eq!(word_right(&c, 8), 9, "punctuation is a word of its own");
        assert_eq!(word_right(&c, 15), 15);
        assert_eq!(word_left(&c, 15), 10);
        assert_eq!(word_left(&c, 9), 8);
        assert_eq!(word_left(&c, 10), 8, "the blank is skipped and the comma is a word of its own");
        assert_eq!(word_left(&c, 5), 0, "blanks are skipped, then the word");
        assert_eq!(word_left(&c, 0), 0);
        let lines = cells("ab\ncd");
        assert_eq!(word_right(&lines, 2), 5, "a break is skipped like a blank");
        assert_eq!(word_left(&lines, 3), 0);
    }

    fn nav(c: &[Cell], width: f32, key: Nav, mods: NavMods, head: usize, sel: Range<usize>) -> usize {
        let l = layout(c, width, &one);
        target(key, mods, c, &l, head, &sel, &one)
    }

    #[test]
    fn plain_arrows_step_and_collapse_a_selection_to_its_edge() {
        let c = cells("hello world");
        let none = NavMods::default();
        assert_eq!(nav(&c, 100.0, Nav::Right, none, 3, 3..3), 4);
        assert_eq!(nav(&c, 100.0, Nav::Left, none, 0, 0..0), 0);
        assert_eq!(nav(&c, 100.0, Nav::Right, none, 11, 11..11), 11);
        assert_eq!(nav(&c, 100.0, Nav::Left, none, 8, 2..8), 2, "Left collapses to the start of the selection");
        assert_eq!(nav(&c, 100.0, Nav::Right, none, 2, 2..8), 8, "Right collapses to its end");
        // with Shift it steps the moving end instead
        let shift = NavMods { shift: true, ..none };
        assert_eq!(nav(&c, 100.0, Nav::Left, shift, 8, 2..8), 7);
        assert_eq!(nav(&c, 100.0, Nav::Right, shift, 8, 2..8), 9);
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_line_and_with_a_modifier_of_the_text() {
        let c = cells("aaa bbb ccc");
        let none = NavMods::default();
        // wrapped to four points: "aaa ", "bbb ", "ccc"
        assert_eq!(nav(&c, 4.0, Nav::Home, none, 6, 6..6), 4);
        assert_eq!(nav(&c, 4.0, Nav::End, none, 6, 6..6), 7);
        assert_eq!(nav(&c, 4.0, Nav::Home, NavMods { line: true, ..none }, 6, 6..6), 0);
        assert_eq!(nav(&c, 4.0, Nav::End, NavMods { word: true, ..none }, 6, 6..6), 11);
        assert_eq!(nav(&c, 4.0, Nav::Left, NavMods { line: true, ..none }, 6, 6..6), 4, "Cmd+Left is the line start");
        assert_eq!(nav(&c, 4.0, Nav::Right, NavMods { line: true, ..none }, 5, 5..5), 7);
    }

    #[test]
    fn shift_home_selects_back_to_the_line_start() {
        // the case of the report: a caret at the end of a line, Shift+Home selects the whole line
        let c = cells("Hello from the run.");
        let shift = NavMods { shift: true, ..NavMods::default() };
        let head = c.len();
        let to = nav(&c, 100.0, Nav::Home, shift, head, head..head);
        assert_eq!(to, 0);
        // the anchor stays where it was, so the selection is 0..len
        let (anchor, head) = (head, to);
        assert_eq!((anchor.min(head), anchor.max(head)), (0, 19));
    }

    #[test]
    fn up_and_down_move_a_line_and_at_the_first_and_last_line_go_to_the_ends() {
        let c = cells("aaa bbb ccc");
        let none = NavMods::default();
        assert_eq!(nav(&c, 4.0, Nav::Down, none, 1, 1..1), 5);
        assert_eq!(nav(&c, 4.0, Nav::Up, none, 5, 5..5), 1);
        assert_eq!(nav(&c, 4.0, Nav::Up, none, 2, 2..2), 0, "no line above: the start of the text");
        assert_eq!(nav(&c, 4.0, Nav::Down, none, 9, 9..9), 11, "no line below: the end of the text");
        assert_eq!(nav(&c, 4.0, Nav::Up, NavMods { line: true, ..none }, 9, 9..9), 0);
        assert_eq!(nav(&c, 4.0, Nav::Down, NavMods { line: true, ..none }, 1, 1..1), 11);
    }

    #[test]
    fn the_keys_of_both_protocols_name_the_same_moves() {
        for (evdev, code, key) in [(105, 37, Nav::Left), (106, 39, Nav::Right), (103, 38, Nav::Up), (108, 40, Nav::Down), (102, 36, Nav::Home), (107, 35, Nav::End)] {
            assert_eq!(nav_of_evdev(evdev), Some(key));
            assert_eq!(nav_of_code(code), Some(key));
        }
        assert_eq!(nav_of_evdev(30), None);
        assert_eq!(nav_of_code(8), None);
    }
}
