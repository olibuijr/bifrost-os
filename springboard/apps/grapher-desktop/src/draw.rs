//! Paints the Grapher window: the bar, the equation sidebar with its field and status lines, and the plot. The geometry
//! comes from `window.rs`, so what is drawn is what a tap hits. Colours are the kit's tokens, resolved with `now()`.

use crate::plot;
use crate::window::{Grapher, SIDE_PT};
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::{CONTENT_BG, DESTRUCTIVE, LABEL_2, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, SEPARATOR_C, SIDEBAR_BG};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// The text under the field when the selected equation is valid.
const HINT: &str = "y = f(x)    x = X(t), y = Y(t)    r = R(t)";
/// The placeholder of an empty field.
const PLACEHOLDER: &str = "Type an equation";

/// Paints the whole window.
pub fn paint(g: &mut Grapher, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());

    let bar = g.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let toolbar = g.toolbar();
    widgets::toolbar(s, ui, bar, &toolbar, None);

    let side = g.sidebar_rect();
    fill(s, ui, side, SIDEBAR_BG.now());
    let list = g.sidebar_list();
    widgets::source_list(s, ui, g.list_rect(), &list, 0.0, None);
    rule(s, ui, SIDE_PT, side.y, 0.0, side.h);
    paint_panel(g, s, ui);

    let area = g.plot_area(ui);
    plot::draw(s, ui, &mut g.labels, area, &g.model);
}

/// The panel under the list: the field of the selected equation, a hint or its error, and the status line.
fn paint_panel(g: &mut Grapher, s: &mut Surface, ui: &Ui) {
    let field = g.field_rect();
    let typed = g.model.selected_row().map(|r| r.text.clone()).unwrap_or_default();
    let mut buffer = TextField::new();
    buffer.push_str(&typed);
    widgets::text_field(s, ui, &buffer, field, PLACEHOLDER, g.model.selected.is_some());

    let problem = g
        .model
        .selected_row()
        .filter(|r| !r.text.trim().is_empty())
        .and_then(|r| r.equation().err());
    let line_y = field.y + field.h + 10.0;
    match problem {
        Some(error) => text_at(&mut g.labels, s, ui, &error, MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE, field.x, line_y, field.w),
        None => text_at(&mut g.labels, s, ui, HINT, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), field.x, line_y, field.w),
    }
    if !g.message.is_empty() {
        let message = g.message.clone();
        text_at(&mut g.labels, s, ui, &message, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), field.x, line_y + 20.0, field.w);
    }
}

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, colour: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), 0.0, colour);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

/// A line of text with its top left at `(x, y)` points, cut to `max` points.
#[allow(clippy::too_many_arguments)]
fn text_at(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, colour: u32, x: f32, y: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, colour);
    s.blit(img, ui.pt(x) as i32, ui.pt(y) as i32, 255, 255);
}
