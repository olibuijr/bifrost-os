//! Portrait-mode kernels: the pixel jobs between a depth map and a finished portrait. A trimap from depth
//! (sure subject, sure background, unknown band), a matte refined onto the image's own edges with the
//! guided filter, a gather-style bokeh blur whose disc size comes from a per-pixel circle-of-confusion
//! radius (sharp subject pixels never smear onto the background, bright highlights are boosted into
//! discs), and the final subject-over-blur composite. Depth blur with a single focus plane is in
//! `depth_blur` and `masked_blur`; this module is the matte and bokeh half. No trained model is involved:
//! the depth and the initial matte come from the caller.
//!
//! Colour is linear RGB `[f32; 3]`; planes are row-major `f32`.

use crate::guided::guided_filter;

/// A trimap from a depth plane (any monotonic depth unit): 255 for pixels within `subject_band` of
/// `focus`, 0 for pixels farther than `background_band`, 128 (unknown) in between. `None` when the bands
/// are negative or out of order.
pub fn trimap_from_depth(depth: &[f32], focus: f32, subject_band: f32, background_band: f32) -> Option<Vec<u8>> {
    if !(subject_band >= 0.0 && background_band >= subject_band) {
        return None;
    }
    Some(
        depth
            .iter()
            .map(|d| {
                let off = (d - focus).abs();
                if off <= subject_band {
                    255
                } else if off > background_band {
                    0
                } else {
                    128
                }
            })
            .collect(),
    )
}

/// The initial matte of a trimap: 1 for subject, 0 for background, 0.5 for unknown.
pub fn trimap_matte(trimap: &[u8]) -> Vec<f32> {
    trimap
        .iter()
        .map(|t| match *t {
            255 => 1.0,
            0 => 0.0,
            _ => 0.5,
        })
        .collect()
}

/// Refines a matte so its transitions follow the edges of a guide plane (the image luminance): a guided
/// filter of `matte` by `guide`, clamped to 0..=1. `diameter` is the window and `epsilon` the smoothness
/// (smaller follows the guide more strictly).
pub fn refine_matte(matte: &[f32], guide: &[f32], w: usize, h: usize, diameter: usize, epsilon: f32) -> Option<Vec<f32>> {
    let out = guided_filter(guide, matte, w, h, diameter, epsilon)?;
    Some(out.into_iter().map(|v| v.clamp(0.0, 1.0)).collect())
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Gather bokeh blur. `radii[i]` is the circle-of-confusion radius of pixel `i` in pixels. A pixel
/// averages every pixel inside its own disc, except that a source pixel only contributes when its own
/// circle reaches the centre (`its radius + 0.5 >= distance`), so a sharp subject does not bleed into a
/// blurred background. Each contribution is weighted `1 + highlight_boost * luma^2`, which turns bright
/// points into visible discs. `None` on a size mismatch.
pub fn bokeh_blur(
    rgb: &[[f32; 3]],
    w: usize,
    h: usize,
    radii: &[f32],
    highlight_boost: f32,
) -> Option<Vec<[f32; 3]>> {
    if w == 0 || h == 0 || rgb.len() != w * h || radii.len() != w * h {
        return None;
    }
    let boost = highlight_boost.max(0.0);
    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let r = radii[i].max(0.0);
            if r < 0.5 {
                out[i] = rgb[i];
                continue;
            }
            let reach = r.ceil() as isize;
            let mut acc = [0.0f32; 3];
            let mut total = 0.0f32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let sx = x as isize + dx;
                    let sy = y as isize + dy;
                    if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                        continue;
                    }
                    let dist = ((dx * dx + dy * dy) as f32).sqrt();
                    if dist > r {
                        continue;
                    }
                    let j = sy as usize * w + sx as usize;
                    if dist > 0.0 && radii[j].max(0.0) + 0.5 < dist {
                        continue;
                    }
                    let l = luma(rgb[j]);
                    let weight = 1.0 + boost * l * l;
                    for c in 0..3 {
                        acc[c] += rgb[j][c] * weight;
                    }
                    total += weight;
                }
            }
            // The centre pixel always contributes, so total is positive.
            out[i] = [acc[0] / total, acc[1] / total, acc[2] / total];
        }
    }
    Some(out)
}

/// Composites the sharp subject over the blurred picture with the matte (`matte * sharp + (1 - matte) *
/// blurred`). `None` when the three buffers differ in length.
pub fn composite_portrait(sharp: &[[f32; 3]], blurred: &[[f32; 3]], matte: &[f32]) -> Option<Vec<[f32; 3]>> {
    if sharp.len() != blurred.len() || sharp.len() != matte.len() {
        return None;
    }
    Some(
        (0..sharp.len())
            .map(|i| {
                let m = matte[i].clamp(0.0, 1.0);
                [
                    sharp[i][0] * m + blurred[i][0] * (1.0 - m),
                    sharp[i][1] * m + blurred[i][1] * (1.0 - m),
                    sharp[i][2] * m + blurred[i][2] * (1.0 - m),
                ]
            })
            .collect(),
    )
}

/// The circle-of-confusion radius for a depth plane: zero inside `subject_band` of `focus`, then growing
/// linearly with the distance from the focus plane at `per_unit` pixels per depth unit up to `max_radius`.
pub fn coc_radius(depth: &[f32], focus: f32, subject_band: f32, per_unit: f32, max_radius: f32) -> Vec<f32> {
    depth
        .iter()
        .map(|d| {
            let off = (d - focus).abs();
            if off <= subject_band {
                0.0
            } else {
                ((off - subject_band) * per_unit).clamp(0.0, max_radius.max(0.0))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimap_bands() {
        let t = trimap_from_depth(&[1.0, 1.2, 2.0, 5.0], 1.0, 0.3, 2.0).unwrap();
        assert_eq!(t, vec![255, 255, 128, 0]);
        assert!(trimap_from_depth(&[1.0], 1.0, 0.5, 0.2).is_none());
        assert_eq!(trimap_matte(&t), vec![1.0, 1.0, 0.5, 0.0]);
    }

    #[test]
    fn refined_matte_stays_in_range_and_keeps_the_sides() {
        // 8 x 4 image with an edge at x = 4; the coarse matte put the edge one pixel early.
        let (w, h) = (8usize, 4usize);
        let mut guide = vec![0.1f32; w * h];
        let mut matte = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                if x >= 4 {
                    guide[y * w + x] = 0.9;
                }
                if x >= 3 {
                    matte[y * w + x] = 1.0;
                }
            }
        }
        let r = refine_matte(&matte, &guide, w, h, 3, 1e-4).unwrap();
        assert!(r.iter().all(|v| *v >= 0.0 && *v <= 1.0));
        assert!(r[1 * w] < 0.5, "background side stays background");
        assert!(r[1 * w + 7] > 0.5, "subject side stays subject");
        assert!(refine_matte(&matte, &guide, w, h + 1, 3, 1e-4).is_none());
    }

    #[test]
    fn zero_radius_is_the_identity_and_uniform_stays_uniform() {
        let img: Vec<[f32; 3]> = (0..25).map(|i| [i as f32 / 25.0, 0.5, 0.25]).collect();
        let out = bokeh_blur(&img, 5, 5, &[0.0; 25], 2.0).unwrap();
        assert_eq!(out, img);
        let flat = vec![[0.4f32, 0.3, 0.2]; 25];
        let out = bokeh_blur(&flat, 5, 5, &[2.0; 25], 3.0).unwrap();
        for p in out {
            assert!((p[0] - 0.4).abs() < 1e-5 && (p[2] - 0.2).abs() < 1e-5);
        }
        assert!(bokeh_blur(&flat, 5, 5, &[2.0; 24], 3.0).is_none());
    }

    #[test]
    fn a_bright_point_spreads_into_its_disc() {
        let mut img = vec![[0.0f32; 3]; 81];
        img[4 * 9 + 4] = [1.0, 1.0, 1.0];
        let out = bokeh_blur(&img, 9, 9, &[3.0; 81], 0.0).unwrap();
        assert!(out[4 * 9 + 5][0] > 0.0);
        assert!(out[4 * 9 + 4][0] < 1.0);
        assert_eq!(out[0][0], 0.0, "far pixels stay dark");
    }

    #[test]
    fn a_sharp_subject_does_not_bleed_onto_the_blurred_background() {
        let mut img = vec![[0.0f32; 3]; 81];
        img[4 * 9 + 4] = [1.0, 1.0, 1.0];
        let mut radii = vec![3.0f32; 81];
        radii[4 * 9 + 4] = 0.0;
        let out = bokeh_blur(&img, 9, 9, &radii, 0.0).unwrap();
        assert_eq!(out[4 * 9 + 5][0], 0.0);
        assert_eq!(out[4 * 9 + 4], [1.0, 1.0, 1.0]);
    }

    #[test]
    fn highlight_boost_makes_bright_points_dominate() {
        let mut img = vec![[0.2f32; 3]; 81];
        img[4 * 9 + 4] = [1.0, 1.0, 1.0];
        let plain = bokeh_blur(&img, 9, 9, &[3.0; 81], 0.0).unwrap();
        let boosted = bokeh_blur(&img, 9, 9, &[3.0; 81], 8.0).unwrap();
        assert!(boosted[4 * 9 + 5][0] > plain[4 * 9 + 5][0]);
    }

    #[test]
    fn composite_blends_by_the_matte() {
        let sharp = [[1.0f32; 3], [1.0; 3], [1.0; 3]];
        let blur = [[0.0f32; 3], [0.0; 3], [0.0; 3]];
        let out = composite_portrait(&sharp, &blur, &[1.0, 0.25, 0.0]).unwrap();
        assert_eq!(out[0], [1.0; 3]);
        assert!((out[1][0] - 0.25).abs() < 1e-6);
        assert_eq!(out[2], [0.0; 3]);
        assert!(composite_portrait(&sharp, &blur, &[1.0]).is_none());
    }

    #[test]
    fn coc_is_zero_in_focus_and_capped_far_away() {
        let r = coc_radius(&[1.0, 1.1, 2.0, 100.0], 1.0, 0.2, 10.0, 12.0);
        assert_eq!(r[0], 0.0);
        assert_eq!(r[1], 0.0);
        assert!((r[2] - 8.0).abs() < 1e-5);
        assert_eq!(r[3], 12.0);
    }
}
