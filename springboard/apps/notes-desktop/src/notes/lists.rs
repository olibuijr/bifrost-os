//! Line-level list commands on a note's Markdown: mark or unmark every checklist item, indent and outdent a list
//! line, and move a line up or down past its neighbour. All are pure functions of the text, so the editor wires them
//! through one undoable rewrite.
use springboard::shell::md;

/// Spaces one indent level adds to a list line.
pub const INDENT: usize = 2;

/// The line index and char column of a caret given as a char offset.
pub fn line_col(text: &str, caret: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for c in text.chars().take(caret) {
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The char offset of the start of `line` (clamped to the end of the text).
pub fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

/// Every checklist item ticked (`done`) or unticked; other lines are left alone.
pub fn set_all_checks(text: &str, done: bool) -> String {
    text.split('\n')
        .map(|l| match md::classify(l).block {
            md::Block::Check(now) if now != done => md::toggle_check(l).unwrap_or_else(|| l.to_string()),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `line` is a list line (bullet, number or checklist item).
pub fn is_list_line(line: &str) -> bool {
    matches!(md::classify(line).block, md::Block::Bullet | md::Block::Numbered(_) | md::Block::Check(_))
}

/// Indents list line `line` by one level; None when it is not a list line or does not exist.
pub fn indent(text: &str, line: usize) -> Option<String> {
    map_line(text, line, |l| is_list_line(l).then(|| format!("{}{l}", " ".repeat(INDENT))))
}

/// Outdents list line `line` by one level; None when it is not indented.
pub fn outdent(text: &str, line: usize) -> Option<String> {
    map_line(text, line, |l| {
        let lead = l.chars().take_while(|&c| c == ' ').count();
        (is_list_line(l) && lead > 0).then(|| l.chars().skip(lead.min(INDENT)).collect())
    })
}

/// Swaps `line` with the one above (`up`) or below it; None at the edge of the note.
pub fn move_line(text: &str, line: usize, up: bool) -> Option<String> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let other = if up { line.checked_sub(1)? } else { line + 1 };
    if line >= lines.len() || other >= lines.len() {
        return None;
    }
    lines.swap(line, other);
    Some(lines.join("\n"))
}

fn map_line(text: &str, line: usize, f: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let new = f(lines.get(line)?)?;
    lines[line] = new;
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_unmarks_every_item() {
        let t = "Shopping\n- [ ] a\n- [x] b\n- plain\n  - [ ] c";
        assert_eq!(set_all_checks(t, true), "Shopping\n- [x] a\n- [x] b\n- plain\n  - [x] c");
        assert_eq!(set_all_checks(t, false), "Shopping\n- [ ] a\n- [ ] b\n- plain\n  - [ ] c");
    }

    #[test]
    fn indent_and_outdent_only_touch_list_lines() {
        let t = "Title\n- one\n  - two";
        assert_eq!(indent(t, 1).unwrap(), "Title\n  - one\n  - two");
        assert_eq!(indent(t, 0), None);
        assert_eq!(outdent(t, 2).unwrap(), "Title\n- one\n- two");
        assert_eq!(outdent(t, 1), None);
        assert_eq!(indent(t, 9), None);
    }

    #[test]
    fn moves_a_line_past_its_neighbour() {
        let t = "a\nb\nc";
        assert_eq!(move_line(t, 1, true).unwrap(), "b\na\nc");
        assert_eq!(move_line(t, 1, false).unwrap(), "a\nc\nb");
        assert_eq!(move_line(t, 0, true), None);
        assert_eq!(move_line(t, 2, false), None);
    }

    #[test]
    fn locates_lines_and_columns() {
        let t = "ab\ncde\nf";
        assert_eq!(line_col(t, 0), (0, 0));
        assert_eq!(line_col(t, 4), (1, 1));
        assert_eq!(line_start(t, 0), 0);
        assert_eq!(line_start(t, 1), 3);
        assert_eq!(line_start(t, 2), 7);
        assert_eq!(line_start(t, 9), 8);
    }
}
