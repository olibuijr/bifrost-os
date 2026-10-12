//! Convert mode, driven through the same touch path the shell uses.

use super::*;
use bifrost_app::kit::Surface;

fn tap(app: &mut Calculator, ui: &Ui, x: f32, y: f32) {
    app.touch(ui, Event::Down(x, y), 0.0);
    app.touch(ui, Event::Up(x, y), 20.0);
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

#[test]
fn convert_mode_types_an_entry_converts_it_and_leaves_the_calculation_alone() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    let (x, y) = centre(app.bar_rect(&ui, BAR_CONVERT));
    tap(&mut app, &ui, x, y);
    assert!(app.convert.is_some());
    assert!(app.describe().contains("Length:m->km:0"));
    for digit in [1, 0] {
        let (x, y) = app.key_centre(&ui, Key::Digit(digit)).unwrap();
        tap(&mut app, &ui, x, y);
    }
    assert_eq!(app.convert.as_ref().unwrap().entry_text(), "10");
    assert_eq!(app.display(), "0", "the calculator's own display is untouched");
    assert_eq!(app.display_text(&ui), "0.01", "10 m in kilometres");

    // To: kilometres, then miles, then feet
    for _ in 0..2 {
        let (x, y) = centre(app.unit_rects(&ui)[2]);
        tap(&mut app, &ui, x, y);
    }
    let conv = app.convert.clone().unwrap();
    assert_eq!(conv.to_unit().symbol, "ft");
    assert_eq!(conv.result_text(crate::format::Locale::En), "32.808399");

    // Swap puts feet on the From side
    let (x, y) = centre(app.unit_rects(&ui)[1]);
    tap(&mut app, &ui, x, y);
    assert_eq!(app.convert.as_ref().unwrap().from_unit().symbol, "ft");

    // Category: temperature resets the units
    let (x, y) = centre(app.category_rect(&ui, 2));
    tap(&mut app, &ui, x, y);
    let conv = app.convert.clone().unwrap();
    assert_eq!(conv.category, Category::Temperature);
    assert_eq!((conv.from_unit().symbol, conv.to_unit().symbol), ("\u{b0}C", "\u{b0}F"));

    // The entry of 10 is now read in Celsius: 10 C is 50 F, and Copy gives that raw result
    assert_eq!(app.copy_text(&ui).as_deref(), Some("50"));
    let mut buf = vec![0; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 30.0);

    // Leaving Convert returns to the calculator
    let (x, y) = centre(app.bar_rect(&ui, BAR_CONVERT));
    tap(&mut app, &ui, x, y);
    assert!(app.convert.is_none());
    assert!(app.describe().contains("calculator_convert=\"off\""));
}
