//! Sharpen and stylize filters with the parameters of the reference Core Image filters:
//! `CISharpenLuminance`, `CIUnsharpMask`, `CIBloom`, `CIGloom`, `CICrystallize`, `CIPixellate`
//! and `CIPointillize`. Pixels are premultiplied RGBA packed in a `u32`, like `gfx::Image`; the
//! output has the size of the input.
use crate::blur_filters::{gaussian_blur, Edge};
use crate::gfx::Image;

fn ch(p: u32) -> [f32; 4] {
    [(p & 255) as f32, (p >> 8 & 255) as f32, (p >> 16 & 255) as f32, (p >> 24) as f32]
}

fn pk(v: [f32; 4]) -> u32 {
    let q = |x: f32| (x + 0.5).clamp(0.0, 255.0) as u32;
    let a = q(v[3]);
    q(v[0]).min(a) | q(v[1]).min(a) << 8 | q(v[2]).min(a) << 16 | a << 24
}

fn fin(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// `CIUnsharpMask`: `out = in + intensity * (in - blur(in, radius))`. Intensity 0 is the identity.
pub fn unsharp_mask(img: &Image, radius: f32, intensity: f32) -> Image {
    let intensity = fin(intensity, 0.0);
    let blurred = gaussian_blur(img, fin(radius, 0.0).max(0.0), Edge::Clamp);
    let mut out = img.clone();
    for ((o, &s), &b) in out.px.iter_mut().zip(&img.px).zip(&blurred.px) {
        let (s, b) = (ch(s), ch(b));
        *o = pk([s[0] + intensity * (s[0] - b[0]), s[1] + intensity * (s[1] - b[1]), s[2] + intensity * (s[2] - b[2]), s[3]]);
    }
    out
}

/// `CISharpenLuminance`: sharpens only the luminance. `sharpness` is the amount (reference default
/// 0.4) and `radius` the blur radius of the detail extraction (default 1.69).
pub fn sharpen_luminance(img: &Image, sharpness: f32, radius: f32) -> Image {
    let k = fin(sharpness, 0.0);
    let blurred = gaussian_blur(img, fin(radius, 0.0).max(0.0), Edge::Clamp);
    let lum = |c: [f32; 4]| c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114;
    let mut out = img.clone();
    for ((o, &s), &b) in out.px.iter_mut().zip(&img.px).zip(&blurred.px) {
        let s = ch(s);
        let d = k * (lum(s) - lum(ch(b)));
        *o = pk([s[0] + d, s[1] + d, s[2] + d, s[3]]);
    }
    out
}

/// `CIBloom`: softens the image and makes the bright parts glow. Screens a blurred copy over the
/// original by `intensity` (0 is the identity).
pub fn bloom(img: &Image, radius: f32, intensity: f32) -> Image {
    glow(img, radius, intensity, false)
}

/// `CIGloom`: dulls the highlights; the blurred copy is mixed in at half brightness, so the image
/// darkens where it was bright.
pub fn gloom(img: &Image, radius: f32, intensity: f32) -> Image {
    glow(img, radius, intensity, true)
}

fn glow(img: &Image, radius: f32, intensity: f32, dull: bool) -> Image {
    let t = fin(intensity, 0.0).clamp(0.0, 1.0);
    let blurred = gaussian_blur(img, fin(radius, 0.0).max(0.0), Edge::Clamp);
    let mut out = img.clone();
    for ((o, &s), &b) in out.px.iter_mut().zip(&img.px).zip(&blurred.px) {
        let (s, b) = (ch(s), ch(b));
        let f = |i: usize| {
            if dull {
                s[i] + t * (b[i] * 0.5 - s[i])
            } else {
                s[i] + t * (255.0 - (255.0 - s[i]) * (255.0 - b[i]) / 255.0 - s[i])
            }
        };
        *o = pk([f(0), f(1), f(2), s[3]]);
    }
    out
}

/// `CIPixellate`: replaces each `scale` by `scale` block (aligned to `center`) by its mean colour.
pub fn pixellate(img: &Image, center: (f32, f32), scale: f32) -> Image {
    let s = fin(scale, 1.0).max(1.0);
    if s <= 1.0 {
        return img.clone();
    }
    let center = (fin(center.0, 0.0), fin(center.1, 0.0));
    let key = |x: usize, y: usize| (((x as f32 + 0.5 - center.0) / s).floor() as i64, ((y as f32 + 0.5 - center.1) / s).floor() as i64);
    let mut sums: std::collections::HashMap<(i64, i64), ([f32; 4], f32)> = std::collections::HashMap::new();
    for y in 0..img.h {
        for x in 0..img.w {
            let e = sums.entry(key(x, y)).or_insert(([0.0; 4], 0.0));
            let c = ch(img.px[y * img.w + x]);
            for i in 0..4 {
                e.0[i] += c[i];
            }
            e.1 += 1.0;
        }
    }
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (sum, n) = sums[&key(x, y)];
            out.px[y * img.w + x] = pk(sum.map(|v| v / n));
        }
    }
    out
}

/// Cell seeds: a jittered grid with spacing `radius * 2`, deterministic.
fn seeds(w: usize, h: usize, radius: f32) -> Vec<(f32, f32)> {
    let step = (radius * 2.0).max(1.0);
    let mut v = Vec::new();
    let (mut gy, mut y) = (0u32, step / 2.0);
    while y < h as f32 + step {
        let (mut gx, mut x) = (0u32, step / 2.0);
        while x < w as f32 + step {
            let hsh = gx.wrapping_mul(73856093) ^ gy.wrapping_mul(19349663);
            let jx = ((hsh & 0xff) as f32 / 255.0 - 0.5) * step * 0.5;
            let jy = ((hsh >> 8 & 0xff) as f32 / 255.0 - 0.5) * step * 0.5;
            v.push((x + jx, y + jy));
            gx += 1;
            x += step;
        }
        gy += 1;
        y += step;
    }
    v
}

fn nearest(seeds: &[(f32, f32)], x: f32, y: f32) -> (usize, f32) {
    let mut best = (0, f32::MAX);
    for (i, s) in seeds.iter().enumerate() {
        let d = (s.0 - x).powi(2) + (s.1 - y).powi(2);
        if d < best.1 {
            best = (i, d);
        }
    }
    best
}

/// `CICrystallize`: polygon cells of about `radius` pixels, each filled with its mean colour.
pub fn crystallize(img: &Image, radius: f32) -> Image {
    let r = fin(radius, 0.0).min(512.0);
    if r < 1.0 {
        return img.clone();
    }
    let sd = seeds(img.w, img.h, r);
    let mut acc = vec![([0.0f32; 4], 0.0f32); sd.len()];
    let mut idx = vec![0usize; img.px.len()];
    for y in 0..img.h {
        for x in 0..img.w {
            let (i, _) = nearest(&sd, x as f32 + 0.5, y as f32 + 0.5);
            idx[y * img.w + x] = i;
            let c = ch(img.px[y * img.w + x]);
            for k in 0..4 {
                acc[i].0[k] += c[k];
            }
            acc[i].1 += 1.0;
        }
    }
    let mut out = Image::new(img.w, img.h);
    for (o, &i) in out.px.iter_mut().zip(&idx) {
        *o = pk(acc[i].0.map(|v| v / acc[i].1));
    }
    out
}

/// `CIPointillize`: round dots of up to `radius` pixels in the colour at each cell centre over a
/// transparent background.
pub fn pointillize(img: &Image, radius: f32) -> Image {
    let r = fin(radius, 0.0).min(512.0);
    if r < 1.0 || img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let sd = seeds(img.w, img.h, r);
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (i, d2) = nearest(&sd, x as f32 + 0.5, y as f32 + 0.5);
            if d2 <= r * r {
                let sx = sd[i].0.clamp(0.0, img.w as f32 - 1.0) as usize;
                let sy = sd[i].1.clamp(0.0, img.h as f32 - 1.0) as usize;
                out.px[y * img.w + x] = img.px[sy * img.w + sx];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: usize, p: u32) -> Image {
        let mut i = Image::new(n, n);
        i.px.iter_mut().for_each(|x| *x = p);
        i
    }

    fn edge() -> Image {
        let mut i = flat(8, 0xff20_2020);
        for y in 0..8 {
            for x in 4..8 {
                i.px[y * 8 + x] = 0xffe0_e0e0;
            }
        }
        i
    }

    #[test]
    fn zero_intensity_is_identity() {
        let i = edge();
        assert_eq!(unsharp_mask(&i, 2.0, 0.0).px, i.px);
        assert_eq!(sharpen_luminance(&i, 0.0, 1.69).px, i.px);
        assert_eq!(bloom(&i, 2.0, 0.0).px, i.px);
        assert_eq!(gloom(&i, 2.0, 0.0).px, i.px);
    }

    #[test]
    fn sharpening_a_flat_image_changes_nothing() {
        let i = flat(6, 0xff80_6040);
        assert_eq!(unsharp_mask(&i, 2.0, 2.0).px, i.px);
        assert_eq!(sharpen_luminance(&i, 1.0, 2.0).px, i.px);
    }

    #[test]
    fn unsharp_mask_steepens_an_edge() {
        let i = edge();
        let o = unsharp_mask(&i, 1.5, 1.0);
        assert!(o.px[3] & 255 < i.px[3] & 255);
        assert!(o.px[4] & 255 > i.px[4] & 255);
    }

    #[test]
    fn bloom_brightens_and_gloom_darkens() {
        let i = edge();
        assert!(bloom(&i, 2.0, 1.0).px[3] & 255 > i.px[3] & 255);
        assert!(gloom(&i, 2.0, 1.0).px[7] & 255 < i.px[7] & 255);
    }

    #[test]
    fn pixellate_averages_blocks() {
        let mut i = Image::new(4, 4);
        i.px.iter_mut().enumerate().for_each(|(k, p)| *p = if k % 4 < 2 { 0xff00_0000 } else { 0xffff_ffff });
        let o = pixellate(&i, (0.0, 0.0), 4.0);
        assert!(o.px.iter().all(|&p| p == o.px[0]));
        assert_eq!(o.px[0] & 255, 128);
        assert_eq!(pixellate(&i, (0.0, 0.0), 1.0).px, i.px);
    }

    #[test]
    fn crystallize_keeps_a_flat_image() {
        let i = flat(10, 0xff40_8020);
        assert_eq!(crystallize(&i, 3.0).px, i.px);
        assert_eq!(crystallize(&i, 0.0).px, i.px);
    }

    #[test]
    fn pointillize_leaves_gaps_transparent() {
        let o = pointillize(&flat(16, 0xffff_ffff), 2.0);
        assert!(o.px.contains(&0));
        assert!(o.px.contains(&0xffff_ffff));
    }

    #[test]
    fn non_finite_input_is_safe() {
        let i = edge();
        let _ = unsharp_mask(&i, f32::NAN, f32::NAN);
        let _ = pixellate(&i, (f32::NAN, 0.0), f32::NAN);
    }
}
