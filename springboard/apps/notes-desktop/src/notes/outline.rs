//! Indentation of list lines: more or less indent (Tab and Shift+Tab) and moving a list item, with the items nested
//! under it, up or down past its neighbour. Pure text logic on lines, tested on the host.

use springboard::shell::md::{self, Block};

/// Spaces added or removed by one level of indentation.
pub const STEP: usize = 2;
/// The deepest indent a list line gets (four levels below the first).
pub const MAX_INDENT: usize = 8;

/// The indent of a list line (bullet, dash, number or checkbox); `None` for any other line.
fn list_indent(line: &str) -> Option<usize> {
    let info = md::classify(line);
    matches!(info.block, Block::Bullet | Block::Dashed | Block::Numbered(_) | Block::Check(_)).then_some(info.indent)
}

/// Gives every list line of `part` one more level; other lines stay as they are. `None` when no line changed
/// (nothing to indent, or all already at the deepest level).
pub fn indent(part: &str) -> Option<String> {
    let mut changed = false;
    let out: Vec<String> = part
        .split('\n')
        .map(|line| match list_indent(line) {
            Some(n) if n + STEP <= MAX_INDENT => {
                changed = true;
                format!("{}{line}", " ".repeat(STEP))
            }
            _ => line.to_string(),
        })
        .collect();
    changed.then(|| out.join("\n"))
}

/// Takes one level off every indented list line of `part`. `None` when no line changed.
pub fn outdent(part: &str) -> Option<String> {
    let mut changed = false;
    let out: Vec<String> = part
        .split('\n')
        .map(|line| match list_indent(line) {
            Some(n) if n > 0 => {
                changed = true;
                line.chars().skip(n.min(STEP)).collect()
            }
            _ => line.to_string(),
        })
        .collect();
    changed.then(|| out.join("\n"))
}

/// The text after moving lines, and how many chars the moved lines' start shifted by (negative when they went up).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub text: String,
    pub shift: isize,
}

/// Moves lines `first..=last` (0-based, inclusive) of `text` up or down past the neighbouring item. A list item
/// takes the items nested under it along, and passes the whole neighbour including its nested items; it never
/// leaves the list it is in. `None` at the top or bottom, or when the move would leave a parent item.
pub fn move_lines(text: &str, first: usize, last: usize, up: bool) -> Option<Moved> {
    let lines: Vec<&str> = text.split('\n').collect();
    let n = lines.len();
    if first > last || last >= n {
        return None;
    }
    let indent_of = |i: usize| list_indent(lines[i]);
    // the moved block takes the items nested under its last line
    let mut last = last;
    if let Some(base) = indent_of(last) {
        while last + 1 < n && indent_of(last + 1).is_some_and(|x| x > base) {
            last += 1;
        }
    }
    let size = |a: usize, b: usize| -> isize { lines[a..b].iter().map(|l| l.chars().count() as isize + 1).sum() };
    // `head` lines stay, then the two blocks swap, then the lines from `tail` on stay
    let (head, neighbour, tail, shift) = if up {
        if first == 0 {
            return None;
        }
        let mut p = first - 1;
        if let Some(mine) = indent_of(first) {
            while p > 0 && indent_of(p).is_some_and(|x| x > mine) {
                p -= 1;
            }
            if indent_of(p).is_some_and(|x| x < mine) {
                return None;
            }
        }
        (p, (p, first), last + 1, -size(p, first))
    } else {
        if last + 1 >= n {
            return None;
        }
        let q = last + 1;
        let mut e = q;
        if let Some(theirs) = indent_of(q) {
            if indent_of(first).is_some_and(|mine| theirs < mine) {
                return None;
            }
            while e + 1 < n && indent_of(e + 1).is_some_and(|x| x > theirs) {
                e += 1;
            }
        }
        (first, (q, e + 1), e + 1, size(q, e + 1))
    };
    let mut out: Vec<&str> = lines[..head].to_vec();
    if up {
        out.extend(&lines[first..=last]);
        out.extend(&lines[neighbour.0..neighbour.1]);
    } else {
        out.extend(&lines[neighbour.0..neighbour.1]);
        out.extend(&lines[first..=last]);
    }
    out.extend(&lines[tail..]);
    Some(Moved { text: out.join("\n"), shift })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indenting_adds_a_level_to_list_lines_only() {
        assert_eq!(indent("- a\n- [ ] b\n1. c\n\u{2013} d\nplain"), Some("  - a\n  - [ ] b\n  1. c\n  \u{2013} d\nplain".into()));
        assert_eq!(indent("plain\n# Head"), None, "no list line, nothing to do");
        assert_eq!(indent("        - deep"), None, "the deepest level stays");
        assert_eq!(indent("      - a"), Some("        - a".into()));
    }

    #[test]
    fn outdenting_removes_a_level_and_stops_at_the_margin() {
        assert_eq!(outdent("    - a\n  - b\n- c"), Some("  - a\n- b\n- c".into()));
        assert_eq!(outdent("- a\nplain"), None);
        assert_eq!(outdent(indent("- a").unwrap().as_str()), Some("- a".into()), "indent then outdent is a round trip");
        assert_eq!(outdent("   - odd"), Some(" - odd".into()));
    }

    const L: &str = "T\n- a\n  - a1\n  - a2\n- b\n- c";

    #[test]
    fn an_item_moves_down_past_its_neighbour_with_its_children() {
        // "a" has two children; moving it down passes "b" and takes a1 and a2 along
        let m = move_lines(L, 1, 1, false).unwrap();
        assert_eq!(m.text, "T\n- b\n- a\n  - a1\n  - a2\n- c");
        assert_eq!(m.shift, "- b\n".len() as isize);
        // "b" moves down past "c"
        assert_eq!(move_lines(L, 4, 4, false).unwrap().text, "T\n- a\n  - a1\n  - a2\n- c\n- b");
        assert_eq!(move_lines(L, 5, 5, false), None, "the last line cannot go down");
    }

    #[test]
    fn an_item_moves_up_past_the_whole_neighbour_including_its_children() {
        let m = move_lines(L, 4, 4, true).unwrap();
        assert_eq!(m.text, "T\n- b\n- a\n  - a1\n  - a2\n- c");
        assert_eq!(m.shift, -("- a\n  - a1\n  - a2\n".len() as isize));
        assert_eq!(move_lines(L, 0, 0, true), None);
    }

    #[test]
    fn a_nested_item_stays_inside_its_parent() {
        // a1 may go below a2, but a2 cannot move up out of the group, nor a1 up past its parent
        assert_eq!(move_lines(L, 2, 2, false).unwrap().text, "T\n- a\n  - a2\n  - a1\n- b\n- c");
        assert_eq!(move_lines(L, 2, 2, true), None);
        assert_eq!(move_lines(L, 3, 3, false), None);
        assert_eq!(move_lines(L, 3, 3, true).unwrap().text, "T\n- a\n  - a2\n  - a1\n- b\n- c");
    }

    #[test]
    fn several_selected_lines_move_together_and_plain_lines_swap() {
        let t = "- a\n- b\n- c\n- d";
        assert_eq!(move_lines(t, 1, 2, true).unwrap().text, "- b\n- c\n- a\n- d");
        assert_eq!(move_lines(t, 0, 1, false).unwrap().text, "- c\n- a\n- b\n- d");
        assert_eq!(move_lines("one\ntwo\nthree", 1, 1, true).unwrap().text, "two\none\nthree");
        assert_eq!(move_lines("one\ntwo", 0, 5, true), None, "a range past the end is refused");
    }
}
