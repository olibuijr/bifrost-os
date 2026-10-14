//! Paints the Disk Utility window and publishes its accessibility nodes. Geometry comes from `window.rs`, so what is
//! drawn is what a tap hits. Colours are the kit's semantic tokens, resolved with `now()`, so Light and Dark follow
//! the appearance. Text is cached per string in `TextCache`.

use crate::form::{Control as FormControl, Form, LIST_ROW_PT, NOTE_PT};
use crate::model::{self, info_rows, Kind, Node};
use crate::window::{DiskUtility, KEY_PT, LINE_PT, PAD_PT, SIDE_PT};
use bifrost_app::controls::sheet::SHEET_PAD_PT;
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node as Control, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, DESTRUCTIVE, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_TITLE2_PT, SEPARATOR_C, SIDEBAR_BG, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Card corner radius (points).
const CARD_R: f32 = 13.0;

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), ui.pt(radius), color);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

/// A text run whose vertical centre is `mid`, cut to `max` points.
fn text_mid(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// The line under the title: what kind of device, and its size.
fn subtitle(node: &Node) -> String {
    let size = model::format_size(node.size);
    match (node.kind, &node.parent) {
        (Kind::Partition, Some(parent)) => format!("Volume on {parent} \u{b7} {size}"),
        (Kind::Partition, None) => format!("Volume \u{b7} {size}"),
        (Kind::Disk, _) if node.removable => format!("External disk \u{b7} {size}"),
        (Kind::Disk, _) => format!("Internal disk \u{b7} {size}"),
    }
}

/// Paints the whole window.
pub fn paint(w: &mut DiskUtility, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = w.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = w.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);

    let side = w.sidebar_rect();
    fill(s, ui, side, 0.0, SIDEBAR_BG.now());
    let list = w.sidebar();
    widgets::source_list(s, ui, side, &list, 0.0, None);
    rule(s, ui, SIDE_PT, side.y, 0.0, side.h);

    match w.selected_node().cloned() {
        Some(node) => paint_detail(w, s, ui, &node),
        None => paint_empty(w, s, ui),
    }

    if let Some((text, is_error)) = w.footer() {
        let y = w.size.1 - 28.0;
        let (x, cw) = (w.content_x(), w.content_w());
        let color = if is_error { DESTRUCTIVE } else { LABEL_2.now() };
        text_mid(&mut w.labels, s, ui, &text, MAC_CALLOUT_PT, Weight::Regular, color, x, y, cw);
    }

    if let Some(pending) = &w.sheet {
        widgets::sheet(s, ui, w.window_rect(), UNIFIED_BAR_PT, &pending.sheet, 1.0, None);
    }
    let parent = w.window_rect();
    if let Some(open) = &w.form {
        paint_form(&mut w.labels, s, ui, &open.form, parent);
    }
}

/// A form sheet: the panel and its buttons, then each row's label and control, then the notes and the refusal.
fn paint_form(labels: &mut TextCache, s: &mut Surface, ui: &Ui, form: &Form, parent: Rect) {
    widgets::sheet(s, ui, parent, UNIFIED_BAR_PT, &form.sheet(), 1.0, None);
    let geo = form.geometry(parent, UNIFIED_BAR_PT);
    for (i, (row, g)) in form.rows.iter().zip(&geo.rows).enumerate() {
        text_mid(labels, s, ui, &row.label, MAC_BODY_PT, Weight::Regular, LABEL.now(), g.label.x, g.label.y + g.label.h / 2.0, g.label.w - 8.0);
        match &row.control {
            FormControl::Text { field, placeholder, .. } => widgets::text_field(s, ui, field, g.control, placeholder, form.focus == Some(i)),
            FormControl::Choice { options, selected } => {
                let names: Vec<&str> = options.iter().map(String::as_str).collect();
                widgets::segmented(s, ui, g.control, &names, *selected, true);
            }
            FormControl::List { rows, on, .. } => {
                for (k, text) in rows.iter().enumerate() {
                    let line = Rect { x: g.control.x, y: g.control.y + k as f32 * LIST_ROW_PT, w: g.control.w, h: LIST_ROW_PT };
                    let boxed = Rect { x: line.x, y: line.y + (LIST_ROW_PT - 14.0) / 2.0, w: 14.0, h: 14.0 };
                    if on.get(k).copied().unwrap_or(false) {
                        fill(s, ui, boxed, 3.0, LINK);
                        // compliance-allow: COL-03 WHITE is the checkmark on the LINK accent, not a panel fill.
                        fill(s, ui, Rect { x: boxed.x + 5.0, y: boxed.y + 5.0, w: 4.0, h: 4.0 }, 1.0, WHITE);
                    } else {
                        fill(s, ui, boxed, 3.0, SEPARATOR_C.now());
                        fill(s, ui, Rect { x: boxed.x + 1.5, y: boxed.y + 1.5, w: 11.0, h: 11.0 }, 2.0, CARD_BG.now());
                    }
                    text_mid(labels, s, ui, text, MAC_BODY_PT, Weight::Regular, LABEL.now(), line.x + 22.0, line.y + LIST_ROW_PT / 2.0, line.w - 22.0);
                }
            }
        }
    }
    let (x, width) = (geo.panel.x + SHEET_PAD_PT, geo.panel.w - SHEET_PAD_PT * 2.0);
    let mut y = geo.notes_y + NOTE_PT / 2.0;
    for note in &form.notes {
        text_mid(labels, s, ui, note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y, width);
        y += NOTE_PT;
    }
    if let Some(error) = &form.error {
        text_mid(labels, s, ui, error, MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE, x, y, width);
    }
}

/// The info pane for the selected row: its name, what it is, a capacity bar and the card of facts.
fn paint_detail(w: &mut DiskUtility, s: &mut Surface, ui: &Ui, node: &Node) {
    let (x, cw) = (w.content_x(), w.content_w());
    let y0 = UNIFIED_BAR_PT + PAD_PT;
    let title = model::display_name(node);
    text_mid(&mut w.labels, s, ui, &title, MAC_TITLE2_PT, Weight::Semibold, LABEL.now(), x, y0 + 16.0, cw);
    let sub = subtitle(node);
    text_mid(&mut w.labels, s, ui, &sub, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);

    // Capacity bar: the track, then the used part of a mounted volume.
    let bar = Rect { x, y: y0 + 74.0, w: cw, h: 8.0 };
    fill(s, ui, bar, 4.0, SEPARATOR_C.now());
    if let Some(usage) = node.usage {
        let used = Rect { x, y: bar.y, w: cw * model::used_fraction(usage), h: bar.h };
        fill(s, ui, used, 4.0, LINK);
    }

    let rows = info_rows(node);
    let card = Rect { x, y: y0 + 100.0, w: cw, h: rows.len() as f32 * LINE_PT + 12.0 };
    fill(s, ui, card, CARD_R, CARD_BG.now());
    for (i, (key, value)) in rows.iter().enumerate() {
        let mid = card.y + 6.0 + i as f32 * LINE_PT + LINE_PT / 2.0;
        text_mid(&mut w.labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + 16.0, mid, KEY_PT);
        text_mid(&mut w.labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + KEY_PT + 24.0, mid, cw - KEY_PT - 40.0);
        if i + 1 < rows.len() {
            rule(s, ui, x + 16.0, card.y + 6.0 + (i + 1) as f32 * LINE_PT, cw - 32.0, 0.0);
        }
    }
    let note = if node.kind == Kind::Disk {
        "Erase formats the whole disk in the format chosen in the Disk menu. First Aid checks a volume."
    } else {
        "Mount and unmount act on this volume only. Erase formats it in the format chosen in the Disk menu; First Aid checks it."
    };
    text_mid(&mut w.labels, s, ui, note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), x + 16.0, card.y + card.h + 18.0, cw - 32.0);
}

/// The start screen: nothing selected, or no disks found.
fn paint_empty(w: &mut DiskUtility, s: &mut Surface, ui: &Ui) {
    let (x, cw) = (w.content_x(), w.content_w());
    let y0 = UNIFIED_BAR_PT + PAD_PT;
    let body = if w.nodes.is_empty() { "No disks were found." } else { "Select a disk or volume to see its capacity, file system, mount point and SMART status." };
    let mut layout = bifrost_app::layout::Layout::from_points(w.size.0 / ui.pt(1.0), w.size.1 / ui.pt(1.0), 0.0, 0.0);
    layout.sidebar = SIDE_PT;
    layout.margin = PAD_PT;
    widgets::large_title(s, ui, &layout, "Disk Utility", y0);
    text_mid(&mut w.labels, s, ui, body, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);
}

/// The accessibility nodes: the toolbar buttons and the sidebar rows, with their enabled state and selection.
pub fn controls(w: &DiskUtility, ui: &Ui) -> Vec<Control> {
    let mut nodes = Vec::new();
    let tb = w.toolbar();
    for (id, r) in w.toolbar_layout(ui).visible {
        let Some(item) = tb.items.iter().find(|i| i.id == id) else { continue };
        nodes.push(Control { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled });
    }
    let list = w.sidebar();
    let side = w.sidebar_rect();
    for (index, item) in list.items.iter().enumerate() {
        if let SidebarItem::Row { id, label, enabled, .. } = item {
            let bounds = list.item_rect(index, side, 0.0);
            let selected = list.selected() == Some(*id);
            nodes.push(Control { id: *id as u64, role: Role::Row, label: label.clone(), value: if selected { "selected".into() } else { String::new() }, bounds, enabled: *enabled });
        }
    }
    nodes
}
