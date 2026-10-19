//! Spatial super resolution without a trained model: the edge-directed 2x upscale and the
//! contrast-adaptive sharpen that a real-time video scaler runs. Pass one fills the diagonal samples
//! along whichever diagonal is smoother; pass two fills the horizontal and vertical samples along the
//! smoother of the two axes; the sharpen then lifts local contrast with a weight that shrinks where the
//! neighbourhood is already near black or white, and clamps to the neighbourhood range so it cannot ring.
//! The plain bicubic and Lanczos scalers are in `upscale` and `lanczos`; temporal upscaling is in
//! `temporal_upscale`. Learned super resolution needs trained weights, which BifrOSt does not ship, so this
//! is the classical, deterministic path. Planes are single-channel `f32` in 0..=1, row-major.

fn at(src: &[f32], w: usize, h: usize, x: isize, y: isize) -> f32 {
    let xx = x.clamp(0, w as isize - 1) as usize;
    let yy = y.clamp(0, h as isize - 1) as usize;
    src[yy * w + xx]
}

/// Edge-directed 2x upscale. The output is `2 * w` by `2 * h`; sample `(2x, 2y)` is exactly input
/// `(x, y)`. `None` on a size mismatch.
pub fn edge_directed_2x(src: &[f32], w: usize, h: usize) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    let (ow, oh) = (2 * w, 2 * h);
    let mut out = vec![0.0f32; ow * oh];
    for y in 0..h {
        for x in 0..w {
            out[2 * y * ow + 2 * x] = src[y * w + x];
        }
    }
    // Pass one: the diagonal sample between four originals.
    for y in 0..h as isize {
        for x in 0..w as isize {
            let a = at(src, w, h, x, y);
            let b = at(src, w, h, x + 1, y);
            let c = at(src, w, h, x, y + 1);
            let d = at(src, w, h, x + 1, y + 1);
            let main_diag = (a - d).abs();
            let anti_diag = (b - c).abs();
            let v = if main_diag < anti_diag {
                (a + d) * 0.5
            } else if anti_diag < main_diag {
                (b + c) * 0.5
            } else {
                (a + b + c + d) * 0.25
            };
            out[(2 * y as usize + 1) * ow + 2 * x as usize + 1] = v;
        }
    }
    // Pass two: the samples to the right of and below each original, from the four filled neighbours.
    // Neighbours outside the plane are missing, not clamped: clamping would read the pixel itself, which is
    // still unwritten here. A missing side takes the other side's value and reports no gradient on that axis.
    let get = |buf: &[f32], x: isize, y: isize| -> Option<f32> {
        if x < 0 || y < 0 || x >= ow as isize || y >= oh as isize {
            None
        } else {
            Some(buf[y as usize * ow + x as usize])
        }
    };
    let pair = |a: Option<f32>, b: Option<f32>| -> (f32, f32) {
        match (a, b) {
            (Some(a), Some(b)) => ((a + b) * 0.5, (a - b).abs()),
            (Some(a), None) | (None, Some(a)) => (a, 0.0),
            (None, None) => (0.0, 0.0),
        }
    };
    for oy in 0..oh as isize {
        for ox in 0..ow as isize {
            if (ox + oy) % 2 == 0 {
                continue; // an original (both even) or a diagonal sample (both odd)
            }
            let (h, dh) = pair(get(&out, ox - 1, oy), get(&out, ox + 1, oy));
            let (vert, dv) = pair(get(&out, ox, oy - 1), get(&out, ox, oy + 1));
            let v = if dh < dv {
                h
            } else if dv < dh {
                vert
            } else {
                (h + vert) * 0.5
            };
            out[oy as usize * ow + ox as usize] = v;
        }
    }
    Some(out)
}

/// Contrast-adaptive sharpen. `strength` in 0..=1 moves the neighbour weight from 1/8 to 1/5. The
/// per-pixel amplitude `sqrt(min(min, 1 - max) / max)` over the pixel and its four neighbours keeps the
/// lift away from pixels already near black or white; the result is clamped to the neighbourhood range.
pub fn contrast_adaptive_sharpen(src: &[f32], w: usize, h: usize, strength: f32) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    let s = strength.clamp(0.0, 1.0);
    let peak = 0.125 + (0.2 - 0.125) * s;
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let c = at(src, w, h, x, y);
            let n = at(src, w, h, x, y - 1);
            let so = at(src, w, h, x, y + 1);
            let e = at(src, w, h, x + 1, y);
            let we = at(src, w, h, x - 1, y);
            let mn = c.min(n).min(so).min(e).min(we);
            let mx = c.max(n).max(so).max(e).max(we);
            let amp = if mx > 1e-6 { (mn.min(1.0 - mx).max(0.0) / mx).sqrt() } else { 0.0 };
            let wgt = -amp * peak;
            let v = (c + wgt * (n + so + e + we)) / (1.0 + 4.0 * wgt);
            out[y as usize * w + x as usize] = v.clamp(mn, mx);
        }
    }
    Some(out)
}

/// The full pass: edge-directed 2x, then the contrast-adaptive sharpen at the new size. Returns the plane
/// and its dimensions.
pub fn super_resolve_2x(src: &[f32], w: usize, h: usize, sharpen: f32) -> Option<(Vec<f32>, usize, usize)> {
    let up = edge_directed_2x(src, w, h)?;
    let sharp = contrast_adaptive_sharpen(&up, 2 * w, 2 * h, sharpen)?;
    Some((sharp, 2 * w, 2 * h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_stays_flat_and_originals_are_kept() {
        let up = edge_directed_2x(&[0.3; 9], 3, 3).unwrap();
        assert_eq!(up.len(), 36);
        assert!(up.iter().all(|v| (v - 0.3).abs() < 1e-6));
        let src = [0.1f32, 0.9, 0.4, 0.6];
        let up = edge_directed_2x(&src, 2, 2).unwrap();
        assert_eq!(up[0], 0.1);
        assert_eq!(up[2], 0.9);
        assert_eq!(up[2 * 4], 0.4);
        assert_eq!(up[2 * 4 + 2], 0.6);
        assert!(edge_directed_2x(&src, 3, 2).is_none());
    }

    #[test]
    fn a_diagonal_edge_is_not_smeared() {
        // 1 where x >= y: a staircase along the main diagonal.
        let mut p = vec![0.0f32; 16];
        for y in 0..4 {
            for x in 0..4 {
                if x >= y {
                    p[y * 4 + x] = 1.0;
                }
            }
        }
        let up = edge_directed_2x(&p, 4, 4).unwrap();
        // Between originals (1,1), (2,1), (1,2), (2,2) the main diagonal is flat (1, 1) while the other
        // diagonal differs (1, 0): the new sample follows the flat one instead of averaging to 0.75.
        assert_eq!(up[3 * 8 + 3], 1.0);
    }

    #[test]
    fn sharpen_keeps_flat_and_deepens_a_soft_shoulder_without_ringing() {
        let flat = contrast_adaptive_sharpen(&[0.5; 6], 6, 1, 1.0).unwrap();
        assert!(flat.iter().all(|v| (v - 0.5).abs() < 1e-6));
        let p = [0.2f32, 0.2, 0.3, 0.8, 0.8, 0.8];
        let out = contrast_adaptive_sharpen(&p, 6, 1, 1.0).unwrap();
        assert!(out[2] < 0.3 && out[2] >= 0.2);
        assert!(out.iter().all(|v| *v >= 0.2 && *v <= 0.8), "no overshoot beyond the input range");
        assert!(contrast_adaptive_sharpen(&p, 5, 1, 1.0).is_none());
    }

    #[test]
    fn super_resolve_doubles_the_size() {
        let (out, w, h) = super_resolve_2x(&[0.2, 0.8, 0.8, 0.2], 2, 2, 0.5).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(out.len(), 16);
        assert!(out.iter().all(|v| *v >= 0.2 - 1e-6 && *v <= 0.8 + 1e-6));
    }
}
