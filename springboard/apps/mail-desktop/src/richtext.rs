//! Rich text for the message being written (the Format menu): bold, italic, underline and colour, kept as one style for each
//! character of the text, plus the line-breaking that draws the styled text and the HTML a styled message is sent as. The
//! style a character gets is the one in force when it is typed, so Format > Bold turns bold on for the text typed next;
//! lists and indents are ordinary characters at the start of a line.

/// The colours a message's text can have (Format > Text Color).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColor {
    Red,
    Orange,
    Green,
    Blue,
    Purple,
    Gray,
}

impl TextColor {
    pub const ALL: [TextColor; 6] = [TextColor::Red, TextColor::Orange, TextColor::Green, TextColor::Blue, TextColor::Purple, TextColor::Gray];

    pub fn label(self) -> &'static str {
        match self {
            TextColor::Red => "Red",
            TextColor::Orange => "Orange",
            TextColor::Green => "Green",
            TextColor::Blue => "Blue",
            TextColor::Purple => "Purple",
            TextColor::Gray => "Gray",
        }
    }

    /// The colour as the HTML of the message writes it.
    fn css(self) -> &'static str {
        match self {
            TextColor::Red => "#d70015",
            TextColor::Orange => "#c93400",
            TextColor::Green => "#248a3d",
            TextColor::Blue => "#0040dd",
            TextColor::Purple => "#8944ab",
            TextColor::Gray => "#6e6e73",
        }
    }
}

/// How one character of the message looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<TextColor>,
}

impl Style {
    pub fn is_plain(self) -> bool {
        self == Style::default()
    }

    /// The `style` attribute value for this look, empty for plain text.
    fn css(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.bold {
            parts.push("font-weight: bold".into());
        }
        if self.italic {
            parts.push("font-style: italic".into());
        }
        if self.underline {
            parts.push("text-decoration: underline".into());
        }
        if let Some(color) = self.color {
            parts.push(format!("color: {}", color.css()));
        }
        parts.join("; ")
    }
}

/// A stretch of one line in one style. `start` is the position of its first character in the whole text, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    pub start: usize,
}

/// One character with its place in the text and its style.
#[derive(Clone, Copy)]
struct Cell {
    index: usize,
    c: char,
    style: Style,
}

/// Groups characters into runs of one style.
fn runs_of(cells: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for cell in cells {
        match runs.last_mut() {
            Some(run) if run.style == cell.style => run.text.push(cell.c),
            _ => runs.push(Run { text: cell.c.to_string(), style: cell.style, start: cell.index }),
        }
    }
    runs
}

fn width_of(cells: &[Cell], measure: &dyn Fn(&str, Style) -> f32) -> f32 {
    runs_of(cells).iter().map(|run| measure(&run.text, run.style)).sum()
}

/// Breaks the text into lines no wider than `width`, where `measure` gives the width of a piece of text in a style. A new
/// line starts at each `\n`; a word that does not fit goes to the next line; a word wider than a line stays whole on a line
/// of its own. Spaces are kept (an indent stays an indent) except those at a break. An empty text is one empty line.
pub fn layout(text: &str, styles: &[Style], width: f32, measure: &dyn Fn(&str, Style) -> f32) -> Vec<Vec<Run>> {
    let cells: Vec<Cell> = text.chars().enumerate().map(|(index, c)| Cell { index, c, style: styles.get(index).copied().unwrap_or_default() }).collect();
    let mut lines = Vec::new();
    for paragraph in cells.split(|cell| cell.c == '\n') {
        let mut line: Vec<Cell> = Vec::new();
        let mut line_width = 0.0;
        let mut i = 0;
        while i < paragraph.len() {
            let space = paragraph[i].c == ' ';
            let mut j = i;
            while j < paragraph.len() && (paragraph[j].c == ' ') == space {
                j += 1;
            }
            let token = &paragraph[i..j];
            let token_width = width_of(token, measure);
            let has_word = line.iter().any(|cell| cell.c != ' ');
            if !space && has_word && line_width + token_width > width {
                while line.last().is_some_and(|cell| cell.c == ' ') {
                    line.pop();
                }
                lines.push(runs_of(&line));
                line.clear();
                line_width = 0.0;
            }
            line.extend_from_slice(token);
            line_width += token_width;
            i = j;
        }
        lines.push(runs_of(&line));
    }
    lines
}

/// Writes `text` as HTML. `previous_space` carries over from the text before, so a space after a space (or at the start of a
/// line) can be told from one between two words.
fn escape(text: &str, out: &mut String, previous_space: &mut bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>\n"),
            // a space that starts a line or follows another would be folded away: it is written so that it shows
            ' ' if *previous_space => out.push_str("&nbsp;"),
            c => out.push(c),
        }
        *previous_space = c == ' ' || c == '\n';
    }
}

/// The text with its styles as HTML: each run of one look is a `span`, newlines are `<br>`, and spaces that would be folded
/// away are non-breaking.
pub fn to_html(text: &str, styles: &[Style]) -> String {
    let cells: Vec<Cell> = text.chars().enumerate().map(|(index, c)| Cell { index, c, style: styles.get(index).copied().unwrap_or_default() }).collect();
    let mut out = String::new();
    let mut previous_space = true;
    for run in runs_of(&cells) {
        let css = run.style.css();
        if css.is_empty() {
            escape(&run.text, &mut out, &mut previous_space);
        } else {
            out.push_str(&format!("<span style=\"{css}\">"));
            escape(&run.text, &mut out, &mut previous_space);
            out.push_str("</span>");
        }
    }
    out
}

/// The HTML document a styled message is sent as: the message, then the signature set off by `-- ` in plain style. None
/// when no character is styled, because then the plain text says it all.
pub fn html_body(text: &str, styles: &[Style], signature: &str) -> Option<String> {
    if styles.iter().take(text.chars().count()).all(|style| style.is_plain()) {
        return None;
    }
    let mut body = to_html(text, styles);
    let signature = signature.trim_end();
    if !signature.trim().is_empty() {
        body.push_str("<br>\n<br>\n-- <br>\n");
        escape(signature, &mut body, &mut true);
    }
    Some(format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body style=\"font-family: sans-serif; font-size: 14px;\">\n{body}\n</body>\n</html>\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10 wide, 12 when bold.
    fn measure(text: &str, style: Style) -> f32 {
        text.chars().count() as f32 * if style.bold { 12.0 } else { 10.0 }
    }

    fn plain(n: usize) -> Vec<Style> {
        vec![Style::default(); n]
    }

    fn texts(lines: &[Vec<Run>]) -> Vec<String> {
        lines.iter().map(|line| line.iter().map(|run| run.text.as_str()).collect::<String>()).collect()
    }

    #[test]
    fn words_wrap_at_the_width_and_newlines_start_lines() {
        let text = "one two three\nfour";
        let lines = layout(text, &plain(text.chars().count()), 80.0, &measure);
        assert_eq!(texts(&lines), vec!["one two", "three", "four"]);
        assert_eq!(texts(&layout("", &[], 80.0, &measure)), vec![""], "an empty text is one empty line");
        assert_eq!(texts(&layout("a\n\nb", &plain(4), 80.0, &measure)), vec!["a", "", "b"]);
    }

    #[test]
    fn an_indent_stays_and_a_word_wider_than_a_line_stays_whole() {
        let lines = layout("    indented", &plain(12), 200.0, &measure);
        assert_eq!(texts(&lines), vec!["    indented"]);
        let lines = layout("  supercalifragilistic", &plain(22), 60.0, &measure);
        assert_eq!(texts(&lines), vec!["  supercalifragilistic"], "no break inside a word, and none after the indent alone");
    }

    #[test]
    fn spaces_at_a_break_go_but_a_space_typed_at_the_end_stays() {
        let lines = layout("ab cd ", &plain(6), 25.0, &measure);
        assert_eq!(texts(&lines), vec!["ab", "cd "]);
    }

    #[test]
    fn runs_split_where_the_style_changes_and_know_where_they_start() {
        let text = "a bold word";
        let mut styles = plain(11);
        for style in &mut styles[2..6] {
            style.bold = true;
        }
        let lines = layout(text, &styles, 1000.0, &measure);
        assert_eq!(lines.len(), 1);
        let runs = &lines[0];
        assert_eq!(runs.iter().map(|r| (r.text.as_str(), r.style.bold, r.start)).collect::<Vec<_>>(), vec![("a ", false, 0), ("bold", true, 2), (" word", false, 6)]);
    }

    #[test]
    fn a_bold_word_takes_its_wider_width_when_wrapping() {
        let text = "aaaa bbbb";
        let mut styles = plain(9);
        for style in &mut styles[5..9] {
            style.bold = true;
        }
        // plain: 4 + 1 + 4 = 90 wide fits in 90; bold makes the second word 48 wide, so the line is 98 and breaks
        assert_eq!(texts(&layout(text, &plain(9), 90.0, &measure)), vec!["aaaa bbbb"]);
        assert_eq!(texts(&layout(text, &styles, 90.0, &measure)), vec!["aaaa", "bbbb"]);
    }

    #[test]
    fn styles_become_spans_and_text_is_escaped() {
        let text = "a <b> & c\nd  e";
        let mut styles = plain(text.chars().count());
        styles[0].bold = true;
        styles[0].color = Some(TextColor::Red);
        for style in &mut styles[2..5] {
            style.italic = true;
            style.underline = true;
        }
        let html = to_html(text, &styles);
        assert_eq!(
            html,
            "<span style=\"font-weight: bold; color: #d70015\">a</span> <span style=\"font-style: italic; text-decoration: underline\">&lt;b&gt;</span> &amp; c<br>\nd &nbsp;e"
        );
    }

    #[test]
    fn a_plain_message_has_no_html_and_a_styled_one_gets_a_document_with_the_signature() {
        assert_eq!(html_body("plain", &plain(5), "Ana"), None);
        assert_eq!(html_body("plain", &[], "Ana"), None, "missing styles are plain");
        let mut styles = plain(5);
        styles[0].bold = true;
        let html = html_body("bold!", &styles, "Ana\nOlafsson").unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<span style=\"font-weight: bold\">b</span>old!<br>\n<br>\n-- <br>\nAna<br>\nOlafsson"), "{html}");
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html_body("bold!", &styles, "  ").unwrap().contains("old!\n</body>"));
    }

    #[test]
    fn styles_beyond_the_text_do_not_make_it_rich() {
        let mut styles = plain(10);
        styles[8].bold = true;
        assert_eq!(html_body("short", &styles, ""), None);
    }
}
