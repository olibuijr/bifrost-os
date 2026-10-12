//! View effects for apps: Gaussian shadows, continuous-corner (squircle) shapes, alpha masks, group opacity
//! and blend modes on CPU layers. Pure std. Parity targets: SwiftUI `shadow(color:radius:x:y:)`,
//! `mask(alignment:_:)`, `clipShape`, `compositingGroup()`, `opacity`, `blendMode`; Core Animation `shadowPath`,
//! `shadowOpacity`, `shadowRadius`, `cornerCurve = .continuous`.
//!
//! Pixels are premultiplied RGBA8 packed like `gfx::pack` (red in the low byte, alpha in the top byte).
//! A `Plane` is an f32 coverage map (0..=1). Window shadows belong to the window server, not here.

use crate::blend::{composite, Mode};

/// A single-channel coverage map.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, a: vec![0.0; w * h] }
    }
    pub fn filled(w: usize, h: usize, v: f32) -> Plane {
        Plane { w, h, a: vec![v; w * h] }
    }
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0.0
        } else {
            self.a[y as usize * self.w + x as usize]
        }
    }
    /// Multiplies by another plane of the same size (mask intersection).
    pub fn multiply(&mut self, o: &Plane) {
        for (d, s) in self.a.iter_mut().zip(&o.a) {
            *d *= *s;
        }
    }
}

/// How a corner is drawn. `Continuous` is the Apple squircle (`cornerCurve = .continuous`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerCurve {
    Circular,
    Continuous,
}

/// Exponent of the continuous corner superellipse.
const SQUIRCLE_N: f32 = 5.0;
/// The continuous corner reaches further along the edge than a circle of the same radius.
const SQUIRCLE_REACH: f32 = 1.2819;

/// Anti-aliased coverage of pixel centre `(px, py)` inside a rounded rect at `(x, y, w, h)`.
pub fn rrect_cover(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32, curve: CornerCurve) -> f32 {
    let (cx, cy) = (x + w * 0.5, y + h * 0.5);
    let (hw, hh) = (w * 0.5, h * 0.5);
    let r = radius.max(0.0).min(hw.min(hh));
    let (dx, dy) = ((px - cx).abs(), (py - cy).abs());
    let (ex, ey) = (dx - (hw - r), dy - (hh - r));
    let d = if ex <= 0.0 || ey <= 0.0 || r <= 0.0 {
        // Straight edge region: the larger per-axis overshoot (negative inside).
        (dx - hw).max(dy - hh)
    } else {
        match curve {
            CornerCurve::Circular => (ex * ex + ey * ey).sqrt() - r,
            CornerCurve::Continuous => {
                // Corner extent e along each edge; superellipse |u/e|^n + |v/e|^n = 1 with the origin at the
                // start of the corner. Distance is estimated as (f - 1) / |grad f| (first order).
                let e = (r * SQUIRCLE_REACH).min(hw.min(hh));
                let (u, v) = (dx - (hw - e), dy - (hh - e));
                if u <= 0.0 || v <= 0.0 {
                    (dx - hw).max(dy - hh)
                } else {
                    let (a, b) = (u / e, v / e);
                    let f = a.powf(SQUIRCLE_N) + b.powf(SQUIRCLE_N);
                    let gx = SQUIRCLE_N * a.powf(SQUIRCLE_N - 1.0) / e;
                    let gy = SQUIRCLE_N * b.powf(SQUIRCLE_N - 1.0) / e;
                    let g = (gx * gx + gy * gy).sqrt().max(1e-6);
                    (f - 1.0) / g
                }
            }
        }
    };
    (0.5 - d).clamp(0.0, 1.0)
}

/// Coverage plane of a rounded rect (continuous or circular corners) in a `w` x `h` plane.
pub fn rrect_plane(w: usize, h: usize, x: f32, y: f32, rw: f32, rh: f32, radius: f32, curve: CornerCurve) -> Plane {
    let mut p = Plane::new(w, h);
    for j in 0..h {
        for i in 0..w {
            p.a[j * w + i] = rrect_cover(i as f32 + 0.5, j as f32 + 0.5, x, y, rw, rh, radius, curve);
        }
    }
    p
}

/// Gaussian kernel for standard deviation `sigma`, radius 3 sigma, normalised to sum 1.
fn kernel(sigma: f32) -> Vec<f32> {
    let r = (sigma * 3.0).ceil().max(1.0) as i32;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp()).collect();
    let s: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= s);
    k
}

/// Separable Gaussian blur of a plane. Outside the plane counts as 0.
pub fn gaussian_blur(p: &Plane, sigma: f32) -> Plane {
    if sigma <= 0.0 {
        return p.clone();
    }
    let k = kernel(sigma);
    let r = (k.len() / 2) as i32;
    let mut tmp = Plane::new(p.w, p.h);
    for y in 0..p.h as i32 {
        for x in 0..p.w as i32 {
            let mut s = 0.0;
            for (i, kv) in k.iter().enumerate() {
                s += kv * p.get(x + i as i32 - r, y);
            }
            tmp.a[y as usize * p.w + x as usize] = s;
        }
    }
    let mut out = Plane::new(p.w, p.h);
    for y in 0..p.h as i32 {
        for x in 0..p.w as i32 {
            let mut s = 0.0;
            for (i, kv) in k.iter().enumerate() {
                s += kv * tmp.get(x, y + i as i32 - r);
            }
            out.a[y as usize * p.w + x as usize] = s;
        }
    }
    out
}

/// Grows (`spread > 0`) or shrinks (`spread < 0`) a plane by `spread` pixels with a round structuring element.
pub fn spread_plane(p: &Plane, spread: f32) -> Plane {
    let n = spread.abs().ceil() as i32;
    if n == 0 {
        return p.clone();
    }
    let grow = spread > 0.0;
    let mut out = Plane::new(p.w, p.h);
    for y in 0..p.h as i32 {
        for x in 0..p.w as i32 {
            let mut best = if grow { 0.0f32 } else { 1.0f32 };
            for dy in -n..=n {
                for dx in -n..=n {
                    if ((dx * dx + dy * dy) as f32).sqrt() > spread.abs() + 0.5 {
                        continue;
                    }
                    let v = p.get(x + dx, y + dy);
                    best = if grow { best.max(v) } else { best.min(v) };
                }
            }
            out.a[y as usize * p.w + x as usize] = best;
        }
    }
    out
}

/// A premultiplied RGBA layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Premultiplies an unpremultiplied RGBA colour (red low byte, alpha top byte) with an extra `alpha`.
pub fn premul(color: u32, alpha: f32) -> u32 {
    let a = ((color >> 24) & 0xff) as f32 / 255.0 * alpha.clamp(0.0, 1.0);
    let c = |s: u32| (((color >> s) & 0xff) as f32 * a + 0.5) as u32;
    c(0) | (c(8) << 8) | (c(16) << 16) | (((a * 255.0 + 0.5) as u32) << 24)
}

impl Layer {
    pub fn new(w: usize, h: usize) -> Layer {
        Layer { w, h, px: vec![0; w * h] }
    }

    pub fn get(&self, x: usize, y: usize) -> u32 {
        self.px[y * self.w + x]
    }

    /// Fills a coverage plane with a solid unpremultiplied colour under `mode`.
    pub fn fill_plane(&mut self, p: &Plane, color: u32, mode: Mode) {
        for i in 0..self.px.len().min(p.a.len()) {
            if p.a[i] > 0.0 {
                self.px[i] = composite(premul(color, p.a[i]), self.px[i], mode);
            }
        }
    }

    /// SwiftUI `mask` / `clipShape`: multiplies every pixel by the mask coverage.
    pub fn mask(&mut self, m: &Plane) {
        for i in 0..self.px.len() {
            let c = if i < m.a.len() { m.a[i] } else { 0.0 };
            let p = self.px[i];
            let s = |sh: u32| ((((p >> sh) & 0xff) as f32 * c) + 0.5) as u32;
            self.px[i] = s(0) | (s(8) << 8) | (s(16) << 16) | (s(24) << 24);
        }
    }

    /// Alpha channel as a plane (the shape that casts a shadow).
    pub fn alpha_plane(&self) -> Plane {
        Plane { w: self.w, h: self.h, a: self.px.iter().map(|p| (p >> 24) as f32 / 255.0).collect() }
    }

    /// Composites `src` onto `self` as one group: `opacity` applies to the flattened group, not to each child,
    /// so overlapping children do not show through each other (`compositingGroup().opacity(x)`).
    pub fn draw_group(&mut self, src: &Layer, opacity: f32, mode: Mode, dx: i32, dy: i32) {
        let a = opacity.clamp(0.0, 1.0);
        for y in 0..src.h as i32 {
            for x in 0..src.w as i32 {
                let (tx, ty) = (x + dx, y + dy);
                if tx < 0 || ty < 0 || tx as usize >= self.w || ty as usize >= self.h {
                    continue;
                }
                let s = src.px[y as usize * src.w + x as usize];
                if s >> 24 == 0 {
                    continue;
                }
                let sc = |sh: u32| ((((s >> sh) & 0xff) as f32 * a) + 0.5) as u32;
                let sp = sc(0) | (sc(8) << 8) | (sc(16) << 16) | (sc(24) << 24);
                let d = &mut self.px[ty as usize * self.w + tx as usize];
                *d = composite(sp, *d, mode);
            }
        }
    }
}

/// Shadow parameters. `radius` follows SwiftUI: the blur radius, Gaussian sigma = radius / 2.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub color: u32,
    pub radius: f32,
    pub x: f32,
    pub y: f32,
    pub spread: f32,
}

/// Renders the shadow of `shape` (a coverage plane) into a new layer of the same size. Draw the content
/// over it. Give the plane a margin (`shadow_margin`) so the shadow is not cut off.
pub fn shadow_layer(shape: &Plane, sh: Shadow) -> Layer {
    let s = gaussian_blur(&spread_plane(shape, sh.spread), sh.radius * 0.5);
    let (dx, dy) = (sh.x.round() as i32, sh.y.round() as i32);
    let mut shifted = Plane::new(s.w, s.h);
    for y in 0..s.h as i32 {
        for x in 0..s.w as i32 {
            shifted.a[y as usize * s.w + x as usize] = s.get(x - dx, y - dy);
        }
    }
    let mut l = Layer::new(shape.w, shape.h);
    l.fill_plane(&shifted, sh.color, Mode::Normal);
    l
}

/// Margin a plane needs so a shadow is not cut off.
pub fn shadow_margin(sh: Shadow) -> i32 {
    (sh.radius * 1.5 + sh.spread.max(0.0) + sh.x.abs().max(sh.y.abs())).ceil() as i32 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: u32 = 0xff00_0000;
    const RED: u32 = 0xff00_00ff;
    const GREEN: u32 = 0xff00_ff00;

    fn square(w: usize, h: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> Plane {
        let mut p = Plane::new(w, h);
        for y in y0..y1 {
            for x in x0..x1 {
                p.a[y * w + x] = 1.0;
            }
        }
        p
    }

    #[test]
    fn circular_rrect_edges_are_anti_aliased() {
        let p = rrect_plane(20, 20, 2.0, 2.0, 16.0, 16.0, 4.0, CornerCurve::Circular);
        assert_eq!(p.get(10, 10), 1.0);
        assert_eq!(p.get(0, 0), 0.0);
        assert_eq!(p.get(2, 10), 1.0);
        assert_eq!(p.get(1, 10), 0.0);
        assert!(p.get(2, 2) < 0.2);
    }

    #[test]
    fn continuous_corner_differs_from_circular_and_keeps_straight_edges() {
        let c = rrect_plane(40, 40, 0.0, 0.0, 40.0, 40.0, 12.0, CornerCurve::Circular);
        let s = rrect_plane(40, 40, 0.0, 0.0, 40.0, 40.0, 12.0, CornerCurve::Continuous);
        assert_eq!(c.get(20, 0), 1.0);
        assert_eq!(s.get(20, 0), 1.0);
        let area = |p: &Plane| p.a.iter().sum::<f32>();
        assert!(area(&s) > area(&c));
        assert!((area(&s) - area(&c)) / area(&c) < 0.08);
        assert_eq!(s.get(0, 0), 0.0);
    }

    #[test]
    fn gaussian_blur_preserves_mass_and_is_symmetric() {
        let mut p = Plane::new(41, 41);
        p.a[20 * 41 + 20] = 1.0;
        let b = gaussian_blur(&p, 3.0);
        let sum: f32 = b.a.iter().sum();
        assert!((sum - 1.0).abs() < 0.01, "{sum}");
        assert!((b.get(17, 20) - b.get(23, 20)).abs() < 1e-6);
        assert!((b.get(20, 17) - b.get(20, 23)).abs() < 1e-6);
        assert!(b.get(20, 20) > b.get(22, 20));
    }

    #[test]
    fn shadow_falloff_is_half_at_the_shape_edge() {
        let shape = square(120, 40, 60, 120, 0, 40);
        let l = shadow_layer(&shape, Shadow { color: BLACK, radius: 8.0, x: 0.0, y: 0.0, spread: 0.0 });
        let a = |x: usize| (l.get(x, 20) >> 24) as f32 / 255.0;
        assert!((a(60) - 0.5).abs() < 0.1, "edge {}", a(60));
        assert!(a(52) < a(56) && a(56) < a(60));
        assert!(a(40) < 0.01);
        assert!(a(70) > 0.97);
    }

    #[test]
    fn shadow_offset_moves_the_shadow() {
        let shape = square(40, 40, 15, 25, 15, 25);
        let l = shadow_layer(&shape, Shadow { color: BLACK, radius: 0.0, x: 6.0, y: 3.0, spread: 0.0 });
        assert_eq!(l.get(25, 20) >> 24, 255);
        assert_eq!(l.get(30, 27) >> 24, 255);
        assert_eq!(l.get(15, 15) >> 24, 0);
        assert_eq!(l.get(20, 13) >> 24, 0);
    }

    #[test]
    fn shadow_spread_grows_the_shape() {
        let shape = square(40, 40, 15, 25, 15, 25);
        let l = shadow_layer(&shape, Shadow { color: BLACK, radius: 0.0, x: 0.0, y: 0.0, spread: 3.0 });
        assert_eq!(l.get(12, 20) >> 24, 255);
        assert_eq!(l.get(10, 20) >> 24, 0);
    }

    #[test]
    fn mask_edge_is_anti_aliased_and_multiplies_alpha() {
        let mut l = Layer::new(10, 1);
        l.fill_plane(&Plane::filled(10, 1, 1.0), RED, Mode::Normal);
        let mut m = Plane::new(10, 1);
        for x in 0..5 {
            m.a[x] = 1.0;
        }
        m.a[5] = 0.5;
        l.mask(&m);
        assert_eq!(l.get(4, 0), 0xff00_00ff);
        assert_eq!(l.get(5, 0) >> 24, 128);
        assert_eq!(l.get(5, 0) & 0xff, 128);
        assert_eq!(l.get(6, 0), 0);
    }

    #[test]
    fn group_opacity_does_not_show_overlapping_children_through_each_other() {
        let mut group = Layer::new(3, 1);
        group.fill_plane(&square(3, 1, 0, 2, 0, 1), RED, Mode::Normal);
        group.fill_plane(&square(3, 1, 1, 3, 0, 1), GREEN, Mode::Normal);
        let mut dst = Layer::new(3, 1);
        dst.px.iter_mut().for_each(|p| *p = 0xffff_ffff);
        dst.draw_group(&group, 0.5, Mode::Normal, 0, 0);
        let p = dst.get(1, 0);
        assert!((p & 0xff) >= 127 && (p & 0xff) <= 128, "no red from the covered child");
        assert_eq!((p >> 8) & 0xff, 255);
        assert_eq!(dst.get(1, 0), dst.get(2, 0));
    }

    #[test]
    fn multiply_blend_darkens() {
        let mut dst = Layer::new(1, 1);
        dst.px[0] = premul(0xff80_8080, 1.0);
        let mut src = Layer::new(1, 1);
        src.px[0] = premul(0xff80_8080, 1.0);
        dst.draw_group(&src, 1.0, Mode::Multiply, 0, 0);
        let r = dst.get(0, 0) & 0xff;
        assert!((63..=65).contains(&r), "{r}");
    }
}
