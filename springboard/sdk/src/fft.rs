//! Fast Fourier transform for apps: an in-place radix-2 complex FFT and its inverse.
//!
//! The reference vector library offers a power-of-two FFT on split complex buffers. This module is the app-facing
//! equivalent: a complex slice goes in, its spectrum comes out in place. The forward transform uses the sign
//! convention `X[k] = sum_n x[n] * e^(-2 pi i n k / N)`, and the inverse divides by `N` so that `ifft(fft(x)) == x`.
//! Lengths must be a power of two; an empty input is a no-op.

use std::f64::consts::PI;

/// A complex sample with `f64` parts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const fn new(re: f64, im: f64) -> Self {
        Complex { re, im }
    }

    /// A real sample with a zero imaginary part.
    pub const fn real(re: f64) -> Self {
        Complex { re, im: 0.0 }
    }

    /// Magnitude `sqrt(re^2 + im^2)`.
    pub fn magnitude(self) -> f64 {
        self.re.hypot(self.im)
    }

    fn mul(self, o: Complex) -> Complex {
        Complex::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}

/// Why a transform could not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FftError {
    /// The length is not a power of two.
    NotPowerOfTwo(usize),
}

impl std::fmt::Display for FftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FftError::NotPowerOfTwo(n) => write!(f, "an FFT needs a power-of-two length, got {n}"),
        }
    }
}

/// Forward FFT, in place.
pub fn fft(data: &mut [Complex]) -> Result<(), FftError> {
    transform(data, false)
}

/// Inverse FFT, in place, scaled by `1/N` so that it undoes [`fft`].
pub fn ifft(data: &mut [Complex]) -> Result<(), FftError> {
    transform(data, true)?;
    let n = data.len() as f64;
    if n > 0.0 {
        for v in data.iter_mut() {
            v.re /= n;
            v.im /= n;
        }
    }
    Ok(())
}

fn transform(data: &mut [Complex], inverse: bool) -> Result<(), FftError> {
    let n = data.len();
    if n <= 1 {
        return Ok(());
    }
    if !n.is_power_of_two() {
        return Err(FftError::NotPowerOfTwo(n));
    }
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            data.swap(i, j);
        }
    }
    // Iterative Cooley-Tukey butterflies over growing sub-transforms.
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let step = sign * 2.0 * PI / len as f64;
        let half = len / 2;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let angle = step * k as f64;
                let w = Complex::new(angle.cos(), angle.sin());
                let a = data[start + k];
                let b = data[start + k + half].mul(w);
                data[start + k] = Complex::new(a.re + b.re, a.im + b.im);
                data[start + k + half] = Complex::new(a.re - b.re, a.im - b.im);
            }
        }
        len <<= 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Complex, b: Complex) -> bool {
        (a.re - b.re).abs() < 1e-9 && (a.im - b.im).abs() < 1e-9
    }

    #[test]
    fn an_impulse_has_a_flat_spectrum() {
        let mut d = vec![Complex::default(); 8];
        d[0] = Complex::real(1.0);
        fft(&mut d).unwrap();
        assert!(d.iter().all(|v| close(*v, Complex::real(1.0))));
    }

    #[test]
    fn a_constant_has_only_a_dc_bin() {
        let mut d = vec![Complex::real(2.0); 4];
        fft(&mut d).unwrap();
        assert!(close(d[0], Complex::real(8.0)));
        assert!(d[1..].iter().all(|v| v.magnitude() < 1e-9));
    }

    #[test]
    fn a_sine_lands_in_its_bin_and_its_mirror() {
        let n = 16;
        let mut d: Vec<Complex> = (0..n).map(|i| Complex::real((2.0 * PI * 3.0 * i as f64 / n as f64).sin())).collect();
        fft(&mut d).unwrap();
        assert!((d[3].magnitude() - n as f64 / 2.0).abs() < 1e-9);
        assert!((d[n - 3].magnitude() - n as f64 / 2.0).abs() < 1e-9);
        assert!(d[5].magnitude() < 1e-9);
    }

    #[test]
    fn inverse_undoes_forward() {
        let original: Vec<Complex> = (0..32).map(|i| Complex::new(i as f64 * 0.5, (i % 5) as f64)).collect();
        let mut d = original.clone();
        fft(&mut d).unwrap();
        ifft(&mut d).unwrap();
        assert!(d.iter().zip(&original).all(|(a, b)| close(*a, *b)));
    }

    #[test]
    fn lengths_that_are_not_powers_of_two_are_refused() {
        let mut d = vec![Complex::real(1.0); 6];
        assert_eq!(fft(&mut d), Err(FftError::NotPowerOfTwo(6)));
        assert_eq!(ifft(&mut d), Err(FftError::NotPowerOfTwo(6)));
        assert_eq!(FftError::NotPowerOfTwo(6).to_string(), "an FFT needs a power-of-two length, got 6");
    }

    #[test]
    fn empty_and_single_inputs_are_left_alone() {
        let mut empty: Vec<Complex> = Vec::new();
        assert_eq!(fft(&mut empty), Ok(()));
        let mut one = vec![Complex::new(3.0, -1.0)];
        ifft(&mut one).unwrap();
        assert!(close(one[0], Complex::new(3.0, -1.0)));
    }
}
