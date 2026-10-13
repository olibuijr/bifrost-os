//! Paints the Color Meter window and publishes its accessibility nodes. Colours are the kit's semantic tokens, resolved
//! with `now()`, so Light and Dark follow the appearance. The magnifier paints one rectangle per display pixel it shows
//! and outlines the aperture; the aperture slider sits under the swatch.

use crate::color::Rgb8;
use crate::loupe::{aperture_cells, cell_of};
use crate::source::PixelRect;
use crate::window::{ColorMeter, CTL_APERTURE, LINE_PT, PAD_PT};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CALLOUT_PT, SEPARATOR_C, TINT};
use bifrost_app::kit::{pack, Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// Card corner radius (points), as the grouped cards in the other system apps.
const CARD_R: f32 = 13.0;
/// The width of the readout's key column (points).
const KEY_PT: f32 = 120.0;
/// The thickness of the magnifier's centre marker (points).
const MARK_PT: f32 = 2.0;
/// The aperture slider's track thickness and knob diameter (points).
const SLIDER_TRACK_PT: f32 = 4.0;
const SLIDER_KNOB_PT: f32 = 16.0;

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), ui.pt(radius), color);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

fn rgba(c: Rgb8) -> u32 {
    pack(c.r, c.g, c.b, 255)
}

/// A text run whose vertical centre is `mid`, cut to `max` points, starting at `x`.
fn text_mid(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// Paints the whole window.
pub fn paint(w: &mut ColorMeter, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let bar = w.bar_rect();
    widgets::toolbar_band(s, ui, bar);
    let tb = w.toolbar();
    widgets::toolbar(s, ui, bar, &tb, None);
    paint_loupe(w, s, ui);
    paint_panel(w, s, ui);
}

/// The magnifier: one square per display pixel around the pointer, with the readout's pixel marked.
fn paint_loupe(w: &mut ColorMeter, s: &mut Surface, ui: &Ui) {
    let l = w.loupe_rect();
    fill(s, ui, l, CARD_R, CARD_BG.now());
    match w.loupe_pixels() {
        Some((rect, px)) => {
            paint_cells(s, ui, l, rect, &px);
            // the aperture, when it covers more than the one pixel the marker shows
            if w.aperture > 1 {
                if let Some(cells) = w.aperture_rect().and_then(|a| aperture_cells(rect, a)) {
                    frame_cells(s, ui, l, rect, cells, TINT);
                }
            }
            if let Some((cx, cy)) = cell_of(rect, w.pointer.x, w.pointer.y) {
                frame_cells(s, ui, l, rect, PixelRect { x: cx, y: cy, w: 1, h: 1 }, LABEL.now());
            }
        }
        None => {
            let mid = l.y + l.h / 2.0;
            text_mid(&mut w.labels, s, ui, "Screen capture is not available", MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), l.x + 12.0, mid, l.w - 24.0);
        }
    }
}

fn paint_cells(s: &mut Surface, ui: &Ui, l: Rect, rect: PixelRect, px: &[Rgb8]) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let cw = l.w / rect.w as f32;
    let ch = l.h / rect.h as f32;
    for row in 0..rect.h {
        for col in 0..rect.w {
            let c = px[(row * rect.w + col) as usize];
            let cell = Rect { x: l.x + col as f32 * cw, y: l.y + row as f32 * ch, w: cw, h: ch };
            fill(s, ui, cell, 0.0, rgba(c));
        }
    }
}

/// Four thin edges around `cells`, a block of cells of the magnifier (cell `(0, 0)` is the top left pixel of `rect`).
fn frame_cells(s: &mut Surface, ui: &Ui, l: Rect, rect: PixelRect, cells: PixelRect, color: u32) {
    let cw = l.w / rect.w as f32;
    let ch = l.h / rect.h as f32;
    let x = l.x + cells.x as f32 * cw;
    let y = l.y + cells.y as f32 * ch;
    let (w, h) = (cells.w as f32 * cw, cells.h as f32 * ch);
    let t = MARK_PT.min(w / 2.0).min(h / 2.0);
    fill(s, ui, Rect { x, y, w, h: t }, 0.0, color);
    fill(s, ui, Rect { x, y: y + h - t, w, h: t }, 0.0, color);
    fill(s, ui, Rect { x, y, w: t, h }, 0.0, color);
    fill(s, ui, Rect { x: x + w - t, y, w: t, h }, 0.0, color);
}

/// The swatch and the readout rows beneath it.
fn paint_panel(w: &mut ColorMeter, s: &mut Surface, ui: &Ui) {
    let p = w.panel_rect();
    let sw = w.swatch_rect();
    let fill_color = match w.reading() {
        Some(r) => rgba(r.codes),
        None => CARD_BG.now(),
    };
    fill(s, ui, sw, CARD_R, fill_color);
    paint_aperture(w, s, ui);

    let top = w.rows_top();
    let bottom = p.y + p.h;
    let rows = w.rows();
    let card_h = (rows.len() as f32 * LINE_PT + 12.0).min((bottom - top).max(0.0));
    fill(s, ui, Rect { x: p.x, y: top, w: p.w, h: card_h }, CARD_R, CARD_BG.now());
    for (i, (key, value)) in rows.iter().enumerate() {
        let row_top = top + 6.0 + i as f32 * LINE_PT;
        let row_bottom = row_top + LINE_PT;
        if row_bottom > top + card_h {
            break;
        }
        let mid = row_top + LINE_PT / 2.0;
        text_mid(&mut w.labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL.now(), p.x + 16.0, mid, KEY_PT - 16.0);
        text_mid(&mut w.labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), p.x + KEY_PT + 8.0, mid, p.w - KEY_PT - 32.0);
        if i + 1 < rows.len() {
            rule(s, ui, p.x + 16.0, row_bottom, p.w - 32.0, 0.0);
        }
    }
}

/// The aperture row: its label and size, and the slider.
fn paint_aperture(w: &mut ColorMeter, s: &mut Surface, ui: &Ui) {
    let row = w.aperture_row();
    fill(s, ui, row, CARD_R, CARD_BG.now());
    let label_mid = row.y + 16.0;
    text_mid(&mut w.labels, s, ui, "Aperture", MAC_BODY_PT, Weight::Regular, LABEL.now(), row.x + 16.0, label_mid, KEY_PT - 16.0);
    let size = format!("{0} x {0} pixels", w.aperture);
    text_mid(&mut w.labels, s, ui, &size, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), row.x + KEY_PT + 8.0, label_mid, row.w - KEY_PT - 32.0);

    let t = w.track_rect();
    let mid = t.y + t.h / 2.0;
    let track = Rect { x: t.x, y: mid - SLIDER_TRACK_PT / 2.0, w: t.w, h: SLIDER_TRACK_PT };
    fill(s, ui, track, SLIDER_TRACK_PT / 2.0, LABEL_3.now());
    let filled = t.w * w.aperture_fraction();
    if filled > 0.0 {
        fill(s, ui, Rect { w: filled.max(SLIDER_TRACK_PT), ..track }, SLIDER_TRACK_PT / 2.0, TINT);
    }
    let knob = Rect { x: t.x + filled - SLIDER_KNOB_PT / 2.0, y: mid - SLIDER_KNOB_PT / 2.0, w: SLIDER_KNOB_PT, h: SLIDER_KNOB_PT };
    fill(s, ui, knob, SLIDER_KNOB_PT / 2.0, SEPARATOR_C.now());
    let inner = Rect { x: knob.x + 1.0, y: knob.y + 1.0, w: knob.w - 2.0, h: knob.h - 2.0 };
    fill(s, ui, inner, inner.w / 2.0, CARD_BG.now());
}

/// The accessibility nodes: the toolbar buttons, the aperture slider and the readout's swatch.
pub fn controls(w: &ColorMeter, ui: &Ui) -> Vec<Node> {
    let mut nodes = Vec::new();
    let bar = w.toolbar();
    for (id, r) in w.toolbar_layout(ui).visible {
        let label = bar.items.iter().find(|i| i.id == id).map(|i| i.label.clone()).unwrap_or_default();
        let enabled = bar.items.iter().any(|i| i.id == id && i.enabled);
        nodes.push(Node { id: id as u64, role: Role::Button, label, value: String::new(), bounds: r, enabled });
    }
    nodes.push(Node {
        id: CTL_APERTURE as u64,
        role: Role::Slider,
        label: "Aperture size".into(),
        value: format!("{} pixels", w.aperture),
        bounds: w.track_rect(),
        enabled: true,
    });
    nodes
}
