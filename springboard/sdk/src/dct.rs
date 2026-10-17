//! Discrete cosine transform for apps: an orthonormal DCT-II in place, and its inverse (DCT-III).
//!
//! The reference vector library offers a DCT of any length on real buffers. This module is the app-facing equivalent:
//! a real slice goes in, its cosine coefficients come out in place, and [`idct2`] reverses it. The scaling is
//! orthonormal, so the transform preserves energy (Parseval) and `idct2(dct2(x)) == x`. The forward transform uses
//! `X[k] = s(k) * sum_n x[n] * cos(pi / N * (n + 1/2) * k)` with `s(0) = sqrt(1/N)` and `s(k) = sqrt(2/N)` otherwise.
//! The direct form costs O(N^2), which is fine for the block sizes apps use; an empty input is a no-op.

use std::f64::consts::PI;

/// Orthonormal DCT-II of `data`, in place.
pub fn dct2(data: &mut [f64]) {
    let n = data.len();
    if n == 0 {
        return;
    }
    let input = data.to_vec();
    for (k, out) in data.iter_mut().enumerate() {
        let sum: f64 = input
            .iter()
            .enumerate()
            .map(|(i, x)| x * (PI / n as f64 * (i as f64 + 0.5) * k as f64).cos())
            .sum();
        *out = sum * scale(k, n);
    }
}

/// Inverse of [`dct2`] (the orthonormal DCT-III), in place.
pub fn idct2(data: &mut [f64]) {
    let n = data.len();
    if n == 0 {
        return;
    }
    let input = data.to_vec();
    for (i, out) in data.iter_mut().enumerate() {
        let sum: f64 = input
            .iter()
            .enumerate()
            .map(|(k, x)| scale(k, n) * x * (PI / n as f64 * (i as f64 + 0.5) * k as f64).cos())
            .sum();
        *out = sum;
    }
}

/// The orthonormal scale factor for coefficient `k` of an `n` point transform.
fn scale(k: usize, n: usize) -> f64 {
    if k == 0 {
        (1.0 / n as f64).sqrt()
    } else {
        (2.0 / n as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn empty_and_single_inputs_are_handled() {
        let mut empty: Vec<f64> = Vec::new();
        dct2(&mut empty);
        idct2(&mut empty);
        assert!(empty.is_empty());

        // A single point is its own transform: s(0) = sqrt(1/1) = 1.
        let mut one = vec![3.5];
        dct2(&mut one);
        assert!(close(one[0], 3.5));
    }

    #[test]
    fn a_constant_has_only_a_dc_coefficient() {
        let mut d = vec![2.0; 8];
        dct2(&mut d);
        assert!(close(d[0], 2.0 * (8.0f64).sqrt()));
        assert!(d[1..].iter().all(|v| close(*v, 0.0)));
    }

    #[test]
    fn an_impulse_at_zero_has_flat_magnitude_scaled_by_s_k() {
        let n = 4;
        let mut d = vec![0.0; n];
        d[0] = 1.0;
        dct2(&mut d);
        assert!(close(d[0], 1.0 / (n as f64).sqrt()));
        for (k, v) in d.iter().enumerate().skip(1) {
            let expected = (2.0 / n as f64).sqrt() * (PI / (2.0 * n as f64) * k as f64).cos();
            assert!(close(*v, expected), "bin {k}");
        }
    }

    #[test]
    fn inverse_undoes_forward() {
        let original = vec![0.25, -1.5, 3.0, 7.75, 0.0, -2.0, 1.25];
        let mut d = original.clone();
        dct2(&mut d);
        idct2(&mut d);
        for (a, b) in d.iter().zip(original.iter()) {
            assert!(close(*a, *b));
        }
    }

    #[test]
    fn energy_is_preserved() {
        let original = vec![1.0, 2.0, -3.0, 4.0, 0.5, -0.5];
        let mut d = original.clone();
        dct2(&mut d);
        let before: f64 = original.iter().map(|x| x * x).sum();
        let after: f64 = d.iter().map(|x| x * x).sum();
        assert!(close(before, after));
    }
}
