//! Text and background colours of the user's own: one line "text background" in hex, kept as the Custom entry at the
//! end of the Color theme cycle.
use super::*;
use termcore::palette::Rgb;

/// The pref that holds the custom colours as typed.
pub const KEY: &str = "terminal.colors";

/// One colour as typed: three or six hex digits, with or without a leading #.
fn parse_color(word: &str) -> Option<Rgb> {
    let digits = word.strip_prefix('#').unwrap_or(word);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize, len: usize| u8::from_str_radix(&digits[at..at + len], 16).ok();
    match digits.len() {
        3 => {
            let (r, g, b) = (channel(0, 1)?, channel(1, 1)?, channel(2, 1)?);
            Some((r * 17, g * 17, b * 17))
        }
        6 => Some((channel(0, 2)?, channel(2, 2)?, channel(4, 2)?)),
        _ => None,
    }
}

/// The colours a typed line gives: text and background. Ok(None) for an empty line, which gives the custom colours up.
pub fn parse_colors(typed: &str) -> Result<Option<(Rgb, Rgb)>, String> {
    let words: Vec<&str> = typed.split_whitespace().collect();
    match words.as_slice() {
        [] => Ok(None),
        [text, background] => match (parse_color(text), parse_color(background)) {
            (Some(text), Some(background)) => Ok(Some((text, background))),
            _ => Err("Colors are three or six hex digits, for example f2f2f2".into()),
        },
        _ => Err("Type the text color and the background color, for example f2f2f2 000000".into()),
    }
}

/// Colours as the line the panel shows and the pref keeps.
pub fn format_colors((text, background): (Rgb, Rgb)) -> String {
    let hex = |(r, g, b): Rgb| format!("{r:02x}{g:02x}{b:02x}");
    format!("{} {}", hex(text), hex(background))
}

impl Shell {
    /// Opens the Text and background colors panel with the colours in use.
    pub(super) fn terminal_open_colors(&mut self, t: f64) {
        let current = self.terminal_colors();
        let mut field = TextField::new(&self.ui, "Text and background, for example f2f2f2 000000", 14.0, false, 32);
        field.set_text(&self.ui, &format_colors(current));
        self.terminal_ui.entry = Some(field);
        self.terminal_ui.panel = Panel::Colors;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Takes the typed colours as the Custom entry and switches to it; an empty line gives the custom colours up.
    pub(super) fn terminal_apply_colors(&mut self, t: f64) {
        let typed = self.terminal_ui.entry.as_ref().map(|field| field.text().to_string()).unwrap_or_default();
        let colors = match parse_colors(&typed) {
            Ok(colors) => colors,
            Err(why) => {
                self.terminal_ui.message = why;
                return;
            }
        };
        self.terminal_ui.custom_colors = colors;
        let custom = profile_table::PROFILES.len();
        match colors {
            Some(_) => self.terminal_ui.profile = Some(custom),
            None if self.terminal_ui.profile == Some(custom) => self.terminal_ui.profile = None,
            None => {}
        }
        let saved = self.prefs.set(KEY, &colors.map(format_colors).unwrap_or_default());
        self.terminal_appearance();
        self.terminal_ui.entry = None;
        self.terminal_close_panel(t);
        self.terminal_ui.message = match (saved, colors) {
            (Err(e), _) => format!("Could not save the colors: {e}"),
            (Ok(()), Some(_)) => "Custom colors in use".into(),
            (Ok(()), None) => "Custom colors removed".into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_three_or_six_hex_digits() {
        assert_eq!(parse_color("f2f2f2"), Some((242, 242, 242)));
        assert_eq!(parse_color("#000000"), Some((0, 0, 0)));
        assert_eq!(parse_color("#fa0"), Some((255, 170, 0)));
        assert_eq!(parse_color("FFF"), Some((255, 255, 255)));
        for bad in ["", "#", "ff", "ffff", "fffff", "fffffff", "ggg", "12345z", "#12 3"] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_line_gives_text_and_background_or_nothing() {
        assert_eq!(parse_colors(" f2f2f2   #000 "), Ok(Some(((242, 242, 242), (0, 0, 0)))));
        assert_eq!(parse_colors(""), Ok(None));
        assert_eq!(parse_colors("  \t"), Ok(None));
        assert!(parse_colors("f2f2f2").unwrap_err().starts_with("Type the text color"));
        assert!(parse_colors("a b c").unwrap_err().starts_with("Type the text color"));
        assert!(parse_colors("f2f2f2 zzz").unwrap_err().starts_with("Colors are"));
    }

    #[test]
    fn the_shown_line_reads_back_the_same() {
        let colors = ((242, 242, 242), (0, 10, 255));
        assert_eq!(format_colors(colors), "f2f2f2 000aff");
        assert_eq!(parse_colors(&format_colors(colors)), Ok(Some(colors)));
    }
}
