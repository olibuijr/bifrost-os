//! Halftone and tile filters with the parameters of the reference Core Image filters:
//! `CIDotScreen`, `CILineScreen`, `CICircularScreen`, `CIHatchedScreen`, `CICMYKHalftone`,
//! `CIKaleidoscope`, `CIOpTile` and `CIAffineTile`. Pixels are premultiplied RGBA packed in a
//! `u32`, like `gfx::Image`; the output has the size of the input. Screens work on the luminance
//! (CMYK halftone per ink) and keep the alpha. `CIPerspectiveTile` and `CITriangleTile` are not
//! built yet.
use crate::distortion_filters::warp;
use crate::gfx::Image;

fn fin(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

fn ch(p: u32) -> [f32; 4] {
    [(p & 255) as f32, (p >> 8 & 255) as f32, (p >> 16 & 255) as f32, (p >> 24) as f32]
}

fn pk(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x + 0.5).clamp(0.0, 255.0) as u32;
    let a = q(v[3]);
    q(v[0]).min(a) | q(v[1]).min(a) << 8 | q(v[2]).min(a) << 16 | a << 24
}

/// Which screen pattern to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Dot,
    Line,
    Circular,
    Hatched,
}

/// Parameters shared by the screens, named as the reference input keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenParams {
    pub center: (f32, f32),
    /// `inputAngle`, radians (reference default 0).
    pub angle: f32,
    /// `inputWidth`: the period of the pattern in pixels (default 6).
    pub width: f32,
    /// `inputSharpness`: 0 is soft, 1 has hard edges (default 0.7).
    pub sharpness: f32,
}

impl Default for ScreenParams {
    fn default() -> Self {
        ScreenParams { center: (0.0, 0.0), angle: 0.0, width: 6.0, sharpness: 0.7 }
    }
}

/// Coverage 0..=1 of the pattern at `(x, y)` for the darkness `dark` (0 white, 1 black).
fn coverage(kind: Screen, p: &ScreenParams, x: f32, y: f32, dark: f32) -> f32 {
    if dark <= 0.0 {
        return 0.0;
    }
    let w = fin(p.width, 6.0).max(1.0);
    let (dx, dy) = (x - fin(p.center.0, 0.0), y - fin(p.center.1, 0.0));
    let (s, c) = fin(p.angle, 0.0).sin_cos();
    let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
    let soft = (1.0 - fin(p.sharpness, 0.7).clamp(0.0, 1.0)) * 0.5 + 0.02;
    let edge = |dist: f32, limit: f32| (0.5 + (limit - dist) / (soft * w)).clamp(0.0, 1.0);
    match kind {
        Screen::Dot => {
            let (fu, fv) = (u.rem_euclid(w) - w / 2.0, v.rem_euclid(w) - w / 2.0);
            edge((fu * fu + fv * fv).sqrt(), w * (dark / std::f32::consts::PI).sqrt())
        }
        Screen::Line => edge((v.rem_euclid(w) - w / 2.0).abs(), dark * w / 2.0),
        Screen::Circular => {
            let d = (dx * dx + dy * dy).sqrt();
            edge((d.rem_euclid(w) - w / 2.0).abs(), dark * w / 2.0)
        }
        Screen::Hatched => {
            let a = edge((v.rem_euclid(w) - w / 2.0).abs(), dark * w / 2.0);
            let b = edge((u.rem_euclid(w) - w / 2.0).abs(), (dark - 0.5).max(0.0) * w);
            a.max(b)
        }
    }
}

/// Applies a screen: the output is black where the pattern covers and white elsewhere, scaled by the
/// pixel's darkness, so a white pixel stays white and a black one turns solid black.
pub fn screen(img: &Image, kind: Screen, p: &ScreenParams) -> Image {
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let c = ch(img.px[y * img.w + x]);
            let straight = if c[3] > 0.0 { 255.0 / c[3] } else { 0.0 };
            let l = (c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114) * straight / 255.0;
            let dark = (1.0 - l).clamp(0.0, 1.0);
            let v = 255.0 * (1.0 - coverage(kind, p, x as f32 + 0.5, y as f32 + 0.5, dark));
            out.px[y * img.w + x] = pk([v * c[3] / 255.0, v * c[3] / 255.0, v * c[3] / 255.0, c[3]]);
        }
    }
    out
}

/// `CICMYKHalftone`: separates the colour into cyan, magenta, yellow and black, screens each at its
/// own angle (offsets of 15, 75, 0 and 45 degrees from `angle`) and recombines. `gcr` and `ucr`
/// are the grey component replacement and under colour removal, both 0..=1.
pub fn cmyk_halftone(img: &Image, p: &ScreenParams, gcr: f32, ucr: f32) -> Image {
    let (gcr, ucr) = (fin(gcr, 1.0).clamp(0.0, 1.0), fin(ucr, 0.5).clamp(0.0, 1.0));
    let angles = [15.0f32, 75.0, 0.0, 45.0];
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let c = ch(img.px[y * img.w + x]);
            let straight = if c[3] > 0.0 { 255.0 / c[3] } else { 0.0 };
            let rgb = [c[0] * straight / 255.0, c[1] * straight / 255.0, c[2] * straight / 255.0].map(|v| v.clamp(0.0, 1.0));
            let k0 = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
            let ink = [1.0 - rgb[0] - k0 * ucr, 1.0 - rgb[1] - k0 * ucr, 1.0 - rgb[2] - k0 * ucr, k0 * gcr];
            let mut res = [255.0f32; 3];
            for (i, &amount) in ink.iter().enumerate() {
                let q = ScreenParams { angle: fin(p.angle, 0.0) + angles[i].to_radians(), ..*p };
                let cov = coverage(Screen::Dot, &q, x as f32 + 0.5, y as f32 + 0.5, amount.clamp(0.0, 1.0));
                // multiplicative ink: cyan absorbs red, magenta green, yellow blue, black all
                for (ch_i, r) in res.iter_mut().enumerate() {
                    let absorbs = i == 3 || i == ch_i;
                    if absorbs {
                        *r *= 1.0 - cov;
                    }
                }
            }
            out.px[y * img.w + x] = pk([res[0] * c[3] / 255.0, res[1] * c[3] / 255.0, res[2] * c[3] / 255.0, c[3]]);
        }
    }
    out
}

/// `CIAffineTile`: repeats the image over the plane after applying the affine transform
/// `[a b c d tx ty]` (reference `inputTransform`): x' = a*x + c*y + tx, y' = b*x + d*y + ty.
/// The identity transform tiles the image at its own size. A singular transform returns a copy.
pub fn affine_tile(img: &Image, m: [f32; 6]) -> Image {
    let [a, b, c, d, tx, ty] = m.map(|v| fin(v, 0.0));
    let det = a * d - b * c;
    if img.w == 0 || img.h == 0 || det.abs() < 1e-6 {
        return img.clone();
    }
    let (w, h) = (img.w as f32, img.h as f32);
    warp(img, |x, y| {
        let (px, py) = (x - tx, y - ty);
        let (ux, uy) = ((d * px - c * py) / det, (-b * px + a * py) / det);
        (ux.rem_euclid(w), uy.rem_euclid(h))
    })
}

/// `CIOpTile`: tiles the image in a mirrored, scaled and rotated grid about `center`. `scale` is
/// the tile scale (default 2.8), `angle` the rotation in radians, `width` the tile width in pixels.
/// Alternate tiles are mirrored, as the reference does.
pub fn op_tile(img: &Image, center: (f32, f32), scale: f32, angle: f32, width: f32) -> Image {
    let (s, w) = (fin(scale, 1.0).max(0.01), fin(width, 65.0).max(1.0));
    let (sn, cs) = fin(angle, 0.0).sin_cos();
    let (cx, cy) = (fin(center.0, 0.0), fin(center.1, 0.0));
    let (iw, ih) = (img.w as f32, img.h as f32);
    warp(img, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let (u, v) = ((dx * cs + dy * sn) / (w * s), (-dx * sn + dy * cs) / (w * s));
        let fold = |t: f32| {
            let m = t.rem_euclid(2.0);
            if m > 1.0 { 2.0 - m } else { m }
        };
        // a tile samples inside the image: pixel centres only, so the edges never blend with transparent outside
        ((fold(u) * iw).clamp(0.5, iw - 0.5), (fold(v) * ih).clamp(0.5, ih - 0.5))
    })
}

/// `CIKaleidoscope`: `count` mirrored wedges about `center`, rotated by `angle` radians.
pub fn kaleidoscope(img: &Image, center: (f32, f32), count: u32, angle: f32) -> Image {
    let n = count.clamp(1, 64) as f32;
    let (cx, cy) = (fin(center.0, 0.0), fin(center.1, 0.0));
    let wedge = std::f32::consts::PI / n;
    let rot = fin(angle, 0.0);
    warp(img, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let r = (dx * dx + dy * dy).sqrt();
        let a = (dy.atan2(dx) - rot).rem_euclid(2.0 * wedge);
        let a = if a > wedge { 2.0 * wedge - a } else { a } + rot;
        (cx + r * a.cos(), cy + r * a.sin())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, p: u32) -> Image {
        let mut i = Image::new(w, h);
        i.px.iter_mut().for_each(|x| *x = p);
        i
    }

    #[test]
    fn a_white_image_stays_white_under_every_screen() {
        let i = flat(12, 12, 0xffff_ffff);
        for k in [Screen::Dot, Screen::Line, Screen::Circular, Screen::Hatched] {
            assert_eq!(screen(&i, k, &ScreenParams::default()).px, i.px, "{k:?}");
        }
    }

    #[test]
    fn a_black_image_goes_mostly_black() {
        let i = flat(12, 12, 0xff00_0000);
        let o = screen(&i, Screen::Line, &ScreenParams::default());
        assert!(o.px.iter().filter(|&&p| p & 255 < 128).count() > 100);
    }

    #[test]
    fn screens_keep_alpha() {
        let i = flat(4, 4, 0x8040_4040);
        assert!(screen(&i, Screen::Dot, &ScreenParams::default()).px.iter().all(|&p| p >> 24 == 0x80));
    }

    #[test]
    fn cmyk_of_white_is_white_and_of_black_is_dark() {
        assert_eq!(cmyk_halftone(&flat(8, 8, 0xffff_ffff), &ScreenParams::default(), 1.0, 0.5).px, flat(8, 8, 0xffff_ffff).px);
        let o = cmyk_halftone(&flat(16, 16, 0xff00_0000), &ScreenParams::default(), 1.0, 0.5);
        assert!(o.px.iter().any(|&p| p & 255 < 100));
    }

    #[test]
    fn identity_affine_tile_is_the_image() {
        let mut i = Image::new(3, 3);
        i.px.iter_mut().enumerate().for_each(|(k, p)| *p = 0xff00_0000 | (k as u32 * 20));
        assert_eq!(affine_tile(&i, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).px, i.px);
    }

    #[test]
    fn affine_tile_repeats_with_a_shift_and_survives_singular_input() {
        let mut i = Image::new(2, 1);
        i.px = vec![0xff00_00ff, 0xff00_ff00];
        let o = affine_tile(&i, [1.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
        assert_eq!(o.px, vec![0xff00_ff00, 0xff00_00ff]);
        assert_eq!(affine_tile(&i, [0.0; 6]).px, i.px);
    }

    #[test]
    fn kaleidoscope_with_one_wedge_mirrors_and_is_finite() {
        let i = flat(8, 8, 0xff12_3456);
        let o = kaleidoscope(&i, (4.0, 4.0), 6, 0.0);
        assert_eq!(o.px[4 * 8 + 4], 0xff12_3456);
        let _ = kaleidoscope(&i, (f32::NAN, 0.0), 0, f32::NAN);
    }

    #[test]
    fn op_tile_output_has_input_size() {
        let i = flat(8, 5, 0xff01_0203);
        let o = op_tile(&i, (4.0, 2.0), 2.8, 0.3, 65.0);
        assert_eq!((o.w, o.h), (8, 5));
        assert!(o.px.iter().any(|&p| p >> 24 == 255));
    }
}
