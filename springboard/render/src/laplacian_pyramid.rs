//! Gaussian and Laplacian pyramids over 8-bit planes. A Gaussian pyramid blurs each level and keeps every second pixel,
//! so each level is half the size of the one before (rounded up). A Laplacian level is the difference between a
//! Gaussian level and the next level expanded back to its size; the last level keeps the coarsest Gaussian values.
//! Collapsing the Laplacian levels rebuilds the base plane exactly. Pure code: no GPU and no shell state.

use crate::filter::Plane;
use crate::gaussian_filter::gaussian_blur;

/// Sigma of the blur applied before each decimation step (the classic 5-tap pyramid kernel).
pub const REDUCE_SIGMA: f32 = 1.0;

/// One Laplacian level. Values are signed differences; the last level holds the coarsest Gaussian values unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaplacianLevel {
    pub width: usize,
    pub height: usize,
    pub values: Vec<i16>,
}

/// Blurs `src` and keeps every second pixel in each direction. The result is `ceil(width / 2)` by `ceil(height / 2)`.
pub fn reduce(src: &Plane) -> Plane {
    let blurred = gaussian_blur(src, REDUCE_SIGMA).expect("the reduce sigma is in range");
    let (w, h) = (src.width.div_ceil(2), src.height.div_ceil(2));
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(blurred.pixels[(y * 2) * src.width + x * 2]);
        }
    }
    Plane { width: w, height: h, pixels }
}

/// Stretches `src` to `width` by `height` with bilinear interpolation. A source with no pixels expands to zeros.
pub fn expand(src: &Plane, width: usize, height: usize) -> Plane {
    if src.width == 0 || src.height == 0 {
        return Plane::filled(width, height, 0);
    }
    let mut pixels = vec![0u8; width * height];
    for y in 0..height {
        let (y0, y1, ty) = axis_taps(y, height, src.height);
        for x in 0..width {
            let (x0, x1, tx) = axis_taps(x, width, src.width);
            let at = |sx: usize, sy: usize| src.pixels[sy * src.width + sx] as f32;
            let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
            let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
            let value = top + (bottom - top) * ty;
            pixels[y * width + x] = value.round().clamp(0.0, 255.0) as u8;
        }
    }
    Plane { width, height, pixels }
}

/// The two source indices and the weight of the second one for destination index `i`, sampled at pixel centres.
fn axis_taps(i: usize, dst_len: usize, src_len: usize) -> (usize, usize, f32) {
    let pos = ((i as f32 + 0.5) * src_len as f32 / dst_len as f32 - 0.5).clamp(0.0, (src_len - 1) as f32);
    let low = pos.floor() as usize;
    let high = (low + 1).min(src_len - 1);
    (low, high, pos - low as f32)
}

/// The Gaussian pyramid of `src`: `levels` planes, the first being `src` itself. Zero levels give an empty list.
pub fn gaussian_pyramid(src: &Plane, levels: usize) -> Vec<Plane> {
    let mut out = Vec::with_capacity(levels);
    if levels == 0 {
        return out;
    }
    out.push(src.clone());
    while out.len() < levels {
        let next = reduce(out.last().expect("the list holds at least one level"));
        out.push(next);
    }
    out
}

/// The Laplacian pyramid of `src` with `levels` levels. Empty for zero levels.
pub fn laplacian_pyramid(src: &Plane, levels: usize) -> Vec<LaplacianLevel> {
    let gaussian = gaussian_pyramid(src, levels);
    let mut out = Vec::with_capacity(gaussian.len());
    for (i, level) in gaussian.iter().enumerate() {
        let values: Vec<i16> = match gaussian.get(i + 1) {
            Some(coarser) => {
                let expanded = expand(coarser, level.width, level.height);
                level
                    .pixels
                    .iter()
                    .zip(&expanded.pixels)
                    .map(|(&fine, &coarse)| fine as i16 - coarse as i16)
                    .collect()
            }
            None => level.pixels.iter().map(|&v| v as i16).collect(),
        };
        out.push(LaplacianLevel { width: level.width, height: level.height, values });
    }
    out
}

/// Rebuilds the base plane from a Laplacian pyramid. None for an empty pyramid or for a level whose value count does not
/// match its size. For a pyramid made by [`laplacian_pyramid`] the result equals the original plane.
pub fn collapse(levels: &[LaplacianLevel]) -> Option<Plane> {
    let (last, finer) = levels.split_last()?;
    if last.width.checked_mul(last.height)? != last.values.len() {
        return None;
    }
    let mut current = Plane { width: last.width, height: last.height, pixels: to_bytes(&last.values)? };
    for level in finer.iter().rev() {
        if level.width.checked_mul(level.height)? != level.values.len() {
            return None;
        }
        let expanded = expand(&current, level.width, level.height);
        let pixels = level
            .values
            .iter()
            .zip(&expanded.pixels)
            .map(|(&diff, &base)| (diff + base as i16).clamp(0, 255) as u8)
            .collect::<Vec<u8>>();
        current = Plane { width: level.width, height: level.height, pixels };
    }
    Some(current)
}

/// The coarsest level's values as bytes. None when a value is outside 0 to 255, which a Gaussian level never produces.
fn to_bytes(values: &[i16]) -> Option<Vec<u8>> {
    values.iter().map(|&v| u8::try_from(v).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reduced_level_is_half_the_size_rounded_up() {
        let src = Plane::filled(9, 5, 40);
        let levels = gaussian_pyramid(&src, 3);
        let sizes: Vec<(usize, usize)> = levels.iter().map(|p| (p.width, p.height)).collect();
        assert_eq!(sizes, vec![(9, 5), (5, 3), (3, 2)]);
    }

    #[test]
    fn zero_levels_give_empty_pyramids() {
        let src = Plane::filled(4, 4, 1);
        assert!(gaussian_pyramid(&src, 0).is_empty());
        assert!(laplacian_pyramid(&src, 0).is_empty());
        assert_eq!(collapse(&[]), None);
    }

    #[test]
    fn a_flat_plane_has_zero_laplacian_levels_except_the_coarsest() {
        let src = Plane::filled(8, 8, 100);
        let lap = laplacian_pyramid(&src, 3);
        assert!(lap[0].values.iter().all(|&v| v == 0));
        assert!(lap[1].values.iter().all(|&v| v == 0));
        assert_eq!(lap[2].values, vec![100i16; 4], "the coarsest level keeps the Gaussian values");
    }

    #[test]
    fn collapsing_the_laplacian_pyramid_restores_the_base_plane() {
        let mut src = Plane::filled(12, 7, 0);
        for (i, p) in src.pixels.iter_mut().enumerate() {
            *p = ((i * 37) % 256) as u8;
        }
        let lap = laplacian_pyramid(&src, 3);
        assert_eq!(collapse(&lap), Some(src));
    }

    #[test]
    fn laplacian_levels_hold_signed_differences_across_an_edge() {
        let mut src = Plane::filled(8, 1, 0);
        for x in 4..8 {
            src.pixels[x] = 200;
        }
        let lap = laplacian_pyramid(&src, 2);
        assert!(lap[0].values.iter().any(|&v| v < 0), "the dark side of the edge is negative");
        assert!(lap[0].values.iter().any(|&v| v > 0), "the bright side of the edge is positive");
        assert!(lap[0].values.iter().all(|&v| (-255..=255).contains(&v)));
    }

    #[test]
    fn expand_interpolates_between_source_pixels() {
        let src = Plane::new(2, 1, vec![0, 255]).unwrap();
        let out = expand(&src, 4, 1);
        assert_eq!(out.pixels, vec![0, 64, 191, 255]);
    }

    #[test]
    fn expand_of_an_empty_plane_is_zero() {
        let src = Plane::new(0, 0, Vec::new()).unwrap();
        assert_eq!(expand(&src, 2, 2).pixels, vec![0u8; 4]);
    }

    #[test]
    fn reduce_keeps_the_even_sample_positions_of_the_blur() {
        let src = Plane::filled(5, 3, 77);
        let out = reduce(&src);
        assert_eq!((out.width, out.height), (3, 2));
        assert_eq!(out.pixels, vec![77u8; 6]);
    }

    #[test]
    fn collapse_refuses_a_level_whose_size_does_not_match_its_neighbour() {
        let levels = vec![
            LaplacianLevel { width: 4, height: 4, values: vec![0; 16] },
            LaplacianLevel { width: 2, height: 2, values: vec![0; 3] },
        ];
        assert_eq!(collapse(&levels), None);
    }
}
