//! Parity sdk-complete-34: headless tests for game controller input, symbol styling, view actions, HTTP exchange
//! logs and URL session configuration. Nothing here opens a device, a socket or a window.

use bifrost_app::gamepad_input::{axis_name, button_name, describe, Axis, AxisRange, Button, Change, ControllerInput, Gamepad};
use bifrost_app::glyph_draw::disc;
use bifrost_app::http_log::{body_size, redact_url, Exchange, Outcome};
use bifrost_app::log::Level;
use bifrost_app::symbol_draw::{paint_layer, SymbolStyle};
use bifrost_app::url::URLRequest;
use bifrost_app::url_configuration::{CacheAction, CachePolicy, URLSessionConfiguration};
use bifrost_app::view::action::{Action, Pose};
use bifrost_render::gfx::{rgba, Surface};
use bifrost_ui::symbol_layers::{RenderingMode, Scale, Weight};
use std::collections::HashSet;
use std::time::Duration;

/// `EV_KEY`, `EV_ABS` and the Linux codes the controller model reads.
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_SOUTH: u16 = 0x130;
const ABS_X: u16 = 0x00;
const ABS_Z: u16 = 0x02;
const ABS_HAT0X: u16 = 0x10;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

// Game controller input.

#[test]
fn every_button_and_axis_has_its_own_name() {
    let buttons: HashSet<&str> = Button::ALL.iter().map(|b| button_name(*b)).collect();
    assert_eq!(buttons.len(), Button::ALL.len());
    let axes: HashSet<&str> = Axis::ALL.iter().map(|a| axis_name(*a)).collect();
    assert_eq!(axes.len(), Axis::ALL.len());
}

#[test]
fn a_pressed_south_button_is_described_and_queued_once() {
    let mut input = ControllerInput::new();
    let change = input.feed(EV_KEY, BTN_SOUTH, 1).expect("a press is a change");
    assert_eq!(describe(&change), "south button pressed");
    assert_eq!(input.poll(), Some(change));
    assert_eq!(input.feed(EV_KEY, BTN_SOUTH, 1), None, "a held button reports nothing more");
}

#[test]
fn a_stick_past_its_dead_zone_is_described_with_two_decimals() {
    let mut input = ControllerInput::new();
    // raw 16384 of the signed 16-bit stick is 0.5 of the travel, which the 10 % dead zone rescales to 0.444...
    let change = input.feed(EV_ABS, ABS_X, 16384).expect("the stick moved");
    assert_eq!(describe(&change), "left stick x 0.44");
}

#[test]
fn the_direction_pad_is_described_by_its_components() {
    let mut input = ControllerInput::new();
    let change = input.feed(EV_ABS, ABS_HAT0X, -1).expect("the pad moved");
    assert_eq!(change, Change::Dpad { x: -1, y: 0 });
    assert_eq!(describe(&change), "direction pad -1 0");
}

#[test]
fn a_trigger_with_its_own_travel_reaches_full_deflection_at_the_maximum() {
    let mut pad = Gamepad::new();
    pad.set_range(Axis::LeftTrigger, AxisRange::trigger(0, 1023, 0.0));
    let mut input = ControllerInput::with_pad(pad);
    assert_eq!(input.feed(EV_ABS, ABS_Z, 1023), Some(Change::Axis { axis: Axis::LeftTrigger, value: 1.0 }));
    assert_eq!(describe(&Change::Axis { axis: Axis::LeftTrigger, value: 1.0 }), "left trigger 1.00");
}

// Symbol styling and drawing.

#[test]
fn the_regular_weight_and_medium_scale_keep_the_base_stroke_and_size() {
    let style = SymbolStyle::default();
    assert_eq!(style.stroke(2.0), 2.0);
    assert_eq!(style.size(20.0), 20.0);
}

#[test]
fn a_black_weight_and_a_large_scale_thicken_the_stroke_and_grow_the_size() {
    let style = SymbolStyle { weight: Weight::Black, scale: Scale::Large, ..SymbolStyle::default() };
    assert!(close(style.stroke(2.0), 3.5), "eight steps from regular at 0.15 each is 1.75 times the stroke");
    assert!(close(style.size(20.0), 25.0));
}

#[test]
fn a_hierarchical_symbol_steps_each_lower_layer_down_in_opacity() {
    let white = rgba(255, 255, 255, 255);
    let style = SymbolStyle { mode: RenderingMode::Hierarchical, ..SymbolStyle::default() };
    assert_eq!(style.layer_colour(0, white), white, "the primary layer is drawn in full");
    assert_eq!(style.layer_colour(1, white) >> 24, 128, "the second layer is half opaque");
    assert_eq!(style.layer_colour(2, white) >> 24, 77, "the third layer is 0.3 opaque");
    assert_eq!(style.layer_colour(9, white) >> 24, 77, "layers past the last step keep that step");
}

#[test]
fn a_monochrome_symbol_draws_every_layer_in_one_colour() {
    let red = rgba(255, 0, 0, 255);
    let style = SymbolStyle::default();
    assert_eq!(style.layer_colour(0, red), red);
    assert_eq!(style.layer_colour(3, red), red);
}

#[test]
fn a_palette_symbol_gives_each_layer_its_slot_and_repeats_the_last_slot() {
    let (first, second) = (rgba(255, 0, 0, 255), rgba(0, 0, 255, 255));
    let caller = rgba(0, 255, 0, 255);
    let style = SymbolStyle { mode: RenderingMode::Palette, palette: vec![first, second], ..SymbolStyle::default() };
    assert_eq!(style.layer_colour(0, caller), first);
    assert_eq!(style.layer_colour(1, caller), second);
    assert_eq!(style.layer_colour(4, caller), second);
}

#[test]
fn a_palette_mode_with_an_empty_palette_falls_back_to_the_caller_colour() {
    let caller = rgba(0, 255, 0, 255);
    let style = SymbolStyle { mode: RenderingMode::Multicolor, ..SymbolStyle::default() };
    assert_eq!(style.layer_colour(2, caller), caller);
}

#[test]
fn a_painted_layer_covers_its_shape_inside_its_rectangle_in_a_headless_surface() {
    let mut px = vec![0u32; 16 * 16];
    let mut surface = Surface { w: 16, h: 16, stride: 16, px: &mut px, y0: 0 };
    let style = SymbolStyle::default();
    let white = rgba(255, 255, 255, 255);
    paint_layer(&mut surface, (2.0, 2.0, 8.0, 8.0), &style, 0, white, disc((6.0, 6.0), 3.0));
    assert_ne!(px[6 * 16 + 6], 0, "the centre of the disc is painted");
    assert_eq!(px[0], 0, "a pixel outside the disc stays clear");
}

// View actions.

#[test]
fn a_move_reaches_its_offset_at_the_end_and_is_halfway_at_half_time() {
    let move_by = Action::MoveBy { dx: 10.0, dy: -4.0, duration: 2.0 };
    assert_eq!(move_by.duration(), 2.0);
    let middle = move_by.pose_at(1.0);
    assert!(close(middle.dx, 5.0) && close(middle.dy, -2.0));
    let end = move_by.pose_at(5.0);
    assert!(close(end.dx, 10.0) && close(end.dy, -4.0));
    assert_eq!(move_by.pose_at(-1.0).dx, 0.0, "before the start the node is at rest");
}

#[test]
fn a_sequence_runs_its_parts_one_after_another() {
    let seq = Action::Sequence(vec![
        Action::MoveBy { dx: 4.0, dy: 0.0, duration: 1.0 },
        Action::Wait { duration: 1.0 },
        Action::MoveBy { dx: 0.0, dy: 6.0, duration: 2.0 },
    ]);
    assert_eq!(seq.duration(), 4.0);
    let during_wait = seq.pose_at(1.5);
    assert!(close(during_wait.dx, 4.0) && close(during_wait.dy, 0.0));
    let during_last = seq.pose_at(3.0);
    assert!(close(during_last.dx, 4.0) && close(during_last.dy, 3.0));
    let done = seq.pose_at(10.0);
    assert!(close(done.dx, 4.0) && close(done.dy, 6.0));
}

#[test]
fn a_group_runs_its_parts_together_and_lasts_as_long_as_the_longest() {
    let group = Action::Group(vec![
        Action::MoveBy { dx: 10.0, dy: 0.0, duration: 1.0 },
        Action::RotateBy { radians: std::f32::consts::FRAC_PI_2, duration: 2.0 },
    ]);
    assert_eq!(group.duration(), 2.0);
    let middle = group.pose_at(1.0);
    assert!(close(middle.dx, 10.0), "the move has finished at one second");
    assert!(close(middle.rotation, std::f32::consts::FRAC_PI_4), "the turn is halfway at one second");
}

#[test]
fn a_repeat_adds_each_run_to_the_runs_before_it() {
    let step = Action::MoveBy { dx: 3.0, dy: 0.0, duration: 1.0 };
    let repeat = Action::Repeat { action: Box::new(step), count: 3 };
    assert_eq!(repeat.duration(), 3.0);
    assert!(close(repeat.pose_at(1.5).dx, 4.5), "one full run of 3 plus half of the second");
    assert!(close(repeat.pose_at(10.0).dx, 9.0), "three runs in all, then the node stays put");
}

#[test]
fn a_fade_sets_the_opacity_from_fully_opaque_to_its_target() {
    let fade = Action::FadeTo { alpha: 0.25, duration: 1.0 };
    assert_eq!(fade.pose_at(0.0).alpha, Some(1.0));
    assert!(close(fade.pose_at(0.5).alpha.unwrap_or(0.0), 0.625));
    assert_eq!(fade.pose_at(1.0).alpha, Some(0.25));
    assert_eq!(Action::Wait { duration: 1.0 }.pose_at(0.5), Pose::REST, "a wait changes nothing");
}

#[test]
fn composed_scales_multiply_and_the_later_fade_sets_the_opacity() {
    let grow = Action::ScaleBy { factor: 2.0, duration: 1.0 };
    let seq = Action::Sequence(vec![
        grow.clone(),
        grow,
        Action::FadeTo { alpha: 0.0, duration: 1.0 },
        Action::FadeTo { alpha: 0.5, duration: 1.0 },
    ]);
    let end = seq.pose_at(10.0);
    assert!(close(end.scale, 4.0));
    assert_eq!(end.alpha, Some(0.5));
}

#[test]
fn a_negative_duration_counts_as_no_time() {
    assert_eq!(Action::Wait { duration: -2.0 }.duration(), 0.0);
}

// HTTP exchange logs.

#[test]
fn a_credential_query_value_is_replaced_and_other_values_are_kept() {
    assert_eq!(
        redact_url("https://api.example.test/feed?page=2&token=abc123&Key=zz"),
        "https://api.example.test/feed?page=2&token=<redacted>&Key=<redacted>"
    );
}

#[test]
fn a_password_in_the_authority_is_replaced_and_the_user_name_is_kept() {
    assert_eq!(redact_url("https://ann:hunter2@example.test/a/b"), "https://ann:<redacted>@example.test/a/b");
}

#[test]
fn a_url_without_secrets_is_unchanged_and_a_fragment_is_dropped() {
    assert_eq!(redact_url("https://example.test/news?q=1"), "https://example.test/news?q=1");
    assert_eq!(redact_url("https://example.test/news#top"), "https://example.test/news");
}

#[test]
fn a_successful_response_is_one_debug_line_with_its_size_and_duration() {
    let exchange = Exchange::response("GET", "https://example.test/feed?token=t", 200, 1536, Duration::from_millis(42));
    assert_eq!(exchange.level(), Level::Debug);
    assert_eq!(exchange.summary(), "GET https://example.test/feed?token=<redacted> 200 1.5 KiB in 42 ms");
}

#[test]
fn the_level_follows_the_status_class_and_a_failed_request_is_an_error() {
    let level_of = |status| Exchange::response("GET", "https://example.test/", status, 0, Duration::ZERO).level();
    assert_eq!(level_of(301), Level::Debug);
    assert_eq!(level_of(404), Level::Notice);
    assert_eq!(level_of(503), Level::Error);
    let failed = Exchange::failed("POST", "https://example.test/upload", "timed out", Duration::from_millis(60_000));
    assert_eq!(failed.level(), Level::Error);
    assert_eq!(failed.outcome, Outcome::Failed { reason: "timed out".into() });
    assert_eq!(failed.summary(), "POST https://example.test/upload failed after 60000 ms: timed out");
}

#[test]
fn body_sizes_switch_units_at_a_kibibyte_and_a_mebibyte() {
    assert_eq!(body_size(0), "0 B");
    assert_eq!(body_size(1023), "1023 B");
    assert_eq!(body_size(1024), "1.0 KiB");
    assert_eq!(body_size(1024 * 1024), "1.0 MiB");
    assert_eq!(body_size(3 * 1024 * 1024 + 512 * 1024), "3.5 MiB");
}

// URL session configuration.

#[test]
fn the_default_session_uses_the_usual_timeouts_and_six_connections_per_host() {
    let config = URLSessionConfiguration::default();
    assert_eq!(config.timeout_interval_for_request, Duration::from_secs(60));
    assert_eq!(config.timeout_interval_for_resource, Duration::from_secs(604_800));
    assert_eq!(config.connection_limit(), 6);
    assert_eq!(config.request_cache_policy, CachePolicy::UseProtocolCachePolicy);
}

#[test]
fn a_session_header_is_added_only_where_the_request_does_not_set_it() {
    let mut config = URLSessionConfiguration::default();
    config.set_header("Accept-Language", "is");
    config.set_header("accept-language", "en");
    assert_eq!(config.http_additional_headers, vec![("accept-language".to_string(), "en".to_string())]);

    let mut request = URLRequest::new("https://example.test/");
    request.set_header("Accept-Language", "da");
    config.prepare(&mut request);
    assert_eq!(request.headers, vec![("Accept-Language".to_string(), "da".to_string())]);

    let mut other = URLRequest::new("https://example.test/");
    config.prepare(&mut other);
    assert_eq!(other.headers, vec![("accept-language".to_string(), "en".to_string())]);
}

#[test]
fn the_session_caps_a_request_timeout_and_never_raises_it() {
    let mut config = URLSessionConfiguration::default();
    config.timeout_interval_for_request = Duration::from_secs(15);

    let mut slow = URLRequest::new("https://example.test/");
    config.prepare(&mut slow);
    assert_eq!(slow.timeout, Duration::from_secs(15));

    let mut quick = URLRequest::new("https://example.test/");
    quick.timeout = Duration::from_secs(5);
    config.prepare(&mut quick);
    assert_eq!(quick.timeout, Duration::from_secs(5));
}

#[test]
fn a_transfer_is_expired_once_it_has_run_for_the_resource_timeout() {
    let mut config = URLSessionConfiguration::default();
    config.timeout_interval_for_resource = Duration::from_secs(30);
    assert!(!config.resource_expired(Duration::from_secs(29)));
    assert!(config.resource_expired(Duration::from_secs(30)));
}

#[test]
fn zero_connections_per_host_still_allows_one() {
    let mut config = URLSessionConfiguration::default();
    config.http_maximum_connections_per_host = 0;
    assert_eq!(config.connection_limit(), 1);
}

#[test]
fn each_cache_policy_chooses_the_cache_the_network_or_a_failure() {
    assert_eq!(CachePolicy::UseProtocolCachePolicy.action(Some(true)), CacheAction::UseCache);
    assert_eq!(CachePolicy::UseProtocolCachePolicy.action(Some(false)), CacheAction::Load);
    assert_eq!(CachePolicy::UseProtocolCachePolicy.action(None), CacheAction::Load);
    assert_eq!(CachePolicy::ReloadIgnoringLocalCache.action(Some(true)), CacheAction::Load);
    assert_eq!(CachePolicy::ReturnCacheDataElseLoad.action(Some(false)), CacheAction::UseCache);
    assert_eq!(CachePolicy::ReturnCacheDataElseLoad.action(None), CacheAction::Load);
    assert_eq!(CachePolicy::ReturnCacheDataDontLoad.action(None), CacheAction::Fail);
}
