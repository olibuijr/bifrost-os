//! Term profiles, URL detection and OSC 133 helpers (data and pure functions; the UI wiring follows).

/// Cursor shape of a Term profile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorStyle {
    Block,
    Underline,
    Bar,
}

impl CursorStyle {
    /// The caret shape the terminal grid draws for this style.
    pub fn shape(self) -> termcore::term::CursorShape {
        match self {
            CursorStyle::Block => termcore::term::CursorShape::Block,
            CursorStyle::Underline => termcore::term::CursorShape::Underline,
            CursorStyle::Bar => termcore::term::CursorShape::Bar,
        }
    }

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            CursorStyle::Block => "Block",
            CursorStyle::Underline => "Underline",
            CursorStyle::Bar => "Bar",
        }
    }

    /// The stored preference value (`terminal.cursor`).
    pub fn key(self) -> &'static str {
        match self {
            CursorStyle::Block => "block",
            CursorStyle::Underline => "underline",
            CursorStyle::Bar => "bar",
        }
    }

    /// The style a stored preference names; anything unknown is the block.
    pub fn from_key(key: Option<&str>) -> Self {
        match key {
            Some("underline") => CursorStyle::Underline,
            Some("bar") => CursorStyle::Bar,
            _ => CursorStyle::Block,
        }
    }

    /// The next style in the menu's cycle: Block, Underline, Bar.
    pub fn next(self) -> Self {
        match self {
            CursorStyle::Block => CursorStyle::Underline,
            CursorStyle::Underline => CursorStyle::Bar,
            CursorStyle::Bar => CursorStyle::Block,
        }
    }
}

/// A Term profile: colours, default text size and cursor. `follows_scheme` is Basic only
/// (black on white in Light, white on black in Dark); the others keep their colours in both schemes.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    /// background opacity 0..=255 (Clear Dark and Clear Light are translucent)
    pub alpha: u8,
    pub font_pt: f32,
    pub cursor: CursorStyle,
    pub blink: bool,
    pub follows_scheme: bool,
}

const fn p(
    name: &'static str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    alpha: u8,
    follows_scheme: bool,
) -> Profile {
    Profile { name, fg, bg, alpha, font_pt: 12.0, cursor: CursorStyle::Block, blink: false, follows_scheme }
}

/// The profiles Term ships (Settings > Profiles).
pub const PROFILES: [Profile; 9] = [
    p("Basic", (0, 0, 0), (255, 255, 255), 255, true),
    p("Pro", (242, 242, 242), (0, 0, 0), 255, false),
    p("Homebrew", (40, 254, 20), (0, 0, 0), 255, false),
    p("Ocean", (255, 255, 255), (34, 79, 188), 255, false),
    p("Novel", (63, 47, 42), (223, 219, 196), 255, false),
    p("Silver Aerogel", (0, 0, 0), (146, 146, 146), 255, false),
    p("Clear Dark", (255, 255, 255), (0, 0, 0), 150, false),
    p("Clear Light", (0, 0, 0), (255, 255, 255), 150, false),
    p("Red Sands", (215, 201, 167), (122, 37, 30), 255, false),
];

/// `(fg, bg)` of profile `i`, resolving Basic against the scheme.
pub fn profile_colors(i: usize, light: bool) -> ((u8, u8, u8), (u8, u8, u8)) {
    let pr = PROFILES[i.min(PROFILES.len() - 1)];
    if pr.follows_scheme && !light {
        (pr.bg, pr.fg)
    } else {
        (pr.fg, pr.bg)
    }
}

/// URLs in one screen row as `(start_col, end_col_exclusive, url)`, for Cmd+click. Columns are char indexes.
pub fn detect_urls(row: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = row.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().take(8).collect();
        let scheme = ["https://", "http://", "ftp://", "file://", "mailto:", "ssh://"]
            .iter()
            .any(|s| rest.starts_with(s));
        if !scheme || (i > 0 && chars[i - 1].is_alphanumeric()) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && !chars[end].is_whitespace() && !"<>\"'`".contains(chars[end]) {
            end += 1;
        }
        // trailing punctuation belongs to the sentence, an unmatched ")" too
        while end > i {
            let c = chars[end - 1];
            let opens = chars[i..end].iter().filter(|&&x| x == '(').count();
            let closes = chars[i..end].iter().filter(|&&x| x == ')').count();
            if ".,;:!?]}'".contains(c) || (c == ')' && closes > opens) {
                end -= 1;
            } else {
                break;
            }
        }
        if end > i + 7 {
            out.push((i, end, chars[i..end].iter().collect()));
        }
        i = end.max(i + 1);
    }
    out
}

/// Byte offsets of each OSC 133 `A` (prompt start) in a byte stream; callers map them to scrollback
/// lines for Cmd+Up and Cmd+Down.
pub fn osc133_prompts(data: &[u8]) -> Vec<usize> {
    let pat = b"\x1b]133;A";
    data.windows(pat.len()).enumerate().filter(|(_, w)| *w == pat).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_follows_scheme_and_others_do_not() {
        assert_eq!(profile_colors(0, true).1, (255, 255, 255));
        assert_eq!(profile_colors(0, false).1, (0, 0, 0));
        assert_eq!(profile_colors(2, true), profile_colors(2, false));
    }

    #[test]
    fn urls_trim_trailing_punctuation() {
        let u = detect_urls("see https://a.io/x_(y), and http://b.dev.");
        assert_eq!(u[0].2, "https://a.io/x_(y)");
        assert_eq!(u[1].2, "http://b.dev");
        assert!(detect_urls("xhttps://no").is_empty());
    }

    #[test]
    fn cursor_styles_cycle_and_round_trip_their_keys() {
        assert_eq!(CursorStyle::Block.next(), CursorStyle::Underline);
        assert_eq!(CursorStyle::Underline.next(), CursorStyle::Bar);
        assert_eq!(CursorStyle::Bar.next(), CursorStyle::Block);
        for style in [CursorStyle::Block, CursorStyle::Underline, CursorStyle::Bar] {
            assert_eq!(CursorStyle::from_key(Some(style.key())), style);
        }
        assert_eq!(CursorStyle::from_key(None), CursorStyle::Block);
        assert_eq!(CursorStyle::from_key(Some("beam")), CursorStyle::Block);
        assert_eq!(CursorStyle::Bar.shape(), termcore::term::CursorShape::Bar);
    }

    #[test]
    fn osc133_marks_found() {
        assert_eq!(osc133_prompts(b"a\x1b]133;A\x07b\x1b]133;A"), vec![1, 10]);
    }
}
