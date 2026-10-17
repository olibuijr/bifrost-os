//! Rich text in a reminder's notes. The notes are stored as Markdown text, and the Format menu writes the marks Markdown
//! reads: `**bold**`, `- ` bullets and `1. ` numbers. A command on text that already has its mark takes the mark away
//! again, so each command is a toggle. Offsets are byte offsets into the text, as the text field reports them.

/// The Format menu's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Bold,
    Bullets,
    Numbers,
}

/// The text after `style` is applied to the selection `(a, b)` (in either order), and the new selection. With no
/// selection, bold inserts an empty pair of marks with the caret between them; a list command changes the line the
/// caret is on.
pub fn apply(text: &str, selection: (usize, usize), style: Style) -> (String, (usize, usize)) {
    let (a, b) = clamp(text, selection);
    match style {
        Style::Bold => bold(text, a, b),
        Style::Bullets | Style::Numbers => lines(text, a, b, style),
    }
}

/// Moves `(a, b)` into order, inside the text and onto character boundaries.
fn clamp(text: &str, (x, y): (usize, usize)) -> (usize, usize) {
    let (mut a, mut b) = (x.min(y).min(text.len()), x.max(y).min(text.len()));
    while !text.is_char_boundary(a) {
        a -= 1;
    }
    while !text.is_char_boundary(b) {
        b += 1;
    }
    (a, b)
}

fn bold(text: &str, a: usize, b: usize) -> (String, (usize, usize)) {
    let wrapped = a >= 2 && text.get(a - 2..a) == Some("**") && text[b..].starts_with("**");
    if wrapped {
        // the selection already sits inside its marks: take them away
        let out = format!("{}{}{}", &text[..a - 2], &text[a..b], &text[b + 2..]);
        return (out, (a - 2, b - 2));
    }
    if a == b {
        return (format!("{}****{}", &text[..a], &text[a..]), (a + 2, a + 2));
    }
    (format!("{}**{}**{}", &text[..a], &text[a..b], &text[b..]), (a + 2, b + 2))
}

/// The lines the selection touches, with the list mark added to each (or taken from each when all of them have it).
fn lines(text: &str, a: usize, b: usize, style: Style) -> (String, (usize, usize)) {
    let start = text[..a].rfind('\n').map_or(0, |i| i + 1);
    let end = text[b..].find('\n').map_or(text.len(), |i| b + i);
    let block: Vec<&str> = text[start..end].split('\n').collect();
    let has_mark = |line: &str| match style {
        Style::Bullets => line.starts_with("- "),
        _ => numbered(line).is_some(),
    };
    let all_marked = block.iter().filter(|l| !l.trim().is_empty()).all(|l| has_mark(l));
    let mut next = 0;
    let rewritten: Vec<String> = block
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                return (*line).to_string();
            }
            if all_marked {
                return match style {
                    Style::Bullets => line["- ".len()..].to_string(),
                    _ => numbered(line).unwrap_or(line).to_string(),
                };
            }
            next += 1;
            match style {
                Style::Bullets => format!("- {line}"),
                _ => format!("{next}. {line}"),
            }
        })
        .collect();
    let block_text = rewritten.join("\n");
    let out = format!("{}{}{}", &text[..start], block_text, &text[end..]);
    (out, (start, start + block_text.len()))
}

/// The text after an ordered-list mark (`12. `), or `None` when the line has none.
fn numbered(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    line[digits..].strip_prefix(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_wraps_the_selection_in_marks() {
        let (out, sel) = apply("buy milk", (4, 8), Style::Bold);
        assert_eq!(out, "buy **milk**");
        assert_eq!(sel, (6, 10));
    }

    #[test]
    fn bold_with_no_selection_leaves_the_caret_between_the_marks() {
        let (out, sel) = apply("buy", (3, 3), Style::Bold);
        assert_eq!(out, "buy****");
        assert_eq!(sel, (5, 5));
    }

    #[test]
    fn bold_again_takes_the_marks_away() {
        let (out, sel) = apply("buy **milk**", (6, 10), Style::Bold);
        assert_eq!(out, "buy milk");
        assert_eq!(sel, (4, 8));
    }

    #[test]
    fn bullets_mark_each_line_the_selection_touches() {
        let (out, _) = apply("eggs\nbread\ntea", (1, 9), Style::Bullets);
        assert_eq!(out, "- eggs\n- bread\ntea");
    }

    #[test]
    fn bullets_again_take_the_marks_away() {
        let text = "- eggs\n- bread";
        let (out, _) = apply(text, (0, text.len()), Style::Bullets);
        assert_eq!(out, "eggs\nbread");
    }

    #[test]
    fn numbers_count_the_lines_that_get_one() {
        let text = "a\n\nb";
        let (out, _) = apply(text, (0, text.len()), Style::Numbers);
        assert_eq!(out, "1. a\n\n2. b");
        let (back, _) = apply(&out, (0, out.len()), Style::Numbers);
        assert_eq!(back, "a\n\nb");
    }

    #[test]
    fn a_selection_out_of_range_or_off_a_character_is_moved_inside_the_text() {
        let (out, _) = apply("héllo", (2, 99), Style::Bold);
        assert!(out.starts_with('h'));
        assert!(out.contains("**"));
    }
}
