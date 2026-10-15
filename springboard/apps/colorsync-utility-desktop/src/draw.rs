//! Paints Colour Utility: the list of profile files, the title and the panel tabs, and the body of the panel shown.
//! Colours are the kit's semantic pairs, resolved with `now()`, so Light and Dark follow the appearance. Text is cached
//! per string in `TextCache`. The gamut is drawn as dots on the CIE 1931 xy chart.

use crate::model::{self, Severity, SPECTRAL_LOCUS, SRGB_AREA};
use crate::window::{ColourUtility, Current, Panel, KEY_PT, LINE_PT, SIDE_PT};
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::system::{CONTROL_ACCENT, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT, MAC_SUBHEAD_PT, SEGMENT_ON, SEPARATOR_C};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;

/// Corner radius of plates and cards, in points. The desktop's flat corners stop at 6.
const CARD_R: f32 = 6.0;
/// Padding inside the body card.
const BODY_PAD_PT: f32 = 12.0;
/// Largest side of the gamut chart, in points.
const PLOT_MAX_PT: f32 = 420.0;
/// The chart spans x from 0 to this and y from 0 to the next value (CIE 1931 xy).
const PLOT_X_MAX: f64 = 0.8;
const PLOT_Y_MAX: f64 = 0.9;
/// Width of the tag and verification labels, and of the readout's key column, in points.
const LABEL_COL_PT: f32 = 80.0;
const READOUT_KEY_PT: f32 = 80.0;

/// The text area of the body: `x` and `w` bound the lines, `top` is where the first line starts (already moved by the
/// scroll), and only the part from `clip` to `bottom` is on screen.
#[derive(Clone, Copy)]
struct Area {
    x: f32,
    w: f32,
    top: f32,
    clip: f32,
    bottom: f32,
}

impl Area {
    fn visible(&self, y: f32) -> bool {
        y + LINE_PT > self.clip && y < self.bottom
    }

    fn mid(&self, line: usize) -> f32 {
        self.top + line as f32 * LINE_PT + LINE_PT / 2.0
    }
}

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), ui.pt(radius), color);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

/// A text run whose vertical centre is `mid`, cut with an ellipsis to `max` points.
fn text(labels: &mut TextCache, s: &mut Surface, ui: &Ui, t: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    if t.is_empty() || max <= 0.0 {
        return;
    }
    let fitted = ui.fit(t, pt, ui.pt(max));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// A dot of radius `r` points centred on (x, y) in points.
fn dot(s: &mut Surface, ui: &Ui, x: f32, y: f32, r: f32, color: u32) {
    s.fill_rrect(ui.pt(x - r), ui.pt(y - r), ui.pt(2.0 * r), ui.pt(2.0 * r), ui.pt(r), color);
}

/// A line drawn as dots about 1.2 points apart.
fn segment(s: &mut Surface, ui: &Ui, a: (f32, f32), b: (f32, f32), color: u32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let steps = ((dx * dx + dy * dy).sqrt() / 1.2).ceil().max(1.0) as usize;
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        dot(s, ui, a.0 + dx * t, a.1 + dy * t, 0.6, color);
    }
}

/// Paints the whole window.
pub fn paint(w: &ColourUtility, labels: &mut TextCache, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let side = w.sidebar_rect();
    widgets::source_list(s, ui, side, &w.sidebar(), w.side_scroll, None);
    rule(s, ui, SIDE_PT, side.y, 0.0, side.h);
    paint_detail(w, labels, s, ui);
}

/// The title, the file path, the panel tabs and the body of the panel shown.
fn paint_detail(w: &ColourUtility, labels: &mut TextCache, s: &mut Surface, ui: &Ui) {
    let x = w.content_x();
    let cw = w.content_w();
    let y = w.title_y();
    let Some(c) = &w.current else {
        text(labels, s, ui, "Colour Utility", MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x, y + 18.0, cw);
        let hint = if w.found.is_empty() {
            w.status.as_str()
        } else {
            "Choose a profile in the list to see its header, tags, checks and gamut."
        };
        text(labels, s, ui, hint, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y + 40.0, cw);
        return;
    };
    text(labels, s, ui, &c.title, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), x, y + 18.0, cw);
    let path = c.path.display().to_string();
    text(labels, s, ui, &path, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), x, y + 40.0, cw);

    for panel in Panel::ALL {
        let r = w.tab_rect(panel);
        let selected = panel == w.panel;
        fill(s, ui, r, CARD_R, if selected { SEGMENT_ON.now() } else { CARD_BG.now() });
        let color = if selected { LABEL.now() } else { LABEL_2.now() };
        text(labels, s, ui, panel.title(), MAC_CALLOUT_PT, Weight::Regular, color, r.x + 12.0, r.y + r.h / 2.0, r.w - 24.0);
    }

    let body = w.body_rect();
    fill(s, ui, body, CARD_R, CARD_BG.now());
    // the gamut chart is not scrolled: its panel has no lines to scroll
    let scroll = if w.panel == Panel::Gamut { 0.0 } else { w.scroll };
    let area = Area {
        x: body.x + BODY_PAD_PT,
        w: body.w - 2.0 * BODY_PAD_PT,
        top: body.y + BODY_PAD_PT - scroll,
        clip: body.y,
        bottom: body.y + body.h,
    };
    match w.panel {
        Panel::Header => paint_header(labels, s, ui, c, area),
        Panel::Tags => paint_tags(labels, s, ui, c, area),
        Panel::Verify => paint_verify(labels, s, ui, c, area),
        Panel::Gamut => paint_gamut(labels, s, ui, c, area),
    }
}

/// A single line of text in the error colour: what the file failed on.
fn paint_error(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: &Current, a: Area) {
    let message = c.error.as_deref().unwrap_or("The profile could not be read.");
    text(labels, s, ui, message, MAC_BODY_PT, Weight::Regular, SYSTEM_RED.now(), a.x, a.mid(0), a.w);
}

/// The header rows: a key column and the value.
fn paint_header(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: &Current, a: Area) {
    let Some(profile) = &c.profile else {
        return paint_error(labels, s, ui, c, a);
    };
    for (i, (key, value)) in model::header_rows(profile).iter().enumerate() {
        if !a.visible(a.top + i as f32 * LINE_PT) {
            continue;
        }
        let mid = a.mid(i);
        text(labels, s, ui, key, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), a.x, mid, KEY_PT - 12.0);
        text(labels, s, ui, value, MAC_BODY_PT, Weight::Regular, LABEL.now(), a.x + KEY_PT, mid, a.w - KEY_PT);
    }
}

/// The tag table: signature, name, data type, offset and size. A tag outside the file shows its signature in red.
fn paint_tags(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: &Current, a: Area) {
    let Some(profile) = &c.profile else {
        return paint_error(labels, s, ui, c, a);
    };
    let name_x = a.x + 64.0;
    let type_x = a.x + a.w - 300.0;
    let offset_x = a.x + a.w - 200.0;
    let size_x = a.x + a.w - 100.0;
    let heads = [(a.x, "Tag"), (name_x, "Name"), (type_x, "Type"), (offset_x, "Offset"), (size_x, "Size")];
    for (x, head) in heads {
        text(labels, s, ui, head, MAC_CALLOUT_PT, Weight::Semibold, LABEL_2.now(), x, a.mid(0), 120.0);
    }
    for (i, tag) in profile.tags.iter().enumerate() {
        let line = i + 1;
        if !a.visible(a.top + line as f32 * LINE_PT) {
            continue;
        }
        let mid = a.mid(line);
        let sig_color = if tag.in_bounds { LABEL.now() } else { SYSTEM_RED.now() };
        text(labels, s, ui, &tag.sig, MAC_BODY_PT, Weight::Regular, sig_color, a.x, mid, 56.0);
        text(labels, s, ui, model::tag_name(&tag.sig), MAC_BODY_PT, Weight::Regular, LABEL.now(), name_x, mid, type_x - name_x - 12.0);
        let kind = if tag.kind.is_empty() { "none" } else { tag.kind.trim_end() };
        text(labels, s, ui, kind, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), type_x, mid, 80.0);
        text(labels, s, ui, &tag.offset.to_string(), MAC_BODY_PT, Weight::Regular, LABEL_2.now(), offset_x, mid, 80.0);
        text(labels, s, ui, &format!("{} bytes", tag.size), MAC_BODY_PT, Weight::Regular, LABEL_2.now(), size_x, mid, 100.0);
    }
}

/// The verification: a summary line, then one line per finding with its severity.
fn paint_verify(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: &Current, a: Area) {
    if c.profile.is_none() {
        return paint_error(labels, s, ui, c, a);
    }
    let errors = c.issues.iter().filter(|i| i.severity == Severity::Error).count();
    let warnings = c.issues.len() - errors;
    let summary = if c.issues.is_empty() {
        "No problems found.".to_string()
    } else {
        format!("{} and {}.", plural(errors, "error"), plural(warnings, "warning"))
    };
    let summary_color = if errors > 0 { SYSTEM_RED.now() } else { LABEL.now() };
    text(labels, s, ui, &summary, MAC_SUBHEAD_PT, Weight::Semibold, summary_color, a.x, a.mid(0), a.w);
    if c.issues.is_empty() {
        let note = "The header, the tag table, the tag data and the required tags pass the checks.";
        text(labels, s, ui, note, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), a.x, a.mid(1), a.w);
    }
    for (i, issue) in c.issues.iter().enumerate() {
        let line = i + 1;
        if !a.visible(a.top + line as f32 * LINE_PT) {
            continue;
        }
        let mid = a.mid(line);
        let color = match issue.severity {
            Severity::Error => SYSTEM_RED.now(),
            Severity::Warning => SYSTEM_ORANGE.now(),
        };
        text(labels, s, ui, issue.severity.label(), MAC_BODY_PT, Weight::Semibold, color, a.x, mid, LABEL_COL_PT);
        text(labels, s, ui, &issue.message, MAC_BODY_PT, Weight::Regular, LABEL.now(), a.x + LABEL_COL_PT, mid, a.w - LABEL_COL_PT);
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The gamut: the CIE 1931 xy chart with the spectral locus, the sRGB primaries for reference, the profile's primaries
/// and white point, and the readout beside it.
fn paint_gamut(labels: &mut TextCache, s: &mut Surface, ui: &Ui, c: &Current, a: Area) {
    if c.profile.is_none() {
        return paint_error(labels, s, ui, c, a);
    }
    let Some(g) = c.gamut else {
        text(labels, s, ui, "This profile has no colorant tags, so it has no gamut to plot.", MAC_BODY_PT, Weight::Regular, LABEL.now(), a.x, a.mid(0), a.w);
        let why = "A gamut needs an RGB matrix profile with rXYZ, gXYZ and bXYZ tags. LUT, CMYK and greyscale profiles have none.";
        text(labels, s, ui, why, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), a.x, a.mid(1), a.w);
        return;
    };

    let side = (a.bottom - a.top - 4.0).min(a.w * 0.62).min(PLOT_MAX_PT).max(0.0);
    let (px, py) = (a.x, a.top);
    fill(s, ui, Rect { x: px, y: py, w: side, h: side }, 0.0, CONTENT_BG.now());
    let map = |x: f64, y: f64| -> (f32, f32) {
        (px + (x / PLOT_X_MAX) as f32 * side, py + side - (y / PLOT_Y_MAX) as f32 * side)
    };

    // grid every 0.1 in x and y, drawn as one-point rules
    for k in 1..=7 {
        let gx = k as f64 * 0.1;
        let (top, bottom) = (map(gx, PLOT_Y_MAX).1, map(gx, 0.0).1);
        rule(s, ui, map(gx, 0.0).0, top, 0.0, bottom - top);
    }
    for k in 1..=8 {
        let gy = k as f64 * 0.1;
        let (left, right) = (map(0.0, gy).0, map(PLOT_X_MAX, gy).0);
        rule(s, ui, left, map(0.0, gy).1, right - left, 0.0);
    }

    // the spectral locus, closed by the line of purples
    let count = SPECTRAL_LOCUS.len();
    for i in 0..count {
        let a_pt = SPECTRAL_LOCUS[i];
        let b_pt = SPECTRAL_LOCUS[(i + 1) % count];
        segment(s, ui, map(a_pt.0, a_pt.1), map(b_pt.0, b_pt.1), LABEL_2.now());
    }

    // the sRGB primaries for reference, then the profile's triangle
    let srgb = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
    for i in 0..3 {
        segment(s, ui, map(srgb[i].0, srgb[i].1), map(srgb[(i + 1) % 3].0, srgb[(i + 1) % 3].1), LABEL_3.now());
    }
    let corners = [g.red, g.green, g.blue];
    for i in 0..3 {
        segment(s, ui, map(corners[i].0, corners[i].1), map(corners[(i + 1) % 3].0, corners[(i + 1) % 3].1), LABEL.now());
    }
    let markers = [(g.red, SYSTEM_RED.now()), (g.green, SYSTEM_GREEN.now()), (g.blue, CONTROL_ACCENT.now())];
    for (p, color) in markers {
        let (x, y) = map(p.0, p.1);
        dot(s, ui, x, y, 3.0, color);
    }
    let (wx, wy) = map(g.white.0, g.white.1);
    dot(s, ui, wx, wy, 2.5, LABEL.now());

    // the readout beside the chart
    let rx = px + side + 24.0;
    let rw = (a.x + a.w - rx).max(0.0);
    let rows = [("Red", g.red), ("Green", g.green), ("Blue", g.blue), ("White", g.white)];
    for (i, (name, p)) in rows.iter().enumerate() {
        let mid = a.mid(i);
        text(labels, s, ui, name, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), rx, mid, READOUT_KEY_PT);
        let value = format!("x {:.4}   y {:.4}", p.0, p.1);
        text(labels, s, ui, &value, MAC_BODY_PT, Weight::Regular, LABEL.now(), rx + READOUT_KEY_PT, mid, rw - READOUT_KEY_PT);
    }
    let area = g.area();
    let ratio = format!("{:.0} %", area / SRGB_AREA * 100.0);
    let area_text = format!("{area:.4} on the xy chart");
    text(labels, s, ui, "Area", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), rx, a.mid(5), READOUT_KEY_PT);
    text(labels, s, ui, &area_text, MAC_BODY_PT, Weight::Regular, LABEL.now(), rx + READOUT_KEY_PT, a.mid(5), rw - READOUT_KEY_PT);
    text(labels, s, ui, "Against sRGB", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), rx, a.mid(6), READOUT_KEY_PT);
    text(labels, s, ui, &ratio, MAC_BODY_PT, Weight::Regular, LABEL.now(), rx + READOUT_KEY_PT, a.mid(6), rw - READOUT_KEY_PT);
    let notes = [
        "The chart is CIE 1931 xy with the spectral locus. The grey triangle is sRGB.",
        "The colorants are D50-adapted, so the sRGB ratio is an estimate.",
    ];
    for (i, note) in notes.iter().enumerate() {
        text(labels, s, ui, note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), rx, a.mid(8 + i), rw);
    }
}
