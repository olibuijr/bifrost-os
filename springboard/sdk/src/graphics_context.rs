//! `GraphicsContext`: the declarative UI `Canvas` drawing surface for apps (fill, stroke, images, text, transforms, clip,
//! opacity, blend mode, layers, shadow). CPU, anti-aliased by 4x4 supersampling. Built on
//! `bifrost_render::effects` (planes, layers, shadows) and `bifrost_render::blend`.
//!
//! `Canvas::new(w, h).draw(|ctx| { ... })` mirrors `Canvas { context, size in ... }`.
//! Gaps against declarative UI: no symbols/resolved views, no filters beyond `add_shadow`, strokes use round joins and caps
//! only, dashes are not implemented.

use bifrost_render::blend::Mode;
use bifrost_render::effects::{shadow_layer, Layer, Plane, Shadow};
use bifrost_render::font::{Font, Style};
use bifrost_render::gfx::Image;

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

const SS: usize = 4;

fn winding(polys: &[(Vec<(f32, f32)>, bool)], px: f32, py: f32) -> i32 {
    let mut wn = 0;
    for (pts, _) in polys {
        let n = pts.len();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]); // fills implicitly close every subpath
            if (a.1 <= py) != (b.1 <= py) {
                let t = (py - a.1) / (b.1 - a.1);
                if a.0 + t * (b.0 - a.0) > px {
                    wn += if b.1 > a.1 { 1 } else { -1 };
                }
            }
        }
    }
    wn
}

/// Coverage plane of a filled path (4x4 supersampling).
fn fill_cover(w: usize, h: usize, polys: &[(Vec<(f32, f32)>, bool)], rule: FillRule) -> Plane {
    let mut p = Plane::new(w, h);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (pts, _) in polys {
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 {
        return p;
    }
    let (bx0, by0) = ((x0.floor().max(0.0)) as usize, (y0.floor().max(0.0)) as usize);
    let (bx1, by1) = (((x1.ceil() as i64).clamp(0, w as i64)) as usize, ((y1.ceil() as i64).clamp(0, h as i64)) as usize);
    for y in by0..by1 {
        for x in bx0..bx1 {
            let mut hit = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let (fx, fy) = (x as f32 + (sx as f32 + 0.5) / SS as f32, y as f32 + (sy as f32 + 0.5) / SS as f32);
                    let wn = winding(polys, fx, fy);
                    let inside = match rule {
                        FillRule::NonZero => wn != 0,
                        FillRule::EvenOdd => wn & 1 != 0,
                    };
                    hit += inside as usize;
                }
            }
            p.a[y * w + x] = hit as f32 / (SS * SS) as f32;
        }
    }
    p
}

/// Coverage of a stroked polyline set: union of segment quads and round joins/caps.
fn stroke_cover(w: usize, h: usize, polys: &[(Vec<(f32, f32)>, bool)], width: f32) -> Plane {
    let hw = (width * 0.5).max(0.0);
    let mut shapes: Vec<(Vec<(f32, f32)>, bool)> = Vec::new();
    for (pts, closed) in polys {
        let n = pts.len();
        let segs = if *closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1e-6 {
                let (nx, ny) = (-dy / len * hw, dx / len * hw);
                shapes.push((vec![(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], true));
            }
        }
        for &(cx, cy) in pts {
            let disc: Vec<(f32, f32)> = (0..16)
                .map(|k| {
                    let t = k as f32 / 16.0 * std::f32::consts::TAU;
                    (cx + hw * t.cos(), cy + hw * t.sin())
                })
                .collect();
            shapes.push((disc, true));
        }
    }
    let mut out = Plane::new(w, h);
    for s in shapes {
        // Each convex piece has a consistent winding sign only up to orientation; union through max coverage.
        let c = fill_cover(w, h, std::slice::from_ref(&s), FillRule::NonZero);
        for (d, v) in out.a.iter_mut().zip(&c.a) {
            *d = d.max(*v);
        }
    }
    out
}

#[derive(Clone)]
struct State {
    t: [f32; 6],
    clip: Option<Plane>,
    opacity: f32,
    blend: Mode,
    shadow: Option<Shadow>,
}

/// Drawing state and target. Colours are unpremultiplied RGBA (red low byte, alpha top byte).
pub struct GraphicsContext {
    pub layer: Layer,
    st: State,
    stack: Vec<State>,
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
            st: State { t: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], clip: None, opacity: 1.0, blend: Mode::Normal, shadow: None },
            stack: Vec::new(),
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
    /// Intersects the clip with a path (`GraphicsContext.clip(to:)`), anti-aliased.
    pub fn clip(&mut self, path: &Path) {
        let c = fill_cover(self.layer.w, self.layer.h, &path.flatten(&self.st.t), FillRule::NonZero);
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

    pub fn stroke(&mut self, path: &Path, color: u32, line_width: f32) {
        let scale = (self.st.t[0] * self.st.t[3] - self.st.t[1] * self.st.t[2]).abs().sqrt();
        let c = stroke_cover(self.layer.w, self.layer.h, &path.flatten(&self.st.t), line_width * scale);
        self.paint(c, color);
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
}
