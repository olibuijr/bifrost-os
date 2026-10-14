//! Parity sdk-complete-02: headless tests for the pattern wrapper. Cells are sampled in memory; no surface is drawn.

use bifrost_app::graphics_context::GraphicsContext;
use bifrost_app::graphics_gradient::Affine;
use bifrost_app::graphics_layer::CgLayer;
use bifrost_app::graphics_pattern::{CgPattern, PatternTiling};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn cell() -> Vec<u32> {
    vec![1, 2, 3, 4]
}

fn pattern(tiling: PatternTiling, matrix: Affine, step: (f32, f32)) -> CgPattern {
    CgPattern::from_pixels(2, 2, cell(), (2.0, 2.0), step, matrix, tiling).unwrap()
}

#[test]
fn a_point_inside_a_cell_reads_the_pixel_at_that_position() {
    let p = pattern(PatternTiling::NoDistortion, Affine::IDENTITY, (4.0, 4.0));
    assert_eq!(p.shade(0.5, 0.5), 1, "top left");
    assert_eq!(p.shade(1.5, 0.5), 2, "top right");
    assert_eq!(p.shade(0.5, 1.5), 3, "bottom left");
    assert_eq!(p.shade(1.5, 1.5), 4, "bottom right");
}

#[test]
fn the_space_between_cells_is_transparent() {
    let p = pattern(PatternTiling::NoDistortion, Affine::IDENTITY, (4.0, 4.0));
    assert_eq!(p.shade(2.5, 0.5), 0, "past the drawn bounds of the cell");
    assert_eq!(p.shade(0.5, 3.5), 0, "below the drawn bounds, before the next row");
}

#[test]
fn the_cell_repeats_every_step_in_both_directions() {
    let p = pattern(PatternTiling::NoDistortion, Affine::IDENTITY, (4.0, 4.0));
    assert_eq!(p.shade(4.5, 0.5), 1, "one step to the right");
    assert_eq!(p.shade(-3.5, 4.5), 1, "one step left and one step down");
}

#[test]
fn the_pattern_matrix_moves_the_cells_in_user_space() {
    let p = pattern(PatternTiling::NoDistortion, Affine::translation(10.0, 0.0), (4.0, 4.0));
    assert_eq!(p.shade(10.5, 0.5), 1, "the first cell now starts at x = 10");
    assert_eq!(p.shade(0.5, 0.5), 0, "nothing is drawn where the cell used to be");
}

#[test]
fn a_mismatched_pixel_count_or_an_empty_cell_is_refused() {
    let short = CgPattern::from_pixels(2, 2, vec![1, 2, 3], (2.0, 2.0), (4.0, 4.0), Affine::IDENTITY, PatternTiling::NoDistortion);
    assert!(short.is_none());
    let empty = CgPattern::from_pixels(0, 2, Vec::new(), (2.0, 2.0), (4.0, 4.0), Affine::IDENTITY, PatternTiling::NoDistortion);
    assert!(empty.is_none());
}

#[test]
fn a_singular_matrix_or_a_zero_step_is_refused() {
    let singular = Affine { a: 1.0, b: 2.0, c: 2.0, d: 4.0, tx: 0.0, ty: 0.0 };
    let flat = CgPattern::from_pixels(2, 2, cell(), (2.0, 2.0), (4.0, 4.0), singular, PatternTiling::NoDistortion);
    assert!(flat.is_none());
    let no_step = CgPattern::from_pixels(2, 2, cell(), (2.0, 2.0), (0.0, 4.0), Affine::IDENTITY, PatternTiling::NoDistortion);
    assert!(no_step.is_none());
}

#[test]
fn the_default_tiling_is_no_distortion_and_the_step_is_kept_when_fitted() {
    assert_eq!(PatternTiling::default(), PatternTiling::NoDistortion);
    let p = pattern(PatternTiling::NoDistortion, Affine::IDENTITY, (3.3, 3.3));
    let fitted = p.fitted(Affine::scale(2.0, 2.0));
    assert!(close(fitted.step().0, 3.3), "no distortion leaves the step alone");
    assert_eq!(fitted.tiling(), PatternTiling::NoDistortion);
}

#[test]
fn constant_spacing_rounds_the_step_to_whole_device_pixels() {
    let p = pattern(PatternTiling::ConstantSpacing, Affine::IDENTITY, (3.3, 3.3));
    let fitted = p.fitted(Affine::IDENTITY);
    assert!(close(fitted.step().0, 3.0), "3.3 device pixels round to 3");
    assert!(close(fitted.bounds().0, 2.0), "constant spacing changes the spacing, not the cell");
}

#[test]
fn constant_spacing_under_a_scale_rounds_in_device_pixels() {
    let p = pattern(PatternTiling::ConstantSpacing, Affine::IDENTITY, (3.3, 3.3));
    let fitted = p.fitted(Affine::scale(2.0, 2.0));
    assert!(close(fitted.step().0, 3.5), "3.3 user units at scale two is 6.6 pixels, which rounds to 7");
}

#[test]
fn minimal_distortion_stretches_the_cell_with_the_step() {
    let p = pattern(PatternTiling::ConstantSpacingMinimalDistortion, Affine::IDENTITY, (3.3, 3.3));
    let fitted = p.fitted(Affine::IDENTITY);
    let factor = 3.0 / 3.3;
    assert!(close(fitted.step().0, 3.0));
    assert!(close(fitted.bounds().0, 2.0 * factor), "the cell shrinks by the same factor as the step");
}

#[test]
fn a_pattern_keeps_its_tiling_when_it_is_fitted_again() {
    let p = pattern(PatternTiling::ConstantSpacing, Affine::IDENTITY, (3.3, 3.3));
    assert_eq!(p.fitted(Affine::IDENTITY).tiling(), PatternTiling::ConstantSpacing);
}

// Layers: the offscreen group an app builds once and draws into a context many times.

#[test]
fn a_new_layer_is_transparent_and_has_no_draws() {
    let layer = CgLayer::new(3, 2).unwrap();
    assert_eq!(layer.size(), (3, 2));
    assert_eq!(layer.draws(), 0);
    assert_eq!(layer.pixels().px, vec![0; 6]);
}

#[test]
fn a_layer_refuses_an_empty_or_overflowing_size() {
    assert!(CgLayer::new(0, 4).is_none());
    assert!(CgLayer::new(4, 0).is_none());
    assert!(CgLayer::new(usize::MAX, 2).is_none());
}

#[test]
fn later_draw_passes_keep_the_pixels_of_earlier_passes() {
    let mut layer = CgLayer::new(3, 2).unwrap();
    layer.draw(|ctx| ctx.layer.px[0] = 0xff11_2233);
    layer.draw(|ctx| ctx.layer.px[5] = 0xff44_5566);
    assert_eq!(layer.pixels().px[0], 0xff11_2233);
    assert_eq!(layer.pixels().px[5], 0xff44_5566);
    assert_eq!(layer.draws(), 2);
}

#[test]
fn a_layer_is_drawn_into_a_context_at_an_offset() {
    let mut badge = CgLayer::new(2, 2).unwrap();
    badge.draw(|ctx| ctx.layer.px[0] = 0xff11_2233);
    let mut page = GraphicsContext::new(4, 4);
    page.draw_layer_at(badge.pixels(), 1, 1);
    assert_eq!(page.layer.px[4 + 1], 0xff11_2233, "the layer's first pixel lands at (1, 1)");
    assert_eq!(page.layer.px[0], 0, "nothing lands outside the offset layer");
    assert_eq!(badge.draws(), 1, "drawing the layer into a context does not count as a layer draw");
}

// Lines and arcs: single strokes drawn with the context's stroke colour and line width, headless on a pixel buffer.

#[test]
fn a_stroked_line_paints_the_pixels_it_crosses_and_no_others() {
    let mut ctx = GraphicsContext::new(10, 10);
    ctx.set_stroke_color(0xffff_0000);
    ctx.set_line_width(2.0);
    ctx.stroke_line((1.0, 5.0), (8.0, 5.0));
    let at = |x: usize, y: usize| ctx.layer.px[y * 10 + x];
    assert_eq!(at(4, 5) >> 24, 0xff, "a pixel on the line is fully covered");
    assert_eq!(at(0, 0) >> 24, 0, "a pixel far from the line stays transparent");
}

#[test]
fn a_stroked_line_leaves_the_current_path_alone() {
    let mut ctx = GraphicsContext::new(10, 10);
    ctx.begin_path();
    ctx.move_to(0.0, 0.0);
    ctx.add_line(9.0, 0.0);
    ctx.stroke_line((1.0, 5.0), (8.0, 5.0));
    assert!(!ctx.current_path().is_empty(), "the path built before the line is still there");
}

#[test]
fn a_stroked_arc_paints_the_ring_and_not_its_centre() {
    let mut ctx = GraphicsContext::new(20, 20);
    ctx.set_stroke_color(0xffff_0000);
    ctx.set_line_width(2.0);
    ctx.stroke_arc((10.0, 10.0), 6.0, 0.0, std::f32::consts::PI, false);
    let at = |x: usize, y: usize| ctx.layer.px[y * 20 + x];
    assert!(at(15, 10) >> 24 > 0, "a pixel on the ring is painted");
    assert_eq!(at(10, 10) >> 24, 0, "the centre of the ring is not painted");
}
