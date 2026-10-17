//! Copy Special: a terminal selection as RTF, with the colours and styles of its cells. The plain text copy stays the
//! first representation on the pasteboard; this is the `public.rtf` one beside it.
use termcore::palette::Rgb;

/// One styled piece of the selection. `bg: None` is the theme background (or no background at all).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Run {
    /// True when `other` would print in the same style, so the two can share one group.
    pub fn same_style(&self, other: &Run) -> bool {
        self.fg == other.fg && self.bg == other.bg && self.bold == other.bold && self.italic == other.italic && self.underline == other.underline
    }
}

/// The colour-table index of `c` (1-based; 0 is the automatic colour), adding it on first use.
fn slot(colors: &mut Vec<Rgb>, c: Rgb) -> usize {
    match colors.iter().position(|x| *x == c) {
        Some(i) => i + 1,
        None => {
            colors.push(c);
            colors.len()
        }
    }
}

/// Escapes text for RTF: braces and backslashes, newlines and tabs as control words, other non-ASCII as `\uN?`.
pub fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\par "),
            '\t' => out.push_str("\\tab "),
            c if (' '..='~').contains(&c) => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

/// A complete RTF document for `runs` in `font_pt` points. The colour table lists each colour once; the foreground is
/// `\cf`, a background is `\cb`, and bold, italic and underline are `\b`, `\i` and `\ul`.
pub fn to_rtf(runs: &[Run], font_pt: f32) -> String {
    let mut colors: Vec<Rgb> = Vec::new();
    let mut body = String::new();
    for run in runs {
        let fg = slot(&mut colors, run.fg);
        let mut open = format!("\\cf{fg}");
        if let Some(bg) = run.bg {
            open.push_str(&format!("\\cb{}", slot(&mut colors, bg)));
        }
        if run.bold {
            open.push_str("\\b");
        }
        if run.italic {
            open.push_str("\\i");
        }
        if run.underline {
            open.push_str("\\ul");
        }
        body.push_str(&format!("{{{open} {}}}", escape(&run.text)));
    }
    let table: String = colors.iter().map(|(r, g, b)| format!("\\red{r}\\green{g}\\blue{b};")).collect();
    let half_points = (font_pt * 2.0).round() as i32;
    format!("{{\\rtf1\\ansi\\deff0{{\\fonttbl{{\\f0 SF Mono;}}}}{{\\colortbl;{table}}}\\f0\\fs{half_points}\n{body}}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, fg: Rgb, bg: Option<Rgb>, bold: bool) -> Run {
        Run { text: text.into(), fg, bg, bold, italic: false, underline: false }
    }

    #[test]
    fn styled_selection_exports_colours_and_styles() {
        let runs = [run("ok", (255, 0, 0), Some((0, 0, 255)), true), run(" plain", (0, 0, 0), None, false)];
        let rtf = to_rtf(&runs, 12.0);
        assert!(rtf.starts_with("{\\rtf1\\ansi"), "{rtf}");
        assert!(rtf.contains("{\\colortbl;\\red255\\green0\\blue0;\\red0\\green0\\blue255;\\red0\\green0\\blue0;}"), "{rtf}");
        assert!(rtf.contains("\\fs24"), "{rtf}");
        assert!(rtf.contains("{\\cf1\\cb2\\b ok}"), "{rtf}");
        assert!(rtf.contains("{\\cf3  plain}"), "{rtf}");
        assert!(rtf.ends_with('}'));
    }

    #[test]
    fn without_a_background_no_cb_is_written() {
        let rtf = to_rtf(&[run("x", (1, 2, 3), None, false)], 12.0);
        assert!(!rtf.contains("\\cb"), "{rtf}");
        assert!(rtf.contains("\\red1\\green2\\blue3;"));
    }

    #[test]
    fn text_is_escaped_and_lines_become_pars() {
        assert_eq!(escape("a{b}\\c\n\té"), "a\\{b\\}\\\\c\\par \\tab \\u233?");
        assert_eq!(escape("😀"), "\\u-10179?\\u-8704?");
    }

    #[test]
    fn a_repeated_colour_is_listed_once() {
        let runs = [run("a", (9, 9, 9), None, false), run("b", (9, 9, 9), None, true)];
        let rtf = to_rtf(&runs, 10.0);
        assert_eq!(rtf.matches("\\red9").count(), 1, "{rtf}");
        assert!(rtf.contains("\\fs20"));
    }
}
