//! The page draws and measures each run in its own family (`Style::font`), so a choice in the Fonts panel, the Font
//! submenu or the format bar shows on the page. A family is drawn with its font file (`text::family_file`): an installed
//! family's own, or the stand-in for Times and Courier. A family with no file, a file that cannot be read or parsed,
//! and text the family has no glyphs for fall back to the system font, as `NSFont` substitutes a font for missing
//! glyphs. Each family's file is read once per run of the app and kept for it (bounded by the families used).

use bifrost_app::kit::ui::Ui;
use bifrost_app::kit::Font;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// One drawing context per family index, `None` once its file failed to load (not tried again).
    static FAMILIES: RefCell<HashMap<u8, Option<Ui>>> = RefCell::new(HashMap::new());
}

/// Runs `f` with the drawing context for family `font` at `ui`'s scale and text settings, or with `ui` itself (the
/// system font) when the family has no font file, its file does not load, or it lacks a glyph of `text`.
pub fn with_family<R>(ui: &Ui, font: u8, text: &str, f: impl FnOnce(&Ui) -> R) -> R {
    let Some(path) = crate::text::family_file(font) else { return f(ui) };
    let mut f = Some(f);
    let done = FAMILIES.with(|cell| {
        // a call made from inside `f` (none today) draws in the system font rather than borrowing twice
        let mut map = cell.try_borrow_mut().ok()?;
        let family = map.entry(font).or_insert_with(|| load(path, ui.s)).as_mut()?;
        if !covers(&family.font, text) {
            return None;
        }
        family.s = ui.s;
        family.text_scale = ui.text_scale;
        family.bold = ui.bold;
        let f = f.take()?;
        Some(f(&*family))
    });
    match done {
        Some(r) => r,
        // `f` has not run: it runs only where `done` becomes `Some`
        None => (f.take().expect("the closure runs once"))(ui),
    }
}

/// The drawing context for the font file at `path`, at `s` pixels a point. The bytes are kept for the whole run.
fn load(path: &str, s: f32) -> Option<Ui> {
    let bytes = std::fs::read(path).ok()?;
    let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    Font::parse(data).map(|font| Ui::new(font, s))
}

/// True when `font` has a glyph for every character of `text` that draws (white space and control characters need none).
pub fn covers(font: &Font<'_>, text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || c.is_control() || font.glyph(c) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 2.0)
    }

    #[test]
    fn a_family_with_no_file_draws_in_the_system_font() {
        // tests see the base families only, and Helvetica has no file: the closure gets the page's own context
        let ui = ui();
        let same = with_family(&ui, 0, "Hello", |u| std::ptr::eq(u, &ui));
        assert!(same);
        assert!(with_family(&ui, 200, "Hello", |u| std::ptr::eq(u, &ui)), "an index past the catalogue too");
        let w = with_family(&ui, 0, "Hello", |u| u.measure("Hello", 13.0, bifrost_app::kit::ui::Weight::Regular));
        assert_eq!(w, ui.measure("Hello", 13.0, bifrost_app::kit::ui::Weight::Regular));
    }

    #[test]
    fn a_font_covers_text_it_has_glyphs_for_and_white_space() {
        let ui = ui();
        assert!(covers(&ui.font, "Hello world\t\n"));
        assert!(!covers(&ui.font, "\u{10ffff}"), "a code point no font maps");
    }

    #[test]
    fn a_missing_font_file_is_not_a_font() {
        assert!(load("/nonexistent/font.ttf", 1.0).is_none());
    }
}
