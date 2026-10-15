//! Paints the System Information window and publishes its accessibility nodes. Colours are the kit's semantic tokens,
//! resolved with `now()`, so Light and Dark follow the appearance. Text is cached per string in `TextCache`.

use crate::model::Row;
use crate::window::{list_rows, Column, SysInfo, B_CANCEL, B_REFRESH, B_SEARCH, B_SORT_DETAILS, B_SORT_NAME, KEY_PT, LINE_PT, LIST_HEAD_PT, PAD_PT, SECTION_BASE, SIDE_PT};
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_TITLE1_PT, SEGMENT_ON, SEPARATOR_C, SIDEBAR_BG};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Card corner radius (points), as the grouped cards in the other system apps.
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

/// Paints the whole window.
pub fn paint(w: &mut SysInfo, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = w.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = w.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);
    if let Some(r) = w.search_rect(ui) {
        match &w.find {
            Some(field) => widgets::search_field(s, ui, field, r, "Search", true),
            None => widgets::search_field(s, ui, &TextField::new(), r, "Search", false),
        }
    }

    let side = w.sidebar_rect();
    fill(s, ui, side, 0.0, SIDEBAR_BG.now());
    let list = w.sidebar();
    widgets::source_list(s, ui, side, &list, w.side_scroll, None);
    rule(s, ui, SIDE_PT, side.y, 0.0, side.h);

    paint_detail(w, s, ui);
    if w.is_loading() {
        paint_loading(w, s, ui);
    }
}

/// The title, its one line, then the item list (for a category that has one) and the card of facts. Rows that do not
/// fit whole are left out until the pane is scrolled to them.
fn paint_detail(w: &mut SysInfo, s: &mut Surface, ui: &Ui) {
    let x = w.content_x();
    let cw = w.content_w();
    let y0 = UNIFIED_BAR_PT + PAD_PT;
    let title = w.selected.title();
    text_mid(&mut w.labels, s, ui, title, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x, y0 + 16.0, cw);
    let subtitle = w.subtitle();
    text_mid(&mut w.labels, s, ui, &subtitle, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);

    let view = w.view();
    let panes = w.panes(&view);
    if let Some(list) = panes.list {
        let lines: Vec<Row> = view.content.summary_rows();
        let picked: Vec<bool> = (0..lines.len()).map(|i| i == view.item).collect();
        paint_list_head(w, s, ui, list);
        paint_rows(&mut w.labels, s, ui, list_rows(list), &lines, w.list_scroll, &picked);
    }
    let rows = view.detail();
    let picked: Vec<bool> = (0..rows.len()).map(|i| w.is_selected(i)).collect();
    paint_rows(&mut w.labels, s, ui, panes.detail, rows, w.scroll, &picked);
}

/// The column header of an item list: "Name" over the key column and "Details" over the value column, in secondary
/// grey, the sorted column in the label colour with a rule under it in the link colour.
fn paint_list_head(w: &mut SysInfo, s: &mut Surface, ui: &Ui, list: Rect) {
    fill(s, ui, list, CARD_R, CARD_BG.now());
    let mid = list.y + LIST_HEAD_PT / 2.0;
    let sorted = w.sort.map(|(column, _)| column);
    let columns: [(Column, &str, f32, f32); 2] = [(Column::Name, "Name", list.x + 16.0, KEY_PT - 16.0), (Column::Details, "Details", list.x + KEY_PT + 8.0, list.w - KEY_PT - 40.0)];
    for (column, label, x, width) in columns {
        let color = if sorted == Some(column) { LABEL.now() } else { LABEL_2.now() };
        text_mid(&mut w.labels, s, ui, label, MAC_CALLOUT_PT, Weight::Semibold, color, x, mid, width);
    }
    if let Some(column) = sorted {
        let (x, width) = match column {
            Column::Name => (list.x + 16.0, KEY_PT - 32.0),
            Column::Details => (list.x + KEY_PT + 8.0, list.w - KEY_PT - 40.0),
        };
        fill(s, ui, Rect { x, y: list.y + LIST_HEAD_PT - 3.0, w: width.max(0.0), h: 2.0 }, 1.0, LINK);
    }
    rule(s, ui, list.x + 16.0, list.y + LIST_HEAD_PT - 1.0, list.w - 32.0, 0.0);
}

/// One card of key and value rows in `pane`, scrolled by `scroll`; `picked` rows sit on the selection plate.
fn paint_rows(labels: &mut TextCache, s: &mut Surface, ui: &Ui, pane: Rect, rows: &[Row], scroll: f32, picked: &[bool]) {
    fill(s, ui, pane, CARD_R, CARD_BG.now());
    let (x, cw) = (pane.x, pane.w);
    for (i, (key, value)) in rows.iter().enumerate() {
        let row_top = pane.y + 6.0 + i as f32 * LINE_PT - scroll;
        let row_bottom = row_top + LINE_PT;
        if row_top < pane.y || row_bottom > pane.y + pane.h {
            continue;
        }
        if picked.get(i).copied().unwrap_or(false) {
            fill(s, ui, Rect { x: x + 6.0, y: row_top + 1.0, w: cw - 12.0, h: LINE_PT - 2.0 }, 5.0, SEGMENT_ON.now());
        }
        let mid = row_top + LINE_PT / 2.0;
        text_mid(labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + 16.0, mid, KEY_PT - 16.0);
        text_mid(labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + KEY_PT + 8.0, mid, cw - KEY_PT - 40.0);
        if i + 1 < rows.len() {
            rule(s, ui, x + 16.0, row_bottom, cw - 32.0, 0.0);
        }
    }
}

/// The "Preparing system profile" sheet: a bar that runs back and forth until the facts land.
fn paint_loading(w: &mut SysInfo, s: &mut Surface, ui: &Ui) {
    let sheet = w.load_sheet();
    let content = widgets::sheet(s, ui, w.window_rect(), UNIFIED_BAR_PT, &sheet, 1.0, None);
    let track = Rect { x: content.x, y: content.y + 4.0, w: content.w, h: 6.0 };
    fill(s, ui, track, 3.0, SEPARATOR_C.now());
    let t = w.load.as_ref().map_or(0.0, |l| l.t);
    let phase = (t / 1400.0).fract() as f32;
    let width = track.w * 0.3;
    let left = (track.x + (track.w + width) * phase - width).max(track.x);
    let right = (track.x + (track.w + width) * phase).min(track.x + track.w);
    if right > left {
        fill(s, ui, Rect { x: left, y: track.y, w: right - left, h: track.h }, 3.0, LINK);
    }
    text_mid(&mut w.labels, s, ui, "Reading the hardware and software of this computer.", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), content.x, track.y + 26.0, content.w);
}

/// The accessibility nodes: the toolbar button and search field, the sidebar sections and rows, and, while the machine
/// is read, the loading sheet's progress and Cancel button.
pub fn controls(w: &SysInfo, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    if w.is_loading() {
        let sheet = w.load_sheet();
        let layout = sheet.layout(w.window_rect(), UNIFIED_BAR_PT, 1.0);
        nodes.push(Node { id: SECTION_BASE - 1, role: Role::ProgressIndicator, label: "Preparing system profile".into(), value: String::new(), bounds: layout.panel, enabled: true });
        for (id, bounds) in layout.buttons {
            if id == B_CANCEL {
                nodes.push(Node { id: id as u64, role: Role::Button, label: "Cancel".into(), value: String::new(), bounds, enabled: true });
            }
        }
        return nodes;
    }
    for (id, r) in w.toolbar_layout(ui).visible {
        if id == B_REFRESH {
            nodes.push(Node { id: id as u64, role: Role::Button, label: "Refresh".into(), value: String::new(), bounds: r, enabled: true });
        } else if id == B_SEARCH {
            nodes.push(Node { id: id as u64, role: Role::TextField, label: "Search".into(), value: w.query().to_string(), bounds: r, enabled: true });
        }
    }
    let view = w.view();
    if let Some(head) = w.panes(&view).list {
        let sorted = w.sort;
        let state = |column: Column| match sorted {
            Some((c, descending)) if c == column => {
                if descending {
                    "sorted descending"
                } else {
                    "sorted ascending"
                }
            }
            _ => "",
        };
        let columns = [(B_SORT_NAME, "Name", Column::Name, head.x, KEY_PT), (B_SORT_DETAILS, "Details", Column::Details, head.x + KEY_PT, head.w - KEY_PT)];
        for (id, label, column, x, width) in columns {
            let bounds = Rect { x, y: head.y, w: width.max(0.0), h: LIST_HEAD_PT };
            nodes.push(Node { id: id as u64, role: Role::Button, label: label.into(), value: state(column).into(), bounds, enabled: w.can_sort() });
        }
    }
    let list = w.sidebar();
    let side = w.sidebar_rect();
    for (index, item) in list.items.iter().enumerate() {
        let bounds = list.item_rect(index, side, w.side_scroll);
        if bounds.h <= 0.0 {
            continue; // a row inside a folded section
        }
        match item {
            SidebarItem::Row { id, label, enabled, .. } => {
                let selected = list.selected() == Some(*id);
                nodes.push(Node { id: *id as u64, role: Role::Row, label: label.clone(), value: if selected { "selected".into() } else { String::new() }, bounds, enabled: *enabled });
            }
            SidebarItem::Section { id, label, collapsed } => {
                nodes.push(Node { id: *id as u64, role: Role::Button, label: label.clone(), value: if *collapsed { "collapsed".into() } else { "expanded".into() }, bounds, enabled: true });
            }
            SidebarItem::Header(_) => {}
        }
    }
    nodes
}
