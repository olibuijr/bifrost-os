//! Euclidean distance transform of an 8-bit single-channel plane. Every pixel gets the distance to the nearest
//! nonzero pixel (a nonzero pixel gets 0). The transform is exact: it uses the separable squared-distance
//! method of Felzenszwalb and Huttenlocher, a lower envelope of parabolas per column and then per row, so it runs
//! in time linear in the number of pixels. An optional search radius limits the answer to nonzero pixels within
//! that distance. Plain std code over `filter::Plane`, no GPU and no app-facing entry point yet.

use crate::filter::Plane;

/// Squared distances along one line. `f[q]` is the cost of position `q` (0 for a seed, `big` otherwise), and
/// `d[q]` receives `min over p of (q - p)^2 + f[p]`. Lines of length 0 have nothing to write.
fn squared_1d(f: &[f64], d: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    // The parabola at `v[k]` is the lower envelope on the interval starting at `z[k]`.
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = intersection(f, q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = intersection(f, q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let offset = q as f64 - v[k] as f64;
        *out = offset * offset + f[v[k]];
    }
}

/// Where the parabolas at `p` and `q` (with `p < q`) cross.
fn intersection(f: &[f64], q: usize, p: usize) -> f64 {
    let (qf, pf) = (q as f64, p as f64);
    ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * (qf - pf))
}

/// Euclidean distance from every pixel to the nearest nonzero pixel, row-major, `width * height` values.
///
/// With `search_limit`, a pixel whose nearest nonzero pixel is farther than the limit gets `f32::INFINITY`.
/// The reference page describes the limit as a search radius and does not state the value for a pixel outside
/// it, so the infinity is a BifrOSt choice. A plane with no nonzero pixel is all infinity.
pub fn euclidean_distance(src: &Plane, search_limit: Option<f32>) -> Vec<f32> {
    let (w, h) = (src.width, src.height);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    // Any real squared distance on this plane is below (w + h)^2, so `big` marks an unreachable cell.
    let big = ((w + h) * (w + h)) as f64 + 1.0;
    let mut cols = vec![0.0f64; w * h];
    let mut line = vec![0.0f64; h.max(w)];
    let mut out = vec![0.0f64; h.max(w)];

    for x in 0..w {
        for (y, l) in line.iter_mut().take(h).enumerate() {
            *l = if src.pixels[y * w + x] != 0 { 0.0 } else { big };
        }
        squared_1d(&line[..h], &mut out[..h]);
        for y in 0..h {
            cols[y * w + x] = out[y];
        }
    }

    let limit_sq = search_limit.map(|r| {
        let r = r.max(0.0) as f64;
        r * r
    });
    let mut result = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            line[x] = cols[y * w + x];
        }
        squared_1d(&line[..w], &mut out[..w]);
        for x in 0..w {
            let sq = out[x];
            let d = if sq >= big {
                f64::INFINITY
            } else {
                sq.sqrt()
            };
            let inside = limit_sq.is_none_or(|lim| sq <= lim);
            result[y * w + x] = if inside && d.is_finite() { d as f32 } else { f32::INFINITY };
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_row_gives_the_distance_to_its_only_seed() {
        // Seed at x = 2: distances 2, 1, 0.
        let p = Plane::new(3, 1, vec![0, 0, 9]).unwrap();
        assert_eq!(euclidean_distance(&p, None), vec![2.0, 1.0, 0.0]);
    }

    #[test]
    fn the_centre_seed_of_a_three_by_three_gives_one_and_root_two() {
        // Edge neighbours are 1 away, corners are sqrt(1 + 1) away.
        let mut px = vec![0u8; 9];
        px[4] = 1;
        let d = euclidean_distance(&Plane::new(3, 3, px).unwrap(), None);
        assert!(close(d[4], 0.0));
        assert!(close(d[1], 1.0));
        assert!(close(d[3], 1.0));
        assert!(close(d[0], 2.0f32.sqrt()));
        assert!(close(d[8], 2.0f32.sqrt()));
    }

    #[test]
    fn a_diagonal_offset_uses_the_full_euclidean_distance() {
        // Seeds at (0, 0) and (4, 4) on a 5 x 5 plane. Pixel (2, 2) is 2 sqrt(2) from both.
        let mut px = vec![0u8; 25];
        px[0] = 1;
        px[24] = 1;
        let d = euclidean_distance(&Plane::new(5, 5, px).unwrap(), None);
        assert!(close(d[12], 8.0f32.sqrt()));
        // Pixel (1, 0) is 1 from the seed at the origin.
        assert!(close(d[1], 1.0));
    }

    #[test]
    fn a_plane_without_seeds_is_all_infinity() {
        let d = euclidean_distance(&Plane::filled(2, 2, 0), None);
        assert!(d.iter().all(|v| v.is_infinite()));
    }

    #[test]
    fn the_search_limit_cuts_off_pixels_beyond_the_radius() {
        // Seed at x = 0 on a 3 x 1 row, limit 1.5: distances 0, 1, then infinity for x = 2 (distance 2).
        let p = Plane::new(3, 1, vec![1, 0, 0]).unwrap();
        let d = euclidean_distance(&p, Some(1.5));
        assert!(close(d[0], 0.0));
        assert!(close(d[1], 1.0));
        assert!(d[2].is_infinite());
    }

    #[test]
    fn an_empty_plane_gives_no_values() {
        let p = Plane::new(0, 4, Vec::new()).unwrap();
        assert!(euclidean_distance(&p, None).is_empty());
    }
}
