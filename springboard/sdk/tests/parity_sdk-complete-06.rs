//! Parity sdk-complete-06: gradient layer content (CAGradientLayer) as a headless model. The colours are read at unit
//! positions of the content: no layer is attached, no frame is composed for a display, and no window is opened.

use bifrost_app::layer_kinds::gradient::{GradientContent, GradientType};

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn red(c: [u8; 4]) -> i32 {
    c[0] as i32
}

#[test]
fn an_axial_gradient_runs_between_its_start_and_end_points_and_holds_beyond_them() {
    let mut g = GradientContent::new(GradientType::Axial, vec![BLACK, WHITE]);
    g.start = (0.25, 0.5);
    g.end = (0.75, 0.5);
    assert!(red(g.color_at(0.25, 0.5)) <= 2, "the start point is the first colour");
    assert!(red(g.color_at(0.75, 0.5)) >= 253, "the end point is the last colour");
    assert!((red(g.color_at(0.5, 0.5)) - 128).abs() <= 2, "the midpoint is halfway along the ramp");
    assert!(red(g.color_at(0.0, 0.5)) <= 2, "before the start point the first colour holds");
    assert!(red(g.color_at(1.0, 0.5)) >= 253, "past the end point the last colour holds");
}

#[test]
fn an_axial_gradient_ignores_positions_across_its_axis() {
    let mut g = GradientContent::new(GradientType::Axial, vec![BLACK, WHITE]);
    g.start = (0.25, 0.5);
    g.end = (0.75, 0.5);
    assert_eq!(g.color_at(0.5, 0.0), g.color_at(0.5, 1.0), "a horizontal axis gives one colour per column");
}

#[test]
fn a_three_stop_axial_gradient_puts_its_middle_colour_at_the_middle_location() {
    let mut g = GradientContent::new(GradientType::Axial, vec![BLACK, WHITE, BLACK]);
    g.locations = Some(vec![0.0, 0.5, 1.0]);
    assert!(red(g.color_at(0.5, 0.5)) >= 253, "the middle stop is white at the midpoint");
    assert!((red(g.color_at(0.5, 0.25)) - 128).abs() <= 2, "a quarter of the way is halfway to the middle stop");
    assert!(red(g.color_at(0.5, 0.0)) <= 2, "the first stop is black at the top");
}

#[test]
fn a_radial_gradient_is_halfway_at_half_its_radius_in_headless_unit_space() {
    let mut g = GradientContent::new(GradientType::Radial, vec![WHITE, BLACK]);
    g.start = (0.5, 0.5);
    g.end = (1.0, 0.5);
    assert!((red(g.color_at(0.75, 0.5)) - 128).abs() <= 2, "the radius runs from the start point to the end point");
}
