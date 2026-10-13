//! Text Editor's settings: the options the Preferences menu and the Edit, Format and View menus toggle, kept in the app's
//! preferences so they hold the next time the app starts. Pure: reading takes a lookup and writing gives the pairs, so
//! the rules are tested without a runtime.

use crate::encoding::Encoding;

/// The options, with the defaults a fresh install has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Edit > Spelling and Grammar > Check Spelling While Typing
    pub spell_enabled: bool,
    /// Edit > Spelling and Grammar > Correct Spelling Automatically
    pub spell_auto: bool,
    /// Edit > Substitutions > Smart Quotes and Smart Dashes
    pub smart_quotes: bool,
    pub smart_dashes: bool,
    /// View > Show Ruler, and View > Wrap to Window
    pub show_ruler: bool,
    pub wrap_window: bool,
    /// File > Print > Header and Footer
    pub header_footer: bool,
    /// Whether File > New makes a plain text document
    pub new_plain: bool,
    /// Preferences > Open and Save: the encoding new plain text documents are saved in, and whether RTF and HTML files
    /// open as plain text showing their markup
    pub default_encoding: Encoding,
    pub markup_as_code: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { spell_enabled: true, spell_auto: false, smart_quotes: false, smart_dashes: false, show_ruler: true, wrap_window: false, header_footer: false, new_plain: false, default_encoding: Encoding::Utf8, markup_as_code: false }
    }
}

impl Options {
    /// The options as stored: each key with "1" or "0".
    pub fn entries(&self) -> [(&'static str, &'static str); 10] {
        let v = |on: bool| if on { "1" } else { "0" };
        [
            ("options.spell_enabled", v(self.spell_enabled)),
            ("options.spell_auto", v(self.spell_auto)),
            ("options.smart_quotes", v(self.smart_quotes)),
            ("options.smart_dashes", v(self.smart_dashes)),
            ("options.show_ruler", v(self.show_ruler)),
            ("options.wrap_window", v(self.wrap_window)),
            ("options.header_footer", v(self.header_footer)),
            ("options.new_plain", v(self.new_plain)),
            ("options.default_encoding", self.default_encoding.id()),
            ("options.markup_as_code", v(self.markup_as_code)),
        ]
    }

    /// The options from stored values; a missing or unreadable value keeps its default.
    pub fn read(get: &dyn Fn(&str) -> Option<String>) -> Options {
        let d = Options::default();
        let flag = |key: &str, default: bool| match get(key).as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => default,
        };
        Options {
            spell_enabled: flag("options.spell_enabled", d.spell_enabled),
            spell_auto: flag("options.spell_auto", d.spell_auto),
            smart_quotes: flag("options.smart_quotes", d.smart_quotes),
            smart_dashes: flag("options.smart_dashes", d.smart_dashes),
            show_ruler: flag("options.show_ruler", d.show_ruler),
            wrap_window: flag("options.wrap_window", d.wrap_window),
            header_footer: flag("options.header_footer", d.header_footer),
            new_plain: flag("options.new_plain", d.new_plain),
            default_encoding: get("options.default_encoding").and_then(|id| Encoding::from_id(&id)).unwrap_or(d.default_encoding),
            markup_as_code: flag("options.markup_as_code", d.markup_as_code),
        }
    }
}

/// The zoom levels View > Zoom In and Zoom Out step through.
pub const ZOOMS: [f32; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

/// The next zoom level above `zoom`, or `zoom` itself at the top.
pub fn zoom_in(zoom: f32) -> f32 {
    ZOOMS.iter().copied().find(|&z| z > zoom + 0.001).unwrap_or(zoom)
}

/// The next zoom level below `zoom`, or `zoom` itself at the bottom.
pub fn zoom_out(zoom: f32) -> f32 {
    ZOOMS.iter().rev().copied().find(|&z| z < zoom - 0.001).unwrap_or(zoom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn options_survive_a_write_and_a_read() {
        let on = Options { spell_enabled: false, spell_auto: true, smart_quotes: true, smart_dashes: true, show_ruler: false, wrap_window: true, header_footer: true, new_plain: true, default_encoding: Encoding::Roman, markup_as_code: true };
        let stored: HashMap<String, String> = on.entries().iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(Options::read(&|k| stored.get(k).cloned()), on);
    }

    #[test]
    fn missing_or_garbled_values_keep_their_defaults() {
        assert_eq!(Options::read(&|_| None), Options::default());
        assert_eq!(Options::read(&|_| Some("maybe".into())), Options::default());
        let only_ruler_off = Options::read(&|k| (k == "options.show_ruler").then(|| "0".to_string()));
        assert!(!only_ruler_off.show_ruler && only_ruler_off.spell_enabled);
        let unknown_encoding = Options::read(&|k| (k == "options.default_encoding").then(|| "klingon".to_string()));
        assert_eq!(unknown_encoding.default_encoding, Encoding::Utf8);
    }

    #[test]
    fn zoom_steps_through_the_levels_and_stops_at_the_ends() {
        assert_eq!(zoom_in(1.0), 1.25);
        assert_eq!(zoom_out(1.0), 0.75);
        assert_eq!(zoom_in(4.0), 4.0);
        assert_eq!(zoom_out(0.5), 0.5);
        assert_eq!(zoom_in(1.1), 1.25, "a level between steps goes to the next one");
    }
}
