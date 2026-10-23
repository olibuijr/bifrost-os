//! Programmer mode, RPN mode, the history groups and the two-row Convert categories, driven through the same touch
//! path the shell uses.

use super::*;
use bifrost_app::kit::Surface;

fn tap(app: &mut Calculator, ui: &Ui, x: f32, y: f32) {
    app.touch(ui, Event::Down(x, y), 0.0);
    app.touch(ui, Event::Up(x, y), 20.0);
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn landscape() -> (Ui, Calculator) {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1920, 1200);
    app.settings.scientific = true;
    (ui, app)
}

fn tap_prog(app: &mut Calculator, ui: &Ui, label: &str) {
    let k = PROG_KEYS.iter().position(|(l, _)| *l == label).unwrap();
    let (x, y) = centre(app.key_rect(ui, KEYS.len() + k));
    tap(app, ui, x, y);
}

fn tap_key(app: &mut Calculator, ui: &Ui, key: Key) {
    let (x, y) = app.key_centre(ui, key).unwrap();
    tap(app, ui, x, y);
}

fn open_programmer(app: &mut Calculator, ui: &Ui) {
    let (x, y) = centre(app.bar_rect(ui, BAR_PROGRAMMER));
    tap(app, ui, x, y);
}

#[test]
fn the_programmer_segment_opens_the_keypad_and_basic_leaves_it() {
    let (ui, mut app) = landscape();
    assert!(app.sci_active(&ui));
    open_programmer(&mut app, &ui);
    assert!(app.programmer_active(&ui) && !app.sci_active(&ui));
    assert_eq!(app.key_count(&ui), KEYS.len() + PROG_KEYS.len());
    for i in 0..app.key_count(&ui) {
        let (x, y, w, h) = app.key_rect(&ui, i);
        assert!(x >= 0.0 && x + w <= 1920.0 && y >= 0.0 && y + h <= 1200.0, "key {i}");
    }
    // the binary row sits between the display and the keys
    let (_, by, _, bh) = app.bit_rect(&ui, 0);
    let (_, ky, _, _) = app.key_rect(&ui, 0);
    assert!(by + bh <= ky + 0.01);
    let bar = app.bar(&ui);
    assert!(bar.items.iter().any(|i| i.label == "Programmer" && i.selected));
    assert!(bar.items.iter().any(|i| i.label == "Basic" && !i.selected));
    let (x, y) = centre(app.bar_rect(&ui, BAR_BASIC));
    tap(&mut app, &ui, x, y);
    assert!(app.programmer.is_none());
}

#[test]
fn programmer_keys_type_in_the_chosen_base_and_a_calculation_reaches_the_history() {
    let (ui, mut app) = landscape();
    open_programmer(&mut app, &ui);
    tap_prog(&mut app, &ui, "HEX");
    tap_prog(&mut app, &ui, "FF");
    assert_eq!(app.display_text(&ui), "FF");
    tap_key(&mut app, &ui, Key::Digit(1));
    assert_eq!(app.display_text(&ui), "FF1");
    tap_key(&mut app, &ui, Key::Clear);
    tap_prog(&mut app, &ui, "A");
    tap_prog(&mut app, &ui, "XOR");
    tap_prog(&mut app, &ui, "F");
    tap_key(&mut app, &ui, Key::Equals);
    assert_eq!(app.display_text(&ui), "5");
    assert_eq!(app.store.len(), 1);
    assert_eq!(app.store.get(0).unwrap().result, "5");
    tap_prog(&mut app, &ui, "BIN");
    assert_eq!(app.display_text(&ui), "101");
    tap_prog(&mut app, &ui, "DEC");
    tap_prog(&mut app, &ui, "2's");
    assert_eq!(app.display_text(&ui), "-5");
    tap_prog(&mut app, &ui, "8 bit");
    assert_eq!(app.programmer.as_ref().unwrap().bits(), 8);
    assert_eq!(app.display_text(&ui), "-5");
    // a decimal point and a percent have no meaning on whole numbers
    tap_key(&mut app, &ui, Key::Decimal);
    tap_key(&mut app, &ui, Key::Percent);
    assert_eq!(app.display_text(&ui), "-5");
}

#[test]
fn tapping_a_bit_flips_it_and_the_bits_are_controls() {
    let (ui, mut app) = landscape();
    open_programmer(&mut app, &ui);
    let (x, y) = centre(app.bit_rect(&ui, 0));
    tap(&mut app, &ui, x, y);
    assert_eq!(app.programmer.as_ref().unwrap().value(), 1);
    let (x, y) = centre(app.bit_rect(&ui, 63));
    tap(&mut app, &ui, x, y);
    assert_eq!(app.programmer.as_ref().unwrap().value(), 0x8000_0000_0000_0001);
    let (x, y) = centre(app.bit_rect(&ui, 0));
    tap(&mut app, &ui, x, y);
    assert_eq!(app.programmer.as_ref().unwrap().value(), 0x8000_0000_0000_0000);
    let controls = app.controls(&ui);
    assert!(controls.len() <= 128, "the shell keeps at most 128 controls, got {}", controls.len());
    let bit = |n: u8| controls.iter().find(|c| c.label == format!("Bit {n}")).unwrap();
    assert_eq!(bit(63).value, "1");
    assert_eq!(bit(0).value, "0");
    assert_eq!(controls.iter().filter(|c| c.label.starts_with("Bit ")).count(), 64);
    assert!(controls.iter().any(|c| c.label == "FlipB"));
    // bits outside a smaller word are neither shown as controls nor hit
    tap_prog(&mut app, &ui, "16 bit");
    let controls = app.controls(&ui);
    assert_eq!(controls.iter().filter(|c| c.label.starts_with("Bit ")).count(), 16);
    let (x, y) = centre(app.bit_rect(&ui, 40));
    assert_eq!(app.hit(&ui, x, y), None);
}

#[test]
fn the_programmer_screen_draws_with_its_readout_and_base_selection() {
    let (ui, mut app) = landscape();
    open_programmer(&mut app, &ui);
    tap_key(&mut app, &ui, Key::Digit(6));
    tap_key(&mut app, &ui, Key::Digit(5));
    assert_eq!(app.programmer.as_ref().unwrap().readout(), "U+0041  A  ASCII 65");
    let mut buf = vec![0; 1920 * 1200];
    app.draw(&mut Surface { w: 1920, h: 1200, stride: 1920, px: &mut buf, y0: 0 }, &ui, 0.0);
    assert!(buf.contains(&scientific()));
    assert!(buf.contains(&ORANGE));
    // a window too narrow for the wide keypad falls back to the basic one
    let mut narrow = Calculator::new(&ui, 400, 1200);
    narrow.set_mode(Mode::Programmer);
    assert!(!narrow.programmer_active(&ui));
    narrow.hardware_key(&ui, Key::Digit(7));
    assert_eq!(narrow.display(), "7");
}

#[test]
fn rpn_mode_labels_enter_and_keeps_the_stack_across_keys() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    app.settings.rpn = true;
    assert!(app.rpn_active() && !app.sci_active(&ui));
    assert!(app.controls(&ui).iter().any(|c| c.label == "Enter"));
    for key in [Key::Digit(4), Key::Equals, Key::Digit(5), Key::Operator(Operator::Multiply)] {
        tap_key(&mut app, &ui, key);
    }
    assert_eq!(app.display_text(&ui), "20");
    assert_eq!(app.store.get(0).unwrap().expression, "4 × 5");
    let mut buf = vec![0; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
    assert!(buf.contains(&ORANGE));
    // choosing the scientific keypad ends RPN
    let (x, y) = centre(app.bar_rect(&ui, BAR_SCIENTIFIC));
    tap(&mut app, &ui, x, y);
    assert!(!app.settings.rpn && app.sci_active(&ui));
}

#[test]
fn a_swipe_to_the_left_deletes_a_history_row() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    for n in 0..3 {
        app.store.push_at(Entry { expression: format!("{n}+1"), result: (n + 1).to_string(), value: (n + 1) as f64 }, 0);
    }
    app.history_open = true;
    let (x, y) = centre(app.result_rect(&ui, 1));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Move(x - ui.pt(100.0), y), 10.0);
    app.touch(&ui, Event::Up(x - ui.pt(100.0), y), 20.0);
    assert_eq!(app.store.len(), 2);
    assert_eq!(app.store.get(0).unwrap().result, "3");
    assert_eq!(app.store.get(1).unwrap().result, "1");
    assert!(app.history_open, "a swipe does not recall");
    // a swipe to the right does nothing
    let (x, y) = centre(app.result_rect(&ui, 0));
    app.touch(&ui, Event::Down(x, y), 0.0);
    app.touch(&ui, Event::Move(x + ui.pt(100.0), y), 10.0);
    app.touch(&ui, Event::Up(x + ui.pt(100.0), y), 20.0);
    assert_eq!(app.store.len(), 2);
}

#[test]
fn clear_history_asks_twice_and_any_other_tap_withdraws_the_question() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    app.store.push(Entry { expression: "2+3".into(), result: "5".into(), value: 5.0 });
    app.history_open = true;
    let (px, py, pw, ph) = app.panel(&ui);
    let clear = (px + pw * 0.75, py + ph - ui.pt(30.0));
    tap(&mut app, &ui, clear.0, clear.1);
    assert!(app.clear_confirm);
    assert!(app.controls(&ui).iter().any(|c| c.label == "Confirm Clear History"));
    let (x, y) = centre(app.result_rect(&ui, 0));
    tap(&mut app, &ui, x, y);
    assert!(!app.clear_confirm && !app.history_open);
    assert_eq!(app.store.len(), 1, "nothing was cleared");
    // with nothing to clear the first tap asks nothing
    app.store.clear();
    app.history_open = true;
    tap(&mut app, &ui, clear.0, clear.1);
    assert!(!app.clear_confirm);
}

#[test]
fn the_history_draws_day_captions() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    let now = history::now_secs();
    app.store.push_at(Entry { expression: "1+1".into(), result: "2".into(), value: 2.0 }, now.saturating_sub(3 * 86_400));
    app.store.push_at(Entry { expression: "2+2".into(), result: "4".into(), value: 4.0 }, now);
    app.history_open = true;
    let mut buf = vec![0; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
    assert!(history::starts_day(|k| app.store.time(k), 0));
    assert!(history::starts_day(|k| app.store.time(k), 1), "the older entry is another day");
    assert_eq!(history::day_caption(app.store.time(0), now), "Today");
}

#[test]
fn convert_shows_all_fourteen_categories_in_two_rows() {
    let ui = crate::test_ui();
    let mut app = Calculator::new(&ui, 1200, 1920);
    let (x, y) = centre(app.bar_rect(&ui, BAR_CONVERT));
    tap(&mut app, &ui, x, y);
    let (_, top, _, h) = app.category_rect(&ui, 0);
    let (_, second, _, _) = app.category_rect(&ui, 7);
    assert!(second >= top + h, "the eighth category starts the second row");
    assert!(app.unit_rects(&ui)[0].1 >= second + h, "the unit row sits under both category rows");
    let (x, y) = centre(app.category_rect(&ui, 7));
    tap(&mut app, &ui, x, y);
    assert_eq!(app.convert.as_ref().unwrap().category, Category::DataSize);
    let (x, y) = centre(app.category_rect(&ui, 12));
    tap(&mut app, &ui, x, y);
    assert_eq!(app.convert.as_ref().unwrap().category, Category::FuelEconomy);
    let controls = app.controls(&ui);
    assert_eq!(controls.iter().filter(|c| Category::ALL.iter().any(|k| k.title() == c.label)).count(), Category::ALL.len());
    let mut ids: Vec<u64> = controls.iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "every control has its own id");
    let mut buf = vec![0; 1200 * 1920];
    app.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
}
