//! Parity sdk-tests-11: headless tests of two pure models, the dated widget text and the scene lifecycle clock.
//! Both models take the time as an argument and read no clock, no window and no display. Every expected value is
//! worked out by hand from the arithmetic in the model.

use bifrost_ui::keys::Key;
use bifrost_ui::panels::{hsb_to_rgb, parse_hex, rgb_to_hsb, to_hex, ColorPanel, FontPanel, FontSelection, Hsb, FONT_SIZES};
use bifrost_ui::scene_loop::{SceneLoop, Step, DEFAULT_MAX_STEP, FRAME_STEPS};
use bifrost_ui::sheet::{ButtonRole, Hit, Sheet, SheetButton, ALERT_W_PT};
use bifrost_ui::widget_date::{next_change, text, Style};
use bifrost_ui::Rect;
use std::cell::Cell;
use std::rc::Rc;

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

// --- sheet: alert geometry, default and cancel buttons, keys and hit tests ---

const PARENT: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
const TITLEBAR_H: f32 = 28.0;

fn confirm_sheet() -> Sheet {
    Sheet::alert(
        "Delete this note?",
        "It cannot be recovered.",
        vec![
            SheetButton::new(1, "Cancel", ButtonRole::Cancel),
            SheetButton::new(2, "Delete", ButtonRole::Destructive),
            SheetButton::new(3, "Keep", ButtonRole::Default),
        ],
    )
}

fn button_centre(layout: &[(u32, Rect)], id: u32) -> (f32, f32) {
    let (_, r) = layout.iter().find(|(b, _)| *b == id).expect("the button is laid out");
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

#[test]
fn an_alert_sheet_is_as_wide_as_the_alert_and_grows_with_its_message() {
    let empty = Sheet::alert("Title", "", vec![]);
    assert_eq!(empty.size, (ALERT_W_PT, 96.0), "no message: padding, title, gap and a button row");
    let one_line = Sheet::alert("Title", &"a".repeat(34), vec![]);
    assert_eq!(one_line.size.1, 119.0, "34 characters fit one line");
    let two_lines = Sheet::alert("Title", &"a".repeat(35), vec![]);
    assert_eq!(two_lines.size.1, 134.0, "35 characters wrap to a second line");
    assert!(two_lines.alert);
}

#[test]
fn a_sheet_reports_its_default_and_cancel_buttons_by_role() {
    let sheet = confirm_sheet();
    assert_eq!(sheet.default_button(), Some(3));
    assert_eq!(sheet.cancel_button(), Some(1));
    let plain = Sheet::new("Plain", (300.0, 200.0), vec![SheetButton::new(9, "OK", ButtonRole::Normal)]);
    assert_eq!(plain.default_button(), None);
    assert_eq!(plain.cancel_button(), None);
    assert!(!plain.alert);
}

#[test]
fn return_answers_the_default_button_and_escape_the_cancel_button() {
    let sheet = confirm_sheet();
    assert_eq!(sheet.key(Key::Return), (Some(3), true));
    assert_eq!(sheet.key(Key::Escape), (Some(1), true));
}

#[test]
fn a_sheet_without_a_cancel_button_ignores_escape_but_still_consumes_it() {
    let sheet = Sheet::new("Plain", (300.0, 200.0), vec![SheetButton::new(9, "OK", ButtonRole::Normal)]);
    assert_eq!(sheet.key(Key::Escape), (None, true), "nothing reaches the blocked parent");
}

#[test]
fn a_click_on_a_button_hits_that_button() {
    let sheet = confirm_sheet();
    let layout = sheet.layout(PARENT, TITLEBAR_H, 1.0);
    let (x, y) = button_centre(&layout.buttons, 2);
    assert_eq!(sheet.hit(PARENT, TITLEBAR_H, 1.0, x, y), Hit::Button(2));
}

#[test]
fn a_click_on_the_sheet_body_hits_the_panel_and_a_click_on_the_dimmed_parent_is_blocked() {
    let sheet = confirm_sheet();
    let layout = sheet.layout(PARENT, TITLEBAR_H, 1.0);
    let panel = layout.panel;
    assert_eq!(sheet.hit(PARENT, TITLEBAR_H, 1.0, panel.x + 2.0, panel.y + 2.0), Hit::Panel);
    assert_eq!(sheet.hit(PARENT, TITLEBAR_H, 1.0, PARENT.x + 1.0, PARENT.y + PARENT.h - 1.0), Hit::Blocked);
    assert_eq!(sheet.hit(PARENT, TITLEBAR_H, 1.0, PARENT.x + PARENT.w + 10.0, PARENT.y), Hit::Outside);
}

// --- font panel: family and style selection, size clamping and change callbacks ---

fn font_panel() -> FontPanel {
    let families = vec![
        ("Helvetica".to_string(), vec!["Regular".to_string(), "Bold".to_string()]),
        ("Menlo".to_string(), vec!["Regular".to_string()]),
    ];
    FontPanel::new(families, FontSelection { family: "Helvetica".into(), style: "Bold".into(), size: 13.0 })
}

#[test]
fn choosing_a_family_keeps_the_style_when_the_family_has_it() {
    let mut panel = font_panel();
    assert!(panel.set_family("Menlo"));
    assert_eq!(panel.selection().style, "Regular", "Menlo has no Bold, so its first style is used");
    assert!(panel.set_family("Helvetica"));
    assert_eq!(panel.selection().style, "Regular", "Regular is kept because Helvetica has it");
    assert_eq!(panel.selection().family, "Helvetica");
}

#[test]
fn an_unknown_family_or_style_is_refused_and_changes_nothing() {
    let mut panel = font_panel();
    assert!(!panel.set_family("Nowhere"));
    assert_eq!(panel.selection().family, "Helvetica");
    assert!(panel.set_style("Bold"));
    assert!(!panel.set_style("Italic"));
    assert_eq!(panel.selection().style, "Bold");
}

#[test]
fn the_size_is_clamped_to_the_field_range() {
    let mut panel = font_panel();
    panel.set_size(0.0);
    assert_eq!(panel.selection().size, 1.0);
    panel.set_size(500.0);
    assert_eq!(panel.selection().size, 288.0);
    panel.set_size(14.5);
    assert_eq!(panel.selection().size, 14.5);
    assert!(FONT_SIZES.contains(&13.0) && FONT_SIZES.len() == 14);
}

#[test]
fn the_change_callback_runs_after_each_accepted_change_only() {
    let mut panel = font_panel();
    let calls = Rc::new(Cell::new(0u32));
    let seen = Rc::clone(&calls);
    panel.on_change(move |_| seen.set(seen.get() + 1));
    panel.set_family("Nowhere");
    panel.set_style("Italic");
    assert_eq!(calls.get(), 0, "refused changes do not notify");
    panel.set_family("Menlo");
    panel.set_size(12.0);
    assert_eq!(calls.get(), 2);
}

// --- colour panel: hex text, hue, saturation and brightness, alpha and callbacks ---

#[test]
fn a_colour_panel_shows_its_colour_as_upper_case_hex() {
    let panel = ColorPanel::new((255, 0, 0));
    assert_eq!(panel.color(), (255, 0, 0));
    assert_eq!(panel.hex(), "#FF0000");
}

#[test]
fn a_hex_field_accepts_six_and_three_digit_colours_and_refuses_the_rest() {
    let mut panel = ColorPanel::new((255, 0, 0));
    assert!(panel.set_hex("#00ff00"));
    assert_eq!(panel.color(), (0, 255, 0));
    assert!(panel.set_hex("#abc"));
    assert_eq!(panel.color(), (170, 187, 204), "three digits repeat each digit");
    assert!(!panel.set_hex("zz"));
    assert_eq!(panel.color(), (170, 187, 204), "invalid text changes nothing");
}

#[test]
fn hex_parsing_trims_space_and_the_hash_and_round_trips_through_to_hex() {
    assert_eq!(parse_hex("  #0a0b0c "), Some((10, 11, 12)));
    assert_eq!(parse_hex("12345"), None);
    assert_eq!(to_hex((1, 2, 255)), "#0102FF");
    assert_eq!(parse_hex(&to_hex((9, 128, 200))), Some((9, 128, 200)));
}

#[test]
fn hue_saturation_and_brightness_convert_the_primary_colours() {
    assert_eq!(hsb_to_rgb(Hsb { h: 0.0, s: 1.0, b: 1.0 }), (255, 0, 0));
    assert_eq!(hsb_to_rgb(Hsb { h: 120.0, s: 1.0, b: 1.0 }), (0, 255, 0));
    let blue = rgb_to_hsb(0, 0, 255);
    assert!((blue.h - 240.0).abs() < 0.01);
    assert!(rgb_to_hsb(255, 255, 255).s.abs() < 0.001, "white has no saturation");
    assert!(rgb_to_hsb(0, 0, 0).b.abs() < 0.001, "black has no brightness");
}

#[test]
fn a_colour_survives_a_round_trip_through_hue_saturation_and_brightness() {
    for c in [(255u8, 0u8, 0u8), (12, 200, 99)] {
        let back = hsb_to_rgb(rgb_to_hsb(c.0, c.1, c.2));
        for (a, b) in [(back.0, c.0), (back.1, c.1), (back.2, c.2)] {
            assert!(a.abs_diff(b) <= 1, "{back:?} is not {c:?}");
        }
    }
}

#[test]
fn setting_a_colour_by_hue_saturation_and_brightness_updates_the_panel() {
    let mut panel = ColorPanel::new((0, 0, 0));
    panel.set_hsb(Hsb { h: 240.0, s: 1.0, b: 1.0 });
    assert_eq!(panel.color(), (0, 0, 255));
    assert_eq!(panel.hex(), "#0000FF");
}

#[test]
fn alpha_is_clamped_between_transparent_and_opaque() {
    let mut panel = ColorPanel::new((0, 0, 0));
    panel.set_alpha(2.0);
    assert_eq!(panel.alpha(), 1.0);
    panel.set_alpha(-1.0);
    assert_eq!(panel.alpha(), 0.0);
}

#[test]
fn the_colour_callback_receives_each_change_with_the_alpha() {
    let mut panel = ColorPanel::new((0, 0, 0));
    let last = Rc::new(Cell::new((0u8, 0f32)));
    let seen = Rc::clone(&last);
    panel.on_change(move |c, a| seen.set((c.0, a)));
    panel.set_rgb((9, 8, 7));
    assert_eq!(last.get(), (9, panel.alpha()));
    panel.set_alpha(0.5);
    assert_eq!(last.get(), (9, 0.5));
}
