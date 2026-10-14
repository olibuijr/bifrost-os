//! Parity sdk-tests-01a: headless tests for the window server's material arithmetic (blur pass counts, pyramid sizes,
//! device regions, window shadows and clipping). These are pure functions; no test creates a device or a surface.

use bifrost_wm::render_server::material::{
    backdrop_levels, clip_region, device_region, kawase_offset, kawase_passes, passes_for, pyramid_sizes, window_shadow, MAX_PASSES,
    SATURATION,
};

#[test]
fn blur_passes_double_their_reach_and_stop_at_the_maximum() {
    assert_eq!(kawase_passes(0.0), 1);
    assert_eq!(kawase_passes(2.0), 1, "a radius of 2 px needs one pass");
    assert_eq!(kawase_passes(3.0), 2);
    assert_eq!(kawase_passes(4.0), 2);
    assert_eq!(kawase_passes(100.0), MAX_PASSES);
    assert_eq!(MAX_PASSES, 6);
}

#[test]
fn the_kernel_offset_is_the_radius_over_the_reach_and_at_least_half_a_texel() {
    assert_eq!(kawase_offset(16.0, 3), 2.0);
    assert_eq!(kawase_offset(1.0, 2), 0.5, "never below half a texel");
    assert_eq!(kawase_offset(64.0, 9), 1.0, "the pass count is clamped before the division");
}

#[test]
fn the_backdrop_scale_implies_its_pyramid_levels() {
    assert_eq!(backdrop_levels(1.0), 0);
    assert_eq!(backdrop_levels(0.5), 1);
    assert_eq!(backdrop_levels(0.25), 2);
    assert_eq!(backdrop_levels(0.125), 3);
    assert_eq!(backdrop_levels(0.0), 0, "a scale of zero implies no levels");
    assert_eq!(backdrop_levels(0.001), MAX_PASSES);
}

#[test]
fn the_pass_count_is_the_larger_of_the_radius_and_the_scale_within_the_maximum() {
    assert_eq!(passes_for(3.0, 0.25), 2);
    assert_eq!(passes_for(0.0, 0.125), 3, "a quarter-size backdrop needs its levels even for a small radius");
    assert_eq!(passes_for(100.0, 1.0), MAX_PASSES);
}

#[test]
fn a_device_region_rounds_outward_to_whole_pixels() {
    assert_eq!(device_region([10.5, 20.2, 30.0, 5.0]), [10, 20, 31, 6]);
    assert_eq!(device_region([5.0, 5.0, 0.0, 0.0]), [5, 5, 0, 0]);
}

#[test]
fn pyramid_levels_halve_and_round_up_with_a_one_texel_floor() {
    assert_eq!(pyramid_sizes(10, 7, 3), vec![(5, 4), (3, 2), (2, 1)]);
    assert_eq!(pyramid_sizes(1, 1, 2), vec![(1, 1), (1, 1)]);
}

#[test]
fn a_window_shadow_is_larger_and_darker_when_the_window_is_active() {
    let active = window_shadow([0.0, 0.0, 100.0, 50.0], 10.0, true, 2.0);
    assert_eq!(active.sigma, 48.0);
    assert_eq!(active.offset, [0.0, 32.0]);
    assert_eq!(active.color, [0.0, 0.0, 0.0, 0.35]);
    assert_eq!(active.radius, 10.0);
    let inactive = window_shadow([0.0, 0.0, 100.0, 50.0], 10.0, false, 1.0);
    assert_eq!(inactive.sigma, 12.0);
    assert_eq!(inactive.offset, [0.0, 6.0]);
    assert_eq!(inactive.color, [0.0, 0.0, 0.0, 0.20]);
}

#[test]
fn a_region_off_the_top_left_loses_its_overhang_and_keeps_its_far_edge() {
    assert_eq!(clip_region([-10, 5, 100, 20], 50, 30), [0, 5, 50, 20]);
    assert_eq!(clip_region([1, 2, 3, 4], 100, 100), [1, 2, 3, 4]);
    let outside = clip_region([60, 0, 10, 10], 50, 50);
    assert!(outside[2] <= 0, "a region past the right edge is empty");
}

#[test]
fn the_saturation_of_the_blurred_backdrop_is_the_shell_value() {
    assert_eq!(SATURATION, 1.8);
}
