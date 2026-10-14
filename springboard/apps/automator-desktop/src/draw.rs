//! Paints Automator's window: the library of actions, the sequence of steps, the editor for the selected step with its
//! hint, the run's log, and the footer. The rectangles are `Automator`'s geometry, which the taps share, so a tap lands
//! on what is drawn. Colours are the kit's semantic tokens, resolved with `now()`.

use crate::window::{
    Automator, Target, FIELD_PT, FOOT_PT, HEAD_PT, HINT_LINES, HINT_LINE_PT, LIBRARY_ID, LOG_LINE_PT, PAD_PT, ROW_PT, STEP_ID,
};
use crate::model::KINDS;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{
    CARD_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SELECTION,
    SEPARATOR_C, SYSTEM_RED, WHITE,
};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Corner radius of a selection plate (points), as the sidebar selections.
const SEL_R: f32 = 9.0;
/// Where the field values start, from the left of the editor (points).
const VALUE_PT: f32 = 130.0;
/// Characters per line of the hint.
const HINT_WIDTH: usize = 72;

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), ui.pt(radius), color);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

/// A text run whose vertical centre is `mid`, cut to `max` points, starting at `x`.
#[allow(clippy::too_many_arguments)]
fn text_mid(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// A column's header: the title in semibold, centred in the header row of the window's body.
fn header(a: &mut Automator, s: &mut Surface, ui: &Ui, title: &str, x: f32, w: f32) {
    text_mid(&mut a.labels, s, ui, title, MAC_CALLOUT_PT, Weight::Semibold, LABEL_2.now(), x, UNIFIED_BAR_PT + HEAD_PT / 2.0, w);
}

/// Paints the whole window.
pub fn paint(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = a.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = a.toolbar();
    let pressed = match a.press {
        Target::Toolbar(id) => Some(id),
        _ => None,
    };
    widgets::toolbar(s, ui, bar, &tb, pressed);
    paint_library(a, s, ui);
    paint_sequence(a, s, ui);
    paint_detail(a, s, ui);
    paint_log(a, s, ui);
    paint_footer(a, s, ui);
}

/// The library: the actions, with their group under each name.
fn paint_library(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    let side = a.library_rect();
    fill(s, ui, side, 0.0, CARD_BG.now());
    rule(s, ui, side.x + side.w, side.y, 0.0, side.h);
    header(a, s, ui, "Library", side.x + 16.0, side.w - 32.0);
    for (i, kind) in KINDS.iter().enumerate() {
        let r = a.library_row(i);
        text_mid(&mut a.labels, s, ui, kind.title(), MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + 16.0, r.y + ROW_PT * 0.36, r.w - 32.0);
        text_mid(&mut a.labels, s, ui, kind.group(), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), r.x + 16.0, r.y + ROW_PT * 0.74, r.w - 32.0);
    }
}

/// The sequence: one numbered row per step, the selected one on a plate, each with its values.
fn paint_sequence(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    let col = a.sequence_rect();
    rule(s, ui, col.x + col.w, col.y, 0.0, col.h);
    header(a, s, ui, "Sequence", col.x + 16.0, col.w - 32.0);
    if a.workflow.steps.is_empty() {
        text_mid(&mut a.labels, s, ui, "Click an action in the library to add it.", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), col.x + 16.0, UNIFIED_BAR_PT + HEAD_PT + 20.0, col.w - 32.0);
        return;
    }
    let count = a.workflow.steps.len().min(a.visible_steps());
    let selected = a.selected;
    for i in 0..count {
        let step = a.workflow.steps[i].clone();
        let r = a.step_row(i);
        let on = selected == Some(i);
        if on {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        let (title_color, sub_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let title = format!("{}  {}", i + 1, step.kind.title());
        text_mid(&mut a.labels, s, ui, &title, MAC_BODY_PT, Weight::Regular, title_color, r.x + 18.0, r.y + ROW_PT * 0.36, r.w - 36.0);
        let summary = step.summary();
        let summary = if summary.is_empty() { step.kind.group().to_string() } else { summary };
        text_mid(&mut a.labels, s, ui, &summary, MAC_FOOTNOTE_PT, Weight::Regular, sub_color, r.x + 18.0, r.y + ROW_PT * 0.74, r.w - 36.0);
    }
}

/// The editor: the selected step's name, its fields (a focused one on a plate), and the hint for the action.
fn paint_detail(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    let d = a.detail_rect();
    rule(s, ui, d.x, d.y, 0.0, d.h);
    let x0 = d.x + 16.0;
    let w = (d.w - 32.0).max(0.0);
    let Some(step) = a.selected_step().cloned() else {
        text_mid(&mut a.labels, s, ui, "Select a step to edit its settings", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, UNIFIED_BAR_PT + HEAD_PT / 2.0, w);
        return;
    };
    text_mid(&mut a.labels, s, ui, step.kind.title(), MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, UNIFIED_BAR_PT + HEAD_PT / 2.0, w);
    let focus = a.focus;
    for (k, field) in step.kind.fields().iter().enumerate() {
        let r = a.field_row(k);
        let focused = focus == Some(k);
        if focused {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        let (label_color, value_color) = if focused { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let mid = r.y + FIELD_PT / 2.0;
        text_mid(&mut a.labels, s, ui, field.label, MAC_BODY_PT, Weight::Regular, label_color, x0, mid, VALUE_PT - 16.0);
        let value = step.value(k);
        let (shown, color) = if value.is_empty() && !focused {
            ("not set".to_string(), LABEL_3.now())
        } else if focused && field.choices.is_empty() {
            (format!("{value}|"), value_color)
        } else {
            (value.to_string(), value_color)
        };
        text_mid(&mut a.labels, s, ui, &shown, MAC_BODY_PT, Weight::Regular, color, x0 + VALUE_PT, mid, w - VALUE_PT);
        rule(s, ui, r.x, r.y + FIELD_PT, r.w, 0.0);
    }
    let top = a.hint_top();
    for (line, text) in wrap(step.kind.hint(), HINT_WIDTH).iter().take(HINT_LINES).enumerate() {
        let mid = top + (line as f32 + 0.5) * HINT_LINE_PT;
        text_mid(&mut a.labels, s, ui, text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x0, mid, w);
    }
}

/// The run's log: the last lines that fit, errors in red.
fn paint_log(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    let r = a.log_rect();
    let d = a.detail_rect();
    rule(s, ui, d.x, r.y - PAD_PT / 2.0, d.w, 0.0);
    let x0 = d.x + 16.0;
    let w = (d.w - 32.0).max(0.0);
    text_mid(&mut a.labels, s, ui, "Log", MAC_CALLOUT_PT, Weight::Semibold, LABEL_2.now(), x0, r.y + HEAD_PT / 2.0, w);
    if a.log.is_empty() {
        let hint = "Press Run to see what each step does.";
        text_mid(&mut a.labels, s, ui, hint, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, r.y + HEAD_PT + 10.0, w);
        return;
    }
    let visible = a.visible_log_lines();
    let start = a.log.len().saturating_sub(visible);
    for (j, line) in a.log[start..].iter().enumerate() {
        let mid = r.y + HEAD_PT + (j as f32 + 0.5) * LOG_LINE_PT;
        let color = if line.error { SYSTEM_RED } else { LABEL_2.now() };
        text_mid(&mut a.labels, s, ui, &line.text, MAC_FOOTNOTE_PT, Weight::Regular, color, x0, mid, w);
    }
}

fn paint_footer(a: &mut Automator, s: &mut Surface, ui: &Ui) {
    let f = a.footer_rect();
    rule(s, ui, 0.0, f.y, f.w, 0.0);
    let text = a.footer_text();
    text_mid(&mut a.labels, s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 12.0, f.y + FOOT_PT / 2.0, f.w - 24.0);
}

/// Word-wraps `text` to lines of at most `width` characters (a longer word keeps its own line).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The accessibility nodes: the toolbar buttons, the library actions, and the visible steps.
pub fn controls(a: &Automator, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let bar = a.toolbar();
    for (id, r) in a.toolbar_layout(ui).visible {
        let label = bar.items.iter().find(|i| i.id == id).map(|i| i.label.clone()).unwrap_or_default();
        let enabled = bar.items.iter().any(|i| i.id == id && i.enabled);
        nodes.push(Node { id: id as u64, role: Role::Button, label, value: String::new(), bounds: r, enabled });
    }
    for (i, kind) in KINDS.iter().enumerate() {
        let label = format!("Add {}", kind.title());
        nodes.push(Node { id: LIBRARY_ID + i as u64, role: Role::Button, label, value: String::new(), bounds: a.library_row(i), enabled: true });
    }
    let count = a.workflow.steps.len().min(a.visible_steps());
    for i in 0..count {
        let step = &a.workflow.steps[i];
        let label = format!("{}. {}", i + 1, step.kind.title());
        nodes.push(Node { id: STEP_ID + i as u64, role: Role::Button, label, value: step.summary(), bounds: a.step_row(i), enabled: true });
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MAX_FIELDS;

    #[test]
    fn wrap_breaks_at_the_width() {
        assert_eq!(wrap("a b c", 3), vec!["a b".to_string(), "c".to_string()]);
        assert_eq!(wrap("", 10), Vec::<String>::new());
        // a word longer than the width keeps its own line
        assert_eq!(wrap("tiny enormousword", 4), vec!["tiny".to_string(), "enormousword".to_string()]);
    }

    #[test]
    fn every_hint_fits_in_the_hint_block() {
        for kind in KINDS {
            assert!(wrap(kind.hint(), HINT_WIDTH).len() <= HINT_LINES, "{:?}", kind);
        }
    }

    #[test]
    fn the_editor_has_room_for_the_most_fields() {
        assert!(KINDS.iter().all(|k| k.fields().len() <= MAX_FIELDS));
    }
}
