//! Image similarity for the on-device vision pipeline: a compact feature print of a picture and a 64-bit difference
//! hash, with the distances that compare them. They answer "does this look like that" for duplicate detection,
//! "more like this" in a photo library and nearest-picture search, without a trained model: the print is built from
//! classical statistics (a coarse tone grid, a gradient-direction histogram and a tone histogram) that
//! `kernel_vision` already computes the pieces of.
//!
//! The planes are single-channel `f32` in row-major order, like the other `kernel_vision` kernels (the scale does not
//! matter: the print is the same for `a * x + b` with `a > 0`). Written from the textbook descriptions of each
//! statistic; nothing is copied from a reference implementation.

use crate::kernel_vision::sobel;

/// The tone grid is `GRID x GRID` cells of mean tone.
pub const GRID: usize = 8;
/// Gradient directions are folded to a half turn and counted in this many bins.
pub const ORIENT_BINS: usize = 8;
/// Tones (after stretching the picture to its own range) are counted in this many bins.
pub const TONE_BINS: usize = 8;
/// Gradient magnitudes below this (tone 0..1 units) are rounding noise, not edges.
const EDGE_FLOOR: f32 = 1e-4;
/// Values in a feature print.
pub const PRINT_LEN: usize = GRID * GRID + ORIENT_BINS + TONE_BINS;

/// The compact description of a picture's look: tone layout, edge directions and tone spread, each part normalised
/// to unit length so no part drowns the others. Two prints are compared with [`distance`].
#[derive(Clone, Debug, PartialEq)]
pub struct FeaturePrint(pub Vec<f32>);

fn valid(src: &[f32], w: usize, h: usize) -> bool {
    w > 0 && h > 0 && src.len() == w * h
}

/// The mean of the plane in each cell of a `gw x gh` grid laid over it. Every cell covers at least one pixel, so a
/// plane smaller than the grid repeats its pixels instead of leaving cells empty.
fn cell_means(src: &[f32], w: usize, h: usize, gw: usize, gh: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(gw * gh);
    for cy in 0..gh {
        let y0 = cy * h / gh;
        let y1 = ((cy + 1) * h / gh).max(y0 + 1).min(h);
        for cx in 0..gw {
            let x0 = cx * w / gw;
            let x1 = ((cx + 1) * w / gw).max(x0 + 1).min(w);
            let mut sum = 0.0f64;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += src[y * w + x] as f64;
                }
            }
            out.push((sum / ((y1 - y0) * (x1 - x0)) as f64) as f32);
        }
    }
    out
}

/// Scales a part to unit length; a part of zeros stays zeros.
fn unit(part: &mut [f32]) {
    let norm = part.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for v in part.iter_mut() {
            *v /= norm;
        }
    }
}

/// The feature print of a plane, or `None` when the size does not match the data. A flat picture gives a valid print
/// (a zero grid, no edge directions, every tone in the first bin).
pub fn feature_print(src: &[f32], w: usize, h: usize) -> Option<FeaturePrint> {
    if !valid(src, w, h) {
        return None;
    }
    // tone layout: the grid of means, centred and scaled by its spread, so brightness and contrast drop out
    let mut grid = cell_means(src, w, h, GRID, GRID);
    let mean = grid.iter().sum::<f32>() / grid.len() as f32;
    let var = grid.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / grid.len() as f32;
    let spread = var.sqrt();
    for v in grid.iter_mut() {
        *v = if spread > 1e-6 { (*v - mean) / spread } else { 0.0 };
    }
    unit(&mut grid);

    // edge directions: gradient magnitude per direction, with directions folded to a half turn (an edge has no sign)
    let (mag, dir) = sobel(src, w, h)?;
    let mut orient = vec![0.0f32; ORIENT_BINS];
    for (m, d) in mag.iter().zip(&dir) {
        // rounding leaves magnitudes near 1e-7 on a flat area; `unit` would blow that noise up to a full edge
        if *m < EDGE_FLOOR {
            continue;
        }
        let mut a = *d;
        if a < 0.0 {
            a += std::f32::consts::PI;
        }
        let bin = ((a / std::f32::consts::PI * ORIENT_BINS as f32).round() as usize) % ORIENT_BINS;
        orient[bin] += *m;
    }
    unit(&mut orient);

    // tone spread: the share of pixels in each tone bin once the picture is stretched to its own range
    let lo = src.iter().cloned().fold(f32::INFINITY, f32::min);
    let hi = src.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut tone = vec![0.0f32; TONE_BINS];
    for v in src {
        let t = if hi - lo > 1e-9 { (v - lo) / (hi - lo) } else { 0.0 };
        let bin = ((t * TONE_BINS as f32) as usize).min(TONE_BINS - 1);
        tone[bin] += 1.0;
    }
    unit(&mut tone);

    let mut all = grid;
    all.extend(orient);
    all.extend(tone);
    Some(FeaturePrint(all))
}

/// How far apart two prints are: the Euclidean distance of their values, 0 for the same look. `None` when the prints
/// are not the same length.
pub fn distance(a: &FeaturePrint, b: &FeaturePrint) -> Option<f32> {
    if a.0.len() != b.0.len() {
        return None;
    }
    Some(a.0.iter().zip(&b.0).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt())
}

/// The candidate nearest to `query` as `(index, distance)`, or `None` when there are no candidates or none can be
/// compared. The first of equally near candidates wins.
pub fn nearest(query: &FeaturePrint, candidates: &[FeaturePrint]) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (i, c) in candidates.iter().enumerate() {
        if let Some(d) = distance(query, c) {
            if best.map_or(true, |(_, b)| d < b) {
                best = Some((i, d));
            }
        }
    }
    best
}

/// The 64-bit difference hash of a plane: it is shrunk to 9 columns by 8 rows and each bit says whether a cell is
/// brighter than the cell to its right. Small edits (resizing, brightness, mild compression) leave most bits as they
/// were, so near-duplicates differ in a few bits. `None` when the size does not match the data.
pub fn difference_hash(src: &[f32], w: usize, h: usize) -> Option<u64> {
    if !valid(src, w, h) {
        return None;
    }
    let cells = cell_means(src, w, h, 9, 8);
    let mut hash = 0u64;
    for row in 0..8 {
        for col in 0..8 {
            if cells[row * 9 + col] > cells[row * 9 + col + 1] {
                hash |= 1 << (row * 8 + col);
            }
        }
    }
    Some(hash)
}

/// How many bits differ between two hashes: 0 for the same picture, 64 for opposite ones.
pub fn hash_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Whether two hashes are of near-duplicate pictures: at most `max_bits` bits differ (about 10 suits resized or
/// re-compressed copies).
pub fn is_near_duplicate(a: u64, b: u64, max_bits: u32) -> bool {
    hash_distance(a, b) <= max_bits
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 32;

    /// Tone grows to the right.
    fn ramp_x() -> Vec<f32> {
        (0..N * N).map(|i| (i % N) as f32 / (N - 1) as f32).collect()
    }

    /// Tone grows downwards.
    fn ramp_y() -> Vec<f32> {
        (0..N * N).map(|i| (i / N) as f32 / (N - 1) as f32).collect()
    }

    fn checker() -> Vec<f32> {
        (0..N * N).map(|i| if ((i % N) / 4 + (i / N) / 4) % 2 == 0 { 0.0 } else { 1.0 }).collect()
    }

    fn print(p: &[f32]) -> FeaturePrint {
        feature_print(p, N, N).unwrap()
    }

    #[test]
    fn a_print_has_the_documented_length_and_a_bad_size_has_none() {
        assert_eq!(print(&ramp_x()).0.len(), PRINT_LEN);
        assert!(feature_print(&[0.0; 10], 4, 4).is_none());
        assert!(feature_print(&[], 0, 0).is_none());
        assert!(difference_hash(&[0.0; 10], 4, 4).is_none());
    }

    #[test]
    fn the_same_picture_is_at_distance_zero() {
        let a = print(&checker());
        assert_eq!(distance(&a, &a), Some(0.0));
    }

    #[test]
    fn brightness_and_contrast_do_not_change_the_look() {
        let plain = ramp_x();
        let adjusted: Vec<f32> = plain.iter().map(|v| 0.5 * v + 0.25).collect();
        let d = distance(&print(&plain), &print(&adjusted)).unwrap();
        assert!(d < 1e-3, "distance {d}");
    }

    #[test]
    fn different_layouts_are_far_apart() {
        let d = distance(&print(&ramp_x()), &print(&ramp_y())).unwrap();
        assert!(d > 1.0, "a horizontal and a vertical ramp differ in layout and edge direction: {d}");
        let e = distance(&print(&ramp_x()), &print(&checker())).unwrap();
        assert!(e > 1.0, "{e}");
    }

    #[test]
    fn a_flat_picture_has_a_finite_print() {
        let flat = print(&vec![0.4; N * N]);
        assert!(flat.0.iter().all(|v| v.is_finite()));
        assert!(flat.0[..GRID * GRID].iter().all(|v| *v == 0.0), "no layout");
        assert!(flat.0[GRID * GRID..GRID * GRID + ORIENT_BINS].iter().all(|v| *v == 0.0), "no edges");
        assert_eq!(flat.0[PRINT_LEN - TONE_BINS], 1.0, "every pixel is in the first tone bin");
    }

    #[test]
    fn the_nearest_candidate_is_the_one_that_looks_alike() {
        let query = print(&ramp_x().iter().map(|v| 0.8 * v + 0.1).collect::<Vec<_>>());
        let candidates = [print(&checker()), print(&ramp_y()), print(&ramp_x())];
        let (i, d) = nearest(&query, &candidates).unwrap();
        assert_eq!(i, 2);
        assert!(d < 1e-3);
        assert_eq!(nearest(&query, &[]), None);
        assert_eq!(nearest(&query, &[FeaturePrint(vec![0.0; 3])]), None, "a print of another length cannot be compared");
    }

    #[test]
    fn prints_of_different_length_have_no_distance() {
        assert_eq!(distance(&FeaturePrint(vec![1.0]), &FeaturePrint(vec![1.0, 2.0])), None);
    }

    #[test]
    fn a_small_plane_still_fills_every_cell() {
        let tiny = feature_print(&[0.0, 1.0, 1.0, 0.0], 2, 2).unwrap();
        assert_eq!(tiny.0.len(), PRINT_LEN);
        assert!(tiny.0.iter().all(|v| v.is_finite()));
        assert!(difference_hash(&[0.0, 1.0], 2, 1).is_some());
    }

    #[test]
    fn the_hash_ignores_brightness_and_flips_for_the_opposite_picture() {
        let plain = ramp_x();
        let bright: Vec<f32> = plain.iter().map(|v| v * 0.5 + 0.3).collect();
        let reversed: Vec<f32> = plain.iter().map(|v| 1.0 - v).collect();
        let h = difference_hash(&plain, N, N).unwrap();
        assert_eq!(h, 0, "tone only grows to the right, so no cell is brighter than its right neighbour");
        assert_eq!(difference_hash(&bright, N, N), Some(h));
        assert_eq!(hash_distance(h, difference_hash(&reversed, N, N).unwrap()), 64);
        assert_eq!(difference_hash(&vec![0.5; N * N], N, N), Some(0), "a flat picture has no brighter cell");
    }

    #[test]
    fn hash_distance_counts_bits_and_near_duplicates_allow_a_few() {
        assert_eq!(hash_distance(0b1011, 0b0010), 2);
        assert_eq!(hash_distance(5, 5), 0);
        assert_eq!(hash_distance(0, u64::MAX), 64);
        assert!(is_near_duplicate(0b1111, 0b1100, 2));
        assert!(!is_near_duplicate(0b1111, 0b1100, 1));
    }

    #[test]
    fn the_hash_tells_a_checker_from_a_ramp() {
        let c = difference_hash(&checker(), N, N).unwrap();
        let r = difference_hash(&ramp_x(), N, N).unwrap();
        assert!(hash_distance(c, r) > 10, "{}", hash_distance(c, r));
    }
}
