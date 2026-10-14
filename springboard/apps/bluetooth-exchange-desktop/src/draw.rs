//! Paints the Bluetooth File Exchange window: the toolbar, the devices on the left, and on the right the files to send,
//! the transfers with their progress, and the files received, then the footer. The rectangles are `BluetoothExchange`'s
//! geometry, which the taps share, so a tap lands on what is drawn. Colours are the kit's semantic tokens, resolved with
//! `now()`.

use crate::model::{self, Phase, Transfer};
use crate::window::{BluetoothExchange, DEV_ROW_PT, FOOT_PT, HEAD_PT, LABEL_PT, PAD_PT, ROW_PT, SEND_ROWS};
use bifrost_app::bluetooth_service::RadioState;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SELECTION, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// Corner radius of a selection plate (points), as the sidebar selections.
const SEL_R: f32 = 9.0;
/// The thickness of a transfer's progress bar (points).
const BAR_PT: f32 = 3.0;

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

/// Paints the whole window.
pub fn paint(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = p.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = p.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);
    paint_devices(p, s, ui);
    paint_detail(p, s, ui);
    paint_footer(p, s, ui);
}

/// The device list: one row per device with its name and whether it is paired; the selected one is marked.
fn paint_devices(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui) {
    let side = p.side_rect();
    fill(s, ui, side, 0.0, CARD_BG.now());
    rule(s, ui, side.x + side.w, side.y, 0.0, side.h);
    let devices: Vec<(bifrost_core::bt_address::BdAddr, String, bool)> = p.bt.devices.all().iter().map(|d| (d.address, d.label(), d.paired)).collect();
    for (i, (address, name, paired)) in devices.iter().enumerate() {
        let r = p.device_row(i);
        let on = p.selected == Some(*address);
        if on {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        let (name_color, sub_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let kind = if *paired { "Paired" } else { "Nearby" };
        text_mid(&mut p.labels, s, ui, name, MAC_BODY_PT, Weight::Regular, name_color, r.x + 18.0, r.y + DEV_ROW_PT * 0.36, r.w - 36.0);
        text_mid(&mut p.labels, s, ui, kind, MAC_FOOTNOTE_PT, Weight::Regular, sub_color, r.x + 18.0, r.y + DEV_ROW_PT * 0.74, r.w - 36.0);
    }
    if devices.is_empty() {
        let (line, hint) = match p.bt.state {
            RadioState::On => ("No devices", "Press Scan to look for nearby devices"),
            RadioState::Off => ("Bluetooth is off", "Turn it on in Settings"),
            RadioState::Unavailable => ("No Bluetooth", "This computer has no adapter"),
        };
        text_mid(&mut p.labels, s, ui, line, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), 16.0, side.y + 24.0, side.w - 32.0);
        text_mid(&mut p.labels, s, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 16.0, side.y + 48.0, side.w - 32.0);
    }
}

/// The detail pane: the selected device, the files to send, the transfers, and the files received.
fn paint_detail(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui) {
    let d = p.detail_rect();
    let x0 = d.x + 16.0;
    let w = (d.w - 32.0).max(0.0);
    let head = p.selected_name().unwrap_or_else(|| "No device selected".to_string());
    text_mid(&mut p.labels, s, ui, &head, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, d.y + HEAD_PT / 2.0, w);
    paint_send(p, s, ui, x0, w);
    paint_transfers(p, s, ui, x0, w);
    paint_received(p, s, ui, x0, w);
}

/// A section label above a list, in footnote grey.
fn section(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui, x0: f32, w: f32, top: f32, text: &str) {
    text_mid(&mut p.labels, s, ui, text, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now(), x0, top + LABEL_PT / 2.0, w);
}

/// The files to send, from Downloads: name on the left, size on the right.
fn paint_send(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui, x0: f32, w: f32) {
    let top = p.rows_top();
    let count = p.model.outbox.len();
    let title = if count > SEND_ROWS { format!("Files to send ({count}, first {SEND_ROWS} shown)") } else { format!("Files to send ({count})") };
    section(p, s, ui, x0, w, top, &title);
    if count == 0 {
        let mid = p.send_row(0).y + ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, "Nothing in Downloads to send", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, mid, w);
        return;
    }
    let files = p.model.outbox.clone();
    for (i, file) in files.iter().take(SEND_ROWS).enumerate() {
        let r = p.send_row(i);
        let mid = r.y + ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, &file.name, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + PAD_PT, mid, r.w - 130.0);
        text_mid(&mut p.labels, s, ui, &model::size_text(file.size), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), r.x + r.w - 100.0, mid, 90.0);
    }
}

/// The transfers, newest last: name, the status, and a progress bar along the bottom of the row.
fn paint_transfers(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui, x0: f32, w: f32) {
    let top = p.transfers_top();
    section(p, s, ui, x0, w, top, "Transfers");
    let start = p.transfer_window_start();
    let shown: Vec<Transfer> = p.model.transfers.iter().skip(start).cloned().collect();
    if shown.is_empty() {
        let mid = p.transfer_row(0).y + ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, "No transfers yet", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, mid, w);
        return;
    }
    for (k, t) in shown.iter().enumerate() {
        let r = p.transfer_row(k);
        let mid = r.y + (ROW_PT - BAR_PT) / 2.0;
        let status_color = match t.phase {
            Phase::Failed(_) => LABEL.now(),
            _ => LABEL_2.now(),
        };
        text_mid(&mut p.labels, s, ui, &t.file.name, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + PAD_PT, mid, r.w - 230.0);
        text_mid(&mut p.labels, s, ui, &t.status_text(), MAC_FOOTNOTE_PT, Weight::Regular, status_color, r.x + r.w - 220.0, mid, 210.0);
        paint_bar(s, ui, t, r);
    }
}

/// A transfer's progress along the bottom of its row: a grey track with the share sent in the selection colour.
fn paint_bar(s: &mut Surface, ui: &Ui, t: &Transfer, r: Rect) {
    let track = Rect { x: r.x + PAD_PT, y: r.y + ROW_PT - BAR_PT - 1.0, w: (r.w - 2.0 * PAD_PT).max(0.0), h: BAR_PT };
    fill(s, ui, track, BAR_PT / 2.0, SEPARATOR_C.now());
    let share = t.fraction();
    if share > 0.0 {
        fill(s, ui, Rect { w: track.w * share, ..track }, BAR_PT / 2.0, SELECTION.now());
    }
}

/// The files received: name and size, newest first, as many as fit above the footer.
fn paint_received(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui, x0: f32, w: f32) {
    let top = p.received_top();
    let count = p.model.inbox.len();
    section(p, s, ui, x0, w, top, &format!("Received ({count})"));
    if count == 0 {
        let mid = p.received_row(0).y + ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, "No files received yet. They arrive in Downloads/Bluetooth.", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, mid, w);
        return;
    }
    let files = p.model.inbox.clone();
    for (k, file) in files.iter().take(p.received_capacity()).enumerate() {
        let r = p.received_row(k);
        let mid = r.y + ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, &file.name, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x + PAD_PT, mid, r.w - 130.0);
        text_mid(&mut p.labels, s, ui, &model::size_text(file.size), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), r.x + r.w - 100.0, mid, 90.0);
    }
}

/// The footer: the last message, or what the selection needs.
fn paint_footer(p: &mut BluetoothExchange, s: &mut Surface, ui: &Ui) {
    let f: Rect = p.footer_rect();
    rule(s, ui, 0.0, f.y, f.w, 0.0);
    let text = p.footer_text();
    text_mid(&mut p.labels, s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 12.0, f.y + FOOT_PT / 2.0, f.w - 24.0);
}

/// The accessibility nodes: the toolbar buttons.
pub fn controls(p: &BluetoothExchange, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let bar = p.toolbar();
    for (id, r) in p.toolbar_layout(ui).visible {
        let label = bar.items.iter().find(|i| i.id == id).map(|i| i.label.clone()).unwrap_or_default();
        let enabled = bar.items.iter().any(|i| i.id == id && i.enabled);
        nodes.push(Node { id: id as u64, role: Role::Button, label, value: String::new(), bounds: r, enabled });
    }
    nodes
}
