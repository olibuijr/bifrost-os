//! Line layout of a document in points: word wrap at the text width, paragraph alignment, line spacing, list indents,
//! tab stops, and the mapping between a text position and a point (the caret, a click). Pure: the widths come from a
//! `Measure` the caller supplies, so the rules are tested with fixed widths and the window draws with the real font.

use crate::text::{is_break, Align, Cell, ListKind, Style, DEFAULT_SIZE, DEFAULT_TAB, SPACINGS};

/// Line height as a multiple of the largest size on the line.
pub const LINE_FACTOR: f32 = 1.25;
/// How far a list paragraph is indented, where its marker hangs.
pub const LIST_INDENT: f32 = 24.0;

/// The width of one character in a style, in points.
pub type Measure<'a> = dyn Fn(char, &Style) -> f32 + 'a;

/// The marker drawn in front of a paragraph's first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    None,
    Bullet,
    /// The paragraph's number in its run of numbered paragraphs, from 1.
    Number(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The first cell of the line, and the cell after its last one. A line never includes its paragraph's newline,
    /// except that a wrapped line ends where the next one starts.
    pub start: usize,
    pub end: usize,
    /// Top edge and height, in points, from the top of the text.
    pub top: f32,
    pub height: f32,
    /// The width of the cells of the line, in points (tabs included).
    pub width: f32,
    pub align: Align,
    /// Whether the line's paragraph is right to left: a left-aligned right to left line starts at the right edge.
    pub rtl: bool,
    /// How far the line's text is indented (a list), and the marker hanging in that indent.
    pub indent: f32,
    pub marker: Marker,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// The width the text was wrapped to.
    pub width: f32,
    /// The height of all the lines.
    pub height: f32,
    /// The distance between tab stops.
    pub tab: f32,
}

/// What a paragraph asks of its lines.
#[derive(Clone, Copy)]
struct Para {
    align: Align,
    rtl: bool,
    indent: f32,
    marker: Marker,
    spacing: f32,
    /// The space above the paragraph's first line and below its last, in points.
    before: f32,
    after: f32,
}

/// How far a tab or character at `x` (from the start of the line's text) advances.
pub fn advance(x: f32, cell: &Cell, tab: f32, measure: &Measure) -> f32 {
    if is_break(cell.ch) {
        0.0
    } else if cell.ch == '\t' && tab > 0.0 {
        ((x / tab).floor() + 1.0) * tab - x
    } else {
        measure(cell.ch, &cell.style)
    }
}

/// The width of the cells `a..b` of a line that starts at `a`.
fn span(cells: &[Cell], a: usize, b: usize, tab: f32, measure: &Measure) -> f32 {
    let mut x = 0.0;
    for cell in &cells[a..b] {
        x += advance(x, cell, tab, measure);
    }
    x
}

/// Wraps `cells` to `width` points with the default tab stops.
#[cfg_attr(not(test), allow(dead_code))]
pub fn layout(cells: &[Cell], width: f32, measure: &Measure) -> Layout {
    layout_tabs(cells, width, measure, DEFAULT_TAB)
}

/// Wraps `cells` to `width` points. Every paragraph has at least one line, so an empty document has one.
pub fn layout_tabs(cells: &[Cell], width: f32, measure: &Measure, tab: f32) -> Layout {
    let mut lines = Vec::new();
    let mut top = 0.0;
    let mut start = 0;
    let mut number = 0;
    loop {
        let end = cells[start..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| start + i);
        let style = cells.get(start).or(cells.last()).map(|c| c.style).unwrap_or_default();
        number = if style.list == ListKind::Numbered { number + 1 } else { 0 };
        let para = Para {
            align: style.align,
            rtl: style.rtl,
            indent: if style.list == ListKind::None { 0.0 } else { LIST_INDENT },
            marker: match style.list {
                ListKind::None => Marker::None,
                ListKind::Bullet => Marker::Bullet,
                ListKind::Numbered => Marker::Number(number),
            },
            spacing: SPACINGS.get(style.spacing as usize).copied().unwrap_or(1.0),
            before: style.space_before as f32,
            after: style.space_after as f32,
        };
        top += para.before;
        wrap(cells, start, end, width, measure, tab, para, &mut top, &mut lines);
        top += para.after;
        if end >= cells.len() {
            break;
        }
        start = end + 1;
    }
    Layout { lines, width, height: top, tab }
}

/// Breaks one paragraph (the cells `start..end`, not its newline) into lines. A break comes after the last space that
/// fits; a word longer than the width is broken where it does not fit.
#[allow(clippy::too_many_arguments)]
fn wrap(cells: &[Cell], start: usize, end: usize, width: f32, measure: &Measure, tab: f32, para: Para, top: &mut f32, out: &mut Vec<Line>) {
    let avail = (width - para.indent).max(1.0);
    if start == end {
        push_line(out, cells, start, end, 0.0, para, para.marker, top);
        return;
    }
    let mut line_start = start;
    let mut used = 0.0;
    let mut marker = para.marker;
    // the cell after the last space on this line, and the width up to it
    let mut space: Option<(usize, f32)> = None;
    let mut i = start;
    while i < end {
        let w = advance(used, &cells[i], tab, measure);
        if used + w > avail && i > line_start {
            let (at, at_width) = match space {
                Some((after, w)) if after > line_start => (after, w),
                _ => (i, used),
            };
            push_line(out, cells, line_start, at, at_width, para, marker, top);
            marker = Marker::None;
            line_start = at;
            used = span(cells, line_start, i, tab, measure);
            space = None;
            continue;
        }
        used += w;
        if is_break(cells[i].ch) {
            // a line or page break ends the line after it
            push_line(out, cells, line_start, i + 1, used, para, marker, top);
            marker = Marker::None;
            line_start = i + 1;
            used = 0.0;
            space = None;
            i += 1;
            continue;
        }
        if cells[i].ch == ' ' {
            space = Some((i + 1, used));
        }
        i += 1;
    }
    push_line(out, cells, line_start, end, used, para, marker, top);
}

#[allow(clippy::too_many_arguments)]
fn push_line(out: &mut Vec<Line>, cells: &[Cell], start: usize, end: usize, width: f32, para: Para, marker: Marker, top: &mut f32) {
    let height = line_height(cells, start, end) * para.spacing;
    out.push(Line { start, end, top: *top, height, width, align: para.align, rtl: para.rtl, indent: para.indent, marker });
    *top += height;
}

/// The height of a line: the largest size on it. An empty line takes the size of the newline that ends it.
fn line_height(cells: &[Cell], start: usize, end: usize) -> f32 {
    let size = if start < end {
        cells[start..end].iter().map(|c| c.style.size).fold(0.0, f32::max)
    } else {
        cells.get(end).map_or(DEFAULT_SIZE, |c| c.style.size)
    };
    size * LINE_FACTOR
}

impl Layout {
    /// The line that holds the text position `index`: the last line that starts at or before it.
    pub fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|l| l.start <= index).unwrap_or(0)
    }

    /// Where a line's text starts across the width: its indent and, for its alignment, the free space.
    pub fn x_offset(&self, line: &Line) -> f32 {
        let free = (self.width - line.indent - line.width).max(0.0);
        // in a right to left paragraph the start of a line is its right edge
        let align = match (line.align, line.rtl) {
            (Align::Left, true) => Align::Right,
            (align, _) => align,
        };
        line.indent
            + match align {
                Align::Center => free / 2.0,
                Align::Right => free,
                Align::Left | Align::Justified => 0.0,
            }
    }

    /// The caret for the text position `index`: x, top and height in points.
    pub fn caret(&self, cells: &[Cell], index: usize, measure: &Measure) -> (f32, f32, f32) {
        let Some(line) = self.lines.get(self.line_of(index)) else {
            return (0.0, 0.0, DEFAULT_SIZE * LINE_FACTOR);
        };
        (self.x_in_line(cells, line, index, measure), line.top, line.height)
    }

    /// The x, in points from the left of the text column, of position `index` on `line`.
    pub fn x_in_line(&self, cells: &[Cell], line: &Line, index: usize, measure: &Measure) -> f32 {
        self.x_offset(line) + span(cells, line.start, index.clamp(line.start, line.end), self.tab, measure)
    }

    /// The text position nearest to the point (`x`, `y`) in points: a click places the caret.
    pub fn index_at(&self, cells: &[Cell], x: f32, y: f32, measure: &Measure) -> usize {
        let Some(line) = self.lines.iter().position(|l| y < l.top + l.height).or(self.lines.len().checked_sub(1)) else {
            return 0;
        };
        self.index_in_line(cells, line, x, measure)
    }

    /// The text position in line `line` nearest to `x`. The caret moves to the nearer side of each character.
    fn index_in_line(&self, cells: &[Cell], line: usize, x: f32, measure: &Measure) -> usize {
        let Some(line) = self.lines.get(line) else { return 0 };
        let mut rel = 0.0;
        let origin = self.x_offset(line);
        for k in line.start..line.end {
            let w = advance(rel, &cells[k], self.tab, measure);
            if x < origin + rel + w / 2.0 {
                return k;
            }
            rel += w;
        }
        // past the end of a line that a break ends, the caret goes before the break, not onto the next line
        if line.end > line.start && is_break(cells[line.end - 1].ch) {
            return line.end - 1;
        }
        line.end
    }

    /// The position one line up or down from `index`, at the same x. `index` itself when there is no line there.
    pub fn move_vertical(&self, cells: &[Cell], index: usize, down: bool, measure: &Measure) -> usize {
        let current = self.line_of(index);
        let target = if down { current + 1 } else { current.wrapping_sub(1) };
        if target >= self.lines.len() {
            return index;
        }
        let (x, _, _) = self.caret(cells, index, measure);
        self.index_in_line(cells, target, x, measure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is one point wide.
    fn one(_: char, _: &Style) -> f32 {
        1.0
    }

    fn cells(s: &str) -> Vec<Cell> {
        s.chars().map(|ch| Cell { ch, style: Style::default() }).collect()
    }

    #[test]
    fn a_right_to_left_paragraph_starts_its_lines_at_the_right_edge() {
        let mut c = cells("ab");
        for cell in &mut c {
            cell.style.rtl = true;
        }
        let l = layout(&c, 10.0, &one);
        assert_eq!(l.x_offset(&l.lines[0]), 8.0, "two points of text sit against the right edge of ten");
        let mut centred = cells("ab");
        for cell in &mut centred {
            cell.style = Style { rtl: true, align: Align::Center, ..Style::default() };
        }
        let l = layout(&centred, 10.0, &one);
        assert_eq!(l.x_offset(&l.lines[0]), 4.0, "an explicit centre is kept");
    }

    #[test]
    fn wraps_after_the_last_space_that_fits() {
        let c = cells("aaa bbb ccc");
        let l = layout(&c, 4.0, &one);
        let spans: Vec<(usize, usize)> = l.lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, vec![(0, 4), (4, 8), (8, 11)]);
        assert_eq!(l.lines[0].width, 4.0);
    }

    #[test]
    fn a_line_break_ends_the_line_without_taking_width() {
        let c = cells("ab\u{2028}cd");
        let l = layout(&c, 100.0, &one);
        let spans: Vec<(usize, usize)> = l.lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, vec![(0, 3), (3, 5)]);
        assert_eq!(l.lines[0].width, 2.0, "the break itself is as wide as nothing");
        assert_eq!(l.lines[1].top, l.lines[0].height);
        assert!(l.lines[1].marker == Marker::None);
    }

    #[test]
    fn a_break_at_the_end_of_a_paragraph_leaves_an_empty_line_for_the_caret() {
        let c = cells("ab\u{c}");
        let l = layout(&c, 100.0, &one);
        let spans: Vec<(usize, usize)> = l.lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, vec![(0, 3), (3, 3)]);
        // the caret after the break is on the new line, the one before it at the end of the first
        assert_eq!(l.line_of(3), 1);
        assert_eq!(l.line_of(2), 0);
        assert_eq!(l.caret(&c, 2, &one).0, 2.0);
    }

    #[test]
    fn a_click_past_a_broken_line_lands_before_its_break() {
        let c = cells("ab\u{2028}cd");
        let l = layout(&c, 100.0, &one);
        assert_eq!(l.index_at(&c, 50.0, 1.0, &one), 2);
        assert_eq!(l.index_at(&c, 50.0, l.lines[1].top + 1.0, &one), 5);
        assert_eq!(l.move_vertical(&c, 1, true, &one), 4);
    }

    #[test]
    fn breaks_a_word_longer_than_the_width() {
        let c = cells("abcdefg");
        let l = layout(&c, 3.0, &one);
        let spans: Vec<(usize, usize)> = l.lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, vec![(0, 3), (3, 6), (6, 7)]);
    }

    #[test]
    fn an_empty_document_has_one_line_and_a_trailing_break_adds_one() {
        let empty = layout(&[], 100.0, &one);
        assert_eq!(empty.lines.len(), 1);
        let c = cells("ab\n");
        let l = layout(&c, 100.0, &one);
        assert_eq!(l.lines.len(), 2);
        assert_eq!((l.lines[1].start, l.lines[1].end), (3, 3));
    }

    #[test]
    fn caret_follows_the_text_and_wraps_with_it() {
        let c = cells("aaa bbb ccc");
        let l = layout(&c, 4.0, &one);
        let (x, top, height) = l.caret(&c, 5, &one);
        assert_eq!(x, 1.0, "one character into the second line");
        assert_eq!(top, l.lines[1].top);
        assert_eq!(height, DEFAULT_SIZE * LINE_FACTOR);
        assert_eq!(l.line_of(4), 1, "the first cell of a wrapped line starts that line");
    }

    #[test]
    fn a_click_lands_on_the_nearer_side_of_a_character() {
        let c = cells("aaa bbb ccc");
        let l = layout(&c, 4.0, &one);
        let second_line_top = l.lines[1].top + 1.0;
        assert_eq!(l.index_at(&c, 0.4, second_line_top, &one), 4);
        assert_eq!(l.index_at(&c, 3.9, second_line_top, &one), 8, "past the end of the line");
        assert_eq!(l.index_at(&c, 0.0, 9999.0, &one), 8, "below the text goes to the last line start");
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let c = cells("aaa bbb ccc");
        let l = layout(&c, 4.0, &one);
        assert_eq!(l.move_vertical(&c, 5, true, &one), 9);
        assert_eq!(l.move_vertical(&c, 5, false, &one), 1);
        assert_eq!(l.move_vertical(&c, 1, false, &one), 1, "no line above the first");
    }

    #[test]
    fn centred_lines_are_offset_by_half_the_free_width() {
        let mut c = cells("ab");
        for cell in &mut c {
            cell.style.align = Align::Center;
        }
        let l = layout(&c, 10.0, &one);
        assert_eq!(l.x_offset(&l.lines[0]), 4.0);
        let (x, _, _) = l.caret(&c, 0, &one);
        assert_eq!(x, 4.0);
    }

    #[test]
    fn a_tab_advances_to_the_next_stop() {
        let c = cells("a\tb");
        let l = layout_tabs(&c, 100.0, &one, 10.0);
        assert_eq!(l.lines[0].width, 11.0, "a, then a tab to the stop at 10, then b");
        let (x, _, _) = l.caret(&c, 2, &one);
        assert_eq!(x, 10.0);
        assert_eq!(l.index_at(&c, 10.2, 1.0, &one), 2);
    }

    #[test]
    fn list_paragraphs_are_indented_numbered_and_spaced() {
        let mut c = cells("ab\ncd\nef");
        for cell in &mut c[..5] {
            cell.style.list = ListKind::Numbered;
        }
        for cell in &mut c[6..] {
            cell.style.spacing = 3;
        }
        let l = layout(&c, 100.0, &one);
        assert_eq!(l.lines[0].marker, Marker::Number(1));
        assert_eq!(l.lines[1].marker, Marker::Number(2));
        assert_eq!(l.lines[2].marker, Marker::None);
        assert_eq!(l.x_offset(&l.lines[0]), LIST_INDENT);
        assert_eq!(l.lines[2].height, DEFAULT_SIZE * LINE_FACTOR * 2.0);
    }

    #[test]
    fn a_wrapped_list_paragraph_marks_only_its_first_line() {
        let mut c = cells("aaa bbb");
        for cell in &mut c {
            cell.style.list = ListKind::Bullet;
        }
        let l = layout(&c, LIST_INDENT + 4.0, &one);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].marker, Marker::Bullet);
        assert_eq!(l.lines[1].marker, Marker::None);
    }
}
