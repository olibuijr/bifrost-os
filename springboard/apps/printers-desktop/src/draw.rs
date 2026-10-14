//! Paints the Print Center window: the toolbar, the printer list, the queue of the selected printer and the footer. The
//! rectangles are `PrintCenter`'s geometry, which the taps share, so a tap lands on what is drawn. Colours are the kit's
//! semantic tokens, resolved with `now()`, so Light and Dark follow the appearance.

use crate::discover::{detail_line, is_added};
use crate::model::{
    created_text, document_tag, info_lines, job_progress, option_lines, pages_text, printer_detail, problem_line, row_summary, share_label, size_text, supply_rgb, supply_text, wrap_words, Level, Pane, HELP,
};
use crate::window::{Field, PrintCenter, COLS_PT, JOB_ROW_PT, PAD_PT, ROW_PT, SIDE_PT};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SELECTION, SEPARATOR_C, SYSTEM_ORANGE, SYSTEM_RED, TINT, WHITE};
use bifrost_app::kit::{pack, Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// Corner radius of a selection plate (points), as the sidebar selections.
const SEL_R: f32 = 9.0;
/// The document tile left of a job's name: its width and height (points) and the gap before the name.
const TILE_W: f32 = 32.0;
const TILE_H: f32 = 22.0;
const TILE_GAP: f32 = 8.0;
/// The average width of a help character (points), used to wrap the topics. A little wide, so a line never overflows.
const HELP_CHAR_PT: f32 = 7.0;
/// The column headings, in order.
const HEADINGS: [&str; 7] = ["Job", "Document", "Owner", "Pages", "Size", "Created", "Status"];

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
fn column_width(cols: &[f32; 7], i: usize, right: f32) -> f32 {
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
        let offline = p.model.is_offline(&printer.name);
        if on {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        // an unreachable printer is dimmed, as the reference app greys it out
        let (name_color, sub_color) = if on {
            (WHITE, WHITE)
        } else if offline {
            (LABEL_2.now(), LABEL_2.now())
        } else {
            (LABEL.now(), LABEL_2.now())
        };
        text_mid(&mut p.labels, s, ui, &printer.name, MAC_BODY_PT, Weight::Regular, name_color, r.x + 18.0, r.y + ROW_PT * 0.36, r.w - 36.0);
        let mid = r.y + ROW_PT * 0.74;
        match problem_line(printer) {
            // a problem replaces the summary: a warning mark, then the sentence
            Some((level, text)) => {
                let mark = if on { WHITE } else if level == Level::Error { SYSTEM_RED } else { SYSTEM_ORANGE };
                warning_mark(&mut p.labels, s, ui, r.x + 18.0, mid, mark, on);
                text_mid(&mut p.labels, s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, mark, r.x + 34.0, mid, r.w - 52.0);
            }
            None => {
                text_mid(&mut p.labels, s, ui, &row_summary(printer, offline), MAC_FOOTNOTE_PT, Weight::Regular, sub_color, r.x + 18.0, mid, r.w - 36.0);
            }
        }
    }
}

/// The warning glyph: a round plate with an exclamation mark, centred on `mid`.
fn warning_mark(labels: &mut TextCache, s: &mut Surface, ui: &Ui, x: f32, mid: f32, color: u32, on_plate: bool) {
    let size = 12.0;
    fill(s, ui, Rect { x, y: mid - size / 2.0, w: size, h: size }, size / 2.0, color);
    let ink = if on_plate { SELECTION.now() } else { WHITE };
    let img = labels.get(ui, "!", MAC_FOOTNOTE_PT, Weight::Semibold, ink);
    s.blit(img, (ui.pt(x + size / 2.0) - img.w as f32 / 2.0) as i32, (ui.pt(mid) - img.h as f32 / 2.0) as i32, 255, 255);
}

/// The queue pane: the selected printer's name and detail, the column headings and its jobs.
fn paint_queue(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    match p.model.pane() {
        Pane::Discover => return paint_discover(p, s, ui),
        Pane::Help => return paint_help(p, s, ui),
        Pane::Queue | Pane::Info => {}
    }
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

    if p.model.show_info {
        paint_info(p, s, ui, &printer);
        return;
    }
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
    for (i, job) in jobs.iter().enumerate() {
        let r = p.job_row(i);
        let on = p.model.selected_jobs.contains(&job.id);
        if on {
            fill(s, ui, r, SEL_R, SELECTION.now());
        }
        let (name_color, state_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let cells = [
            job.id.to_string(),
            job.name.clone(),
            job.user.clone(),
            pages_text(job),
            size_text(job.size_kb),
            created_text(job.created, p.tz_offset, p.now_secs),
            p.model.job_status(job),
        ];
        let mid = r.y + JOB_ROW_PT / 2.0;
        let tile = Rect { x: cols[1], y: mid - TILE_H / 2.0, w: TILE_W, h: TILE_H };
        document_tile(&mut p.labels, s, ui, tile, &document_tag(&job.name), on);
        for (k, text) in cells.iter().enumerate() {
            let color = if k >= 3 { state_color } else { name_color };
            // the document name starts after its tile
            let (x, max) = if k == 1 { (cols[k] + TILE_W + TILE_GAP, column_width(&cols, k, x0 + w) - TILE_W - TILE_GAP) } else { (cols[k], column_width(&cols, k, x0 + w)) };
            text_mid(&mut p.labels, s, ui, text, MAC_BODY_PT, Weight::Regular, color, x, mid, max);
        }
        // a printing job shows how far it is under its status
        if let Some(done) = job_progress(job) {
            let width = column_width(&cols, 6, x0 + w).min(110.0);
            let bar = Rect { x: cols[6], y: r.y + JOB_ROW_PT - 7.0, w: width, h: 3.0 };
            let (track, fill_color) = if on { (pack(255, 255, 255, 90), WHITE) } else { (SEPARATOR_C.now(), TINT) };
            fill(s, ui, bar, 0.0, track);
            fill(s, ui, Rect { w: bar.w * done, ..bar }, 0.0, fill_color);
        }
    }
}

/// A small page-shaped tile with the document's type in capitals, drawn in place of a first-page preview.
fn document_tile(labels: &mut TextCache, s: &mut Surface, ui: &Ui, r: Rect, tag: &str, on_plate: bool) {
    let (plate, ink) = if on_plate { (pack(255, 255, 255, 70), WHITE) } else { (SEPARATOR_C.now(), LABEL_2.now()) };
    fill(s, ui, r, 3.0, plate);
    let img = labels.get(ui, tag, MAC_FOOTNOTE_PT, Weight::Semibold, ink);
    s.blit(img, (ui.pt(r.x + r.w / 2.0) - img.w as f32 / 2.0) as i32, (ui.pt(r.y + r.h / 2.0) - img.h as f32 / 2.0) as i32, 255, 255);
}

/// The Add Printer pane: the printers the network announced, one row each, and the button that adds the address on the
/// clipboard for a printer that is not listed.
fn paint_discover(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    let d = p.detail_rect();
    let x0 = d.x + PAD_PT;
    let w = (d.w - 2.0 * PAD_PT).max(0.0);
    text_mid(&mut p.labels, s, ui, "Add a Printer", MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, d.y + 24.0, w);
    text_mid(&mut p.labels, s, ui, "Printers on this network that announce themselves", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, d.y + 50.0, w);
    let button = p.clipboard_rect();
    fill(s, ui, button, 6.0, TINT);
    text_mid(&mut p.labels, s, ui, "Use Address from Clipboard", MAC_BODY_PT, Weight::Semibold, WHITE, button.x + 14.0, button.y + button.h / 2.0, button.w - 28.0);
    let found = p.discover.found.clone();
    if found.is_empty() {
        let message = if p.discover.failed() { "Printers on the network cannot be listed here. Copy an address and use the button above." } else { "Looking for printers…" };
        let top = p.found_row(0).y;
        text_mid(&mut p.labels, s, ui, message, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, top + 16.0, w);
        return;
    }
    let bottom = p.footer_rect().y;
    for (i, printer) in found.iter().enumerate() {
        let r = p.found_row(i);
        if r.y + r.h > bottom {
            break;
        }
        let added = is_added(printer, &p.model.printers);
        text_mid(&mut p.labels, s, ui, &printer.name, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + 12.0, r.y + r.h * 0.34, r.w - 110.0);
        text_mid(&mut p.labels, s, ui, &detail_line(printer), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), r.x + 12.0, r.y + r.h * 0.72, r.w - 24.0);
        if added {
            text_mid(&mut p.labels, s, ui, "Added", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), r.x + r.w - 70.0, r.y + r.h * 0.34, 58.0);
        }
        rule(s, ui, r.x + 12.0, r.y + r.h, r.w - 24.0, 0.0);
    }
}

/// The help pane: each topic's heading and its text, wrapped to the pane. Lines that would pass the footer are left off.
fn paint_help(p: &mut PrintCenter, s: &mut Surface, ui: &Ui) {
    let d = p.detail_rect();
    let x0 = d.x + PAD_PT;
    let w = (d.w - 2.0 * PAD_PT).max(0.0);
    text_mid(&mut p.labels, s, ui, "Print Center Help", MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, d.y + 24.0, w);
    let bottom = p.footer_rect().y - 8.0;
    let max_chars = ((w / HELP_CHAR_PT) as usize).max(20);
    let mut y = d.y + 52.0;
    for (heading, body) in HELP {
        if y + 20.0 > bottom {
            return;
        }
        text_mid(&mut p.labels, s, ui, heading, MAC_CALLOUT_PT, Weight::Semibold, LABEL.now(), x0, y + 8.0, w);
        y += 20.0;
        for line in wrap_words(body, max_chars) {
            if y + 16.0 > bottom {
                return;
            }
            text_mid(&mut p.labels, s, ui, &line, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, y + 8.0, w);
            y += 16.0;
        }
        y += 8.0;
    }
}

/// The info pane: the editable description and location, what the scheduler reports and the supply levels.
fn paint_info(p: &mut PrintCenter, s: &mut Surface, ui: &Ui, printer: &bifrost_app::controls::cups::Printer) {
    let d = p.detail_rect();
    let x0 = d.x + PAD_PT;
    let w = (d.w - 2.0 * PAD_PT).max(0.0);
    if p.fields_shown() {
        for (field, label) in [(Field::Description, "Description"), (Field::Location, "Location")] {
            let r = p.field_rect(field);
            text_mid(&mut p.labels, s, ui, label, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x0, r.y + r.h / 2.0, 100.0);
            widgets::text_field(s, ui, if field == Field::Description { &p.description } else { &p.location }, r, label, p.focus == Some(field));
        }
        let save = p.save_rect();
        fill(s, ui, save, 6.0, TINT);
        text_mid(&mut p.labels, s, ui, "Save", MAC_BODY_PT, Weight::Semibold, WHITE, save.x + 26.0, save.y + save.h / 2.0, save.w - 30.0);
        if p.share_shown() {
            let share = p.share_rect();
            fill(s, ui, share, 6.0, SEPARATOR_C.now());
            let label = share_label(printer.shared);
            text_mid(&mut p.labels, s, ui, label, MAC_BODY_PT, Weight::Regular, LABEL.now(), share.x + 14.0, share.y + share.h / 2.0, share.w - 28.0);
        }
    }
    let mut y = p.info_lines_top();
    let offline = p.model.is_offline(&printer.name);
    let mut lines = info_lines(printer, offline);
    if let Some(options) = p.model.options_for(&printer.name) {
        lines.extend(option_lines(options));
    }
    for (label, value) in lines {
        text_mid(&mut p.labels, s, ui, &label, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x0, y + 11.0, 100.0);
        text_mid(&mut p.labels, s, ui, &value, MAC_BODY_PT, Weight::Regular, LABEL.now(), x0 + 110.0, y + 11.0, w - 110.0);
        y += 22.0;
    }
    if printer.supplies.is_empty() {
        return;
    }
    y += 10.0;
    text_mid(&mut p.labels, s, ui, "Supplies", MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, y + 11.0, w);
    y += 30.0;
    for supply in &printer.supplies {
        text_mid(&mut p.labels, s, ui, &supply_text(supply), MAC_BODY_PT, Weight::Regular, LABEL.now(), x0, y + 9.0, 200.0);
        let bar = Rect { x: x0 + 220.0, y: y + 5.0, w: (w - 220.0).clamp(0.0, 220.0), h: 8.0 };
        fill(s, ui, bar, 3.0, SEPARATOR_C.now());
        if let Some(level) = supply.level {
            let color = supply_rgb(&supply.color).map_or(TINT, |(r, g, b)| pack(r, g, b, 255));
            fill(s, ui, Rect { w: bar.w * f32::from(level) / 100.0, ..bar }, 3.0, color);
        }
        y += 22.0;
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
