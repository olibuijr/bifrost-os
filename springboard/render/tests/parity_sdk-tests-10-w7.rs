//! Parity sdk-tests-10: headless tests of the symbol set (`bifrost_render::symbols`): the symbol names and code points,
//! the font files searched, and the rasterised symbols from the packed symbol font.
//!
//! Headless: symbols are rasterised into in-memory packed pixels (raster images). No display, device or symbol font
//! outside the assets folder is read.

use bifrost_render::symbols::{Symbol, Symbols, FONT_FILES};

fn packed_assets() -> Symbols {
    Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets"))
}

#[test]
fn headless_the_symbol_set_lists_twenty_two_distinct_symbols() {
    assert_eq!(Symbol::ALL.len(), 22);
    let names: std::collections::HashSet<&str> = Symbol::ALL.iter().map(|s| s.name()).collect();
    assert_eq!(names.len(), Symbol::ALL.len(), "every symbol has its own glyph name");
    assert!(Symbol::ALL.iter().all(|s| !s.name().is_empty()));
}

#[test]
fn headless_a_symbol_is_named_after_its_catalogue_glyph() {
    assert_eq!(Symbol::Wifi.name(), "wifi");
    assert_eq!(Symbol::Battery.name(), "battery.100");
    assert_eq!(Symbol::Gear.name(), "gearshape.fill");
    assert_eq!(Symbol::GearOutline.name(), "gearshape", "the outlined gear is the unfilled glyph");
    assert_eq!(Symbol::Speaker.name(), "speaker.wave.2.fill");
    assert_eq!(Symbol::Magnifyingglass.name(), "magnifyingglass");
}

#[test]
fn headless_a_symbol_maps_to_its_closest_code_point_for_fonts_without_glyph_names() {
    assert_eq!(Symbol::Bolt.unicode(), Some('\u{26A1}'));
    assert_eq!(Symbol::Checkmark.unicode(), Some('\u{2713}'));
    assert_eq!(Symbol::Gear.unicode(), Some('\u{2699}'));
    assert_eq!(Symbol::GearOutline.unicode(), Symbol::Gear.unicode(), "both gears share one code point");
    assert_eq!(Symbol::Return.unicode(), Some('\u{23CE}'));
}

#[test]
fn headless_only_wifi_and_battery_have_no_code_point() {
    let without: Vec<Symbol> = Symbol::ALL.iter().copied().filter(|s| s.unicode().is_none()).collect();
    assert_eq!(without, vec![Symbol::Wifi, Symbol::Battery]);
}

#[test]
fn headless_only_the_openly_licensed_symbol_font_is_searched() {
    assert_eq!(FONT_FILES, ["Symbols.ttf"], "the Noto symbol face is the only file the loader opens");
}

#[test]
fn headless_a_symbol_set_with_no_fonts_has_no_symbols_and_draws_nothing() {
    let none = Symbols::new(Vec::new());
    assert!(Symbol::ALL.iter().all(|&s| !none.has(s)));
    assert!(none.glyph(Symbol::Shift).is_none());
    assert!(none.render(Symbol::Shift, 20.0, 0xffff_ffff).is_none());
    assert!(none.render_fit(Symbol::Shift, 30.0, 20.0, 0xffff_ffff).is_none());
}

#[test]
fn headless_a_default_symbol_set_is_empty() {
    let empty = Symbols::default();
    assert!(!empty.has(Symbol::Gear));
    assert!(empty.render(Symbol::Gear, 16.0, 0xff00_0000).is_none());
}

#[test]
fn headless_loading_from_a_folder_without_symbol_fonts_finds_none() {
    let none = Symbols::load("/nonexistent/bifrost-symbol-fonts");
    assert!(Symbol::ALL.iter().all(|&s| !none.has(s)));
    assert!(none.render(Symbol::Shift, 20.0, 0xffff_ffff).is_none());
}

#[test]
fn headless_the_packed_symbol_font_rasterises_the_keyboard_and_media_symbols_with_solid_ink() {
    let syms = packed_assets();
    for s in [Symbol::Shift, Symbol::DeleteLeft, Symbol::Return, Symbol::Bolt, Symbol::Play, Symbol::SunMax] {
        let img = syms.render(s, 40.0, 0xffff_ffff).unwrap_or_else(|| panic!("{s:?} rasterises"));
        assert!(img.w > 4 && img.h > 4, "{s:?} is trimmed to its ink: {}x{}", img.w, img.h);
        assert_eq!(img.px.len(), img.w * img.h, "one packed pixel per pixel of the image");
        assert!(img.px.iter().any(|&p| p >> 24 > 160), "{s:?} has solid ink");
    }
}

#[test]
fn headless_a_symbol_is_fitted_inside_its_box_with_its_aspect_kept() {
    let syms = packed_assets();
    let fit = syms.render_fit(Symbol::Shift, 30.0, 20.0, 0xffff_ffff).expect("the shift symbol fits");
    assert!(fit.w <= 30 && fit.h <= 20, "{}x{} stays in the 30 by 20 box", fit.w, fit.h);
    assert!(fit.w == 30 || fit.h >= 19, "the box is filled on one side: {}x{}", fit.w, fit.h);
}

#[test]
fn headless_a_symbol_missing_from_the_packed_font_is_not_drawn() {
    let syms = packed_assets();
    assert!(!syms.has(Symbol::Wifi), "no packed symbol font has the wireless glyph");
    assert!(syms.render(Symbol::Wifi, 20.0, 0xffff_ffff).is_none());
}
