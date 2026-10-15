//! Parity sdk-tests-02: headless tests of the layer appearance geometry (`bifrost_ui::layer_style`): corner radii,
//! corner masks, the shadow footprint and the content mask. Everything is pure arithmetic; no frame is painted.

use bifrost_ui::layer_style::{corner_radii, effective_radius, mask_contains, CornerMask, Shadow};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn headless_a_radius_is_capped_at_half_the_shorter_side() {
    assert_eq!(effective_radius(20.0, 100.0, 40.0), 20.0);
    assert_eq!(effective_radius(30.0, 100.0, 40.0), 20.0, "the corners never overlap");
    assert_eq!(effective_radius(6.0, 100.0, 40.0), 6.0);
}

#[test]
fn headless_a_radius_that_is_not_a_positive_finite_number_draws_square_corners() {
    for bad in [0.0_f32, -4.0, f32::NAN, f32::INFINITY] {
        assert_eq!(effective_radius(bad, 100.0, 40.0), 0.0, "radius {bad}");
    }
    assert_eq!(effective_radius(10.0, 0.0, 40.0), 0.0, "an empty box has no corners to round");
    assert_eq!(effective_radius(10.0, 100.0, -1.0), 0.0);
}

#[test]
fn headless_the_corner_mask_keeps_only_the_named_corners() {
    let only_min = corner_radii(10.0, 100.0, 100.0, CornerMask::MIN_X_MIN_Y);
    assert_eq!(only_min, [10.0, 0.0, 0.0, 0.0]);
    let right_side = CornerMask::MAX_X_MIN_Y.union(CornerMask::MAX_X_MAX_Y);
    assert_eq!(corner_radii(10.0, 100.0, 100.0, right_side), [0.0, 10.0, 0.0, 10.0]);
    assert_eq!(corner_radii(10.0, 100.0, 100.0, CornerMask::ALL), [10.0; 4]);
    assert_eq!(corner_radii(10.0, 100.0, 100.0, CornerMask::NONE), [0.0; 4]);
}

#[test]
fn headless_a_mask_contains_exactly_the_corners_it_was_built_from() {
    assert!(CornerMask::ALL.contains(CornerMask::MAX_X_MAX_Y));
    assert!(CornerMask::NONE.contains(CornerMask::NONE));
    assert!(!CornerMask::MIN_X_MIN_Y.contains(CornerMask::ALL));
    let two = CornerMask::MIN_X_MIN_Y.union(CornerMask::MIN_X_MAX_Y);
    assert!(two.contains(CornerMask::MIN_X_MAX_Y));
    assert!(!two.contains(CornerMask::MAX_X_MIN_Y));
}

#[test]
fn headless_a_shadow_without_opacity_is_not_visible() {
    let none = Shadow::default();
    assert_eq!(none.opacity, 0.0);
    assert!(!none.is_visible());
    let faint = Shadow { opacity: 0.01, radius: 2.0, offset: (0.0, 0.0) };
    assert!(faint.is_visible());
}

#[test]
fn headless_shadow_opacity_is_clamped_and_a_non_finite_one_reads_as_none() {
    let shadow = |opacity: f32| Shadow { opacity, radius: 0.0, offset: (0.0, 0.0) };
    assert_eq!(shadow(1.7).effective_opacity(), 1.0);
    assert_eq!(shadow(-0.2).effective_opacity(), 0.0);
    assert_eq!(shadow(f32::NAN).effective_opacity(), 0.0);
    assert!(!shadow(f32::INFINITY).is_visible());
}

#[test]
fn headless_the_shadow_footprint_moves_by_the_offset_and_grows_by_the_blur() {
    let shadow = Shadow { opacity: 0.5, radius: 4.0, offset: (2.0, -3.0) };
    assert_eq!(shadow.footprint(rect(10.0, 10.0, 20.0, 20.0)), rect(8.0, 3.0, 28.0, 28.0));
}

#[test]
fn headless_a_negative_blur_adds_no_growth_and_a_non_finite_offset_adds_no_move() {
    let frame = rect(10.0, 10.0, 20.0, 20.0);
    let negative = Shadow { opacity: 1.0, radius: -5.0, offset: (0.0, 0.0) };
    assert_eq!(negative.footprint(frame), frame);
    let broken = Shadow { opacity: 1.0, radius: 3.0, offset: (f32::NAN, 5.0) };
    assert_eq!(broken.footprint(frame), rect(7.0, 7.0, 26.0, 26.0), "the NaN offset counts as zero");
}

#[test]
fn headless_the_repaint_area_covers_both_the_shadow_and_the_frame() {
    let shadow = Shadow { opacity: 1.0, radius: 0.0, offset: (10.0, 0.0) };
    assert_eq!(shadow.repaint_rect(rect(0.0, 0.0, 10.0, 10.0)), rect(0.0, 0.0, 20.0, 10.0));
    let inside = Shadow { opacity: 1.0, radius: 6.0, offset: (0.0, 0.0) };
    assert_eq!(inside.repaint_rect(rect(10.0, 10.0, 10.0, 10.0)), rect(4.0, 4.0, 22.0, 22.0));
}

#[test]
fn headless_the_content_mask_cuts_a_masked_corner_and_keeps_the_edges() {
    let frame = rect(0.0, 0.0, 100.0, 100.0);
    assert!(!mask_contains(frame, 20.0, CornerMask::ALL, 1.0, 1.0), "the corner outside the arc is clipped");
    assert!(mask_contains(frame, 20.0, CornerMask::ALL, 20.0, 20.0), "the arc centre is inside");
    assert!(mask_contains(frame, 20.0, CornerMask::ALL, 50.0, 1.0), "the straight top edge is inside");
    assert!(mask_contains(frame, 20.0, CornerMask::ALL, 1.0, 50.0), "the straight left edge is inside");
}

#[test]
fn headless_the_content_mask_leaves_unmasked_corners_alone() {
    let frame = rect(0.0, 0.0, 100.0, 100.0);
    assert!(mask_contains(frame, 20.0, CornerMask::MIN_X_MIN_Y, 99.0, 99.0), "the bottom-right is not masked");
    assert!(!mask_contains(frame, 20.0, CornerMask::MIN_X_MIN_Y, 1.0, 1.0));
    assert!(!mask_contains(frame, 20.0, CornerMask::MAX_X_MAX_Y, 99.0, 99.0));
}

#[test]
fn headless_the_content_mask_is_half_open_like_the_frame() {
    let frame = rect(0.0, 0.0, 100.0, 100.0);
    assert!(!mask_contains(frame, 0.0, CornerMask::NONE, 100.0, 50.0), "the far edge is outside");
    assert!(mask_contains(frame, 0.0, CornerMask::NONE, 0.0, 0.0), "the near edge is inside");
}

#[test]
fn headless_an_oversized_radius_on_the_content_mask_is_capped_to_half_the_side() {
    let frame = rect(0.0, 0.0, 100.0, 100.0);
    assert!(!mask_contains(frame, 200.0, CornerMask::ALL, 5.0, 5.0));
    assert!(mask_contains(frame, 200.0, CornerMask::ALL, 25.0, 25.0), "the capped circle reaches (25, 25)");
}
