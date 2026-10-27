use super::type_select_at;
use bifrost_ui::menu::{Menu, MenuItem};

#[test]
fn letters_typed_a_few_hundred_milliseconds_apart_extend_one_prefix() {
    let mut m = Menu::new(vec![MenuItem::new(0, "Save"), MenuItem::new(1, "Select All"), MenuItem::new(2, "Settings")]);
    assert_eq!(type_select_at(&mut m, 's', 10_000.0), Some(0));
    assert_eq!(type_select_at(&mut m, 'e', 10_200.0), Some(1), "within the window: 'se'");
    assert_eq!(type_select_at(&mut m, 't', 10_400.0), Some(2), "'set'");
    // after a pause longer than the window the typing starts again
    assert_eq!(type_select_at(&mut m, 's', 20_000.0), Some(2));
    assert_eq!(type_select_at(&mut m, 'a', 30_000.0), None, "'a' alone matches no title");
}
