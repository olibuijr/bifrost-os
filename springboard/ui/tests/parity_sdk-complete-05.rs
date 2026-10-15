//! Parity sdk-complete-05: headless tests for scroll targets (the offset that reveals a document rectangle) and for the
//! display mode choice. Each test drives the public API of a pure model and asserts concrete values. Nothing here needs
//! a display, a device or the clock.

use bifrost_ui::display_manager::{Criteria, Decision, DisplayManager, DynamicRange, Mode};
use bifrost_ui::scroll::{offset_for_rect, Anchor};

fn sdr(rate: f32) -> Mode {
    Mode { refresh_rate: rate, dynamic_range: DynamicRange::Sdr }
}

fn hdr(rate: f32) -> Mode {
    Mode { refresh_rate: rate, dynamic_range: DynamicRange::Hdr }
}

fn want(rate: f32, dynamic_range: DynamicRange) -> Criteria {
    Criteria { refresh_rate: rate, dynamic_range }
}

#[test]
fn leading_anchor_puts_the_top_edge_at_the_viewport_top() {
    // a 40 point row at 300 in a 200 point viewport over 1000 points of content
    assert_eq!(offset_for_rect(300.0, 40.0, 200.0, 1000.0, Anchor::Leading), 300.0);
}

#[test]
fn center_anchor_puts_the_middle_of_the_row_in_the_middle_of_the_viewport() {
    assert_eq!(offset_for_rect(300.0, 40.0, 200.0, 1000.0, Anchor::Center), 220.0);
}

#[test]
fn trailing_anchor_puts_the_bottom_edge_at_the_viewport_bottom() {
    assert_eq!(offset_for_rect(300.0, 40.0, 200.0, 1000.0, Anchor::Trailing), 140.0);
}

#[test]
fn a_target_near_the_top_clamps_to_offset_zero() {
    assert_eq!(offset_for_rect(0.0, 40.0, 200.0, 1000.0, Anchor::Center), 0.0);
    assert_eq!(offset_for_rect(10.0, 40.0, 200.0, 1000.0, Anchor::Trailing), 0.0);
}

#[test]
fn a_target_near_the_end_clamps_to_the_largest_offset() {
    // the largest offset is 1000 - 200 = 800
    assert_eq!(offset_for_rect(950.0, 40.0, 200.0, 1000.0, Anchor::Leading), 800.0);
    assert_eq!(offset_for_rect(990.0, 10.0, 200.0, 1000.0, Anchor::Center), 800.0);
}

#[test]
fn content_shorter_than_the_viewport_never_scrolls() {
    assert_eq!(offset_for_rect(30.0, 40.0, 200.0, 120.0, Anchor::Leading), 0.0);
    assert_eq!(offset_for_rect(30.0, 40.0, 200.0, 120.0, Anchor::Trailing), 0.0);
}

#[test]
fn non_finite_input_gives_offset_zero() {
    assert_eq!(offset_for_rect(f32::NAN, 40.0, 200.0, 1000.0, Anchor::Leading), 0.0);
    assert_eq!(offset_for_rect(300.0, f32::INFINITY, 200.0, 1000.0, Anchor::Center), 0.0);
    assert_eq!(offset_for_rect(300.0, 40.0, 200.0, f32::NAN, Anchor::Trailing), 0.0);
}

#[test]
fn a_request_that_no_mode_can_meet_cancels_a_pending_switch() {
    let mut m = DisplayManager::new();
    let modes = [sdr(60.0), sdr(24.0)];
    assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(24.0)));
    assert!(m.is_switch_in_progress());
    assert_eq!(m.request(want(24.0, DynamicRange::Hdr), &modes), Decision::Unsupported);
    assert!(!m.is_switch_in_progress());
    assert!(m.finish_switch().is_none());
    assert_eq!(m.current_mode(), None);
}

#[test]
fn a_new_request_retargets_a_pending_switch() {
    let mut m = DisplayManager::new();
    let modes = [sdr(48.0), sdr(60.0), sdr(24.0)];
    assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(24.0)));
    assert_eq!(m.request(want(60.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(60.0)));
    assert_eq!(m.finish_switch(), Some(sdr(60.0)));
    assert_eq!(m.current_mode(), Some(sdr(60.0)));
}

#[test]
fn a_request_for_the_current_mode_while_a_switch_is_pending_keeps_the_panel() {
    let mut m = DisplayManager::new();
    let modes = [sdr(60.0), sdr(24.0)];
    m.request(want(60.0, DynamicRange::Sdr), &modes);
    m.finish_switch();
    assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(24.0)));
    assert_eq!(m.request(want(60.0, DynamicRange::Sdr), &modes), Decision::Keep);
    assert!(!m.is_switch_in_progress());
    assert_eq!(m.current_mode(), Some(sdr(60.0)));
}

#[test]
fn an_hdr_request_picks_the_hdr_mode_at_the_closest_rate() {
    let mut m = DisplayManager::new();
    let modes = [sdr(24.0), hdr(23.976), hdr(60.0)];
    assert_eq!(m.request(want(24.0, DynamicRange::Hdr), &modes), Decision::Switch(hdr(23.976)));
}
