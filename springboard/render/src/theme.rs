//! Theme definitions: the named colours of [`crate::ui`] can be given in a theme file, read at startup.
//!
//! A theme file is TOML, in the subset a colour table needs: `#` comments, a `[colors]` table, and lines of the form
//! `NAME = "#rrggbb"` or `NAME = "#rrggbbaa"` (the alpha defaults to 255). `NAME` is a constant name from
//! [`builtin`], such as `TINT` or `SYSTEM_RED`. Unknown names, other tables, and values that are not a hex colour are
//! ignored, so a theme written for a newer build still loads, and a missing or unreadable file gives the built-in
//! colours. The parser is a small reader, not a general TOML parser: arrays, inline tables and multi-line strings are
//! not accepted and their lines are skipped.
//!
//! The dark and light colours of a [`Pair`](crate::appearance::Pair) constant (`LABEL`, `CARD_BG`, `SYSTEM_RED`, ...,
//! the list is [`builtin_pairs`]) are set with `NAME.dark = "#..."` and `NAME.light = "#..."`; `NAME = "#..."` sets
//! both. Once [`load_at_startup`] has read a file, `Pair::now` returns the file's colours for every pair it changes
//! ([`swapped`]); with no file, or a file that names none, every pair keeps its built-in colours. A pair is found by
//! its built-in colours, so two constants that share the same built-in dark and light values change together.

use crate::appearance::Pair;
use crate::gfx::pack;
use crate::ui;
use std::collections::BTreeMap;
use std::path::Path;

/// The named colours, with the built-in value of each.
pub fn builtin() -> Vec<(&'static str, u32)> {
    vec![
        ("WHITE", ui::WHITE),
        ("BLACK", ui::BLACK),
        ("SECONDARY", ui::SECONDARY),
        ("CELL", ui::CELL),
        ("TINT", ui::TINT),
        ("SEPARATOR", ui::SEPARATOR),
        ("TRACK", ui::TRACK),
        ("TOAST_FILL", ui::TOAST_FILL),
        ("SYSTEM_RED", ui::SYSTEM_RED),
        ("SYSTEM_ORANGE", ui::SYSTEM_ORANGE),
        ("SYSTEM_YELLOW", ui::SYSTEM_YELLOW),
        ("SYSTEM_GREEN", ui::SYSTEM_GREEN),
        ("SYSTEM_PURPLE", ui::SYSTEM_PURPLE),
    ]
}

/// The named dark/light colours of [`crate::ui`], with the built-in value of each.
pub fn builtin_pairs() -> Vec<(&'static str, Pair)> {
    vec![
        ("BAR_TINT", ui::BAR_TINT),
        ("MENU_TINT", ui::MENU_TINT),
        ("MENU_SOLID", ui::MENU_SOLID),
        ("DOCK_TINT", ui::DOCK_TINT),
        ("SIDEBAR_BG", ui::SIDEBAR_BG),
        ("DETAIL_BG", ui::DETAIL_BG),
        ("CONTENT_BG", ui::CONTENT_BG),
        ("CARD_BG", ui::CARD_BG),
        ("SEPARATOR_C", ui::SEPARATOR_C),
        ("LABEL", ui::LABEL),
        ("LABEL_2", ui::LABEL_2),
        ("SELECTION", ui::SELECTION),
        ("ROW_PRESSED", ui::ROW_PRESSED),
        ("FIELD_BG", ui::FIELD_BG),
        ("LABEL_3", ui::LABEL_3),
        ("SIDEBAR_PLATE", ui::SIDEBAR_PLATE),
        ("CAPSULE_BG", ui::CAPSULE_BG),
        ("CAPSULE_EDGE", ui::CAPSULE_EDGE),
        ("CAPSULE_SHADOW", ui::CAPSULE_SHADOW),
        ("SEGMENT_ON", ui::SEGMENT_ON),
        ("CONTROL_PRESSED", ui::CONTROL_PRESSED),
        ("HAIRLINE_C", ui::HAIRLINE_C),
        ("MATERIAL_SIDEBAR", ui::MATERIAL_SIDEBAR),
        ("MATERIAL_TITLEBAR", ui::MATERIAL_TITLEBAR),
        ("MATERIAL_POPOVER", ui::MATERIAL_POPOVER),
        ("MATERIAL_HUD", ui::MATERIAL_HUD),
        ("MATERIAL_WINDOW", ui::MATERIAL_WINDOW),
        ("MATERIAL_SELECTION", ui::MATERIAL_SELECTION),
        ("MATERIAL_MENU", ui::MATERIAL_MENU),
        ("MATERIAL_SHEET", ui::MATERIAL_SHEET),
        ("MATERIAL_TOOLTIP", ui::MATERIAL_TOOLTIP),
        ("MATERIAL_HEADER", ui::MATERIAL_HEADER),
        ("MATERIAL_CONTENT", ui::MATERIAL_CONTENT),
        ("MATERIAL_UNDER_WINDOW", ui::MATERIAL_UNDER_WINDOW),
        ("MATERIAL_UNDER_PAGE", ui::MATERIAL_UNDER_PAGE),
        ("MATERIAL_FULLSCREEN", ui::MATERIAL_FULLSCREEN),
        ("MATERIAL_SELECTION_EMPHASIZED", ui::MATERIAL_SELECTION_EMPHASIZED),
        ("MATERIAL_SELECTION_INACTIVE", ui::MATERIAL_SELECTION_INACTIVE),
        ("MATERIAL_SIDEBAR_SOLID", ui::MATERIAL_SIDEBAR_SOLID),
        ("MATERIAL_TITLEBAR_SOLID", ui::MATERIAL_TITLEBAR_SOLID),
        ("MATERIAL_HUD_SOLID", ui::MATERIAL_HUD_SOLID),
        ("MATERIAL_BORDER_CONTRAST", ui::MATERIAL_BORDER_CONTRAST),
        ("SEPARATOR_CONTRAST", ui::SEPARATOR_CONTRAST),
        ("WINDOW_FRAME_FOCUSED", ui::WINDOW_FRAME_FOCUSED),
        ("WINDOW_FRAME_BLURRED", ui::WINDOW_FRAME_BLURRED),
        ("TRACK_C", ui::TRACK_C),
        ("TRAFFIC_INACTIVE_C", ui::TRAFFIC_INACTIVE_C),
        ("QUATERNARY_LABEL", ui::system::QUATERNARY_LABEL),
        ("GRID", ui::system::GRID),
        ("LINK", ui::system::LINK),
        ("SYSTEM_RED", ui::system::SYSTEM_RED),
        ("SYSTEM_ORANGE", ui::system::SYSTEM_ORANGE),
        ("SYSTEM_YELLOW", ui::system::SYSTEM_YELLOW),
        ("SYSTEM_GREEN", ui::system::SYSTEM_GREEN),
        ("SYSTEM_MINT", ui::system::SYSTEM_MINT),
        ("SYSTEM_TEAL", ui::system::SYSTEM_TEAL),
        ("SYSTEM_CYAN", ui::system::SYSTEM_CYAN),
        ("SYSTEM_BLUE", ui::system::SYSTEM_BLUE),
        ("SYSTEM_INDIGO", ui::system::SYSTEM_INDIGO),
        ("SYSTEM_PURPLE", ui::system::SYSTEM_PURPLE),
        ("SYSTEM_PINK", ui::system::SYSTEM_PINK),
        ("SYSTEM_BROWN", ui::system::SYSTEM_BROWN),
        ("SYSTEM_GRAY", ui::system::SYSTEM_GRAY),
        ("FIND_HIGHLIGHT", ui::system::FIND_HIGHLIGHT),
        ("SHADOW", ui::system::SHADOW),
    ]
}

/// The colours in force: the built-in values with the theme file's overrides applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    colours: BTreeMap<String, u32>,
    /// the (dark, light) colours of each named pair
    pairs: BTreeMap<String, (u32, u32)>,
    /// built-in (dark, light) -> the file's (dark, light), for the pairs the file changes
    swaps: Vec<((u32, u32), (u32, u32))>,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            colours: builtin().into_iter().map(|(name, c)| (name.to_string(), c)).collect(),
            pairs: builtin_pairs().into_iter().map(|(name, p)| (name.to_string(), (p.dark, p.light))).collect(),
            swaps: Vec::new(),
        }
    }
}

impl Theme {
    /// The colour of a named constant, when the name is one of [`builtin`].
    pub fn colour(&self, name: &str) -> Option<u32> {
        self.colours.get(name).copied()
    }

    /// The dark and light colours of a named pair, when the name is one of [`builtin_pairs`].
    pub fn pair(&self, name: &str) -> Option<(u32, u32)> {
        self.pairs.get(name).copied()
    }

    /// The colours a pair with these built-in dark and light values takes in this theme; `None` when the theme leaves
    /// it as it is.
    pub fn swap(&self, dark: u32, light: u32) -> Option<(u32, u32)> {
        self.swaps.iter().find(|(from, _)| *from == (dark, light)).map(|(_, to)| *to)
    }

    /// Reads a theme from `text`. Starts from the built-in colours and replaces each `[colors]` entry that names one of
    /// them with a valid hex colour.
    pub fn parse(text: &str) -> Theme {
        let mut theme = Theme::default();
        let mut in_colors = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(table) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                in_colors = table.trim() == "colors";
                continue;
            }
            if !in_colors {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim();
            let Some(colour) = quoted(value.trim()).and_then(hex_colour) else { continue };
            let (name, side) = match key.split_once('.') {
                Some((name, side)) => (name.trim(), Some(side.trim())),
                None => (key, None),
            };
            match side {
                None => {
                    if let Some(slot) = theme.colours.get_mut(name) {
                        *slot = colour;
                    }
                    if let Some(slot) = theme.pairs.get_mut(name) {
                        *slot = (colour, colour);
                    }
                }
                Some("dark") => {
                    if let Some(slot) = theme.pairs.get_mut(name) {
                        slot.0 = colour;
                    }
                }
                Some("light") => {
                    if let Some(slot) = theme.pairs.get_mut(name) {
                        slot.1 = colour;
                    }
                }
                Some(_) => {}
            }
        }
        theme.swaps = builtin_pairs()
            .into_iter()
            .filter_map(|(name, built)| {
                let now = theme.pairs.get(name).copied()?;
                (now != (built.dark, built.light)).then_some(((built.dark, built.light), now))
            })
            .collect();
        theme
    }

    /// Reads the theme file at `path`. A missing or unreadable file gives the built-in colours.
    pub fn load(path: &Path) -> Theme {
        match std::fs::read_to_string(path) {
            Ok(text) => Theme::parse(&text),
            Err(_) => Theme::default(),
        }
    }
}

/// The theme file read at startup, when `BIFROST_THEME` is not set.
pub const DEFAULT_FILE: &str = "/usr/share/bifrost/theme.toml";

static CURRENT: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();

/// Reads the theme at startup: the file named by `BIFROST_THEME`, else [`DEFAULT_FILE`]. Later calls keep the first
/// result. Returns the theme in force.
pub fn load_at_startup() -> &'static Theme {
    CURRENT.get_or_init(|| {
        let path = std::env::var_os("BIFROST_THEME").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(DEFAULT_FILE).to_path_buf());
        Theme::load(&path)
    })
}

/// The theme in force, or the built-in colours when [`load_at_startup`] has not run.
pub fn current() -> &'static Theme {
    CURRENT.get_or_init(Theme::default)
}

/// The colours `Pair::now` uses for a pair whose built-in dark and light values are these, when the theme read at
/// startup changes it. Does not read the theme itself: before [`load_at_startup`], and with no file, it is `None`.
pub fn swapped(dark: u32, light: u32) -> Option<(u32, u32)> {
    CURRENT.get().and_then(|t| t.swap(dark, light))
}

/// The text inside a double-quoted TOML string, or `None` when the value is not one. A `#` inside the quotes is part
/// of the string; anything after the closing quote (such as a comment) is ignored.
fn quoted(value: &str) -> Option<&str> {
    let inner = value.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(&inner[..end])
}

/// `#rrggbb` or `#rrggbbaa` as a packed colour.
fn hex_colour(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        6 => Some(pack(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some(pack(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_colours_are_the_ui_constants() {
        let t = Theme::default();
        assert_eq!(t.colour("TINT"), Some(ui::TINT));
        assert_eq!(t.colour("SYSTEM_RED"), Some(ui::SYSTEM_RED));
        assert_eq!(t.colour("NOT_A_COLOUR"), None);
    }

    #[test]
    fn a_colour_table_overrides_the_named_colours() {
        let t = Theme::parse("[colors]\nTINT = \"#ff8800\"\nSYSTEM_RED = \"#11223344\"\n");
        assert_eq!(t.colour("TINT"), Some(pack(255, 136, 0, 255)));
        assert_eq!(t.colour("SYSTEM_RED"), Some(pack(0x11, 0x22, 0x33, 0x44)));
        assert_eq!(t.colour("SYSTEM_GREEN"), Some(ui::SYSTEM_GREEN), "a colour the file leaves out keeps its value");
    }

    #[test]
    fn unknown_keys_tables_and_bad_values_are_ignored() {
        let text = "# theme for the lab\n[colors]\nFUTURE_COLOUR = \"#010203\"\nTINT = \"blue\"\nSYSTEM_RED = \"#12345\"\n\
                    SYSTEM_GREEN = 12\n[window]\nTINT = \"#000000\"\n";
        let t = Theme::parse(text);
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn a_comment_after_a_value_does_not_change_it() {
        let t = Theme::parse("[colors]\nTINT = \"#00ff00\" # green\n");
        assert_eq!(t.colour("TINT"), Some(pack(0, 255, 0, 255)));
    }

    #[test]
    fn a_missing_file_gives_the_built_in_colours() {
        let t = Theme::load(Path::new("/nonexistent/bifrost-theme.toml"));
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn every_pair_name_is_listed_once_and_defaults_to_its_built_in_colours() {
        let names: Vec<&str> = builtin_pairs().iter().map(|p| p.0).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "a name appears twice");
        let t = Theme::default();
        assert_eq!(t.pair("CARD_BG"), Some((ui::CARD_BG.dark, ui::CARD_BG.light)));
        assert!(t.swaps.is_empty() && t.swap(ui::CARD_BG.dark, ui::CARD_BG.light).is_none());
    }

    #[test]
    fn a_pair_takes_its_dark_and_light_colours_from_the_file() {
        let t = Theme::parse("[colors]\nCARD_BG.dark = \"#102030\"\nCARD_BG.light = \"#eeeeee\"\nLABEL = \"#445566\"\nSIDEBAR_BG.dark = \"#000001\"\n");
        assert_eq!(t.pair("CARD_BG"), Some((pack(0x10, 0x20, 0x30, 255), pack(0xee, 0xee, 0xee, 255))));
        assert_eq!(t.pair("LABEL"), Some((pack(0x44, 0x55, 0x66, 255), pack(0x44, 0x55, 0x66, 255))), "a plain name sets both");
        assert_eq!(t.pair("SIDEBAR_BG"), Some((pack(0, 0, 1, 255), ui::SIDEBAR_BG.light)), "one side leaves the other");
        assert_eq!(t.pair("DETAIL_BG"), Some((ui::DETAIL_BG.dark, ui::DETAIL_BG.light)), "a pair the file leaves out keeps its colours");
        assert_eq!(t.swap(ui::CARD_BG.dark, ui::CARD_BG.light), Some((pack(0x10, 0x20, 0x30, 255), pack(0xee, 0xee, 0xee, 255))));
        assert_eq!(t.swap(ui::DETAIL_BG.dark, ui::DETAIL_BG.light), None);
    }

    #[test]
    fn a_plain_constant_name_still_sets_the_plain_colour_and_the_pair_of_that_name() {
        let t = Theme::parse("[colors]\nSYSTEM_RED = \"#ff0000\"\n");
        assert_eq!(t.colour("SYSTEM_RED"), Some(pack(255, 0, 0, 255)));
        assert_eq!(t.pair("SYSTEM_RED"), Some((pack(255, 0, 0, 255), pack(255, 0, 0, 255))));
    }

    #[test]
    fn a_pair_resolves_to_the_files_colour() {
        // both sides the same, so the answer does not depend on the Light/Dark position of the running test binary
        let t = Theme::parse("[colors]\nCARD_BG = \"#336699\"\n");
        assert_eq!(ui::CARD_BG.now_in(&t), pack(0x33, 0x66, 0x99, 255));
        assert_eq!(t.swap(ui::DETAIL_BG.dark, ui::DETAIL_BG.light), None, "an untouched pair resolves as before");
    }

    #[test]
    fn bad_pair_keys_and_values_change_nothing() {
        let t = Theme::parse("[colors]\nCARD_BG.middle = \"#102030\"\nCARD_BG.dark = \"blue\"\nNOT_A_PAIR.dark = \"#102030\"\n");
        assert_eq!(t, Theme::default());
    }
}
