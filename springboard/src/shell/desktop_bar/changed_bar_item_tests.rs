use super::{changed_bar_rects, Menu};

fn item(menu: Menu, left: f32, width: f32, label: &str) -> (Menu, f32, f32, String) {
    (menu, left, width, label.to_string())
}

#[test]
fn the_same_layout_changes_nothing() {
    let bar = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 3%")];
    assert!(changed_bar_rects(&bar, &bar, 2560, 44, 8).is_empty());
}

#[test]
fn a_new_cpu_label_of_the_same_width_damages_only_that_label() {
    let before = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 3%"), item(Menu::Memory, 1970., 70., "RAM 40%")];
    let after = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 4%"), item(Menu::Memory, 1970., 70., "RAM 40%")];
    // the label's span, 8 px more on each side
    let rects = changed_bar_rects(&before, &after, 2560, 44, 8);
    assert_eq!(rects, vec![(1892, 0, 86, 44)]);
    let pixels: i32 = rects.iter().map(|r| r.2 * r.3).sum();
    assert!(pixels < 2560 * 44 / 10, "far less than the bar: {pixels}");
}

#[test]
fn a_wider_label_also_damages_the_item_that_moved_with_it() {
    let before = vec![item(Menu::Cpu, 1900., 70., "CPU 9%"), item(Menu::Memory, 1970., 70., "RAM 9%")];
    // RAM grew by 10 px, so CPU moved left by 10 and both changed
    let after = vec![item(Menu::Cpu, 1890., 70., "CPU 9%"), item(Menu::Memory, 1960., 80., "RAM 10%")];
    let rects = changed_bar_rects(&before, &after, 2560, 44, 0);
    assert_eq!(rects, vec![(1890, 0, 150, 44)]);
}

#[test]
fn items_that_appeared_or_went_and_the_screen_edges_are_handled() {
    let before = vec![item(Menu::App, 0., 50., "A")];
    let after = vec![item(Menu::Cpu, 2540., 40., "CPU 1%")];
    let rects = changed_bar_rects(&before, &after, 2560, 44, 8);
    assert_eq!(rects, vec![(0, 0, 58, 44), (2532, 0, 28, 44)], "a gone item and a new one, clipped to the screen");
}
