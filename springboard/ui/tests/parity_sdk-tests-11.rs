//! Parity sdk-tests-11: headless tests of two pure models, the dated widget text and the scene lifecycle clock.
//! Both models take the time as an argument and read no clock, no window and no display. Every expected value is
//! worked out by hand from the arithmetic in the model.

use bifrost_ui::scene_loop::{SceneLoop, Step, DEFAULT_MAX_STEP, FRAME_STEPS};
use bifrost_ui::widget_date::{next_change, text, Style};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// --- widget_date: the text shown for a target date at a moment ---

#[test]
fn a_future_target_reads_as_a_relative_phrase_from_now() {
    assert_eq!(text(Style::Relative, 130.0, 0.0), "in 2 minutes");
    assert_eq!(text(Style::Relative, 130.0, 10.0), "in 2 minutes", "the distance is exactly 120 s here");
}

#[test]
fn a_past_target_reads_as_a_relative_phrase_ago() {
    assert_eq!(text(Style::Relative, -7200.0, 0.0), "2 hours ago");
    assert_eq!(text(Style::Relative, -130.0, 0.0), "2 minutes ago");
}

#[test]
fn a_fraction_of_a_second_is_cut_off_before_the_wording_is_chosen() {
    assert_eq!(text(Style::Relative, 59.9, 0.0), "in 59 seconds");
    assert_eq!(text(Style::Timer, 59.9, 0.0), "0:59");
}

#[test]
fn a_timer_shows_the_distance_as_a_clock_without_a_sign() {
    assert_eq!(text(Style::Timer, 3725.4, 0.0), "1:02:05");
    assert_eq!(text(Style::Timer, -65.0, 0.0), "1:05", "a past target shows the same clock");
    assert_eq!(text(Style::Timer, 500.0, 500.0), "0:00");
}

#[test]
fn a_timer_counts_down_as_the_moment_moves_forward() {
    assert_eq!(text(Style::Timer, 100.0, 40.0), "1:00");
    assert_eq!(text(Style::Timer, 100.0, 99.0), "0:01");
}

#[test]
fn a_clock_changes_on_the_next_whole_second() {
    assert_eq!(next_change(Style::Timer, 500.0, 7.5), 8.0);
    assert_eq!(next_change(Style::Timer, 500.0, 100.0), 101.0, "an exact second still waits one full second");
}

#[test]
fn a_future_relative_target_changes_at_the_multiple_just_below_its_distance() {
    assert_eq!(next_change(Style::Relative, 130.0, 0.0), 10.0);
    assert_eq!(next_change(Style::Relative, 130.0, 10.0), 70.0, "120 s away at 10 s, the next multiple is 60 s");
}

#[test]
fn a_past_relative_target_changes_at_the_multiple_just_above_its_distance() {
    assert_eq!(next_change(Style::Relative, -130.0, 0.0), 50.0);
}

#[test]
fn the_relative_text_changes_at_the_reported_instant() {
    let target = 130.0;
    let at = next_change(Style::Relative, target, 0.0);
    assert_eq!(text(Style::Relative, target, at - 0.5), "in 2 minutes");
    assert_eq!(text(Style::Relative, target, at + 0.5), "in 1 minute");
}

// --- scene_loop: presentation, pause, clamped frame time and the ordered steps ---

#[test]
fn a_new_scene_is_not_presented_and_runs_no_frame() {
    let mut scene = SceneLoop::new(640.0, 480.0);
    assert!(!scene.is_presented());
    assert!(!scene.is_paused());
    assert_eq!(scene.size(), (640.0, 480.0));
    assert_eq!(scene.frame(0.0), None, "a scene that is not presented runs nothing");
}

#[test]
fn presenting_and_dismissing_report_each_change_once() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    assert!(scene.present());
    assert!(!scene.present(), "a second present changes nothing");
    assert!(scene.dismiss());
    assert!(!scene.dismiss(), "a second dismiss changes nothing");
    assert_eq!(scene.frame(1.0), None, "no frame runs after the scene is dismissed");
}

#[test]
fn a_presented_scene_runs_the_five_steps_in_the_fixed_order() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    let frame = scene.frame(1.0).unwrap();
    assert_eq!(
        frame.steps,
        &[Step::Update, Step::EvaluateActions, Step::SimulatePhysics, Step::ApplyConstraints, Step::FinishUpdate]
    );
    assert_eq!(frame.steps, &FRAME_STEPS[..]);
}

#[test]
fn the_first_frame_after_presenting_advances_by_nothing() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    let frame = scene.frame(1.0).unwrap();
    assert_eq!(frame.delta, 0.0);
    assert_eq!(frame.current_time, 0.0);
}

#[test]
fn each_frame_advances_the_clock_by_the_elapsed_time_clamped_to_the_max_step() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    scene.frame(1.0);
    let long = scene.frame(1.25).unwrap();
    assert!(near(long.delta, DEFAULT_MAX_STEP), "0.25 s elapsed is clamped to the default step");
    assert!(near(long.current_time, DEFAULT_MAX_STEP));
    let short = scene.frame(1.30).unwrap();
    assert!(near(short.delta, 0.05), "0.05 s elapsed is used as it is");
    assert!(near(short.current_time, DEFAULT_MAX_STEP + 0.05));
}

#[test]
fn a_custom_max_step_changes_the_clamp_and_invalid_values_are_refused() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    assert!(scene.set_max_step(0.5));
    assert!(near(scene.max_step(), 0.5));
    scene.present();
    scene.frame(0.0);
    let frame = scene.frame(0.4).unwrap();
    assert!(near(frame.delta, 0.4), "0.4 s is below the 0.5 s clamp");
    assert!(!scene.set_max_step(0.0));
    assert!(!scene.set_max_step(-1.0));
    assert!(!scene.set_max_step(f64::NAN));
    assert!(!scene.set_max_step(f64::INFINITY));
    assert!(near(scene.max_step(), 0.5), "refused values leave the clamp alone");
}

#[test]
fn paused_frames_run_no_steps_and_the_clock_stops() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    scene.frame(1.0);
    scene.frame(1.05);
    scene.set_paused(true);
    assert!(scene.is_paused());
    let paused = scene.frame(2.0).unwrap();
    assert!(paused.steps.is_empty());
    assert_eq!(paused.delta, 0.0);
    assert!(near(paused.current_time, 0.05));
}

#[test]
fn resuming_counts_only_the_time_since_the_last_paused_frame() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    scene.frame(1.0);
    scene.frame(1.05);
    scene.set_paused(true);
    scene.frame(2.0);
    scene.set_paused(false);
    let resumed = scene.frame(2.05).unwrap();
    assert!(near(resumed.delta, 0.05), "the second spent paused does not count");
    assert!(near(resumed.current_time, 0.10));
    assert_eq!(resumed.steps.len(), 5);
}

#[test]
fn a_clock_that_goes_backwards_counts_as_no_time_and_still_runs_the_steps() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    scene.frame(5.0);
    let frame = scene.frame(4.0).unwrap();
    assert_eq!(frame.delta, 0.0);
    assert_eq!(frame.steps.len(), 5);
}

#[test]
fn a_new_presentation_starts_the_scene_clock_from_zero() {
    let mut scene = SceneLoop::new(10.0, 10.0);
    scene.present();
    scene.frame(1.0);
    scene.frame(1.05);
    assert!(near(scene.current_time(), 0.05));
    scene.dismiss();
    scene.present();
    assert_eq!(scene.current_time(), 0.0);
    let frame = scene.frame(10.0).unwrap();
    assert_eq!(frame.delta, 0.0);
    assert_eq!(frame.current_time, 0.0);
}

#[test]
fn resizing_reports_only_real_changes_and_takes_negative_sizes_as_zero() {
    let mut scene = SceneLoop::new(100.0, 50.0);
    assert!(!scene.resize(100.0, 50.0), "the same size is not a change");
    assert!(scene.resize(200.0, 50.0));
    assert_eq!(scene.size(), (200.0, 50.0));
    assert!(scene.resize(-5.0, 50.0));
    assert_eq!(scene.size(), (0.0, 50.0));
}

#[test]
fn a_negative_initial_size_is_taken_as_zero() {
    let scene = SceneLoop::new(-3.0, -4.0);
    assert_eq!(scene.size(), (0.0, 0.0));
}

#[test]
fn a_scene_can_be_resized_while_presented_and_keeps_running() {
    let mut scene = SceneLoop::new(100.0, 100.0);
    scene.present();
    scene.frame(0.0);
    assert!(scene.resize(320.0, 240.0));
    let frame = scene.frame(0.05).unwrap();
    assert!(near(frame.delta, 0.05));
    assert_eq!(scene.size(), (320.0, 240.0));
}
