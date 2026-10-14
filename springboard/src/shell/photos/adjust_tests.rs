//! The Adjust pane's Auto button: it sits in the row under the sliders and toggles Auto Enhance.

use super::tests::photos;
use super::*;

#[test]
fn the_adjust_pane_has_an_auto_button_under_its_sliders() {
    let (mut p, _dir, ui) = photos("adjust-auto", 1);
    p.open_viewer(0);
    p.activate(&ui, Target::Edit, 0.0);
    let top = p.edit_panel_top(&ui);
    let row = ui.pt(EDIT_ROW_PT);
    let y = top + row * (3.0 + EDIT_SLIDER_ROWS as f32) + row / 2.0;
    let x = p.w as f32 / 2.0;
    assert_eq!(p.edit_hit(&ui, x, y), Some(Target::EditTool(Tool::AutoEnhance)));
    p.activate(&ui, Target::EditTool(Tool::AutoEnhance), 0.0);
    assert!(p.editor.as_ref().is_some_and(|e| e.edit.auto), "the button turns Auto Enhance on");
}
