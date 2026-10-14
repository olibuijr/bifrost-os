//! Parity sdk-complete-05: headless tests for showing a guest in a window. The guest view is a pure model, so these tests
//! place the picture, map pointer points and route keys with concrete values. No test starts a guest or opens a window.

use bifrost_core::guest_display::{GuestView, Rect, Route};

const RELEASE_KEY: u32 = 0x1d;

#[test]
fn a_guest_view_has_no_picture_before_it_is_resized() {
    let view = GuestView::new(640, 480, false, RELEASE_KEY);
    assert_eq!(view.rect(), Rect::EMPTY);
    assert_eq!(view.guest_point(0, 0), None);
}

#[test]
fn resizing_centres_a_whole_scale_picture_in_the_view() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    assert_eq!(view.rect(), Rect { x: 480, y: 210, w: 640, h: 480 });
}

#[test]
fn a_resize_moves_the_picture_for_the_new_view_size() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    view.resize(800, 600);
    assert_eq!(view.rect(), Rect { x: 80, y: 60, w: 640, h: 480 });
}

#[test]
fn a_wide_guest_letterboxes_when_scaling_is_not_whole_numbered() {
    let mut view = GuestView::new(1920, 1080, false, RELEASE_KEY);
    view.resize(800, 800);
    assert_eq!(view.rect(), Rect { x: 0, y: 175, w: 800, h: 450 });
}

#[test]
fn a_view_point_maps_to_the_guest_pixel_under_it() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    assert_eq!(view.guest_point(480, 210), Some((0, 0)));
    assert_eq!(view.guest_point(1119, 689), Some((639, 479)));
    assert_eq!(view.guest_point(479, 210), None, "the point is left of the picture");
}

#[test]
fn a_click_outside_the_picture_captures_nothing() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    assert!(!view.click(10, 10));
    assert!(!view.is_captured());
    assert_eq!(view.key(30, true), Route::Host);
}

#[test]
fn a_click_on_the_picture_captures_the_keyboard_once() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    assert!(view.click(500, 300));
    assert!(view.is_captured());
    assert!(!view.click(501, 301), "a second click does not capture again");
    assert_eq!(view.key(30, true), Route::Guest);
    assert_eq!(view.key(30, false), Route::Guest);
}

#[test]
fn the_release_key_gives_the_keyboard_back_to_the_host() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    view.click(500, 300);
    assert_eq!(view.key(RELEASE_KEY, false), Route::Guest, "only the press releases the keyboard");
    assert_eq!(view.key(RELEASE_KEY, true), Route::Host);
    assert!(!view.is_captured());
    assert_eq!(view.key(30, true), Route::Host);
}

#[test]
fn losing_focus_releases_the_keyboard_and_a_later_click_captures_it_again() {
    let mut view = GuestView::new(640, 480, true, RELEASE_KEY);
    view.resize(1600, 900);
    view.click(500, 300);
    view.focus_lost();
    assert!(!view.is_captured());
    assert!(view.click(500, 300));
    assert_eq!(view.key(30, true), Route::Guest);
}
