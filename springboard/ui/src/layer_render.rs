//! CPU rendering of a styled layer: background, corner radius (circular or continuous curve) with masked corners,
//! border, shadow (colour, opacity, offset, blur, optional shadow path), a mask, layer filters (Gaussian blur, colour
//! matrix, background blur) and blend modes, plus the compositing of a tree of layers with 3D transforms, a
//! sublayer transform, perspective and "transform layers" that keep their children in 3D.
//! Pixels are premultiplied RGBA packed as 0xAABBGGRR (red in the low byte), the same layout as the renderer's
//! images, so a [`Pixmap`] converts to an image with no per-pixel work. Pure std code.
use crate::layer_style::{corner_radii, CornerMask, Shadow};
use crate::transform3d::Transform3D;

/// A premultiplied RGBA pixel buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixmap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

#[inline]
fn unpack(p: u32) -> [f32; 4] {
    [(p & 255) as f32 / 255.0, ((p >> 8) & 255) as f32 / 255.0, ((p >> 16) & 255) as f32 / 255.0, (p >> 24) as f32 / 255.0]
}

#[inline]
fn pack(c: [f32; 4]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(c[0]) | q(c[1]) << 8 | q(c[2]) << 16 | q(c[3]) << 24
}

/// A straight-alpha colour `0xAABBGGRR` to a premultiplied pixel scaled by `k` (0..1).
fn premul(c: u32, k: f32) -> [f32; 4] {
    let s = unpack(c);
    let a = s[3] * k.clamp(0.0, 1.0);
    [s[0] * a, s[1] * a, s[2] * a, a]
}

impl Pixmap {
    /// A transparent buffer. Zero sizes give an empty buffer.
    pub fn new(w: usize, h: usize) -> Pixmap {
        Pixmap { w, h, px: vec![0; w * h] }
    }

    /// A buffer filled with one premultiplied pixel.
    pub fn filled(w: usize, h: usize, px: u32) -> Pixmap {
        Pixmap { w, h, px: vec![px; w * h] }
    }

    /// The pixel at `(x, y)`; outside the buffer it is transparent.
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0
        } else {
            self.px[y as usize * self.w + x as usize]
        }
    }

    /// The copy of the rectangle at `(x, y)` size `w` by `h`; pixels outside the buffer are transparent.
    pub fn crop(&self, x: i32, y: i32, w: usize, h: usize) -> Pixmap {
        let mut out = Pixmap::new(w, h);
        for j in 0..h {
            for i in 0..w {
                out.px[j * w + i] = self.get(x + i as i32, y + j as i32);
            }
        }
        out
    }

    fn alpha_plane(&self) -> Vec<f32> {
        self.px.iter().map(|p| (p >> 24) as f32 / 255.0).collect()
    }
}

/// How a layer's pixels combine with what is under them.
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
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    /// Adds the colours, clamped.
    PlusLighter,
    /// Adds the colours and subtracts from white, clamped.
    PlusDarker,
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        b * 2.0 * s
    } else {
        let t = 2.0 * s - 1.0;
        b + t - b * t
    }
}

fn blend_channel(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Normal | BlendMode::PlusLighter | BlendMode::PlusDarker => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => b + s - b * s,
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => {
            if b == 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
    }
}

/// One premultiplied pixel `src` (scaled by `opacity`) over `dst` under `mode`.
pub fn blend_pixel(dst: u32, src: u32, mode: BlendMode, opacity: f32) -> u32 {
    let k = opacity.clamp(0.0, 1.0);
    let s = unpack(src).map(|v| v * k);
    let d = unpack(dst);
    if s[3] <= 0.0 {
        return dst;
    }
    match mode {
        BlendMode::PlusLighter => return pack([d[0] + s[0], d[1] + s[1], d[2] + s[2], d[3] + s[3]]),
        BlendMode::PlusDarker => {
            let f = |dc: f32, sc: f32| (dc + sc - 1.0).max(0.0);
            let a = (d[3] + s[3]).min(1.0);
            return pack([f(d[0], s[0]).min(a), f(d[1], s[1]).min(a), f(d[2], s[2]).min(a), a]);
        }
        _ => {}
    }
    let (sa, da) = (s[3], d[3]);
    let ra = sa + da - sa * da;
    if mode == BlendMode::Normal {
        return pack([s[0] + d[0] * (1.0 - sa), s[1] + d[1] * (1.0 - sa), s[2] + d[2] * (1.0 - sa), ra]);
    }
    let mut out = [0.0f32; 4];
    for c in 0..3 {
        let cs = s[c] / sa;
        let cb = if da > 0.0 { d[c] / da } else { 0.0 };
        out[c] = (1.0 - sa) * d[c] + (1.0 - da) * s[c] + sa * da * blend_channel(mode, cb, cs);
    }
    out[3] = ra;
    pack(out)
}

/// Composites `src` onto `dst` with its top-left at `(ox, oy)`.
pub fn composite_onto(dst: &mut Pixmap, src: &Pixmap, ox: i32, oy: i32, mode: BlendMode, opacity: f32) {
    for j in 0..src.h as i32 {
        let y = oy + j;
        if y < 0 || y as usize >= dst.h {
            continue;
        }
        for i in 0..src.w as i32 {
            let x = ox + i;
            if x < 0 || x as usize >= dst.w {
                continue;
            }
            let s = src.px[j as usize * src.w + i as usize];
            if s >> 24 == 0 {
                continue;
            }
            let di = y as usize * dst.w + x as usize;
            dst.px[di] = blend_pixel(dst.px[di], s, mode, opacity);
        }
    }
}

// ---- shapes ----

/// The shape of the corner curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerCurve {
    /// A quarter circle.
    Circular,
    /// A smoother, squircle-like curve with a gentler start and end.
    Continuous,
}

/// Coverage (0..1) of the pixel centred at `(px, py)` inside a `w` by `h` box with the given per-corner radii (order
/// min-min, max-min, min-max, max-max), with about one pixel of antialiasing.
fn shape_coverage(px: f32, py: f32, w: f32, h: f32, r: [f32; 4], curve: CornerCurve) -> f32 {
    let (right, bottom) = (px > w / 2.0, py > h / 2.0);
    let rad = r[(bottom as usize) * 2 + right as usize];
    let dx = if right { px - (w - rad) } else { rad - px };
    let dy = if bottom { py - (h - rad) } else { rad - py };
    let edge = (px.min(w - px)).min(py.min(h - py));
    let d = if rad > 0.0 && dx > 0.0 && dy > 0.0 {
        match curve {
            CornerCurve::Circular => (dx * dx + dy * dy).sqrt() - rad,
            CornerCurve::Continuous => {
                // Superellipse of exponent 5, with the implicit value scaled back to a distance.
                let (u, v) = (dx / rad, dy / rad);
                ((u.powi(5) + v.powi(5)).powf(0.2) - 1.0) * rad
            }
        }
    } else {
        -edge
    };
    (0.5 - d).clamp(0.0, 1.0)
}

/// An outline for a shadow, in layer coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum ShadowPath {
    /// A rectangle `(x, y, w, h)` with one corner radius.
    RoundedRect { rect: (f32, f32, f32, f32), radius: f32 },
    /// Closed polygons filled with the even-odd rule (several give holes).
    Polygons(Vec<Vec<(f32, f32)>>),
}

fn polygon_inside(polys: &[Vec<(f32, f32)>], x: f32, y: f32) -> bool {
    let mut inside = false;
    for poly in polys {
        let n = poly.len();
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
                inside = !inside;
            }
        }
    }
    inside
}

impl ShadowPath {
    fn coverage(&self, x: f32, y: f32) -> f32 {
        match self {
            ShadowPath::RoundedRect { rect, radius } => {
                let r = crate::layer_style::effective_radius(*radius, rect.2, rect.3);
                shape_coverage(x - rect.0, y - rect.1, rect.2, rect.3, [r; 4], CornerCurve::Circular)
            }
            ShadowPath::Polygons(p) => {
                // 3x3 supersampling.
                let mut n = 0;
                for j in 0..3 {
                    for i in 0..3 {
                        if polygon_inside(p, x - 0.5 + (i as f32 + 0.5) / 3.0, y - 0.5 + (j as f32 + 0.5) / 3.0) {
                            n += 1;
                        }
                    }
                }
                n as f32 / 9.0
            }
        }
    }
}

// ---- filters ----

/// A 4x5 colour matrix on straight-alpha channels in 0..1: row r gives `out_r = m[r][0]*R + m[r][1]*G + m[r][2]*B
/// + m[r][3]*A + m[r][4]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorMatrix(pub [[f32; 5]; 4]);

impl ColorMatrix {
    pub const IDENTITY: ColorMatrix = ColorMatrix([
        [1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
    ]);

    /// Saturation: 0 is grey, 1 is unchanged, above 1 is oversaturated.
    pub fn saturation(s: f32) -> ColorMatrix {
        let (r, g, b) = (0.2126, 0.7152, 0.0722);
        let i = 1.0 - s;
        ColorMatrix([
            [r * i + s, g * i, b * i, 0.0, 0.0],
            [r * i, g * i + s, b * i, 0.0, 0.0],
            [r * i, g * i, b * i + s, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ])
    }

    /// Brightness added to every colour channel.
    pub fn brightness(delta: f32) -> ColorMatrix {
        let mut m = Self::IDENTITY;
        for r in 0..3 {
            m.0[r][4] = delta;
        }
        m
    }

    /// Inverts the colour channels.
    pub fn invert() -> ColorMatrix {
        let mut m = Self::IDENTITY;
        for r in 0..3 {
            m.0[r][r] = -1.0;
            m.0[r][4] = 1.0;
        }
        m
    }

    /// Applies the matrix to one premultiplied pixel.
    pub fn apply(&self, p: u32) -> u32 {
        let c = unpack(p);
        let a = c[3];
        let s = if a > 0.0 { [c[0] / a, c[1] / a, c[2] / a, a] } else { [0.0, 0.0, 0.0, 0.0] };
        let mut o = [0.0f32; 4];
        for (r, row) in self.0.iter().enumerate() {
            o[r] = (row[0] * s[0] + row[1] * s[1] + row[2] * s[2] + row[3] * s[3] + row[4]).clamp(0.0, 1.0);
        }
        pack([o[0] * o[3], o[1] * o[3], o[2] * o[3], o[3]])
    }
}

/// A filter on a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerFilter {
    /// Gaussian blur of the layer's pixels; the value is the standard deviation in pixels.
    GaussianBlur(f32),
    /// A colour matrix over the layer's pixels.
    ColorMatrix(ColorMatrix),
    /// Blurs what is behind the layer (its backdrop) inside the layer's shape, before the layer is drawn.
    BackgroundBlur(f32),
    /// A colour matrix over the backdrop inside the layer's shape.
    BackgroundColorMatrix(ColorMatrix),
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let r = (sigma * 3.0).ceil().max(1.0) as i32;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    k
}

fn blur_plane(src: &[f32], w: usize, h: usize, k: &[f32]) -> Vec<f32> {
    let r = (k.len() / 2) as i32;
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sx = x as i32 + i as i32 - r;
                if sx >= 0 && (sx as usize) < w {
                    acc += kv * src[y * w + sx as usize];
                }
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sy = y as i32 + i as i32 - r;
                if sy >= 0 && (sy as usize) < h {
                    acc += kv * tmp[sy as usize * w + x];
                }
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Gaussian blur of all four premultiplied channels. A sigma that is not positive and finite leaves it unchanged.
pub fn gaussian_blur(p: &Pixmap, sigma: f32) -> Pixmap {
    if !(sigma > 0.0 && sigma.is_finite()) || p.w == 0 || p.h == 0 {
        return p.clone();
    }
    let k = gaussian_kernel(sigma);
    let mut chans: Vec<Vec<f32>> = (0..4).map(|c| p.px.iter().map(|v| unpack(*v)[c]).collect()).collect();
    for ch in chans.iter_mut() {
        *ch = blur_plane(ch, p.w, p.h, &k);
    }
    let px = (0..p.w * p.h).map(|i| pack([chans[0][i], chans[1][i], chans[2][i], chans[3][i]])).collect();
    Pixmap { w: p.w, h: p.h, px }
}

// ---- layer style ----

/// Everything that decides how one layer looks, apart from its content pixels and its place in a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStyle {
    /// Fill behind the content, straight alpha `0xAABBGGRR`; 0 for none.
    pub background: u32,
    pub corner_radius: f32,
    pub masked_corners: CornerMask,
    pub corner_curve: CornerCurve,
    /// Clip the content (and children) to the rounded shape.
    pub masks_to_bounds: bool,
    pub border_width: f32,
    /// Border colour, straight alpha `0xAABBGGRR`.
    pub border_color: u32,
    pub shadow: Shadow,
    /// Shadow colour, straight alpha `0xAABBGGRR` (opacity is in [`Shadow`]).
    pub shadow_color: u32,
    /// When set the shadow follows this path instead of the layer's pixels.
    pub shadow_path: Option<ShadowPath>,
    /// A mask the same size as the layer: the layer shows where the mask has alpha.
    pub mask: Option<Pixmap>,
    pub filters: Vec<LayerFilter>,
    pub blend: BlendMode,
    /// Layer opacity, 0..1.
    pub opacity: f32,
}

impl Default for LayerStyle {
    fn default() -> Self {
        LayerStyle {
            background: 0,
            corner_radius: 0.0,
            masked_corners: CornerMask::ALL,
            corner_curve: CornerCurve::Circular,
            masks_to_bounds: false,
            border_width: 0.0,
            border_color: 0xFF00_0000,
            shadow: Shadow::default(),
            shadow_color: 0xFF00_0000,
            shadow_path: None,
            mask: None,
            filters: Vec::new(),
            blend: BlendMode::Normal,
            opacity: 1.0,
        }
    }
}

impl LayerStyle {
    /// Whole pixels the styled output grows by on every side (shadow reach and layer blur).
    pub fn padding(&self) -> usize {
        let mut p = 0.0f32;
        if self.shadow.is_visible() {
            let blur = if self.shadow.radius.is_finite() { self.shadow.radius.max(0.0) } else { 0.0 };
            let (dx, dy) = if self.shadow.offset.0.is_finite() && self.shadow.offset.1.is_finite() {
                self.shadow.offset
            } else {
                (0.0, 0.0)
            };
            p = p.max(blur * 1.5 + dx.abs().max(dy.abs()));
        }
        for f in &self.filters {
            if let LayerFilter::GaussianBlur(s) = f {
                if s.is_finite() && *s > 0.0 {
                    p = p.max(s * 3.0);
                }
            }
        }
        p.ceil() as usize
    }

    fn radii(&self, w: f32, h: f32) -> [f32; 4] {
        corner_radii(self.corner_radius, w, h, self.masked_corners)
    }
}

/// A styled layer rendered into a buffer that is larger than the content by `pad` pixels on every side.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub pixmap: Pixmap,
    pub pad: usize,
}

fn scale_px(p: u32, k: f32) -> u32 {
    if k >= 1.0 {
        return p;
    }
    let c = unpack(p);
    pack([c[0] * k, c[1] * k, c[2] * k, c[3] * k])
}

/// Renders a layer's content with its style. `backdrop` is the picture behind the layer, cropped to the layer's
/// frame and the same size as `content`, used by the background filters.
pub fn render_styled(content: &Pixmap, style: &LayerStyle, backdrop: Option<&Pixmap>) -> Rendered {
    let pad = style.padding();
    let (w, h) = (content.w, content.h);
    let mut out = Pixmap::new(w + 2 * pad, h + 2 * pad);
    if w == 0 || h == 0 {
        return Rendered { pixmap: out, pad };
    }
    let r = style.radii(w as f32, h as f32);
    let mut cov = vec![1.0f32; w * h];
    if r.iter().any(|v| *v > 0.0) {
        for y in 0..h {
            for x in 0..w {
                cov[y * w + x] = shape_coverage(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32, r, style.corner_curve);
            }
        }
    }
    // Layer body: background under content.
    let mut body = Pixmap::new(w, h);
    if style.background >> 24 != 0 {
        let bg = pack(premul(style.background, 1.0));
        for i in 0..w * h {
            body.px[i] = scale_px(bg, cov[i]);
        }
    }
    for i in 0..w * h {
        let c = if style.masks_to_bounds { scale_px(content.px[i], cov[i]) } else { content.px[i] };
        body.px[i] = blend_pixel(body.px[i], c, BlendMode::Normal, 1.0);
    }
    // Background filters: transform the backdrop inside the shape and put it under the body.
    if let Some(bd) = backdrop.filter(|b| b.w == w && b.h == h) {
        let mut back = bd.clone();
        let mut touched = false;
        for f in &style.filters {
            match f {
                LayerFilter::BackgroundBlur(s) => {
                    back = gaussian_blur(&back, *s);
                    touched = true;
                }
                LayerFilter::BackgroundColorMatrix(m) => {
                    back.px.iter_mut().for_each(|p| *p = m.apply(*p));
                    touched = true;
                }
                _ => {}
            }
        }
        if touched {
            for i in 0..w * h {
                let under = scale_px(back.px[i], cov[i]);
                body.px[i] = blend_pixel(under, body.px[i], BlendMode::Normal, 1.0);
            }
        }
    }
    // Mask layer.
    if let Some(m) = style.mask.as_ref().filter(|m| m.w == w && m.h == h) {
        for i in 0..w * h {
            body.px[i] = scale_px(body.px[i], (m.px[i] >> 24) as f32 / 255.0);
        }
    }
    // Border over the body: the ring between the outer shape and the shape inset by the width.
    if style.border_width > 0.0 && style.border_width.is_finite() && style.border_color >> 24 != 0 {
        let bw = style.border_width.min(w.min(h) as f32 / 2.0);
        let ri = r.map(|v| (v - bw).max(0.0));
        let (iw, ih) = (w as f32 - 2.0 * bw, h as f32 - 2.0 * bw);
        let col = pack(premul(style.border_color, 1.0));
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let inner = if iw > 0.0 && ih > 0.0 {
                    shape_coverage(fx - bw, fy - bw, iw, ih, ri, style.corner_curve)
                } else {
                    0.0
                };
                let ring = (cov[y * w + x] - inner).clamp(0.0, 1.0);
                if ring > 0.0 {
                    let i = y * w + x;
                    body.px[i] = blend_pixel(body.px[i], scale_px(col, ring), BlendMode::Normal, 1.0);
                }
            }
        }
    }
    // Colour matrices over the layer, then place it into the padded buffer and blur it if asked.
    for f in &style.filters {
        if let LayerFilter::ColorMatrix(m) = f {
            body.px.iter_mut().for_each(|p| *p = m.apply(*p));
        }
    }
    let mut padded = Pixmap::new(out.w, out.h);
    composite_onto(&mut padded, &body, pad as i32, pad as i32, BlendMode::Normal, 1.0);
    for f in &style.filters {
        if let LayerFilter::GaussianBlur(s) = f {
            padded = gaussian_blur(&padded, *s);
        }
    }
    // Shadow under everything.
    if style.shadow.is_visible() && style.shadow_color >> 24 != 0 {
        let alpha: Vec<f32> = match &style.shadow_path {
            Some(path) => {
                let mut a = vec![0.0f32; out.w * out.h];
                for y in 0..out.h {
                    for x in 0..out.w {
                        a[y * out.w + x] = path.coverage(x as f32 + 0.5 - pad as f32, y as f32 + 0.5 - pad as f32);
                    }
                }
                a
            }
            None => padded.alpha_plane(),
        };
        let (dx, dy) = if style.shadow.offset.0.is_finite() && style.shadow.offset.1.is_finite() {
            style.shadow.offset
        } else {
            (0.0, 0.0)
        };
        let (idx, idy) = (dx.round() as i32, dy.round() as i32);
        let mut shifted = vec![0.0f32; out.w * out.h];
        for y in 0..out.h as i32 {
            for x in 0..out.w as i32 {
                let (sx, sy) = (x - idx, y - idy);
                if sx >= 0 && sy >= 0 && (sx as usize) < out.w && (sy as usize) < out.h {
                    shifted[y as usize * out.w + x as usize] = alpha[sy as usize * out.w + sx as usize];
                }
            }
        }
        let sigma = style.shadow.radius / 2.0;
        let blurred =
            if sigma > 0.0 && sigma.is_finite() { blur_plane(&shifted, out.w, out.h, &gaussian_kernel(sigma)) } else { shifted };
        let k = style.shadow.effective_opacity();
        let col = premul(style.shadow_color, 1.0);
        for i in 0..out.w * out.h {
            let a = blurred[i].clamp(0.0, 1.0) * k;
            if a > 0.0 {
                out.px[i] = pack([col[0] * a, col[1] * a, col[2] * a, col[3] * a]);
            }
        }
    }
    composite_onto(&mut out, &padded, 0, 0, BlendMode::Normal, 1.0);
    Rendered { pixmap: out, pad }
}

// ---- 3D projection ----

/// Draws `src` into `dst` with `m` mapping source pixel coordinates (z = 0) to destination coordinates, bilinear,
/// with perspective division. Nothing is drawn when any corner is behind the eye, when the matrix is singular, or
/// when the layer faces away and `double_sided` is false. Returns whether anything was drawn.
pub fn draw_transformed(dst: &mut Pixmap, src: &Pixmap, m: Transform3D, mode: BlendMode, opacity: f32, double_sided: bool) -> bool {
    if src.w == 0 || src.h == 0 {
        return false;
    }
    let (sw, sh) = (src.w as f32, src.h as f32);
    let corners = [(0.0, 0.0), (sw, 0.0), (sw, sh), (0.0, sh)];
    let mut q = [(0.0f32, 0.0f32); 4];
    for (i, c) in corners.iter().enumerate() {
        match m.project(c.0, c.1) {
            Some(p) => q[i] = p,
            None => return false,
        }
    }
    let area: f32 = (0..4).map(|i| q[i].0 * q[(i + 1) % 4].1 - q[(i + 1) % 4].0 * q[i].1).sum::<f32>() / 2.0;
    if area == 0.0 || !area.is_finite() || (area < 0.0 && !double_sided) {
        return false;
    }
    // Homography (source -> destination) and its inverse.
    let a = m.m;
    let h = [[a[0][0], a[1][0], a[3][0]], [a[0][1], a[1][1], a[3][1]], [a[0][3], a[1][3], a[3][3]]];
    let Some(inv) = invert3(h) else { return false };
    let x0 = q.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let y0 = q.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
    let x1 = (q.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32).min(dst.w as i32);
    let y1 = (q.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32).min(dst.h as i32);
    let mut any = false;
    for y in y0..y1 {
        for x in x0..x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let w = inv[2][0] * fx + inv[2][1] * fy + inv[2][2];
            if w.abs() < 1e-9 {
                continue;
            }
            let sx = (inv[0][0] * fx + inv[0][1] * fy + inv[0][2]) / w;
            let sy = (inv[1][0] * fx + inv[1][1] * fy + inv[1][2]) / w;
            if sx < 0.0 || sy < 0.0 || sx >= sw || sy >= sh {
                continue;
            }
            let p = sample(src, sx - 0.5, sy - 0.5);
            if p >> 24 == 0 {
                continue;
            }
            let i = y as usize * dst.w + x as usize;
            dst.px[i] = blend_pixel(dst.px[i], p, mode, opacity);
            any = true;
        }
    }
    any
}

fn sample(src: &Pixmap, x: f32, y: f32) -> u32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ix, iy) = (ix as i32, iy as i32);
    // Edge pixels clamp, so a layer keeps its colour right up to its bounds.
    let g = |dx: i32, dy: i32| unpack(src.get((ix + dx).clamp(0, src.w as i32 - 1), (iy + dy).clamp(0, src.h as i32 - 1)));
    let (p00, p10, p01, p11) = (g(0, 0), g(1, 0), g(0, 1), g(1, 1));
    let mut o = [0.0f32; 4];
    for c in 0..4 {
        o[c] = p00[c] * (1.0 - fx) * (1.0 - fy) + p10[c] * fx * (1.0 - fy) + p01[c] * (1.0 - fx) * fy + p11[c] * fx * fy;
    }
    pack(o)
}

pub(crate) fn invert3(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let c = |r: usize, c: usize| m[r][c];
    let det = c(0, 0) * (c(1, 1) * c(2, 2) - c(1, 2) * c(2, 1)) - c(0, 1) * (c(1, 0) * c(2, 2) - c(1, 2) * c(2, 0))
        + c(0, 2) * (c(1, 0) * c(2, 1) - c(1, 1) * c(2, 0));
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            (c(1, 1) * c(2, 2) - c(1, 2) * c(2, 1)) * d,
            (c(0, 2) * c(2, 1) - c(0, 1) * c(2, 2)) * d,
            (c(0, 1) * c(1, 2) - c(0, 2) * c(1, 1)) * d,
        ],
        [
            (c(1, 2) * c(2, 0) - c(1, 0) * c(2, 2)) * d,
            (c(0, 0) * c(2, 2) - c(0, 2) * c(2, 0)) * d,
            (c(0, 2) * c(1, 0) - c(0, 0) * c(1, 2)) * d,
        ],
        [
            (c(1, 0) * c(2, 1) - c(1, 1) * c(2, 0)) * d,
            (c(0, 1) * c(2, 0) - c(0, 0) * c(2, 1)) * d,
            (c(0, 0) * c(1, 1) - c(0, 1) * c(1, 0)) * d,
        ],
    ])
}

// ---- layer tree ----

/// A node of a layer tree: content, style, place and transforms. Coordinates are in the parent's space.
#[derive(Clone, Debug)]
pub struct StyledLayer {
    pub content: Pixmap,
    pub style: LayerStyle,
    /// Top-left of the layer in its parent.
    pub position: (f32, f32),
    /// The layer's own transform, applied about its centre.
    pub transform: Transform3D,
    /// Applied to the layer's children, about this layer's centre.
    pub sublayer_transform: Transform3D,
    /// When true the children keep their 3D relationship to this layer (and to each other) instead of being
    /// flattened into this layer's plane first.
    pub transform_layer: bool,
    pub double_sided: bool,
    pub children: Vec<StyledLayer>,
}

/// The largest group buffer side, so a wild transform cannot ask for a huge allocation.
const MAX_GROUP_SIDE: i32 = 4096;

impl StyledLayer {
    pub fn new(content: Pixmap) -> StyledLayer {
        StyledLayer {
            content,
            style: LayerStyle::default(),
            position: (0.0, 0.0),
            transform: Transform3D::IDENTITY,
            sublayer_transform: Transform3D::IDENTITY,
            transform_layer: false,
            double_sided: true,
            children: Vec::new(),
        }
    }

    fn centre(&self) -> (f32, f32) {
        (self.content.w as f32 / 2.0, self.content.h as f32 / 2.0)
    }

    /// Maps this layer's local coordinates into its parent's space.
    fn to_parent(&self) -> Transform3D {
        let (cx, cy) = self.centre();
        self.transform.about(cx, cy, 0.0).concat(Transform3D::make_translation(self.position.0, self.position.1, 0.0))
    }

    /// Maps this layer's local coordinates to its children's parent space (the sublayer transform).
    fn sub(&self) -> Transform3D {
        let (cx, cy) = self.centre();
        self.sublayer_transform.about(cx, cy, 0.0)
    }
}

/// Composites a layer tree into `dst`. `root_to_dst` maps the root's parent space to `dst` (for example a
/// perspective projection).
pub fn render_tree(dst: &mut Pixmap, root: &StyledLayer, root_to_dst: Transform3D) {
    draw_layer(dst, root, root_to_dst);
}

fn draw_layer(dst: &mut Pixmap, layer: &StyledLayer, parent_world: Transform3D) {
    let world = layer.to_parent().concat(parent_world);
    let child_world = layer.sub().concat(world);
    // Backdrop for background filters: what is already in dst under the layer's footprint.
    let wants_backdrop = layer.style.filters.iter().any(|f| matches!(f, LayerFilter::BackgroundBlur(_) | LayerFilter::BackgroundColorMatrix(_)));
    let backdrop = if wants_backdrop && world.is_affine() {
        let (ox, oy) = world.project(0.0, 0.0).unwrap_or((0.0, 0.0));
        Some(dst.crop(ox.round() as i32, oy.round() as i32, layer.content.w, layer.content.h))
    } else {
        None
    };
    let r = render_styled(&layer.content, &layer.style, backdrop.as_ref());
    let pad = r.pad as f32;
    if layer.transform_layer {
        // Own pixels in the shared 3D space, then children sorted far to near, each with the full matrix.
        let m = Transform3D::make_translation(-pad, -pad, 0.0).concat(world);
        draw_transformed(dst, &r.pixmap, m, layer.style.blend, layer.style.opacity, layer.double_sided);
        let mut order: Vec<&StyledLayer> = layer.children.iter().collect();
        order.sort_by(|a, b| {
            let za = a.to_parent().concat(child_world).depth_of(a.centre().0, a.centre().1);
            let zb = b.to_parent().concat(child_world).depth_of(b.centre().0, b.centre().1);
            za.total_cmp(&zb)
        });
        for c in order {
            draw_layer(dst, c, child_world);
        }
        return;
    }
    if layer.children.is_empty() {
        let m = Transform3D::make_translation(-pad, -pad, 0.0).concat(world);
        draw_transformed(dst, &r.pixmap, m, layer.style.blend, layer.style.opacity, layer.double_sided);
        return;
    }
    // Flattened group: render the layer and its children into one plane (large enough for children that stick out
    // of the layer), then place that plane with `world`.
    let sub = layer.sub();
    let (mut minx, mut miny) = (-pad, -pad);
    let (mut maxx, mut maxy) = (layer.content.w as f32 + pad, layer.content.h as f32 + pad);
    for c in &layer.children {
        for corner in [(0.0, 0.0), (c.content.w as f32, 0.0), (c.content.w as f32, c.content.h as f32), (0.0, c.content.h as f32)] {
            if let Some((x, y)) = c.to_parent().concat(sub).project(corner.0, corner.1) {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
    }
    let (ox, oy) = ((-minx).ceil() as i32, (-miny).ceil() as i32);
    let pw = ((maxx.ceil() as i32) + ox).clamp(1, MAX_GROUP_SIDE);
    let ph = ((maxy.ceil() as i32) + oy).clamp(1, MAX_GROUP_SIDE);
    let mut plane = Pixmap::new(pw as usize, ph as usize);
    composite_onto(&mut plane, &r.pixmap, ox - r.pad as i32, oy - r.pad as i32, BlendMode::Normal, 1.0);
    let mut kids = Pixmap::new(plane.w, plane.h);
    let local = sub.concat(Transform3D::make_translation(ox as f32, oy as f32, 0.0));
    for c in &layer.children {
        draw_layer(&mut kids, c, local);
    }
    if layer.style.masks_to_bounds {
        clip_to_shape(&mut kids, &layer.style, layer.content.w, layer.content.h, (ox, oy));
    }
    composite_onto(&mut plane, &kids, 0, 0, BlendMode::Normal, 1.0);
    let m = Transform3D::make_translation(-(ox as f32), -(oy as f32), 0.0).concat(world);
    draw_transformed(dst, &plane, m, layer.style.blend, layer.style.opacity, layer.double_sided);
}

/// Clears everything outside the layer's rounded shape; `origin` is where the layer's top-left sits in `p`.
fn clip_to_shape(p: &mut Pixmap, style: &LayerStyle, w: usize, h: usize, origin: (i32, i32)) {
    let r = style.radii(w as f32, h as f32);
    for y in 0..p.h {
        for x in 0..p.w {
            let (fx, fy) = (x as f32 + 0.5 - origin.0 as f32, y as f32 + 0.5 - origin.1 as f32);
            let inside = fx >= 0.0 && fy >= 0.0 && fx < w as f32 && fy < h as f32;
            let k = if inside { shape_coverage(fx, fy, w as f32, h as f32, r, style.corner_curve) } else { 0.0 };
            let i = y * p.w + x;
            p.px[i] = scale_px(p.px[i], k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xFF00_00FF;
    const BLUE: u32 = 0xFFFF_0000;

    fn solid(w: usize, h: usize, c: u32) -> Pixmap {
        Pixmap::filled(w, h, c)
    }

    #[test]
    fn corner_radius_cuts_masked_corners_only() {
        let mut s = LayerStyle { corner_radius: 8.0, masks_to_bounds: true, ..LayerStyle::default() };
        s.masked_corners = CornerMask::MIN_X_MIN_Y;
        let r = render_styled(&solid(20, 20, RED), &s, None);
        assert_eq!(r.pad, 0);
        assert_eq!(r.pixmap.get(0, 0) >> 24, 0);
        assert_eq!(r.pixmap.get(19, 19), RED);
        assert_eq!(r.pixmap.get(10, 10), RED);
    }

    #[test]
    fn continuous_curve_differs_from_circular_at_the_corner_diagonal() {
        let mk = |curve| {
            let s = LayerStyle { corner_radius: 10.0, masks_to_bounds: true, corner_curve: curve, ..LayerStyle::default() };
            render_styled(&solid(20, 20, RED), &s, None).pixmap.get(2, 2) >> 24
        };
        // The superellipse bulges further into the corner square than the circle, so it covers more at (2, 2).
        assert!(mk(CornerCurve::Continuous) > mk(CornerCurve::Circular));
    }

    #[test]
    fn border_paints_the_outer_ring_only() {
        let s = LayerStyle { border_width: 2.0, border_color: BLUE, ..LayerStyle::default() };
        let r = render_styled(&solid(12, 12, RED), &s, None);
        assert_eq!(r.pixmap.get(0, 5), BLUE);
        assert_eq!(r.pixmap.get(1, 5), BLUE);
        assert_eq!(r.pixmap.get(6, 6), RED);
    }

    #[test]
    fn shadow_extends_outside_with_colour_and_offset() {
        let s = LayerStyle {
            shadow: Shadow { opacity: 1.0, radius: 4.0, offset: (4.0, 4.0) },
            shadow_color: 0xFF00_0000,
            ..LayerStyle::default()
        };
        let r = render_styled(&solid(10, 10, RED), &s, None);
        assert!(r.pad >= 4);
        let p = r.pad as i32;
        // Right of the layer, inside the offset shadow: dark and partly opaque.
        let px = r.pixmap.get(p + 12, p + 6);
        assert!(px >> 24 > 0 && px & 255 == 0);
        // Far left of the layer there is no shadow.
        assert_eq!(r.pixmap.get(0, p + 5) >> 24, 0);
    }

    #[test]
    fn shadow_path_overrides_the_layer_shape() {
        let s = LayerStyle {
            shadow: Shadow { opacity: 1.0, radius: 0.0, offset: (0.0, 0.0) },
            shadow_path: Some(ShadowPath::RoundedRect { rect: (0.0, 0.0, 4.0, 4.0), radius: 0.0 }),
            ..LayerStyle::default()
        };
        let r = render_styled(&solid(10, 10, 0), &s, None);
        assert_eq!(r.pixmap.get(r.pad as i32 + 2, r.pad as i32 + 2), 0xFF00_0000);
        assert_eq!(r.pixmap.get(r.pad as i32 + 8, r.pad as i32 + 8), 0);
    }

    #[test]
    fn polygon_shadow_path_follows_even_odd() {
        let tri = ShadowPath::Polygons(vec![vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]]);
        assert!(tri.coverage(2.0, 2.0) > 0.9);
        assert!(tri.coverage(8.0, 8.0) < 0.1);
    }

    #[test]
    fn mask_hides_where_the_mask_is_clear() {
        let mut mask = Pixmap::new(10, 10);
        for y in 0..10 {
            for x in 0..5 {
                mask.px[y * 10 + x] = 0xFF00_0000;
            }
        }
        let s = LayerStyle { mask: Some(mask), ..LayerStyle::default() };
        let r = render_styled(&solid(10, 10, RED), &s, None);
        assert_eq!(r.pixmap.get(2, 5), RED);
        assert_eq!(r.pixmap.get(7, 5), 0);
    }

    #[test]
    fn gaussian_blur_spreads_and_keeps_total_roughly() {
        let mut p = Pixmap::new(21, 21);
        p.px[10 * 21 + 10] = 0xFFFF_FFFF;
        let b = gaussian_blur(&p, 2.0);
        assert!(b.get(10, 10) >> 24 < 255);
        assert!(b.get(12, 10) >> 24 > 0);
        let sum: u32 = b.px.iter().map(|v| v >> 24).sum();
        assert!((sum as i32 - 255).abs() < 40, "{sum}");
        assert_eq!(gaussian_blur(&p, 0.0), p);
    }

    #[test]
    fn layer_blur_pads_the_output() {
        let s = LayerStyle { filters: vec![LayerFilter::GaussianBlur(2.0)], ..LayerStyle::default() };
        let r = render_styled(&solid(6, 6, RED), &s, None);
        assert_eq!(r.pad, 6);
        assert!(r.pixmap.get(r.pad as i32 - 2, r.pad as i32 + 3) >> 24 > 0);
    }

    #[test]
    fn colour_matrix_inverts_and_desaturates() {
        let s = LayerStyle { filters: vec![LayerFilter::ColorMatrix(ColorMatrix::invert())], ..LayerStyle::default() };
        let r = render_styled(&solid(2, 2, RED), &s, None);
        assert_eq!(r.pixmap.get(0, 0), 0xFFFF_FF00);
        let c = unpack(ColorMatrix::saturation(0.0).apply(RED));
        assert!((c[0] - c[1]).abs() < 0.01 && (c[1] - c[2]).abs() < 0.01);
    }

    #[test]
    fn background_blur_softens_the_backdrop_under_the_layer() {
        let mut back = solid(16, 16, 0xFF00_0000);
        for y in 0..16 {
            for x in 8..16 {
                back.px[y * 16 + x] = 0xFFFF_FFFF;
            }
        }
        let s = LayerStyle { filters: vec![LayerFilter::BackgroundBlur(3.0)], ..LayerStyle::default() };
        let r = render_styled(&Pixmap::new(16, 16), &s, Some(&back));
        let edge = r.pixmap.get(8, 8) & 255;
        assert!(edge > 20 && edge < 235, "{edge}");
    }

    #[test]
    fn blend_modes_combine_with_the_backdrop() {
        let grey = 0xFF80_8080;
        assert_eq!(blend_pixel(grey, grey, BlendMode::Normal, 1.0), grey);
        let m = blend_pixel(grey, grey, BlendMode::Multiply, 1.0) & 255;
        assert!((m as i32 - 64).abs() <= 2);
        let s = blend_pixel(grey, grey, BlendMode::Screen, 1.0) & 255;
        assert!(s > 0xB0);
        assert_eq!(blend_pixel(0xFFFF_FFFF, 0xFFFF_FFFF, BlendMode::Difference, 1.0) & 0x00FF_FFFF, 0);
        assert_eq!(blend_pixel(grey, RED, BlendMode::PlusLighter, 1.0) & 255, 255);
        assert_eq!(blend_pixel(grey, RED, BlendMode::Normal, 0.0), grey);
    }

    #[test]
    fn translation_moves_a_layer_and_scale_grows_it() {
        let mut dst = Pixmap::new(30, 30);
        let m = Transform3D::make_translation(10.0, 5.0, 0.0);
        assert!(draw_transformed(&mut dst, &solid(4, 4, RED), m, BlendMode::Normal, 1.0, true));
        assert_eq!(dst.get(11, 6), RED);
        assert_eq!(dst.get(5, 6), 0);
        let mut big = Pixmap::new(30, 30);
        draw_transformed(&mut big, &solid(4, 4, RED), Transform3D::make_scale(2.0, 2.0, 1.0), BlendMode::Normal, 1.0, true);
        assert_eq!(big.get(7, 7), RED);
        assert_eq!(big.get(9, 9), 0);
    }

    #[test]
    fn perspective_shrinks_a_layer_pushed_away_and_back_face_is_culled() {
        let persp = Transform3D::make_perspective(100.0);
        let far = Transform3D::make_translation(0.0, 0.0, -100.0).concat(persp);
        let mut dst = Pixmap::new(40, 40);
        // z = -100 gives w = 2, so a 20-wide layer covers 10 pixels.
        assert!(draw_transformed(&mut dst, &solid(20, 20, RED), far, BlendMode::Normal, 1.0, true));
        let covered = dst.px.iter().filter(|p| **p >> 24 != 0).count();
        assert!((covered as i32 - 100).abs() <= 24, "{covered}");
        // A half turn about y faces away.
        let flip = Transform3D::make_rotation(std::f32::consts::PI, 0.0, 1.0, 0.0).concat(Transform3D::make_translation(30.0, 0.0, 0.0));
        let mut d2 = Pixmap::new(40, 40);
        assert!(!draw_transformed(&mut d2, &solid(20, 20, RED), flip, BlendMode::Normal, 1.0, false));
        assert!(draw_transformed(&mut d2, &solid(20, 20, RED), flip, BlendMode::Normal, 1.0, true));
    }

    #[test]
    fn points_behind_the_eye_are_not_drawn() {
        let m = Transform3D::make_translation(0.0, 0.0, 300.0).concat(Transform3D::make_perspective(100.0));
        let mut dst = Pixmap::new(20, 20);
        assert!(!draw_transformed(&mut dst, &solid(5, 5, RED), m, BlendMode::Normal, 1.0, true));
    }

    #[test]
    fn sublayer_transform_moves_children_not_the_parent() {
        let mut parent = StyledLayer::new(solid(10, 10, BLUE));
        parent.sublayer_transform = Transform3D::make_translation(20.0, 0.0, 0.0);
        let mut child = StyledLayer::new(solid(4, 4, RED));
        child.position = (1.0, 1.0);
        parent.children.push(child);
        let mut dst = Pixmap::new(40, 20);
        render_tree(&mut dst, &parent, Transform3D::IDENTITY);
        assert_eq!(dst.get(5, 5), BLUE);
        assert_eq!(dst.get(22, 2), RED);
        assert_eq!(dst.get(2, 2), BLUE);
    }

    #[test]
    fn transform_layer_keeps_children_in_3d_while_a_flat_layer_flattens_them() {
        let build = |tl: bool| {
            let mut p = StyledLayer::new(solid(20, 20, BLUE));
            p.position = (10.0, 0.0);
            p.transform = Transform3D::make_rotation(0.8, 0.0, 1.0, 0.0);
            p.transform_layer = tl;
            let mut c = StyledLayer::new(solid(10, 10, RED));
            c.position = (5.0, 5.0);
            c.transform = Transform3D::make_translation(0.0, 0.0, 15.0);
            p.children.push(c);
            let mut dst = Pixmap::new(60, 30);
            render_tree(&mut dst, &p, Transform3D::make_perspective(120.0));
            dst
        };
        let flat = build(false);
        let deep = build(true);
        assert_ne!(flat.px, deep.px);
        assert!(deep.px.iter().any(|p| *p >> 24 != 0 && *p & 255 > 0xC0));
    }

    #[test]
    fn masks_to_bounds_clips_children() {
        let mut parent = StyledLayer::new(solid(10, 10, BLUE));
        parent.style.masks_to_bounds = true;
        let mut child = StyledLayer::new(solid(10, 10, RED));
        child.position = (5.0, 0.0);
        parent.children.push(child);
        let mut dst = Pixmap::new(30, 10);
        render_tree(&mut dst, &parent, Transform3D::IDENTITY);
        assert_eq!(dst.get(7, 5), RED);
        assert_eq!(dst.get(12, 5), 0);
    }
}

// ---- hit testing through transforms ----

/// The z = 0 plane point that `m` maps to the screen point `(x, y)`; `None` when `m` is singular.
pub fn unproject(m: Transform3D, x: f32, y: f32) -> Option<(f32, f32)> {
    let a = m.m;
    let h = [[a[0][0], a[1][0], a[3][0]], [a[0][1], a[1][1], a[3][1]], [a[0][3], a[1][3], a[3][3]]];
    let inv = invert3(h)?;
    let w = inv[2][0] * x + inv[2][1] * y + inv[2][2];
    if w.abs() < 1e-9 {
        return None;
    }
    Some(((inv[0][0] * x + inv[0][1] * y + inv[0][2]) / w, (inv[1][0] * x + inv[1][1] * y + inv[1][2]) / w))
}

impl StyledLayer {
    /// The front-most layer under the screen point `p`, as the path of child indices from this layer (empty for this
    /// layer itself). Follows the same transforms, sublayer transform, perspective and depth order as
    /// [`render_tree`]. Back faces are not hit when the layer is not double sided.
    pub fn hit_test(&self, p: (f32, f32), parent_world: Transform3D) -> Option<Vec<usize>> {
        let world = self.to_parent().concat(parent_world);
        let child_world = self.sub().concat(world);
        let mut order: Vec<usize> = (0..self.children.len()).collect();
        let depth = |i: usize| {
            let c = &self.children[i];
            c.to_parent().concat(child_world).depth_of(c.centre().0, c.centre().1)
        };
        order.sort_by(|a, b| depth(*a).total_cmp(&depth(*b)));
        for i in order.into_iter().rev() {
            if let Some(mut path) = self.children[i].hit_test(p, child_world) {
                path.insert(0, i);
                return Some(path);
            }
        }
        let (lx, ly) = unproject(world, p.0, p.1)?;
        let inside = lx >= 0.0 && ly >= 0.0 && lx < self.content.w as f32 && ly < self.content.h as f32;
        if !inside || self.style.opacity <= 0.0 {
            return None;
        }
        if !self.double_sided {
            let q = [(0.0, 0.0), (self.content.w as f32, 0.0), (self.content.w as f32, self.content.h as f32)]
                .map(|c| world.project(c.0, c.1));
            if let [Some(a), Some(b), Some(c)] = q {
                if (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0) < 0.0 {
                    return None;
                }
            }
        }
        Some(Vec::new())
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;

    #[test]
    fn hits_follow_translation_and_scale_and_children_win() {
        let mut p = StyledLayer::new(Pixmap::filled(10, 10, 0xFF00_00FF));
        p.position = (20.0, 0.0);
        p.transform = Transform3D::make_scale(2.0, 2.0, 1.0);
        let mut c = StyledLayer::new(Pixmap::filled(4, 4, 0xFF00_00FF));
        c.position = (0.0, 0.0);
        p.children.push(c);
        // The parent is 10x10 scaled by 2 about its centre (5,5): it spans screen 15..35 on x.
        assert_eq!(p.hit_test((16.0, 2.0), Transform3D::IDENTITY), Some(vec![0]));
        assert_eq!(p.hit_test((30.0, 10.0), Transform3D::IDENTITY), Some(vec![]));
        assert_eq!(p.hit_test((5.0, 5.0), Transform3D::IDENTITY), None);
    }

    #[test]
    fn unproject_inverts_a_perspective_projection() {
        let m = Transform3D::make_rotation(0.6, 0.0, 1.0, 0.0).concat(Transform3D::make_perspective(200.0));
        let (sx, sy) = m.project(10.0, 7.0).unwrap();
        let (x, y) = unproject(m, sx, sy).unwrap();
        assert!((x - 10.0).abs() < 1e-2 && (y - 7.0).abs() < 1e-2);
    }
}
