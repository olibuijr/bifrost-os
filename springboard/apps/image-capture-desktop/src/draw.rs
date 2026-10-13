//! Paints the Image Capture window: the toolbar, the sources on the left, the pictures of a camera or the settings and
//! preview of a scanner on the right, and the footer. The rectangles are `ImageCapture`'s geometry, which the taps
//! share, so a tap lands on what is drawn. Colours are the kit's semantic tokens, resolved with `now()`.

use crate::window::{ImageCapture, FILE_ROW_PT, FOOT_PT, HEAD_PT, SET_ROW_PT, SRC_ROW_PT};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SELECTION, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Image, Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;
use bifrost_ui::device_browser::DeviceKind;
use bifrost_ui::scan_job::ScanSettings;

/// Corner radius of a selection plate (points), as the sidebar selections.
const SEL_R: f32 = 9.0;
/// Where the setting values start, from the left of the pane (points).
const VALUE_X_PT: f32 = 170.0;

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
pub fn paint(p: &mut ImageCapture, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = p.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = p.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);
    paint_sources(p, s, ui);
    paint_detail(p, s, ui);
    paint_footer(p, s, ui);
}

/// The sidebar: one row per camera, SD card or scanner, with its kind under its name; the selected one is marked.
fn paint_sources(p: &mut ImageCapture, s: &mut Surface, ui: &Ui) {
    let side = p.side_rect();
    fill(s, ui, side, 0.0, CARD_BG.now());
    rule(s, ui, side.x + side.w, side.y, 0.0, side.h);
    let sources = p.model.sources.clone();
    let selected = p.model.selected;
    for (i, source) in sources.iter().enumerate() {
        let r = p.source_row(i);
        let on = selected == Some(i);
        if on {
            fill(s, ui, Rect { x: r.x + 8.0, y: r.y + 2.0, w: r.w - 16.0, h: r.h - 4.0 }, SEL_R, SELECTION.now());
        }
        let (name_color, sub_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let kind = match source.kind {
            DeviceKind::Camera => "Camera or SD card",
            DeviceKind::Scanner => "Scanner",
        };
        text_mid(&mut p.labels, s, ui, &source.name, MAC_BODY_PT, Weight::Regular, name_color, r.x + 18.0, r.y + SRC_ROW_PT * 0.36, r.w - 36.0);
        text_mid(&mut p.labels, s, ui, kind, MAC_FOOTNOTE_PT, Weight::Regular, sub_color, r.x + 18.0, r.y + SRC_ROW_PT * 0.74, r.w - 36.0);
    }
    if sources.is_empty() {
        text_mid(&mut p.labels, s, ui, "No devices", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), 16.0, side.y + 24.0, side.w - 32.0);
    }
}

/// The detail pane: the selected source's name, then its pictures or its scanner settings and preview.
fn paint_detail(p: &mut ImageCapture, s: &mut Surface, ui: &Ui) {
    let d = p.detail_rect();
    let x0 = d.x + 16.0;
    let w = (d.w - 32.0).max(0.0);
    let Some(source) = p.model.selected_source().cloned() else {
        text_mid(&mut p.labels, s, ui, "Choose a camera, SD card or scanner", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, d.y + 40.0, w);
        return;
    };
    text_mid(&mut p.labels, s, ui, &source.name, MAC_HEADLINE_PT, Weight::Semibold, LABEL.now(), x0, d.y + HEAD_PT / 2.0, w);
    match source.kind {
        DeviceKind::Camera => paint_pictures(p, s, ui, x0, w),
        DeviceKind::Scanner => paint_scanner(p, s, ui, x0, w),
    }
}

/// The camera's pictures: one row each with its size; a picked row is marked.
fn paint_pictures(p: &mut ImageCapture, s: &mut Surface, ui: &Ui, x0: f32, w: f32) {
    let files = p.model.files.clone();
    if files.is_empty() {
        let top = p.rows_top() + 12.0;
        text_mid(&mut p.labels, s, ui, "No pictures on this camera", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x0, top, w);
        return;
    }
    for (i, file) in files.iter().enumerate() {
        let r = p.file_row(i);
        let on = p.model.picked.contains(&i);
        if on {
            fill(s, ui, r, SEL_R, SELECTION.now());
        }
        let (name_color, size_color) = if on { (WHITE, WHITE) } else { (LABEL.now(), LABEL_2.now()) };
        let mid = r.y + FILE_ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, &file.name, MAC_BODY_PT, Weight::Regular, name_color, r.x + 10.0, mid, r.w - 130.0);
        text_mid(&mut p.labels, s, ui, &size_text(file.size), MAC_FOOTNOTE_PT, Weight::Regular, size_color, r.x + r.w - 110.0, mid, 100.0);
    }
}

/// The scanner's three settings, then its preview.
fn paint_scanner(p: &mut ImageCapture, s: &mut Surface, ui: &Ui, x0: f32, w: f32) {
    let settings: ScanSettings = p.model.settings();
    let rows = [
        ("Resolution", format!("{} dpi", settings.resolution_dpi)),
        ("Colour", settings.color.label().to_string()),
        ("Format", settings.format.label().to_string()),
    ];
    for (k, (label, value)) in rows.iter().enumerate() {
        let r = p.setting_row(k);
        rule(s, ui, r.x, r.y, r.w, 0.0);
        let mid = r.y + SET_ROW_PT / 2.0;
        text_mid(&mut p.labels, s, ui, label, MAC_BODY_PT, Weight::Regular, LABEL.now(), x0, mid, VALUE_X_PT - 16.0);
        text_mid(&mut p.labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x0 + VALUE_X_PT, mid, w - VALUE_X_PT);
    }
    paint_preview(p, s, ui);
}

/// The preview, scaled down to fit its frame (never up), or a line saying there is none yet.
fn paint_preview(p: &mut ImageCapture, s: &mut Surface, ui: &Ui) {
    let r = p.preview_rect();
    let Some(src) = p.preview_image.as_ref() else {
        let mid = r.y + 16.0;
        text_mid(&mut p.labels, s, ui, "No preview yet. Press Preview to see the page.", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), r.x, mid, r.w);
        return;
    };
    let (bw, bh) = (ui.pt(r.w).max(1.0), ui.pt(r.h).max(1.0));
    let k = (bw / src.w as f32).min(bh / src.h as f32).min(1.0);
    let (w, h) = (((src.w as f32 * k) as usize).max(1), ((src.h as f32 * k) as usize).max(1));
    if k < 1.0 && p.preview_scaled.as_ref().map(|(size, _)| *size) != Some((w, h)) {
        p.preview_scaled = Some(((w, h), src.resized(w, h)));
    }
    let img: &Image = match &p.preview_scaled {
        Some((size, scaled)) if k < 1.0 && *size == (w, h) => scaled,
        _ => src,
    };
    let x = ui.pt(r.x) + (bw - img.w as f32) / 2.0;
    let y = ui.pt(r.y) + (bh - img.h as f32) / 2.0;
    s.blit(img, x as i32, y as i32, 255, 255);
}

/// The footer: the last result, or what the selection holds.
fn paint_footer(p: &mut ImageCapture, s: &mut Surface, ui: &Ui) {
    let f: Rect = p.footer_rect();
    rule(s, ui, 0.0, f.y, f.w, 0.0);
    let text = p.footer_text();
    text_mid(&mut p.labels, s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), 12.0, f.y + FOOT_PT / 2.0, f.w - 24.0);
}

/// A size in bytes as the people read it: KB, MB, or GB.
pub fn size_text(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        format!("{:.0} KB", kb.max(1.0))
    } else if kb < 1024.0 * 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{:.2} GB", kb / (1024.0 * 1024.0))
    }
}

/// The accessibility nodes: the toolbar buttons.
pub fn controls(p: &ImageCapture, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let bar = p.toolbar();
    for (id, r) in p.toolbar_layout(ui).visible {
        let label = bar.items.iter().find(|i| i.id == id).map(|i| i.label.clone()).unwrap_or_default();
        let enabled = bar.items.iter().any(|i| i.id == id && i.enabled);
        nodes.push(Node { id: id as u64, role: Role::Button, label, value: String::new(), bounds: r, enabled });
    }
    nodes
}
