//! `GraphicsContext`: the declarative UI `Canvas` drawing surface for apps (fill, stroke, images, text, transforms, clip,
//! opacity, blend mode, layers, shadow). CPU, anti-aliased by 4x4 supersampling. Built on
//! `bifrost_render::effects` (planes, layers, shadows) and `bifrost_render::blend`.
//!
//! `Canvas::new(w, h).draw(|ctx| { ... })` mirrors `Canvas { context, size in ... }`.
//! Core Graphics surface: current path (`begin_path`, `add_path`, `fill_path`, `stroke_path`, `clip_path`), CTM
//! (`concatenate`, `translate_by`, `scale_by`), line width, cap, join, miter limit and dash, fill and stroke colours,
//! linear and radial gradients, and `save_gstate` / `restore_gstate`.
//! Gaps: no symbols/resolved views, no filters beyond `add_shadow`, no shadow under gradients, `stroke_path`
//! measures the line width and dash with the CTM at stroke time (uniform scale), no PDF context.

use bifrost_render::blend::{composite, Mode};
use bifrost_render::effects::{premul, shadow_layer, Layer, Plane, Shadow};
use bifrost_render::font::{Font, Style};
use bifrost_render::gfx::Image;

use crate::graphics_gradient::{linear_param, radial_param, Affine, Gradient, GradientOptions};
use crate::graphics_stroke::{stroke_outline, LineCap, LineJoin, StrokeStyle};

/// A path verb in user space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Path {
        self.segs.push(Seg::Move(x, y));
        self
    }
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Path {
        self.segs.push(Seg::Line(x, y));
        self
    }
    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Path {
        self.segs.push(Seg::Quad(cx, cy, x, y));
        self
    }
    pub fn cubic_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> &mut Path {
        self.segs.push(Seg::Cubic(c1x, c1y, c2x, c2y, x, y));
        self
    }
    pub fn close(&mut self) -> &mut Path {
        self.segs.push(Seg::Close);
        self
    }
    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Path {
        let mut p = Path::new();
        p.move_to(x, y).line_to(x + w, y).line_to(x + w, y + h).line_to(x, y + h).close();
        p
    }
    pub fn ellipse(x: f32, y: f32, w: f32, h: f32) -> Path {
        const K: f32 = 0.552_284_8;
        let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let mut p = Path::new();
        p.move_to(cx + rx, cy)
            .cubic_to(cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry)
            .cubic_to(cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy)
            .cubic_to(cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry)
            .cubic_to(cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy)
            .close();
        p
    }

    /// `CGPath.currentPoint`.
    pub fn current_point(&self) -> Option<(f32, f32)> {
        let mut start = None;
        let mut cur = None;
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) => {
                    start = Some((x, y));
                    cur = start;
                }
                Seg::Line(x, y) | Seg::Quad(_, _, x, y) | Seg::Cubic(_, _, _, _, x, y) => cur = Some((x, y)),
                Seg::Close => cur = start,
            }
        }
        cur
    }
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }
    /// `addQuadCurve(to:control:)`.
    pub fn add_quad_curve(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Path {
        self.quad_to(cx, cy, x, y)
    }
    /// `addCurve(to:control1:control2:)`.
    pub fn add_curve(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> &mut Path {
        self.cubic_to(c1x, c1y, c2x, c2y, x, y)
    }
    /// `addRect(_:)`: a closed rectangle as its own subpath.
    pub fn add_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Path {
        self.segs.extend(Path::rect(x, y, w, h).segs);
        self
    }
    /// `addEllipse(in:)`.
    pub fn add_ellipse(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Path {
        self.segs.extend(Path::ellipse(x, y, w, h).segs);
        self
    }
    /// `addRoundedRect(in:cornerWidth:cornerHeight:)`; the corner size is clamped to half the rectangle.
    pub fn add_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, corner_w: f32, corner_h: f32) -> &mut Path {
        const K: f32 = 0.552_284_8;
        let (rx, ry) = (corner_w.clamp(0.0, w / 2.0), corner_h.clamp(0.0, h / 2.0));
        let (kx, ky) = (rx * K, ry * K);
        let (r, b) = (x + w, y + h);
        self.move_to(x + rx, y)
            .line_to(r - rx, y)
            .cubic_to(r - rx + kx, y, r, y + ry - ky, r, y + ry)
            .line_to(r, b - ry)
            .cubic_to(r, b - ry + ky, r - rx + kx, b, r - rx, b)
            .line_to(x + rx, b)
            .cubic_to(x + rx - kx, b, x, b - ry + ky, x, b - ry)
            .line_to(x, y + ry)
            .cubic_to(x, y + ry - ky, x + rx - kx, y, x + rx, y)
            .close()
    }
    /// `addArc(center:radius:startAngle:endAngle:clockwise:)` in a y-down space. Angles grow from the +x axis
    /// towards +y; `clockwise` sweeps towards decreasing angles. A line joins the current point to the arc start.
    pub fn add_arc(&mut self, cx: f32, cy: f32, radius: f32, start: f32, end: f32, clockwise: bool) -> &mut Path {
        use std::f32::consts::{FRAC_PI_2, TAU};
        let mut sweep = end - start;
        if clockwise {
            if sweep > 0.0 {
                sweep -= TAU * (sweep / TAU).ceil().max(1.0);
            }
        } else if sweep < 0.0 {
            sweep += TAU * (-sweep / TAU).ceil().max(1.0);
        }
        let sweep = sweep.clamp(-TAU, TAU);
        let (sx, sy) = (cx + radius * start.cos(), cy + radius * start.sin());
        if self.current_point().is_some() {
            self.line_to(sx, sy);
        } else {
            self.move_to(sx, sy);
        }
        let n = ((sweep.abs() / FRAC_PI_2).ceil() as usize).max(1);
        let step = sweep / n as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for i in 0..n {
            let (a0, a1) = (start + step * i as f32, start + step * (i + 1) as f32);
            let (c0, s0, c1, s1) = (a0.cos(), a0.sin(), a1.cos(), a1.sin());
            self.cubic_to(
                cx + radius * (c0 - k * s0),
                cy + radius * (s0 + k * c0),
                cx + radius * (c1 + k * s1),
                cy + radius * (s1 - k * c1),
                cx + radius * c1,
                cy + radius * s1,
            );
        }
        self
    }
    /// `addPath(_:transform:)`.
    pub fn add_path(&mut self, other: &Path, t: &Affine) -> &mut Path {
        self.segs.extend(other.transformed(t).segs);
        self
    }
    /// A copy with every point mapped through `t` (`CGPath.copy(using:)`).
    pub fn transformed(&self, t: &Affine) -> Path {
        let m = |x: f32, y: f32| t.apply(x, y);
        let segs = self
            .segs
            .iter()
            .map(|s| match *s {
                Seg::Move(x, y) => {
                    let p = m(x, y);
                    Seg::Move(p.0, p.1)
                }
                Seg::Line(x, y) => {
                    let p = m(x, y);
                    Seg::Line(p.0, p.1)
                }
                Seg::Quad(cx, cy, x, y) => {
                    let (c, p) = (m(cx, cy), m(x, y));
                    Seg::Quad(c.0, c.1, p.0, p.1)
                }
                Seg::Cubic(ax, ay, bx, by, x, y) => {
                    let (a, b, p) = (m(ax, ay), m(bx, by), m(x, y));
                    Seg::Cubic(a.0, a.1, b.0, b.1, p.0, p.1)
                }
                Seg::Close => Seg::Close,
            })
            .collect();
        Path { segs }
    }
    /// `CGPath.boundingBoxOfPath` as `(x, y, w, h)`: the tight bounds of the flattened curves; `None` when empty.
    pub fn bounding_box(&self) -> Option<(f32, f32, f32, f32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (pts, _) in self.flatten(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) {
            for (x, y) in pts {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        (x0 <= x1).then_some((x0, y0, x1 - x0, y1 - y0))
    }
    /// `CGPath.contains(_:using:)`: whether the point lies inside the filled path under `rule`.
    pub fn contains(&self, x: f32, y: f32, rule: FillRule) -> bool {
        let mut wn = 0;
        for (pts, _) in self.flatten(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) {
            let n = pts.len();
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.1 <= y) != (b.1 <= y) {
                    let t = (y - a.1) / (b.1 - a.1);
                    if a.0 + t * (b.0 - a.0) > x {
                        wn += if b.1 > a.1 { 1 } else { -1 };
                    }
                }
            }
        }
        match rule {
            FillRule::NonZero => wn != 0,
            FillRule::EvenOdd => wn & 1 != 0,
        }
    }

    /// Flattens to polylines in device space under `t` (`[a, b, c, d, tx, ty]`). The bool marks a closed subpath.
    fn flatten(&self, t: &[f32; 6]) -> Vec<(Vec<(f32, f32)>, bool)> {
        let ap = |x: f32, y: f32| (t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5]);
        let mut out: Vec<(Vec<(f32, f32)>, bool)> = Vec::new();
        let mut cur: Vec<(f32, f32)> = Vec::new();
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) => {
                    if cur.len() > 1 {
                        out.push((std::mem::take(&mut cur), false));
                    }
                    cur = vec![ap(x, y)];
                }
                Seg::Line(x, y) => cur.push(ap(x, y)),
                Seg::Quad(cx, cy, x, y) => {
                    let p0 = *cur.last().unwrap_or(&(0.0, 0.0));
                    let (c, p1) = (ap(cx, cy), ap(x, y));
                    for i in 1..=16 {
                        let u = i as f32 / 16.0;
                        let v = 1.0 - u;
                        cur.push((
                            v * v * p0.0 + 2.0 * v * u * c.0 + u * u * p1.0,
                            v * v * p0.1 + 2.0 * v * u * c.1 + u * u * p1.1,
                        ));
                    }
                }
                Seg::Cubic(ax, ay, bx, by, x, y) => {
                    let p0 = *cur.last().unwrap_or(&(0.0, 0.0));
                    let (c1, c2, p1) = (ap(ax, ay), ap(bx, by), ap(x, y));
                    for i in 1..=24 {
                        let u = i as f32 / 24.0;
                        let v = 1.0 - u;
                        let (a, b, c, d) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
                        cur.push((
                            a * p0.0 + b * c1.0 + c * c2.0 + d * p1.0,
                            a * p0.1 + b * c1.1 + c * c2.1 + d * p1.1,
                        ));
                    }
                }
                Seg::Close => {
                    if cur.len() > 1 {
                        let first = cur[0];
                        out.push((std::mem::take(&mut cur), true));
                        cur = vec![first];
                    }
                }
            }
        }
        if cur.len() > 1 {
            out.push((cur, false));
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Vertical sub-scanlines per pixel row; horizontal coverage within a sub-scanline is exact.
const SS: usize = 4;

/// Coverage plane of filled polygons. Each pixel row is sampled at `SS` sub-scanlines; spans between crossings
/// (non-zero or even-odd) add their exact horizontal overlap to the pixels they touch.
fn fill_cover(w: usize, h: usize, polys: &[(Vec<(f32, f32)>, bool)], rule: FillRule) -> Plane {
    let mut p = Plane::new(w, h);
    // (x0, y0, x1, y1, direction) with y0 < y1; fills close every subpath implicitly.
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
    for (pts, _) in polys {
        let n = pts.len();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            if a.1 == b.1 || !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
                continue;
            }
            let e = if a.1 < b.1 { (a.0, a.1, b.0, b.1, 1) } else { (b.0, b.1, a.0, a.1, -1) };
            ymin = ymin.min(e.1);
            ymax = ymax.max(e.3);
            edges.push(e);
        }
    }
    if edges.is_empty() {
        return p;
    }
    let by0 = ymin.floor().max(0.0) as usize;
    let by1 = (ymax.ceil().max(0.0) as usize).min(h);
    let mut acc = vec![0.0f32; w];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for y in by0..by1 {
        acc.iter_mut().for_each(|v| *v = 0.0);
        let mut any = false;
        for s in 0..SS {
            let sy = y as f32 + (s as f32 + 0.5) / SS as f32;
            xs.clear();
            for &(x0, y0, x1, y1, dir) in &edges {
                if sy >= y0 && sy < y1 {
                    xs.push((x0 + (sy - y0) / (y1 - y0) * (x1 - x0), dir));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wn = 0;
            for i in 0..xs.len().saturating_sub(1) {
                wn += xs[i].1;
                let inside = match rule {
                    FillRule::NonZero => wn != 0,
                    FillRule::EvenOdd => wn & 1 != 0,
                };
                if inside {
                    add_span(&mut acc, xs[i].0, xs[i + 1].0);
                    any = true;
                }
            }
        }
        if any {
            for x in 0..w {
                p.a[y * w + x] = (acc[x] / SS as f32).min(1.0);
            }
        }
    }
    p
}

/// Adds the horizontal overlap of the span `[xa, xb]` with each pixel it covers.
fn add_span(acc: &mut [f32], xa: f32, xb: f32) {
    let w = acc.len() as f32;
    let (xa, xb) = (xa.clamp(0.0, w), xb.clamp(0.0, w));
    if xb <= xa {
        return;
    }
    let (i0, i1) = (xa.floor() as usize, xb.floor() as usize);
    if i0 == i1 {
        acc[i0] += xb - xa;
        return;
    }
    acc[i0] += (i0 + 1) as f32 - xa;
    for v in &mut acc[i0 + 1..i1] {
        *v += 1.0;
    }
    if i1 < acc.len() {
        acc[i1] += xb - i1 as f32;
    }
}

/// Coverage of a stroke: the outline pieces from the stroker filled together with the non-zero rule.
fn stroke_cover(w: usize, h: usize, polys: &[(Vec<(f32, f32)>, bool)], style: &StrokeStyle) -> Plane {
    let pieces: Vec<(Vec<(f32, f32)>, bool)> = stroke_outline(polys, style).into_iter().map(|p| (p, true)).collect();
    fill_cover(w, h, &pieces, FillRule::NonZero)
}

#[derive(Clone)]
struct State {
    t: [f32; 6],
    clip: Option<Plane>,
    opacity: f32,
    blend: Mode,
    shadow: Option<Shadow>,
    fill_color: u32,
    stroke_color: u32,
    line: StrokeStyle,
}

/// Drawing state and target. Colours are unpremultiplied RGBA (red low byte, alpha top byte).
pub struct GraphicsContext {
    pub layer: Layer,
    st: State,
    stack: Vec<State>,
    /// Current path in device space (`CGContext.path`); it is not part of the saved state.
    cur: Path,
}

fn mul(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    // result = a applied after b: x' = a(b(x))
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

impl GraphicsContext {
    pub fn new(w: usize, h: usize) -> GraphicsContext {
        GraphicsContext {
            layer: Layer::new(w, h),
            st: State { t: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], clip: None,
                opacity: 1.0,
                blend: Mode::Normal,
                shadow: None,
                fill_color: 0xff00_0000,
                stroke_color: 0xff00_0000,
                line: StrokeStyle::new(1.0),
            },
            stack: Vec::new(),
            cur: Path::new(),
        }
    }

    pub fn save(&mut self) {
        self.stack.push(self.st.clone());
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.st = s;
        }
    }
    pub fn translate(&mut self, x: f32, y: f32) {
        self.st.t = mul(&self.st.t, &[1.0, 0.0, 0.0, 1.0, x, y]);
    }
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.st.t = mul(&self.st.t, &[sx, 0.0, 0.0, sy, 0.0, 0.0]);
    }
    pub fn rotate(&mut self, radians: f32) {
        let (s, c) = radians.sin_cos();
        self.st.t = mul(&self.st.t, &[c, s, -s, c, 0.0, 0.0]);
    }
    /// Multiplies the opacity for later drawing (`GraphicsContext.opacity`).
    pub fn set_opacity(&mut self, o: f32) {
        self.st.opacity = o.clamp(0.0, 1.0);
    }
    pub fn set_blend_mode(&mut self, m: Mode) {
        self.st.blend = m;
    }
    /// Shadow applied to each later fill, stroke, image and text (`GraphicsContext.addFilter(.shadow)`).
    pub fn add_shadow(&mut self, s: Option<Shadow>) {
        self.st.shadow = s;
    }
    /// Intersects the clip with a path (`GraphicsContext.clip(to:)`), anti-aliased, non-zero rule.
    pub fn clip(&mut self, path: &Path) {
        self.clip_with(path, FillRule::NonZero);
    }

    /// `CGContext.clip(using:)` for an explicit path.
    pub fn clip_with(&mut self, path: &Path, rule: FillRule) {
        let c = fill_cover(self.layer.w, self.layer.h, &path.flatten(&self.st.t), rule);
        self.intersect_clip(c);
    }

    /// `CGContext.clip(to: rect)`.
    pub fn clip_to_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.clip(&Path::rect(x, y, w, h));
    }

    fn intersect_clip(&mut self, c: Plane) {
        match &mut self.st.clip {
            Some(old) => old.multiply(&c),
            None => self.st.clip = Some(c),
        }
    }

    fn paint(&mut self, mut cover: Plane, color: u32) {
        if let Some(c) = &self.st.clip {
            cover.multiply(c);
        }
        if let Some(sh) = self.st.shadow {
            let s = shadow_layer(&cover, sh);
            self.layer.draw_group(&s, self.st.opacity, self.st.blend, 0, 0);
        }
        self.layer.fill_plane(&cover, premul_scale(color, self.st.opacity), self.st.blend);
    }

    pub fn fill(&mut self, path: &Path, color: u32, rule: FillRule) {
        let c = fill_cover(self.layer.w, self.layer.h, &path.flatten(&self.st.t), rule);
        self.paint(c, color);
    }

    /// Strokes with `line_width` and the current cap, join, miter limit and dash.
    pub fn stroke(&mut self, path: &Path, color: u32, line_width: f32) {
        let polys = path.flatten(&self.st.t);
        self.stroke_polys(&polys, color, line_width);
    }

    /// `strokeLineSegments` for one segment: strokes the line from `from` to `to`, in user space, with the stroke
    /// colour and line parameters. The current path is not changed.
    pub fn stroke_line(&mut self, from: (f32, f32), to: (f32, f32)) {
        let mut line = Path::new();
        line.move_to(from.0, from.1).line_to(to.0, to.1);
        let (color, width) = (self.st.stroke_color, self.st.line.width);
        self.stroke(&line, color, width);
    }

    /// Strokes the arc around `center` (user space) from `start` to `end` radians, with the stroke colour and line
    /// parameters, as `addArc` followed by a stroke would. The current path is not changed.
    pub fn stroke_arc(&mut self, center: (f32, f32), radius: f32, start: f32, end: f32, clockwise: bool) {
        let mut arc = Path::new();
        arc.add_arc(center.0, center.1, radius, start, end, clockwise);
        let (color, width) = (self.st.stroke_color, self.st.line.width);
        self.stroke(&arc, color, width);
    }

    fn stroke_polys(&mut self, polys: &[(Vec<(f32, f32)>, bool)], color: u32, line_width: f32) {
        let t = &self.st.t;
        let scale = (t[0] * t[3] - t[1] * t[2]).abs().sqrt();
        let mut style = self.st.line.clone();
        style.width = line_width * scale;
        style.dash_phase *= scale;
        style.dash = style.dash.iter().map(|d| d * scale).collect();
        let c = stroke_cover(self.layer.w, self.layer.h, polys, &style);
        self.paint(c, color);
    }

    // ---- CGContext state ----

    /// `saveGState`.
    pub fn save_gstate(&mut self) {
        self.save();
    }
    /// `restoreGState`.
    pub fn restore_gstate(&mut self) {
        self.restore();
    }
    /// `ctm`.
    pub fn ctm(&self) -> Affine {
        Affine::from_array(self.st.t)
    }
    /// `concatenate(_:)`: `m` applies to user space before the current transform.
    pub fn concatenate(&mut self, m: &Affine) {
        self.st.t = m.concatenating(&self.ctm()).to_array();
    }
    /// `translateBy(x:y:)`.
    pub fn translate_by(&mut self, x: f32, y: f32) {
        self.translate(x, y);
    }
    /// `scaleBy(x:y:)`.
    pub fn scale_by(&mut self, sx: f32, sy: f32) {
        self.scale(sx, sy);
    }
    /// `rotate(by:)`.
    pub fn rotate_by(&mut self, radians: f32) {
        self.rotate(radians);
    }
    /// `setAlpha`.
    pub fn set_alpha(&mut self, a: f32) {
        self.set_opacity(a);
    }
    pub fn set_fill_color(&mut self, color: u32) {
        self.st.fill_color = color;
    }
    pub fn set_stroke_color(&mut self, color: u32) {
        self.st.stroke_color = color;
    }
    pub fn set_line_width(&mut self, w: f32) {
        self.st.line.width = w.max(0.0);
    }
    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.st.line.cap = cap;
    }
    pub fn set_line_join(&mut self, join: LineJoin) {
        self.st.line.join = join;
    }
    pub fn set_miter_limit(&mut self, limit: f32) {
        self.st.line.miter_limit = limit.max(1.0);
    }
    /// `setLineDash(phase:lengths:)`; an empty slice gives a solid line.
    pub fn set_line_dash(&mut self, phase: f32, lengths: &[f32]) {
        self.st.line.dash_phase = phase;
        self.st.line.dash = lengths.to_vec();
    }
    /// `setShadow(offset:blur:color:)`; the offset is in device pixels, `blur` is the blur radius.
    pub fn set_shadow(&mut self, dx: f32, dy: f32, blur: f32, color: u32) {
        self.st.shadow = Some(Shadow { color, radius: blur, x: dx, y: dy, spread: 0.0 });
    }

    // ---- CGContext current path ----

    pub fn begin_path(&mut self) {
        self.cur = Path::new();
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        let (px, py) = self.ctm().apply(x, y);
        self.cur.move_to(px, py);
    }
    pub fn add_line(&mut self, x: f32, y: f32) {
        let (px, py) = self.ctm().apply(x, y);
        self.cur.line_to(px, py);
    }
    pub fn close_path(&mut self) {
        self.cur.close();
    }
    /// `addPath(_:)`: the path is placed with the CTM at call time, as Core Graphics does.
    pub fn add_path(&mut self, path: &Path) {
        let t = self.ctm();
        self.cur.segs.extend(path.transformed(&t).segs);
    }
    /// The current path in device space.
    pub fn current_path(&self) -> &Path {
        &self.cur
    }
    /// `fillPath(using:)` with the fill colour; clears the current path.
    pub fn fill_path(&mut self, rule: FillRule) {
        let path = std::mem::take(&mut self.cur);
        let c = fill_cover(self.layer.w, self.layer.h, &path.flatten(&Affine::IDENTITY.to_array()), rule);
        self.paint(c, self.st.fill_color);
    }
    /// `strokePath()` with the stroke colour and line parameters; clears the current path.
    pub fn stroke_path(&mut self) {
        let path = std::mem::take(&mut self.cur);
        let polys = path.flatten(&Affine::IDENTITY.to_array());
        self.stroke_polys(&polys, self.st.stroke_color, self.st.line.width);
    }
    /// `clip(using:)` with the current path; clears it.
    pub fn clip_path(&mut self, rule: FillRule) {
        let path = std::mem::take(&mut self.cur);
        let c = fill_cover(self.layer.w, self.layer.h, &path.flatten(&Affine::IDENTITY.to_array()), rule);
        self.intersect_clip(c);
    }

    // ---- gradients ----

    /// `drawLinearGradient(_:start:end:options:)`, points in user space, painted through the clip.
    pub fn draw_linear_gradient(&mut self, g: &Gradient, start: (f32, f32), end: (f32, f32), o: GradientOptions) {
        self.paint_shader(|p| linear_param(start, end, p, o).map(|t| g.color_at(t)));
    }

    /// `drawRadialGradient(_:startCenter:startRadius:endCenter:endRadius:options:)`.
    pub fn draw_radial_gradient(
        &mut self,
        g: &Gradient,
        c0: (f32, f32),
        r0: f32,
        c1: (f32, f32),
        r1: f32,
        o: GradientOptions,
    ) {
        self.paint_shader(|p| radial_param(c0, r0, c1, r1, p, o).map(|t| g.color_at(t)));
    }

    /// Paints `shader(user_point)` over every clipped pixel; `None` leaves the pixel untouched.
    fn paint_shader(&mut self, shader: impl Fn((f32, f32)) -> Option<u32>) {
        let Some(inv) = self.ctm().inverted() else { return };
        let (w, h) = (self.layer.w, self.layer.h);
        for y in 0..h {
            for x in 0..w {
                let cov = self.st.clip.as_ref().map_or(1.0, |c| c.a[y * w + x]);
                if cov <= 0.0 {
                    continue;
                }
                let (ux, uy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
                let Some(color) = shader((ux, uy)) else { continue };
                let src = premul(color, cov * self.st.opacity);
                let d = &mut self.layer.px[y * w + x];
                *d = composite(src, *d, self.st.blend);
            }
        }
    }

    /// `CGLayer` content: draws a layer made with `new_layer_context` at a pixel offset, through clip, opacity and blend.
    pub fn draw_layer_at(&mut self, layer: &Layer, dx: i32, dy: i32) {
        let mut src = Layer::new(self.layer.w, self.layer.h);
        for y in 0..layer.h as i32 {
            for x in 0..layer.w as i32 {
                let (tx, ty) = (x + dx, y + dy);
                if tx >= 0 && ty >= 0 && (tx as usize) < src.w && (ty as usize) < src.h {
                    src.px[ty as usize * src.w + tx as usize] = layer.px[y as usize * layer.w + x as usize];
                }
            }
        }
        self.draw_layer_pixels(src);
    }

    /// `CGLayerCreateWithContext`: an offscreen context of `w` by `h` pixels to draw into and reuse via
    /// `draw_layer_at`. It starts with an identity transform and no clip.
    pub fn new_layer_context(w: usize, h: usize) -> GraphicsContext {
        GraphicsContext::new(w, h)
    }

    /// Draws a premultiplied image scaled into the rect `(x, y, w, h)` in user space (axis-aligned transforms).
    pub fn draw_image(&mut self, img: &Image, x: f32, y: f32, w: f32, h: f32) {
        if img.w == 0 || img.h == 0 {
            return;
        }
        let t = self.st.t;
        let (dx0, dy0) = (t[0] * x + t[4], t[3] * y + t[5]);
        let (dw, dh) = (t[0] * w, t[3] * h);
        let mut src = Layer::new(self.layer.w, self.layer.h);
        for py in 0..src.h {
            for pxl in 0..src.w {
                let (u, v) = ((pxl as f32 + 0.5 - dx0) / dw, (py as f32 + 0.5 - dy0) / dh);
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                src.px[py * src.w + pxl] = img.px[(v * img.h as f32) as usize * img.w + (u * img.w as f32) as usize];
            }
        }
        self.draw_layer_pixels(src);
    }

    /// Draws text with `font` at the baseline-box origin `(x, y)` in user space (translation only).
    pub fn draw_text(&mut self, font: &Font<'_>, text: &str, style: &Style, x: f32, y: f32) {
        let img = font.render(text, style);
        let t = self.st.t;
        let (ox, oy) = ((t[0] * x + t[4]).round() as i32, (t[3] * y + t[5]).round() as i32);
        let mut src = Layer::new(self.layer.w, self.layer.h);
        for j in 0..img.h as i32 {
            for i in 0..img.w as i32 {
                let (tx, ty) = (i + ox, j + oy);
                if tx >= 0 && ty >= 0 && (tx as usize) < src.w && (ty as usize) < src.h {
                    src.px[ty as usize * src.w + tx as usize] = img.px[j as usize * img.w + i as usize];
                }
            }
        }
        self.draw_layer_pixels(src);
    }

    fn draw_layer_pixels(&mut self, mut src: Layer) {
        if let Some(c) = &self.st.clip {
            src.mask(c);
        }
        if let Some(sh) = self.st.shadow {
            let s = shadow_layer(&src.alpha_plane(), sh);
            self.layer.draw_group(&s, self.st.opacity, self.st.blend, 0, 0);
        }
        self.layer.draw_group(&src, self.st.opacity, self.st.blend, 0, 0);
    }

    /// Draws `f` into a fresh layer and composites it as one group with the current opacity and blend mode
    /// (`GraphicsContext.drawLayer`): overlapping content inside does not show through at partial opacity.
    pub fn draw_layer(&mut self, f: impl FnOnce(&mut GraphicsContext)) {
        let mut inner = GraphicsContext::new(self.layer.w, self.layer.h);
        inner.st.t = self.st.t;
        inner.st.clip = self.st.clip.clone();
        f(&mut inner);
        self.draw_layer_pixels(inner.layer);
    }
}

fn premul_scale(color: u32, opacity: f32) -> u32 {
    // Keep the colour unpremultiplied; scale alpha only, so `fill_plane` premultiplies once.
    let a = (((color >> 24) & 0xff) as f32 * opacity.clamp(0.0, 1.0) + 0.5) as u32;
    (color & 0x00ff_ffff) | (a << 24)
}

/// declarative UI `Canvas { context, size in ... }`.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h }
    }
    /// Runs the drawing closure and returns the rendered premultiplied layer.
    pub fn draw(&self, f: impl FnOnce(&mut GraphicsContext, (f32, f32))) -> Layer {
        let mut ctx = GraphicsContext::new(self.w, self.h);
        f(&mut ctx, (self.w as f32, self.h as f32));
        ctx.layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_gradient::{Gradient, GradientOptions};
    use crate::graphics_stroke::{LineCap, LineJoin};

    const RED: u32 = 0xff00_00ff;
    const GREEN: u32 = 0xff00_ff00;

    #[test]
    fn rect_fill_is_pixel_exact_and_half_pixel_edges_blend() {
        let l = Canvas::new(8, 4).draw(|c, _| c.fill(&Path::rect(1.0, 1.0, 3.5, 2.0), RED, FillRule::NonZero));
        assert_eq!(l.get(1, 1), 0xff00_00ff);
        assert_eq!(l.get(0, 1), 0);
        assert_eq!(l.get(3, 2), 0xff00_00ff);
        assert_eq!(l.get(4, 1) >> 24, 128);
        assert_eq!(l.get(5, 1), 0);
    }

    #[test]
    fn even_odd_leaves_a_hole() {
        let mut p = Path::rect(0.0, 0.0, 8.0, 8.0);
        p.segs.extend(Path::rect(2.0, 2.0, 4.0, 4.0).segs);
        let l = Canvas::new(8, 8).draw(|c, _| c.fill(&p, RED, FillRule::EvenOdd));
        assert_eq!(l.get(4, 4), 0);
        assert_eq!(l.get(1, 1) >> 24, 255);
    }

    #[test]
    fn stroke_covers_the_line_width() {
        let mut p = Path::new();
        p.move_to(0.0, 4.0).line_to(10.0, 4.0);
        let l = Canvas::new(10, 8).draw(|c, _| c.stroke(&p, RED, 2.0));
        assert_eq!(l.get(5, 3) >> 24, 255);
        assert_eq!(l.get(5, 4) >> 24, 255);
        assert_eq!(l.get(5, 2), 0);
        assert_eq!(l.get(5, 5), 0);
    }

    #[test]
    fn translate_and_clip_apply() {
        let l = Canvas::new(8, 4).draw(|c, _| {
            c.save();
            c.clip(&Path::rect(0.0, 0.0, 4.0, 4.0));
            c.translate(2.0, 0.0);
            c.fill(&Path::rect(0.0, 0.0, 6.0, 4.0), RED, FillRule::NonZero);
            c.restore();
        });
        assert_eq!(l.get(1, 1), 0);
        assert_eq!(l.get(2, 1) >> 24, 255);
        assert_eq!(l.get(3, 1) >> 24, 255);
        assert_eq!(l.get(4, 1), 0);
    }

    #[test]
    fn layer_opacity_is_group_opacity() {
        let l = Canvas::new(4, 1).draw(|c, _| {
            c.set_opacity(0.5);
            c.draw_layer(|g| {
                g.fill(&Path::rect(0.0, 0.0, 3.0, 1.0), RED, FillRule::NonZero);
                g.fill(&Path::rect(1.0, 0.0, 3.0, 1.0), GREEN, FillRule::NonZero);
            });
        });
        // Overlap shows green at 50% alpha with no red contribution.
        assert_eq!(l.get(1, 0) & 0xff, 0);
        assert_eq!(l.get(1, 0) >> 24, 128);
        assert_eq!(l.get(1, 0), l.get(2, 0));
    }

    #[test]
    fn per_draw_opacity_lets_overlaps_show_through() {
        let l = Canvas::new(4, 1).draw(|c, _| {
            c.set_opacity(0.5);
            c.fill(&Path::rect(0.0, 0.0, 3.0, 1.0), RED, FillRule::NonZero);
            c.fill(&Path::rect(1.0, 0.0, 3.0, 1.0), GREEN, FillRule::NonZero);
        });
        assert!(l.get(1, 0) & 0xff > 0);
        assert!(l.get(1, 0) >> 24 > 128);
    }

    #[test]
    fn ellipse_fills_centre_not_corner() {
        let l = Canvas::new(10, 10).draw(|c, _| c.fill(&Path::ellipse(0.0, 0.0, 10.0, 10.0), RED, FillRule::NonZero));
        assert_eq!(l.get(5, 5) >> 24, 255);
        assert_eq!(l.get(0, 0), 0);
    }

    #[test]
    fn transformed_path_fills_at_the_new_place() {
        let mut p = Path::new();
        p.add_rect(0.0, 0.0, 2.0, 2.0);
        let t = Affine::scale(2.0, 2.0).translated_by(3.0, 0.0);
        let l = Canvas::new(10, 4).draw(|c, _| c.fill(&p.transformed(&t), RED, FillRule::NonZero));
        assert_eq!(l.get(3, 0) >> 24, 255);
        assert_eq!(l.get(8, 3) >> 24, 0);
        assert_eq!(l.get(2, 0), 0);
    }

    #[test]
    fn concatenate_matches_translate_then_scale_order() {
        let l = Canvas::new(10, 2).draw(|c, _| {
            c.concatenate(&Affine::translation(4.0, 0.0));
            c.concatenate(&Affine::scale(2.0, 1.0));
            c.fill(&Path::rect(0.0, 0.0, 2.0, 2.0), RED, FillRule::NonZero);
        });
        assert_eq!(l.get(4, 0) >> 24, 255);
        assert_eq!(l.get(7, 0) >> 24, 255);
        assert_eq!(l.get(8, 0), 0);
        assert_eq!(c_ctm_after_scale().a, 2.0);
    }

    fn c_ctm_after_scale() -> Affine {
        let mut c = GraphicsContext::new(2, 2);
        c.scale_by(2.0, 2.0);
        c.ctm()
    }

    #[test]
    fn save_and_restore_gstate_cover_ctm_clip_and_colours() {
        let mut c = GraphicsContext::new(8, 2);
        c.set_fill_color(RED);
        c.save_gstate();
        c.translate_by(4.0, 0.0);
        c.clip_to_rect(0.0, 0.0, 2.0, 2.0);
        c.set_fill_color(GREEN);
        c.add_path(&Path::rect(-10.0, 0.0, 30.0, 2.0));
        c.fill_path(FillRule::NonZero);
        c.restore_gstate();
        c.add_path(&Path::rect(0.0, 0.0, 1.0, 1.0));
        c.fill_path(FillRule::NonZero);
        assert_eq!(c.layer.get(4, 0), GREEN);
        assert_eq!(c.layer.get(6, 0), 0);
        assert_eq!(c.layer.get(0, 0), RED);
        assert!(c.current_path().is_empty());
    }

    #[test]
    fn clip_to_path_intersects_with_clip_to_rect() {
        let l = Canvas::new(10, 10).draw(|c, _| {
            c.clip_to_rect(0.0, 0.0, 5.0, 10.0);
            c.clip(&Path::ellipse(0.0, 0.0, 10.0, 10.0));
            c.fill(&Path::rect(0.0, 0.0, 10.0, 10.0), RED, FillRule::NonZero);
        });
        assert_eq!(l.get(4, 5) >> 24, 255);
        assert_eq!(l.get(7, 5), 0);
        assert_eq!(l.get(0, 0), 0);
    }

    #[test]
    fn clip_with_even_odd_leaves_a_hole() {
        let mut ring = Path::rect(0.0, 0.0, 8.0, 8.0);
        ring.add_rect(2.0, 2.0, 4.0, 4.0);
        let l = Canvas::new(8, 8).draw(|c, _| {
            c.clip_with(&ring, FillRule::EvenOdd);
            c.fill(&Path::rect(0.0, 0.0, 8.0, 8.0), RED, FillRule::NonZero);
        });
        assert_eq!(l.get(4, 4), 0);
        assert_eq!(l.get(1, 1) >> 24, 255);
    }

    #[test]
    fn stroke_square_cap_extends_past_the_end_and_butt_does_not() {
        let mut p = Path::new();
        p.move_to(2.0, 4.0).line_to(6.0, 4.0);
        let butt = Canvas::new(10, 8).draw(|c, _| c.stroke(&p, RED, 2.0));
        assert_eq!(butt.get(1, 4), 0);
        let sq = Canvas::new(10, 8).draw(|c, _| {
            c.set_line_cap(LineCap::Square);
            c.stroke(&p, RED, 2.0);
        });
        assert_eq!(sq.get(1, 4) >> 24, 255);
        assert_eq!(sq.get(6, 4) >> 24, 255);
        assert_eq!(sq.get(7, 4), 0);
    }

    #[test]
    fn stroke_dash_leaves_gaps() {
        let l = Canvas::new(10, 4).draw(|c, _| {
            c.set_line_dash(0.0, &[2.0, 2.0]);
            c.begin_path();
            c.move_to(0.0, 2.0);
            c.add_line(10.0, 2.0);
            c.set_line_width(2.0);
            c.set_stroke_color(RED);
            c.stroke_path();
        });
        assert_eq!(l.get(1, 1) >> 24, 255);
        assert_eq!(l.get(3, 1), 0);
        assert_eq!(l.get(5, 1) >> 24, 255);
    }

    #[test]
    fn stroke_join_bevel_cuts_the_corner_miter_fills_it() {
        let mut p = Path::new();
        p.move_to(1.0, 9.0).line_to(1.0, 3.0).line_to(9.0, 3.0);
        let miter = Canvas::new(10, 10).draw(|c, _| c.stroke(&p, RED, 2.0));
        assert_eq!(miter.get(0, 2) >> 24, 255);
        let bevel = Canvas::new(10, 10).draw(|c, _| {
            c.set_line_join(LineJoin::Bevel);
            c.stroke(&p, RED, 2.0);
        });
        assert!(bevel.get(0, 2) >> 24 < 255);
    }

    #[test]
    fn linear_gradient_runs_left_to_right_through_the_clip() {
        let g = Gradient::new(&[(0.0, 0xff00_0000), (1.0, 0xffff_ffff)]).unwrap();
        let l = Canvas::new(10, 2).draw(|c, _| {
            c.clip_to_rect(0.0, 0.0, 10.0, 1.0);
            c.draw_linear_gradient(&g, (0.0, 0.0), (10.0, 0.0), GradientOptions::default());
        });
        assert!(l.get(1, 0) & 0xff < l.get(8, 0) & 0xff);
        assert_eq!(l.get(5, 1), 0);
    }

    #[test]
    fn radial_gradient_is_centred_and_respects_extend() {
        let g = Gradient::new(&[(0.0, 0xffff_ffff), (1.0, 0xff00_0000)]).unwrap();
        let none = Canvas::new(10, 10).draw(|c, _| {
            c.draw_radial_gradient(&g, (5.0, 5.0), 0.0, (5.0, 5.0), 3.0, GradientOptions::default());
        });
        assert!(none.get(5, 5) & 0xff > 150);
        assert_eq!(none.get(0, 0), 0);
        let ext = Canvas::new(10, 10).draw(|c, _| {
            c.draw_radial_gradient(
                &g,
                (5.0, 5.0),
                0.0,
                (5.0, 5.0),
                3.0,
                GradientOptions { draws_before_start: false, draws_after_end: true },
            );
        });
        assert_eq!(ext.get(0, 0), 0xff00_0000);
    }

    #[test]
    fn alpha_and_blend_mode_apply_to_gradients_and_paths() {
        let l = Canvas::new(2, 1).draw(|c, _| {
            c.set_alpha(0.5);
            c.fill(&Path::rect(0.0, 0.0, 2.0, 1.0), RED, FillRule::NonZero);
        });
        assert_eq!(l.get(0, 0) >> 24, 128);
        let mul = Canvas::new(2, 1).draw(|c, _| {
            c.fill(&Path::rect(0.0, 0.0, 2.0, 1.0), 0xffff_ffff, FillRule::NonZero);
            c.set_blend_mode(Mode::Multiply);
            c.fill(&Path::rect(0.0, 0.0, 2.0, 1.0), RED, FillRule::NonZero);
        });
        assert_eq!(mul.get(0, 0) & 0x00ff_ffff, 0x0000_00ff);
    }

    #[test]
    fn arcs_curves_and_rounded_rects_have_expected_bounds() {
        let mut p = Path::new();
        p.add_arc(10.0, 10.0, 5.0, 0.0, std::f32::consts::TAU, false);
        let (x, y, w, h) = p.bounding_box().unwrap();
        assert!((x - 5.0).abs() < 0.05 && (y - 5.0).abs() < 0.05 && (w - 10.0).abs() < 0.1 && (h - 10.0).abs() < 0.1);
        let mut half = Path::new();
        half.add_arc(0.0, 0.0, 4.0, 0.0, std::f32::consts::PI, false);
        assert!(half.bounding_box().unwrap().1 >= -0.01, "counter-clockwise sweep in y-down goes through +y");
        let mut rr = Path::new();
        rr.add_rounded_rect(0.0, 0.0, 10.0, 10.0, 3.0, 3.0);
        assert!(!rr.contains(0.2, 0.2, FillRule::NonZero));
        assert!(rr.contains(5.0, 5.0, FillRule::NonZero));
        assert_eq!(rr.bounding_box(), Some((0.0, 0.0, 10.0, 10.0)));
        assert_eq!(Path::new().bounding_box(), None);
    }

    #[test]
    fn contains_honours_the_fill_rule() {
        let mut p = Path::rect(0.0, 0.0, 8.0, 8.0);
        p.add_rect(2.0, 2.0, 4.0, 4.0);
        assert!(p.contains(4.0, 4.0, FillRule::NonZero));
        assert!(!p.contains(4.0, 4.0, FillRule::EvenOdd));
        assert!(p.contains(1.0, 1.0, FillRule::EvenOdd));
    }

    #[test]
    fn curves_reach_their_end_points() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0).add_quad_curve(5.0, 10.0, 10.0, 0.0).add_curve(10.0, 5.0, 5.0, 5.0, 0.0, 5.0);
        assert_eq!(p.current_point(), Some((0.0, 5.0)));
        assert!(p.bounding_box().unwrap().2 >= 10.0);
    }

    #[test]
    fn layer_context_is_reused_with_an_offset() {
        let mut layer = GraphicsContext::new_layer_context(2, 2);
        layer.fill(&Path::rect(0.0, 0.0, 2.0, 2.0), RED, FillRule::NonZero);
        let l = Canvas::new(8, 2).draw(|c, _| {
            c.draw_layer_at(&layer.layer, 1, 0);
            c.draw_layer_at(&layer.layer, 5, 0);
        });
        assert_eq!(l.get(0, 0), 0);
        assert_eq!(l.get(1, 0), RED);
        assert_eq!(l.get(6, 1), RED);
        assert_eq!(l.get(4, 0), 0);
    }

    #[test]
    fn set_shadow_draws_a_blurred_copy_behind() {
        let l = Canvas::new(20, 20).draw(|c, _| {
            c.set_shadow(3.0, 3.0, 2.0, 0xff00_0000);
            c.fill(&Path::rect(4.0, 4.0, 6.0, 6.0), RED, FillRule::NonZero);
        });
        assert_eq!(l.get(6, 6), RED);
        assert!(l.get(11, 11) >> 24 > 0);
    }

    #[test]
    fn many_paths_fill_quickly() {
        let start = std::time::Instant::now();
        let mut c = GraphicsContext::new(256, 256);
        for i in 0..1000 {
            let f = (i % 200) as f32;
            c.fill(&Path::ellipse(f, f * 0.5, 40.0, 40.0), RED, FillRule::NonZero);
        }
        assert!(c.layer.get(30, 30) >> 24 > 0);
        assert!(start.elapsed().as_secs() < 30);
    }
}
