//! Parity sdk-tests-05: headless tests for the lasso selection model (`bifrost_ui::lasso`) and the tool picker state
//! (`bifrost_ui::tool_picker`). Both are pure models over drawing data: no canvas is drawn and no input device is read.

use bifrost_ui::cg::path::Affine;
use bifrost_ui::drawing::{Drawing, Ink, InkKind, Stroke, StrokePoint};
use bifrost_ui::lasso::{Lasso, Selection};
use bifrost_ui::tool_picker::{width_range, EraserKind, Tool, ToolItem, ToolPicker};
use bifrost_ui::Rect;

fn square_loop() -> Lasso {
    let mut l = Lasso::new();
    for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        l.add(x, y);
    }
    l
}

fn stroke(points: &[(f32, f32)]) -> Stroke {
    let points = points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect();
    Stroke { ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 }, points }
}

fn pen(color: u32, width: f32) -> Ink {
    Ink { kind: InkKind::Pen, color, width }
}

fn assert_rect(r: Option<Rect>, x: f32, y: f32, w: f32, h: f32) {
    let r = r.expect("a rectangle");
    assert_eq!((r.x, r.y, r.w, r.h), (x, y, w, h));
}

// Lasso: the loop and its hit test.

#[test]
fn headless_a_loop_with_fewer_than_three_points_encloses_nothing() {
    let mut l = Lasso::new();
    assert!(!l.contains(0.0, 0.0), "an empty loop");
    l.add(0.0, 0.0);
    l.add(10.0, 10.0);
    assert!(!l.contains(5.0, 5.0), "two points do not close an area");
}

#[test]
fn a_lasso_point_with_a_non_finite_coordinate_is_dropped() {
    let mut l = Lasso::new();
    l.add(f32::NAN, 0.0);
    l.add(0.0, f32::INFINITY);
    l.add(1.0, 2.0);
    assert_eq!(l.points(), &[(1.0, 2.0)]);
}

#[test]
fn headless_a_square_loop_contains_its_interior_and_not_its_outside() {
    let l = square_loop();
    assert!(l.contains(5.0, 5.0));
    assert!(!l.contains(15.0, 5.0));
    assert!(!l.contains(-1.0, 5.0));
    assert!(!l.contains(5.0, 11.0));
}

#[test]
fn headless_a_concave_loop_leaves_its_notch_outside() {
    let mut l = Lasso::new();
    for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (7.0, 10.0), (7.0, 3.0), (3.0, 3.0), (3.0, 10.0), (0.0, 10.0)] {
        l.add(x, y);
    }
    assert!(l.contains(1.0, 5.0), "the left arm");
    assert!(l.contains(5.0, 1.0), "the base");
    assert!(!l.contains(5.0, 5.0), "the notch between the arms");
}

// Lasso: selecting strokes and transforming the selection.

#[test]
fn headless_selection_takes_every_stroke_with_a_point_inside_the_loop() {
    let drawing = Drawing::from_strokes(vec![
        stroke(&[(5.0, 5.0), (50.0, 50.0)]),
        stroke(&[(40.0, 40.0), (60.0, 60.0)]),
        stroke(&[(2.0, 2.0), (3.0, 3.0)]),
    ]);
    let sel = square_loop().select(&drawing);
    assert_eq!(sel.indices(), &[0, 2], "a stroke crossing the loop counts, a stroke outside does not");
    assert!(sel.contains_stroke(0) && !sel.contains_stroke(1) && sel.contains_stroke(2));
}

#[test]
fn headless_selection_of_an_empty_drawing_is_empty() {
    let sel = square_loop().select(&Drawing::new());
    assert!(sel.is_empty());
    assert_eq!(sel.bounds(&Drawing::new()), None);
}

#[test]
fn headless_selection_bounds_cover_only_the_selected_strokes() {
    let drawing = Drawing::from_strokes(vec![stroke(&[(1.0, 1.0), (4.0, 4.0)]), stroke(&[(100.0, 100.0), (101.0, 101.0)]), stroke(&[(20.0, 20.0), (30.0, 25.0)])]);
    let sel = Selection::from_indices(vec![0, 2]);
    assert_rect(sel.bounds(&drawing), 1.0, 1.0, 29.0, 24.0);
    assert_eq!(Selection::from_indices(vec![]).bounds(&drawing), None);
}

#[test]
fn from_indices_sorts_and_drops_repeats() {
    let sel = Selection::from_indices(vec![3, 1, 3, 2]);
    assert_eq!(sel.indices(), &[1, 2, 3]);
    assert!(sel.contains_stroke(2) && !sel.contains_stroke(0));
}

#[test]
fn moving_a_selection_moves_only_the_selected_strokes() {
    let mut drawing = Drawing::from_strokes(vec![stroke(&[(1.0, 1.0)]), stroke(&[(5.0, 5.0)])]);
    Selection::from_indices(vec![0]).transform(&mut drawing, Affine::translate(10.0, 5.0));
    assert_eq!((drawing.strokes()[0].points[0].x, drawing.strokes()[0].points[0].y), (11.0, 6.0));
    assert_eq!((drawing.strokes()[1].points[0].x, drawing.strokes()[1].points[0].y), (5.0, 5.0), "the unselected stroke stays");
}

#[test]
fn scaling_a_selection_doubles_its_points_about_the_origin() {
    let mut drawing = Drawing::from_strokes(vec![stroke(&[(3.0, 4.0), (6.0, 8.0)])]);
    Selection::from_indices(vec![0]).transform(&mut drawing, Affine::scale(2.0, 2.0));
    let pts: Vec<(f32, f32)> = drawing.strokes()[0].points.iter().map(|p| (p.x, p.y)).collect();
    assert_eq!(pts, vec![(6.0, 8.0), (12.0, 16.0)]);
}

#[test]
fn a_selection_with_a_stale_index_moves_the_strokes_that_remain() {
    let mut drawing = Drawing::from_strokes(vec![stroke(&[(1.0, 1.0)])]);
    Selection::from_indices(vec![0, 9]).transform(&mut drawing, Affine::translate(1.0, 0.0));
    assert_eq!(drawing.strokes()[0].points[0].x, 2.0, "the index that is no longer in the drawing is skipped");
}

// Tool picker: the default set, selection and visibility.

#[test]
fn headless_the_default_picker_offers_the_inking_tools_both_erasers_and_the_lasso_in_order() {
    let picker = ToolPicker::new();
    let tools: Vec<Tool> = picker.items().iter().map(|i| i.tool).collect();
    assert_eq!(
        tools,
        vec![
            Tool::Ink(InkKind::Pen),
            Tool::Ink(InkKind::Marker),
            Tool::Ink(InkKind::Pencil),
            Tool::Eraser(EraserKind::Bitmap),
            Tool::Eraser(EraserKind::Vector),
            Tool::Lasso,
        ]
    );
    assert_eq!(picker.selected(), Tool::Ink(InkKind::Pen), "the pen is selected first");
    assert!(!picker.is_visible(), "the picker starts hidden");
}

#[test]
fn the_default_inking_tools_have_their_documented_widths() {
    let picker = ToolPicker::new();
    let width = |kind| picker.items().iter().find(|i| i.tool == Tool::Ink(kind)).map(|i| i.width);
    assert_eq!(width(InkKind::Pen), Some(3.0));
    assert_eq!(width(InkKind::Marker), Some(16.0));
    assert_eq!(width(InkKind::Pencil), Some(2.0));
}

#[test]
fn the_width_range_of_each_ink_kind_is_fixed() {
    assert_eq!(width_range(InkKind::Pen), (1.0, 20.0));
    assert_eq!(width_range(InkKind::Marker), (4.0, 40.0));
    assert_eq!(width_range(InkKind::Pencil), (1.0, 12.0));
}

#[test]
fn selecting_a_tool_on_the_picker_changes_the_selection() {
    let mut picker = ToolPicker::new();
    assert!(picker.select(Tool::Lasso));
    assert_eq!(picker.selected(), Tool::Lasso);
    assert!(picker.select(Tool::Ink(InkKind::Marker)));
    assert_eq!(picker.selected(), Tool::Ink(InkKind::Marker));
}

#[test]
fn selecting_a_tool_that_is_not_on_the_picker_keeps_the_selection() {
    let mut picker = ToolPicker::with_items(vec![ToolItem { tool: Tool::Ink(InkKind::Pencil), color: 0xff00_0000, width: 2.0 }]).expect("one item");
    assert!(!picker.select(Tool::Lasso), "the lasso is not on this picker");
    assert_eq!(picker.selected(), Tool::Ink(InkKind::Pencil));
}

#[test]
fn headless_the_current_ink_follows_the_selected_inking_tool() {
    let mut picker = ToolPicker::new();
    assert_eq!(picker.current_ink(), Some(pen(0xff00_0000, 3.0)));
    picker.select(Tool::Ink(InkKind::Marker));
    assert_eq!(picker.current_ink(), Some(Ink { kind: InkKind::Marker, color: 0xccff_d60a, width: 16.0 }));
}

#[test]
fn an_eraser_or_the_lasso_gives_no_current_ink() {
    let mut picker = ToolPicker::new();
    picker.select(Tool::Eraser(EraserKind::Vector));
    assert_eq!(picker.current_ink(), None);
    picker.select(Tool::Lasso);
    assert_eq!(picker.current_ink(), None);
}

#[test]
fn headless_a_colour_applies_to_inking_tools_only() {
    let mut picker = ToolPicker::new();
    assert!(picker.set_color(Tool::Ink(InkKind::Pencil), 0xff12_3456));
    picker.select(Tool::Ink(InkKind::Pencil));
    assert_eq!(picker.current_ink().map(|i| i.color), Some(0xff12_3456));
    assert!(!picker.set_color(Tool::Lasso, 0xff00_0000), "the lasso has no colour");
    assert!(!picker.set_color(Tool::Eraser(EraserKind::Bitmap), 0xff00_0000), "an eraser has no colour");
}

#[test]
fn headless_a_width_is_clamped_to_the_range_of_its_ink_kind() {
    let mut picker = ToolPicker::new();
    assert_eq!(picker.set_width(Tool::Ink(InkKind::Pen), 50.0), Some(20.0));
    assert_eq!(picker.set_width(Tool::Ink(InkKind::Pen), 0.0), Some(1.0));
    assert_eq!(picker.set_width(Tool::Ink(InkKind::Marker), 1.0), Some(4.0));
    assert_eq!(picker.set_width(Tool::Ink(InkKind::Pencil), 20.0), Some(12.0));
    picker.select(Tool::Ink(InkKind::Pencil));
    assert_eq!(picker.current_ink().map(|i| i.width), Some(12.0));
}

#[test]
fn a_width_is_refused_for_a_non_finite_value_or_a_tool_without_ink() {
    let mut picker = ToolPicker::new();
    assert_eq!(picker.set_width(Tool::Ink(InkKind::Pen), f32::NAN), None);
    assert_eq!(picker.set_width(Tool::Lasso, 4.0), None);
    assert_eq!(picker.set_width(Tool::Eraser(EraserKind::Bitmap), 4.0), None);
}

#[test]
fn a_custom_picker_is_refused_when_it_is_empty_repeats_a_tool_or_has_a_bad_width() {
    let pencil = ToolItem { tool: Tool::Ink(InkKind::Pencil), color: 0xff00_0000, width: 2.0 };
    assert!(ToolPicker::with_items(vec![]).is_none());
    assert!(ToolPicker::with_items(vec![pencil, pencil]).is_none(), "a tool appears once");
    let nan = ToolItem { width: f32::NAN, ..pencil };
    assert!(ToolPicker::with_items(vec![nan]).is_none(), "an inking width must be finite");
}

#[test]
fn a_custom_picker_clamps_its_widths_and_selects_its_first_item() {
    let wide = ToolItem { tool: Tool::Ink(InkKind::Pencil), color: 0xff00_0000, width: 99.0 };
    let lasso = ToolItem { tool: Tool::Lasso, color: 0, width: 0.0 };
    let picker = ToolPicker::with_items(vec![wide, lasso]).expect("two items");
    assert_eq!(picker.items()[0].width, 12.0);
    assert_eq!(picker.selected(), Tool::Ink(InkKind::Pencil));
}

#[test]
fn the_picker_visibility_can_be_set_and_read_back() {
    let mut picker = ToolPicker::new();
    picker.set_visible(true);
    assert!(picker.is_visible());
    picker.set_visible(false);
    assert!(!picker.is_visible());
}
