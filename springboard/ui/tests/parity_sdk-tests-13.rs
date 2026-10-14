//! Parity sdk-tests-13: headless tests of four pure models in `bifrost_ui`: symbol layers (variable colour, weights,
//! scales, rendering modes), focus filter contexts, widget access grants, and the render synchronizer timeline. Each
//! model is pure arithmetic or a small store; no test draws, opens a window, reads a clock or touches the network.

use bifrost_ui::focus_filter::{AppContext, FocusFilters};
use bifrost_ui::render_synchronizer::{RenderSynchronizer, SyncError};
use bifrost_ui::symbol_layers::{
    cycle_progress, layer_opacity, palette_slot, rendering_opacity, Fill, Inactive, Playback, RenderingMode, Scale, Weight,
    DIM_OPACITY, HIERARCHY,
};
use bifrost_ui::widget_access::{host_matches, normalize_host, Access, Grant, HostError, MAX_HOST_LEN};

const SEC_NS: i64 = 1_000_000_000;
const WEATHER: &str = "is.example.weather";

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// ---- symbol layers: variable colour ----

#[test]
fn headless_cumulative_fill_lights_each_layer_from_its_turn_to_the_end_of_the_cycle() {
    for layer in 0..4 {
        assert!(close(layer_opacity(layer, 4, 1.0, Fill::Cumulative, Inactive::Dim), 1.0), "layer {layer} is lit at the end");
        assert!(close(layer_opacity(layer, 4, 0.0, Fill::Cumulative, Inactive::Dim), DIM_OPACITY), "layer {layer} rests at the start");
    }
    assert!(close(layer_opacity(1, 4, 0.5, Fill::Cumulative, Inactive::Hide), 1.0));
    assert!(close(layer_opacity(2, 4, 0.5, Fill::Cumulative, Inactive::Hide), 0.0), "layer 2 has not had its turn at half way");
}

#[test]
fn headless_a_cumulative_layer_half_way_through_its_step_is_half_lit() {
    // four layers at progress 0.375: layer 1 is half way through its step, 0.3 + 0.7 * 0.5 = 0.65 under Dim.
    assert!(close(layer_opacity(1, 4, 0.375, Fill::Cumulative, Inactive::Dim), 0.65));
}

#[test]
fn headless_iterative_fill_lights_one_layer_at_a_time() {
    assert!(close(layer_opacity(0, 4, 0.0, Fill::Iterative, Inactive::Dim), 1.0));
    assert!(close(layer_opacity(1, 4, 0.0, Fill::Iterative, Inactive::Dim), DIM_OPACITY));
    assert!(close(layer_opacity(1, 4, 0.25, Fill::Iterative, Inactive::Dim), 1.0));
    assert!(close(layer_opacity(0, 4, 0.25, Fill::Iterative, Inactive::Dim), DIM_OPACITY));
    assert!(close(layer_opacity(3, 4, 0.99, Fill::Iterative, Inactive::Hide), 1.0));
    assert!(close(layer_opacity(2, 4, 0.99, Fill::Iterative, Inactive::Hide), 0.0));
}

#[test]
fn headless_progress_is_clamped_and_nan_reads_as_the_start() {
    assert!(close(layer_opacity(0, 2, 7.0, Fill::Iterative, Inactive::Hide), 1.0), "progress past one reads as one");
    assert!(close(layer_opacity(1, 2, -3.0, Fill::Iterative, Inactive::Hide), 0.0), "negative progress reads as zero");
    assert!(close(layer_opacity(0, 2, f32::NAN, Fill::Iterative, Inactive::Hide), 1.0), "NaN reads as the start");
}

#[test]
fn headless_a_layer_past_the_count_or_an_empty_image_has_no_opacity() {
    assert!(close(layer_opacity(4, 4, 1.0, Fill::Cumulative, Inactive::Dim), 0.0));
    assert!(close(layer_opacity(0, 0, 1.0, Fill::Iterative, Inactive::Dim), 0.0));
}

#[test]
fn headless_non_reversing_playback_restarts_each_period() {
    assert!(close(cycle_progress(1.0, 4.0, Playback::NonReversing), 0.25));
    assert!(close(cycle_progress(5.0, 4.0, Playback::NonReversing), 0.25), "one full period later the phase is the same");
}

#[test]
fn headless_reversing_playback_rises_to_one_and_falls_back() {
    assert!(close(cycle_progress(1.0, 4.0, Playback::Reversing), 0.5));
    assert!(close(cycle_progress(2.0, 4.0, Playback::Reversing), 1.0));
    assert!(close(cycle_progress(3.0, 4.0, Playback::Reversing), 0.5));
}

#[test]
fn headless_a_bad_cycle_time_reads_as_the_start_of_the_cycle() {
    assert!(close(cycle_progress(f32::INFINITY, 4.0, Playback::NonReversing), 0.0));
    assert!(close(cycle_progress(-4.0, 2.0, Playback::Reversing), 0.0));
    assert!(close(cycle_progress(1.0, -2.0, Playback::NonReversing), 0.0));
}

// ---- symbol layers: weights, scales and rendering modes ----

#[test]
fn headless_weights_are_nine_steps_and_each_step_is_heavier_than_the_last() {
    let steps = [
        Weight::UltraLight,
        Weight::Thin,
        Weight::Light,
        Weight::Regular,
        Weight::Medium,
        Weight::Semibold,
        Weight::Bold,
        Weight::Heavy,
        Weight::Black,
    ];
    for (i, weight) in steps.iter().enumerate() {
        assert_eq!(weight.index(), i);
    }
    for pair in steps.windows(2) {
        assert!(pair[0].stroke_factor() < pair[1].stroke_factor(), "{:?} is lighter than {:?}", pair[0], pair[1]);
    }
}

#[test]
fn headless_the_stroke_factor_steps_by_fifteen_percent_from_regular() {
    assert!(close(Weight::Regular.stroke_factor(), 1.0));
    assert!(close(Weight::Thin.stroke_factor(), 0.7));
    assert!(close(Weight::Semibold.stroke_factor(), 1.3));
    assert!(close(Weight::Heavy.stroke_factor(), 1.6));
    assert!(close(Weight::UltraLight.stroke_factor(), 0.55));
}

#[test]
fn headless_scale_steps_are_fixed_multipliers_of_the_base_size() {
    assert!(close(Scale::Small.factor(), 0.75));
    assert!(close(Scale::Medium.factor(), 1.0));
    assert!(close(Scale::Large.factor(), 1.25));
    assert!(close(17.0 * Scale::Large.factor(), 21.25));
}

#[test]
fn headless_hierarchical_mode_steps_opacity_down_by_layer_rank() {
    assert_eq!(HIERARCHY, [1.0, 0.5, 0.3]);
    assert!(close(rendering_opacity(RenderingMode::Hierarchical, 0), 1.0), "the primary layer is full");
    assert!(close(rendering_opacity(RenderingMode::Hierarchical, 1), 0.5));
    assert!(close(rendering_opacity(RenderingMode::Hierarchical, 2), 0.3));
    assert!(close(rendering_opacity(RenderingMode::Hierarchical, 9), 0.3), "ranks past the last step keep the last step");
}

#[test]
fn headless_monochrome_palette_and_multicolor_modes_keep_full_opacity() {
    for mode in [RenderingMode::Monochrome, RenderingMode::Palette, RenderingMode::Multicolor] {
        assert!(close(rendering_opacity(mode, 2), 1.0), "{mode:?} does not step opacity by rank");
    }
}

#[test]
fn headless_palette_slots_follow_rank_and_the_last_colour_repeats() {
    assert_eq!(palette_slot(0, 3), Some(0));
    assert_eq!(palette_slot(1, 3), Some(1));
    assert_eq!(palette_slot(7, 3), Some(2));
    assert_eq!(palette_slot(0, 0), None, "an empty palette has no slot");
}

// ---- focus filters ----

fn context(pairs: &[(&str, &str)]) -> AppContext {
    let mut c = AppContext::new();
    for (key, value) in pairs {
        c.set(key, value);
    }
    c
}

#[test]
fn headless_the_context_of_the_active_focus_applies_and_others_are_kept() {
    let mut f = FocusFilters::new();
    f.set_context("Work", context(&[("mode", "Quiet")]));
    f.set_context("Personal", context(&[("mode", "Open")]));
    f.activate(Some("Personal"));
    assert_eq!(f.current().and_then(|c| c.get("mode")), Some("Open"));
    f.activate(None);
    assert!(f.current().is_none(), "no Focus on means no context applies");
    assert_eq!(f.context("Work").and_then(|c| c.get("mode")), Some("Quiet"));
}

#[test]
fn headless_a_focus_without_a_stored_context_applies_nothing() {
    let mut f = FocusFilters::new();
    f.activate(Some("Sleep"));
    assert_eq!(f.active(), Some("Sleep"));
    assert!(f.current().is_none());
}

#[test]
fn headless_an_app_context_keeps_one_value_per_key_in_first_set_order() {
    let mut c = AppContext::new();
    assert!(c.is_empty());
    c.set("room", "3");
    c.set("mode", "Quiet");
    c.set("room", "4");
    assert_eq!(c.pairs(), &[("room".to_string(), "4".to_string()), ("mode".to_string(), "Quiet".to_string())]);
    assert_eq!(c.len(), 2);
    assert_eq!(c.get("missing"), None);
}

#[test]
fn headless_the_generation_counts_stored_changes_and_invalidations() {
    let mut f = FocusFilters::new();
    f.set_context("Work", context(&[("mode", "Quiet")]));
    f.set_context("Work", context(&[("mode", "Quiet")]));
    assert_eq!(f.generation(), 1, "the same context again changes nothing");
    assert_eq!(f.invalidate(), 2);
    assert!(f.remove("Work"));
    assert!(!f.remove("Work"));
    assert_eq!(f.generation(), 3, "removing an absent context changes nothing");
}

// ---- widget access ----

#[test]
fn headless_a_url_reduces_to_its_lower_case_host() {
    assert_eq!(normalize_host("HTTPS://Api.Example.COM/v1?x=1#top").unwrap(), "api.example.com");
    assert_eq!(normalize_host("example.com.").unwrap(), "example.com");
    assert_eq!(normalize_host("   "), Err(HostError::Empty));
    assert_eq!(normalize_host("https://"), Err(HostError::Empty));
}

#[test]
fn headless_text_that_is_not_a_plain_host_name_is_refused() {
    for raw in ["example.com:8080", "user@example.com", "a_b.example.com", "-bad.example.com", "bad-.example.com", "a..b", "a:b"] {
        assert!(matches!(normalize_host(raw), Err(HostError::Malformed(_))), "{raw} is not a plain host name");
    }
}

#[test]
fn headless_a_host_name_is_limited_to_253_bytes_and_63_byte_labels() {
    let label = "a".repeat(63);
    let fits = format!("{label}.{label}.{label}.{}", "a".repeat(61));
    assert_eq!(fits.len(), MAX_HOST_LEN);
    assert_eq!(normalize_host(&fits).unwrap(), fits);
    let too_long = format!("{label}.{label}.{label}.{}", "a".repeat(62));
    assert!(matches!(normalize_host(&too_long), Err(HostError::Malformed(_))));
    assert!(matches!(normalize_host(&"a".repeat(64)), Err(HostError::Malformed(_))));
}

#[test]
fn headless_a_host_matches_itself_and_its_subdomains_only() {
    assert!(host_matches("example.com", "example.com"));
    assert!(host_matches("ws.api.example.com", "example.com"));
    assert!(!host_matches("example.com", "api.example.com"), "a parent domain is not covered");
    assert!(!host_matches("notexample.com", "example.com"));
    assert!(!host_matches("example.com.evil.net", "example.com"));
}

#[test]
fn headless_a_widget_holds_no_access_until_it_is_granted() {
    let access = Access::default();
    assert!(!access.may_locate(WEATHER));
    assert!(!access.may_contact(WEATHER, "https://api.example.com/"));
    assert_eq!(access.grant_of(WEATHER), Grant::default());
}

#[test]
fn headless_a_network_grant_reaches_the_host_and_its_subdomains_for_one_widget() {
    let mut access = Access::default();
    assert_eq!(access.allow_host(WEATHER, "https://API.Open-Meteo.com/v1").unwrap(), "api.open-meteo.com");
    assert!(access.may_contact(WEATHER, "api.open-meteo.com"));
    assert!(access.may_contact(WEATHER, "https://ws.api.open-meteo.com/x?y=1"));
    assert!(!access.may_contact(WEATHER, "open-meteo.com"));
    assert!(!access.may_contact("is.example.other", "api.open-meteo.com"), "grants belong to one widget");
    assert!(!access.may_contact(WEATHER, "not a host"));
}

#[test]
fn headless_a_malformed_host_is_never_stored_in_a_grant() {
    let mut access = Access::default();
    assert_eq!(access.allow_host(WEATHER, "a:b"), Err(HostError::Malformed("a:b".into())));
    assert_eq!(access.grant_of(WEATHER), Grant::default());
}

#[test]
fn headless_revoking_a_host_stops_its_traffic_and_leaves_the_others() {
    let mut access = Access::default();
    access.allow_host(WEATHER, "example.com").unwrap();
    access.allow_host(WEATHER, "cdn.example.org").unwrap();
    assert!(access.revoke_host(WEATHER, "https://example.com/"));
    assert!(!access.may_contact(WEATHER, "example.com"));
    assert!(access.may_contact(WEATHER, "cdn.example.org"));
    assert!(!access.revoke_host(WEATHER, "example.com"), "a host that is gone cannot be revoked twice");
    assert!(!access.revoke_host(WEATHER, "bad host"));
}

#[test]
fn headless_location_and_network_are_granted_separately() {
    let mut access = Access::default();
    access.allow_host(WEATHER, "example.com").unwrap();
    access.set_location(WEATHER, true);
    assert!(access.may_locate(WEATHER));
    assert!(access.grant_of(WEATHER).location);
    assert_eq!(access.grant_of(WEATHER).hosts.len(), 1);
    access.set_location(WEATHER, false);
    assert!(!access.may_locate(WEATHER));
    assert!(access.may_contact(WEATHER, "example.com"), "withdrawing location keeps the network grant");
}

#[test]
fn headless_forgetting_a_widget_drops_its_grants_and_keeps_the_others() {
    let mut access = Access::default();
    access.set_location(WEATHER, true);
    access.allow_host(WEATHER, "example.com").unwrap();
    access.allow_host("is.example.clock", "time.example.com").unwrap();
    access.forget(WEATHER);
    assert_eq!(access.grant_of(WEATHER), Grant::default());
    assert!(access.may_contact("is.example.clock", "time.example.com"));
}

// ---- render synchronizer ----

#[test]
fn headless_a_new_timeline_is_paused_at_media_time_zero_with_no_renderers() {
    let s = RenderSynchronizer::default();
    assert_eq!(s.rate(), 0.0);
    assert_eq!(s.current_time_us(SEC_NS), 0);
    assert!(s.renderers().is_empty());
}

#[test]
fn headless_media_time_follows_the_rate_from_the_anchor() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(1.0, 0, 0).unwrap();
    assert_eq!(s.current_time_us(2 * SEC_NS), 2_000_000);
    s.set_rate(2.0, 0, 0).unwrap();
    assert_eq!(s.current_time_us(3 * SEC_NS), 6_000_000);
}

#[test]
fn headless_host_times_before_the_anchor_read_earlier_media_times() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(1.0, 10_000_000, 5 * SEC_NS).unwrap();
    assert_eq!(s.current_time_us(5 * SEC_NS), 10_000_000);
    assert_eq!(s.current_time_us(4 * SEC_NS), 9_000_000);
}

#[test]
fn headless_a_paused_timeline_holds_its_media_time_and_crosses_nothing() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(0.0, 2_000_000, SEC_NS).unwrap();
    assert_eq!(s.current_time_us(9 * SEC_NS), 2_000_000);
    assert!(s.boundary_crossings(0, 9 * SEC_NS, &[3_000_000]).is_empty());
}

#[test]
fn headless_changing_the_rate_re_anchors_without_a_jump() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(1.0, 0, 0).unwrap();
    assert_eq!(s.current_time_us(3 * SEC_NS), 3_000_000);
    s.set_rate(0.5, 3_000_000, 3 * SEC_NS).unwrap();
    assert_eq!(s.current_time_us(5 * SEC_NS), 4_000_000);
    s.set_rate(2.0, 1_000_000, SEC_NS).unwrap();
    assert_eq!(s.current_time_us(2 * SEC_NS), 3_000_000);
}

#[test]
fn headless_a_negative_rate_runs_the_media_time_backwards() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(-1.0, 10_000_000, 0).unwrap();
    assert_eq!(s.current_time_us(4 * SEC_NS), 6_000_000);
}

#[test]
fn headless_a_non_finite_rate_is_refused_and_leaves_the_timeline_alone() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(1.0, 0, 0).unwrap();
    assert_eq!(s.set_rate(f64::NAN, 0, 0), Err(SyncError::InvalidRate));
    assert_eq!(s.set_rate(f64::INFINITY, 0, 0), Err(SyncError::InvalidRate));
    assert_eq!(s.rate(), 1.0);
}

#[test]
fn headless_renderers_attach_once_by_id_and_detach_by_id() {
    let mut s = RenderSynchronizer::default();
    s.add_renderer("video").unwrap();
    s.add_renderer("captions").unwrap();
    assert_eq!(s.add_renderer("video"), Err(SyncError::DuplicateRenderer));
    assert_eq!(s.renderers(), &["video".to_string(), "captions".to_string()]);
    s.remove_renderer("video").unwrap();
    assert_eq!(s.renderers(), &["captions".to_string()]);
    assert_eq!(s.remove_renderer("video"), Err(SyncError::UnknownRenderer));
}

#[test]
fn headless_forward_crossings_include_the_end_and_exclude_the_start() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(1.0, 0, 0).unwrap();
    assert_eq!(s.boundary_crossings(0, 2 * SEC_NS, &[2_000_000, 0, 3_000_000, 1_000_000]), vec![1_000_000, 2_000_000]);
}

#[test]
fn headless_backward_crossings_run_from_the_start_down_to_the_end() {
    let mut s = RenderSynchronizer::default();
    s.set_rate(-1.0, 3_000_000, 0).unwrap();
    assert_eq!(s.boundary_crossings(0, 2 * SEC_NS, &[1_000_000, 2_500_000, 3_000_000, 4_000_000]), vec![2_500_000, 1_000_000]);
}
