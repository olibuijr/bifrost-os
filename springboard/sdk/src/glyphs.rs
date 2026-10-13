//! Line glyphs named after the system symbol set (`NativeImage(systemSymbolName:)`), drawn from lines, arcs and discs so they scale
//! to any size and need no symbol font (the device has none). Toolbars, source lists, search and path bars take a
//! symbol name; `glyph` draws it.
use crate::kit::ui::FIELD_BG;
use crate::kit::Ui;
use crate::widgets::blit;
use bifrost_render::canvas::{Canvas, PathData, PathPaint, PathVerb, RectF};
use bifrost_ui::symbol_effect::{Pose, SymbolEffect};
use bifrost_ui::Rect;
use std::f32::consts::PI;

// The line, disc, ring, arc and polygon primitives of the symbol set, on the `Canvas` vocabulary. Same geometry as the
// shell's Surface helpers: a line is a stroke of width `w` with round ends, a ring is a stroke inside radius `r`, an arc
// is a stroke of width `t` centred on radius `r` between `a0` and `a1` (clockwise from +x), a polygon fills even-odd.

/// A straight stroke of width `w` between two points, round at both ends.
pub fn line(c: &mut dyn Canvas, a: (f32, f32), b: (f32, f32), w: f32, color: u32) {
    let path = PathData { verbs: vec![PathVerb::Move(a.0, a.1), PathVerb::Line(b.0, b.1)] };
    c.path(&path, PathPaint::Stroke(w), color);
}

/// A filled disc of radius `r` centred on (`cx`, `cy`).
pub fn disc(c: &mut dyn Canvas, cx: f32, cy: f32, r: f32, color: u32) {
    c.fill_rrect(RectF::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, color);
}

/// A circle outline of radius `r`, `t` px thick (inside the radius).
pub fn ring(c: &mut dyn Canvas, cx: f32, cy: f32, r: f32, t: f32, color: u32) {
    c.stroke_rrect(RectF::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r, t, color);
}

/// An arc of radius `r`, `t` px thick, between angles `a0..a1` (radians, clockwise from +x on screen). The arc is a
/// polyline of 4 segments per 90 degrees, stroked with round ends.
#[allow(clippy::too_many_arguments)]
pub fn arc(c: &mut dyn Canvas, cx: f32, cy: f32, r: f32, t: f32, a0: f32, a1: f32, color: u32) {
    c.path(&PathData { verbs: arc_verbs(cx, cy, r, a0, a1) }, PathPaint::Stroke(t), color);
}

/// The polyline of an arc: one move and at least one segment per 22.5 degrees (`PI / 8`), at least one segment.
pub fn arc_verbs(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<PathVerb> {
    let steps = (((a1 - a0).abs() / (PI / 8.0)).ceil() as usize).max(1);
    (0..=steps)
        .map(|k| {
            let a = a0 + (a1 - a0) * k as f32 / steps as f32;
            let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
            if k == 0 { PathVerb::Move(x, y) } else { PathVerb::Line(x, y) }
        })
        .collect()
}

/// A filled polygon (even-odd), `pts` in order.
pub fn polygon(c: &mut dyn Canvas, pts: &[(f32, f32)], color: u32) {
    if pts.is_empty() {
        return;
    }
    c.path(&PathData { verbs: polygon_verbs(pts) }, PathPaint::FillEvenOdd, color);
}

/// The closed outline of a polygon: a move to the first point, a line to each other point, and a close.
pub fn polygon_verbs(pts: &[(f32, f32)]) -> Vec<PathVerb> {
    let mut verbs = Vec::with_capacity(pts.len() + 1);
    for (k, &(x, y)) in pts.iter().enumerate() {
        verbs.push(if k == 0 { PathVerb::Move(x, y) } else { PathVerb::Line(x, y) });
    }
    verbs.push(PathVerb::Close);
    verbs
}

/// Every name `glyph` draws, for galleries and tests.
pub const NAMES: [&str; 60] = [
    "chevron.left", "chevron.right", "sidebar.left", "square.grid.2x2", "list.bullet", "rectangle.split.3x1",
    "magnifyingglass", "xmark.circle.fill", "folder", "folder.badge.plus", "checkmark.circle", "clock",
    "menubar.dock.rectangle", "doc", "arrow.down.circle", "house", "desktopcomputer", "internaldrive", "trash", "tag",
    "music.note", "film", "photo", "ellipsis", "square.and.arrow.up", "plus", "globe", "arrow.up.arrow.down",
    "chevron.down", "backward.fill", "forward.fill", "play.fill", "pause.fill", "gobackward.15", "goforward.30",
    "person", "person.crop.circle", "star", "star.fill", "envelope", "phone", "bookmark", "textformat", "pencil",
    "square.and.pencil", "antenna.radiowaves.left.and.right", "mic", "music.note.list", "opticaldisc", "radio",
    "calculator", "cloud.sun", "ellipsis.circle", "info.circle", "flag", "calendar",
    "arrow.clockwise", "xmark", "rectangle.grid.1x2", "chevron.up",
];

/// True when `name` has a drawing.
pub fn has(name: &str) -> bool {
    NAMES.contains(&name)
}

/// The bounds a glyph is drawn in under `pose`: scaled about its centre, then offset by the pose's fraction of its size.
pub fn posed_bounds(bounds: Rect, pose: Pose) -> Rect {
    let w = bounds.w * pose.scale;
    let h = bounds.h * pose.scale;
    let cx = bounds.x + bounds.w / 2.0 + pose.dx * bounds.w;
    let cy = bounds.y + bounds.h / 2.0 + pose.dy * bounds.h;
    Rect { x: cx - w / 2.0, y: cy - h / 2.0, w, h }
}

/// `color` with its alpha multiplied by the pose's opacity (colours are packed ARGB, alpha in the top byte).
pub fn posed_color(color: u32, opacity: f32) -> u32 {
    let a = ((color >> 24) as f32 * opacity.clamp(0.0, 1.0)).round() as u32;
    (color & 0x00ff_ffff) | (a.min(255) << 24)
}

/// Draws symbol `name` with `effect` applied at `now` seconds (see `springboard::ui::symbol_effect`). Scale and offset
/// move the bounds and opacity fades the colour. Rotation is not drawn yet: glyphs have no rotation transform, so a
/// spinning or wiggling glyph draws upright. False when the name is unknown.
pub fn glyph_effect(s: &mut dyn Canvas, ui: &Ui, name: &str, bounds: Rect, color: u32, effect: &SymbolEffect, now: f64) -> bool {
    let pose = effect.pose(now);
    glyph(s, ui, name, posed_bounds(bounds, pose), posed_color(color, pose.opacity))
}

/// Draws symbol `name` centred in `bounds` (points) in `color`; false when the name is unknown (nothing drawn).
pub fn glyph(s: &mut dyn Canvas, ui: &Ui, name: &str, bounds: Rect, color: u32) -> bool {
    if !has(name) {
        return false;
    }
    let side = ui.pt(bounds.w.min(bounds.h));
    let (x0, y0) = (ui.pt(bounds.x) + (ui.pt(bounds.w) - side) / 2.0, ui.pt(bounds.y) + (ui.pt(bounds.h) - side) / 2.0);
    let t = (side * 0.085).max(1.2);
    let p = |u: f32, v: f32| (x0 + u * side, y0 + v * side);
    let poly = |s: &mut dyn Canvas, pts: &[(f32, f32)], closed: bool| {
        for w in pts.windows(2) {
            line(s, p(w[0].0, w[0].1), p(w[1].0, w[1].1), t, color);
        }
        if closed && pts.len() > 2 {
            let (a, b) = (pts[pts.len() - 1], pts[0]);
            line(s, p(a.0, a.1), p(b.0, b.1), t, color);
        }
    };
    let rbox = |s: &mut dyn Canvas, u: f32, v: f32, w: f32, h: f32, r: f32| {
        s.stroke_rrect(RectF::new(x0 + u * side, y0 + v * side, w * side, h * side), r * side, t, color);
    };
    let dot = |s: &mut dyn Canvas, u: f32, v: f32, r: f32| disc(s, x0 + u * side, y0 + v * side, r * side, color);
    let circle = |s: &mut dyn Canvas, u: f32, v: f32, r: f32| ring(s, x0 + u * side, y0 + v * side, r * side, t, color);
    let fill = |s: &mut dyn Canvas, pts: &[(f32, f32)]| {
        let v: Vec<(f32, f32)> = pts.iter().map(|&(u, v)| p(u, v)).collect();
        polygon(s, &v, color);
    };
    let bar = |s: &mut dyn Canvas, u: f32, v: f32, w: f32, h: f32| s.fill_rrect(RectF::new(x0 + u * side, y0 + v * side, w * side, h * side), w * side / 2.0, color);
    let arcd = |s: &mut dyn Canvas, u: f32, v: f32, r: f32, a0: f32, a1: f32| arc(s, x0 + u * side, y0 + v * side, r * side, t, a0, a1, color);
    // a five-point star around (0.5, 0.53)
    let star: Vec<(f32, f32)> = (0..10)
        .map(|k| {
            let a = -PI / 2.0 + k as f32 * PI / 5.0;
            let r = if k % 2 == 0 { 0.44 } else { 0.18 };
            (0.5 + r * a.cos(), 0.53 + r * a.sin())
        })
        .collect();
    let numeral = |s: &mut dyn Canvas, text: &str| {
        let img = ui.text(text, (side * 0.3 / ui.pt(1.0)).max(5.0), crate::kit::Weight::Semibold, color);
        let (cx, cy) = p(0.5, 0.54);
        blit(s, &img, cx as i32 - img.w as i32 / 2, cy as i32 - img.h as i32 / 2);
    };
    let folder = [(0.08, 0.22), (0.38, 0.22), (0.46, 0.32), (0.92, 0.32), (0.92, 0.80), (0.08, 0.80)];
    match name {
        "chevron.left" => poly(s, &[(0.62, 0.18), (0.34, 0.5), (0.62, 0.82)], false),
        "chevron.right" => poly(s, &[(0.38, 0.18), (0.66, 0.5), (0.38, 0.82)], false),
        "sidebar.left" => {
            rbox(s, 0.06, 0.18, 0.88, 0.64, 0.12);
            poly(s, &[(0.38, 0.18), (0.38, 0.82)], false);
            poly(s, &[(0.15, 0.33), (0.28, 0.33)], false);
            poly(s, &[(0.15, 0.45), (0.28, 0.45)], false);
        }
        "square.grid.2x2" => {
            for (u, v) in [(0.1, 0.1), (0.56, 0.1), (0.1, 0.56), (0.56, 0.56)] {
                rbox(s, u, v, 0.34, 0.34, 0.08);
            }
        }
        "list.bullet" => {
            for v in [0.26, 0.5, 0.74] {
                dot(s, 0.16, v, 0.06);
                poly(s, &[(0.34, v), (0.9, v)], false);
            }
        }
        "rectangle.split.3x1" => {
            rbox(s, 0.06, 0.2, 0.88, 0.6, 0.1);
            poly(s, &[(0.36, 0.2), (0.36, 0.8)], false);
            poly(s, &[(0.64, 0.2), (0.64, 0.8)], false);
        }
        // Files's gallery view: a large preview over a strip of thumbnails
        "rectangle.grid.1x2" => {
            rbox(s, 0.1, 0.12, 0.8, 0.5, 0.08);
            for u in [0.1, 0.38, 0.66] {
                rbox(s, u, 0.7, 0.24, 0.18, 0.05);
            }
        }
        "magnifyingglass" => {
            circle(s, 0.42, 0.42, 0.27);
            poly(s, &[(0.62, 0.62), (0.86, 0.86)], false);
        }
        "xmark.circle.fill" => {
            dot(s, 0.5, 0.5, 0.48);
            let knock = FIELD_BG.now();
            line(s, p(0.34, 0.34), p(0.66, 0.66), t, knock);
            line(s, p(0.66, 0.34), p(0.34, 0.66), t, knock);
        }
        "folder" => poly(s, &folder, true),
        "folder.badge.plus" => {
            poly(s, &folder, true);
            poly(s, &[(0.5, 0.44), (0.5, 0.70)], false);
            poly(s, &[(0.37, 0.57), (0.63, 0.57)], false);
        }
        "checkmark.circle" => {
            circle(s, 0.5, 0.5, 0.4);
            poly(s, &[(0.32, 0.52), (0.45, 0.65), (0.68, 0.38)], false);
        }
        "clock" => {
            circle(s, 0.5, 0.5, 0.4);
            poly(s, &[(0.5, 0.28), (0.5, 0.5), (0.65, 0.59)], false);
        }
        "menubar.dock.rectangle" => {
            rbox(s, 0.06, 0.16, 0.88, 0.68, 0.1);
            poly(s, &[(0.28, 0.7), (0.72, 0.7)], false);
        }
        "doc" => {
            poly(s, &[(0.24, 0.08), (0.6, 0.08), (0.78, 0.26), (0.78, 0.92), (0.24, 0.92)], true);
            poly(s, &[(0.6, 0.08), (0.6, 0.26), (0.78, 0.26)], false);
        }
        "arrow.down.circle" => {
            circle(s, 0.5, 0.5, 0.4);
            poly(s, &[(0.5, 0.28), (0.5, 0.7)], false);
            poly(s, &[(0.35, 0.55), (0.5, 0.7), (0.65, 0.55)], false);
        }
        "house" => {
            poly(s, &[(0.1, 0.5), (0.5, 0.14), (0.9, 0.5)], false);
            poly(s, &[(0.22, 0.4), (0.22, 0.86), (0.78, 0.86), (0.78, 0.4)], false);
            poly(s, &[(0.42, 0.86), (0.42, 0.62), (0.58, 0.62), (0.58, 0.86)], false);
        }
        "desktopcomputer" => {
            rbox(s, 0.08, 0.14, 0.84, 0.56, 0.08);
            poly(s, &[(0.5, 0.7), (0.5, 0.84)], false);
            poly(s, &[(0.32, 0.86), (0.68, 0.86)], false);
        }
        "internaldrive" => {
            rbox(s, 0.06, 0.3, 0.88, 0.42, 0.1);
            dot(s, 0.78, 0.51, 0.05);
        }
        "trash" => {
            poly(s, &[(0.14, 0.24), (0.86, 0.24)], false);
            poly(s, &[(0.38, 0.24), (0.38, 0.12), (0.62, 0.12), (0.62, 0.24)], false);
            poly(s, &[(0.24, 0.3), (0.3, 0.88), (0.7, 0.88), (0.76, 0.3)], false);
        }
        "tag" => {
            poly(s, &[(0.1, 0.14), (0.5, 0.14), (0.9, 0.54), (0.54, 0.9), (0.1, 0.48)], true);
            circle(s, 0.3, 0.34, 0.05);
        }
        "music.note" => {
            dot(s, 0.36, 0.76, 0.13);
            poly(s, &[(0.47, 0.76), (0.47, 0.14), (0.78, 0.24)], false);
        }
        "film" => {
            rbox(s, 0.08, 0.16, 0.84, 0.68, 0.08);
            poly(s, &[(0.28, 0.16), (0.28, 0.84)], false);
            poly(s, &[(0.72, 0.16), (0.72, 0.84)], false);
        }
        "photo" => {
            rbox(s, 0.06, 0.16, 0.88, 0.68, 0.1);
            poly(s, &[(0.14, 0.76), (0.4, 0.48), (0.58, 0.66), (0.7, 0.55), (0.86, 0.72)], false);
            dot(s, 0.7, 0.34, 0.07);
        }
        "ellipsis" => {
            for u in [0.22, 0.5, 0.78] {
                dot(s, u, 0.5, 0.075);
            }
        }
        "square.and.arrow.up" => {
            poly(s, &[(0.32, 0.42), (0.22, 0.42), (0.22, 0.9), (0.78, 0.9), (0.78, 0.42), (0.68, 0.42)], false);
            poly(s, &[(0.5, 0.64), (0.5, 0.1)], false);
            poly(s, &[(0.36, 0.24), (0.5, 0.1), (0.64, 0.24)], false);
        }
        "arrow.clockwise" => {
            arcd(s, 0.5, 0.54, 0.38, 0.0, PI);
            arcd(s, 0.5, 0.54, 0.38, -PI, -PI / 2.0);
            poly(s, &[(0.4, 0.06), (0.51, 0.16), (0.4, 0.26)], false);
        }
        "xmark" => {
            poly(s, &[(0.24, 0.24), (0.76, 0.76)], false);
            poly(s, &[(0.76, 0.24), (0.24, 0.76)], false);
        }
        "plus" => {
            poly(s, &[(0.5, 0.16), (0.5, 0.84)], false);
            poly(s, &[(0.16, 0.5), (0.84, 0.5)], false);
        }
        "globe" => {
            circle(s, 0.5, 0.5, 0.42);
            circle(s, 0.5, 0.5, 0.2);
            poly(s, &[(0.08, 0.5), (0.92, 0.5)], false);
            poly(s, &[(0.5, 0.08), (0.5, 0.92)], false);
        }
        "arrow.up.arrow.down" => {
            poly(s, &[(0.32, 0.84), (0.32, 0.16)], false);
            poly(s, &[(0.18, 0.3), (0.32, 0.16), (0.46, 0.3)], false);
            poly(s, &[(0.68, 0.16), (0.68, 0.84)], false);
            poly(s, &[(0.54, 0.7), (0.68, 0.84), (0.82, 0.7)], false);
        }
        "chevron.down" => poly(s, &[(0.18, 0.36), (0.5, 0.66), (0.82, 0.36)], false),
        "chevron.up" => poly(s, &[(0.18, 0.64), (0.5, 0.34), (0.82, 0.64)], false),
        "backward.fill" => {
            fill(s, &[(0.5, 0.24), (0.5, 0.76), (0.08, 0.5)]);
            fill(s, &[(0.92, 0.24), (0.92, 0.76), (0.5, 0.5)]);
        }
        "forward.fill" => {
            fill(s, &[(0.5, 0.24), (0.5, 0.76), (0.92, 0.5)]);
            fill(s, &[(0.08, 0.24), (0.08, 0.76), (0.5, 0.5)]);
        }
        "play.fill" => fill(s, &[(0.24, 0.12), (0.24, 0.88), (0.88, 0.5)]),
        "pause.fill" => {
            bar(s, 0.24, 0.14, 0.17, 0.72);
            bar(s, 0.59, 0.14, 0.17, 0.72);
        }
        "gobackward.15" => {
            arcd(s, 0.5, 0.52, 0.4, -PI / 2.0, PI);
            poly(s, &[(0.6, 0.02), (0.49, 0.12), (0.6, 0.22)], false);
            numeral(s, "15");
        }
        "goforward.30" => {
            arcd(s, 0.5, 0.52, 0.4, 0.0, PI);
            arcd(s, 0.5, 0.52, 0.4, -PI, -PI / 2.0);
            poly(s, &[(0.4, 0.02), (0.51, 0.12), (0.4, 0.22)], false);
            numeral(s, "30");
        }
        "person" => {
            circle(s, 0.5, 0.3, 0.17);
            arcd(s, 0.5, 0.9, 0.34, -PI, 0.0);
            poly(s, &[(0.16, 0.9), (0.84, 0.9)], false);
        }
        "person.crop.circle" => {
            circle(s, 0.5, 0.5, 0.44);
            circle(s, 0.5, 0.4, 0.14);
            arcd(s, 0.5, 0.88, 0.26, -PI * 0.86, -PI * 0.14);
        }
        "star" => poly(s, &star, true),
        "star.fill" => fill(s, &star),
        "envelope" => {
            rbox(s, 0.06, 0.2, 0.88, 0.6, 0.08);
            poly(s, &[(0.1, 0.26), (0.5, 0.56), (0.9, 0.26)], false);
        }
        "phone" => poly(
            s,
            &[(0.2, 0.1), (0.34, 0.1), (0.42, 0.3), (0.32, 0.4), (0.6, 0.68), (0.7, 0.58), (0.9, 0.66), (0.9, 0.8), (0.78, 0.9), (0.5, 0.8), (0.2, 0.5), (0.1, 0.22)],
            true,
        ),
        "bookmark" => poly(s, &[(0.24, 0.08), (0.76, 0.08), (0.76, 0.92), (0.5, 0.7), (0.24, 0.92)], true),
        "textformat" => {
            poly(s, &[(0.04, 0.82), (0.28, 0.18), (0.52, 0.82)], false);
            poly(s, &[(0.13, 0.6), (0.43, 0.6)], false);
            circle(s, 0.73, 0.68, 0.13);
            poly(s, &[(0.88, 0.48), (0.88, 0.82)], false);
        }
        "pencil" => {
            poly(s, &[(0.7, 0.1), (0.9, 0.3), (0.34, 0.86), (0.1, 0.9), (0.14, 0.66)], true);
            poly(s, &[(0.6, 0.2), (0.8, 0.4)], false);
        }
        "square.and.pencil" => {
            poly(s, &[(0.5, 0.16), (0.14, 0.16), (0.14, 0.88), (0.86, 0.88), (0.86, 0.52)], false);
            poly(s, &[(0.8, 0.06), (0.94, 0.2), (0.56, 0.58), (0.38, 0.62), (0.42, 0.44)], true);
        }
        "antenna.radiowaves.left.and.right" => {
            dot(s, 0.5, 0.42, 0.07);
            poly(s, &[(0.5, 0.48), (0.5, 0.92)], false);
            for r in [0.2, 0.36] {
                arcd(s, 0.5, 0.42, r, -PI / 4.0, PI / 4.0);
                arcd(s, 0.5, 0.42, r, PI * 0.75, PI);
                arcd(s, 0.5, 0.42, r, -PI, -PI * 0.75);
            }
        }
        "mic" => {
            rbox(s, 0.36, 0.06, 0.28, 0.5, 0.14);
            arcd(s, 0.5, 0.4, 0.26, 0.0, PI);
            poly(s, &[(0.5, 0.66), (0.5, 0.88)], false);
            poly(s, &[(0.36, 0.9), (0.64, 0.9)], false);
        }
        "music.note.list" => {
            poly(s, &[(0.06, 0.2), (0.5, 0.2)], false);
            poly(s, &[(0.06, 0.4), (0.5, 0.4)], false);
            poly(s, &[(0.06, 0.6), (0.36, 0.6)], false);
            dot(s, 0.56, 0.8, 0.11);
            poly(s, &[(0.66, 0.8), (0.66, 0.16), (0.92, 0.24)], false);
        }
        "opticaldisc" => {
            circle(s, 0.5, 0.5, 0.42);
            circle(s, 0.5, 0.5, 0.1);
            arcd(s, 0.5, 0.5, 0.27, -PI * 0.9, -PI * 0.6);
        }
        "radio" => {
            rbox(s, 0.06, 0.34, 0.88, 0.54, 0.08);
            poly(s, &[(0.2, 0.34), (0.72, 0.1)], false);
            circle(s, 0.32, 0.61, 0.13);
            for v in [0.5, 0.61, 0.72] {
                poly(s, &[(0.58, v), (0.82, v)], false);
            }
        }
        "calculator" => {
            rbox(s, 0.18, 0.06, 0.64, 0.88, 0.1);
            rbox(s, 0.28, 0.16, 0.44, 0.18, 0.03);
            for v in [0.5, 0.65, 0.8] {
                for u in [0.34, 0.5, 0.66] {
                    dot(s, u, v, 0.045);
                }
            }
        }
        "cloud.sun" => {
            circle(s, 0.32, 0.32, 0.13);
            for k in 0..5 {
                let a = PI * 0.75 + k as f32 * PI * 0.25;
                let (c, sn) = (a.cos(), a.sin());
                poly(s, &[(0.32 + 0.19 * c, 0.32 + 0.19 * sn), (0.32 + 0.27 * c, 0.32 + 0.27 * sn)], false);
            }
            arcd(s, 0.6, 0.6, 0.2, -PI, 0.0);
            arcd(s, 0.36, 0.74, 0.12, PI / 2.0, PI);
            arcd(s, 0.36, 0.74, 0.12, -PI, -PI / 2.0);
            arcd(s, 0.82, 0.74, 0.12, -PI / 2.0, PI / 2.0);
            poly(s, &[(0.36, 0.86), (0.82, 0.86)], false);
        }
        "ellipsis.circle" => {
            circle(s, 0.5, 0.5, 0.42);
            for u in [0.3, 0.5, 0.7] {
                dot(s, u, 0.5, 0.05);
            }
        }
        "info.circle" => {
            circle(s, 0.5, 0.5, 0.42);
            dot(s, 0.5, 0.3, 0.05);
            poly(s, &[(0.5, 0.44), (0.5, 0.72)], false);
        }
        "flag" => {
            poly(s, &[(0.2, 0.08), (0.2, 0.92)], false);
            poly(s, &[(0.2, 0.12), (0.84, 0.12), (0.7, 0.32), (0.84, 0.52), (0.2, 0.52)], true);
        }
        "calendar" => {
            rbox(s, 0.08, 0.14, 0.84, 0.76, 0.1);
            poly(s, &[(0.08, 0.36), (0.92, 0.36)], false);
            poly(s, &[(0.3, 0.06), (0.3, 0.2)], false);
            poly(s, &[(0.7, 0.06), (0.7, 0.2)], false);
            for v in [0.52, 0.7] {
                for u in [0.3, 0.5, 0.7] {
                    dot(s, u, v, 0.04);
                }
            }
        }
        "checklist" => {
            for v in [0.22, 0.5, 0.78] {
                poly(s, &[(0.08, v), (0.16, v + 0.07), (0.3, v - 0.08)], false);
                poly(s, &[(0.42, v), (0.92, v)], false);
            }
        }
        "newspaper" => {
            rbox(s, 0.16, 0.14, 0.76, 0.72, 0.08);
            poly(s, &[(0.16, 0.36), (0.06, 0.36), (0.06, 0.78), (0.14, 0.86)], false);
            rbox(s, 0.28, 0.26, 0.24, 0.2, 0.02);
            poly(s, &[(0.6, 0.3), (0.8, 0.3)], false);
            poly(s, &[(0.6, 0.42), (0.8, 0.42)], false);
            poly(s, &[(0.28, 0.58), (0.8, 0.58)], false);
            poly(s, &[(0.28, 0.72), (0.8, 0.72)], false);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Surface;

    #[test]
    fn every_named_glyph_draws_ink_and_unknown_names_draw_nothing() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(crate::kit::Font::parse(data).unwrap(), 2.0);
        for name in NAMES {
            let mut px = vec![0u32; 48 * 48];
            assert!(glyph(&mut Surface { w: 48, h: 48, stride: 48, px: &mut px, y0: 0 }, &ui, name, Rect { x: 2.0, y: 2.0, w: 20.0, h: 20.0 }, crate::kit::pack(255, 255, 255, 255)), "{name}");
            assert!(px.iter().filter(|p| **p != 0).count() > 20, "{name} left ink");
        }
        let mut px = vec![0u32; 48 * 48];
        assert!(!glyph(&mut Surface { w: 48, h: 48, stride: 48, px: &mut px, y0: 0 }, &ui, "no.such.symbol", Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }, 0xffff_ffff));
        assert!(px.iter().all(|p| *p == 0));
    }

    #[test]
    fn rest_pose_leaves_bounds_and_colour_unchanged() {
        let b = Rect { x: 10.0, y: 20.0, w: 40.0, h: 30.0 };
        let r = posed_bounds(b, Pose::REST);
        assert_eq!((r.x, r.y, r.w, r.h), (10.0, 20.0, 40.0, 30.0));
        assert_eq!(posed_color(0xff12_3456, 1.0), 0xff12_3456);
    }

    #[test]
    fn scale_keeps_the_centre_fixed() {
        let b = Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 };
        let r = posed_bounds(b, Pose { scale: 0.5, ..Pose::REST });
        assert_eq!((r.w, r.h), (20.0, 10.0));
        assert_eq!((r.x + r.w / 2.0, r.y + r.h / 2.0), (20.0, 10.0));
    }

    #[test]
    fn offset_moves_by_a_fraction_of_the_size() {
        let b = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        let r = posed_bounds(b, Pose { dx: 0.25, dy: -0.5, ..Pose::REST });
        assert_eq!((r.x + r.w / 2.0, r.y + r.h / 2.0), (30.0, 0.0));
    }

    #[test]
    fn opacity_scales_only_the_alpha_byte() {
        assert_eq!(posed_color(0xff00_ff00, 0.5), 0x8000_ff00);
        assert_eq!(posed_color(0xff00_ff00, 0.0), 0x0000_ff00);
        // Out-of-range opacity is clamped, never wrapped.
        assert_eq!(posed_color(0xff00_ff00, 3.0), 0xff00_ff00);
    }

    #[test]
    fn effect_pose_before_start_draws_the_glyph_as_usual() {
        let e = SymbolEffect::new(bifrost_ui::symbol_effect::Effect::Disappear, Default::default(), 5.0);
        let p = e.pose(1.0);
        let r = posed_bounds(Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }, p);
        assert_eq!((r.w, r.h), (20.0, 20.0));
        assert_eq!(posed_color(0xff00_0000, p.opacity), 0xff00_0000);
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::*;
    use crate::widgets::recorder::{Op, Rec};

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0)
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_quarter_arc_is_four_segments_from_three_oclock_to_six_oclock() {
        let v = arc_verbs(10.0, 20.0, 5.0, 0.0, PI / 2.0);
        assert_eq!(v.len(), 5, "ceil((pi/2) / (pi/8)) = 4 segments, five points");
        match (v[0], v[4]) {
            (PathVerb::Move(x0, y0), PathVerb::Line(x1, y1)) => {
                assert!(near((x0, y0), (15.0, 20.0)), "start at (cx + r, cy)");
                assert!(near((x1, y1), (10.0, 25.0)), "end at (cx, cy + r)");
            }
            other => panic!("unexpected verbs {other:?}"),
        }
        assert!(v[1..].iter().all(|p| matches!(p, PathVerb::Line(..))));
    }

    #[test]
    fn a_zero_span_arc_still_has_one_segment() {
        let v = arc_verbs(0.0, 0.0, 4.0, 1.0, 1.0);
        assert_eq!(v.len(), 2);
        match (v[0], v[1]) {
            (PathVerb::Move(x0, y0), PathVerb::Line(x1, y1)) => assert!(near((x0, y0), (x1, y1)), "no span, no length"),
            other => panic!("unexpected verbs {other:?}"),
        }
    }

    #[test]
    fn a_triangle_outline_is_a_move_two_lines_and_a_close() {
        let v = polygon_verbs(&[(0.0, 0.0), (4.0, 0.0), (0.0, 3.0)]);
        assert_eq!(
            v,
            vec![PathVerb::Move(0.0, 0.0), PathVerb::Line(4.0, 0.0), PathVerb::Line(0.0, 3.0), PathVerb::Close]
        );
    }

    #[test]
    fn an_empty_polygon_draws_nothing() {
        let mut r = Rec::default();
        polygon(&mut r, &[], 0xffff_ffff);
        assert!(r.ops.is_empty());
    }

    #[test]
    fn a_line_is_a_round_stroke_of_its_width_between_its_ends() {
        let mut r = Rec::default();
        line(&mut r, (1.0, 2.0), (9.0, 2.0), 3.0, 0xff11_2233);
        assert_eq!(
            r.ops,
            vec![Op::Path(vec![PathVerb::Move(1.0, 2.0), PathVerb::Line(9.0, 2.0)], PathPaint::Stroke(3.0), 0xff11_2233)]
        );
    }

    #[test]
    fn a_disc_is_a_fill_of_its_bounding_square_with_the_radius_as_corner() {
        let mut r = Rec::default();
        disc(&mut r, 10.0, 10.0, 4.0, 0xff00_00ff);
        assert_eq!(r.ops, vec![Op::Fill(RectF::new(6.0, 6.0, 8.0, 8.0), 4.0, 0xff00_00ff)]);
    }

    #[test]
    fn a_ring_is_a_stroke_inside_its_bounding_square() {
        let mut r = Rec::default();
        ring(&mut r, 10.0, 10.0, 5.0, 1.5, 0xffff_ffff);
        assert_eq!(r.ops, vec![Op::Stroke(RectF::new(5.0, 5.0, 10.0, 10.0), 5.0, 1.5, 0xffff_ffff)]);
    }

    #[test]
    fn a_chevron_draws_its_two_segments_and_an_unknown_name_draws_nothing() {
        let u = ui();
        let mut r = Rec::default();
        assert!(glyph(&mut r, &u, "chevron.left", Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }, 0xffff_ffff));
        let strokes = r.ops.iter().filter(|o| matches!(o, Op::Path(_, PathPaint::Stroke(_), _))).count();
        assert_eq!(strokes, 2, "an open three-point chevron is two segments");

        let mut none = Rec::default();
        assert!(!glyph(&mut none, &u, "no.such.symbol", Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }, 0xffff_ffff));
        assert!(none.ops.is_empty());
    }

    #[test]
    fn the_up_chevron_is_the_down_chevron_turned_over() {
        assert!(has("chevron.up") && has("chevron.down"));
        let u = ui();
        let b = Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 };
        let (mut up, mut down) = (Rec::default(), Rec::default());
        assert!(glyph(&mut up, &u, "chevron.up", b, 0xffff_ffff));
        assert!(glyph(&mut down, &u, "chevron.down", b, 0xffff_ffff));
        assert_eq!(up.ops.len(), down.ops.len(), "the same two segments");
    }
}

