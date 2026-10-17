//! Photo-adjustment kernels: the spatial half of a photo editor's adjustment stack. The per-pixel colour
//! adjustments (exposure, saturation, vibrance, temperature, highlights and shadows, vignette) already
//! live in `colorfx`, `color_curve`, `edge_tone_filters` and `photo_effects`; this module adds what needs
//! neighbours or whole-image statistics: box blur, unsharp mask, midtone-weighted clarity, an
//! edge-preserving bilateral denoise, percentile auto levels, and a dark-channel dehaze. Planes are
//! single-channel row-major `f32` in 0..=1; colour is `[f32; 3]`. The GPU twin of the unsharp pass is
//! `springboard/gpu/shaders/photo_adjust.comp`. Written from the standard descriptions of each method
//! (unsharp masking, bilateral filtering by Tomasi and Manduchi, the dark channel prior).

use crate::kernel_vision::{summed_area, window_sum};

/// A box blur of `radius` (window `2 * radius + 1`, clamped at the border). `None` on a size mismatch.
pub fn box_blur_plane(src: &[f32], w: usize, h: usize, radius: usize) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    let t = summed_area(src, w, h);
    let r = radius as isize;
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let x0 = (x - r).max(0);
            let x1 = (x + r + 1).min(w as isize);
            let y0 = (y - r).max(0);
            let y1 = (y + r + 1).min(h as isize);
            let n = ((x1 - x0) * (y1 - y0)) as f64;
            out[y as usize * w + x as usize] = (window_sum(&t, w, h, x0, y0, x1, y1) / n) as f32;
        }
    }
    Some(out)
}

/// Unsharp mask: adds `amount` times the detail (pixel minus its blur) back to the pixel, but only where
/// the detail exceeds `threshold`, so flat noise is not sharpened. Output is clamped to 0..=1.
pub fn unsharp_mask(src: &[f32], w: usize, h: usize, radius: usize, amount: f32, threshold: f32) -> Option<Vec<f32>> {
    let blur = box_blur_plane(src, w, h, radius)?;
    Some(
        src.iter()
            .zip(blur.iter())
            .map(|(s, b)| {
                let d = s - b;
                if d.abs() > threshold {
                    (s + amount * d).clamp(0.0, 1.0)
                } else {
                    *s
                }
            })
            .collect(),
    )
}

/// Clarity: local contrast over a wide radius, weighted so it acts on the midtones and leaves pure
/// black and white alone (`4 v (1 - v)` peaks at 0.5).
pub fn clarity(src: &[f32], w: usize, h: usize, radius: usize, amount: f32) -> Option<Vec<f32>> {
    let blur = box_blur_plane(src, w, h, radius)?;
    Some(
        src.iter()
            .zip(blur.iter())
            .map(|(s, b)| {
                let weight = 4.0 * s.clamp(0.0, 1.0) * (1.0 - s.clamp(0.0, 1.0));
                (s + amount * (s - b) * weight).clamp(0.0, 1.0)
            })
            .collect(),
    )
}

/// Bilateral denoise: each pixel becomes the mean of its window weighted by spatial closeness
/// (`sigma_s`, pixels) and by similarity of value (`sigma_r`, same unit as the plane), so edges survive.
pub fn bilateral(src: &[f32], w: usize, h: usize, radius: usize, sigma_s: f32, sigma_r: f32) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h || sigma_s <= 0.0 || sigma_r <= 0.0 {
        return None;
    }
    let r = radius as isize;
    let two_ss = 2.0 * sigma_s * sigma_s;
    let two_sr = 2.0 * sigma_r * sigma_r;
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let centre = src[y as usize * w + x as usize];
            let mut acc = 0.0f32;
            let mut total = 0.0f32;
            for dy in -r..=r {
                for dx in -r..=r {
                    let sx = x + dx;
                    let sy = y + dy;
                    if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                        continue;
                    }
                    let v = src[sy as usize * w + sx as usize];
                    let spatial = (-((dx * dx + dy * dy) as f32) / two_ss).exp();
                    let range = (-((v - centre) * (v - centre)) / two_sr).exp();
                    acc += v * spatial * range;
                    total += spatial * range;
                }
            }
            out[y as usize * w + x as usize] = acc / total;
        }
    }
    Some(out)
}

/// Auto levels: stretches the `low_percent`..`high_percent` range of the plane to 0..=1 (clipping the
/// tails). A flat plane, or an empty range, is returned unchanged.
pub fn auto_levels(src: &[f32], low_percent: f32, high_percent: f32) -> Vec<f32> {
    if src.is_empty() || !(low_percent >= 0.0 && high_percent <= 100.0 && low_percent < high_percent) {
        return src.to_vec();
    }
    let mut sorted: Vec<f32> = src.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return src.to_vec();
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let last = (sorted.len() - 1) as f32;
    let lo = sorted[(low_percent / 100.0 * last).floor() as usize];
    let hi = sorted[(high_percent / 100.0 * last).ceil() as usize];
    if hi - lo <= 1e-9 {
        return src.to_vec();
    }
    src.iter().map(|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0)).collect()
}

/// Dark-channel-prior dehaze. The dark channel is the minimum over colour and a `(2 * radius + 1)`
/// window; the airlight is the colour of the pixel with the largest dark channel; the transmission is
/// `1 - omega * dark(I / A)`, floored at 0.1; the scene is `(I - A) / t + A`, clamped to 0..=1.
pub fn dehaze(rgb: &[[f32; 3]], w: usize, h: usize, radius: usize, omega: f32) -> Option<Vec<[f32; 3]>> {
    if w == 0 || h == 0 || rgb.len() != w * h {
        return None;
    }
    let r = radius as isize;
    let dark_of = |buf: &[[f32; 3]]| -> Vec<f32> {
        let mins: Vec<f32> = buf.iter().map(|p| p[0].min(p[1]).min(p[2])).collect();
        let mut out = vec![0.0f32; w * h];
        for y in 0..h as isize {
            for x in 0..w as isize {
                let mut m = f32::MAX;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let sx = x + dx;
                        let sy = y + dy;
                        if sx >= 0 && sy >= 0 && sx < w as isize && sy < h as isize {
                            m = m.min(mins[sy as usize * w + sx as usize]);
                        }
                    }
                }
                out[y as usize * w + x as usize] = m;
            }
        }
        out
    };
    let dark = dark_of(rgb);
    let mut best = 0usize;
    for i in 1..dark.len() {
        if dark[i] > dark[best] {
            best = i;
        }
    }
    let air = [rgb[best][0].max(1e-3), rgb[best][1].max(1e-3), rgb[best][2].max(1e-3)];
    let normalised: Vec<[f32; 3]> = rgb.iter().map(|p| [p[0] / air[0], p[1] / air[1], p[2] / air[2]]).collect();
    let dark_n = dark_of(&normalised[..]);
    Some(
        (0..rgb.len())
            .map(|i| {
                let t = (1.0 - omega * dark_n[i]).max(0.1);
                let mut px = [0.0f32; 3];
                for c in 0..3 {
                    px[c] = ((rgb[i][c] - air[c]) / t + air[c]).clamp(0.0, 1.0);
                }
                px
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_blur_averages_and_keeps_flat() {
        assert_eq!(box_blur_plane(&[0.5; 9], 3, 3, 1).unwrap(), vec![0.5; 9]);
        let out = box_blur_plane(&[0.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, 0.0, 0.0], 3, 3, 1).unwrap();
        assert!((out[4] - 1.0).abs() < 1e-6);
        assert!(box_blur_plane(&[0.0; 8], 3, 3, 1).is_none());
    }

    #[test]
    fn unsharp_overshoots_an_edge_and_leaves_flat_alone() {
        // 6 x 2 plane, step between columns 2 and 3.
        let plane: Vec<f32> = (0..12).map(|i| if i % 6 >= 3 { 0.6 } else { 0.2 }).collect();
        let out = unsharp_mask(&plane, 6, 2, 1, 1.0, 0.0).unwrap();
        assert!(out[3] > 0.6, "bright side of the edge overshoots");
        assert!(out[2] < 0.2, "dark side undershoots");
        assert!((out[0] - 0.2).abs() < 1e-6, "flat far side unchanged");
        let gated = unsharp_mask(&plane, 6, 2, 1, 1.0, 0.5).unwrap();
        assert_eq!(gated, plane);
    }

    #[test]
    fn clarity_ignores_pure_black_and_white() {
        let plane = [0.0, 1.0, 0.0, 1.0];
        let out = clarity(&plane, 2, 2, 1, 2.0).unwrap();
        assert_eq!(out, plane.to_vec());
        let mid = [0.4, 0.6, 0.4, 0.6];
        let out = clarity(&mid, 2, 2, 1, 1.0).unwrap();
        assert!(out[1] > 0.6 && out[0] < 0.4);
    }

    #[test]
    fn bilateral_keeps_edges_for_small_sigma_r_and_blurs_for_large() {
        let plane: Vec<f32> = (0..8).map(|x| if x >= 4 { 1.0 } else { 0.0 }).collect();
        let sharp = bilateral(&plane, 8, 1, 2, 1.5, 0.1).unwrap();
        assert!(sharp[3] < 1e-6 && sharp[4] > 1.0 - 1e-6);
        let soft = bilateral(&plane, 8, 1, 2, 1.5, 100.0).unwrap();
        assert!(soft[3] > 0.1 && soft[4] < 0.9);
        assert!(bilateral(&plane, 8, 1, 2, 0.0, 0.1).is_none());
    }

    #[test]
    fn auto_levels_stretches_a_ramp_and_keeps_flat() {
        let out = auto_levels(&[0.2, 0.5, 0.8], 0.0, 100.0);
        assert!((out[0] - 0.0).abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6 && (out[2] - 1.0).abs() < 1e-6);
        assert_eq!(auto_levels(&[0.4; 4], 0.0, 100.0), vec![0.4; 4]);
        assert_eq!(auto_levels(&[0.1, 0.9], 60.0, 40.0), vec![0.1, 0.9]);
        assert!(auto_levels(&[], 0.0, 100.0).is_empty());
    }

    #[test]
    fn dehaze_darkens_a_hazy_pixel_and_stays_in_range() {
        let hazy = [[0.5f32, 0.5, 0.5], [0.7, 0.6, 0.55]];
        let out = dehaze(&hazy, 2, 1, 0, 0.95).unwrap();
        assert!(out[0][0] < 0.5);
        assert!(out.iter().all(|p| p.iter().all(|c| *c >= 0.0 && *c <= 1.0)));
        assert!(dehaze(&hazy, 3, 1, 0, 0.95).is_none());
    }
}
