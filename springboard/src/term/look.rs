//! A profile's look beyond its colour theme: the 16 ANSI colours, Bold as Bright, Allow Blink and the character and
//! line spacing. Saved in the shell prefs as `terminal.look.*` (see [`Look::saved`] and [`Look::from_prefs`]); the grid
//! paints with `Look::theme` and copy-as-RTF reads the same colours.
use termcore::palette::{Rgb, Theme, THEMES};

/// Smallest and largest spacing multiplier, and the stepper's step.
pub const SPACING_MIN: f32 = 0.8;
pub const SPACING_MAX: f32 = 2.0;
pub const SPACING_STEP: f32 = 0.1;

/// Smallest and largest window opacity, and the largest background-image dim (a fully dark image hides the text).
pub const OPACITY_MIN: f32 = 0.5;
pub const OPACITY_MAX: f32 = 1.0;
pub const DIM_MAX: f32 = 0.9;

/// Longest background image path kept, in bytes (see [`ImagePath`]).
const PATH_CAP: usize = 255;

/// A background image path held inline so [`Look`] stays `Copy`. An empty path means no image; a path longer than
/// the cap or not valid UTF-8 is refused (it reads back as no image).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagePath {
    bytes: [u8; PATH_CAP],
    len: u8,
}

impl Default for ImagePath {
    // (an array this long has no derived Default)
    fn default() -> Self {
        ImagePath { bytes: [0; PATH_CAP], len: 0 }
    }
}

impl ImagePath {
    pub fn new(path: &str) -> ImagePath {
        let mut out = ImagePath::default();
        if path.len() <= PATH_CAP {
            out.bytes[..path.len()].copy_from_slice(path.as_bytes());
            out.len = path.len() as u8;
        }
        out
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// How a background image fills the grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    /// covers the grid, cropping the overflow
    #[default]
    Fill,
    /// fits inside the grid, keeping its aspect ratio
    Fit,
    /// repeats at its own size
    Tile,
}

impl ImageMode {
    pub fn key(self) -> &'static str {
        match self {
            ImageMode::Fill => "fill",
            ImageMode::Fit => "fit",
            ImageMode::Tile => "tile",
        }
    }

    /// Reads a stored mode; anything unknown is Fill.
    pub fn from_key(text: &str) -> ImageMode {
        match text {
            "fit" => ImageMode::Fit,
            "tile" => ImageMode::Tile,
            _ => ImageMode::Fill,
        }
    }
}

/// Clamps window opacity to the range; a value that is not finite is fully opaque.
pub fn clamp_opacity(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(OPACITY_MIN, OPACITY_MAX)
    } else {
        OPACITY_MAX
    }
}

/// Clamps the image dim to 0..=DIM_MAX; a value that is not finite is no dim.
pub fn clamp_dim(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, DIM_MAX)
    } else {
        0.0
    }
}

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
    /// Window opacity, OPACITY_MIN..=OPACITY_MAX; below 1.0 the desk shows through the grid's background.
    pub opacity: f32,
    /// Optional background image drawn under the cells (empty for none).
    pub background_image: ImagePath,
    pub image_mode: ImageMode,
    /// How much black the image is dimmed by, 0.0..=DIM_MAX, so the text stays readable.
    pub image_dim: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            ansi: None,
            bold_bright: true,
            allow_blink: true,
            char_spacing: 1.0,
            line_spacing: 1.0,
            opacity: OPACITY_MAX,
            background_image: ImagePath::default(),
            image_mode: ImageMode::Fill,
            image_dim: 0.0,
        }
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
            ("terminal.look.opacity", format!("{:.2}", self.opacity)),
            ("terminal.look.image", self.background_image.as_str().to_string()),
            ("terminal.look.image_mode", self.image_mode.key().to_string()),
            ("terminal.look.image_dim", format!("{:.2}", self.image_dim)),
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
            opacity: get("terminal.look.opacity")
                .and_then(|v| v.parse::<f32>().ok())
                .map_or(d.opacity, clamp_opacity),
            background_image: get("terminal.look.image").map_or(d.background_image, |v| ImagePath::new(&v)),
            image_mode: get("terminal.look.image_mode").map_or(d.image_mode, |v| ImageMode::from_key(&v)),
            image_dim: get("terminal.look.image_dim")
                .and_then(|v| v.parse::<f32>().ok())
                .map_or(d.image_dim, clamp_dim),
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
        let look = Look {
            ansi: Some(ansi),
            bold_bright: false,
            allow_blink: false,
            char_spacing: 1.3,
            line_spacing: 0.9,
            opacity: 0.7,
            background_image: ImagePath::new("/home/user/Pictures/grid.png"),
            image_mode: ImageMode::Tile,
            image_dim: 0.25,
        };
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
    fn opacity_and_dim_are_clamped_and_default_to_opaque_and_clear() {
        assert_eq!(clamp_opacity(0.1), OPACITY_MIN);
        assert_eq!(clamp_opacity(2.0), OPACITY_MAX);
        assert_eq!(clamp_opacity(f32::NAN), OPACITY_MAX);
        assert_eq!(clamp_opacity(0.75), 0.75);
        assert_eq!(clamp_dim(-1.0), 0.0);
        assert_eq!(clamp_dim(0.99), DIM_MAX);
        assert_eq!(clamp_dim(f32::INFINITY), 0.0);
        let look = Look::default();
        assert_eq!(look.opacity, 1.0);
        assert!(look.background_image.is_empty());
        assert_eq!(look.image_mode, ImageMode::Fill);
    }

    #[test]
    fn translucency_and_background_image_persist() {
        let look = Look {
            opacity: 0.6,
            background_image: ImagePath::new("/srv/bg/wall.jpg"),
            image_mode: ImageMode::Fit,
            image_dim: 0.4,
            ..Look::default()
        };
        let store: HashMap<&str, String> = look.saved().into_iter().collect();
        assert_eq!(store["terminal.look.opacity"], "0.60");
        assert_eq!(store["terminal.look.image"], "/srv/bg/wall.jpg");
        assert_eq!(store["terminal.look.image_mode"], "fit");
        let back = Look::from_prefs(|k| store.get(k).cloned());
        assert_eq!(back, look);
        assert_eq!(back.background_image.as_str(), "/srv/bg/wall.jpg");
    }

    #[test]
    fn bad_translucency_values_fall_back() {
        let look = Look::from_prefs(|k| match k {
            "terminal.look.opacity" => Some("0.01".into()),
            "terminal.look.image_mode" => Some("spiral".into()),
            "terminal.look.image_dim" => Some("inf".into()),
            _ => None,
        });
        assert_eq!(look.opacity, OPACITY_MIN);
        assert_eq!(look.image_mode, ImageMode::Fill);
        assert_eq!(look.image_dim, 0.0);
        assert!(look.background_image.is_empty());
        let long = "x".repeat(PATH_CAP + 1);
        assert!(ImagePath::new(&long).is_empty(), "an over-long path is refused");
    }

    #[test]
    fn spacing_steps_land_on_tenths() {
        assert_eq!(clamp_spacing(1.04), 1.0);
        assert_eq!(clamp_spacing(1.16), 1.2);
        assert_eq!(clamp_spacing(0.1), SPACING_MIN);
        assert_eq!(clamp_spacing(1.0 + SPACING_STEP), 1.1);
    }
}
