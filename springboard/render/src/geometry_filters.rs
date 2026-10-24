//! Geometry adjustment filters that `perspective` and `affine` do not cover:
//!
//! - `CIPerspectiveTransform` and `CIPerspectiveTransformWithExtent`: the image rectangle is mapped
//!   onto an arbitrary quadrilateral. The output covers the quad's bounding box (or a caller-chosen
//!   extent) and carries its origin, because the result can sit anywhere in the plane.
//! - `CILanczosScaleTransform` and `CIBicubicScaleTransform`: resampling by `scale` vertically and
//!   `scale * aspect_ratio` horizontally, with the Lanczos-3 window or the two-parameter cubic family
//!   (`b`, `c`; the reference defaults are b = 0 and c = 0.75). When an axis shrinks, the kernel is
//!   widened by the shrink factor so each output pixel averages its footprint.
//! - `CIStraightenFilter`: rotates the image about its centre and scales it up just enough that the
//!   rotated picture still covers the whole input rectangle, so no transparent corners appear.
//!
//! Coordinates are image pixels with the origin at the top left and y growing downwards; pixel `i`
//! covers `[i, i + 1)`. The reference works with y growing upwards, so a positive straighten angle
//! here turns the picture anticlockwise on screen, as the reference does. Pixels are premultiplied
//! RGBA packed with red in the low byte (`gfx::Image`); resampling filters premultiplied values and
//! clamps the negative lobes so colour never exceeds alpha. CPU only.
use crate::distortion_filters::sample;
use crate::gfx::Image;
use crate::perspective::{homography, map_point, Point};

/// The largest output side these filters produce.
pub const MAX_SIDE: usize = 16384;

/// A filter result with its position: the output's top-left pixel sits at (`x`, `y`) in the input's
/// coordinate space.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub image: Image,
    pub x: i64,
    pub y: i64,
}

fn well_formed(img: &Image) -> bool {
    img.w > 0 && img.h > 0 && img.w.checked_mul(img.h) == Some(img.px.len())
}

/// `CIPerspectiveTransform`: maps the corners of `img` (top-left, top-right, bottom-right,
/// bottom-left) onto `corners` and returns the picture over the corners' bounding box. `None` for a
/// malformed image, non-finite or degenerate corners, or a bounding box past `MAX_SIDE`.
pub fn perspective_transform(img: &Image, corners: [Point; 4]) -> Option<Placed> {
    if corners.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return None;
    }
    let min_x = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor();
    let min_y = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor();
    let max_x = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil();
    let max_y = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil();
    let (w, h) = ((max_x - min_x) as i64, (max_y - min_y) as i64);
    if w <= 0 || h <= 0 || w > MAX_SIDE as i64 || h > MAX_SIDE as i64 {
        return None;
    }
    perspective_transform_with_extent(img, corners, (min_x as i64, min_y as i64, w as usize, h as usize))
}

/// `CIPerspectiveTransformWithExtent`: as `perspective_transform`, but the output covers `extent`
/// (`x`, `y`, `w`, `h` in the input's coordinate space) whatever the corners are. Points outside the
/// mapped quad are transparent.
pub fn perspective_transform_with_extent(img: &Image, corners: [Point; 4], extent: (i64, i64, usize, usize)) -> Option<Placed> {
    let (ex, ey, w, h) = extent;
    if !well_formed(img) || w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let source = [(0.0, 0.0), (img.w as f32, 0.0), (img.w as f32, img.h as f32), (0.0, img.h as f32)];
    // Inverse mapping: a point of the quad back to the source rectangle. A projective map of a
    // convex quad is one to one on the plane, so points outside the quad land outside the source and
    // sample as transparent.
    let back = homography(corners, source)?;
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = ((ex + x as i64) as f32 + 0.5, (ey + y as i64) as f32 + 0.5);
            if let Some((sx, sy)) = map_point(&back, p) {
                out.px[y * w + x] = sample(img, sx, sy);
            }
        }
    }
    Some(Placed { image: out, x: ex, y: ey })
}

/// The cubic filter family with parameters `b` and `c`, of support 2.
pub fn cubic(b: f32, c: f32, t: f32) -> f32 {
    let x = t.abs();
    if x < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * x * x * x + (-18.0 + 12.0 * b + 6.0 * c) * x * x + (6.0 - 2.0 * b)) / 6.0
    } else if x < 2.0 {
        ((-b - 6.0 * c) * x * x * x + (6.0 * b + 30.0 * c) * x * x + (-12.0 * b - 48.0 * c) * x + (8.0 * b + 24.0 * c)) / 6.0
    } else {
        0.0
    }
}

/// For each output index along one axis, the first source index and the normalised weights.
fn taps(src: usize, dst: usize, support: f32, kernel: &dyn Fn(f32) -> f32) -> Vec<(usize, Vec<f32>)> {
    let scale = dst as f32 / src as f32;
    let widen = (1.0 / scale).max(1.0);
    let radius = support * widen;
    (0..dst)
        .map(|i| {
            let centre = (i as f32 + 0.5) / scale;
            let lo = ((centre - radius).floor().max(0.0) as usize).min(src - 1);
            let hi = ((centre + radius).ceil() as usize).clamp(lo + 1, src);
            let mut w: Vec<f32> = (lo..hi).map(|j| kernel((j as f32 + 0.5 - centre) / widen)).collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1e-6 {
                w.iter_mut().for_each(|v| *v /= sum);
            } else {
                // No weight at all: fall back to the nearest source sample.
                let near = (centre.floor() as usize).clamp(lo, hi - 1);
                w.iter_mut().enumerate().for_each(|(k, v)| *v = if lo + k == near { 1.0 } else { 0.0 });
            }
            (lo, w)
        })
        .collect()
}

/// Separable resampling of `img` to `w` by `h` with `kernel` of the given support.
fn resample(img: &Image, w: usize, h: usize, support: f32, kernel: &dyn Fn(f32) -> f32) -> Image {
    let xs = taps(img.w, w, support, kernel);
    let ys = taps(img.h, h, support, kernel);
    let unpack = |p: u32| [(p & 0xff) as f32, (p >> 8 & 0xff) as f32, (p >> 16 & 0xff) as f32, (p >> 24) as f32];
    // Rows first: img.h rows of w samples.
    let mut mid = vec![[0.0f32; 4]; w * img.h];
    for y in 0..img.h {
        let row = &img.px[y * img.w..(y + 1) * img.w];
        for (x, (lo, wt)) in xs.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for (k, &f) in wt.iter().enumerate() {
                let v = unpack(row[lo + k]);
                for c in 0..4 {
                    acc[c] += f * v[c];
                }
            }
            mid[y * w + x] = acc;
        }
    }
    let mut out = Image::new(w, h);
    for (y, (lo, wt)) in ys.iter().enumerate() {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for (k, &f) in wt.iter().enumerate() {
                let v = mid[(lo + k) * w + x];
                for c in 0..4 {
                    acc[c] += f * v[c];
                }
            }
            let a = (acc[3] + 0.5).clamp(0.0, 255.0) as u32;
            let q = |v: f32| ((v + 0.5).clamp(0.0, 255.0) as u32).min(a);
            out.px[y * w + x] = q(acc[0]) | q(acc[1]) << 8 | q(acc[2]) << 16 | a << 24;
        }
    }
    out
}

/// The output size of a scale transform: `scale * aspect_ratio` across and `scale` down, rounded
/// and at least one pixel. `None` for a non-positive or non-finite factor or a side past `MAX_SIDE`.
pub fn scaled_size(w: usize, h: usize, scale: f32, aspect_ratio: f32) -> Option<(usize, usize)> {
    let (sx, sy) = (scale * aspect_ratio, scale);
    if !(sx.is_finite() && sy.is_finite() && sx > 0.0 && sy > 0.0) {
        return None;
    }
    let ow = ((w as f32 * sx).round() as usize).max(1);
    let oh = ((h as f32 * sy).round() as usize).max(1);
    (ow <= MAX_SIDE && oh <= MAX_SIDE).then_some((ow, oh))
}

/// `CILanczosScaleTransform` (reference defaults: scale 1, aspect ratio 1).
pub fn lanczos_scale_transform(img: &Image, scale: f32, aspect_ratio: f32) -> Option<Image> {
    if !well_formed(img) {
        return None;
    }
    let (w, h) = scaled_size(img.w, img.h, scale, aspect_ratio)?;
    Some(resample(img, w, h, crate::lanczos::LOBES, &crate::lanczos::lanczos))
}

/// `CIBicubicScaleTransform` (reference defaults: scale 1, aspect ratio 1, b 0, c 0.75). `b` and `c`
/// are clamped to 0..=1, the reference range.
pub fn bicubic_scale_transform(img: &Image, scale: f32, aspect_ratio: f32, b: f32, c: f32) -> Option<Image> {
    if !well_formed(img) {
        return None;
    }
    let (w, h) = scaled_size(img.w, img.h, scale, aspect_ratio)?;
    let clean = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let (b, c) = (clean(b), clean(c));
    Some(resample(img, w, h, 2.0, &move |t| cubic(b, c, t)))
}

/// The zoom `CIStraightenFilter` applies for `angle` so the rotated `w` by `h` picture covers the
/// whole `w` by `h` rectangle.
pub fn straighten_zoom(w: usize, h: usize, angle: f32) -> f32 {
    let (c, s) = (angle.cos().abs(), angle.sin().abs());
    let ratio = (w as f32 / h as f32).max(h as f32 / w as f32);
    c + ratio * s
}

/// `CIStraightenFilter`: turns `img` anticlockwise by `angle` radians about its centre and zooms in so
/// the corners stay covered. The output has the input's size. `None` for a malformed image or a
/// non-finite angle.
pub fn straighten(img: &Image, angle: f32) -> Option<Image> {
    if !well_formed(img) || !angle.is_finite() {
        return None;
    }
    if angle == 0.0 {
        return Some(img.clone());
    }
    let zoom = straighten_zoom(img.w, img.h, angle);
    let (cx, cy) = (img.w as f32 / 2.0, img.h as f32 / 2.0);
    let (c, s) = (angle.cos(), angle.sin());
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            // Inverse of an anticlockwise turn on a y-down screen, then undo the zoom.
            // The zoom keeps every output point inside the source; clamping to the outer pixel
            // centres only stops the bilinear edge from fading the border pixels.
            let sx = ((dx * c - dy * s) / zoom + cx).clamp(0.5, img.w as f32 - 0.5);
            let sy = ((dx * s + dy * c) / zoom + cy).clamp(0.5, img.h as f32 - 0.5);
            out.px[y * img.w + x] = sample(img, sx, sy);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: usize, h: usize) -> Image {
        let mut i = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                i.px[y * w + x] = (x as u32 * 20) | ((y as u32 * 20) << 8) | 0xff00_0000;
            }
        }
        i
    }

    fn rect(w: f32, h: f32) -> [Point; 4] {
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }

    #[test]
    fn perspective_onto_its_own_rectangle_is_identity() {
        let img = ramp(5, 4);
        let out = perspective_transform(&img, rect(5.0, 4.0)).unwrap();
        assert_eq!((out.x, out.y), (0, 0));
        assert_eq!(out.image, img);
    }

    #[test]
    fn perspective_translation_moves_the_origin_and_keeps_the_pixels() {
        let img = ramp(4, 4);
        let corners = rect(4.0, 4.0).map(|p| (p.0 + 3.0, p.1 - 2.0));
        let out = perspective_transform(&img, corners).unwrap();
        assert_eq!((out.x, out.y), (3, -2));
        assert_eq!(out.image, img);
    }

    #[test]
    fn perspective_output_covers_the_bounding_box_and_a_trapezoid_leaves_clear_corners() {
        let img = ramp(4, 4);
        let out = perspective_transform(&img, rect(8.0, 8.0)).unwrap();
        assert_eq!((out.image.w, out.image.h), (8, 8));
        // A trapezoid narrower at the top: the top corners of the box are outside the quad.
        let trap = [(3.0, 0.0), (5.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        let t = perspective_transform(&img, trap).unwrap();
        assert_eq!(t.image.px[0] >> 24, 0);
        assert_eq!(t.image.px[7] >> 24, 0);
        assert_eq!(t.image.px[4 * 8 + 4] >> 24, 255);
    }

    #[test]
    fn perspective_refuses_degenerate_or_non_finite_corners() {
        let img = ramp(4, 4);
        assert!(perspective_transform(&img, [(1.0, 1.0); 4]).is_none());
        assert!(perspective_transform(&img, [(f32::NAN, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]).is_none());
        assert!(perspective_transform_with_extent(&img, rect(4.0, 4.0), (0, 0, 0, 4)).is_none());
    }

    #[test]
    fn extent_crops_the_result() {
        let img = ramp(4, 4);
        let out = perspective_transform_with_extent(&img, rect(4.0, 4.0), (2, 1, 2, 2)).unwrap();
        assert_eq!(out.image.px, vec![img.px[4 + 2], img.px[4 + 3], img.px[8 + 2], img.px[8 + 3]]);
    }

    #[test]
    fn cubic_defaults_interpolate() {
        assert!((cubic(0.0, 0.75, 0.0) - 1.0).abs() < 1e-6);
        assert!(cubic(0.0, 0.75, 1.0).abs() < 1e-6);
        assert!(cubic(0.0, 0.75, 2.0).abs() < 1e-6);
        assert!(cubic(0.0, 0.75, 1.5) < 0.0, "negative lobe sharpens");
    }

    #[test]
    fn unit_scale_is_identity_for_lanczos_and_bicubic() {
        let img = ramp(6, 5);
        assert_eq!(lanczos_scale_transform(&img, 1.0, 1.0).unwrap(), img);
        assert_eq!(bicubic_scale_transform(&img, 1.0, 1.0, 0.0, 0.75).unwrap(), img);
    }

    #[test]
    fn scale_and_aspect_ratio_set_the_size() {
        let img = ramp(10, 8);
        let up = lanczos_scale_transform(&img, 2.0, 1.0).unwrap();
        assert_eq!((up.w, up.h), (20, 16));
        let wide = bicubic_scale_transform(&img, 0.5, 2.0, 0.0, 0.75).unwrap();
        assert_eq!((wide.w, wide.h), (10, 4));
        assert!(lanczos_scale_transform(&img, 0.0, 1.0).is_none());
        assert!(bicubic_scale_transform(&img, f32::NAN, 1.0, 0.0, 0.75).is_none());
        assert_eq!(scaled_size(3, 3, 0.01, 1.0), Some((1, 1)));
    }

    #[test]
    fn shrinking_a_flat_image_keeps_its_colour_and_colour_stays_under_alpha() {
        let flat = Image { w: 9, h: 9, px: vec![0x8040_2010; 81] };
        let small = lanczos_scale_transform(&flat, 1.0 / 3.0, 1.0).unwrap();
        assert_eq!((small.w, small.h), (3, 3));
        assert!(small.px.iter().all(|&p| p == 0x8040_2010));
        // A hard edge rings under Lanczos; premultiplied colour must still not pass alpha.
        let mut edge = Image::new(8, 1);
        edge.px[4..].iter_mut().for_each(|p| *p = 0xffff_ffff);
        for p in lanczos_scale_transform(&edge, 2.5, 1.0).unwrap().px {
            let a = p >> 24;
            assert!((p & 0xff) <= a && (p >> 8 & 0xff) <= a && (p >> 16 & 0xff) <= a);
        }
    }

    #[test]
    fn straighten_zero_is_identity_and_a_small_angle_keeps_the_corners_covered() {
        let img = ramp(12, 8);
        assert_eq!(straighten(&img, 0.0).unwrap(), img);
        let out = straighten(&img, 0.2).unwrap();
        for i in [0, 11, 7 * 12, 7 * 12 + 11] {
            assert_eq!(out.px[i] >> 24, 255, "corner {i} is covered");
        }
        assert!(straighten(&img, f32::INFINITY).is_none());
    }

    #[test]
    fn straighten_a_quarter_turn_moves_the_top_right_corner_to_the_top_left() {
        let img = ramp(4, 4);
        let out = straighten(&img, std::f32::consts::FRAC_PI_2).unwrap();
        assert!((straighten_zoom(4, 4, std::f32::consts::FRAC_PI_2) - 1.0).abs() < 1e-5);
        assert_eq!(out.px[0], img.px[3]);
    }
}
