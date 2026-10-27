use super::{kb_lift, Rect};

fn rect(y: i32, h: i32) -> Rect {
    Rect { x: 0, y, w: 600, h }
}

#[test]
fn a_window_that_clears_the_keyboard_does_not_move() {
    assert_eq!(kb_lift(rect(200, 600), 1200, 40), 0);
    // the bottom edge exactly on the keyboard's top is clear too
    assert_eq!(kb_lift(rect(200, 1000), 1200, 40), 0);
}

#[test]
fn a_window_under_the_keyboard_moves_up_by_the_overlap() {
    assert_eq!(kb_lift(rect(900, 600), 1200, 40), 300);
}

#[test]
fn the_lift_stops_at_the_menu_bar() {
    // the overlap is 900 but the top is only 60 px below the menu bar
    assert_eq!(kb_lift(rect(100, 1800), 1000, 40), 60);
    // a window already at the menu bar cannot move
    assert_eq!(kb_lift(rect(40, 1800), 1000, 40), 0);
}
