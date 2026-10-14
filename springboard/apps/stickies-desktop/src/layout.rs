//! Line layout of a note body: the wrapped lines as byte spans, the caret's line and column, and the byte offset
//! under a point. The width of text comes in as a closure over byte ranges, so the layout is tested without a font and
//! a run of text can be measured with the style its characters have.

/// One wrapped line of the body: the byte range `start..end`, and whether it ends its paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub last: bool,
}

/// Width of the text between two byte offsets of the body, in the unit of `max` (pixels).
pub type Measure<'a> = &'a dyn Fn(usize, usize) -> f32;

/// Wraps `text` to `max`. A paragraph is a run between line breaks; words wrap greedily, the spaces after a word stay
/// on its line, and a word wider than `max` breaks between characters. An empty paragraph is an empty line, so the
/// result always has at least one line.
pub fn lines(text: &str, max: f32, measure: Measure) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        wrap_paragraph(text, start, end, max, measure, &mut out);
        if end >= text.len() {
            break;
        }
        start = end + 1;
    }
    out
}

fn wrap_paragraph(text: &str, start: usize, end: usize, max: f32, measure: Measure, out: &mut Vec<Line>) {
    let mut line_start = start;
    let mut line_end = start;
    let mut i = start;
    while i < end {
        let word_start = i;
        let mut j = i;
        for c in text[i..end].chars() {
            if c == ' ' || c == '\t' {
                break;
            }
            j += c.len_utf8();
        }
        let word_end = j;
        for c in text[j..end].chars() {
            if c != ' ' && c != '\t' {
                break;
            }
            j += c.len_utf8();
        }
        i = j;
        if line_end > line_start && measure(line_start, word_end) > max {
            out.push(Line { start: line_start, end: line_end, last: false });
            line_start = word_start;
        }
        // a word wider than the line breaks between characters
        while measure(line_start, word_end) > max {
            let mut cut = line_start;
            for (off, c) in text[line_start..word_end].char_indices() {
                let next = line_start + off + c.len_utf8();
                if next >= word_end || measure(line_start, next) > max {
                    break;
                }
                cut = next;
            }
            if cut == line_start {
                // not even one character fits: take one, so the loop ends
                cut = line_start + text[line_start..].chars().next().map_or(1, char::len_utf8);
                if cut >= word_end {
                    break;
                }
            }
            out.push(Line { start: line_start, end: cut, last: false });
            line_start = cut;
        }
        line_end = i;
    }
    out.push(Line { start: line_start, end: end.max(line_start), last: true });
}

/// The index of the line the caret at byte `caret` sits on. At a wrap point the caret belongs to the line it starts.
pub fn line_of(lines: &[Line], caret: usize) -> usize {
    for (i, l) in lines.iter().enumerate() {
        if caret < l.end || (caret == l.end && l.last) {
            return i;
        }
    }
    lines.len().saturating_sub(1)
}

/// The distance from the line's left edge to byte `caret` (clamped into the line).
pub fn x_of(line: &Line, caret: usize, measure: Measure) -> f32 {
    let at = caret.clamp(line.start, line.end);
    measure(line.start, at)
}

/// The byte offset in `line` whose left edge is nearest to `x`.
pub fn index_at(text: &str, line: &Line, x: f32, measure: Measure) -> usize {
    let mut best = line.start;
    let mut best_d = x.abs();
    for (off, c) in text[line.start..line.end].char_indices() {
        let at = line.start + off + c.len_utf8();
        let d = (measure(line.start, at) - x).abs();
        if d < best_d {
            best = at;
            best_d = d;
        }
    }
    best
}

/// The caret after moving one line up or down, keeping the column. The first line goes to the start of the text and
/// the last line to its end.
pub fn move_vertical(text: &str, lines: &[Line], caret: usize, down: bool, measure: Measure) -> usize {
    let at = line_of(lines, caret);
    let target = if down { at + 1 } else { at.wrapping_sub(1) };
    match lines.get(target) {
        Some(line) => index_at(text, line, x_of(&lines[at], caret, measure), measure),
        None if down => text.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One unit per character of the text.
    fn units(text: &str) -> impl Fn(usize, usize) -> f32 + '_ {
        move |a, b| text[a..b].chars().count() as f32
    }

    fn spans<'a>(text: &'a str, ls: &[Line]) -> Vec<&'a str> {
        ls.iter().map(|l| &text[l.start..l.end]).collect()
    }

    #[test]
    fn words_wrap_greedily_and_keep_their_trailing_spaces() {
        let text = "hello world foo";
        let ls = lines(text, 11.0, &units(text));
        assert_eq!(spans(text, &ls), vec!["hello world ", "foo"]);
        assert_eq!(ls.iter().map(|l| l.last).collect::<Vec<_>>(), vec![false, true]);
    }

    #[test]
    fn paragraphs_and_empty_lines_are_kept() {
        let text = "ab\n\ncd\n";
        let ls = lines(text, 10.0, &units(text));
        assert_eq!(spans(text, &ls), vec!["ab", "", "cd", ""]);
        assert!(ls.iter().all(|l| l.last));
        assert_eq!(lines("", 10.0, &units("")), vec![Line { start: 0, end: 0, last: true }]);
    }

    #[test]
    fn a_word_wider_than_the_line_breaks_between_characters() {
        let text = "abcdefghij";
        let ls = lines(text, 4.0, &units(text));
        assert_eq!(spans(text, &ls), vec!["abcd", "efgh", "ij"]);
        // a line narrower than one character still ends
        let narrow = lines("abc", 0.5, &units("abc"));
        assert_eq!(spans("abc", &narrow), vec!["a", "b", "c"]);
    }

    #[test]
    fn wrapping_keeps_multibyte_text_on_character_boundaries() {
        let text = "héllo wörld";
        let ls = lines(text, 6.0, &units(text));
        assert_eq!(spans(text, &ls), vec!["héllo ", "wörld"]);
        assert_eq!(ls.last().map(|l| l.end), Some(text.len()));
    }

    #[test]
    fn the_caret_at_a_wrap_point_belongs_to_the_next_line() {
        let text = "hello world foo";
        let ls = lines(text, 11.0, &units(text));
        assert_eq!(line_of(&ls, 0), 0);
        assert_eq!(line_of(&ls, 11), 0);
        assert_eq!(line_of(&ls, 12), 1);
        assert_eq!(line_of(&ls, text.len()), 1);
        let multi = lines("ab\ncd", 10.0, &units("ab\ncd"));
        assert_eq!(line_of(&multi, 2), 0, "the end of a paragraph is on its own line");
        assert_eq!(line_of(&multi, 3), 1);
    }

    #[test]
    fn x_and_index_are_inverse_to_each_other() {
        let text = "abcdef";
        let ls = lines(text, 10.0, &units(text));
        let line = ls[0];
        assert_eq!(x_of(&line, 3, &units(text)), 3.0);
        assert_eq!(index_at(text, &line, 3.0, &units(text)), 3);
        assert_eq!(index_at(text, &line, 3.4, &units(text)), 3);
        assert_eq!(index_at(text, &line, 3.6, &units(text)), 4);
        assert_eq!(index_at(text, &line, -5.0, &units(text)), 0);
        assert_eq!(index_at(text, &line, 99.0, &units(text)), 6);
    }

    #[test]
    fn a_measure_can_weigh_each_part_of_the_line_differently() {
        // the first two characters count double, as bold characters would
        let text = "abcd";
        let wide = |a: usize, b: usize| -> f32 {
            (a..b).map(|i| if i < 2 { 2.0 } else { 1.0 }).sum()
        };
        assert_eq!(wide(0, 2), 4.0);
        let ls = lines(text, 10.0, &wide);
        assert_eq!(x_of(&ls[0], 2, &wide), 4.0);
        assert_eq!(index_at(text, &ls[0], 5.0, &wide), 3);
    }

    #[test]
    fn vertical_movement_keeps_the_column_and_stops_at_the_text_edges() {
        let text = "abcdef\nxy\nghijkl";
        let ls = lines(text, 20.0, &units(text));
        // from column 4 of the first line down to the short line: its end
        assert_eq!(move_vertical(text, &ls, 4, true, &units(text)), 9);
        // from column 2 of the short line down to the third line: column 2 there
        assert_eq!(move_vertical(text, &ls, 9, true, &units(text)), 12);
        assert_eq!(move_vertical(text, &ls, 13, false, &units(text)), 9);
        assert_eq!(move_vertical(text, &ls, 3, false, &units(text)), 0, "up from the first line");
        assert_eq!(move_vertical(text, &ls, 14, true, &units(text)), text.len(), "down from the last line");
    }
}
