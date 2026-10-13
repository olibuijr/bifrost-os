//! Paints the System Information window and publishes its accessibility nodes. Colours are the kit's semantic tokens,
//! resolved with `now()`, so Light and Dark follow the appearance. Text is cached per string in `TextCache`.

use crate::window::{SysInfo, B_REFRESH, KEY_PT, LINE_PT, PAD_PT, SIDE_PT};
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_LARGE_TITLE_PT, SEPARATOR_C, SIDEBAR_BG};
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

    let side = w.sidebar_rect();
    fill(s, ui, side, 0.0, SIDEBAR_BG.now());
    let list = w.sidebar();
    widgets::source_list(s, ui, side, &list, 0.0, None);
    rule(s, ui, SIDE_PT, side.y, 0.0, side.h);

    paint_detail(w, s, ui);
}

/// The title, its one line, and the card of rows for the selected category. Rows that do not fit whole are left out
/// until the card is scrolled to them.
fn paint_detail(w: &mut SysInfo, s: &mut Surface, ui: &Ui) {
    let x = w.content_x();
    let cw = w.content_w();
    let y0 = UNIFIED_BAR_PT + PAD_PT;
    let title = w.selected.title();
    let blurb = w.selected.blurb();
    text_mid(&mut w.labels, s, ui, title, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), x, y0 + 16.0, cw);
    text_mid(&mut w.labels, s, ui, blurb, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);

    let top = w.card_top();
    let avail = (w.size.1 - top - PAD_PT).max(0.0);
    let rows = w.rows();
    fill(s, ui, Rect { x, y: top, w: cw, h: avail }, CARD_R, CARD_BG.now());
    for (i, (key, value)) in rows.iter().enumerate() {
        let row_top = top + 6.0 + i as f32 * LINE_PT - w.scroll;
        let row_bottom = row_top + LINE_PT;
        if row_top < top || row_bottom > top + avail {
            continue;
        }
        let mid = row_top + LINE_PT / 2.0;
        text_mid(&mut w.labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + 16.0, mid, KEY_PT - 16.0);
        text_mid(&mut w.labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + KEY_PT + 8.0, mid, cw - KEY_PT - 40.0);
        if i + 1 < rows.len() {
            rule(s, ui, x + 16.0, row_bottom, cw - 32.0, 0.0);
        }
    }
}

/// The accessibility nodes: the toolbar button, the sidebar rows and the detail card.
pub fn controls(w: &SysInfo, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    for (id, r) in w.toolbar_layout(ui).visible {
        if id != B_REFRESH {
            continue;
        }
        nodes.push(Node { id: id as u64, role: Role::Button, label: "Refresh".into(), value: String::new(), bounds: r, enabled: true });
    }
    let list = w.sidebar();
    let side = w.sidebar_rect();
    for (index, item) in list.items.iter().enumerate() {
        if let SidebarItem::Row { id, label, enabled, .. } = item {
            let bounds = list.item_rect(index, side, 0.0);
            let selected = list.selected() == Some(*id);
            nodes.push(Node { id: *id as u64, role: Role::Row, label: label.clone(), value: if selected { "selected".into() } else { String::new() }, bounds, enabled: *enabled });
        }
    }
    nodes
}
