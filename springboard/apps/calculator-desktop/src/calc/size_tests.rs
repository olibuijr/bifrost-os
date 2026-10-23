//! The window sizes of the keypads (Golden Gate Calculator resizes its window per keypad) and the compact Basic window
//! of 27-Golden-Gate-Calculator.png: each keypad lays out whole inside the size it asks for.

use super::*;
use bifrost_app::kit::Surface;

/// A calculator laid out at a content size in points.
fn at(ui: &Ui, (w, h): (f32, f32)) -> Calculator {
    Calculator::new(ui, ui.pt(w).round() as i32, ui.pt(h).round() as i32)
}

fn tap(app: &mut Calculator, ui: &Ui, x: f32, y: f32) {
    app.touch(ui, Event::Down(x, y), 0.0);
    app.touch(ui, Event::Up(x, y), 20.0);
}

fn keys_inside(app: &Calculator, ui: &Ui) {
    let top = ui.pt(UNIFIED_BAR_PT);
    for i in 0..app.key_count(ui) {
        let (x, y, w, h) = app.key_rect(ui, i);
        assert!(x >= 0.0 && x + w <= app.w as f32 + 0.5, "key {i} inside the width");
        assert!(y >= top && y + h <= app.h as f32 + 0.5, "key {i} under the toolbar and inside the height");
    }
}

#[test]
fn each_keypad_asks_for_its_own_window_size() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    assert_eq!(app.preferred_size(), BASIC_SIZE_PT);
    app.set_mode(Mode::Scientific);
    assert_eq!(app.preferred_size(), SCIENTIFIC_SIZE_PT);
    app.set_mode(Mode::Programmer);
    assert_eq!(app.preferred_size(), PROGRAMMER_SIZE_PT);
    app.menu_action(&ui, 14); // View > Unit Conversion
    assert_eq!(app.preferred_size(), CONVERT_SIZE_PT);
    app.set_mode(Mode::Basic);
    assert_eq!(app.preferred_size(), BASIC_SIZE_PT);
    // RPN rides on the basic keypad
    app.settings.scientific = true;
    app.settings.rpn = true;
    assert_eq!(app.preferred_size(), BASIC_SIZE_PT);
}

#[test]
fn the_compact_basic_window_matches_the_reference_keypad() {
    let ui = crate::test_ui();
    let app = at(&ui, BASIC_SIZE_PT);
    assert!(!app.wide(&ui));
    let l = app.layout(&ui);
    let p = ui.pt(1.0);
    // four 48-point keys, 6 points apart, 10 points in from the sides (27-Golden-Gate-Calculator.png)
    assert!((l.cw / p - 48.0).abs() < 0.5, "key {} pt", l.cw / p);
    assert!((l.gap / p - 6.0).abs() < 0.2, "gap {} pt", l.gap / p);
    assert!((l.x / p - 10.0).abs() < 0.5, "side {} pt", l.x / p);
    keys_inside(&app, &ui);
    // the display fits between the toolbar's expression line and the Delete control
    let pt = app.display_pt_max(&ui);
    assert!(pt >= MAC_LARGE_TITLE_PT && pt < DISPLAY_LARGE_PT, "a compact display, got {pt}");
    let display_top = app.deck_top(&ui) - ui.pt(62.0) - ui.pt(pt * LINE_EM);
    let expression_top = display_top - ui.pt(MAC_TITLE1_PT * LINE_EM + SPACE_M);
    assert!(expression_top >= ui.pt(UNIFIED_BAR_PT) - 1.0, "the expression line starts under the toolbar");
    let mut buf = vec![0; (app.w * app.h) as usize];
    let mut app = app;
    app.draw(&mut Surface { w: app.w as usize, h: app.h as usize, stride: app.w as usize, px: &mut buf, y0: 0 }, &ui, 0.0);
    assert!(buf.contains(&ORANGE), "the operator keys draw");
}

#[test]
fn a_large_window_keeps_the_full_margins() {
    let ui = crate::test_ui();
    let app = Calculator::new(&ui, 1200, 1920);
    assert_eq!(app.margin_scale(&ui), 1.0);
    let l = app.layout(&ui);
    assert_eq!(l.gap, ui.pt(SPACE_M), "the 12-point gap of a wide window");
}

#[test]
fn the_compact_window_shows_one_keypad_pull_down_that_switches_and_resizes() {
    let ui = crate::test_ui();
    let mut app = at(&ui, BASIC_SIZE_PT);
    let layout = app.bar_layout(&ui);
    assert!(layout.overflow.is_empty(), "nothing hides behind a » button");
    let (x, y, w, h) = app.bar_rect(&ui, BAR_MODE);
    assert!(w > 0.0, "the pull-down is on the bar");
    assert_eq!(app.bar_rect(&ui, BAR_SCIENTIFIC), (0.0, 0.0, 0.0, 0.0), "the segments are not");
    assert!(app.take_mode_menu(&ui).is_none(), "closed until tapped");
    tap(&mut app, &ui, x + w / 2.0, y + h / 2.0);
    let (mx, my, items) = app.take_mode_menu(&ui).expect("the tap opens the pull-down");
    assert!(mx >= x - 1.0 && my >= y + h - 1.0, "it opens under the button");
    let titles: Vec<&str> = items.iter().map(|i| i.label.trim_start_matches(['\u{2713}', '\u{2003}', ' '])).collect();
    assert_eq!(titles, ["Basic", "Scientific", "Programmer", "", "Convert", "RPN Mode"]);
    assert!(items[0].label.starts_with('\u{2713}'), "Basic is checked");
    assert!(app.take_mode_menu(&ui).is_none(), "taken once");
    assert!(app.context_action(&ui, items[1].id));
    assert_eq!(app.preferred_size(), SCIENTIFIC_SIZE_PT);
}

#[test]
fn the_wide_keypads_fit_their_windows_with_the_full_toolbar() {
    let ui = crate::test_ui();
    let mut sci = at(&ui, SCIENTIFIC_SIZE_PT);
    sci.set_mode(Mode::Scientific);
    assert!(sci.sci_active(&ui), "the scientific window is wide enough for ten columns");
    assert!(sci.bar_layout(&ui).overflow.is_empty(), "the four segments fit");
    keys_inside(&sci, &ui);
    assert!(sci.display_pt_max(&ui) >= MAC_LARGE_TITLE_PT);

    let mut prog = at(&ui, PROGRAMMER_SIZE_PT);
    prog.set_mode(Mode::Programmer);
    assert!(prog.programmer_active(&ui));
    assert!(prog.bar_layout(&ui).overflow.is_empty());
    keys_inside(&prog, &ui);
    assert!(prog.deck_top(&ui) - ui.pt(62.0) - ui.pt(MAC_LARGE_TITLE_PT * LINE_EM) >= ui.pt(UNIFIED_BAR_PT), "the display clears the toolbar");
}

#[test]
fn the_convert_window_fits_every_category_title() {
    let ui = crate::test_ui();
    let mut app = at(&ui, CONVERT_SIZE_PT);
    app.menu_action(&ui, 14);
    assert!(app.convert.is_some());
    for (index, category) in Category::ALL.iter().enumerate() {
        let (cx, _, cw, _) = app.category_rect(&ui, index);
        assert!(cx >= 0.0 && cx + cw <= app.w as f32, "{} inside the window", category.title());
        assert!(ui.measure(category.title(), MAC_CALLOUT_PT, Weight::Regular) <= cw - ui.pt(SPACE_S), "{} fits", category.title());
    }
    keys_inside(&app, &ui);
}
