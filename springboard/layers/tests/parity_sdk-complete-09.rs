//! Parity sdk-complete-09: headless tests for glass grouping in a glass effect container and the glass material it
//! draws. No test composes a frame.

use bifrost_layers::geometry::{Color, Rect};
use bifrost_layers::glass_group::group_shapes;
use bifrost_layers::layer::{Layer, LayerId, Material};

fn square(x: f32, y: f32) -> Rect {
    Rect::new(x, y, 10.0, 10.0)
}

#[test]
fn shapes_within_the_spacing_of_each_other_form_one_glass_group() {
    let shapes = [square(0.0, 0.0), square(12.0, 0.0), square(40.0, 0.0)];
    assert_eq!(group_shapes(&shapes, 2.0), vec![vec![0, 1], vec![2]]);
}

#[test]
fn shapes_farther_apart_than_the_spacing_stay_in_separate_groups() {
    let shapes = [square(0.0, 0.0), square(12.0, 0.0), square(40.0, 0.0)];
    assert_eq!(group_shapes(&shapes, 1.0), vec![vec![0], vec![1], vec![2]]);
}

#[test]
fn overlapping_shapes_join_at_zero_spacing() {
    let shapes = [square(0.0, 0.0), square(5.0, 5.0)];
    assert_eq!(group_shapes(&shapes, 0.0), vec![vec![0, 1]]);
}

#[test]
fn a_diagonal_neighbour_joins_when_both_gaps_are_within_the_spacing() {
    // Gaps of 2 horizontally and 2 vertically: the larger gap is 2.
    let shapes = [square(0.0, 0.0), square(12.0, 12.0)];
    assert_eq!(group_shapes(&shapes, 2.0), vec![vec![0, 1]]);
    assert_eq!(group_shapes(&shapes, 1.9), vec![vec![0], vec![1]]);
}

#[test]
fn joining_is_transitive_along_a_chain_of_shapes() {
    let shapes = [square(0.0, 0.0), square(12.0, 0.0), square(24.0, 0.0)];
    assert_eq!(group_shapes(&shapes, 2.0), vec![vec![0, 1, 2]]);
}

#[test]
fn groups_are_listed_by_their_lowest_index_and_hold_ascending_indices() {
    let shapes = [square(40.0, 0.0), square(0.0, 0.0), square(12.0, 0.0)];
    assert_eq!(group_shapes(&shapes, 2.0), vec![vec![0], vec![1, 2]]);
}

#[test]
fn a_shape_with_a_non_finite_frame_stands_alone() {
    let shapes = [square(0.0, 0.0), Rect::new(f32::NAN, 0.0, 1.0, 1.0)];
    assert_eq!(group_shapes(&shapes, 100.0), vec![vec![0], vec![1]]);
}

#[test]
fn a_negative_spacing_counts_as_zero() {
    let shapes = [square(0.0, 0.0), square(12.0, 0.0)];
    assert_eq!(group_shapes(&shapes, -5.0), vec![vec![0], vec![1]]);
    assert!(group_shapes(&[], 2.0).is_empty());
}

#[test]
fn a_glass_material_is_the_same_only_when_every_parameter_matches() {
    let mut layer = Layer::new(LayerId(7));
    assert_eq!(layer.material, Material::None);
    layer.material = Material::Glass { blur: 10.0, tint: Color::CLEAR, saturation: 1.0 };
    assert_ne!(layer.material, Material::Glass { blur: 12.0, tint: Color::CLEAR, saturation: 1.0 });
    assert_eq!(layer.material, Material::Glass { blur: 10.0, tint: Color::CLEAR, saturation: 1.0 });
}
