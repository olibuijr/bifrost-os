//! Distortion filters with the parameters of the reference Core Image filters: `CITwirlDistortion`,
//! `CIPinchDistortion`, `CIBumpDistortion`, `CIVortexDistortion` and `CIGlassDistortion`.
//! Pixels are premultiplied RGBA packed in a `u32` (red in the low byte), like `gfx::Image`.
//! Every filter is a backwards warp: for each output pixel it computes a source position and reads
//! it with bilinear filtering; positions outside the image read transparent black. Pixel
//! coordinates are in image space with the origin at the top left; `center` is in pixels.
//! Differences from the reference: the falloff is linear in the distance from the centre, and the
//! output has the size of the input. `CICircleSplashDistortion` and `CITorusLensDistortion` live in
//! `lens_distortions`, the one implementation the SDK (`image_distort`) and the filter catalogue use.
use crate::gfx::Image;

/// Bilinear sample at `(x, y)` in pixel-centre coordinates; outside reads transparent black.
pub(crate) fn sample(img: &Image, x: f32, y: f32) -> u32 {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |ix: f32, iy: f32| -> [f32; 4] {
        if ix < 0.0 || iy < 0.0 || ix >= img.w as f32 || iy >= img.h as f32 {
            return [0.0; 4];
        }
        let p = img.px[iy as usize * img.w + ix as usize];
        [(p & 255) as f32, (p >> 8 & 255) as f32, (p >> 16 & 255) as f32, (p >> 24) as f32]
    };
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let mut o = [0u32; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bot = c[i] + (d[i] - c[i]) * tx;
        o[i] = (top + (bot - top) * ty + 0.5).clamp(0.0, 255.0) as u32;
    }
    o[0] | o[1] << 8 | o[2] << 16 | o[3] << 24
}

pub(crate) fn warp(img: &Image, f: impl Fn(f32, f32) -> (f32, f32)) -> Image {
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (sx, sy) = f(x as f32 + 0.5, y as f32 + 0.5);
            out.px[y * img.w + x] = sample(img, sx, sy);
        }
    }
    out
}

fn clean(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// `CITwirlDistortion`: rotates the pixels around `center` by up to `angle` radians at the centre,
/// fading to no rotation at `radius`.
pub fn twirl(img: &Image, center: (f32, f32), radius: f32, angle: f32) -> Image {
    swirl(img, center, radius, angle, 1.0)
}

/// `CIVortexDistortion`: like the twirl but the rotation falls off with the square of the distance
/// from the centre, so it is tighter at the middle.
pub fn vortex(img: &Image, center: (f32, f32), radius: f32, angle: f32) -> Image {
    swirl(img, center, radius, angle, 2.0)
}

fn swirl(img: &Image, c: (f32, f32), radius: f32, angle: f32, power: f32) -> Image {
    let r = clean(radius);
    let angle = if angle.is_finite() { angle } else { 0.0 };
    warp(img, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if r <= 0.0 || d >= r || angle == 0.0 {
            return (x, y);
        }
        let a = angle * (1.0 - d / r).powf(power);
        let (s, co) = a.sin_cos();
        (c.0 + dx * co + dy * s, c.1 - dx * s + dy * co)
    })
}

/// `CIPinchDistortion`: pulls the pixels towards `center`. `scale` is 0 for no change up to 1 for
/// the strongest pinch; the effect fades to nothing at `radius`.
pub fn pinch(img: &Image, center: (f32, f32), radius: f32, scale: f32) -> Image {
    radial(img, center, radius, scale.clamp(0.0, 1.0), true)
}

/// `CIBumpDistortion`: a bulge (positive `scale`) or dent (negative) around `center`, in -1..=1.
pub fn bump(img: &Image, center: (f32, f32), radius: f32, scale: f32) -> Image {
    radial(img, center, radius, scale.clamp(-1.0, 1.0), false)
}

fn radial(img: &Image, c: (f32, f32), radius: f32, scale: f32, pinch: bool) -> Image {
    let r = clean(radius);
    let scale = if scale.is_finite() { scale } else { 0.0 };
    warp(img, |x, y| {
        let (dx, dy) = (x - c.0, y - c.1);
        let d = (dx * dx + dy * dy).sqrt();
        if r <= 0.0 || d >= r || d == 0.0 || scale == 0.0 {
            return (x, y);
        }
        let fall = 1.0 - d / r;
        let k = if pinch { 1.0 + scale * fall } else { 1.0 - scale * fall };
        (c.0 + dx * k, c.1 + dy * k)
    })
}

/// `CIGlassDistortion`: displaces each pixel by the slope of the `texture` image's luminance, like
/// looking through textured glass. `scale` is the displacement in pixels per full luminance step.
/// The texture is tiled over the image.
pub fn glass(img: &Image, texture: &Image, scale: f32) -> Image {
    let s = if scale.is_finite() { scale } else { 0.0 };
    if texture.w == 0 || texture.h == 0 || s == 0.0 {
        return img.clone();
    }
    let lum = |x: i64, y: i64| -> f32 {
        let p = texture.px[y.rem_euclid(texture.h as i64) as usize * texture.w + x.rem_euclid(texture.w as i64) as usize];
        ((p & 255) as f32 * 0.299 + (p >> 8 & 255) as f32 * 0.587 + (p >> 16 & 255) as f32 * 0.114) / 255.0
    };
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (xi, yi) = (x as i64, y as i64);
            let gx = lum(xi + 1, yi) - lum(xi - 1, yi);
            let gy = lum(xi, yi + 1) - lum(xi, yi - 1);
            out.px[y * img.w + x] = sample(img, x as f32 + 0.5 + gx * s, y as f32 + 0.5 + gy * s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Image {
        let mut i = Image::new(n, n);
        for (k, p) in i.px.iter_mut().enumerate() {
            *p = (k as u32 % 251) | 0xff00_0000;
        }
        i
    }

    #[test]
    fn zero_strength_is_identity() {
        let i = ramp(8);
        let c = (4.0, 4.0);
        assert_eq!(twirl(&i, c, 3.0, 0.0).px, i.px);
        assert_eq!(vortex(&i, c, 3.0, 0.0).px, i.px);
        assert_eq!(pinch(&i, c, 3.0, 0.0).px, i.px);
        assert_eq!(bump(&i, c, 3.0, 0.0).px, i.px);
        assert_eq!(glass(&i, &i, 0.0).px, i.px);
    }

    #[test]
    fn zero_radius_is_identity() {
        let i = ramp(8);
        assert_eq!(twirl(&i, (4.0, 4.0), 0.0, 3.0).px, i.px);
    }

    #[test]
    fn pixels_outside_the_radius_are_untouched() {
        let i = ramp(16);
        let o = twirl(&i, (8.0, 8.0), 3.0, 2.0);
        assert_eq!(o.px[0], i.px[0]);
        assert_ne!(o.px, i.px);
    }

    #[test]
    fn bump_and_pinch_move_pixels_in_opposite_directions() {
        let mut i = Image::new(9, 9);
        i.px.iter_mut().for_each(|p| *p = 0xff00_0000);
        i.px[4 * 9 + 5] = 0xffff_ffff;
        let b = bump(&i, (4.5, 4.5), 4.0, 1.0);
        let p = pinch(&i, (4.5, 4.5), 4.0, 1.0);
        assert_ne!(b.px, p.px);
        assert_ne!(b.px, i.px);
    }

    #[test]
    fn nan_parameters_do_not_panic() {
        let i = ramp(4);
        let _ = twirl(&i, (f32::NAN, 1.0), f32::NAN, f32::INFINITY);
        let _ = glass(&i, &Image::new(0, 0), f32::NAN);
    }

    #[test]
    fn glass_on_a_flat_texture_does_not_move_pixels() {
        let i = ramp(6);
        let mut t = Image::new(2, 2);
        t.px.iter_mut().for_each(|p| *p = 0xff80_8080);
        assert_eq!(glass(&i, &t, 10.0).px, i.px);
    }
}
