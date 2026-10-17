//! Theme definitions: the named colours of [`crate::ui`] can be given in a theme file, read at startup.
//!
//! A theme file is TOML, in the subset a colour table needs: `#` comments, a `[colors]` table, and lines of the form
//! `NAME = "#rrggbb"` or `NAME = "#rrggbbaa"` (the alpha defaults to 255). `NAME` is a constant name from
//! [`builtin`], such as `TINT` or `SYSTEM_RED`. Unknown names, other tables, and values that are not a hex colour are
//! ignored, so a theme written for a newer build still loads, and a missing or unreadable file gives the built-in
//! colours. The parser is a small reader, not a general TOML parser: arrays, inline tables and multi-line strings are
//! not accepted and their lines are skipped.

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

/// The colours in force: the built-in values with the theme file's overrides applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    colours: BTreeMap<String, u32>,
}

impl Default for Theme {
    fn default() -> Self {
        Theme { colours: builtin().into_iter().map(|(name, c)| (name.to_string(), c)).collect() }
    }
}

impl Theme {
    /// The colour of a named constant, when the name is one of [`builtin`].
    pub fn colour(&self, name: &str) -> Option<u32> {
        self.colours.get(name).copied()
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
            if let Some(slot) = theme.colours.get_mut(key) {
                *slot = colour;
            }
        }
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
}
