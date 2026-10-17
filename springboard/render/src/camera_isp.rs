//! Camera image-processing kernels on single-channel `f32` planes (std only): the stages a photo pipeline runs between
//! the sensor and the display, each as a pure function you can run per colour plane or on luma.
//!
//! - `lens_shading_gain` and `apply_gain`: a radial gain that flattens vignetting.
//! - `non_local_means`: edge-preserving noise reduction (the bilateral filter and unsharp mask are in
//!   `kernel_photo_adjust`).
//! - `merge_burst`: align a burst to its first frame by block search, then average with weights that reject blocks that
//!   did not match (moving subjects), the core of multi-frame noise reduction.
//! - `exposure_fusion`: blend an exposure bracket by contrast and well-exposedness through Laplacian pyramids
//!   (Mertens, Kautz and Van Reeth), with no HDR intermediate.
//! - `local_tone_map`: compress the base layer of the log luminance (found with the guided filter) and keep the detail
//!   layer, so shadows lift and highlights settle without flattening texture.
//!
//! Planes are `effects::Plane` (row-major, values nominally 0..=1 but not clamped). Border pixels use clamped
//! coordinates unless a function says otherwise. Stages that find offsets or weights by search are written for clarity;
//! a GPU version tiles the same loops.

use crate::effects::Plane;
use crate::guided::guided_filter;

/// The value at `(x, y)` with coordinates clamped into the plane.
fn at(p: &Plane, x: i32, y: i32) -> f32 {
    let xc = x.clamp(0, p.w as i32 - 1) as usize;
    let yc = y.clamp(0, p.h as i32 - 1) as usize;
    p.a[yc * p.w + xc]
}

fn same_size(planes: &[Plane]) -> bool {
    planes.iter().all(|p| p.w == planes[0].w && p.h == planes[0].h && p.w > 0 && p.h > 0 && p.a.len() == p.w * p.h)
}

/// A radial gain plane: 1 at the centre rising smoothly to `corner_gain` at the corners (`1 + (corner_gain - 1) r^2`
/// with `r` the distance from the centre relative to the corner distance).
pub fn lens_shading_gain(w: usize, h: usize, corner_gain: f32) -> Plane {
    let mut g = Plane::new(w, h);
    let (cx, cy) = ((w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5);
    let norm = cx * cx + cy * cy;
    for y in 0..h {
        for x in 0..w {
            let r2 = if norm < 1e-12 { 0.0 } else { ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)) / norm };
            g.a[y * w + x] = 1.0 + (corner_gain - 1.0) * r2;
        }
    }
    g
}

/// Multiplies a plane by a gain plane of the same size. `None` when the sizes differ.
pub fn apply_gain(p: &Plane, gain: &Plane) -> Option<Plane> {
    if p.w != gain.w || p.h != gain.h {
        return None;
    }
    Some(Plane { w: p.w, h: p.h, a: p.a.iter().zip(&gain.a).map(|(v, g)| v * g).collect() })
}

/// Non-local means (Buades, Coll and Morel): each pixel is the mean of the pixels in a `(2 search_radius + 1)` window
/// weighted by how much their `(2 patch_radius + 1)` patches resemble this pixel's patch, `exp(-d / h^2)` with `d` the
/// mean squared patch difference. Larger `h` smooths more. `None` for an empty plane or non-positive `h`.
pub fn non_local_means(p: &Plane, patch_radius: usize, search_radius: usize, h: f32) -> Option<Plane> {
    if p.w == 0 || p.h == 0 || p.a.len() != p.w * p.h || !(h > 0.0) {
        return None;
    }
    let (pr, sr) = (patch_radius as i32, search_radius as i32);
    let count = ((2 * pr + 1) * (2 * pr + 1)) as f32;
    let mut out = Plane::new(p.w, p.h);
    for y in 0..p.h as i32 {
        for x in 0..p.w as i32 {
            let (mut sum, mut wsum) = (0.0f32, 0.0f32);
            for sy in -sr..=sr {
                for sx in -sr..=sr {
                    let mut d = 0.0f32;
                    for py in -pr..=pr {
                        for px in -pr..=pr {
                            let diff = at(p, x + px, y + py) - at(p, x + sx + px, y + sy + py);
                            d += diff * diff;
                        }
                    }
                    let w = (-(d / count) / (h * h)).exp();
                    sum += w * at(p, x + sx, y + sy);
                    wsum += w;
                }
            }
            out.a[y as usize * p.w + x as usize] = sum / wsum;
        }
    }
    Some(out)
}

/// Merges a burst into its first frame. The reference is cut into `block` pixel squares; for each other frame a search
/// of `+-range` pixels finds the offset with the smallest mean absolute difference, and the aligned block joins the
/// average with weight `1 / (1 + (mad / tolerance)^2)`, so a block that still disagrees (motion, occlusion) barely
/// counts. `None` for an empty burst, frames of different sizes, a zero block or a non-positive tolerance.
pub fn merge_burst(frames: &[Plane], block: usize, range: usize, tolerance: f32) -> Option<Plane> {
    if frames.is_empty() || !same_size(frames) || block == 0 || !(tolerance > 0.0) {
        return None;
    }
    let (w, h) = (frames[0].w, frames[0].h);
    let reference = &frames[0];
    let mut out = Plane::new(w, h);
    let r = range as i32;
    let mut by = 0;
    while by < h {
        let bh = block.min(h - by);
        let mut bx = 0;
        while bx < w {
            let bw = block.min(w - bx);
            let mut sum: Vec<f32> = Vec::with_capacity(bw * bh);
            for y in 0..bh {
                for x in 0..bw {
                    sum.push(reference.a[(by + y) * w + bx + x]);
                }
            }
            let mut wsum = 1.0f32;
            for frame in &frames[1..] {
                let mad_at = |dx: i32, dy: i32| -> f32 {
                    let mut m = 0.0f32;
                    for y in 0..bh {
                        for x in 0..bw {
                            let a = reference.a[(by + y) * w + bx + x];
                            let b = at(frame, (bx + x) as i32 + dx, (by + y) as i32 + dy);
                            m += (a - b).abs();
                        }
                    }
                    m / (bw * bh) as f32
                };
                let (mut best, mut best_dx, mut best_dy) = (mad_at(0, 0), 0, 0);
                for dy in -r..=r {
                    for dx in -r..=r {
                        let m = mad_at(dx, dy);
                        if m < best {
                            best = m;
                            best_dx = dx;
                            best_dy = dy;
                        }
                    }
                }
                let weight = 1.0 / (1.0 + (best / tolerance) * (best / tolerance));
                wsum += weight;
                for y in 0..bh {
                    for x in 0..bw {
                        sum[y * bw + x] += weight * at(frame, (bx + x) as i32 + best_dx, (by + y) as i32 + best_dy);
                    }
                }
            }
            for y in 0..bh {
                for x in 0..bw {
                    out.a[(by + y) * w + bx + x] = sum[y * bw + x] / wsum;
                }
            }
            bx += block;
        }
        by += block;
    }
    Some(out)
}

/// 2x2 box reduction (clamped at odd edges): the result is `ceil(w / 2)` by `ceil(h / 2)`.
fn down(p: &Plane) -> Plane {
    let (w, h) = (p.w.div_ceil(2), p.h.div_ceil(2));
    let mut o = Plane::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let s = at(p, 2 * x, 2 * y) + at(p, 2 * x + 1, 2 * y) + at(p, 2 * x, 2 * y + 1) + at(p, 2 * x + 1, 2 * y + 1);
            o.a[y as usize * w + x as usize] = s * 0.25;
        }
    }
    o
}

/// Bilinear enlargement of `p` to `w` by `h` (sampled at pixel centres).
fn up(p: &Plane, w: usize, h: usize) -> Plane {
    let mut o = Plane::new(w, h);
    for y in 0..h {
        let sy = ((y as f32 + 0.5) * p.h as f32 / h as f32 - 0.5).max(0.0);
        let y0 = sy.floor() as i32;
        let fy = sy - y0 as f32;
        for x in 0..w {
            let sx = ((x as f32 + 0.5) * p.w as f32 / w as f32 - 0.5).max(0.0);
            let x0 = sx.floor() as i32;
            let fx = sx - x0 as f32;
            let top = at(p, x0, y0) * (1.0 - fx) + at(p, x0 + 1, y0) * fx;
            let bottom = at(p, x0, y0 + 1) * (1.0 - fx) + at(p, x0 + 1, y0 + 1) * fx;
            o.a[y * w + x] = top * (1.0 - fy) + bottom * fy;
        }
    }
    o
}

fn gauss_pyramid(p: &Plane, levels: usize) -> Vec<Plane> {
    let mut v = vec![p.clone()];
    while v.len() < levels {
        let next = down(&v[v.len() - 1]);
        v.push(next);
    }
    v
}

/// Laplacian levels (finest first); the last entry is the coarsest Gaussian level itself.
fn lap_pyramid(p: &Plane, levels: usize) -> Vec<Plane> {
    let g = gauss_pyramid(p, levels);
    let mut out = Vec::with_capacity(g.len());
    for i in 0..g.len() {
        if i + 1 < g.len() {
            let e = up(&g[i + 1], g[i].w, g[i].h);
            out.push(Plane { w: g[i].w, h: g[i].h, a: g[i].a.iter().zip(&e.a).map(|(a, b)| a - b).collect() });
        } else {
            out.push(g[i].clone());
        }
    }
    out
}

fn collapse(levels: &[Plane]) -> Plane {
    let mut cur = levels[levels.len() - 1].clone();
    for l in levels[..levels.len() - 1].iter().rev() {
        let e = up(&cur, l.w, l.h);
        cur = Plane { w: l.w, h: l.h, a: l.a.iter().zip(&e.a).map(|(a, b)| a + b).collect() };
    }
    cur
}

/// Exposure fusion of a bracket (values 0..=1): per-pixel weights from local contrast (the absolute 3x3 Laplacian) times
/// well-exposedness (a Gaussian around 0.5, sigma 0.2), normalised across frames and blended in a `levels`-level
/// Laplacian pyramid so seams do not show. `None` for an empty list, different sizes, or zero levels.
pub fn exposure_fusion(frames: &[Plane], levels: usize) -> Option<Plane> {
    if frames.is_empty() || !same_size(frames) || levels == 0 {
        return None;
    }
    let (w, h) = (frames[0].w, frames[0].h);
    let mut weights: Vec<Plane> = Vec::with_capacity(frames.len());
    for f in frames {
        let mut wp = Plane::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let c = at(f, x, y);
                let lap = (at(f, x - 1, y) + at(f, x + 1, y) + at(f, x, y - 1) + at(f, x, y + 1) - 4.0 * c).abs();
                let well = (-((c - 0.5) * (c - 0.5)) / (2.0 * 0.2 * 0.2)).exp();
                wp.a[y as usize * w + x as usize] = lap * well + 1e-12;
            }
        }
        weights.push(wp);
    }
    for i in 0..w * h {
        let total: f32 = weights.iter().map(|wp| wp.a[i]).sum();
        for wp in weights.iter_mut() {
            wp.a[i] /= total;
        }
    }
    let mut blended: Vec<Plane> = Vec::new();
    for (f, wp) in frames.iter().zip(&weights) {
        let lap = lap_pyramid(f, levels);
        let gw = gauss_pyramid(wp, levels);
        if blended.is_empty() {
            blended = lap.iter().map(|l| Plane::new(l.w, l.h)).collect();
        }
        for ((b, l), g) in blended.iter_mut().zip(&lap).zip(&gw) {
            for ((o, lv), gv) in b.a.iter_mut().zip(&l.a).zip(&g.a) {
                *o += lv * gv;
            }
        }
    }
    Some(collapse(&blended))
}

/// Local tone mapping of a positive luminance plane (Durand and Dorsey): the log luminance splits into a base layer
/// (guided filter with the log image as its own guide, window `diameter`, regularisation `epsilon`) and a detail
/// layer; the base is compressed by `compression` (0 to 1, smaller flattens more) about its maximum and the detail is
/// added back. The result peaks near 1. `None` when the guided filter refuses (even or zero diameter, empty plane).
pub fn local_tone_map(luma: &Plane, diameter: usize, epsilon: f32, compression: f32) -> Option<Plane> {
    let log: Vec<f32> = luma.a.iter().map(|v| v.max(1e-6).ln()).collect();
    let base = guided_filter(&log, &log, luma.w, luma.h, diameter, epsilon)?;
    let base_max = base.iter().copied().fold(f32::MIN, f32::max);
    let a = log
        .iter()
        .zip(&base)
        .map(|(l, b)| {
            let detail = l - b;
            (compression * (b - base_max) + detail).exp()
        })
        .collect();
    Some(Plane { w: luma.w, h: luma.h, a })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.a[y * w + x] = f(x, y);
            }
        }
        p
    }

    fn noisy_step(w: usize, h: usize) -> Plane {
        plane(w, h, |x, y| {
            let base = if x < w / 2 { 0.2 } else { 0.8 };
            base + if (x + y) % 2 == 0 { 0.02 } else { -0.02 }
        })
    }

    #[test]
    fn lens_gain_is_one_at_the_centre_and_corner_gain_at_the_corners() {
        let g = lens_shading_gain(5, 5, 2.0);
        assert!((g.a[2 * 5 + 2] - 1.0).abs() < 1e-6);
        assert!((g.a[0] - 2.0).abs() < 1e-5);
        assert!((g.a[24] - 2.0).abs() < 1e-5);
        assert_eq!(lens_shading_gain(1, 1, 3.0).a, vec![1.0]);
    }

    #[test]
    fn gain_undoes_a_vignette() {
        let gain = lens_shading_gain(9, 7, 1.8);
        let vignetted = plane(9, 7, |x, y| 0.4 / gain.a[y * 9 + x]);
        let fixed = apply_gain(&vignetted, &gain).unwrap();
        assert!(fixed.a.iter().all(|v| (v - 0.4).abs() < 1e-5));
        assert!(apply_gain(&vignetted, &lens_shading_gain(3, 3, 1.0)).is_none());
    }

    #[test]
    fn non_local_means_denoises_flat_areas_and_keeps_the_edge() {
        let p = noisy_step(24, 16);
        let f = non_local_means(&p, 1, 4, 0.1).unwrap();
        assert!((f.a[8 * 24 + 5] - 0.2).abs() < 0.01, "{}", f.a[8 * 24 + 5]);
        assert!((f.a[8 * 24 + 18] - 0.8).abs() < 0.01, "{}", f.a[8 * 24 + 18]);
        assert!(f.a[8 * 24 + 11] < 0.3 && f.a[8 * 24 + 12] > 0.7);
        assert!(non_local_means(&p, 1, 4, 0.0).is_none());
    }

    fn texture(x: i32, y: i32) -> f32 {
        (((x * 7 + y * 13).rem_euclid(17)) as f32) / 17.0
    }

    #[test]
    fn burst_merge_aligns_a_shifted_frame() {
        let (w, h) = (32, 16);
        let reference = plane(w, h, |x, y| texture(x as i32, y as i32));
        // The second frame is the scene moved right by 2 pixels.
        let shifted = plane(w, h, |x, y| texture(x as i32 - 2, y as i32));
        let merged = merge_burst(&[reference.clone(), shifted], 8, 3, 0.05).unwrap();
        for y in 2..h - 2 {
            for x in 4..w - 4 {
                assert!((merged.a[y * w + x] - reference.a[y * w + x]).abs() < 1e-5, "({x},{y})");
            }
        }
    }

    #[test]
    fn burst_merge_rejects_a_frame_that_does_not_match() {
        let (w, h) = (16, 16);
        let reference = plane(w, h, |x, y| texture(x as i32, y as i32) * 0.4);
        let wrong = Plane::filled(w, h, 1.0);
        let merged = merge_burst(&[reference.clone(), wrong], 8, 2, 0.02).unwrap();
        for i in 0..w * h {
            assert!((merged.a[i] - reference.a[i]).abs() < 0.02, "{i}");
        }
        assert!(merge_burst(&[], 8, 2, 0.02).is_none());
        assert!(merge_burst(&[Plane::new(4, 4), Plane::new(5, 4)], 4, 1, 0.1).is_none());
    }

    #[test]
    fn burst_merge_averages_independent_noise() {
        let (w, h) = (16, 16);
        let a = plane(w, h, |x, y| 0.5 + if (x + y) % 2 == 0 { 0.05 } else { -0.05 });
        let b = plane(w, h, |x, y| 0.5 + if (x + y) % 2 == 0 { -0.05 } else { 0.05 });
        let m = merge_burst(&[a, b], 16, 0, 0.5).unwrap();
        // Nearly cancelled (the weight of the second frame is a little below 1).
        assert!(m.a.iter().all(|v| (v - 0.5).abs() < 0.03));
    }

    #[test]
    fn fusion_takes_each_region_from_the_frame_that_exposes_it() {
        let (w, h) = (32, 32);
        let detail = |x: usize, y: usize| 0.5 + if (x + y) % 2 == 0 { 0.1 } else { -0.1 };
        let a = plane(w, h, |x, y| if x < 16 { detail(x, y) } else { 1.0 });
        let b = plane(w, h, |x, y| if x < 16 { 0.0 } else { detail(x, y) });
        let f = exposure_fusion(&[a, b], 3).unwrap();
        let mean = |x0: usize, x1: usize| {
            let mut s = 0.0;
            let mut n = 0.0;
            for y in 4..28 {
                for x in x0..x1 {
                    s += f.a[y * w + x];
                    n += 1.0;
                }
            }
            s / n
        };
        assert!((mean(0, 6) - 0.5).abs() < 0.1, "{}", mean(0, 6));
        assert!((mean(26, 32) - 0.5).abs() < 0.1, "{}", mean(26, 32));
        assert!(exposure_fusion(&[], 3).is_none());
        assert!(exposure_fusion(&[Plane::filled(4, 4, 0.5)], 0).is_none());
    }

    #[test]
    fn fusion_of_one_frame_is_that_frame() {
        let p = plane(16, 16, |x, y| texture(x as i32, y as i32));
        let f = exposure_fusion(&[p.clone()], 3).unwrap();
        for i in 0..256 {
            assert!((f.a[i] - p.a[i]).abs() < 1e-4, "{i}");
        }
    }

    #[test]
    fn tone_map_compresses_the_range_between_regions() {
        let (w, h) = (32, 16);
        let p = plane(w, h, |x, _| if x < 16 { 0.01 } else { 1.0 });
        let t = local_tone_map(&p, 5, 1e-4, 0.3).unwrap();
        assert!(t.a.iter().all(|v| v.is_finite() && *v > 0.0));
        let left = t.a[8 * w + 4];
        let right = t.a[8 * w + 27];
        let ratio = right / left;
        assert!(ratio > 1.0 && ratio < 10.0, "{ratio}");
        assert!(local_tone_map(&p, 4, 1e-4, 0.3).is_none());
    }
}
