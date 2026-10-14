//! Separable blur filters on 8-bit single-channel planes: a Gaussian with a
//! caller sigma, a box filter and a tent filter. Each is a one-dimensional
//! kernel applied along rows and then along columns. Pixels outside the plane
//! repeat the nearest edge pixel. The code is plain std and runs without a GPU;
//! the shell's glass blur uses its own dual-Kawase pyramid and does not call it.

use crate::filter::Plane;

/// Largest sigma accepted, so the kernel stays a few hundred taps at most.
pub const MAX_SIGMA: f32 = 64.0;

/// Normalised Gaussian weights for `sigma`, covering three sigmas on each side.
/// Returns `None` unless `0 < sigma <= MAX_SIGMA`.
pub fn gaussian_weights(sigma: f32) -> Option<Vec<f32>> {
    if !(sigma > 0.0 && sigma <= MAX_SIGMA) {
        return None;
    }
    let radius = (3.0 * sigma).ceil() as i32;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    Some(normalise(weights))
}

/// Box weights for a window of `2 * radius + 1` taps, each `1 / taps`.
pub fn box_weights(radius: usize) -> Vec<f32> {
    normalise(vec![1.0; 2 * radius + 1])
}

/// Tent weights for `2 * radius + 1` taps, rising linearly to the centre.
pub fn tent_weights(radius: usize) -> Vec<f32> {
    let r = radius as i64;
    let weights: Vec<f32> = (-r..=r).map(|i| (r + 1 - i.abs()) as f32).collect();
    normalise(weights)
}

fn normalise(weights: Vec<f32>) -> Vec<f32> {
    let total: f32 = weights.iter().sum();
    weights.into_iter().map(|w| w / total).collect()
}

/// Blurs `src` with a Gaussian of the given sigma. `None` when sigma is out of range.
pub fn gaussian_blur(src: &Plane, sigma: f32) -> Option<Plane> {
    let kernel = gaussian_weights(sigma)?;
    Some(separable(src, &kernel))
}

/// Blurs `src` with a box of `2 * radius + 1` taps per side.
pub fn box_blur(src: &Plane, radius: usize) -> Plane {
    separable(src, &box_weights(radius))
}

/// Blurs `src` with a tent of `2 * radius + 1` taps per side.
pub fn tent_blur(src: &Plane, radius: usize) -> Plane {
    separable(src, &tent_weights(radius))
}

/// Applies `kernel` along rows, then along columns. The kernel has odd length.
fn separable(src: &Plane, kernel: &[f32]) -> Plane {
    let rows = pass(src, kernel, true);
    pass(&rows, kernel, false)
}

fn pass(src: &Plane, kernel: &[f32], horizontal: bool) -> Plane {
    let (w, h) = (src.width, src.height);
    if w == 0 || h == 0 {
        return src.clone();
    }
    let radius = (kernel.len() / 2) as isize;
    let mut pixels = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (k, &weight) in kernel.iter().enumerate() {
                let d = k as isize - radius;
                let (sx, sy) = if horizontal {
                    (x as isize + d, y as isize)
                } else {
                    (x as isize, y as isize + d)
                };
                let cx = sx.clamp(0, w as isize - 1) as usize;
                let cy = sy.clamp(0, h as isize - 1) as usize;
                acc += weight * src.pixels[cy * w + cx] as f32;
            }
            pixels[y * w + x] = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
    Plane { width: w, height: h, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[u8]) -> Plane {
        Plane::new(values.len(), 1, values.to_vec()).unwrap()
    }

    #[test]
    fn gaussian_weights_are_normalised_and_symmetric() {
        let w = gaussian_weights(1.0).unwrap();
        assert_eq!(w.len(), 7);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        for i in 0..w.len() / 2 {
            assert!((w[i] - w[w.len() - 1 - i]).abs() < 1e-7);
        }
        assert!(w[3] > w[2] && w[2] > w[1]);
    }

    #[test]
    fn gaussian_rejects_out_of_range_sigma() {
        assert!(gaussian_weights(0.0).is_none());
        assert!(gaussian_weights(-1.0).is_none());
        assert!(gaussian_weights(f32::NAN).is_none());
        assert!(gaussian_weights(MAX_SIGMA + 1.0).is_none());
        assert!(gaussian_blur(&Plane::filled(2, 2, 1), 0.0).is_none());
    }

    #[test]
    fn blur_of_a_flat_plane_is_unchanged() {
        let src = Plane::filled(6, 4, 90);
        assert_eq!(gaussian_blur(&src, 2.0).unwrap().pixels, vec![90u8; 24]);
        assert_eq!(box_blur(&src, 2).pixels, vec![90u8; 24]);
        assert_eq!(tent_blur(&src, 2).pixels, vec![90u8; 24]);
    }

    #[test]
    fn gaussian_spreads_an_impulse_symmetrically() {
        let mut src = Plane::filled(9, 9, 0);
        src.pixels[4 * 9 + 4] = 255;
        let out = gaussian_blur(&src, 1.0).unwrap();
        let at = |x: usize, y: usize| out.pixels[y * 9 + x];
        assert!(at(4, 4) > at(3, 4));
        assert!(at(3, 4) > at(2, 4));
        assert_eq!(at(3, 4), at(5, 4));
        assert_eq!(at(4, 3), at(4, 5));
        assert_eq!(at(0, 0), 0);
    }

    #[test]
    fn box_blur_averages_three_taps() {
        let src = row(&[0, 0, 255, 0, 0]);
        assert_eq!(box_blur(&src, 1).pixels, vec![0, 85, 85, 85, 0]);
    }

    #[test]
    fn tent_blur_uses_one_two_one_weights() {
        let src = row(&[0, 0, 255, 0, 0]);
        assert_eq!(tent_blur(&src, 1).pixels, vec![0, 64, 128, 64, 0]);
    }

    #[test]
    fn radius_zero_is_the_identity() {
        let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(box_blur(&src, 0), src);
        assert_eq!(tent_blur(&src, 0), src);
    }

    #[test]
    fn empty_plane_is_returned_as_is() {
        let src = Plane::new(0, 0, Vec::new()).unwrap();
        assert_eq!(box_blur(&src, 2), src);
    }
}
