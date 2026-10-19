//! The system symbol set the shell draws, resolved from the openly licensed symbol fonts BifrOSt packs. No vendor
//! symbol font is ever loaded: `FONT_FILES` lists `Symbols.ttf` (Noto Sans Symbols 2, SIL OFL 1.1) and nothing
//! else. Each symbol is looked up by its glyph name first (only a CFF face carries glyph names; `Symbols.ttf` has
//! none), then by Unicode, and callers fall back to hand-drawn shapes (`sdf_glyphs`, the SDK glyph set) when neither
//! has it (Wi-Fi, battery and the gear).

use crate::font::{Font, Style};
use crate::gfx::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    Wifi,
    Battery,
    Bolt,
    Lock,
    ChevronLeft,
    ChevronRight,
    Checkmark,
    Gear,
    /// The outlined gear Golden Gate menus put before Settings….
    GearOutline,
    Airplane,
    SunMin,
    SunMax,
    Speaker,
    Keyboard,
    DeleteLeft,
    Shift,
    Capslock,
    Return,
    Magnifyingglass,
    Xmark,
    Play,
    Pause,
}

impl Symbol {
    pub const ALL: [Symbol; 22] = [
        Symbol::Wifi,
        Symbol::Battery,
        Symbol::Bolt,
        Symbol::Lock,
        Symbol::ChevronLeft,
        Symbol::ChevronRight,
        Symbol::Checkmark,
        Symbol::Gear,
        Symbol::GearOutline,
        Symbol::Airplane,
        Symbol::SunMin,
        Symbol::SunMax,
        Symbol::Speaker,
        Symbol::Keyboard,
        Symbol::DeleteLeft,
        Symbol::Shift,
        Symbol::Capslock,
        Symbol::Return,
        Symbol::Magnifyingglass,
        Symbol::Xmark,
        Symbol::Play,
        Symbol::Pause,
    ];

    /// The symbol's name, which is also its glyph name in a font that carries glyph names.
    pub fn name(self) -> &'static str {
        match self {
            Symbol::Wifi => "wifi",
            Symbol::Battery => "battery.100",
            Symbol::Bolt => "bolt.fill",
            Symbol::Lock => "lock.fill",
            Symbol::ChevronLeft => "chevron.left",
            Symbol::ChevronRight => "chevron.right",
            Symbol::Checkmark => "checkmark",
            Symbol::Gear => "gearshape.fill",
            Symbol::GearOutline => "gearshape",
            Symbol::Airplane => "airplane",
            Symbol::SunMin => "sun.min",
            Symbol::SunMax => "sun.max",
            Symbol::Speaker => "speaker.wave.2.fill",
            Symbol::Keyboard => "keyboard",
            Symbol::DeleteLeft => "delete.left",
            Symbol::Shift => "shift",
            Symbol::Capslock => "capslock",
            Symbol::Return => "return",
            Symbol::Magnifyingglass => "magnifyingglass",
            Symbol::Xmark => "xmark",
            Symbol::Play => "play.fill",
            Symbol::Pause => "pause.fill",
        }
    }

    /// The closest Unicode character, for fonts that map symbols by code point.
    pub fn unicode(self) -> Option<char> {
        Some(match self {
            Symbol::Bolt => '\u{26A1}',
            Symbol::Lock => '\u{1F512}',
            Symbol::ChevronLeft => '\u{2039}',
            Symbol::ChevronRight => '\u{203A}',
            Symbol::Checkmark => '\u{2713}',
            Symbol::Gear | Symbol::GearOutline => '\u{2699}',
            Symbol::Airplane => '\u{2708}',
            Symbol::SunMin => '\u{263C}',
            Symbol::SunMax => '\u{2600}',
            Symbol::Speaker => '\u{1F50A}',
            Symbol::Keyboard => '\u{2328}',
            Symbol::DeleteLeft => '\u{232B}',
            Symbol::Shift => '\u{21E7}',
            Symbol::Capslock => '\u{21EA}',
            Symbol::Return => '\u{23CE}',
            Symbol::Magnifyingglass => '\u{1F50D}',
            Symbol::Xmark => '\u{2715}',
            Symbol::Play => '\u{25B6}',
            Symbol::Pause => '\u{23F8}',
            Symbol::Wifi | Symbol::Battery => return None,
        })
    }
}

/// Symbol fonts in priority order (`FONT_FILES`).
#[derive(Default)]
pub struct Symbols {
    fonts: Vec<Font<'static>>,
}

/// Asset file names tried by `Symbols::load`, best first. Every entry is an openly licensed face credited in
/// `CREDITS.md`; a vendor symbol font dropped into the assets folder is never read.
pub const FONT_FILES: [&str; 1] = ["Symbols.ttf"];

/// The font file mapped (shared page cache) where the platform has mmap, else read and leaked.
fn read_static(path: &str) -> Option<&'static [u8]> {
    #[cfg(target_os = "linux")]
    if let Ok(d) = crate::assets::map_file(path) {
        return Some(d);
    }
    std::fs::read(path).ok().map(|d| &*Box::leak(d.into_boxed_slice()))
}

impl Symbols {
    pub fn new(fonts: Vec<Font<'static>>) -> Self {
        Symbols { fonts }
    }

    /// Loads whichever of `FONT_FILES` exist in `dir`; missing files are skipped.
    pub fn load(dir: &str) -> Self {
        let fonts = FONT_FILES
            .iter()
            .filter_map(|f| {
                let data = read_static(&format!("{dir}/{f}"))?;
                Font::parse(data)
            })
            .collect();
        Symbols { fonts }
    }

    /// The first font that has the symbol, by glyph name then by Unicode.
    pub fn glyph(&self, sym: Symbol) -> Option<(&Font<'static>, u16)> {
        let by_name = self.fonts.iter().find_map(|f| f.glyph_named(sym.name()).map(|g| (f, g)));
        by_name.or_else(|| {
            let c = sym.unicode()?;
            self.fonts.iter().find_map(|f| Some(f.glyph(c)).filter(|&g| g != 0).map(|g| (f, g)))
        })
    }

    pub fn has(&self, sym: Symbol) -> bool {
        self.glyph(sym).is_some()
    }

    /// The symbol trimmed to its ink, scaled so the em is `px` tall.
    pub fn render(&self, sym: Symbol, px: f32, color: u32) -> Option<Image> {
        let (font, gid) = self.glyph(sym)?;
        let img = font.render_glyph(gid, &Style { px, embolden: 0.0, color, shadow: false }).trimmed();
        (img.w > 0).then_some(img)
    }

    /// The symbol scaled to fit a `w` x `h` px box (aspect kept), trimmed.
    pub fn render_fit(&self, sym: Symbol, w: f32, h: f32, color: u32) -> Option<Image> {
        let probe = self.render(sym, 100.0, color)?;
        let k = (w / probe.w as f32).min(h / probe.h as f32);
        let img = self.render(sym, 100.0 * k, color)?;
        // antialiased edges can add a pixel to the ink box: shrink by the overshoot and render once more
        let over = (img.w as f32 / w).max(img.h as f32 / h);
        if over > 1.0 { self.render(sym, 100.0 * k / over, color) } else { Some(img) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn load() -> Symbols {
        Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets"))
    }

    #[test]
    fn every_symbol_has_a_name_and_most_a_code_point() {
        let names: std::collections::HashSet<_> = Symbol::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), Symbol::ALL.len(), "names are unique");
        assert!(Symbol::ALL.iter().filter(|s| s.unicode().is_none()).count() <= 2);
    }

    #[test]
    fn missing_fonts_mean_no_symbols() {
        let none = Symbols::load("/nonexistent");
        assert!(Symbol::ALL.iter().all(|&s| !none.has(s)));
        assert!(none.render(Symbol::Shift, 20.0, pack(255, 255, 255, 255)).is_none());
    }

    #[test]
    fn only_openly_licensed_symbol_fonts_are_listed() {
        // adding a file to FONT_FILES means adding it here too, which is where its licence gets checked
        const OPEN: [&str; 1] = ["Symbols.ttf"];
        assert!(FONT_FILES.iter().all(|f| OPEN.contains(f)), "{FONT_FILES:?}");
        // each one is credited in the font section of CREDITS.md, where every font is under the SIL OFL 1.1
        let credits = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CREDITS.md")).expect("CREDITS.md");
        let fonts = credits.split("\n## ").find(|s| s.starts_with("Fonts")).expect("a Fonts section");
        assert!(fonts.contains("SIL Open Font Licence"));
        for f in FONT_FILES {
            assert!(fonts.contains(&format!("`{f}`")), "{f} is not credited as an open font");
        }
    }

    #[test]
    fn a_vendor_symbol_font_in_the_assets_folder_is_never_loaded() {
        let Ok(open) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/Symbols.ttf")) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("bifrost-symbols-vendor-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let at = dir.to_string_lossy().into_owned();
        // a valid face under the vendor's file names: only the names in FONT_FILES are ever opened
        for name in ["SFUISymbols-Regular.otf", "SFUISymbols-Regular.ttf"] {
            std::fs::write(dir.join(name), &open).unwrap();
        }
        assert!(Symbols::load(&at).fonts.is_empty(), "the vendor file names are not read");
        std::fs::write(dir.join("Symbols.ttf"), &open).unwrap();
        assert_eq!(Symbols::load(&at).fonts.len(), FONT_FILES.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symbols_supplies_the_keyboard_glyphs() {
        let syms = load();
        if syms.fonts.is_empty() {
            return;
        }
        // the gear has a hand-drawn fallback (callers draw it when no symbol font has U+2699)
        for s in [Symbol::Shift, Symbol::DeleteLeft, Symbol::Return, Symbol::Bolt, Symbol::Play, Symbol::SunMax] {
            let img = syms.render(s, 40.0, pack(255, 255, 255, 255)).unwrap_or_else(|| panic!("{s:?}"));
            assert!(img.w > 4 && img.h > 4, "{s:?} is trimmed to its ink: {}x{}", img.w, img.h);
            assert!(img.px.iter().any(|&p| p >> 24 > 160), "{s:?} has solid ink");
        }
        assert!(!syms.has(Symbol::Wifi), "no font on the bifrost has the wifi symbol");
        let fit = syms.render_fit(Symbol::Shift, 30.0, 20.0, pack(255, 255, 255, 255)).unwrap();
        assert!(fit.w <= 30 && fit.h <= 20 && (fit.w == 30 || fit.h >= 19), "{}x{}", fit.w, fit.h);
    }
}
