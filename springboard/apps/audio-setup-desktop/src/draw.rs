//! Paints the Audio Setup window and publishes its accessibility nodes. Geometry comes from `window.rs`, so what is
//! drawn is what a tap hits. Colours are the kit's semantic tokens, resolved with `now()`, so Light and Dark follow
//! the appearance. Text is cached per string in `TextCache`.

use crate::model::{format_buffer, format_channels, format_latency, format_rate, format_stream, format_volume, subtitle, DeviceInfo, Kind};
use crate::window::{AudioSetup, EditorLayout, KEY_PT, LINE_PT, MEMBER_BASE, N_CANCEL, N_KIND_AGG, N_KIND_MULTI, N_NAME, N_SAVE, SIDE_PT};
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CAPSULE_BG, CAPSULE_EDGE, CARD_BG, CONTENT_BG, DESTRUCTIVE, FIELD_BG, FILL, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT, SEGMENT_ON, SEPARATOR_C, SIDEBAR_BG, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;

/// Card corner radius (points): grouped cards are 13 pt, fields and segments are smaller.
const CARD_R: f32 = 13.0;
const FIELD_R: f32 = 6.0;

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

/// A text run whose right edge is at `right`, vertically centred on `mid`.
fn text_right(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, right: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let width = ui.measure(&fitted, pt, weight) / ui.pt(1.0);
    text_mid(labels, s, ui, &fitted, pt, weight, color, right - width, mid, max);
}

/// A text run centred in a rectangle.
fn text_center(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, color: u32, r: Rect) {
    let fitted = ui.fit(label, pt, ui.pt(r.w.max(0.0)));
    let img_w = ui.measure(&fitted, pt, Weight::Regular) / ui.pt(1.0);
    text_mid(labels, s, ui, &fitted, pt, Weight::Regular, color, r.x + (r.w - img_w) / 2.0, r.y + r.h / 2.0, r.w);
}

/// Paints the whole window.
pub fn paint(w: &mut AudioSetup, s: &mut Surface, ui: &Ui) {
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

    if w.editor.is_some() {
        paint_editor(w, s, ui);
    } else if let Some(d) = w.selected_device().cloned() {
        paint_detail(w, s, ui, &d);
    } else {
        paint_empty(w, s, ui);
    }

    if let Some(err) = w.error.clone() {
        let y = w.size.1 - 28.0;
        let (x, cw) = (w.content_x(), w.content_w());
        text_mid(&mut w.labels, s, ui, &err, MAC_CALLOUT_PT, Weight::Regular, DESTRUCTIVE, x, y, cw);
    }
}

/// The detail view for a device the user picked: name, the summary, and the card of numbers.
fn paint_detail(w: &mut AudioSetup, s: &mut Surface, ui: &Ui, d: &DeviceInfo) {
    let x = w.content_x();
    let cw = w.content_w();
    let y0 = UNIFIED_BAR_PT + crate::window::PAD_PT;
    text_mid(&mut w.labels, s, ui, &d.name, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), x, y0 + 16.0, cw);
    let sub = if d.alive { subtitle(d) } else { format!("{} · unplugged", subtitle(d)) };
    text_mid(&mut w.labels, s, ui, &sub, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);

    let rate = d.rate;
    let range = if d.buffer_range.1 > 0 { format!(" (range {} to {})", d.buffer_range.0, d.buffer_range.1) } else { String::new() };
    let rows: Vec<(&str, String)> = vec![
        ("Format", d.output_format.as_ref().map_or_else(|| "Unknown".to_string(), format_stream)),
        ("Sample rate", format_rate(rate)),
        ("Channels", format_channels(d.input_channels, d.output_channels)),
        ("Buffer size", format!("{}{}", format_buffer(d.buffer_frames, rate), range)),
        ("Latency", format_latency(d.latency_frames, rate)),
        ("Safety offset", format_latency(d.safety_frames, rate)),
        ("Volume", d.volume.map_or_else(|| "Not available".to_string(), format_volume)),
        ("Identifier", d.uid.clone()),
    ];
    let top = y0 + 82.0;
    let card = Rect { x, y: top, w: cw, h: rows.len() as f32 * LINE_PT + 12.0 };
    fill(s, ui, card, CARD_R, CARD_BG.now());
    for (i, (key, value)) in rows.iter().enumerate() {
        let mid = card.y + 6.0 + i as f32 * LINE_PT + LINE_PT / 2.0;
        text_mid(&mut w.labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + 16.0, mid, KEY_PT);
        text_mid(&mut w.labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + KEY_PT + 24.0, mid, cw - KEY_PT - 40.0);
        if i + 1 < rows.len() {
            rule(s, ui, x + 16.0, card.y + 6.0 + (i + 1) as f32 * LINE_PT, cw - 32.0, 0.0);
        }
    }
    if d.kind == Kind::MultiOutput {
        let note = "Every output gets the same mix. The members share one rate.";
        text_mid(&mut w.labels, s, ui, note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_3.now(), x + 16.0, card.y + card.h + 18.0, cw - 32.0);
    }
}

/// The start screen: no device chosen, or no server to ask.
fn paint_empty(w: &mut AudioSetup, s: &mut Surface, ui: &Ui) {
    let (x, cw) = (w.content_x(), w.content_w());
    let y0 = UNIFIED_BAR_PT + crate::window::PAD_PT;
    let (title, body) = if w.error.is_some() {
        ("Audio Setup", "The audio server is not running, so there are no devices to show.")
    } else if w.devices.is_empty() {
        ("Audio Setup", "No audio devices are connected.")
    } else {
        ("Audio Setup", "Select a device to see its format, sample rate, buffer size, latency and volume.")
    };
    text_mid(&mut w.labels, s, ui, title, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), x, y0 + 16.0, cw);
    text_mid(&mut w.labels, s, ui, body, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y0 + 50.0, cw);
}

/// The aggregate and multi-output editor: name, kind, the members as a card, the clock, and the footer.
fn paint_editor(w: &mut AudioSetup, s: &mut Surface, ui: &Ui) {
    let Some(editor) = w.editor.clone() else { return };
    let geo: EditorLayout = w.editor_layout();
    let x = w.content_x();
    let cw = w.content_w();
    let y0 = UNIFIED_BAR_PT + crate::window::PAD_PT;
    let heading = match (editor.draft.kind, editor.draft.uid.is_some()) {
        (Kind::MultiOutput, false) => "New Multi-Output",
        (Kind::MultiOutput, true) => "Edit Multi-Output",
        (_, false) => "New Aggregate",
        (_, true) => "Edit Aggregate",
    };
    text_mid(&mut w.labels, s, ui, heading, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), x, y0 + 16.0, cw);

    // Name
    text_mid(&mut w.labels, s, ui, "Name", MAC_BODY_PT, Weight::Regular, LABEL.now(), x, geo.name.y + geo.name.h / 2.0, KEY_PT);
    fill(s, ui, geo.name, FIELD_R, FIELD_BG.now());
    let caret = if editor.draft.name.is_empty() { "|".to_string() } else { format!("{}|", editor.draft.name) };
    text_mid(&mut w.labels, s, ui, &caret, MAC_BODY_PT, Weight::Regular, LABEL.now(), geo.name.x + 8.0, geo.name.y + geo.name.h / 2.0, geo.name.w - 16.0);

    // Kind: two halves in a capsule
    text_mid(&mut w.labels, s, ui, "Kind", MAC_BODY_PT, Weight::Regular, LABEL.now(), x, geo.kind.y + geo.kind.h / 2.0, KEY_PT);
    fill(s, ui, geo.kind, 7.0, CAPSULE_BG.now());
    let half = geo.kind.w / 2.0;
    let agg_on = editor.draft.kind == Kind::Aggregate;
    if agg_on {
        fill(s, ui, Rect { x: geo.kind.x + 2.0, y: geo.kind.y + 2.0, w: half - 4.0, h: geo.kind.h - 4.0 }, 5.0, SEGMENT_ON.now());
    } else {
        fill(s, ui, Rect { x: geo.kind.x + half + 2.0, y: geo.kind.y + 2.0, w: half - 4.0, h: geo.kind.h - 4.0 }, 5.0, SEGMENT_ON.now());
    }
    let agg_rect = Rect { x: geo.kind.x, y: geo.kind.y, w: half, h: geo.kind.h };
    let multi_rect = Rect { x: geo.kind.x + half, y: geo.kind.y, w: half, h: geo.kind.h };
    text_center(&mut w.labels, s, ui, "Aggregate", MAC_BODY_PT, LABEL.now(), agg_rect);
    text_center(&mut w.labels, s, ui, "Multi-Output", MAC_BODY_PT, LABEL.now(), multi_rect);

    let hint = if agg_on {
        "One device from several. The clock source keeps time, and the others are resampled to it."
    } else {
        "One mix sent to every output. All members must have outputs and run at one rate."
    };
    text_mid(&mut w.labels, s, ui, hint, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, geo.kind.y + geo.kind.h + 14.0, cw);

    // Members
    let editable: Vec<DeviceInfo> = w.editable().into_iter().cloned().collect();
    let card_top = geo.members_top - 6.0;
    let card_h = (geo.member_rows.len() as f32) * crate::window::MEMBER_PT + 12.0;
    if editable.is_empty() {
        text_mid(&mut w.labels, s, ui, "No devices are connected.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + 16.0, geo.members_top + 12.0, cw - 32.0);
    }
    fill(s, ui, Rect { x, y: card_top, w: cw, h: card_h }, CARD_R, CARD_BG.now());
    for (i, row) in geo.member_rows.iter().enumerate() {
        let Some(d) = editable.get(i) else { break };
        let member = editor.draft.is_member(&d.uid);
        let box_r = Rect { x: row.x + 16.0, y: row.y + (row.h - 16.0) / 2.0, w: 16.0, h: 16.0 };
        if member {
            fill(s, ui, box_r, 4.0, LINK);
        } else {
            fill(s, ui, box_r, 4.0, FIELD_BG.now());
            s.stroke_rrect(ui.pt(box_r.x), ui.pt(box_r.y), ui.pt(box_r.w), ui.pt(box_r.h), ui.pt(4.0), ui.pt(1.0).max(1.0), CAPSULE_EDGE.now());
        }
        text_mid(&mut w.labels, s, ui, &d.name, MAC_BODY_PT, Weight::Regular, LABEL.now(), row.x + 44.0, row.y + 15.0, row.w - 150.0);
        text_mid(&mut w.labels, s, ui, &subtitle(d), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), row.x + 44.0, row.y + 31.0, row.w - 150.0);
        if member {
            let clock = editor.draft.clock() == Some(d.uid.as_str());
            let (label, color) = if clock { ("Clock", LINK) } else { ("Set as Clock", LABEL_2.now()) };
            text_right(&mut w.labels, s, ui, label, MAC_SUBHEAD_PT, Weight::Regular, color, row.x + row.w - 16.0, row.y + row.h / 2.0, 120.0);
        }
        if i + 1 < geo.member_rows.len() && i + 1 < editable.len() {
            rule(s, ui, row.x + 44.0, row.y + row.h, row.w - 60.0, 0.0);
        }
    }

    // Footer
    rule(s, ui, 0.0, geo.footer_top, w.size.0, 0.0);
    if let Some(msg) = &editor.message {
        let max = geo.cancel.x - x - 12.0;
        text_mid(&mut w.labels, s, ui, msg, MAC_CALLOUT_PT, Weight::Regular, DESTRUCTIVE, x, geo.save.y + geo.save.h / 2.0, max);
    }
    fill(s, ui, geo.save, 7.0, LINK);
    let save_label = if editor.draft.uid.is_some() { "Save" } else { "Create" };
    text_center(&mut w.labels, s, ui, save_label, MAC_BODY_PT, WHITE, geo.save);
    fill(s, ui, geo.cancel, 7.0, FILL);
    text_center(&mut w.labels, s, ui, "Cancel", MAC_BODY_PT, LABEL.now(), geo.cancel);
}

/// The accessibility nodes: the toolbar buttons, the sidebar rows and, while the editor is open, its controls.
pub fn controls(w: &AudioSetup, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let tb = w.toolbar();
    for (id, r) in w.toolbar_layout(ui).visible {
        let Some(item) = tb.items.iter().find(|i| i.id == id) else { continue };
        nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled });
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
    if let Some(editor) = &w.editor {
        let geo = w.editor_layout();
        nodes.push(Node { id: N_NAME, role: Role::TextField, label: "Name".into(), value: editor.draft.name.clone(), bounds: geo.name, enabled: true });
        let half = geo.kind.w / 2.0;
        let agg = Rect { x: geo.kind.x, y: geo.kind.y, w: half, h: geo.kind.h };
        let multi = Rect { x: geo.kind.x + half, y: geo.kind.y, w: half, h: geo.kind.h };
        nodes.push(Node { id: N_KIND_AGG, role: Role::Button, label: "Aggregate".into(), value: if editor.draft.kind == Kind::Aggregate { "on".into() } else { String::new() }, bounds: agg, enabled: true });
        nodes.push(Node { id: N_KIND_MULTI, role: Role::Button, label: "Multi-Output".into(), value: if editor.draft.kind == Kind::MultiOutput { "on".into() } else { String::new() }, bounds: multi, enabled: true });
        for (i, row) in geo.member_rows.iter().enumerate() {
            let Some(d) = w.editable().get(i).cloned() else { break };
            let on = editor.draft.is_member(&d.uid);
            nodes.push(Node { id: MEMBER_BASE + i as u64, role: Role::Checkbox, label: d.name.clone(), value: if on { "on".into() } else { "off".into() }, bounds: *row, enabled: true });
        }
        nodes.push(Node { id: N_SAVE, role: Role::Button, label: "Save".into(), value: String::new(), bounds: geo.save, enabled: true });
        nodes.push(Node { id: N_CANCEL, role: Role::Button, label: "Cancel".into(), value: String::new(), bounds: geo.cancel, enabled: true });
    }
    nodes
}
