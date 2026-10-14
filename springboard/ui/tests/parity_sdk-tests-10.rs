//! Stack, distribution, grid, overlay and fitting layout as pure functions from a size to rectangles.
//!
//! Headless: the layout functions take numbers and return rectangles. No window, display or device is involved, and
//! every expected value below is worked out by hand from the arithmetic.

use bifrost_ui::geometry::EdgeInsets;
use bifrost_ui::stack::{adaptive_columns, distribute, first_fitting, grid, overlay, stack, Anchor, Axis, Distribution, StackItem};
use bifrost_ui::Rect;

const BOUNDS: Rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 };
const NO_INSETS: EdgeInsets = EdgeInsets { top: 0.0, left: 0.0, bottom: 0.0, right: 0.0 };

fn widths(rects: &[Rect]) -> Vec<f32> {
    rects.iter().map(|r| r.w).collect()
}

fn xs(rects: &[Rect]) -> Vec<f32> {
    rects.iter().map(|r| r.x).collect()
}

#[test]
fn fixed_items_keep_their_size_and_are_separated_by_the_spacing() {
    let rects = stack(BOUNDS, Axis::Horizontal, 10.0, NO_INSETS, &[StackItem::fixed(50.0), StackItem::fixed(80.0)]);
    assert_eq!(widths(&rects), [50.0, 80.0]);
    assert_eq!(xs(&rects), [0.0, 60.0]);
    assert!(rects.iter().all(|r| r.y == 0.0 && r.h == 100.0), "each item takes the full cross axis");
}

#[test]
fn spare_space_is_shared_by_flexible_items_in_proportion_to_their_weight() {
    let rects = stack(BOUNDS, Axis::Horizontal, 0.0, NO_INSETS, &[StackItem::flexible(0.0, 1.0), StackItem::flexible(0.0, 3.0)]);
    assert_eq!(widths(&rects), [75.0, 225.0]);
}

#[test]
fn a_vertical_stack_lays_out_along_the_height() {
    let rects = stack(BOUNDS, Axis::Vertical, 0.0, NO_INSETS, &[StackItem::fixed(20.0), StackItem::flexible(0.0, 1.0)]);
    let heights: Vec<f32> = rects.iter().map(|r| r.h).collect();
    assert_eq!(heights, [20.0, 80.0]);
    assert_eq!(rects.iter().map(|r| r.y).collect::<Vec<_>>(), [0.0, 20.0]);
    assert!(rects.iter().all(|r| r.w == 300.0 && r.x == 0.0));
}

#[test]
fn insets_shrink_the_area_the_stack_fills() {
    let insets = EdgeInsets { top: 5.0, left: 10.0, bottom: 5.0, right: 20.0 };
    let rects = stack(BOUNDS, Axis::Horizontal, 0.0, insets, &[StackItem::flexible(0.0, 1.0)]);
    assert_eq!(rects, [Rect { x: 10.0, y: 5.0, w: 270.0, h: 90.0 }]);
}

#[test]
fn a_shortfall_is_taken_from_the_flexible_items_first() {
    let rects = stack(BOUNDS, Axis::Horizontal, 0.0, NO_INSETS, &[StackItem::fixed(200.0), StackItem::flexible(200.0, 1.0)]);
    assert_eq!(widths(&rects), [200.0, 100.0]);
    assert_eq!(xs(&rects), [0.0, 200.0]);
}

#[test]
fn a_shortfall_with_no_flexible_item_shrinks_every_item_proportionally() {
    let rects = stack(BOUNDS, Axis::Horizontal, 0.0, NO_INSETS, &[StackItem::fixed(200.0), StackItem::fixed(200.0)]);
    assert_eq!(widths(&rects), [150.0, 150.0]);
    assert_eq!(xs(&rects), [0.0, 150.0]);
}

#[test]
fn negative_sizes_are_treated_as_zero() {
    let rects = stack(BOUNDS, Axis::Horizontal, 0.0, NO_INSETS, &[StackItem::fixed(-5.0), StackItem::fixed(40.0)]);
    assert_eq!(widths(&rects), [0.0, 40.0]);
}

#[test]
fn fill_equally_gives_every_view_the_same_length() {
    let parts = distribute(300.0, 10.0, &[10.0, 20.0], Distribution::FillEqually);
    assert_eq!(parts, [(0.0, 145.0), (155.0, 145.0)]);
}

#[test]
fn fill_proportionally_keeps_the_size_ratio_and_fills_the_stack() {
    let parts = distribute(200.0, 0.0, &[10.0, 30.0], Distribution::FillProportionally);
    assert_eq!(parts, [(0.0, 50.0), (50.0, 150.0)]);
}

#[test]
fn fill_proportionally_with_no_intrinsic_length_splits_the_space_evenly() {
    let parts = distribute(100.0, 0.0, &[0.0, 0.0], Distribution::FillProportionally);
    assert_eq!(parts, [(0.0, 50.0), (50.0, 50.0)]);
}

#[test]
fn equal_spacing_spreads_the_gaps_across_the_stack() {
    let parts = distribute(100.0, 0.0, &[10.0, 10.0, 10.0], Distribution::EqualSpacing);
    assert_eq!(parts, [(0.0, 10.0), (45.0, 10.0), (90.0, 10.0)]);
}

#[test]
fn equal_spacing_never_closes_a_gap_below_the_spacing() {
    let parts = distribute(25.0, 5.0, &[10.0, 10.0, 10.0], Distribution::EqualSpacing);
    assert_eq!(parts.iter().map(|p| p.0).collect::<Vec<_>>(), [0.0, 15.0, 30.0]);
}

#[test]
fn equal_centering_keeps_the_centres_equally_far_apart() {
    let parts = distribute(100.0, 0.0, &[10.0, 10.0, 10.0], Distribution::EqualCentering);
    assert_eq!(parts, [(0.0, 10.0), (45.0, 10.0), (90.0, 10.0)]);
}

#[test]
fn the_default_distribution_packs_intrinsic_sizes_with_the_spacing() {
    let parts = distribute(300.0, 5.0, &[10.0, 20.0], Distribution::Fill);
    assert_eq!(parts, [(0.0, 10.0), (15.0, 20.0)]);
    assert!(distribute(300.0, 5.0, &[], Distribution::FillEqually).is_empty());
}

#[test]
fn adaptive_columns_fit_as_many_minimum_cells_as_the_width_allows() {
    assert_eq!(adaptive_columns(300.0, 100.0, 10.0), 2, "three cells need 320");
    assert_eq!(adaptive_columns(320.0, 100.0, 10.0), 3, "exactly three cells");
    assert_eq!(adaptive_columns(90.0, 100.0, 0.0), 1, "narrower than one cell");
    assert_eq!(adaptive_columns(500.0, 0.0, 0.0), 1, "no minimum size gives one column");
}

#[test]
fn a_grid_fills_rows_left_to_right() {
    let cells = grid(BOUNDS, 2, 10.0, 5.0, 20.0, 3);
    assert_eq!(
        cells,
        [
            Rect { x: 0.0, y: 0.0, w: 145.0, h: 20.0 },
            Rect { x: 155.0, y: 0.0, w: 145.0, h: 20.0 },
            Rect { x: 0.0, y: 25.0, w: 145.0, h: 20.0 },
        ]
    );
}

#[test]
fn a_grid_with_no_columns_is_one_column() {
    let cells = grid(BOUNDS, 0, 0.0, 0.0, 10.0, 2);
    assert_eq!(cells, [Rect { x: 0.0, y: 0.0, w: 300.0, h: 10.0 }, Rect { x: 0.0, y: 10.0, w: 300.0, h: 10.0 }]);
}

#[test]
fn an_overlay_aligns_each_layer_to_the_anchor() {
    assert_eq!(overlay(BOUNDS, NO_INSETS, Anchor::Center, &[(100.0, 40.0)]), [Rect { x: 100.0, y: 30.0, w: 100.0, h: 40.0 }]);
    assert_eq!(overlay(BOUNDS, NO_INSETS, Anchor::TopLeading, &[(100.0, 40.0)]), [Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 }]);
    assert_eq!(overlay(BOUNDS, NO_INSETS, Anchor::BottomTrailing, &[(100.0, 40.0)]), [Rect { x: 200.0, y: 60.0, w: 100.0, h: 40.0 }]);
}

#[test]
fn an_overlay_layer_larger_than_the_area_is_clamped_to_it() {
    assert_eq!(overlay(BOUNDS, NO_INSETS, Anchor::Center, &[(400.0, 150.0)]), [Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 }]);
    assert_eq!(overlay(BOUNDS, NO_INSETS, Anchor::Center, &[(-5.0, 10.0)])[0].w, 0.0);
}

#[test]
fn anchors_name_their_fractions_of_the_free_space() {
    assert_eq!(Anchor::Leading.unit(), (0.0, 0.5));
    assert_eq!(Anchor::Trailing.unit(), (1.0, 0.5));
    assert_eq!(Anchor::Top.unit(), (0.5, 0.0));
    assert_eq!(Anchor::BottomLeading.unit(), (0.0, 1.0));
}

#[test]
fn view_that_fits_picks_the_first_candidate_that_fits_the_constrained_axes() {
    let candidates = [(400.0, 50.0), (250.0, 120.0), (200.0, 90.0)];
    assert_eq!(first_fitting((300.0, 100.0), &candidates, (true, true)), Some(2));
    assert_eq!(first_fitting((300.0, 100.0), &candidates, (true, false)), Some(1));
    assert_eq!(first_fitting((300.0, 100.0), &[(400.0, 400.0)], (true, true)), None);
    assert_eq!(first_fitting((300.0, 100.0), &[(300.00005, 10.0)], (true, true)), Some(0), "within the 1e-4 tolerance");
}
