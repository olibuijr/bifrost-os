//! Paints the Print Center window: the toolbar, the printer list, the queue of the selected printer and the footer. The
//! rectangles are `PrintCenter`'s geometry, which the taps share, so a tap lands on what is drawn. Colours are the kit's
//! semantic tokens, resolved with `now()`, so Light and Dark follow the appearance.

use crate::model::{job_state_name, printer_detail, printer_summary, size_text};
use crate::window::{PrintCenter, COLS_PT, JOB_ROW_PT, PAD_PT, ROW_PT, SIDE_PT};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SELECTION, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// Corner radius of a selection plate (points), as the sidebar selections.
const SEL_R: f32 = 9.0;
/// The column headings, in order.
const HEADINGS: [&str; 5] = ["Job", "Document", "Owner", "Size", "Status"];

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

/// The width of queue column `i` in points: up to the next column (or `right`), less a gutter.
fn column_width(cols: &[f32; 5], i: usize, right: f32) -> f32 {
    let end = cols.get(i + 1).copied().unwrap_or(right);
    (end - cols[i] - 8.0).max(0.0)
}

/// Paints the whole window.
pub fn paint(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = p.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = p.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);
    paint_printers(p, s, ui);
    paint_queue(p, s, ui);
    paint_footer(p, s, ui);
}

/// The printer list: one row per printer with its name and a summary line; the selected one is marked.
fn paint_printers(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    let side = p.side_rect();
    fill(s, ui, side, 0.0, CARD_BG.now());
    rule(s, ui, side.x + side.w, side.y, 0.0, side.h);
    let printers = p.model.printers.clone();
    let selected = p.model.selected.clone();
    if printers.is_empty() {
        text_mid(&mut p.labels, s, ui, "No printers", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), 16.0, side.y + 24.0, SIDE_PT - 32.0);
        return;
    }
    for (i, printer) in printers.iter().enumerate() {
        let r = p.printer_row(i);
        let on = selected.as_deref() == Some(printer.name.as_str());
        if on {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        let (name_color, sub_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        text_mid(&mut p.labels, s, ui, &printer.name, MAC_BODY_PT, Weight::Regular, name_color, r.x + 18.0, r.y + ROW_PT * 0.36, r.w - 36.0);
        text_mid(&mut p.labels, s, ui, &printer_summary(printer), MAC_FOOTNOTE_PT, Weight::Regular, sub_color, r.x + 18.0, r.y + ROW_PT * 0.74, r.w - 36.0);
    }
}

/// The queue pane: the selected printer's name and detail, the column headings and its jobs.
fn paint_queue(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    let d = p.detail_rect();
    let x0 = d.x + PAD_PT;
    let w = (d.w - 2.0 * PAD_PT).max(0.0);
    let printer = p.model.selected_printer().cloned();
    let Some(printer) = printer else {
        text_mid(&mut p.labels, s, ui, "No printer is selected", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, d.y + 40.0, w);
        return;
    };
    text_mid(&mut p.labels, s, ui, &printer.name, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, d.y + 24.0, w);
    text_mid(&mut p.labels, s, ui, &printer_detail(&printer), MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, d.y + 50.0, w);

    let cols = p.columns();
    let head_mid = p.queue_top() + COLS_PT / 2.0;
    for (i, (x, label)) in cols.iter().zip(HEADINGS).enumerate() {
        text_mid(&mut p.labels, s, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), *x, head_mid, column_width(&cols, i, x0 + w));
    }
    rule(s, ui, x0, p.job_row_top(), w, 0.0);

    let jobs = p.model.jobs.clone();
    if jobs.is_empty() {
        let top = p.job_row_top() + 20.0;
        text_mid(&mut p.labels, s, ui, "No jobs in the queue", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0 + 8.0, top, w);
        return;
    }
    let selected_job = p.model.selected_job;
    for (i, job) in jobs.iter().enumerate() {
        let r = p.job_row(i);
        let on = selected_job == Some(job.id);
        if on {
            fill(s, ui, r, SEL_R, SELECTION.now());
        }
        let (name_color, state_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let cells = [job.id.to_string(), job.name.clone(), job.user.clone(), size_text(job.size_kb), job_state_name(job.state).to_string()];
        let mid = r.y + JOB_ROW_PT / 2.0;
        for (k, text) in cells.iter().enumerate() {
            let color = if k == 4 { state_color } else { name_color };
            text_mid(&mut p.labels, s, ui, text, MAC_BODY_PT, Weight::Regular, color, cols[k], mid, column_width(&cols, k, x0 + w));
        }
    }
}

/// The footer: the last result, or the number of printers.
fn paint_footer(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    let f = p.footer_rect();
    rule(s, ui, 0.0, f.y, f.w, 0.0);
    let text = p.footer_text();
    text_mid(&mut p.labels, s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 12.0, f.y + f.h / 2.0, f.w - 24.0);
}

/// The accessibility nodes: the toolbar buttons.
pub fn controls(p: &PrintCenter, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let bar = p.toolbar();
    for (id, r) in p.toolbar_layout(ui).visible {
        let label = bar.items.iter().find(|i| i.id == id).map(|i| i.label.clone()).unwrap_or_default();
        let enabled = bar.items.iter().any(|i| i.id == id && i.enabled);
        nodes.push(Node { id: id as u64, role: Role::Button, label, value: String::new(), bounds: r, enabled });
    }
    nodes
}
