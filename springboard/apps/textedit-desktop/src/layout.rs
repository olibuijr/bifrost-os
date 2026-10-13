//! Line layout of a document in points: word wrap at the text width, paragraph alignment, and the mapping between a
//! text position and a point (the caret, a click). Pure: the widths come from a `Measure` the caller supplies, so the
//! rules are tested with fixed widths and the window draws with the real font.

use crate::text::{Align, Cell, Style, DEFAULT_SIZE};

/// Line height as a multiple of the largest size on the line.
pub const LINE_FACTOR: f32 = 1.25;

/// The width of one character in a style, in points.
pub type Measure<'a> = dyn Fn(char, &Style) -> f32 + 'a;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The first cell of the line, and the cell after its last one. A line never includes its paragraph's newline,
    /// except that a wrapped line ends where the next one starts.
    pub start: usize,
    pub end: usize,
    /// Top edge and height, in points, from the top of the text.
    pub top: f32,
    pub height: f32,
    /// The width of the cells of the line, in points.
    pub width: f32,
    pub align: Align,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// The width the text was wrapped to.
    pub width: f32,
    /// The height of all the lines.
    pub height: f32,
}

/// Wraps `cells` to `width` points. Every paragraph has at least one line, so an empty document has one.
pub fn layout(cells: &[Cell], width: f32, measure: &Measure) -> Layout {
    let mut lines = Vec::new();
    let mut top = 0.0;
    let mut start = 0;
    loop {
        let end = cells[start..].iter().position(|c| c.ch == '\n').map_or(cells.len(), |i| start + i);
        let align = cells.get(start).or(cells.last()).map_or(Align::Left, |c| c.style.align);
        wrap(cells, start, end, width, measure, align, &mut top, &mut lines);
        if end >= cells.len() {
            break;
        }
        start = end + 1;
    }
    Layout { lines, width, height: top }
}

/// Breaks one paragraph (the cells `start..end`, not its newline) into lines. A break comes after the last space that
/// fits; a word longer than the width is broken where it does not fit.
#[allow(clippy::too_many_arguments)]
fn wrap(
    cells: &[Cell],
    start: usize,
    end: usize,
    width: f32,
    measure: &Measure,
    align: Align,
    top: &mut f32,
    out: &mut Vec<Line>,
) {
    if start == end {
        push_line(out, cells, start, end, 0.0, align, top);
        return;
    }
    let mut line_start = start;
    let mut used = 0.0;
    // the cell after the last space on this line, and the width up to it
    let mut space: Option<(usize, f32)> = None;
    let mut i = start;
    while i < end {
        let w = measure(cells[i].ch, &cells[i].style);
        if used + w > width && i > line_start {
            let (at, at_width) = match space {
                Some((after, w)) if after > line_start => (after, w),
                _ => (i, used),
            };
            push_line(out, cells, line_start, at, at_width, align, top);
            line_start = at;
            used = (line_start..i).map(|k| measure(cells[k].ch, &cells[k].style)).sum();
            space = None;
            continue;
        }
        used += w;
        if cells[i].ch == ' ' {
            space = Some((i + 1, used));
        }
        i += 1;
    }
    push_line(out, cells, line_start, end, used, align, top);
}

fn push_line(out: &mut Vec<Line>, cells: &[Cell], start: usize, end: usize, width: f32, align: Align, top: &mut f32) {
    let height = line_height(cells, start, end);
    out.push(Line { start, end, top: *top, height, width, align });
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

    /// Where a line's text starts across the width, for its alignment.
    pub fn x_offset(&self, line: &Line) -> f32 {
        let free = (self.width - line.width).max(0.0);
        match line.align {
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
        let mut x = self.x_offset(line);
        for k in line.start..index.min(line.end) {
            x += measure(cells[k].ch, &cells[k].style);
        }
        x
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
        let mut px = self.x_offset(line);
        for k in line.start..line.end {
            let w = measure(cells[k].ch, &cells[k].style);
            if x < px + w / 2.0 {
                return k;
            }
            px += w;
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
    fn wraps_after_the_last_space_that_fits() {
        let c = cells("aaa bbb ccc");
        let l = layout(&c, 4.0, &one);
        let spans: Vec<(usize, usize)> = l.lines.iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(spans, vec![(0, 4), (4, 8), (8, 11)]);
        assert_eq!(l.lines[0].width, 4.0);
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
}
