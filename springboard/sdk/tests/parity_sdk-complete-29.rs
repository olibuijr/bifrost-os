//! Parity sdk-complete-29: headless tests for the models behind the scripting attach, audio player and recording
//! permission, mixing, camera region and capture filter rows. Every test runs the public API of a pure model and
//! asserts concrete values. None opens a socket, touches a device, reads the clock or needs a display.

use bifrost_app::audio_application::{AudioApplication, RecordPermission};
use bifrost_app::audio_player_control::{clamp_rate, PlayerControl, VolumeFade, RATE_MAX, RATE_MIN};
use bifrost_app::script_target::{attach, socket_path, AttachError, Locator, RunningApp};
use bifrost_ui::camera_region::Stabilizer;
use bifrost_ui::capture_content::{CaptureDisplay, CaptureWindow, ShareableContent};
use bifrost_ui::capture_filter::{ContentFilter, Rule, Scope};
use bifrost_ui::playback_arbiter::{Arbiter, ArbiterError};
use bifrost_ui::Rect;
use std::path::{Path, PathBuf};

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn running(id: &str, pid: Option<u32>, url: Option<&str>) -> RunningApp {
    RunningApp { id: id.into(), pid, url: url.map(String::from) }
}

// Scripting attach: an app is named by bundle id, process id or bundle URL.

#[test]
fn a_bundle_id_attaches_to_the_first_matching_running_app() {
    let apps = vec![running("is.example.notes", Some(412), None), running("is.example.notes", Some(900), None)];
    let target = attach(&Locator::BundleId("is.example.notes".into()), &apps).unwrap();
    assert_eq!(target.pid, Some(412), "the shell's list order decides between two instances");
}

#[test]
fn a_process_id_attaches_to_that_process_and_no_other() {
    let apps = vec![running("is.example.notes", Some(412), None), running("is.example.files", Some(977), None)];
    assert_eq!(attach(&Locator::Pid(977), &apps).unwrap().id, "is.example.files");
    assert_eq!(attach(&Locator::Pid(978), &apps), Err(AttachError::NotRunning(Locator::Pid(978))));
}

#[test]
fn a_bundle_url_attaches_whether_or_not_it_ends_in_a_slash() {
    let apps = vec![running("is.example.notes", Some(412), Some("file:///apps/notes/"))];
    let plain = attach(&Locator::Url("file:///apps/notes".into()), &apps).unwrap();
    let slashed = attach(&Locator::Url("file:///apps/notes/".into()), &apps).unwrap();
    assert_eq!(plain, slashed);
    assert_eq!(plain.url.as_deref(), Some("file:///apps/notes/"));
}

#[test]
fn an_app_without_a_reported_pid_or_url_is_found_by_id_only() {
    let apps = vec![running("is.example.clock", None, None)];
    assert_eq!(attach(&Locator::Pid(9), &apps), Err(AttachError::NotRunning(Locator::Pid(9))));
    assert_eq!(attach(&Locator::Url("file:///apps/clock".into()), &apps), Err(AttachError::NotRunning(Locator::Url("file:///apps/clock".into()))));
    assert_eq!(attach(&Locator::BundleId("is.example.clock".into()), &apps).unwrap().pid, None);
}

#[test]
fn a_locator_that_names_nothing_is_invalid_not_missing() {
    let apps = vec![running("is.example.notes", Some(412), None)];
    assert_eq!(attach(&Locator::BundleId(String::new()), &apps), Err(AttachError::Invalid(Locator::BundleId(String::new()))));
    assert_eq!(attach(&Locator::Url("/".into()), &apps), Err(AttachError::Invalid(Locator::Url("/".into()))));
    assert_eq!(attach(&Locator::Pid(0), &apps), Err(AttachError::Invalid(Locator::Pid(0))));
}

#[test]
fn attach_errors_say_which_app_was_looked_for() {
    let missing = AttachError::NotRunning(Locator::BundleId("is.example.absent".into()));
    assert_eq!(missing.to_string(), "no running app matches bundle id \"is.example.absent\"");
    assert_eq!(AttachError::Invalid(Locator::Pid(0)).to_string(), "invalid target: process id 0");
}

#[test]
fn an_empty_running_list_attaches_to_nothing() {
    assert!(matches!(attach(&Locator::Pid(412), &[]), Err(AttachError::NotRunning(_))));
}

#[test]
fn the_control_socket_is_the_explicit_path_when_one_is_given() {
    let p = socket_path("is.example.notes", Some(Path::new("/run/custom/notes.sock")), Some(Path::new("/run/user/1000")), Path::new("/data"));
    assert_eq!(p, PathBuf::from("/run/custom/notes.sock"));
}

#[test]
fn the_control_socket_is_per_app_under_the_runtime_directory() {
    let p = socket_path("is.example.notes", None, Some(Path::new("/run/user/1000")), Path::new("/data"));
    assert_eq!(p, PathBuf::from("/run/user/1000/bifrost/apps/is.example.notes.sock"));
}

#[test]
fn the_control_socket_falls_back_to_the_data_directory() {
    let p = socket_path("is.example.notes", None, None, Path::new("/home/u/notes"));
    assert_eq!(p, PathBuf::from("/home/u/notes/control.sock"));
}

// Audio player control: pan, rate and timed volume fades.

#[test]
fn a_player_starts_at_full_volume_centre_pan_and_normal_rate() {
    let p = PlayerControl::new();
    assert_eq!((p.volume_at(0), p.pan(), p.rate()), (1.0, 0.0, 1.0));
    assert!(!p.is_fading(0));
}

#[test]
fn pan_and_rate_are_clamped_and_nan_resets_them() {
    let mut p = PlayerControl::new();
    p.set_pan(3.0);
    assert_eq!(p.pan(), 1.0);
    p.set_pan(f32::NAN);
    assert_eq!(p.pan(), 0.0);
    p.set_rate(0.1);
    assert_eq!(p.rate(), RATE_MIN);
    p.set_rate(9.0);
    assert_eq!(p.rate(), RATE_MAX);
    assert_eq!(clamp_rate(f32::NAN), 1.0);
}

#[test]
fn a_fade_ramps_linearly_from_the_volume_in_effect_to_the_target() {
    let mut p = PlayerControl::new();
    p.set_volume(0.2);
    p.fade_volume(1.0, 1000, 500);
    assert!((p.volume_at(500) - 0.2).abs() < 1e-6, "the ramp starts where the volume was");
    assert!((p.volume_at(1000) - 0.6).abs() < 1e-6, "halfway through a 0.2 to 1.0 ramp is 0.6");
    assert!(p.is_fading(1499));
    assert_eq!(p.volume_at(1500), 1.0);
    assert!(!p.is_fading(1500));
}

#[test]
fn a_zero_length_fade_is_immediate_and_setting_the_volume_cancels_a_fade() {
    let mut p = PlayerControl::new();
    p.fade_volume(0.0, 0, 100);
    assert_eq!(p.volume_at(100), 0.0);
    p.fade_volume(0.0, 5000, 200);
    p.set_volume(0.5);
    assert_eq!(p.volume_at(300), 0.5);
    assert!(!p.is_fading(300));
}

#[test]
fn a_fade_clamps_its_endpoints_to_the_volume_range() {
    let fade = VolumeFade::new(-1.0, 4.0, 0, 100);
    assert_eq!(fade.volume_at(0), 0.0);
    assert_eq!(fade.target(), 1.0);
    assert_eq!(fade.volume_at(100), 1.0);
}

// Audio application: microphone permission and input mute.

#[test]
fn the_permission_is_asked_only_while_undetermined() {
    let mut app = AudioApplication::new();
    let mut asked = 0;
    assert_eq!(app.request_record_permission(|| { asked += 1; true }), RecordPermission::Granted);
    assert_eq!(app.request_record_permission(|| { asked += 1; false }), RecordPermission::Granted);
    assert_eq!(asked, 1, "a stored answer is returned without asking again");
}

#[test]
fn a_refused_prompt_is_stored_as_denied() {
    let mut app = AudioApplication::new();
    assert_eq!(app.request_record_permission(|| false), RecordPermission::Denied);
    assert!(!app.may_capture());
}

#[test]
fn a_denied_permission_changes_only_through_settings() {
    let mut app = AudioApplication::new();
    app.request_record_permission(|| false);
    assert_eq!(app.request_record_permission(|| true), RecordPermission::Denied);
    app.set_permission_from_settings(RecordPermission::Granted);
    assert!(app.may_capture());
}

#[test]
fn input_mute_queues_one_event_per_real_change() {
    let mut app = AudioApplication::new();
    app.request_record_permission(|| true);
    assert!(app.set_input_muted(true));
    assert!(!app.set_input_muted(true), "setting the same state is not a change");
    assert!(app.set_input_muted(false));
    assert_eq!(app.take_mute_events(), vec![true, false]);
    assert!(app.take_mute_events().is_empty());
}

#[test]
fn capture_needs_both_permission_and_an_unmuted_input() {
    let mut app = AudioApplication::new();
    app.request_record_permission(|| true);
    assert!(app.may_capture());
    app.set_input_muted(true);
    assert!(!app.may_capture());
}

// Playback mixing: each source has a volume and pan, and the sum is clamped.

#[test]
fn a_source_from_an_unregistered_participant_is_left_out_of_the_mix() {
    let arbiter = Arbiter::new();
    let loud = [1.0_f32; 8];
    assert!(arbiter.mix(&[("is.example.ghost", &loud)], 4).iter().all(|s| *s == 0.0));
}

#[test]
fn a_registered_source_is_scaled_by_its_volume() {
    let mut arbiter = Arbiter::new();
    arbiter.register("is.example.player");
    arbiter.set_source_volume("is.example.player", 0.5).unwrap();
    let block = [0.8_f32, 0.8];
    let out = arbiter.mix(&[("is.example.player", &block)], 1);
    assert!((out[0] - 0.4).abs() < 1e-3, "left channel at half volume, got {}", out[0]);
    assert!((out[1] - 0.4).abs() < 1e-3, "right channel at half volume, got {}", out[1]);
}

#[test]
fn overlapping_loud_sources_are_clamped_to_full_scale() {
    let mut arbiter = Arbiter::new();
    arbiter.register("a");
    arbiter.register("b");
    let loud = [1.0_f32; 2];
    let out = arbiter.mix(&[("a", &loud), ("b", &loud)], 1);
    assert_eq!(out, vec![1.0, 1.0]);
}

#[test]
fn a_short_source_is_padded_with_silence() {
    let mut arbiter = Arbiter::new();
    arbiter.register("a");
    let one_frame = [0.5_f32, 0.5];
    let out = arbiter.mix(&[("a", &one_frame)], 3);
    assert_eq!(out.len(), 6);
    assert!(out[2..].iter().all(|s| *s == 0.0));
}

#[test]
fn a_mix_setting_on_an_unknown_participant_is_refused() {
    let mut arbiter = Arbiter::new();
    assert_eq!(arbiter.set_source_volume("nobody", 0.3), Err(ArbiterError::Unknown));
    assert_eq!(arbiter.set_source_pan("nobody", 0.3), Err(ArbiterError::Unknown));
    assert_eq!(arbiter.source_volume("nobody"), 1.0);
}

#[test]
fn a_source_pan_is_clamped_and_nan_centres_it() {
    let mut arbiter = Arbiter::new();
    arbiter.register("a");
    arbiter.set_source_pan("a", -4.0).unwrap();
    assert_eq!(arbiter.source_pan("a"), -1.0);
    arbiter.set_source_pan("a", f32::NAN).unwrap();
    assert_eq!(arbiter.source_pan("a"), 0.0);
}

// Camera region: the stabilised region of interest, kept inside the frame.

#[test]
fn a_region_in_a_frame_that_does_not_start_at_the_origin_stays_inside_it() {
    let mut s = Stabilizer::new(rect(100.0, 50.0, 200.0, 100.0));
    assert_eq!(s.observe(rect(0.0, 0.0, 50.0, 40.0)), rect(100.0, 50.0, 50.0, 40.0));
}

#[test]
fn a_zero_sized_frame_shows_an_empty_region_at_its_origin() {
    let mut s = Stabilizer::new(rect(0.0, 0.0, 0.0, 0.0));
    assert_eq!(s.observe(rect(10.0, 10.0, 50.0, 40.0)), rect(0.0, 0.0, 0.0, 0.0));
}

#[test]
fn a_negative_dead_band_is_read_as_no_dead_band() {
    let mut s = Stabilizer::with_dead_band(rect(0.0, 0.0, 200.0, 100.0), -5.0);
    s.observe(rect(10.0, 10.0, 50.0, 40.0));
    assert_eq!(s.observe(rect(12.0, 10.0, 50.0, 40.0)), rect(11.0, 10.0, 50.0, 40.0), "a 2 point move still closes half the gap");
}

#[test]
fn a_new_frame_size_pulls_the_shown_region_back_inside() {
    let mut s = Stabilizer::new(rect(0.0, 0.0, 200.0, 100.0));
    s.observe(rect(150.0, 60.0, 50.0, 40.0));
    s.set_frame(rect(0.0, 0.0, 120.0, 100.0));
    assert_eq!(s.current(), Some(rect(70.0, 60.0, 50.0, 40.0)));
}

// Capture filter: which windows a capture takes, and the stream's scale.

fn sample_content() -> ShareableContent {
    let displays = vec![
        CaptureDisplay { id: 1, frame: rect(0.0, 0.0, 1000.0, 800.0) },
        CaptureDisplay { id: 2, frame: rect(1000.0, 0.0, 1000.0, 800.0) },
    ];
    let window = |id: u32, pid: u32, frame: Rect| CaptureWindow { id, title: format!("w{id}"), app_pid: pid, frame, on_screen: true, desktop: false };
    let windows = vec![
        window(1, 10, rect(100.0, 100.0, 200.0, 200.0)),
        window(2, 20, rect(1100.0, 100.0, 200.0, 200.0)),
        window(3, 10, rect(300.0, 300.0, 200.0, 200.0)),
    ];
    ShareableContent::new(displays, Vec::new(), windows)
}

fn ids(list: Vec<&CaptureWindow>) -> Vec<u32> {
    list.iter().map(|w| w.id).collect()
}

#[test]
fn a_new_filter_renders_at_one_pixel_per_point_without_the_menu_bar() {
    let f = ContentFilter::window(1);
    assert_eq!(f.scale(), 1.0);
    assert!(!f.include_menu_bar);
    assert_eq!(f.scope, Scope::Window(1));
}

#[test]
fn a_zero_negative_or_infinite_scale_is_read_as_one() {
    for bad in [0.0_f32, -2.0, f32::INFINITY, f32::NAN] {
        assert_eq!(ContentFilter::window(1).with_scale(bad).scale(), 1.0, "scale {bad}");
    }
    assert_eq!(ContentFilter::window(1).with_scale(2.0).scale(), 2.0);
}

#[test]
fn the_menu_bar_and_scale_builders_change_only_their_own_field() {
    let f = ContentFilter::display(1, Rule::IncludeWindows(vec![1])).with_menu_bar(true).with_scale(1.5625);
    assert!(f.include_menu_bar);
    assert_eq!(f.point_pixel_scale, 1.5625);
    assert_eq!(f.scope, Scope::Display { display: 1, rule: Rule::IncludeWindows(vec![1]) });
}

#[test]
fn a_window_filter_captures_that_window_wherever_it_is() {
    let c = sample_content();
    assert_eq!(ids(ContentFilter::window(2).captured(&c)), vec![2]);
    assert!(ContentFilter::window(99).captured(&c).is_empty());
}

#[test]
fn an_app_rule_on_a_display_captures_that_apps_windows_on_it() {
    let c = sample_content();
    let f = ContentFilter::display(1, Rule::IncludeApps { pids: vec![10], except: vec![] });
    assert_eq!(ids(f.captured(&c)), vec![1, 3]);
}

#[test]
fn an_excluded_window_is_kept_even_when_its_app_is_excluded() {
    let c = sample_content();
    let f = ContentFilter::display(1, Rule::ExcludeApps { pids: vec![10], except: vec![3] });
    assert_eq!(ids(f.captured(&c)), vec![3]);
}

#[test]
fn a_display_filter_on_a_display_with_no_matching_windows_captures_nothing() {
    let c = sample_content();
    let f = ContentFilter::display(2, Rule::IncludeApps { pids: vec![10], except: vec![] });
    assert!(f.captured(&c).is_empty());
    assert!(!f.captures_window(&c, 1));
}
