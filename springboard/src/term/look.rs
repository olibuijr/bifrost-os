//! A profile's look beyond its colour theme: the 16 ANSI colours, Bold as Bright, Allow Blink and the character and
//! line spacing. Saved in the shell prefs as `terminal.look.*` (see [`Look::saved`] and [`Look::from_prefs`]); the grid
//! paints with `Look::theme` and copy-as-RTF reads the same colours.
use termcore::palette::{Rgb, Theme, THEMES};

/// Smallest and largest spacing multiplier, and the stepper's step.
pub const SPACING_MIN: f32 = 0.8;
pub const SPACING_MAX: f32 = 2.0;
pub const SPACING_STEP: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The ANSI palette the profile set; None follows the colour theme.
    pub ansi: Option<[Rgb; 16]>,
    /// Bold text in ANSI 0-7 is drawn in the bright colour (8-15).
    pub bold_bright: bool,
    /// Text marked blink is allowed to blink. Stored now; the core does not track SGR 5 yet, so nothing draws it.
    pub allow_blink: bool,
    pub char_spacing: f32,
    pub line_spacing: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look { ansi: None, bold_bright: true, allow_blink: true, char_spacing: 1.0, line_spacing: 1.0 }
    }
}

/// Clamps a spacing value to the stepper range and rounds it to the step (so 1.0 stays exactly 1.0).
pub fn clamp_spacing(v: f32) -> f32 {
    let v = v.clamp(SPACING_MIN, SPACING_MAX);
    ((v / SPACING_STEP).round() * SPACING_STEP * 10.0).round() / 10.0
}

/// `rrggbb` for each of the 16 colours, comma separated.
pub fn format_ansi(ansi: &[Rgb; 16]) -> String {
    ansi.iter().map(|(r, g, b)| format!("{r:02x}{g:02x}{b:02x}")).collect::<Vec<_>>().join(",")
}

/// Reads the 16 colours [`format_ansi`] wrote; anything else is None.
pub fn parse_ansi(text: &str) -> Option<[Rgb; 16]> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 16 {
        return None;
    }
    let mut out = [(0u8, 0u8, 0u8); 16];
    for (slot, part) in out.iter_mut().zip(parts) {
        if part.len() != 6 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let n = u32::from_str_radix(part, 16).ok()?;
        *slot = ((n >> 16) as u8, (n >> 8) as u8, n as u8);
    }
    Some(out)
}

impl Look {
    /// The palette in use: the profile's own, else the theme's.
    pub fn palette(&self, theme: usize) -> [Rgb; 16] {
        self.ansi.unwrap_or(THEMES[theme.min(THEMES.len() - 1)].ansi)
    }

    /// Theme `theme` with this look's palette, owned, for the grid to paint with.
    pub fn theme(&self, theme: usize) -> Theme {
        let base = &THEMES[theme.min(THEMES.len() - 1)];
        Theme { name: base.name, fg: base.fg, bg: base.bg, ansi: self.palette(theme) }
    }

    /// The `(key, value)` pairs to store. An unset palette is stored as an empty value (it means the theme's).
    pub fn saved(&self) -> Vec<(&'static str, String)> {
        vec![
            ("terminal.look.ansi", self.ansi.as_ref().map(format_ansi).unwrap_or_default()),
            ("terminal.look.bold_bright", flag(self.bold_bright)),
            ("terminal.look.allow_blink", flag(self.allow_blink)),
            ("terminal.look.char_spacing", format!("{:.1}", self.char_spacing)),
            ("terminal.look.line_spacing", format!("{:.1}", self.line_spacing)),
        ]
    }

    /// Reads what [`Look::saved`] stored. Missing or malformed values fall back to the defaults.
    pub fn from_prefs(get: impl Fn(&str) -> Option<String>) -> Look {
        let d = Look::default();
        let spacing = |key: &str, fallback: f32| {
            get(key).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).map_or(fallback, clamp_spacing)
        };
        Look {
            ansi: get("terminal.look.ansi").and_then(|v| parse_ansi(&v)),
            bold_bright: get("terminal.look.bold_bright").map_or(d.bold_bright, |v| v == "1"),
            allow_blink: get("terminal.look.allow_blink").map_or(d.allow_blink, |v| v == "1"),
            char_spacing: spacing("terminal.look.char_spacing", d.char_spacing),
            line_spacing: spacing("terminal.look.line_spacing", d.line_spacing),
        }
    }
}

fn flag(on: bool) -> String {
    String::from(if on { "1" } else { "0" })
}

/// The name shown beside an ANSI well (the xterm order).
pub fn ansi_name(i: usize) -> &'static str {
    const NAMES: [&str; 16] = [
        "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White", "Bright Black", "Bright Red",
        "Bright Green", "Bright Yellow", "Bright Blue", "Bright Magenta", "Bright Cyan", "Bright White",
    ];
    NAMES[i.min(15)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn palette_persistence_round_trips() {
        let mut ansi = THEMES[0].ansi;
        ansi[1] = (0x12, 0x34, 0x56);
        ansi[15] = (0xFF, 0x00, 0x7F);
        let look = Look { ansi: Some(ansi), bold_bright: false, allow_blink: false, char_spacing: 1.3, line_spacing: 0.9 };
        let store: HashMap<&str, String> = look.saved().into_iter().collect();
        let back = Look::from_prefs(|k| store.get(k).cloned());
        assert_eq!(back, look);
        assert_eq!(back.palette(0)[1], (0x12, 0x34, 0x56));
    }

    #[test]
    fn an_unset_palette_follows_the_theme_and_survives_a_round_trip() {
        let look = Look::default();
        let store: HashMap<&str, String> = look.saved().into_iter().collect();
        assert_eq!(store["terminal.look.ansi"], "");
        assert_eq!(Look::from_prefs(|k| store.get(k).cloned()).ansi, None);
        assert_eq!(look.palette(1), THEMES[1].ansi);
        assert_eq!(look.theme(2).ansi, THEMES[2].ansi);
    }

    #[test]
    fn bad_values_fall_back_and_spacing_is_clamped() {
        assert_eq!(parse_ansi("000000,ffffff"), None);
        assert_eq!(parse_ansi(&format!("{},zzzzzz", vec!["000000"; 15].join(","))), None);
        let look = Look::from_prefs(|k| match k {
            "terminal.look.char_spacing" => Some("9.5".into()),
            "terminal.look.line_spacing" => Some("nan".into()),
            _ => None,
        });
        assert_eq!(look.char_spacing, SPACING_MAX);
        assert_eq!(look.line_spacing, 1.0);
        assert!(look.bold_bright && look.allow_blink);
    }

    #[test]
    fn spacing_steps_land_on_tenths() {
        assert_eq!(clamp_spacing(1.04), 1.0);
        assert_eq!(clamp_spacing(1.16), 1.2);
        assert_eq!(clamp_spacing(0.1), SPACING_MIN);
        assert_eq!(clamp_spacing(1.0 + SPACING_STEP), 1.1);
    }
}
