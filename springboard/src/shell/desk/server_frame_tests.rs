use super::{frame_follows, Rect};

#[test]
fn a_new_position_is_taken_and_reported_as_a_change() {
    let mut r = Rect { x: 852, y: 339, w: 1184, h: 810 };
    assert!(frame_follows(&mut r, 800, 360));
    assert_eq!(r, Rect { x: 800, y: 360, w: 1184, h: 810 }, "the size is not touched");
}

#[test]
fn the_same_position_is_not_a_change() {
    let mut r = Rect { x: 800, y: 360, w: 1184, h: 810 };
    assert!(!frame_follows(&mut r, 800, 360));
    assert_eq!(r, Rect { x: 800, y: 360, w: 1184, h: 810 });
}

#[test]
fn a_window_dragged_partly_off_screen_keeps_the_reported_negative_position() {
    let mut r = Rect { x: 100, y: 100, w: 600, h: 400 };
    assert!(frame_follows(&mut r, -80, 28));
    assert_eq!((r.x, r.y), (-80, 28));
}
