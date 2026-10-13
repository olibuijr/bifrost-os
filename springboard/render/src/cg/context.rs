//! The drawing context: a graphics-state stack over a pixel surface. Everything is drawn in user space
//! (through the current transform), clipped by an intersecting coverage stack, painted with the fill or stroke
//! paint under the chosen blend mode, alpha and shadow, and composited into the surface (or into the innermost
//! transparency layer). Paths come from `cg::path`, scan conversion and stroking from `cg::raster`.
//!
//! The current path lives in device space (points are transformed when they are added), as the transform may
//! change while a path is being built.
use super::color::Color;
use super::paint::Paint;
use super::path::{Affine, Path, Rect as PathRect};
use super::raster::{self, Coverage, FillRule, StrokeStyle};
pub use super::raster::{LineCap, LineJoin};
use crate::blend::{blend_channel, Mode};
use crate::font::{Font, Style};
use crate::gfx::{over, scale, Image, Surface};
use std::sync::Arc;

/// A rectangle in user space (origin top-left unless the transform says otherwise).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn max_x(&self) -> f32 {
        self.x + self.w
    }

    pub fn max_y(&self) -> f32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        !(self.w > 0.0 && self.h > 0.0)
    }
}

/// How a source combines with what is already there. The first sixteen are the separable and non-separable
/// blend formulas, the rest the Porter-Duff operators and the two additive modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Clear,
    Copy,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Xor,
    PlusDarker,
    PlusLighter,
}

impl BlendMode {
    fn separable(self) -> Option<Mode> {
        Some(match self {
            BlendMode::Normal => Mode::Normal,
            BlendMode::Multiply => Mode::Multiply,
            BlendMode::Screen => Mode::Screen,
            BlendMode::Overlay => Mode::Overlay,
            BlendMode::Darken => Mode::Darken,
            BlendMode::Lighten => Mode::Lighten,
            BlendMode::ColorDodge => Mode::ColorDodge,
            BlendMode::ColorBurn => Mode::ColorBurn,
            BlendMode::SoftLight => Mode::SoftLight,
            BlendMode::HardLight => Mode::HardLight,
            BlendMode::Difference => Mode::Difference,
            BlendMode::Exclusion => Mode::Exclusion,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// The context picks (bilinear).
    Default,
    /// Nearest neighbour.
    None,
    Low,
    Medium,
    High,
}

/// What `draw_path` paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathDrawing {
    Fill,
    EoFill,
    Stroke,
    FillStroke,
    EoFillStroke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDrawing {
    Fill,
    Invisible,
}

/// Shadow cast by what is drawn: offset in user space, blur as a Gaussian diameter, colour with its alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub offset: (f32, f32),
    pub blur: f32,
    pub color: Color,
}

#[allow(dead_code)]
#[derive(Clone)]
struct GState {
    ctm: Affine,
    clip: Option<Arc<Coverage>>,
    fill: Paint,
    stroke: Paint,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
    dash: Vec<f32>,
    dash_phase: f32,
    flatness: f32,
    alpha: f32,
    blend: BlendMode,
    shadow: Option<Shadow>,
    interp: Interpolation,
    antialias: bool,
    allows_antialias: bool,
    font_smoothing: bool,
    allows_font_smoothing: bool,
    subpixel_positioning: bool,
    subpixel_quantization: bool,
    font_size: f32,
    char_spacing: f32,
    text_mode: TextDrawing,
}

impl GState {
    fn new() -> Self {
        GState {
            ctm: Affine::IDENTITY,
            clip: None,
            fill: Paint::Solid(Color::black()),
            stroke: Paint::Solid(Color::black()),
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            flatness: 0.6,
            alpha: 1.0,
            blend: BlendMode::Normal,
            shadow: None,
            interp: Interpolation::Default,
            antialias: true,
            allows_antialias: true,
            font_smoothing: true,
            allows_font_smoothing: true,
            subpixel_positioning: false,
            subpixel_quantization: true,
            font_size: 12.0,
            char_spacing: 0.0,
            text_mode: TextDrawing::Fill,
        }
    }
}

/// An open transparency layer: its pixels and the state that composites it when it ends.
struct Layer {
    buf: Vec<u32>,
    alpha: f32,
    blend: BlendMode,
    shadow: Option<Shadow>,
    clip: Option<Arc<Coverage>>,
}

/// Premultiplied float pixel source, queried per device pixel.
enum Src<'s> {
    Solid([f32; 4]),
    Sample(&'s dyn Fn(i32, i32) -> [f32; 4]),
}

impl Src<'_> {
    #[inline]
    fn at(&self, x: i32, y: i32) -> [f32; 4] {
        match self {
            Src::Solid(c) => *c,
            Src::Sample(f) => f(x, y),
        }
    }
}

pub struct DrawContext<'a> {
    surf: Surface<'a>,
    layers: Vec<Layer>,
    gs: GState,
    stack: Vec<GState>,
    /// the current path, device space
    path: Path,
    /// maps text space to user space; its translation is the text position
    tm: Affine,
}

// ---------------------------------------------------------------------------------------------------------
// pixel helpers

#[inline]
fn unpack(p: u32) -> [f32; 4] {
    let f = |s: u32| ((p >> s) & 255) as f32 / 255.0;
    [f(0), f(8), f(16), f(24)]
}

#[inline]
fn pack_f(c: [f32; 4]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(c[0]) | q(c[1]) << 8 | q(c[2]) << 16 | q(c[3]) << 24
}

fn premul(c: &Color) -> [f32; 4] {
    let [r, g, b, a] = c.to_srgb().unwrap_or([0.0; 4]);
    [r * a, g * a, b * a, a]
}

/// The path-module rectangle for a context rectangle.
fn prect(r: Rect) -> PathRect {
    PathRect::new(r.x, r.y, r.w, r.h)
}

fn mul4(c: [f32; 4], k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, c[3] * k]
}

fn add4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        for v in &mut out {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for v in &mut out {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    out
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap_or(std::cmp::Ordering::Equal));
    let (lo, mid, hi) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    if c[hi] > c[lo] {
        out[mid] = (c[mid] - c[lo]) * s / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out
}

/// One premultiplied source pixel `s` composited on the premultiplied backdrop `d` under `mode`.
pub fn blend_pixel(mode: BlendMode, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (s[3], d[3]);
    let out = match mode {
        BlendMode::Clear => [0.0; 4],
        BlendMode::Copy => s,
        BlendMode::SourceIn => mul4(s, da),
        BlendMode::SourceOut => mul4(s, 1.0 - da),
        BlendMode::SourceAtop => add4(mul4(s, da), mul4(d, 1.0 - sa)),
        BlendMode::DestinationOver => add4(d, mul4(s, 1.0 - da)),
        BlendMode::DestinationIn => mul4(d, sa),
        BlendMode::DestinationOut => mul4(d, 1.0 - sa),
        BlendMode::DestinationAtop => add4(mul4(d, sa), mul4(s, 1.0 - da)),
        BlendMode::Xor => add4(mul4(s, 1.0 - da), mul4(d, 1.0 - sa)),
        BlendMode::PlusDarker => {
            let f = |i: usize| (1.0 - ((1.0 - d[i]) + (1.0 - s[i]))).max(0.0);
            [f(0), f(1), f(2), f(3)]
        }
        BlendMode::PlusLighter => {
            let f = |i: usize| (s[i] + d[i]).min(1.0);
            [f(0), f(1), f(2), f(3)]
        }
        _ => {
            if sa <= 0.0 {
                return d;
            }
            let cs = [s[0] / sa, s[1] / sa, s[2] / sa];
            let cb = if da > 0.0 { [d[0] / da, d[1] / da, d[2] / da] } else { cs };
            let blended = match mode.separable() {
                Some(m) => [blend_channel(m, cb[0], cs[0]), blend_channel(m, cb[1], cs[1]), blend_channel(m, cb[2], cs[2])],
                None => match mode {
                    BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
                    BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
                    BlendMode::Color => set_lum(cs, lum(cb)),
                    _ => set_lum(cb, lum(cs)),
                },
            };
            let mut o = [0.0f32; 4];
            for i in 0..3 {
                o[i] = (1.0 - da) * s[i] + (1.0 - sa) * d[i] + sa * da * blended[i];
            }
            o[3] = sa + da - sa * da;
            o
        }
    };
    [out[0].clamp(0.0, 1.0), out[1].clamp(0.0, 1.0), out[2].clamp(0.0, 1.0), out[3].clamp(0.0, 1.0)]
}

/// Composites premultiplied float source `s` into the packed pixel `dst` with coverage `c` (0..=255).
#[inline]
fn blend_into(mode: BlendMode, dst: u32, s: [f32; 4], c: u32) -> u32 {
    if mode == BlendMode::Normal {
        let sp = pack_f(s);
        return over(dst, if c == 255 { sp } else { scale(sp, c) });
    }
    let d = unpack(dst);
    let b = blend_pixel(mode, s, d);
    let t = c as f32 / 255.0;
    pack_f([d[0] + (b[0] - d[0]) * t, d[1] + (b[1] - d[1]) * t, d[2] + (b[2] - d[2]) * t, d[3] + (b[3] - d[3]) * t])
}

/// Three box-blur passes on rows and columns approximate a Gaussian of the same radius.
fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    let mut line = vec![0.0f32; w.max(h)];
    let mut col = vec![0.0f32; h];
    for _ in 0..3 {
        for y in 0..h {
            blur_line(&mut buf[y * w..(y + 1) * w], &mut line[..w], r, inv);
        }
        for x in 0..w {
            for y in 0..h {
                col[y] = buf[y * w + x];
            }
            blur_line(&mut col, &mut line[..h], r, inv);
            for y in 0..h {
                buf[y * w + x] = col[y];
            }
        }
    }
}

fn blur_line(v: &mut [f32], tmp: &mut [f32], r: usize, inv: f32) {
    let n = v.len();
    let mut sum = 0.0f32;
    for &x in v.iter().take(r.min(n)) {
        sum += x;
    }
    for i in 0..n {
        if i + r < n {
            sum += v[i + r];
        }
        tmp[i] = sum * inv;
        if i >= r {
            sum -= v[i - r];
        }
    }
    v.copy_from_slice(&tmp[..n]);
}

#[inline]
fn clip_at(clip: &Option<Arc<Coverage>>, x: i32, y: i32) -> u8 {
    match clip {
        None => 255,
        Some(c) => cov_at(c, x, y),
    }
}

#[inline]
fn cov_at(c: &Coverage, x: i32, y: i32) -> u8 {
    let (dx, dy) = (x - c.x, y - c.y);
    if dx < 0 || dy < 0 || dx as usize >= c.w || dy as usize >= c.h {
        0
    } else {
        c.a[dy as usize * c.w + dx as usize]
    }
}

/// The product of two coverages, over the overlap of their boxes.
fn intersect(a: &Coverage, b: &Coverage) -> Coverage {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w as i32).min(b.x + b.w as i32);
    let y1 = (a.y + a.h as i32).min(b.y + b.h as i32);
    if x1 <= x0 || y1 <= y0 {
        return Coverage { x: x0, y: y0, w: 0, h: 0, a: Vec::new() };
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut data = Vec::with_capacity(w * h);
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (cov_at(a, x, y) as u32, cov_at(b, x, y) as u32);
            data.push(((p * q + 127) / 255) as u8);
        }
    }
    Coverage { x: x0, y: y0, w, h, a: data }
}

fn rect_path(r: Rect) -> Path {
    Path::from_rect(prect(r))
}

fn ellipse_path(r: Rect) -> Path {
    Path::from_ellipse(prect(r))
}

fn bbox_of(points: &[(f32, f32)]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn texel(img: &Image, x: i64, y: i64, wrap: bool) -> [f32; 4] {
    let (w, h) = (img.w as i64, img.h as i64);
    let (x, y) = if wrap { (x.rem_euclid(w), y.rem_euclid(h)) } else { (x.clamp(0, w - 1), y.clamp(0, h - 1)) };
    unpack(img.px[(y * w + x) as usize])
}

/// Per-device-pixel image lookup through the inverse transform; `dest` is the image's user-space rectangle.
fn sampler<'i>(img: &'i Image, inv: Affine, dest: Rect, q: Interpolation, wrap: bool) -> impl Fn(i32, i32) -> [f32; 4] + 'i {
    let (iw, ih) = (img.w as f32, img.h as f32);
    let nearest = q == Interpolation::None;
    move |x, y| {
        let (ux, uy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
        let u = (ux - dest.x) / dest.w * iw;
        let v = (uy - dest.y) / dest.h * ih;
        if nearest {
            return texel(img, u.floor() as i64, v.floor() as i64, wrap);
        }
        let (u, v) = (u - 0.5, v - 0.5);
        let (fu, fv) = (u.floor(), v.floor());
        let (tx, ty) = (u - fu, v - fv);
        let (x0, y0) = (fu as i64, fv as i64);
        let a = texel(img, x0, y0, wrap);
        let b = texel(img, x0 + 1, y0, wrap);
        let c = texel(img, x0, y0 + 1, wrap);
        let d = texel(img, x0 + 1, y0 + 1, wrap);
        let mut o = [0.0f32; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bot = c[i] + (d[i] - c[i]) * tx;
            o[i] = top + (bot - top) * ty;
        }
        o
    }
}

/// Halves `img` with a box filter while it stays at least as large as `(tw, th)`; keeps big downscales from aliasing.
fn shrink(img: &Image, tw: usize, th: usize) -> Image {
    let mut cur = img.clone();
    while cur.w / 2 >= tw.max(1) && cur.h / 2 >= th.max(1) && cur.w >= 2 && cur.h >= 2 {
        let (nw, nh) = (cur.w / 2, cur.h / 2);
        let mut out = Image::new(nw, nh);
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = unpack(cur.px[(y * 2 + dy) * cur.w + x * 2 + dx]);
                    for i in 0..4 {
                        acc[i] += p[i] * 0.25;
                    }
                }
                out.px[y * nw + x] = pack_f(acc);
            }
        }
        cur = out;
    }
    cur
}

/// Draws into the innermost layer, or the base surface when none is open.
fn target<'t>(surf: &'t mut Surface<'_>, layers: &'t mut [Layer]) -> Surface<'t> {
    let (w, h, stride, y0) = (surf.w, surf.h, surf.stride, surf.y0);
    match layers.last_mut() {
        Some(l) => Surface { w, h, stride, px: &mut l.buf[..], y0 },
        None => Surface { w, h, stride, px: &mut *surf.px, y0 },
    }
}

fn cmyk(c: f32, m: f32, y: f32, k: f32, a: f32) -> Color {
    Color::srgb((1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k), a)
}

// ---------------------------------------------------------------------------------------------------------

impl<'a> DrawContext<'a> {
    /// A context drawing into `surface` (its band, if the surface is one). User space starts as device space.
    pub fn new(surface: &'a mut Surface<'_>) -> Self {
        let (w, h, stride, y0) = (surface.w, surface.h, surface.stride, surface.y0);
        Self::over_pixels(w, h, stride, &mut *surface.px, y0)
    }

    /// A context over raw pixels: `px` starts at row `y0` of a `w` x `h` frame with `stride` pixels per row.
    pub fn over_pixels(w: usize, h: usize, stride: usize, px: &'a mut [u32], y0: usize) -> Self {
        DrawContext {
            surf: Surface { w, h, stride, px, y0 },
            layers: Vec::new(),
            gs: GState::new(),
            stack: Vec::new(),
            path: Path::new(),
            tm: Affine::IDENTITY,
        }
    }

    /// Frame size in device pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.surf.w, self.surf.h)
    }

    /// Nothing is buffered: bitmap contexts draw synchronously.
    pub fn flush(&mut self) {}

    pub fn synchronize(&mut self) {}

    // ---- graphics state stack -------------------------------------------------------------------------

    pub fn save_gstate(&mut self) {
        self.stack.push(self.gs.clone());
    }

    /// Pops the last saved state; does nothing with an empty stack.
    pub fn restore_gstate(&mut self) {
        if let Some(g) = self.stack.pop() {
            self.gs = g;
        }
    }

    pub fn saved_depth(&self) -> usize {
        self.stack.len()
    }

    // ---- transform ------------------------------------------------------------------------------------

    pub fn ctm(&self) -> Affine {
        self.gs.ctm
    }

    /// The current transform as the user-space-to-device-space transform.
    pub fn user_to_device_transform(&self) -> Affine {
        self.gs.ctm
    }

    pub fn concat_ctm(&mut self, t: Affine) {
        self.gs.ctm = t.then(self.gs.ctm);
    }

    pub fn translate_ctm(&mut self, tx: f32, ty: f32) {
        self.concat_ctm(Affine::translate(tx, ty));
    }

    pub fn scale_ctm(&mut self, sx: f32, sy: f32) {
        self.concat_ctm(Affine::scale(sx, sy));
    }

    pub fn rotate_ctm(&mut self, radians: f32) {
        self.concat_ctm(Affine::rotate(radians));
    }

    /// Uniform scale of the current transform (square root of the area factor).
    fn unit_scale(&self) -> f32 {
        self.gs.ctm.determinant().abs().sqrt()
    }

    pub fn point_to_device(&self, p: (f32, f32)) -> (f32, f32) {
        self.gs.ctm.apply(p.0, p.1)
    }

    pub fn point_to_user(&self, p: (f32, f32)) -> (f32, f32) {
        match self.gs.ctm.invert() {
            Some(i) => i.apply(p.0, p.1),
            None => p,
        }
    }

    pub fn size_to_device(&self, s: (f32, f32)) -> (f32, f32) {
        let c = self.gs.ctm;
        (c.a * s.0 + c.c * s.1, c.b * s.0 + c.d * s.1)
    }

    pub fn size_to_user(&self, s: (f32, f32)) -> (f32, f32) {
        match self.gs.ctm.invert() {
            Some(i) => (i.a * s.0 + i.c * s.1, i.b * s.0 + i.d * s.1),
            None => s,
        }
    }

    pub fn rect_to_device(&self, r: Rect) -> Rect {
        let c = self.gs.ctm;
        bbox_of(&[c.apply(r.x, r.y), c.apply(r.max_x(), r.y), c.apply(r.max_x(), r.max_y()), c.apply(r.x, r.max_y())])
    }

    pub fn rect_to_user(&self, r: Rect) -> Rect {
        match self.gs.ctm.invert() {
            Some(i) => bbox_of(&[i.apply(r.x, r.y), i.apply(r.max_x(), r.y), i.apply(r.max_x(), r.max_y()), i.apply(r.x, r.max_y())]),
            None => r,
        }
    }

    // ---- attributes -----------------------------------------------------------------------------------

    pub fn set_line_width(&mut self, w: f32) {
        self.gs.line_width = w.max(0.0);
    }

    pub fn line_width(&self) -> f32 {
        self.gs.line_width
    }

    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.gs.cap = cap;
    }

    pub fn set_line_join(&mut self, join: LineJoin) {
        self.gs.join = join;
    }

    pub fn set_miter_limit(&mut self, limit: f32) {
        self.gs.miter_limit = limit.max(1.0);
    }

    /// `phase` into the pattern of alternating on/off lengths; an empty slice is a solid line.
    pub fn set_line_dash(&mut self, phase: f32, lengths: &[f32]) {
        let ok = lengths.iter().all(|l| *l >= 0.0 && l.is_finite()) && lengths.iter().any(|l| *l > 0.0);
        self.gs.dash = if ok { lengths.to_vec() } else { Vec::new() };
        self.gs.dash_phase = phase;
    }

    pub fn set_flatness(&mut self, f: f32) {
        self.gs.flatness = f.max(0.0);
    }

    pub fn flatness(&self) -> f32 {
        self.gs.flatness
    }

    /// Opacity applied to everything drawn, clamped to 0..=1.
    pub fn set_alpha(&mut self, a: f32) {
        self.gs.alpha = if a.is_nan() { 1.0 } else { a.clamp(0.0, 1.0) };
    }

    pub fn alpha(&self) -> f32 {
        self.gs.alpha
    }

    pub fn set_blend_mode(&mut self, mode: BlendMode) {
        self.gs.blend = mode;
    }

    pub fn blend_mode(&self) -> BlendMode {
        self.gs.blend
    }

    /// A shadow in the default colour (black at a third of full opacity).
    pub fn set_shadow(&mut self, offset: (f32, f32), blur: f32) {
        self.set_shadow_with_color(offset, blur, Some(Color::srgb(0.0, 0.0, 0.0, 1.0 / 3.0)));
    }

    /// `None` removes the shadow.
    pub fn set_shadow_with_color(&mut self, offset: (f32, f32), blur: f32, color: Option<Color>) {
        self.gs.shadow = color.map(|color| Shadow { offset, blur: blur.max(0.0), color });
    }

    pub fn clear_shadow(&mut self) {
        self.gs.shadow = None;
    }

    pub fn shadow(&self) -> Option<Shadow> {
        self.gs.shadow.clone()
    }

    pub fn set_interpolation_quality(&mut self, q: Interpolation) {
        self.gs.interp = q;
    }

    pub fn interpolation_quality(&self) -> Interpolation {
        self.gs.interp
    }

    pub fn set_should_antialias(&mut self, on: bool) {
        self.gs.antialias = on;
    }

    pub fn set_allows_antialiasing(&mut self, on: bool) {
        self.gs.allows_antialias = on;
    }

    pub fn set_should_smooth_fonts(&mut self, on: bool) {
        self.gs.font_smoothing = on;
    }

    pub fn set_allows_font_smoothing(&mut self, on: bool) {
        self.gs.allows_font_smoothing = on;
    }

    pub fn set_should_subpixel_position_fonts(&mut self, on: bool) {
        self.gs.subpixel_positioning = on;
    }

    pub fn set_should_subpixel_quantize_fonts(&mut self, on: bool) {
        self.gs.subpixel_quantization = on;
    }

    /// Edges are anti-aliased only when both the state flag and the context permission allow it.
    fn antialiased(&self) -> bool {
        self.gs.antialias && self.gs.allows_antialias
    }

    pub fn set_fill_paint(&mut self, p: Paint) {
        self.gs.fill = p;
    }

    pub fn set_stroke_paint(&mut self, p: Paint) {
        self.gs.stroke = p;
    }

    pub fn fill_paint(&self) -> &Paint {
        &self.gs.fill
    }

    pub fn stroke_paint(&self) -> &Paint {
        &self.gs.stroke
    }

    pub fn set_fill_color(&mut self, c: Color) {
        self.gs.fill = Paint::Solid(c);
    }

    pub fn set_stroke_color(&mut self, c: Color) {
        self.gs.stroke = Paint::Solid(c);
    }

    pub fn set_fill_rgb(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.set_fill_color(Color::srgb(r, g, b, a));
    }

    pub fn set_stroke_rgb(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.set_stroke_color(Color::srgb(r, g, b, a));
    }

    pub fn set_fill_gray(&mut self, w: f32, a: f32) {
        self.set_fill_color(Color::generic_gray(w, a));
    }

    pub fn set_stroke_gray(&mut self, w: f32, a: f32) {
        self.set_stroke_color(Color::generic_gray(w, a));
    }

    /// Plain device-CMYK conversion (no profile).
    pub fn set_fill_cmyk(&mut self, c: f32, m: f32, y: f32, k: f32, a: f32) {
        self.set_fill_color(cmyk(c, m, y, k, a));
    }

    pub fn set_stroke_cmyk(&mut self, c: f32, m: f32, y: f32, k: f32, a: f32) {
        self.set_stroke_color(cmyk(c, m, y, k, a));
    }

    // ---- path building (current path, device space) ---------------------------------------------------

    pub fn begin_path(&mut self) {
        self.path = Path::new();
    }

    pub fn is_path_empty(&self) -> bool {
        self.path.is_empty()
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        let (dx, dy) = self.gs.ctm.apply(x, y);
        self.path.move_to((dx, dy));
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        let (dx, dy) = self.gs.ctm.apply(x, y);
        self.path.line_to((dx, dy));
    }

    pub fn quad_curve_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let c = self.gs.ctm;
        let (a, b) = (c.apply(cx, cy), c.apply(x, y));
        self.path.quad_to(a, b);
    }

    pub fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c = self.gs.ctm;
        let (a, b, e) = (c.apply(c1x, c1y), c.apply(c2x, c2y), c.apply(x, y));
        self.path.cubic_to(a, b, e);
    }

    pub fn close_path(&mut self) {
        self.path.close();
    }

    /// Adds `p`, given in user space, to the current path.
    pub fn add_path(&mut self, p: &Path) {
        self.path.add_path(&p.transformed(&self.gs.ctm));
    }

    /// The current path in user space.
    pub fn copy_path(&self) -> Path {
        self.user_path()
    }

    pub fn add_rect(&mut self, r: Rect) {
        self.add_path(&rect_path(r));
    }

    pub fn add_rects(&mut self, rects: &[Rect]) {
        for r in rects {
            self.add_rect(*r);
        }
    }

    pub fn add_ellipse_in_rect(&mut self, r: Rect) {
        self.add_path(&ellipse_path(r));
    }

    /// A polyline through `points` (a new subpath).
    pub fn add_lines(&mut self, points: &[(f32, f32)]) {
        let Some(first) = points.first() else { return };
        self.move_to(first.0, first.1);
        for p in &points[1..] {
            self.line_to(p.0, p.1);
        }
    }

    pub fn add_arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, clockwise: bool) {
        let mut t = self.user_continuation();
        t.add_arc((cx, cy), r, start, end, clockwise);
        self.path.add_path(&t.transformed(&self.gs.ctm));
    }

    pub fn add_arc_to_point(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, r: f32) {
        let mut t = self.user_continuation();
        t.add_arc_to((x1, y1), (x2, y2), r);
        self.path.add_path(&t.transformed(&self.gs.ctm));
    }

    /// A user-space path that starts at the current point (if any), to extend with an arc.
    fn user_continuation(&self) -> Path {
        let mut t = Path::new();
        if let (Some(cp), Some(inv)) = (self.path.current_point(), self.gs.ctm.invert()) {
            let (x, y) = inv.apply(cp.0, cp.1);
            t.move_to((x, y));
        }
        t
    }

    fn user_path(&self) -> Path {
        match self.gs.ctm.invert() {
            Some(inv) => self.path.transformed(&inv),
            None => self.path.clone(),
        }
    }

    pub fn path_current_point(&self) -> Option<(f32, f32)> {
        self.path.current_point().map(|p| self.point_to_user(p))
    }

    /// Bounding box of the current path in user space (control points included).
    pub fn path_bounding_box(&self) -> Rect {
        match self.user_path().bounds() {
            Some(b) => Rect::new(b.x, b.y, b.w, b.h),
            None => Rect::ZERO,
        }
    }

    /// Whether the user-space point lies in the area `mode` would paint.
    pub fn path_contains_point(&self, p: (f32, f32), mode: PathDrawing) -> bool {
        let user = self.user_path();
        let fill = |rule| user.contains(p, rule);
        let stroked = || raster::stroke_path(&user, &self.stroke_style()).contains(p, FillRule::NonZero);
        match mode {
            PathDrawing::Fill => fill(FillRule::NonZero),
            PathDrawing::EoFill => fill(FillRule::EvenOdd),
            PathDrawing::Stroke => stroked(),
            PathDrawing::FillStroke => fill(FillRule::NonZero) || stroked(),
            PathDrawing::EoFillStroke => fill(FillRule::EvenOdd) || stroked(),
        }
    }

    /// Replaces the current path by the outline `stroke_path` would paint.
    pub fn replace_path_with_stroked_path(&mut self) {
        let outline = raster::stroke_path(&self.user_path(), &self.stroke_style());
        self.path = outline.transformed(&self.gs.ctm);
    }

    fn stroke_style(&self) -> StrokeStyle {
        StrokeStyle {
            width: self.gs.line_width,
            cap: self.gs.cap,
            join: self.gs.join,
            miter_limit: self.gs.miter_limit,
            dash: self.gs.dash.clone(),
            dash_phase: self.gs.dash_phase,
        }
    }

    // ---- coverage -------------------------------------------------------------------------------------

    /// Coverage of `path` under `xf` without the clip.
    fn raw_cov(&self, path: &Path, rule: FillRule, xf: &Affine) -> Coverage {
        let mut c = raster::fill(path, rule, xf, None, self.surf.w, self.surf.h);
        if !self.antialiased() {
            for v in &mut c.a {
                *v = if *v >= 128 { 255 } else { 0 };
            }
        }
        c
    }

    /// Coverage of `path` under `xf`, with the clip multiplied in.
    fn shape_cov(&self, path: &Path, rule: FillRule, xf: &Affine) -> Coverage {
        let mut c = raster::fill(path, rule, xf, self.gs.clip.as_deref(), self.surf.w, self.surf.h);
        if !self.antialiased() {
            for v in &mut c.a {
                *v = if *v >= 128 { 255 } else { 0 };
            }
        }
        c
    }

    /// Coverage of everything `clip` lets through (the whole visible frame when there is none).
    fn full_cov(&self, clip: &Option<Arc<Coverage>>) -> Coverage {
        match clip {
            Some(c) => (**c).clone(),
            None => {
                let (w, h) = (self.surf.w, self.surf.h - self.surf.y0.min(self.surf.h));
                Coverage { x: 0, y: self.surf.y0 as i32, w, h, a: vec![255; w * h] }
            }
        }
    }

    // ---- clipping -------------------------------------------------------------------------------------

    fn intersect_clip(&mut self, c: Coverage) {
        let merged = match &self.gs.clip {
            Some(old) => intersect(old, &c),
            None => c,
        };
        self.gs.clip = Some(Arc::new(merged));
    }

    pub fn reset_clip(&mut self) {
        self.gs.clip = None;
    }

    pub fn clip_to_rect(&mut self, r: Rect) {
        let ctm = self.gs.ctm;
        let c = self.raw_cov(&rect_path(r), FillRule::NonZero, &ctm);
        self.intersect_clip(c);
    }

    /// Clips to the union of the rectangles.
    pub fn clip_to_rects(&mut self, rects: &[Rect]) {
        let mut p = Path::new();
        for r in rects {
            p.add_rect(prect(*r));
        }
        self.clip_to_path(&p, FillRule::NonZero);
    }

    /// Clips to the interior of `p` (user space).
    pub fn clip_to_path(&mut self, p: &Path, rule: FillRule) {
        let ctm = self.gs.ctm;
        let c = self.raw_cov(p, rule, &ctm);
        self.intersect_clip(c);
    }

    /// Clips to the current path and starts a new one.
    pub fn clip(&mut self) {
        self.clip_current(FillRule::NonZero);
    }

    pub fn eo_clip(&mut self) {
        self.clip_current(FillRule::EvenOdd);
    }

    fn clip_current(&mut self, rule: FillRule) {
        let c = self.raw_cov(&self.path, rule, &Affine::IDENTITY);
        self.intersect_clip(c);
        self.begin_path();
    }

    /// Clips to the alpha of `mask`, stretched over `rect`.
    pub fn clip_to_mask(&mut self, rect: Rect, mask: &Image) {
        if mask.w == 0 || mask.h == 0 || rect.is_empty() {
            self.intersect_clip(Coverage { x: 0, y: 0, w: 0, h: 0, a: Vec::new() });
            return;
        }
        let Some(inv) = self.gs.ctm.invert() else { return };
        let ctm = self.gs.ctm;
        let mut c = self.raw_cov(&rect_path(rect), FillRule::NonZero, &ctm);
        let samp = sampler(mask, inv, rect, self.gs.interp, false);
        for row in 0..c.h {
            for col in 0..c.w {
                let i = row * c.w + col;
                if c.a[i] == 0 {
                    continue;
                }
                let a = samp(c.x + col as i32, c.y + row as i32)[3];
                c.a[i] = (c.a[i] as f32 * a + 0.5) as u8;
            }
        }
        self.intersect_clip(c);
    }

    /// The bounding box of the clip in user space; the whole frame without a clip, empty when nothing is visible.
    pub fn clip_bounding_box(&self) -> Rect {
        let frame = (0.0, self.surf.y0 as f32, self.surf.w as f32, (self.surf.h - self.surf.y0.min(self.surf.h)) as f32);
        let (x, y, w, h) = match &self.gs.clip {
            Some(c) => {
                if c.w == 0 || c.h == 0 {
                    return Rect::ZERO;
                }
                (c.x as f32, c.y as f32, c.w as f32, c.h as f32)
            }
            None => frame,
        };
        self.rect_to_user(Rect::new(x, y, w, h))
    }

    // ---- painting -------------------------------------------------------------------------------------

    /// Paints `cov` with the fill or stroke paint. A solid colour is a constant source; any other paint is
    /// sampled through the inverse of the CTM, since paint coordinates are user space.
    fn paint_cov(&mut self, cov: &Coverage, fill: bool) {
        let paint = if fill { self.gs.fill.clone() } else { self.gs.stroke.clone() };
        if let Paint::Solid(c) = &paint {
            self.composite(cov, &Src::Solid(premul(c)));
            return;
        }
        let Some(inv) = self.gs.ctm.invert() else { return };
        let sample = |x: i32, y: i32| {
            let (ux, uy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
            unpack(paint.shade(ux, uy))
        };
        self.composite(cov, &Src::Sample(&sample));
    }

    /// Writes `src` through `cov` into the target under the current alpha and blend mode, shadow first.
    fn composite(&mut self, cov: &Coverage, src: &Src<'_>) {
        let (sw, sh, top) = (self.surf.w as i32, self.surf.h as i32, self.surf.y0 as i32);
        let x0 = cov.x.max(0);
        let x1 = (cov.x + cov.w as i32).min(sw);
        let y0 = cov.y.max(top);
        let y1 = (cov.y + cov.h as i32).min(sh);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if let Some(sd) = self.gs.shadow.clone() {
            self.draw_shadow(cov, src, &sd);
        }
        let (alpha, mode, stride) = (self.gs.alpha, self.gs.blend, self.surf.stride);
        let mut t = target(&mut self.surf, &mut self.layers);
        for y in y0..y1 {
            let crow = (y - cov.y) as usize * cov.w;
            let prow = (y - top) as usize * stride;
            for x in x0..x1 {
                let c = cov.a[crow + (x - cov.x) as usize] as u32;
                if c == 0 {
                    continue;
                }
                let s = mul4(src.at(x, y), alpha);
                let p = &mut t.px[prow + x as usize];
                *p = blend_into(mode, *p, s, c);
            }
        }
    }

    fn draw_shadow(&mut self, cov: &Coverage, src: &Src<'_>, sd: &Shadow) {
        if cov.w == 0 || cov.h == 0 {
            return;
        }
        let k = self.unit_scale();
        let sigma = sd.blur * k * 0.5;
        let r = if sigma < 0.3 { 0 } else { (sigma * 1.2).round().max(1.0) as usize };
        let pad = 3 * r;
        let (bw, bh) = (cov.w + 2 * pad, cov.h + 2 * pad);
        let mut buf = vec![0.0f32; bw * bh];
        let alpha = self.gs.alpha;
        for row in 0..cov.h {
            for col in 0..cov.w {
                let c = cov.a[row * cov.w + col];
                if c == 0 {
                    continue;
                }
                let a = src.at(cov.x + col as i32, cov.y + row as i32)[3];
                buf[(row + pad) * bw + col + pad] = c as f32 / 255.0 * a * alpha;
            }
        }
        box_blur(&mut buf, bw, bh, r);
        let ct = self.gs.ctm;
        let (ox, oy) = (ct.a * sd.offset.0 + ct.c * sd.offset.1, ct.b * sd.offset.0 + ct.d * sd.offset.1);
        let (dx, dy) = (ox.round() as i32, oy.round() as i32);
        let (left, upper) = (cov.x - pad as i32 + dx, cov.y - pad as i32 + dy);
        let (sw, sh, top, stride) = (self.surf.w as i32, self.surf.h as i32, self.surf.y0 as i32, self.surf.stride);
        let clip = self.gs.clip.clone();
        let [cr, cg, cb, ca] = sd.color.to_srgb().unwrap_or([0.0; 4]);
        let mut t = target(&mut self.surf, &mut self.layers);
        for row in 0..bh {
            let y = upper + row as i32;
            if y < top || y >= sh {
                continue;
            }
            for c in 0..bw {
                let x = left + c as i32;
                let v = buf[row * bw + c];
                if x < 0 || x >= sw || v <= 0.0 {
                    continue;
                }
                let m = clip_at(&clip, x, y) as f32 / 255.0;
                let a = (v * ca * m).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let s = pack_f([cr * a, cg * a, cb * a, a]);
                let p = &mut t.px[(y - top) as usize * stride + x as usize];
                *p = over(*p, s);
            }
        }
    }

    fn fill_path_with(&mut self, path: &Path, rule: FillRule, xf: &Affine) {
        let cov = self.shape_cov(path, rule, xf);
        self.paint_cov(&cov, true);
    }

    fn stroke_user_path(&mut self, user: &Path) {
        let outline = raster::stroke_path(user, &self.stroke_style());
        let ctm = self.gs.ctm;
        let cov = self.shape_cov(&outline, FillRule::NonZero, &ctm);
        self.paint_cov(&cov, false);
    }

    fn fill_current(&mut self, rule: FillRule) {
        let p = self.path.clone();
        self.fill_path_with(&p, rule, &Affine::IDENTITY);
    }

    fn stroke_current(&mut self) {
        let user = self.user_path();
        self.stroke_user_path(&user);
    }

    /// Fills the current path (non-zero rule) and starts a new one.
    pub fn fill_path(&mut self) {
        self.draw_path(PathDrawing::Fill);
    }

    pub fn eo_fill_path(&mut self) {
        self.draw_path(PathDrawing::EoFill);
    }

    pub fn stroke_path(&mut self) {
        self.draw_path(PathDrawing::Stroke);
    }

    pub fn draw_path(&mut self, mode: PathDrawing) {
        match mode {
            PathDrawing::Fill => self.fill_current(FillRule::NonZero),
            PathDrawing::EoFill => self.fill_current(FillRule::EvenOdd),
            PathDrawing::Stroke => self.stroke_current(),
            PathDrawing::FillStroke => {
                self.fill_current(FillRule::NonZero);
                self.stroke_current();
            }
            PathDrawing::EoFillStroke => {
                self.fill_current(FillRule::EvenOdd);
                self.stroke_current();
            }
        }
        self.begin_path();
    }

    /// Fills a user-space path with the fill paint (the current path is left alone).
    pub fn fill(&mut self, p: &Path, rule: FillRule) {
        let ctm = self.gs.ctm;
        self.fill_path_with(p, rule, &ctm);
    }

    /// Strokes a user-space path with the stroke paint and line attributes.
    pub fn stroke(&mut self, p: &Path) {
        self.stroke_user_path(p);
    }

    pub fn fill_rect(&mut self, r: Rect) {
        self.fill(&rect_path(r), FillRule::NonZero);
    }

    pub fn fill_rects(&mut self, rects: &[Rect]) {
        let mut p = Path::new();
        for r in rects {
            p.add_rect(prect(*r));
        }
        self.fill(&p, FillRule::NonZero);
    }

    pub fn fill_ellipse_in_rect(&mut self, r: Rect) {
        self.fill(&ellipse_path(r), FillRule::NonZero);
    }

    pub fn stroke_rect(&mut self, r: Rect) {
        self.stroke(&rect_path(r));
    }

    pub fn stroke_rect_with_width(&mut self, r: Rect, width: f32) {
        let old = self.gs.line_width;
        self.gs.line_width = width.max(0.0);
        self.stroke_rect(r);
        self.gs.line_width = old;
    }

    pub fn stroke_ellipse_in_rect(&mut self, r: Rect) {
        self.stroke(&ellipse_path(r));
    }

    /// Strokes the segments (p0, p1), (p2, p3), ...; an odd trailing point is ignored.
    pub fn stroke_line_segments(&mut self, points: &[(f32, f32)]) {
        let mut p = Path::new();
        for pair in points.chunks_exact(2) {
            p.move_to(pair[0]);
            p.line_to(pair[1]);
        }
        self.stroke(&p);
    }

    /// Erases `r` to transparent (under the clip), whatever blend mode and alpha are set.
    pub fn clear_rect(&mut self, r: Rect) {
        let (blend, alpha, shadow) = (self.gs.blend, self.gs.alpha, self.gs.shadow.clone());
        self.gs.blend = BlendMode::Clear;
        self.gs.alpha = 1.0;
        self.gs.shadow = None;
        self.fill_rect(r);
        self.gs.blend = blend;
        self.gs.alpha = alpha;
        self.gs.shadow = shadow;
    }

    // ---- images ---------------------------------------------------------------------------------------

    /// Draws `img` stretched over `dest`, as sampled by the interpolation quality.
    pub fn draw_image(&mut self, img: &Image, dest: Rect) {
        self.image_impl(img, dest, None, false);
    }

    /// `mask`: paint the image's alpha with this premultiplied colour instead of its own colours.
    /// `exact`: one image pixel per device pixel, so sample without filtering.
    fn image_impl(&mut self, img: &Image, dest: Rect, mask: Option<&Paint>, exact: bool) {
        if img.w == 0 || img.h == 0 || dest.w == 0.0 || dest.h == 0.0 {
            return;
        }
        let Some(inv) = self.gs.ctm.invert() else { return };
        let q = if exact { Interpolation::None } else { self.gs.interp };
        let ct = self.gs.ctm;
        let sx = ct.a.hypot(ct.b) * dest.w.abs() / img.w as f32;
        let sy = ct.c.hypot(ct.d) * dest.h.abs() / img.h as f32;
        let shrunk;
        let source: &Image = if matches!(q, Interpolation::Medium | Interpolation::High) && (sx < 0.5 || sy < 0.5) {
            let (tw, th) = ((img.w as f32 * sx.min(1.0)).ceil() as usize, (img.h as f32 * sy.min(1.0)).ceil() as usize);
            shrunk = shrink(img, tw, th);
            &shrunk
        } else {
            img
        };
        let cov = self.shape_cov(&rect_path(dest), FillRule::NonZero, &ct);
        let samp = sampler(source, inv, dest, q, false);
        match mask {
            None => self.composite(&cov, &Src::Sample(&samp)),
            Some(paint) => {
                let m = move |x: i32, y: i32| {
                    let (ux, uy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
                    mul4(unpack(paint.shade(ux, uy)), samp(x, y)[3])
                };
                self.composite(&cov, &Src::Sample(&m));
            }
        }
    }

    /// Fills the clip with copies of `img`, each stretched over a `tile`-sized cell; the cell at `tile` is the origin.
    pub fn draw_tiled_image(&mut self, img: &Image, tile: Rect) {
        if img.w == 0 || img.h == 0 || tile.w == 0.0 || tile.h == 0.0 {
            return;
        }
        let Some(inv) = self.gs.ctm.invert() else { return };
        let cov = self.full_cov(&self.gs.clip);
        let samp = sampler(img, inv, tile, self.gs.interp, true);
        self.composite(&cov, &Src::Sample(&samp));
    }

    /// Draws the contents of an offscreen bitmap at `p` (its size in user units).
    pub fn draw_layer_at_point(&mut self, layer: &Bitmap, p: (f32, f32)) {
        let img = layer.make_image();
        self.draw_image(&img, Rect::new(p.0, p.1, layer.width() as f32, layer.height() as f32));
    }

    pub fn draw_layer_in_rect(&mut self, layer: &Bitmap, r: Rect) {
        let img = layer.make_image();
        self.draw_image(&img, r);
    }

    // ---- transparency layers --------------------------------------------------------------------------

    /// Draws until `end_transparency_layer` go to an offscreen group that is composited as one object with the
    /// alpha, blend mode and shadow set now (inside the group they start out neutral).
    pub fn begin_transparency_layer(&mut self) {
        self.layers.push(Layer {
            buf: vec![0; self.surf.px.len()],
            alpha: self.gs.alpha,
            blend: self.gs.blend,
            shadow: self.gs.shadow.clone(),
            clip: self.gs.clip.clone(),
        });
        self.gs.alpha = 1.0;
        self.gs.blend = BlendMode::Normal;
        self.gs.shadow = None;
    }

    /// As `begin_transparency_layer`, also clipping the group to `r`.
    pub fn begin_transparency_layer_with_rect(&mut self, r: Rect) {
        self.begin_transparency_layer();
        let ct = self.gs.ctm;
        let mut c = raster::fill(&rect_path(r), FillRule::NonZero, &ct, None, self.surf.w, self.surf.h);
        if let Some(l) = self.layers.last_mut() {
            if let Some(old) = &l.clip {
                c = intersect(old, &c);
            }
            l.clip = Some(Arc::new(c));
        }
    }

    pub fn transparency_layer_depth(&self) -> usize {
        self.layers.len()
    }

    pub fn end_transparency_layer(&mut self) {
        let Some(layer) = self.layers.pop() else { return };
        self.gs.alpha = layer.alpha;
        self.gs.blend = layer.blend;
        self.gs.shadow = layer.shadow;
        let cov = self.full_cov(&layer.clip);
        let (stride, y0, len) = (self.surf.stride, self.surf.y0, layer.buf.len());
        let buf = &layer.buf;
        let samp = move |x: i32, y: i32| {
            if x < 0 || y < y0 as i32 || (x as usize) >= stride {
                return [0.0; 4];
            }
            let i = (y as usize - y0) * stride + x as usize;
            if i < len {
                unpack(buf[i])
            } else {
                [0.0; 4]
            }
        };
        self.composite(&cov, &Src::Sample(&samp));
    }

    // ---- text -----------------------------------------------------------------------------------------

    pub fn set_font_size(&mut self, size: f32) {
        self.gs.font_size = size.max(0.0);
    }

    pub fn set_character_spacing(&mut self, s: f32) {
        self.gs.char_spacing = s;
    }

    pub fn set_text_drawing_mode(&mut self, m: TextDrawing) {
        self.gs.text_mode = m;
    }

    pub fn set_text_matrix(&mut self, m: Affine) {
        self.tm = m;
    }

    pub fn text_matrix(&self) -> Affine {
        self.tm
    }

    pub fn set_text_position(&mut self, x: f32, y: f32) {
        self.tm.tx = x;
        self.tm.ty = y;
    }

    pub fn text_position(&self) -> (f32, f32) {
        (self.tm.tx, self.tm.ty)
    }

    /// Draws a glyph-coverage image whose pen origin is (x, y) in the current space; `px` is the rendered size.
    fn blit_text_image(&mut self, img: &Image, font: &Font<'_>, px: f32, x: f32, y: f32) {
        // the glyph renderer pads every side by this many pixels and puts the baseline below the ascent
        const PAD: f32 = 2.0;
        let k = self.unit_scale();
        if k <= 0.0 || img.w == 0 || img.h == 0 {
            return;
        }
        let baseline = PAD + font.ascent * px / font.upem;
        let (mut left, mut top) = (x - PAD / k, y - baseline / k);
        let c = self.gs.ctm;
        let aligned = c.b.abs() < 1e-4 && c.c.abs() < 1e-4 && c.a > 0.0 && c.d > 0.0 && (c.a - c.d).abs() < 1e-3 * c.a;
        if aligned {
            let (dx, dy) = c.apply(left, top);
            left += (dx.round() - dx) / c.a;
            top += (dy.round() - dy) / c.d;
        }
        let dest = Rect::new(left, top, img.w as f32 / k, img.h as f32 / k);
        let paint = self.gs.fill.clone();
        self.image_impl(img, dest, Some(&paint), aligned);
    }

    fn glyph_style(&self, px: f32) -> Style {
        Style { px, embolden: 0.0, color: 0xffff_ffff, shadow: false }
    }

    /// Draws `text` with its pen origin at (x, y) in the current space; returns the advance in the same units.
    fn draw_run(&mut self, font: &Font<'_>, text: &str, size: f32, x: f32, y: f32) -> f32 {
        let adv = font.measure(text, size);
        let px = size * self.unit_scale();
        if px < 0.5 || text.is_empty() || self.gs.text_mode == TextDrawing::Invisible {
            return adv;
        }
        let img = font.render(text, &self.glyph_style(px));
        self.blit_text_image(&img, font, px, x, y);
        adv
    }

    /// Draws `text` at the text position in the text matrix's space and moves the position past it.
    pub fn show_text(&mut self, font: &Font<'_>, text: &str) {
        let size = self.gs.font_size;
        let spacing = self.gs.char_spacing;
        let saved = self.gs.ctm;
        self.gs.ctm = self.tm.then(saved);
        let mut adv = 0.0;
        if spacing == 0.0 {
            adv = self.draw_run(font, text, size, 0.0, 0.0);
        } else {
            for ch in text.chars() {
                let mut buf = [0u8; 4];
                let s: &str = ch.encode_utf8(&mut buf);
                adv += self.draw_run(font, s, size, adv, 0.0) + spacing;
            }
        }
        self.gs.ctm = saved;
        self.tm.tx += adv * self.tm.a;
        self.tm.ty += adv * self.tm.b;
    }

    pub fn show_text_at_point(&mut self, font: &Font<'_>, x: f32, y: f32, text: &str) {
        self.set_text_position(x, y);
        self.show_text(font, text);
    }

    /// Draws each glyph with its pen origin at the given user-space position.
    pub fn show_glyphs_at_positions(&mut self, font: &Font<'_>, glyphs: &[(u16, (f32, f32))]) {
        let size = self.gs.font_size;
        let px = size * self.unit_scale();
        if px < 0.5 || self.gs.text_mode == TextDrawing::Invisible {
            return;
        }
        let st = self.glyph_style(px);
        for &(gid, (x, y)) in glyphs {
            let img = font.render_glyph(gid, &st);
            self.blit_text_image(&img, font, px, x, y);
        }
    }

    /// Draws consecutive glyphs from the text position, advancing by `advances` (user units) or, when `None`, by the
    /// font's own advance widths.
    pub fn show_glyphs_with_advances(&mut self, font: &Font<'_>, gids: &[u16], advances: Option<&[f32]>) {
        let size = self.gs.font_size;
        let mut pos = Vec::with_capacity(gids.len());
        let mut pen = 0.0f32;
        for (i, &g) in gids.iter().enumerate() {
            let (x, y) = self.tm.apply(pen, 0.0);
            pos.push((g, (x, y)));
            pen += advances.and_then(|a| a.get(i).copied()).unwrap_or_else(|| font.advance(g) * size / font.upem) + self.gs.char_spacing;
        }
        self.show_glyphs_at_positions(font, &pos);
        self.tm.tx += pen * self.tm.a;
        self.tm.ty += pen * self.tm.b;
    }

    pub fn show_glyphs(&mut self, font: &Font<'_>, gids: &[u16]) {
        self.show_glyphs_with_advances(font, gids, None);
    }
}

// ---------------------------------------------------------------------------------------------------------

/// An owned pixel buffer with a row stride that a context can draw into (premultiplied, red in the low byte).
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    w: usize,
    h: usize,
    /// pixels per row
    stride: usize,
    px: Vec<u32>,
}

impl Bitmap {
    /// A transparent bitmap with tightly packed rows.
    pub fn new(w: usize, h: usize) -> Self {
        Bitmap { w, h, stride: w, px: vec![0; w * h] }
    }

    /// A transparent bitmap whose rows are `bytes_per_row` apart; `None` unless that is a multiple of 4 and holds a row.
    pub fn with_bytes_per_row(w: usize, h: usize, bytes_per_row: usize) -> Option<Self> {
        if bytes_per_row % 4 != 0 || bytes_per_row < w * 4 {
            return None;
        }
        let stride = bytes_per_row / 4;
        Some(Bitmap { w, h, stride, px: vec![0; stride * h] })
    }

    /// Adopts `data` (RGBA bytes, premultiplied) as the initial contents; `None` when it is too short or misshapen.
    pub fn from_bytes(w: usize, h: usize, bytes_per_row: usize, data: &[u8]) -> Option<Self> {
        let mut b = Self::with_bytes_per_row(w, h, bytes_per_row)?;
        if data.len() < bytes_per_row * h {
            return None;
        }
        for (dst, src) in b.px.iter_mut().zip(data.chunks_exact(4)) {
            *dst = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
        }
        Some(b)
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn bytes_per_row(&self) -> usize {
        self.stride * 4
    }

    pub fn pixels(&self) -> &[u32] {
        &self.px
    }

    /// The pixel data as bytes, `bytes_per_row` per row, R G B A.
    pub fn data(&self) -> Vec<u8> {
        self.px.iter().flat_map(|p| p.to_le_bytes()).collect()
    }

    /// A drawing context over the bitmap.
    pub fn context(&mut self) -> DrawContext<'_> {
        let (w, h, stride) = (self.w, self.h, self.stride);
        DrawContext::over_pixels(w, h, stride, &mut self.px[..], 0)
    }

    /// A copy of the current contents as an image (tight rows).
    pub fn make_image(&self) -> Image {
        let mut img = Image::new(self.w, self.h);
        for y in 0..self.h {
            img.px[y * self.w..(y + 1) * self.w].copy_from_slice(&self.px[y * self.stride..y * self.stride + self.w]);
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn red() -> Color {
        Color::srgb(1.0, 0.0, 0.0, 1.0)
    }

    #[test]
    fn state_stack_restores_attributes_and_ignores_extra_restores() {
        let mut bm = Bitmap::new(4, 4);
        let mut cx = bm.context();
        cx.set_line_width(5.0);
        cx.set_alpha(0.5);
        cx.save_gstate();
        cx.set_line_width(1.0);
        cx.set_alpha(0.25);
        cx.translate_ctm(3.0, 4.0);
        assert_eq!(cx.saved_depth(), 1);
        cx.restore_gstate();
        assert_eq!((cx.line_width(), cx.alpha()), (5.0, 0.5));
        assert_eq!(cx.ctm(), Affine::IDENTITY);
        cx.restore_gstate();
        assert_eq!(cx.saved_depth(), 0);
    }

    #[test]
    fn ctm_operations_compose_in_cg_order() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.translate_ctm(10.0, 0.0);
        cx.scale_ctm(2.0, 3.0);
        // user (1, 1): scaled to (2, 3) first, then moved by 10
        assert_eq!(cx.point_to_device((1.0, 1.0)), (12.0, 3.0));
        let back = cx.point_to_user((12.0, 3.0));
        assert!(close(back.0, 1.0) && close(back.1, 1.0));
        assert_eq!(cx.size_to_device((1.0, 1.0)), (2.0, 3.0));
        let r = cx.rect_to_device(Rect::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(r, Rect::new(10.0, 0.0, 2.0, 3.0));
        let u = cx.rect_to_user(r);
        assert!(close(u.w, 1.0) && close(u.h, 1.0));
    }

    #[test]
    fn rotated_rect_conversion_takes_the_bounding_box() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.rotate_ctm(std::f32::consts::FRAC_PI_2);
        let r = cx.rect_to_device(Rect::new(0.0, 0.0, 2.0, 1.0));
        assert!(close(r.w, 1.0) && close(r.h, 2.0));
    }

    #[test]
    fn clip_bounding_box_without_clip_is_the_frame_in_user_space() {
        let mut bm = Bitmap::new(8, 6);
        let mut cx = bm.context();
        assert_eq!(cx.clip_bounding_box(), Rect::new(0.0, 0.0, 8.0, 6.0));
        cx.scale_ctm(2.0, 2.0);
        assert_eq!(cx.clip_bounding_box(), Rect::new(0.0, 0.0, 4.0, 3.0));
    }

    #[test]
    fn shadow_setters_and_clearing() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.set_shadow((1.0, 2.0), 3.0);
        let s = cx.shadow().unwrap();
        assert_eq!((s.offset, s.blur), ((1.0, 2.0), 3.0));
        assert!(close(s.color.alpha(), 1.0 / 3.0));
        cx.set_shadow_with_color((0.0, 0.0), 1.0, None);
        assert!(cx.shadow().is_none());
    }

    #[test]
    fn dash_with_no_positive_length_is_solid() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.set_line_dash(0.0, &[0.0, 0.0]);
        assert!(cx.gs.dash.is_empty());
        cx.set_line_dash(1.0, &[3.0, 2.0]);
        assert_eq!(cx.gs.dash, vec![3.0, 2.0]);
    }

    #[test]
    fn alpha_is_clamped() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.set_alpha(3.0);
        assert_eq!(cx.alpha(), 1.0);
        cx.set_alpha(-1.0);
        assert_eq!(cx.alpha(), 0.0);
    }

    #[test]
    fn bitmap_geometry_and_bytes() {
        assert!(Bitmap::with_bytes_per_row(4, 2, 15).is_none());
        assert!(Bitmap::with_bytes_per_row(4, 2, 8).is_none());
        let mut b = Bitmap::with_bytes_per_row(2, 2, 12).unwrap();
        assert_eq!((b.width(), b.height(), b.bytes_per_row()), (2, 2, 12));
        assert_eq!(b.data().len(), 24);
        b.px[3] = 0xff00_00ff; // second row, first pixel
        let img = b.make_image();
        assert_eq!(img.px, vec![0, 0, 0xff00_00ff, 0]);
        let again = Bitmap::from_bytes(2, 2, 12, &b.data()).unwrap();
        assert_eq!(again, b);
        assert!(Bitmap::from_bytes(2, 2, 12, &[0; 10]).is_none());
    }

    #[test]
    fn plus_lighter_adds_and_plus_darker_subtracts() {
        let s = [0.5, 0.25, 0.0, 1.0];
        let d = [0.25, 0.25, 0.5, 1.0];
        let l = blend_pixel(BlendMode::PlusLighter, s, d);
        assert!(close(l[0], 0.75) && close(l[1], 0.5) && close(l[2], 0.5) && close(l[3], 1.0));
        let k = blend_pixel(BlendMode::PlusDarker, s, d);
        assert!(close(k[0], 0.0) && close(k[1], 0.0) && close(k[2], 0.0) && close(k[3], 1.0));
    }

    #[test]
    fn porter_duff_operators_follow_their_definitions() {
        let s = [0.5, 0.0, 0.0, 0.5]; // red at half alpha, premultiplied
        let d = [0.0, 0.25, 0.0, 0.25];
        assert_eq!(blend_pixel(BlendMode::Clear, s, d), [0.0; 4]);
        assert_eq!(blend_pixel(BlendMode::Copy, s, d), s);
        assert!(close(blend_pixel(BlendMode::SourceIn, s, d)[3], 0.125));
        assert!(close(blend_pixel(BlendMode::SourceOut, s, d)[3], 0.375));
        assert!(close(blend_pixel(BlendMode::SourceAtop, s, d)[3], 0.25), "atop keeps the backdrop alpha");
        assert!(close(blend_pixel(BlendMode::Xor, s, d)[3], 0.5 * 0.75 + 0.25 * 0.5));
        assert!(close(blend_pixel(BlendMode::DestinationOut, s, d)[3], 0.125));
        assert!(close(blend_pixel(BlendMode::DestinationOver, s, d)[3], 0.25 + 0.5 * 0.75));
    }

    #[test]
    fn separable_modes_match_the_blend_module() {
        // opaque grey over opaque white under multiply is the grey
        let out = blend_pixel(BlendMode::Multiply, [0.5, 0.5, 0.5, 1.0], [1.0, 1.0, 1.0, 1.0]);
        assert!(close(out[0], 0.5) && close(out[3], 1.0));
        // normal at half alpha over white
        let n = blend_pixel(BlendMode::Normal, [0.0, 0.0, 0.0, 0.5], [1.0, 1.0, 1.0, 1.0]);
        assert!(close(n[0], 0.5));
    }

    #[test]
    fn non_separable_modes_keep_the_requested_component() {
        let src = [0.8, 0.2, 0.2, 1.0];
        let dst = [0.2, 0.4, 0.8, 1.0];
        let lumi = |c: [f32; 4]| lum([c[0], c[1], c[2]]);
        let l = blend_pixel(BlendMode::Luminosity, src, dst);
        assert!((lumi(l) - lumi(src)).abs() < 1e-3, "luminosity takes the source luma");
        let c = blend_pixel(BlendMode::Color, src, dst);
        assert!((lumi(c) - lumi(dst)).abs() < 1e-3, "color keeps the backdrop luma");
        let h = blend_pixel(BlendMode::Hue, src, dst);
        assert!((lumi(h) - lumi(dst)).abs() < 1e-3);
        let s = blend_pixel(BlendMode::Saturation, src, dst);
        assert!((lumi(s) - lumi(dst)).abs() < 1e-3);
    }

    #[test]
    fn transparent_source_leaves_the_backdrop_in_general_modes() {
        let d = [0.1, 0.2, 0.3, 0.5];
        assert_eq!(blend_pixel(BlendMode::Overlay, [0.0; 4], d), d);
    }

    #[test]
    fn intersect_multiplies_over_the_overlap() {
        let a = Coverage { x: 0, y: 0, w: 4, h: 1, a: vec![255, 255, 128, 0] };
        let b = Coverage { x: 2, y: 0, w: 4, h: 1, a: vec![255, 128, 255, 255] };
        let c = intersect(&a, &b);
        assert_eq!((c.x, c.y, c.w, c.h), (2, 0, 2, 1));
        assert_eq!(c.a, vec![128, 0]);
        let far = Coverage { x: 50, y: 50, w: 1, h: 1, a: vec![255] };
        let none = intersect(&a, &far);
        assert_eq!((none.w, none.h), (0, 0));
    }

    #[test]
    fn box_blur_spreads_and_conserves_mass() {
        let (w, h) = (15, 15);
        let mut buf = vec![0.0f32; w * h];
        buf[7 * w + 7] = 1.0;
        box_blur(&mut buf, w, h, 2);
        let sum: f32 = buf.iter().sum();
        assert!((sum - 1.0).abs() < 1e-3);
        assert!(buf[7 * w + 7] < 0.2 && buf[7 * w + 7] > buf[7 * w + 10]);
        let mut same = vec![1.0f32; 9];
        box_blur(&mut same, 3, 3, 0);
        assert_eq!(same, vec![1.0; 9]);
    }

    #[test]
    fn shrink_halves_with_a_box_filter() {
        let mut img = Image::new(4, 4);
        for (i, p) in img.px.iter_mut().enumerate() {
            *p = if i % 2 == 0 { 0xffff_ffff } else { 0xff00_0000 };
        }
        let s = shrink(&img, 2, 2);
        assert_eq!((s.w, s.h), (2, 2));
        let mid = s.px[0] & 0xff;
        assert!((126..=129).contains(&mid), "average of white and black: {mid}");
        assert_eq!(shrink(&img, 4, 4).w, 4, "no shrink when the target is not smaller");
    }

    #[test]
    fn sampler_nearest_and_bilinear_and_wrap() {
        let img = Image { w: 2, h: 1, px: vec![0xff00_00ff, 0xff00_ff00] }; // red, green
        let dest = Rect::new(0.0, 0.0, 2.0, 1.0);
        let near = sampler(&img, Affine::IDENTITY, dest, Interpolation::None, false);
        assert_eq!(near(0, 0)[0], 1.0);
        assert_eq!(near(1, 0)[1], 1.0);
        let lin = sampler(&img, Affine::IDENTITY, dest, Interpolation::Low, false);
        assert!(close(lin(1, 0)[1], 1.0), "pixel centre 1.5 sits on the second texel");
        let wrap = sampler(&img, Affine::IDENTITY, dest, Interpolation::None, true);
        assert_eq!(wrap(2, 0)[0], 1.0, "tile repeats");
        assert_eq!(wrap(-1, 0)[1], 1.0);
    }

    #[test]
    fn cmyk_black_and_white() {
        assert_eq!(cmyk(0.0, 0.0, 0.0, 1.0, 1.0), Color::srgb(0.0, 0.0, 0.0, 1.0));
        assert_eq!(cmyk(0.0, 0.0, 0.0, 0.0, 0.5), Color::srgb(1.0, 1.0, 1.0, 0.5));
    }

    #[test]
    fn ending_a_layer_that_was_never_begun_is_a_no_op() {
        let mut bm = Bitmap::new(2, 2);
        let mut cx = bm.context();
        cx.end_transparency_layer();
        assert_eq!(cx.transparency_layer_depth(), 0);
        cx.set_alpha(0.5);
        cx.begin_transparency_layer();
        assert_eq!(cx.transparency_layer_depth(), 1);
        assert_eq!(cx.alpha(), 1.0, "inside the group the alpha starts neutral");
    }

    // The tests below drive the scan converter in `cg::raster`.

    fn px(bm: &Bitmap, x: usize, y: usize) -> u32 {
        bm.pixels()[y * bm.bytes_per_row() / 4 + x]
    }

    #[test]
    fn fill_rect_paints_inside_only() {
        let mut bm = Bitmap::new(8, 8);
        {
            let mut cx = bm.context();
            cx.set_fill_color(red());
            cx.fill_rect(Rect::new(2.0, 2.0, 4.0, 4.0));
        }
        assert_eq!(px(&bm, 3, 3), 0xff00_00ff);
        assert_eq!(px(&bm, 0, 0), 0);
        assert_eq!(px(&bm, 6, 6), 0);
    }

    #[test]
    fn clip_confines_fills_and_nested_clips_intersect() {
        let mut bm = Bitmap::new(8, 8);
        {
            let mut cx = bm.context();
            cx.set_fill_color(red());
            cx.save_gstate();
            cx.clip_to_rect(Rect::new(0.0, 0.0, 4.0, 8.0));
            cx.clip_to_rect(Rect::new(2.0, 0.0, 6.0, 8.0));
            cx.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0));
            let b = cx.clip_bounding_box();
            assert!(close(b.x, 2.0) && close(b.w, 2.0));
            cx.restore_gstate();
            assert_eq!(cx.clip_bounding_box(), Rect::new(0.0, 0.0, 8.0, 8.0));
        }
        assert_eq!(px(&bm, 1, 1), 0);
        assert_eq!(px(&bm, 3, 1), 0xff00_00ff);
        assert_eq!(px(&bm, 5, 1), 0);
    }

    #[test]
    fn clear_rect_erases() {
        let mut bm = Bitmap::new(4, 4);
        {
            let mut cx = bm.context();
            cx.set_fill_color(red());
            cx.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0));
            cx.clear_rect(Rect::new(1.0, 1.0, 2.0, 2.0));
        }
        assert_eq!(px(&bm, 1, 1), 0);
        assert_eq!(px(&bm, 0, 0), 0xff00_00ff);
    }

    #[test]
    fn half_alpha_and_transparency_layer() {
        let mut bm = Bitmap::new(4, 4);
        {
            let mut cx = bm.context();
            cx.set_alpha(0.5);
            cx.begin_transparency_layer();
            cx.set_fill_color(red());
            cx.fill_rect(Rect::new(0.0, 0.0, 3.0, 3.0));
            cx.fill_rect(Rect::new(1.0, 1.0, 3.0, 3.0));
            cx.end_transparency_layer();
        }
        let a = (px(&bm, 2, 2) >> 24) as i32;
        assert!((a - 128).abs() <= 1, "overlap is one group at half alpha, not two: {a}");
    }

    #[test]
    fn image_draw_scales_and_tiles() {
        let img = Image { w: 1, h: 1, px: vec![0xff00_ff00] };
        let mut bm = Bitmap::new(4, 4);
        {
            let mut cx = bm.context();
            cx.draw_image(&img, Rect::new(0.0, 0.0, 2.0, 2.0));
            cx.draw_tiled_image(&img, Rect::new(0.0, 0.0, 1.0, 1.0));
        }
        assert_eq!(px(&bm, 1, 1), 0xff00_ff00);
        assert_eq!(px(&bm, 3, 3), 0xff00_ff00);
    }

    #[test]
    fn shadow_lands_offset_from_the_shape() {
        let mut bm = Bitmap::new(16, 16);
        {
            let mut cx = bm.context();
            cx.set_shadow_with_color((6.0, 0.0), 0.0, Some(Color::srgb(0.0, 0.0, 1.0, 1.0)));
            cx.set_fill_color(red());
            cx.fill_rect(Rect::new(2.0, 2.0, 4.0, 4.0));
        }
        assert_eq!(px(&bm, 3, 3), 0xff00_00ff);
        assert_ne!(px(&bm, 9, 3) >> 16 & 0xff, 0, "blue shadow six pixels right");
    }
}
