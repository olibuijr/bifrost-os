//! Fourier and cosine transforms over `f64` slices: an in-place radix-2 complex FFT and its inverse, a direct DFT for any
//! length, and the unnormalized DCT-II with its inverse. Mirrors the transform routines of the vector maths framework
//! (`vDSP` Fourier and cosine functions). Pure functions, no I/O.
//!
//! Conventions (the same as the classic definitions, no hidden scale factors except where noted):
//! * Forward DFT: `X[k] = sum_n x[n] * e^(-2 pi i k n / N)`.
//! * `fft` works in place on separate real and imaginary slices and needs a power-of-two length.
//! * `ifft` includes the `1/N` scale, so `ifft(fft(x)) == x`.
//! * DCT-II: `X[k] = sum_n x[n] * cos(pi / N * (n + 1/2) * k)`, unnormalized.
//! * `idct2` is the matching inverse, `x[n] = (X[0] + 2 * sum_{k>=1} X[k] * cos(...)) / N`, so `idct2(dct2(x)) == x`.
//!
//! Limits: no real-input packing, no multi-signal batch call, no single-precision variants, and the direct DFT and DCT
//! are O(N^2). Only power-of-two lengths get the fast path.

use std::f64::consts::PI;
use std::fmt;

/// `FourierError`: why a transform could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FourierError {
    /// The input is empty.
    Empty,
    /// The real and imaginary slices have different lengths.
    LengthMismatch,
    /// The fast transform needs a power-of-two length; the length is carried.
    NotPowerOfTwo(usize),
}

impl fmt::Display for FourierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FourierError::Empty => write!(f, "the input is empty"),
            FourierError::LengthMismatch => write!(f, "the real and imaginary parts differ in length"),
            FourierError::NotPowerOfTwo(n) => write!(f, "the length {} is not a power of two", n),
        }
    }
}

/// Shared radix-2 decimation-in-time kernel. `inverse` flips the sign of the exponent and applies the `1/N` scale.
fn radix2(re: &mut [f64], im: &mut [f64], inverse: bool) -> Result<(), FourierError> {
    let n = re.len();
    if n == 0 {
        return Err(FourierError::Empty);
    }
    if im.len() != n {
        return Err(FourierError::LengthMismatch);
    }
    if !n.is_power_of_two() {
        return Err(FourierError::NotPowerOfTwo(n));
    }
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Butterflies, doubling the span each stage.
    let sign = if inverse { 1.0f64 } else { -1.0f64 };
    let mut span = 2usize;
    while span <= n {
        let half = span / 2;
        let step = sign * 2.0 * PI / span as f64;
        for start in (0..n).step_by(span) {
            for k in 0..half {
                let (s, c) = (step * k as f64).sin_cos();
                let a = start + k;
                let b = a + half;
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        span <<= 1;
    }
    if inverse {
        let scale = 1.0f64 / n as f64;
        for v in re.iter_mut() {
            *v *= scale;
        }
        for v in im.iter_mut() {
            *v *= scale;
        }
    }
    Ok(())
}

/// In-place forward FFT of the complex sequence `re + i * im`. The length must be a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) -> Result<(), FourierError> {
    radix2(re, im, false)
}

/// In-place inverse FFT, scaled by `1/N`. The length must be a power of two.
pub fn ifft(re: &mut [f64], im: &mut [f64]) -> Result<(), FourierError> {
    radix2(re, im, true)
}

/// Direct DFT of any non-empty length. Returns the real and imaginary parts of the spectrum.
pub fn dft(re: &[f64], im: &[f64]) -> Result<(Vec<f64>, Vec<f64>), FourierError> {
    let n = re.len();
    if n == 0 {
        return Err(FourierError::Empty);
    }
    if im.len() != n {
        return Err(FourierError::LengthMismatch);
    }
    let mut out_re = vec![0.0f64; n];
    let mut out_im = vec![0.0f64; n];
    for k in 0..n {
        let mut sr = 0.0f64;
        let mut si = 0.0f64;
        for m in 0..n {
            // Reduce k * m modulo n first so the angle stays small and exact for small inputs.
            let angle = -2.0 * PI * ((k * m) % n) as f64 / n as f64;
            let (s, c) = angle.sin_cos();
            sr += re[m] * c - im[m] * s;
            si += re[m] * s + im[m] * c;
        }
        out_re[k] = sr;
        out_im[k] = si;
    }
    Ok((out_re, out_im))
}

/// Unnormalized DCT-II of a real sequence of any non-empty length.
pub fn dct2(x: &[f64]) -> Result<Vec<f64>, FourierError> {
    let n = x.len();
    if n == 0 {
        return Err(FourierError::Empty);
    }
    let nf = n as f64;
    let mut out = vec![0.0f64; n];
    for (k, slot) in out.iter_mut().enumerate() {
        let mut acc = 0.0f64;
        for (m, v) in x.iter().enumerate() {
            acc += v * (PI / nf * (m as f64 + 0.5) * k as f64).cos();
        }
        *slot = acc;
    }
    Ok(out)
}

/// Inverse of `dct2`: `idct2(dct2(x))` returns `x` up to rounding.
pub fn idct2(spectrum: &[f64]) -> Result<Vec<f64>, FourierError> {
    let n = spectrum.len();
    if n == 0 {
        return Err(FourierError::Empty);
    }
    let nf = n as f64;
    let mut out = vec![0.0f64; n];
    for (m, slot) in out.iter_mut().enumerate() {
        let mut acc = spectrum[0];
        for (k, v) in spectrum.iter().enumerate().skip(1) {
            acc += 2.0f64 * *v * (PI / nf * (m as f64 + 0.5) * k as f64).cos();
        }
        *slot = acc / nf;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn fft_of_an_impulse_is_flat() {
        let mut re = vec![1.0, 0.0, 0.0, 0.0];
        let mut im = vec![0.0; 4];
        fft(&mut re, &mut im).unwrap();
        assert!(re.iter().all(|v| close(*v, 1.0)));
        assert!(im.iter().all(|v| close(*v, 0.0)));
    }

    #[test]
    fn fft_of_a_constant_is_one_bin() {
        let mut re = vec![1.0; 4];
        let mut im = vec![0.0; 4];
        fft(&mut re, &mut im).unwrap();
        assert!(close(re[0], 4.0) && close(im[0], 0.0));
        for k in 1..4 {
            assert!(close(re[k], 0.0) && close(im[k], 0.0), "bin {} not zero", k);
        }
    }

    #[test]
    fn fft_of_one_two_three_four_matches_the_hand_result() {
        // X0 = 10, X1 = -2 + 2i, X2 = -2, X3 = -2 - 2i.
        let mut re = vec![1.0, 2.0, 3.0, 4.0];
        let mut im = vec![0.0; 4];
        fft(&mut re, &mut im).unwrap();
        assert!(close(re[0], 10.0) && close(im[0], 0.0));
        assert!(close(re[1], -2.0) && close(im[1], 2.0));
        assert!(close(re[2], -2.0) && close(im[2], 0.0));
        assert!(close(re[3], -2.0) && close(im[3], -2.0));
    }

    #[test]
    fn ifft_undoes_fft() {
        let mut re = vec![1.0, 2.0, 3.0, 4.0];
        let mut im = vec![0.0; 4];
        fft(&mut re, &mut im).unwrap();
        ifft(&mut re, &mut im).unwrap();
        for (got, want) in re.iter().zip([1.0, 2.0, 3.0, 4.0].iter()) {
            assert!(close(*got, *want));
        }
        assert!(im.iter().all(|v| close(*v, 0.0)));
    }

    #[test]
    fn fft_rejects_bad_lengths() {
        let mut re = vec![1.0, 2.0, 3.0];
        let mut im = vec![0.0; 3];
        assert_eq!(fft(&mut re, &mut im), Err(FourierError::NotPowerOfTwo(3)));
        let mut empty: Vec<f64> = Vec::new();
        let mut no_im: Vec<f64> = Vec::new();
        assert_eq!(fft(&mut empty, &mut no_im), Err(FourierError::Empty));
        let mut short_im = vec![0.0; 2];
        let mut re4 = vec![1.0; 4];
        assert_eq!(fft(&mut re4, &mut short_im), Err(FourierError::LengthMismatch));
    }

    #[test]
    fn dft_handles_any_length_and_agrees_with_fft() {
        // DFT of [1, 0, 0] is all ones, length 3 has no fast path.
        let (r, i) = dft(&[1.0, 0.0, 0.0], &[0.0, 0.0, 0.0]).unwrap();
        assert!(r.iter().all(|v| close(*v, 1.0)));
        assert!(i.iter().all(|v| close(*v, 0.0)));
        // Same spectrum as the fast path on a length-4 input.
        let (dr, di) = dft(&[1.0, 2.0, 3.0, 4.0], &[0.0; 4]).unwrap();
        assert!(close(dr[1], -2.0) && close(di[1], 2.0));
        assert!(close(dr[3], -2.0) && close(di[3], -2.0));
    }

    #[test]
    fn dct2_of_a_constant_is_one_dc_term() {
        // X0 = 4; for k >= 1 the cosines of (n + 1/2) * k * pi / 4 cancel in pairs.
        let x = dct2(&[1.0, 1.0, 1.0, 1.0]).unwrap();
        assert!(close(x[0], 4.0));
        for k in 1..4 {
            assert!(close(x[k], 0.0), "coefficient {} not zero", k);
        }
    }

    #[test]
    fn dct2_of_an_impulse_is_a_cosine_row_and_inverts() {
        // X[k] = cos(pi * k / 8) for an impulse at n = 0.
        let x = dct2(&[1.0, 0.0, 0.0, 0.0]).unwrap();
        assert!(close(x[1], (PI / 8.0).cos()));
        assert!(close(x[2], (PI / 4.0).cos()));
        let back = idct2(&x).unwrap();
        assert!(close(back[0], 1.0));
        assert!(back[1..].iter().all(|v| close(*v, 0.0)));
    }

    #[test]
    fn dct_rejects_empty_input() {
        assert_eq!(dct2(&[]), Err(FourierError::Empty));
        assert_eq!(idct2(&[]), Err(FourierError::Empty));
    }
}
