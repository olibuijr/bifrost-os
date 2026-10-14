//! Parity sdk-rest-02: headless tests for markup ink gestures and undo. No test draws a window or surface.

use bifrost_ui::markup_ink::{InkCanvas, InkTool, HISTORY_LIMIT};

fn canvas() -> InkCanvas {
    InkCanvas::new(100.0, 100.0).unwrap()
}

fn draw(canvas: &mut InkCanvas, points: &[(f32, f32)]) {
    let (x, y) = points[0];
    canvas.pointer_down(x, y);
    for &(x, y) in &points[1..] {
        canvas.pointer_move(x, y);
    }
    assert!(canvas.pointer_up());
}

#[test]
fn a_pen_gesture_commits_clamped_points_and_keeps_the_tool_and_colour_from_pointer_down() {
    let mut c = canvas();
    c.set_color(0xff12_3456);
    c.pointer_down(-5.0, 10.0);
    c.set_tool(InkTool::Marker);
    c.set_color(0xdead_beef);
    c.pointer_move(120.0, 20.0);
    assert!(!c.can_undo(), "an uncommitted gesture cannot be undone");
    assert!(c.pointer_up());
    assert_eq!(c.strokes()[0].tool, InkTool::Pen);
    assert_eq!(c.strokes()[0].color, 0x12_3456);
    assert_eq!(c.strokes()[0].points, [(0.0, 10.0), (100.0, 20.0)]);
    assert!(c.can_undo());
}

#[test]
fn an_eraser_removes_a_touched_stroke_and_undo_restores_it() {
    let mut c = canvas();
    draw(&mut c, &[(10.0, 10.0), (90.0, 10.0)]);
    c.set_tool(InkTool::Eraser);
    c.pointer_down(50.0, 10.0);
    assert!(c.active().is_some());
    assert!(c.pointer_up());
    assert!(c.strokes().is_empty());
    assert!(c.undo());
    assert_eq!(c.strokes().len(), 1);
}

#[test]
fn an_eraser_gesture_that_touches_nothing_does_not_create_an_undo_step() {
    let mut c = canvas();
    draw(&mut c, &[(10.0, 10.0)]);
    c.set_tool(InkTool::Eraser);
    c.pointer_down(90.0, 90.0);
    assert!(!c.pointer_up());
    assert!(c.undo(), "the preceding pen stroke is still the latest undo step");
    assert!(!c.can_undo());
}

#[test]
fn a_tool_change_does_not_change_the_gesture_already_in_progress() {
    let mut c = canvas();
    draw(&mut c, &[(10.0, 10.0)]);
    c.set_tool(InkTool::Eraser);
    c.pointer_down(90.0, 90.0);
    c.set_tool(InkTool::Pen);
    c.pointer_move(10.0, 10.0);
    assert!(c.pointer_up());
    assert!(c.strokes().is_empty(), "the active eraser continues to erase");

    draw(&mut c, &[(20.0, 20.0)]);
    c.set_tool(InkTool::Pen);
    c.pointer_down(90.0, 20.0);
    c.set_tool(InkTool::Eraser);
    c.pointer_move(20.0, 20.0);
    assert!(c.pointer_up());
    assert_eq!(c.strokes().len(), 2, "the active pen is not turned into an eraser");
    assert_eq!(c.strokes()[1].tool, InkTool::Pen);
}

#[test]
fn a_no_op_eraser_does_not_evict_undo_history_at_the_limit() {
    let mut c = canvas();
    for i in 0..HISTORY_LIMIT {
        let x = (i % 90) as f32 + 1.0;
        let y = (i / 90) as f32 + 1.0;
        draw(&mut c, &[(x, y)]);
    }
    c.set_tool(InkTool::Eraser);
    c.pointer_down(100.0, 100.0);
    assert!(!c.pointer_up());
    for _ in 0..HISTORY_LIMIT {
        assert!(c.undo());
    }
    assert!(c.strokes().is_empty());
    assert!(!c.can_undo());
}

#[test]
fn invalid_sizes_and_non_finite_pointer_events_do_not_change_the_canvas() {
    assert!(InkCanvas::new(0.0, 10.0).is_err());
    assert!(InkCanvas::new(f32::INFINITY, 10.0).is_err());
    let mut c = canvas();
    c.pointer_down(f32::NAN, 1.0);
    c.pointer_move(1.0, f32::INFINITY);
    assert!(!c.pointer_up());
    assert!(c.strokes().is_empty());
    assert!(!c.can_undo());
}
