//! Video-processing kernels: the per-frame jobs of a video pipeline that sit before an encoder and
//! between a decoder and the display. Encoder-side analysis (sum of absolute differences, the 4x4
//! Hadamard SATD cost, a 2x luma downscale for look-ahead, a histogram scene-cut score), line doubling
//! (edge-directed deinterlace of one field), a motion-adaptive temporal denoise, backward warping of a
//! frame by a flow field, and a Gaussian pyramid step. Motion search itself is `block_motion`; frame
//! interpolation is `frame_interp`; virtual shutter blending and median deghosting are in
//! `temporal_upscale`. Luma planes are `u8`; float planes are row-major `f32`. Written from the textbook
//! definitions of each operation.

/// Sum of absolute differences between the `size` x `size` block at `(x, y)` of `cur` and the block at
/// `(rx, ry)` of `reference`, both planes `stride` wide. `None` when a block leaves its plane.
pub fn sad(cur: &[u8], x: usize, y: usize, reference: &[u8], rx: usize, ry: usize, stride: usize, size: usize) -> Option<u32> {
    if stride == 0 || size == 0 {
        return None;
    }
    let fits = |p: &[u8], px: usize, py: usize| px + size <= stride && (py + size) * stride <= p.len();
    if !fits(cur, x, y) || !fits(reference, rx, ry) {
        return None;
    }
    let mut total = 0u32;
    for j in 0..size {
        for i in 0..size {
            let a = cur[(y + j) * stride + x + i] as i32;
            let b = reference[(ry + j) * stride + rx + i] as i32;
            total += (a - b).unsigned_abs();
        }
    }
    Some(total)
}

fn hadamard4(v: [i32; 4]) -> [i32; 4] {
    let s0 = v[0] + v[1];
    let s1 = v[0] - v[1];
    let s2 = v[2] + v[3];
    let s3 = v[2] - v[3];
    [s0 + s2, s1 + s3, s0 - s2, s1 - s3]
}

/// The 4x4 SATD cost: the absolute sum of the Hadamard transform of the difference block, halved. A
/// better rate predictor than SAD because it sees how compressible the residual is. `None` when a block
/// leaves its plane.
pub fn satd4x4(cur: &[u8], x: usize, y: usize, reference: &[u8], rx: usize, ry: usize, stride: usize) -> Option<u32> {
    let fits = |p: &[u8], px: usize, py: usize| stride > 0 && px + 4 <= stride && (py + 4) * stride <= p.len();
    if !fits(cur, x, y) || !fits(reference, rx, ry) {
        return None;
    }
    let mut d = [[0i32; 4]; 4];
    for j in 0..4 {
        for i in 0..4 {
            d[j][i] = cur[(y + j) * stride + x + i] as i32 - reference[(ry + j) * stride + rx + i] as i32;
        }
    }
    for row in d.iter_mut() {
        *row = hadamard4(*row);
    }
    let mut sum = 0i32;
    for i in 0..4 {
        let col = hadamard4([d[0][i], d[1][i], d[2][i], d[3][i]]);
        for v in col {
            sum += v.abs();
        }
    }
    Some((sum / 2) as u32)
}

/// Halves a luma plane in both directions by averaging 2x2 blocks (rounded); an odd last row or column is
/// dropped. Returns the new plane with its width and height, or `None` for a plane under 2x2 or a size
/// mismatch.
pub fn downscale_2x(luma: &[u8], w: usize, h: usize) -> Option<(Vec<u8>, usize, usize)> {
    if w < 2 || h < 2 || luma.len() != w * h {
        return None;
    }
    let (nw, nh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            let s = luma[2 * y * w + 2 * x] as u32
                + luma[2 * y * w + 2 * x + 1] as u32
                + luma[(2 * y + 1) * w + 2 * x] as u32
                + luma[(2 * y + 1) * w + 2 * x + 1] as u32;
            out.push(((s + 2) / 4) as u8);
        }
    }
    Some((out, nw, nh))
}

/// Scene-cut score between two luma planes: half the L1 distance of their normalised 256-bin
/// histograms, 0 for identical distributions up to 1 for disjoint ones. `None` for empty or unequal planes.
pub fn scene_cut_score(prev: &[u8], cur: &[u8]) -> Option<f32> {
    if prev.is_empty() || prev.len() != cur.len() {
        return None;
    }
    let mut a = [0u32; 256];
    let mut b = [0u32; 256];
    for v in prev {
        a[*v as usize] += 1;
    }
    for v in cur {
        b[*v as usize] += 1;
    }
    let n = prev.len() as f32;
    let mut dist = 0.0f32;
    for i in 0..256 {
        dist += (a[i] as f32 / n - b[i] as f32 / n).abs();
    }
    Some(0.5 * dist)
}

/// Edge-directed line interpolation (ELA) of one field: even rows are kept, each odd row is rebuilt from the
/// rows above and below along the direction (vertical, or one of the two diagonals) with the smallest
/// difference. The last row of an even-height plane copies the row above. `None` on a size mismatch.
pub fn deinterlace_ela(luma: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || luma.len() != w * h {
        return None;
    }
    let mut out = luma.to_vec();
    for y in (1..h).step_by(2) {
        if y + 1 >= h {
            for x in 0..w {
                out[y * w + x] = luma[(y - 1) * w + x];
            }
            continue;
        }
        let above = &luma[(y - 1) * w..y * w];
        let below = &luma[(y + 1) * w..(y + 2) * w];
        for x in 0..w {
            let mut pair = (above[x] as i32, below[x] as i32);
            if x > 0 && x + 1 < w {
                let mut best = (pair.0 - pair.1).abs();
                let d1 = (above[x - 1] as i32 - below[x + 1] as i32).abs();
                if d1 < best {
                    best = d1;
                    pair = (above[x - 1] as i32, below[x + 1] as i32);
                }
                let d2 = (above[x + 1] as i32 - below[x - 1] as i32).abs();
                if d2 < best {
                    pair = (above[x + 1] as i32, below[x - 1] as i32);
                }
            }
            out[y * w + x] = ((pair.0 + pair.1 + 1) / 2) as u8;
        }
    }
    Some(out)
}

/// Motion-adaptive temporal denoise: each pixel is blended towards the previous frame by `strength` (0..=1)
/// scaled down linearly to zero as the frame difference grows to `threshold`, so still areas are
/// averaged and moving areas pass through. `None` on a length mismatch or a non-positive threshold.
pub fn temporal_denoise(prev: &[f32], cur: &[f32], threshold: f32, strength: f32) -> Option<Vec<f32>> {
    if prev.len() != cur.len() || !(threshold > 0.0) {
        return None;
    }
    let s = strength.clamp(0.0, 1.0);
    Some(
        prev.iter()
            .zip(cur.iter())
            .map(|(p, c)| {
                let wgt = s * (1.0 - (c - p).abs() / threshold).max(0.0);
                c * (1.0 - wgt) + p * wgt
            })
            .collect(),
    )
}

fn sample_clamped(src: &[f32], w: usize, h: usize, fx: f32, fy: f32) -> f32 {
    let x = fx.clamp(0.0, (w - 1) as f32);
    let y = fy.clamp(0.0, (h - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let top = src[y0 * w + x0] * (1.0 - tx) + src[y0 * w + x1] * tx;
    let bottom = src[y1 * w + x0] * (1.0 - tx) + src[y1 * w + x1] * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Backward warp: output pixel `(x, y)` samples `src` bilinearly at `(x + fx, y + fy)` where `(fx, fy)` is
/// the flow vector stored for that pixel. Samples outside the frame clamp to the border. `None` on a
/// size mismatch.
pub fn flow_warp(src: &[f32], w: usize, h: usize, flow: &[[f32; 2]]) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || src.len() != w * h || flow.len() != w * h {
        return None;
    }
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let f = flow[y * w + x];
            out[y * w + x] = sample_clamped(src, w, h, x as f32 + f[0], y as f32 + f[1]);
        }
    }
    Some(out)
}

/// One Gaussian pyramid step: blur with the binomial kernel `[1 4 6 4 1] / 16` (border clamped) and keep
/// every second pixel. Returns the smaller plane and its size `((w + 1) / 2, (h + 1) / 2)`.
pub fn pyramid_down(src: &[f32], w: usize, h: usize) -> Option<(Vec<f32>, usize, usize)> {
    if w == 0 || h == 0 || src.len() != w * h {
        return None;
    }
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (k, wt) in K.iter().enumerate() {
                let sx = (x as isize + k as isize - 2).clamp(0, w as isize - 1) as usize;
                s += src[y * w + sx] * wt;
            }
            tmp[y * w + x] = s;
        }
    }
    let (nw, nh) = ((w + 1) / 2, (h + 1) / 2);
    let mut out = vec![0.0f32; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let mut s = 0.0;
            for (k, wt) in K.iter().enumerate() {
                let sy = (2 * y as isize + k as isize - 2).clamp(0, h as isize - 1) as usize;
                s += tmp[sy * w + 2 * x] * wt;
            }
            out[y * nw + x] = s;
        }
    }
    Some((out, nw, nh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sad_is_zero_for_equal_blocks_and_counts_differences() {
        let a = [0u8; 16];
        let mut b = [0u8; 16];
        assert_eq!(sad(&a, 0, 0, &b, 0, 0, 4, 4), Some(0));
        b[5] = 7;
        b[10] = 3;
        assert_eq!(sad(&a, 0, 0, &b, 0, 0, 4, 4), Some(10));
        assert_eq!(sad(&a, 0, 0, &b, 1, 1, 4, 4), None);
        assert_eq!(sad(&a, 0, 0, &b, 0, 0, 4, 0), None);
    }

    #[test]
    fn satd_sees_a_dc_offset_as_one_coefficient() {
        let a = [10u8; 16];
        let b = [11u8; 16];
        // Difference is constant -1: only the DC Hadamard coefficient (-16) is non-zero, halved to 8.
        assert_eq!(satd4x4(&a, 0, 0, &b, 0, 0, 4), Some(8));
        assert_eq!(satd4x4(&a, 0, 0, &a, 0, 0, 4), Some(0));
        assert_eq!(satd4x4(&a, 1, 0, &b, 0, 0, 4), None);
    }

    #[test]
    fn downscale_averages_2x2_and_drops_odd_edges() {
        let luma = [10u8, 20, 30, 40, 50, 60, 70, 80, 90];
        // 3 x 3 plane: only the top-left 2x2 survives.
        let (out, w, h) = downscale_2x(&luma, 3, 3).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![((10 + 20 + 40 + 50 + 2) / 4) as u8]);
        assert!(downscale_2x(&[1u8], 1, 1).is_none());
    }

    #[test]
    fn scene_cut_is_zero_for_the_same_picture_and_one_for_disjoint_ones() {
        let dark = vec![10u8; 64];
        let bright = vec![240u8; 64];
        assert_eq!(scene_cut_score(&dark, &dark), Some(0.0));
        assert!((scene_cut_score(&dark, &bright).unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(scene_cut_score(&dark, &bright[..8]), None);
    }

    #[test]
    fn ela_keeps_even_rows_and_follows_a_diagonal_edge() {
        // Vertical structure: the odd row equals its neighbours.
        let row = [10u8, 20, 30, 40, 50];
        let mut luma = Vec::new();
        luma.extend_from_slice(&row);
        luma.extend_from_slice(&[0; 5]);
        luma.extend_from_slice(&row);
        luma.extend_from_slice(&[0; 5]);
        let out = deinterlace_ela(&luma, 5, 4).unwrap();
        assert_eq!(&out[0..5], &row);
        assert_eq!(&out[5..10], &row);
        assert_eq!(&out[15..20], &row, "last odd row copies the row above");
        // A diagonal edge that moves two pixels between field rows.
        let mut d = Vec::new();
        d.extend_from_slice(&[0u8, 0, 0, 0, 100, 100, 100, 100]);
        d.extend_from_slice(&[0u8; 8]);
        d.extend_from_slice(&[0u8, 0, 100, 100, 100, 100, 100, 100]);
        let out = deinterlace_ela(&d, 8, 3).unwrap();
        assert_eq!(out[8 + 3], 100, "plain vertical averaging would give 50");
    }

    #[test]
    fn temporal_denoise_averages_still_pixels_and_passes_motion() {
        let prev = [0.5f32, 0.0];
        let cur = [0.52f32, 1.0];
        let out = temporal_denoise(&prev, &cur, 0.2, 1.0).unwrap();
        assert!(out[0] < 0.52 && out[0] > 0.5, "still pixel moves towards the previous frame");
        assert_eq!(out[1], 1.0, "large change passes through");
        assert!(temporal_denoise(&prev, &cur[..1], 0.2, 1.0).is_none());
        assert!(temporal_denoise(&prev, &cur, 0.0, 1.0).is_none());
    }

    #[test]
    fn zero_flow_is_identity_and_unit_flow_shifts() {
        let ramp = [0.0f32, 1.0, 2.0, 3.0];
        assert_eq!(flow_warp(&ramp, 4, 1, &[[0.0, 0.0]; 4]).unwrap(), ramp.to_vec());
        assert_eq!(flow_warp(&ramp, 4, 1, &[[1.0, 0.0]; 4]).unwrap(), vec![1.0, 2.0, 3.0, 3.0]);
        let half = flow_warp(&ramp, 4, 1, &[[0.5, 0.0]; 4]).unwrap();
        assert!((half[1] - 1.5).abs() < 1e-6);
        assert!(flow_warp(&ramp, 4, 1, &[[0.0, 0.0]; 3]).is_none());
    }

    #[test]
    fn pyramid_keeps_flat_planes_and_halves_the_size() {
        let (out, w, h) = pyramid_down(&[0.7; 25], 5, 5).unwrap();
        assert_eq!((w, h), (3, 3));
        assert!(out.iter().all(|v| (v - 0.7).abs() < 1e-5));
        assert!(pyramid_down(&[0.0; 3], 2, 2).is_none());
    }
}
