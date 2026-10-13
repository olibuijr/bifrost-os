//! Contract tests for `ZStack`, `ViewThatFits` and `Grid` (`overlay.rs`).
use super::overlay::{Columns, Grid, ViewThatFits, ZStack};
use super::{View, ViewBase};
use crate::kit::{Font, Ui};
use bifrost_ui::geometry::EdgeInsets;
use bifrost_ui::stack::Anchor;
use bifrost_ui::Rect;

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

const B: Rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 };

/// A leaf view with a fixed intrinsic size (zero means no preference).
struct Sized {
    base: ViewBase,
    size: (f32, f32),
}

fn leaf(w: f32, h: f32) -> Box<dyn View> {
    Box::new(Sized { base: ViewBase::default(), size: (w, h) })
}

impl View for Sized {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        self.size
    }
}

fn frames(v: &dyn View) -> Vec<Rect> {
    v.children().iter().map(|c| c.frame()).collect()
}

#[test]
fn zstack_fills_or_aligns_each_layer_in_the_same_area() {
    let mut z = ZStack::new(vec![leaf(0.0, 0.0), leaf(50.0, 20.0)]).anchored(Anchor::BottomTrailing);
    z.layout(&ui(), B);
    assert_eq!(frames(&z), vec![B, Rect { x: 250.0, y: 80.0, w: 50.0, h: 20.0 }]);
    assert_eq!(z.frame(), B);
}

#[test]
fn zstack_respects_insets_and_its_intrinsic_size_is_the_largest_child() {
    let mut z = ZStack::new(vec![leaf(0.0, 0.0), leaf(400.0, 10.0)]).inset(EdgeInsets::all(10.0));
    z.layout(&ui(), B);
    assert_eq!(z.children()[0].frame(), Rect { x: 10.0, y: 10.0, w: 280.0, h: 80.0 });
    assert_eq!(z.children()[1].frame().w, 280.0, "clamped to the inset area");
    assert_eq!(z.intrinsic(&ui()), (400.0, 10.0));
}

#[test]
fn view_that_fits_shows_the_first_candidate_that_fits_and_hides_the_rest() {
    let mut v = ViewThatFits::new(vec![leaf(320.0, 24.0), leaf(100.0, 24.0), leaf(40.0, 14.0)]);
    v.layout(&ui(), Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 });
    assert_eq!(v.shown(), Some(1), "the 320-point row is too wide");
    assert!(v.children()[0].base().hidden && !v.children()[1].base().hidden && v.children()[2].base().hidden);
    assert_eq!(v.children()[1].frame(), Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 });
    assert_eq!(v.intrinsic(&ui()), (100.0, 24.0));
}

#[test]
fn view_that_fits_falls_back_to_the_last_candidate_and_honours_the_axes() {
    let mut v = ViewThatFits::new(vec![leaf(320.0, 24.0), leaf(100.0, 24.0), leaf(40.0, 14.0)]);
    v.layout(&ui(), Rect { x: 0.0, y: 0.0, w: 30.0, h: 50.0 });
    assert_eq!(v.shown(), Some(2), "nothing fits 30 points wide, so the fallback shows");
    let mut tall = ViewThatFits::new(vec![leaf(500.0, 20.0), leaf(50.0, 900.0)]).constrained(true, false);
    tall.layout(&ui(), Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 });
    assert_eq!(tall.shown(), Some(1), "height is free, so the tall candidate fits");
}

#[test]
fn grid_fills_rows_left_to_right_in_equal_cells() {
    let mut g = Grid::new(Columns::Fixed(3), 20.0, 10.0, vec![leaf(0.0, 0.0), leaf(0.0, 0.0), leaf(0.0, 0.0), leaf(0.0, 0.0)]);
    g.layout(&ui(), B);
    let f = frames(&g);
    assert_eq!(f[3].x, f[0].x, "the fourth cell wraps to column 0");
    assert_eq!(f[3].y, 30.0);
    assert!((f[2].right() - 300.0).abs() < 0.01, "the last column ends at the bounds");
}

#[test]
fn adaptive_grid_picks_the_column_count_from_the_width() {
    let g = Grid::new(Columns::Adaptive { min_cell: 200.0 }, 40.0, 16.0, vec![]);
    assert_eq!(g.column_count(416.0), 2);
    assert_eq!(g.column_count(415.0), 1);
    assert_eq!(Grid::new(Columns::Fixed(0), 40.0, 0.0, vec![]).column_count(300.0), 1);
}

#[test]
fn zero_row_height_uses_the_tallest_child() {
    let mut g = Grid::new(Columns::Fixed(2), 0.0, 0.0, vec![leaf(50.0, 40.0), leaf(50.0, 30.0), leaf(50.0, 25.0)]);
    g.layout(&ui(), B);
    assert_eq!(g.children()[2].frame().y, 40.0);
    assert_eq!(g.children()[2].frame().h, 40.0);
}
