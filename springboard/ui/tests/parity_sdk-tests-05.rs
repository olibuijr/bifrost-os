//! Headless tests for the pure models behind the animation, render loop and angle rows of the parity matrix:
//! curves and springs (`bifrost_ui::anim`), the render loop (`bifrost_ui::render_loop`) and typed angles
//! (`bifrost_ui::angle`). The models read no clock and draw nothing, so these run without a display, a device or
//! the network, and every value is asserted against a concrete number.
use bifrost_ui::angle::Angle;
use bifrost_ui::anim::{Animation, Curve};
use bifrost_ui::render_loop::{RenderLoop, MAX_FPS, MIN_FPS};
use std::f32::consts::{FRAC_PI_2, PI};

fn near(got: f32, want: f32) -> bool {
    (got - want).abs() < 1e-3
}

#[test]
fn headless_linear_and_bezier_curves_map_time_to_progress_one_to_one() {
    assert_eq!(Curve::Linear.at(0.25), 0.25);
    // control points (1/3, 1/3) and (2/3, 2/3) make the Bezier the identity
    assert!(near(Curve::Bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0).at(0.3), 0.3));
}

#[test]
fn headless_ease_in_lags_and_ease_out_leads_the_linear_curve() {
    assert!(Curve::EaseIn.at(0.25) < 0.25);
    assert!(Curve::EaseOut.at(0.25) > 0.25);
    assert!(near(Curve::EaseInOut.at(0.5), 0.5));
}

#[test]
fn headless_only_an_underdamped_spring_overshoots_its_target() {
    let peak = |zeta: f32| (0..=400).map(|i| Curve::Spring { damping_ratio: zeta, response: 0.5 }.at(i as f32 / 400.0)).fold(f32::MIN, f32::max);
    assert!(peak(0.3) > 1.2, "underdamped peak {}", peak(0.3));
    assert!(peak(1.0) <= 1.0 + 1e-3, "critically damped peak {}", peak(1.0));
    assert_eq!(Curve::Spring { damping_ratio: 1.0, response: 0.5 }.at(1.0), 1.0);
}

#[test]
fn headless_an_animation_interpolates_between_its_ends_and_stops() {
    let a = Animation::new(10.0, 30.0, 5.0, 2.0, Curve::Linear);
    assert_eq!(a.value(4.0), 10.0, "before the start it holds the first value");
    assert_eq!(a.value(6.0), 20.0);
    assert!(!a.is_done(6.0));
    assert!(a.is_done(7.0));
    assert_eq!(a.value(99.0), 30.0);
}

#[test]
fn headless_a_retargeted_spring_starts_where_the_old_one_is_and_keeps_moving() {
    let a = Animation::spring(0.0, 100.0, 0.0, 0.7, 0.5);
    let v = a.velocity_at(0.1);
    assert!(v > 0.0);
    let b = a.retarget(0.1, 200.0);
    assert!((b.value(0.1) - a.value(0.1)).abs() < 1e-4, "no jump at the retarget");
    assert!(b.velocity > 0.0, "the spring carries its velocity");
    assert_eq!(b.to, 200.0);
}

#[test]
fn headless_the_frame_rate_is_clamped_to_the_documented_range() {
    assert_eq!((MIN_FPS, MAX_FPS), (1, 120));
    assert_eq!(RenderLoop::new(0).preferred_fps(), MIN_FPS);
    assert_eq!(RenderLoop::new(1000).preferred_fps(), MAX_FPS);
    let mut l = RenderLoop::new(30);
    assert!(near(l.frame_interval() as f32, 1.0 / 30.0));
    l.set_preferred_fps(60);
    assert_eq!(l.preferred_fps(), 60);
}

#[test]
fn headless_a_timed_loop_draws_on_schedule_and_does_not_catch_up_after_a_late_tick() {
    let mut l = RenderLoop::new(4);
    assert!(l.tick(0.0));
    assert!(!l.tick(0.1), "the next frame is due at 0.25 s");
    assert!(l.tick(0.25));
    assert!(l.tick(1.0), "a frame three intervals late is drawn once");
    assert!(!l.tick(1.1), "the schedule restarts one interval after the late frame");
    assert_eq!(l.frames_displayed(), 3);
}

#[test]
fn headless_an_app_resigning_active_pauses_the_loop_until_it_becomes_active_again() {
    let mut l = RenderLoop::new(60);
    l.set_pause_on_resign(true);
    l.set_resume_on_active(true);
    l.app_resigned_active();
    assert!(l.is_paused());
    assert!(!l.tick(0.0));
    l.app_became_active();
    assert!(!l.is_paused());
    assert!(l.tick(0.5), "a resumed loop draws at once");
}

#[test]
fn headless_an_on_demand_loop_draws_one_frame_per_batch_of_requests() {
    let mut l = RenderLoop::new(60);
    l.set_on_demand(true);
    assert!(!l.tick(0.0), "nothing is drawn without a request");
    l.request_redraw();
    l.request_redraw();
    l.request_redraw();
    assert!(l.tick(0.0), "three requests before a tick make one frame");
    assert!(!l.tick(0.1));
    l.request_redraw();
    assert!(l.tick(0.2));
    assert_eq!(l.frames_displayed(), 2);
}

#[test]
fn headless_an_angle_reads_back_in_degrees_and_radians() {
    assert!(near(Angle::degrees(180.0).as_radians(), PI));
    assert!(near(Angle::radians(FRAC_PI_2).as_degrees(), 90.0));
    assert!(near(Angle::ZERO.as_degrees(), 0.0));
}

#[test]
fn headless_an_angle_has_the_trigonometry_of_its_radians() {
    assert!(near(Angle::degrees(90.0).sin(), 1.0));
    assert!(Angle::degrees(90.0).cos().abs() < 1e-6);
    assert!(near(Angle::degrees(45.0).tan(), 1.0));
}

#[test]
fn headless_an_angle_normalizes_into_minus_180_to_180_degrees() {
    assert!(near(Angle::degrees(270.0).normalized().as_degrees(), -90.0));
    assert!(near(Angle::degrees(-90.0).normalized().as_degrees(), -90.0));
    assert!(near(Angle::degrees(45.0).normalized().as_degrees(), 45.0));
}

#[test]
fn headless_angle_arithmetic_keeps_the_unit() {
    assert!(near((Angle::degrees(30.0) + Angle::degrees(60.0)).as_degrees(), 90.0));
    assert!(near((Angle::degrees(90.0) - Angle::degrees(30.0)).as_degrees(), 60.0));
    assert!(near((-Angle::degrees(45.0)).as_degrees(), -45.0));
    assert!(near((Angle::degrees(20.0) * 3.0).as_degrees(), 60.0));
    assert!(Angle::ZERO < Angle::degrees(1.0));
}
