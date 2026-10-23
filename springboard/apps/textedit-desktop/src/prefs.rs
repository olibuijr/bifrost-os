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
    /// Edit > Substitutions > Smart Links: a web or mail address becomes a link when the word ends
    pub smart_links: bool,
    /// Edit > Substitutions > Text Replacement: a fixed set of shortcuts (such as (c) and ...) becomes its symbol
    pub text_replacement: bool,
    /// View > Show Ruler, and View > Wrap to Window
    pub show_ruler: bool,
    pub wrap_window: bool,
    /// View > Use Dark Background for Windows: the page is dark and its text light
    pub dark_background: bool,
    /// File > Print > Header and Footer
    pub header_footer: bool,
    /// File > Print > Rewrap to Fit Page: the printed lines are wrapped to the page, not kept at the window's width
    pub rewrap: bool,
    /// Whether File > New makes a plain text document
    pub new_plain: bool,
    /// Preferences: the font family of new rich text (an index into `text::families`, every installed family; stored by
    /// its name, so the choice holds when the installed families change), and the encoding new plain text documents are
    /// saved in
    pub default_font: u8,
    pub default_encoding: Encoding,
    /// Preferences > Open and Save: whether RTF and HTML files open as plain text showing their markup
    pub markup_as_code: bool,
    /// Preferences > Customize Encodings List: one bit per encoding in `encoding::ALL`, set when the encoding is listed in
    /// the Format menu's Text Encoding
    pub encodings: u8,
}

/// The bits of `Options::encodings` when every encoding is listed.
pub const ALL_ENCODINGS: u8 = (1 << crate::encoding::ALL.len()) - 1;

impl Default for Options {
    fn default() -> Self {
        Options {
            spell_enabled: true,
            spell_auto: false,
            smart_quotes: false,
            smart_dashes: false,
            smart_links: false,
            text_replacement: false,
            show_ruler: true,
            wrap_window: false,
            dark_background: false,
            header_footer: false,
            rewrap: true,
            new_plain: false,
            default_font: 0,
            default_encoding: Encoding::Utf8,
            markup_as_code: false,
            encodings: ALL_ENCODINGS,
        }
    }
}

impl Options {
    /// The options as stored: each key with "1" or "0", or the value of the choice.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        let v = |on: bool| if on { "1".to_string() } else { "0".to_string() };
        vec![
            ("options.spell_enabled", v(self.spell_enabled)),
            ("options.spell_auto", v(self.spell_auto)),
            ("options.smart_quotes", v(self.smart_quotes)),
            ("options.smart_dashes", v(self.smart_dashes)),
            ("options.smart_links", v(self.smart_links)),
            ("options.text_replacement", v(self.text_replacement)),
            ("options.show_ruler", v(self.show_ruler)),
            ("options.wrap_window", v(self.wrap_window)),
            ("options.dark_background", v(self.dark_background)),
            ("options.header_footer", v(self.header_footer)),
            ("options.rewrap", v(self.rewrap)),
            ("options.new_plain", v(self.new_plain)),
            ("options.default_font", default_font_name(self.default_font).to_string()),
            ("options.default_encoding", self.default_encoding.id().to_string()),
            ("options.markup_as_code", v(self.markup_as_code)),
            ("options.encodings", self.encodings.to_string()),
        ]
    }

    /// Whether the encoding at `index` in `encoding::ALL` is listed in the Text Encoding menu.
    pub fn lists_encoding(&self, index: usize) -> bool {
        index < 8 && self.encodings & (1 << index) != 0
    }

    /// The list with the encoding at `index` listed or not as `on`.
    pub fn with_encoding(&self, index: usize, on: bool) -> u8 {
        if on {
            self.encodings | (1 << index)
        } else {
            self.encodings & !(1 << index)
        }
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
            smart_links: flag("options.smart_links", d.smart_links),
            text_replacement: flag("options.text_replacement", d.text_replacement),
            show_ruler: flag("options.show_ruler", d.show_ruler),
            wrap_window: flag("options.wrap_window", d.wrap_window),
            dark_background: flag("options.dark_background", d.dark_background),
            header_footer: flag("options.header_footer", d.header_footer),
            rewrap: flag("options.rewrap", d.rewrap),
            new_plain: flag("options.new_plain", d.new_plain),
            default_font: get("options.default_font").and_then(|v| default_font_of(&v)).unwrap_or(d.default_font),
            default_encoding: get("options.default_encoding").and_then(|id| Encoding::from_id(&id)).unwrap_or(d.default_encoding),
            markup_as_code: flag("options.markup_as_code", d.markup_as_code),
            encodings: get("options.encodings").and_then(|n| n.parse::<u8>().ok()).map(|n| n & ALL_ENCODINGS).unwrap_or(d.encodings),
        }
    }
}

/// The name the default font `font` is stored under: its family's name (the default family for an index past them).
fn default_font_name(font: u8) -> &'static str {
    crate::text::families().get(font as usize).map_or(crate::text::FAMILIES[0], String::as_str)
}

/// The family a stored default font names: a family name (any case), as it is written now, or the index into the three
/// base families that earlier versions wrote. None for a family that is not installed (the default then holds).
fn default_font_of(stored: &str) -> Option<u8> {
    if let Ok(index) = stored.trim().parse::<u8>() {
        return ((index as usize) < crate::text::FAMILIES.len()).then_some(index);
    }
    crate::text::family_index(stored)
}

/// The settings the Preferences panel lists, in the order it shows them. Each one is a toggle or a choice that cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    NewPlain,
    DefaultFont,
    DefaultEncoding,
    MarkupAsCode,
    SpellEnabled,
    SpellAuto,
    SmartQuotes,
    SmartDashes,
    SmartLinks,
    TextReplacement,
    ShowRuler,
    WrapWindow,
    HeaderFooter,
    Rewrap,
    DarkBackground,
}

impl Setting {
    pub const ALL: [Setting; 15] = [
        Setting::NewPlain,
        Setting::DefaultFont,
        Setting::DefaultEncoding,
        Setting::MarkupAsCode,
        Setting::SpellEnabled,
        Setting::SpellAuto,
        Setting::SmartQuotes,
        Setting::SmartDashes,
        Setting::SmartLinks,
        Setting::TextReplacement,
        Setting::ShowRuler,
        Setting::WrapWindow,
        Setting::HeaderFooter,
        Setting::Rewrap,
        Setting::DarkBackground,
    ];

    /// The name of the setting in the panel.
    pub fn label(self) -> &'static str {
        match self {
            Setting::NewPlain => "New Documents",
            Setting::DefaultFont => "Default Font",
            Setting::DefaultEncoding => "Plain Text Encoding",
            Setting::MarkupAsCode => "Show RTF and HTML Files as Code",
            Setting::SpellEnabled => "Check Spelling While Typing",
            Setting::SpellAuto => "Correct Spelling Automatically",
            Setting::SmartQuotes => "Smart Quotes",
            Setting::SmartDashes => "Smart Dashes",
            Setting::SmartLinks => "Smart Links",
            Setting::TextReplacement => "Text Replacement",
            Setting::ShowRuler => "Show Ruler",
            Setting::WrapWindow => "Wrap to Window",
            Setting::HeaderFooter => "Print Header and Footer",
            Setting::Rewrap => "Rewrap to Fit Page",
            Setting::DarkBackground => "Use Dark Background for Windows",
        }
    }

    /// The value shown beside the name: the choice in effect, or On or Off.
    pub fn value(self, o: &Options) -> String {
        let on = |b: bool| (if b { "On" } else { "Off" }).to_string();
        match self {
            Setting::NewPlain => (if o.new_plain { "Plain Text" } else { "Rich Text" }).to_string(),
            Setting::DefaultFont => default_font_name(o.default_font).to_string(),
            Setting::DefaultEncoding => o.default_encoding.name().to_string(),
            Setting::MarkupAsCode => on(o.markup_as_code),
            Setting::SpellEnabled => on(o.spell_enabled),
            Setting::SpellAuto => on(o.spell_auto),
            Setting::SmartQuotes => on(o.smart_quotes),
            Setting::SmartDashes => on(o.smart_dashes),
            Setting::SmartLinks => on(o.smart_links),
            Setting::TextReplacement => on(o.text_replacement),
            Setting::ShowRuler => on(o.show_ruler),
            Setting::WrapWindow => on(o.wrap_window),
            Setting::HeaderFooter => on(o.header_footer),
            Setting::Rewrap => on(o.rewrap),
            Setting::DarkBackground => on(o.dark_background),
        }
    }

    /// Chooses the next value: a toggle flips, a choice moves to the next one in its list (and wraps).
    pub fn change(self, o: &mut Options) {
        match self {
            Setting::NewPlain => o.new_plain = !o.new_plain,
            // every installed family in turn (Preferences > Default Font lists them all)
            Setting::DefaultFont => o.default_font = ((o.default_font as usize + 1) % crate::text::families().len().clamp(1, 256)) as u8,
            Setting::DefaultEncoding => {
                let all = crate::encoding::ALL;
                let at = all.iter().position(|e| *e == o.default_encoding).unwrap_or(0);
                o.default_encoding = all[(at + 1) % all.len()];
            }
            Setting::MarkupAsCode => o.markup_as_code = !o.markup_as_code,
            Setting::SpellEnabled => o.spell_enabled = !o.spell_enabled,
            Setting::SpellAuto => o.spell_auto = !o.spell_auto,
            Setting::SmartQuotes => o.smart_quotes = !o.smart_quotes,
            Setting::SmartDashes => o.smart_dashes = !o.smart_dashes,
            Setting::SmartLinks => o.smart_links = !o.smart_links,
            Setting::TextReplacement => o.text_replacement = !o.text_replacement,
            Setting::ShowRuler => o.show_ruler = !o.show_ruler,
            Setting::WrapWindow => o.wrap_window = !o.wrap_window,
            Setting::HeaderFooter => o.header_footer = !o.header_footer,
            Setting::Rewrap => o.rewrap = !o.rewrap,
            Setting::DarkBackground => o.dark_background = !o.dark_background,
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
        let on = Options {
            spell_enabled: false,
            spell_auto: true,
            smart_quotes: true,
            smart_dashes: true,
            smart_links: true,
            text_replacement: true,
            show_ruler: false,
            wrap_window: true,
            dark_background: true,
            header_footer: true,
            rewrap: false,
            new_plain: true,
            default_font: 2,
            default_encoding: Encoding::Roman,
            markup_as_code: true,
            encodings: 0b0010_0101,
        };
        let stored: HashMap<String, String> = on.entries().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
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
        let no_such_font = Options::read(&|k| (k == "options.default_font").then(|| "9".to_string()));
        assert_eq!(no_such_font.default_font, 0);
        assert!(Options::default().rewrap, "printing rewraps to the page by default");
        assert_eq!(Options::default().encodings, ALL_ENCODINGS, "every encoding is listed by default");
    }

    /// The default font is stored by family name; the index earlier versions stored still reads as its base family.
    #[test]
    fn the_default_font_is_stored_by_name_and_an_old_index_still_reads() {
        let courier = Options { default_font: 2, ..Options::default() };
        let stored: HashMap<String, String> = courier.entries().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(stored.get("options.default_font").map(String::as_str), Some("Courier"), "the name, not the index");
        let read = |v: &str| Options::read(&|k| (k == "options.default_font").then(|| v.to_string())).default_font;
        assert_eq!(read("Courier"), 2);
        assert_eq!(read("times"), 1, "any case");
        assert_eq!(read("1"), 1, "the index an earlier version wrote");
        assert_eq!(read("7"), 0, "an old index past the base families is the default");
        assert_eq!(read("No Such Family"), 0, "a family that is not installed is the default");
        assert_eq!(Setting::DefaultFont.value(&courier), "Courier");
    }

    #[test]
    fn the_encoding_list_is_a_bit_per_encoding() {
        let o = Options { encodings: 0b0000_0101, ..Options::default() };
        assert!(o.lists_encoding(0) && !o.lists_encoding(1) && o.lists_encoding(2));
        assert_eq!(o.with_encoding(1, true), 0b0000_0111);
        assert_eq!(o.with_encoding(0, false), 0b0000_0100);
        assert_eq!(Options::read(&|k| (k == "options.encodings").then(|| "255".to_string())).encodings, ALL_ENCODINGS, "bits past the list are dropped");
    }

    #[test]
    fn every_setting_has_a_value_and_a_change_that_comes_back() {
        let start = Options::default();
        for setting in Setting::ALL {
            assert!(!setting.label().is_empty());
            let mut o = start;
            setting.change(&mut o);
            assert_ne!(o, start, "{:?} changes the options", setting);
            assert_ne!(setting.value(&o), setting.value(&start), "{:?} shows the new value", setting);
        }
        // a toggle comes back after two changes
        let mut o = start;
        Setting::Rewrap.change(&mut o);
        Setting::Rewrap.change(&mut o);
        assert_eq!(o, start);
        assert_eq!(Setting::Rewrap.value(&start), "On");
        // a choice steps through its list and wraps
        let mut f = start;
        for _ in 0..crate::text::FAMILIES.len() {
            Setting::DefaultFont.change(&mut f);
        }
        assert_eq!(f.default_font, start.default_font);
        let mut e = start;
        for _ in 0..crate::encoding::ALL.len() {
            Setting::DefaultEncoding.change(&mut e);
        }
        assert_eq!(e.default_encoding, start.default_encoding);
        assert_eq!(Setting::NewPlain.value(&start), "Rich Text");
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
