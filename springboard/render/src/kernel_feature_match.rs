//! Feature detection and descriptor matching, the kernels under image alignment and photogrammetry:
//! a FAST-style segment-test corner detector (a pixel is a corner when nine contiguous pixels of the
//! 16-pixel circle of radius 3 are all brighter or all darker than it by a threshold) with 3x3
//! non-maximum suppression, a BRIEF-style 256-bit binary descriptor (intensity comparisons between point
//! pairs in the smoothed patch around a keypoint, with the pairs derived from a fixed integer hash so the
//! pattern is identical on every machine), Hamming distance, and brute-force matching with Lowe's ratio
//! test and an optional mutual (cross) check. Dense two-view disparity is in `stereo`; blob detection is in
//! `feature_detect`. Written from the published descriptions of FAST, BRIEF and the ratio test.

use crate::kernel_materialx::hash32;

/// A 256-bit binary descriptor.
pub type Descriptor = [u64; 4];

/// A detected corner: pixel position and a score (larger is stronger).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corner {
    pub x: usize,
    pub y: usize,
    pub score: u32,
}

const CIRCLE: [(i32, i32); 16] = [
    (0, -3),
    (1, -3),
    (2, -2),
    (3, -1),
    (3, 0),
    (3, 1),
    (2, 2),
    (1, 3),
    (0, 3),
    (-1, 3),
    (-2, 2),
    (-3, 1),
    (-3, 0),
    (-3, -1),
    (-2, -2),
    (-1, -3),
];

/// The segment-test score of pixel `(x, y)`: zero unless nine contiguous circle pixels are all brighter
/// or all darker than the centre by more than `threshold`; then the summed excess over the whole circle.
fn corner_score(img: &[u8], w: usize, x: usize, y: usize, threshold: i32) -> u32 {
    let centre = img[y * w + x] as i32;
    let mut state = [0i8; 16];
    let mut score = 0u32;
    for (k, (dx, dy)) in CIRCLE.iter().enumerate() {
        let v = img[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize] as i32;
        let d = v - centre;
        if d > threshold {
            state[k] = 1;
            score += (d - threshold) as u32;
        } else if d < -threshold {
            state[k] = -1;
            score += (-d - threshold) as u32;
        }
    }
    for sign in [1i8, -1i8] {
        let mut run = 0;
        // Walk the circle twice so a run can wrap around the start.
        for k in 0..32 {
            if state[k % 16] == sign {
                run += 1;
                if run >= 9 {
                    return score;
                }
            } else {
                run = 0;
            }
        }
    }
    0
}

/// Detects segment-test corners. The three-pixel border is never reported. `None` on a size mismatch.
pub fn fast_corners(img: &[u8], w: usize, h: usize, threshold: u8) -> Option<Vec<Corner>> {
    if w == 0 || h == 0 || img.len() != w * h {
        return None;
    }
    let mut scores = vec![0u32; w * h];
    if w > 6 && h > 6 {
        for y in 3..h - 3 {
            for x in 3..w - 3 {
                scores[y * w + x] = corner_score(img, w, x, y, threshold as i32);
            }
        }
    }
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let s = scores[y * w + x];
            if s == 0 {
                continue;
            }
            let mut keep = true;
            'nms: for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let n = scores[ny as usize * w + nx as usize];
                    // Neighbours earlier in raster order must be strictly weaker, later ones not stronger.
                    let earlier = dy < 0 || (dy == 0 && dx < 0);
                    if (earlier && n >= s) || (!earlier && n > s) {
                        keep = false;
                        break 'nms;
                    }
                }
            }
            if keep {
                out.push(Corner { x, y, score: s });
            }
        }
    }
    Some(out)
}

/// The patch radius a descriptor needs around its keypoint (sample offsets reach 12, the smoothing 1).
pub const PATCH_RADIUS: usize = 13;

fn smoothed(img: &[u8], w: usize, x: usize, y: usize) -> u32 {
    let mut s = 0u32;
    for dy in 0..3 {
        for dx in 0..3 {
            s += img[(y + dy - 1) * w + x + dx - 1] as u32;
        }
    }
    s / 9
}

/// The i-th test pair as `(dx1, dy1, dx2, dy2)`, each in -12..=12, from the fixed integer hash.
fn test_pair(i: u32) -> (i32, i32, i32, i32) {
    let c = |k: u32| (hash32(i.wrapping_mul(4).wrapping_add(k).wrapping_add(0x9e37_79b9)) % 25) as i32 - 12;
    (c(0), c(1), c(2), c(3))
}

/// The 256-bit descriptor of a keypoint, or `None` when the patch leaves the image.
pub fn describe(img: &[u8], w: usize, h: usize, x: usize, y: usize) -> Option<Descriptor> {
    if w == 0 || h == 0 || img.len() != w * h || x < PATCH_RADIUS || y < PATCH_RADIUS || x + PATCH_RADIUS >= w || y + PATCH_RADIUS >= h {
        return None;
    }
    let mut d = [0u64; 4];
    for bit in 0..256u32 {
        let (x1, y1, x2, y2) = test_pair(bit);
        let a = smoothed(img, w, (x as i32 + x1) as usize, (y as i32 + y1) as usize);
        let b = smoothed(img, w, (x as i32 + x2) as usize, (y as i32 + y2) as usize);
        if a < b {
            d[(bit / 64) as usize] |= 1u64 << (bit % 64);
        }
    }
    Some(d)
}

/// The number of differing bits.
pub fn hamming(a: &Descriptor, b: &Descriptor) -> u32 {
    (0..4).map(|i| (a[i] ^ b[i]).count_ones()).sum()
}

/// A match between descriptor `a` of the first set and `b` of the second, with its Hamming distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub a: usize,
    pub b: usize,
    pub distance: u32,
}

fn two_best(d: &Descriptor, set: &[Descriptor]) -> Option<(usize, u32, u32)> {
    let mut best: Option<(usize, u32)> = None;
    let mut second = u32::MAX;
    for (i, e) in set.iter().enumerate() {
        let dist = hamming(d, e);
        let current = best;
        match current {
            None => best = Some((i, dist)),
            Some((_, bd)) => {
                if dist < bd {
                    second = bd;
                    best = Some((i, dist));
                } else if dist < second {
                    second = dist;
                }
            }
        }
    }
    best.map(|(i, bd)| (i, bd, second))
}

/// Brute-force matching of `a` into `b`. A match is kept when the best distance is strictly below `ratio`
/// times the second best (Lowe's test; a lone candidate always passes the ratio) and, with `cross_check`, when
/// the best match of that `b` descriptor back into `a` is the same `a` descriptor.
pub fn match_descriptors(a: &[Descriptor], b: &[Descriptor], ratio: f32, cross_check: bool) -> Vec<Match> {
    let mut out = Vec::new();
    for (i, d) in a.iter().enumerate() {
        let Some((j, best, second)) = two_best(d, b) else {
            continue;
        };
        if second != u32::MAX && best as f32 >= ratio * second as f32 {
            continue;
        }
        if cross_check {
            match two_best(&b[j], a) {
                Some((back, _, _)) if back == i => {}
                _ => continue,
            }
        }
        out.push(Match { a: i, b: j, distance: best });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_image() -> Vec<u8> {
        let mut img = vec![0u8; 400];
        for y in 7..12 {
            for x in 7..12 {
                img[y * 20 + x] = 255;
            }
        }
        img
    }

    #[test]
    fn fast_finds_the_square_corner_and_not_its_inside_or_flat_ground() {
        let corners = fast_corners(&square_image(), 20, 20, 50).unwrap();
        assert!(corners.iter().any(|c| (c.x as i32 - 7).abs() <= 1 && (c.y as i32 - 7).abs() <= 1), "{corners:?}");
        assert!(!corners.iter().any(|c| c.x == 9 && c.y == 9));
        assert!(fast_corners(&vec![10u8; 400], 20, 20, 20).unwrap().is_empty());
        assert!(fast_corners(&[0u8; 10], 20, 20, 20).is_none());
        assert!(fast_corners(&[0u8; 25], 5, 5, 20).unwrap().is_empty(), "too small for the circle");
    }

    #[test]
    fn hamming_counts_bits() {
        let a = [0u64, 0, 0, 0];
        let b = [0b1011u64, 0, 0, u64::MAX];
        assert_eq!(hamming(&a, &b), 3 + 64);
        assert_eq!(hamming(&b, &b), 0);
    }

    fn textured(seed: u32) -> Vec<u8> {
        (0..80u32 * 80).map(|i| (hash32(i ^ seed) & 0xff) as u8).collect()
    }

    #[test]
    fn descriptors_need_a_full_patch_and_are_deterministic() {
        let img = textured(1);
        assert!(describe(&img, 80, 80, 5, 40).is_none());
        assert!(describe(&img, 80, 80, 40, 70).is_none());
        let a = describe(&img, 80, 80, 40, 40).unwrap();
        assert_eq!(a, describe(&img, 80, 80, 40, 40).unwrap());
        let other = describe(&img, 80, 80, 60, 40).unwrap();
        assert!(hamming(&a, &other) > 80, "distant keypoints differ");
    }

    #[test]
    fn shifted_copy_matches_point_for_point() {
        let a_img = textured(7);
        let mut b_img = vec![0u8; 80 * 80];
        for y in 0..80usize {
            for x in 0..80usize {
                b_img[y * 80 + x] = if x >= 5 && y >= 3 { a_img[(y - 3) * 80 + x - 5] } else { (hash32((y * 80 + x) as u32 ^ 0xabcd) & 0xff) as u8 };
            }
        }
        let pts: Vec<(usize, usize)> = (0..10).map(|i| (20 + 3 * i, 20 + 2 * i)).collect();
        let da: Vec<Descriptor> = pts.iter().map(|(x, y)| describe(&a_img, 80, 80, *x, *y).unwrap()).collect();
        // The second set is in reverse order, so the matcher must find the permutation.
        let db: Vec<Descriptor> = pts.iter().rev().map(|(x, y)| describe(&b_img, 80, 80, x + 5, y + 3).unwrap()).collect();
        let matches = match_descriptors(&da, &db, 0.8, true);
        assert_eq!(matches.len(), 10);
        for m in matches {
            assert_eq!(m.b, 9 - m.a);
            assert_eq!(m.distance, 0);
        }
    }

    #[test]
    fn ratio_test_rejects_ambiguous_matches_and_empty_sets_match_nothing() {
        let d = [1u64, 2, 3, 4];
        // Two identical candidates: best == second, so a ratio below 1 refuses the match.
        assert!(match_descriptors(&[d], &[d, d], 0.8, false).is_empty());
        // A lone candidate passes.
        assert_eq!(match_descriptors(&[d], &[d], 0.8, true).len(), 1);
        assert!(match_descriptors(&[d], &[], 0.8, false).is_empty());
        assert!(match_descriptors(&[], &[d], 0.8, false).is_empty());
    }

    #[test]
    fn cross_check_drops_one_sided_matches() {
        let near = [0u64, 0, 0, 0];
        let a1 = [1u64, 0, 0, 0];
        // Both a descriptors prefer the same b; only the closer one survives the mutual check.
        let matches = match_descriptors(&[near, a1], &[near, [u64::MAX; 4]], 0.9, true);
        assert_eq!(matches, vec![Match { a: 0, b: 0, distance: 0 }]);
    }
}
