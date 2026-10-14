//! Parity sdk-tests-07 (wave 7): headless tests for the pointer and pen gesture models. Each model is a pure state
//! machine or a pure geometry function driven through its public API with concrete values. Nothing is drawn, no
//! window is opened, no input device is read, and no clock is consulted: every time is a fixed number.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use bifrost_ui::drawing::{Ink, InkKind, Stroke, StrokePoint};
use bifrost_ui::gesture::{LongPress, Magnify, Pan, Rotate, State, Tap};
use bifrost_ui::scribble::{classify, resolve, Action, Gesture, SCRATCH_MIN_REVERSALS, TAP_EXTENT};
use bifrost_ui::stylus::{pen_angles, LogicalRange, StylusProfile, StylusReport};
use bifrost_ui::trackpad_gesture::{Event, Kind, TwoFinger};
use bifrost_ui::Rect;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// ---- gesture: tap, long press, pan, pinch and rotation recognizers ----

#[test]
fn headless_a_single_tap_ends_a_one_tap_recognizer_on_release() {
    let mut tap = Tap::new(1);
    tap.down(0.0, 0.0, 0.0);
    assert_eq!(tap.up(0.1), State::Ended);
}

#[test]
fn headless_a_double_tap_ends_only_on_the_second_release() {
    let mut tap = Tap::new(2);
    tap.down(0.0, 0.0, 0.0);
    assert_eq!(tap.up(0.1), State::Possible, "the first tap of two is not the end");
    tap.down(0.0, 0.0, 0.2);
    assert_eq!(tap.up(0.3), State::Ended);
}

#[test]
fn headless_a_pause_longer_than_the_multi_tap_window_restarts_the_count() {
    let mut tap = Tap::new(2);
    tap.down(0.0, 0.0, 0.0);
    assert_eq!(tap.up(0.1), State::Possible);
    tap.down(0.0, 0.0, 1.0);
    assert_eq!(tap.up(1.1), State::Possible, "the second tap came too late to count with the first");
}

#[test]
fn headless_a_press_held_past_the_long_press_time_is_not_a_tap() {
    let mut tap = Tap::new(1);
    tap.down(0.0, 0.0, 0.0);
    assert_eq!(tap.up(0.6), State::Failed);
}

#[test]
fn headless_a_tap_that_drifts_past_the_slop_fails_and_a_small_drift_does_not() {
    let mut tap = Tap::new(1);
    tap.down(0.0, 0.0, 0.0);
    assert_eq!(tap.moved(2.0, 2.0), State::Possible, "2.8 points is inside the 4 point slop");
    assert_eq!(tap.moved(10.0, 0.0), State::Failed);
    let mut drifted = Tap::new(1);
    drifted.down(0.0, 0.0, 0.0);
    assert_eq!(drifted.moved(10.0, 0.0), State::Failed);
}

#[test]
fn headless_a_tap_recognizer_needs_at_least_one_tap() {
    assert_eq!(Tap::new(0).required, 1);
    assert_eq!(Tap::new(3).required, 3);
}

#[test]
fn headless_a_long_press_begins_after_its_hold_time_and_ends_on_release() {
    let mut press = LongPress::new();
    press.down(0.0, 0.0, 0.0);
    assert_eq!(press.tick(0.2), State::Possible);
    assert_eq!(press.tick(0.5), State::Began, "held for the full 0.5 seconds");
    assert_eq!(press.tick(0.7), State::Changed);
    assert_eq!(press.up(), State::Ended);
}

#[test]
fn headless_a_long_press_released_before_it_began_fails() {
    let mut press = LongPress::new();
    press.down(0.0, 0.0, 0.0);
    assert_eq!(press.tick(0.1), State::Possible);
    assert_eq!(press.up(), State::Failed);
}

#[test]
fn headless_moving_before_a_long_press_begins_cancels_it() {
    let mut press = LongPress::new();
    press.down(0.0, 0.0, 0.0);
    assert_eq!(press.moved(0.0, 10.0), State::Failed);
}

#[test]
fn headless_a_long_press_that_already_began_ignores_later_movement() {
    let mut press = LongPress::new();
    press.down(0.0, 0.0, 0.0);
    assert_eq!(press.tick(0.6), State::Began);
    assert_eq!(press.moved(50.0, 0.0), State::Possible);
}

#[test]
fn headless_a_pan_begins_past_the_slop_and_reports_its_translation() {
    let mut pan = Pan::new();
    pan.down(10.0, 10.0, 0.0);
    assert_eq!(pan.moved(12.0, 10.0, 0.01), State::Possible, "2 points is inside the slop");
    assert_eq!(pan.moved(30.0, 10.0, 0.02), State::Began);
    assert_eq!(pan.moved(40.0, 10.0, 0.03), State::Changed);
    assert_eq!(pan.translation(), (30.0, 0.0));
    assert_eq!(pan.up(), State::Ended);
}

#[test]
fn headless_a_pan_reports_release_velocity_in_points_per_second() {
    let mut pan = Pan::new();
    pan.down(10.0, 10.0, 0.0);
    pan.moved(12.0, 10.0, 0.01);
    pan.moved(30.0, 10.0, 0.02);
    pan.moved(40.0, 10.0, 0.03);
    let (vx, vy) = pan.velocity();
    assert!((vx - 1000.0).abs() < 1.0, "30 points over 0.03 seconds is 1000 points per second, got {vx}");
    assert!(near(vy, 0.0));
}

#[test]
fn headless_a_pan_released_before_it_began_fails() {
    let mut pan = Pan::new();
    pan.down(0.0, 0.0, 0.0);
    assert_eq!(pan.up(), State::Failed);
}

#[test]
fn headless_a_pinch_reports_magnification_relative_to_the_first_spacing() {
    let mut pinch = Magnify::default();
    assert_eq!(pinch.touches((0.0, 0.0), (100.0, 0.0)), State::Possible);
    assert_eq!(pinch.touches((0.0, 0.0), (150.0, 0.0)), State::Began);
    assert!(near(pinch.magnification(), 0.5), "150 over 100 is a magnification of 0.5");
    assert_eq!(pinch.touches((0.0, 0.0), (200.0, 0.0)), State::Changed);
    assert!(near(pinch.magnification(), 1.0), "twice the spacing is 1.0");
    assert_eq!(pinch.up(), State::Ended);
}

#[test]
fn headless_a_pinch_inwards_is_negative_magnification() {
    let mut pinch = Magnify::default();
    pinch.touches((0.0, 0.0), (100.0, 0.0));
    assert_eq!(pinch.touches((0.0, 0.0), (50.0, 0.0)), State::Began);
    assert!(near(pinch.magnification(), -0.5));
}

#[test]
fn headless_a_pinch_inside_the_slop_does_not_begin() {
    let mut pinch = Magnify::default();
    pinch.touches((0.0, 0.0), (100.0, 0.0));
    assert_eq!(pinch.touches((0.0, 0.0), (102.0, 0.0)), State::Possible);
    assert!(near(pinch.magnification(), 0.02));
}

#[test]
fn headless_fingers_that_start_on_one_point_cannot_pinch() {
    let mut pinch = Magnify::default();
    assert_eq!(pinch.touches((5.0, 5.0), (5.0, 5.0)), State::Failed);
}

#[test]
fn headless_a_pinch_that_never_began_fails_on_lift() {
    let mut pinch = Magnify::default();
    pinch.touches((0.0, 0.0), (100.0, 0.0));
    assert_eq!(pinch.up(), State::Failed);
}

#[test]
fn headless_a_quarter_turn_of_the_fingers_begins_a_rotation_in_degrees() {
    let mut turn = Rotate::default();
    assert_eq!(turn.touches((0.0, 0.0), (10.0, 0.0)), State::Possible);
    assert_eq!(turn.touches((0.0, 0.0), (0.0, 10.0)), State::Began);
    assert!(near(turn.rotation(), FRAC_PI_2), "a quarter turn is pi over two radians");
    assert!((turn.rotation_degrees() - 90.0).abs() < 1e-3);
    assert_eq!(turn.touches((0.0, 0.0), (-10.0, 0.0)), State::Changed);
    assert!(near(turn.rotation(), PI));
    assert_eq!(turn.up(), State::Ended);
}

#[test]
fn headless_a_rotation_crossing_the_half_turn_keeps_counting_the_short_way() {
    let mut turn = Rotate::default();
    let start = 0.9 * PI;
    let end = -0.9 * PI;
    turn.touches((0.0, 0.0), (10.0 * start.cos(), 10.0 * start.sin()));
    assert_eq!(turn.touches((0.0, 0.0), (10.0 * end.cos(), 10.0 * end.sin())), State::Began);
    assert!((turn.rotation() - 0.2 * PI).abs() < 1e-3, "0.9 pi to -0.9 pi is a 0.2 pi turn, got {}", turn.rotation());
}

#[test]
fn headless_a_turn_under_the_rotation_slop_does_not_begin() {
    let mut turn = Rotate::default();
    turn.touches((0.0, 0.0), (10.0, 0.0));
    assert_eq!(turn.touches((0.0, 0.0), (10.0, 0.1)), State::Possible, "about 0.01 radians is inside the slop");
}

#[test]
fn headless_a_rotation_that_never_began_fails_on_lift() {
    let mut turn = Rotate::default();
    turn.touches((0.0, 0.0), (10.0, 0.0));
    assert_eq!(turn.up(), State::Failed);
}

// ---- trackpad gesture: pinch and rotation as one event stream ----

#[test]
fn headless_a_two_finger_pinch_reports_began_then_changed_events() {
    let mut touch = TwoFinger::new();
    assert!(touch.frame((0.0, 0.0), (100.0, 0.0)).is_empty());
    assert_eq!(
        touch.frame((0.0, 0.0), (150.0, 0.0)),
        vec![Event { kind: Kind::Magnify, state: State::Began, value: 0.5 }]
    );
    assert_eq!(
        touch.frame((0.0, 0.0), (200.0, 0.0)),
        vec![Event { kind: Kind::Magnify, state: State::Changed, value: 1.0 }]
    );
}

#[test]
fn headless_a_two_finger_turn_reports_a_rotation_event() {
    let mut touch = TwoFinger::new();
    assert!(touch.frame((0.0, 0.0), (100.0, 0.0)).is_empty());
    let events = touch.frame((0.0, 0.0), (0.0, 100.0));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, Kind::Rotate);
    assert_eq!(events[0].state, State::Began);
    assert!(near(events[0].value, FRAC_PI_2));
}

#[test]
fn headless_one_frame_that_pinches_and_turns_reports_magnify_before_rotate() {
    let mut touch = TwoFinger::new();
    touch.frame((0.0, 0.0), (100.0, 0.0));
    let events = touch.frame((0.0, 0.0), (0.0, 150.0));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, Kind::Magnify);
    assert_eq!(events[0].state, State::Began);
    assert_eq!(events[1].kind, Kind::Rotate);
    assert_eq!(events[1].state, State::Began);
}

#[test]
fn headless_lifting_fingers_ends_each_gesture_that_began() {
    let mut touch = TwoFinger::new();
    touch.frame((0.0, 0.0), (100.0, 0.0));
    touch.frame((0.0, 0.0), (150.0, 0.0));
    touch.frame((0.0, 0.0), (200.0, 0.0));
    assert_eq!(touch.lift(), vec![Event { kind: Kind::Magnify, state: State::Ended, value: 1.0 }]);
}

#[test]
fn headless_lifting_fingers_that_never_began_reports_nothing() {
    let mut touch = TwoFinger::new();
    touch.frame((0.0, 0.0), (100.0, 0.0));
    touch.frame((0.0, 0.0), (102.0, 0.0));
    assert!(touch.lift().is_empty());
}

// ---- stylus: digitizer ranges, pen angles and stroke points ----

const PROFILE: StylusProfile = StylusProfile {
    pressure: LogicalRange { min: 0.0, max: 1024.0 },
    tilt: LogicalRange { min: -90.0, max: 90.0 },
};

fn report(pressure: f32, tilt_x: f32, tilt_y: f32) -> StylusReport {
    StylusReport { x: 3.0, y: 4.0, time_offset: 0.5, pressure, tilt_x, tilt_y, touching: true, eraser: false }
}

#[test]
fn headless_a_logical_range_maps_values_to_zero_to_one_and_clamps_outside() {
    let r = LogicalRange { min: 0.0, max: 1024.0 };
    assert!(near(r.unit(256.0), 0.25));
    assert!(near(r.unit(-1.0), 0.0), "below the range clamps to 0");
    assert!(near(r.unit(2000.0), 1.0), "above the range clamps to 1");
}

#[test]
fn headless_a_signed_range_puts_its_midpoint_at_zero() {
    let tilt = LogicalRange { min: -90.0, max: 90.0 };
    assert!(near(tilt.signed(0.0), 0.0));
    assert!(near(tilt.signed(-90.0), -1.0));
    assert!(near(tilt.signed(90.0), 1.0));
}

#[test]
fn headless_a_degenerate_range_maps_every_value_to_zero() {
    let flat = LogicalRange { min: 5.0, max: 5.0 };
    assert!(near(flat.unit(9.0), 0.0));
    assert!(near(flat.signed(9.0), 0.0));
}

#[test]
fn headless_an_upright_pen_is_vertical_with_no_azimuth() {
    let (altitude, azimuth) = pen_angles(0.0, 0.0);
    assert!(near(altitude, FRAC_PI_2));
    assert!(near(azimuth, 0.0));
}

#[test]
fn headless_a_pen_leaning_forty_five_degrees_about_x_has_forty_five_degrees_altitude() {
    let (altitude, azimuth) = pen_angles(FRAC_PI_4, 0.0);
    assert!((altitude - FRAC_PI_4).abs() < 1e-5, "altitude {altitude}");
    assert!(near(azimuth, 0.0));
}

#[test]
fn headless_a_pen_lying_flat_along_y_points_its_azimuth_along_y() {
    let (altitude, azimuth) = pen_angles(0.0, FRAC_PI_2);
    assert!(near(altitude, 0.0));
    assert!(near(azimuth, FRAC_PI_2));
}

#[test]
fn headless_a_touching_report_becomes_a_stroke_point_with_mapped_force_and_angles() {
    let p = report(512.0, 0.0, 0.0).to_point(&PROFILE).expect("a touching report is a stroke point");
    assert_eq!((p.x, p.y, p.time_offset), (3.0, 4.0, 0.5));
    assert!(near(p.force, 0.5));
    assert!(near(p.altitude, FRAC_PI_2));
    assert!(near(p.azimuth, 0.0));
    assert_eq!(p.opacity, 1.0);
    assert_eq!(p.size, (0.0, 0.0));
}

#[test]
fn headless_pressure_above_the_device_range_clamps_to_full_force() {
    let p = report(4096.0, 0.0, 0.0).to_point(&PROFILE).expect("touching");
    assert!(near(p.force, 1.0));
}

#[test]
fn headless_a_full_tilt_on_the_device_range_lies_the_pen_flat() {
    let p = report(0.0, 90.0, 0.0).to_point(&PROFILE).expect("touching");
    assert!(near(p.altitude, 0.0));
    assert!(near(p.force, 0.0));
}

#[test]
fn headless_a_negative_tilt_points_the_azimuth_the_other_way() {
    let p = report(0.0, -90.0, 0.0).to_point(&PROFILE).expect("touching");
    assert!(near(p.altitude, 0.0));
    assert!((p.azimuth.abs() - PI).abs() < 1e-5, "azimuth {}", p.azimuth);
}

#[test]
fn headless_hover_reports_are_not_stroke_points_and_the_eraser_end_maps_like_the_pen() {
    let mut hover = report(800.0, 0.0, 0.0);
    hover.touching = false;
    assert!(hover.to_point(&PROFILE).is_none());
    let pen = report(512.0, 0.0, 0.0).to_point(&PROFILE).expect("touching");
    let mut eraser = report(512.0, 0.0, 0.0);
    eraser.eraser = true;
    let erased = eraser.to_point(&PROFILE).expect("touching");
    assert!(eraser.eraser, "the eraser flag is carried on the report");
    assert!(near(erased.force, pen.force));
    assert!(near(erased.altitude, pen.altitude));
}

// ---- scribble: tap, scratch-out and writing over views ----

fn stroke(points: &[(f32, f32)]) -> Stroke {
    Stroke {
        ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 },
        points: points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect(),
    }
}

fn zigzag(amplitude: f32, turns: usize) -> Stroke {
    let pts: Vec<(f32, f32)> = (0..=turns * 2)
        .map(|i| (if i % 2 == 0 { 0.0 } else { amplitude }, i as f32 * 5.0))
        .collect();
    stroke(&pts)
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn headless_a_stroke_without_points_has_no_scribble_gesture() {
    assert_eq!(classify(&stroke(&[])), None);
}

#[test]
fn headless_a_stroke_no_longer_than_the_tap_extent_is_a_tap() {
    let at_limit = stroke(&[(10.0, 10.0), (10.0 + TAP_EXTENT, 10.0)]);
    assert_eq!(classify(&at_limit), Some(Gesture::Tap));
    assert_eq!(classify(&stroke(&[(10.0, 10.0)])), Some(Gesture::Tap));
}

#[test]
fn headless_a_straight_stroke_is_writing() {
    let line = stroke(&[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)]);
    assert_eq!(classify(&line), Some(Gesture::Write));
}

#[test]
fn headless_a_tight_zigzag_is_a_scratch_out() {
    assert_eq!(SCRATCH_MIN_REVERSALS, 3);
    assert_eq!(classify(&zigzag(30.0, 4)), Some(Gesture::ScratchOut));
}

#[test]
fn headless_steps_under_the_jitter_limit_are_not_turns() {
    let shaky = stroke(&[(0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (40.0, 0.0)]);
    assert_eq!(classify(&shaky), Some(Gesture::Write));
}

#[test]
fn headless_a_zigzag_with_too_few_turns_is_writing() {
    let two_turns = stroke(&[(0.0, 0.0), (40.0, 0.0), (0.0, 0.0), (40.0, 0.0)]);
    assert_eq!(classify(&two_turns), Some(Gesture::Write));
}

#[test]
fn headless_a_tap_selects_the_topmost_view_under_it() {
    let targets = [(1, rect(0.0, 0.0, 100.0, 100.0)), (2, rect(20.0, 20.0, 40.0, 40.0))];
    assert_eq!(resolve(&stroke(&[(30.0, 30.0)]), &targets), Action::Tap(2));
}

#[test]
fn headless_a_tap_outside_every_view_is_ignored() {
    let targets = [(1, rect(0.0, 0.0, 10.0, 10.0))];
    assert_eq!(resolve(&stroke(&[(50.0, 50.0)]), &targets), Action::Ignored);
}

#[test]
fn headless_a_scratch_out_deletes_each_view_it_crosses_bottom_first() {
    let targets = [
        (7, rect(-5.0, 0.0, 20.0, 60.0)),
        (8, rect(200.0, 200.0, 10.0, 10.0)),
        (9, rect(25.0, 0.0, 20.0, 60.0)),
    ];
    assert_eq!(resolve(&zigzag(30.0, 4), &targets), Action::Delete(vec![7, 9]));
}

#[test]
fn headless_a_scratch_out_over_empty_space_is_ignored() {
    let targets = [(1, rect(500.0, 500.0, 10.0, 10.0))];
    assert_eq!(resolve(&zigzag(30.0, 4), &targets), Action::Ignored);
}

#[test]
fn headless_writing_goes_to_the_view_under_the_centre_of_the_stroke() {
    let targets = [(3, rect(0.0, 0.0, 200.0, 40.0))];
    let line = stroke(&[(10.0, 10.0), (50.0, 12.0), (90.0, 14.0)]);
    assert_eq!(resolve(&line, &targets), Action::Write(3));
}

#[test]
fn headless_overlapping_views_receive_writing_on_the_top_one() {
    let targets = [(1, rect(0.0, 0.0, 200.0, 40.0)), (2, rect(0.0, 0.0, 200.0, 40.0))];
    let line = stroke(&[(10.0, 10.0), (50.0, 12.0), (90.0, 14.0)]);
    assert_eq!(resolve(&line, &targets), Action::Write(2));
}

#[test]
fn headless_with_no_views_every_scribble_is_ignored() {
    assert_eq!(resolve(&stroke(&[(1.0, 1.0)]), &[]), Action::Ignored);
}
