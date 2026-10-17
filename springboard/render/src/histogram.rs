//! Tone and brightness tools on 8-bit samples: histograms, histogram equalization and specification, level stretching,
//! gamma and piecewise linear tone curves. Each tool is a pure function of the samples or of a 256-entry lookup table,
//! so a tone change costs one table build and one pass over the pixels.

/// Counts how many samples take each of the 256 values.
pub fn histogram(samples: &[u8]) -> [u32; 256] {
    let mut counts = [0u32; 256];
    for &s in samples {
        counts[s as usize] += 1;
    }
    counts
}

/// The identity lookup table: every value maps to itself.
pub fn identity_lut() -> [u8; 256] {
    core::array::from_fn(|i| i as u8)
}

/// Applies a lookup table to every sample.
pub fn apply_lut(samples: &[u8], lut: &[u8; 256]) -> Vec<u8> {
    samples.iter().map(|&s| lut[s as usize]).collect()
}

/// Builds the histogram equalization table: each value maps to its cumulative share of the samples, spread over
/// 0..=255. An empty or single-valued histogram has nothing to spread, so it maps to the identity.
pub fn equalization_lut(hist: &[u32; 256]) -> [u8; 256] {
    let total: u64 = hist.iter().map(|&c| c as u64).sum();
    let first_nonzero = hist.iter().copied().find(|&c| c > 0).unwrap_or(0) as u64;
    if total == 0 || total == first_nonzero {
        return identity_lut();
    }
    let span = total - first_nonzero;
    let mut cumulative = 0u64;
    core::array::from_fn(|i| {
        cumulative += hist[i] as u64;
        let below_min = cumulative.saturating_sub(first_nonzero);
        ((below_min * 255 + span / 2) / span) as u8
    })
}

/// Histogram equalization of a sample set.
pub fn equalize(samples: &[u8]) -> Vec<u8> {
    apply_lut(samples, &equalization_lut(&histogram(samples)))
}

/// A linear level stretch: `black` maps to 0, `white` maps to 255 and values between them are scaled and rounded.
/// Values outside the range clip. Returns None when `white` is not above `black`.
pub fn levels_lut(black: u8, white: u8) -> Option<[u8; 256]> {
    if white <= black {
        return None;
    }
    let span = (white - black) as f64;
    Some(core::array::from_fn(|i| {
        let v = (i as f64 - black as f64) * 255.0 / span;
        v.round().clamp(0.0, 255.0) as u8
    }))
}

/// A piecewise linear tone curve through `points` as `(input, output)` pairs. The inputs must be strictly
/// increasing. Values left of the first point take its output, values right of the last point take its output, and
/// values between two points are interpolated and rounded. Returns None when there are no points or the inputs
/// are not strictly increasing.
pub fn curve_lut(points: &[(u8, u8)]) -> Option<[u8; 256]> {
    if points.is_empty() || points.windows(2).any(|w| w[0].0 >= w[1].0) {
        return None;
    }
    Some(core::array::from_fn(|i| curve_value(points, i as u32)))
}

fn curve_value(points: &[(u8, u8)], x: u32) -> u8 {
    let (first, last) = (points[0], points[points.len() - 1]);
    if x <= first.0 as u32 {
        return first.1;
    }
    if x >= last.0 as u32 {
        return last.1;
    }
    for w in points.windows(2) {
        let (x0, y0) = (w[0].0 as u32, w[0].1 as f64);
        let (x1, y1) = (w[1].0 as u32, w[1].1 as f64);
        if x <= x1 {
            let t = (x - x0) as f64 / (x1 - x0) as f64;
            return (y0 + (y1 - y0) * t).round() as u8;
        }
    }
    last.1
}

/// A gamma table: each value `v` maps to `255 * (v / 255)^(1 / gamma)`, rounded. A gamma of 1 is the identity and a
/// gamma above 1 brightens the mid tones. Returns None when `gamma` is not a positive finite number.
pub fn gamma_lut(gamma: f64) -> Option<[u8; 256]> {
    if !gamma.is_finite() || gamma <= 0.0 {
        return None;
    }
    let exponent = 1.0 / gamma;
    Some(core::array::from_fn(|i| {
        let v = (i as f64 / 255.0).powf(exponent) * 255.0;
        v.round().clamp(0.0, 255.0) as u8
    }))
}

/// Histogram specification: a table that reshapes the `source` histogram into the `target` one. Each source value maps
/// to the smallest target value whose cumulative share reaches the source value's cumulative share, so the output
/// follows the target's tones. An empty histogram on either side has nothing to match, so it maps to the identity.
pub fn specification_lut(source: &[u32; 256], target: &[u32; 256]) -> [u8; 256] {
    let s_total: u64 = source.iter().map(|&c| c as u64).sum();
    let t_total: u64 = target.iter().map(|&c| c as u64).sum();
    if s_total == 0 || t_total == 0 {
        return identity_lut();
    }
    let mut t_cdf = [0u64; 256];
    let mut acc = 0u64;
    for (j, &c) in target.iter().enumerate() {
        acc += c as u64;
        t_cdf[j] = acc;
    }
    let mut s_acc = 0u64;
    core::array::from_fn(|i| {
        s_acc += source[i] as u64;
        // Smallest j with t_cdf[j] / t_total >= s_acc / s_total, compared without division.
        let need = s_acc as u128 * t_total as u128;
        let j = t_cdf.iter().position(|&c| c as u128 * s_total as u128 >= need).unwrap_or(255);
        j as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_counts_each_value() {
        let h = histogram(&[0, 0, 255, 7]);
        assert_eq!(h[0], 2);
        assert_eq!(h[7], 1);
        assert_eq!(h[255], 1);
        assert_eq!(h.iter().sum::<u32>(), 4);
    }

    #[test]
    fn equalization_spreads_two_levels_over_the_full_range() {
        let lut = equalization_lut(&histogram(&[10, 10, 200, 200]));
        assert_eq!(lut[10], 0, "the darkest occupied value maps to black");
        assert_eq!(lut[200], 255, "the brightest occupied value maps to white");
        assert_eq!(equalize(&[10, 10, 200, 200]), vec![0, 0, 255, 255]);
    }

    #[test]
    fn equalization_leaves_a_flat_or_empty_image_alone() {
        assert_eq!(equalize(&[128, 128, 128]), vec![128, 128, 128]);
        assert_eq!(equalization_lut(&[0u32; 256]), identity_lut());
    }

    #[test]
    fn levels_stretch_maps_the_black_and_white_points_to_the_ends() {
        let lut = levels_lut(50, 200).expect("white is above black");
        assert_eq!(lut[50], 0);
        assert_eq!(lut[200], 255);
        assert_eq!(lut[0], 0, "below black clips");
        assert_eq!(lut[255], 255, "above white clips");
        assert_eq!(lut[125], 128, "75 * 255 / 150 is 127.5, which rounds up");
    }

    #[test]
    fn levels_refuse_an_empty_range() {
        assert!(levels_lut(9, 9).is_none());
        assert!(levels_lut(200, 50).is_none());
    }

    #[test]
    fn tone_curve_interpolates_between_its_points() {
        let lut = curve_lut(&[(0, 0), (128, 64), (255, 255)]).expect("sorted points");
        assert_eq!(lut[0], 0);
        assert_eq!(lut[64], 32);
        assert_eq!(lut[128], 64);
        assert_eq!(lut[255], 255);
    }

    #[test]
    fn tone_curve_holds_its_end_values_outside_the_points() {
        let lut = curve_lut(&[(64, 100), (192, 200)]).expect("two points");
        assert_eq!(lut[0], 100);
        assert_eq!(lut[255], 200);
        assert_eq!(lut[128], 150);
    }

    #[test]
    fn tone_curve_refuses_unsorted_or_empty_points() {
        assert!(curve_lut(&[]).is_none());
        assert!(curve_lut(&[(10, 0), (10, 5)]).is_none(), "equal inputs have no slope");
        assert!(curve_lut(&[(90, 0), (10, 5)]).is_none());
    }

    #[test]
    fn gamma_table_brightens_mid_tones_and_refuses_bad_gamma() {
        let lut = gamma_lut(2.0).expect("a positive gamma");
        assert_eq!((lut[0], lut[255]), (0, 255));
        assert_eq!(lut[64], 128, "255 * sqrt(64 / 255) is 127.75");
        assert_eq!(gamma_lut(1.0), Some(identity_lut()));
        assert!(gamma_lut(0.0).is_none());
        assert!(gamma_lut(f64::NAN).is_none());
    }

    #[test]
    fn specification_moves_the_source_tones_onto_the_target() {
        let source = histogram(&[0, 0, 255, 255]);
        let target = histogram(&[10, 20, 30, 40]);
        let lut = specification_lut(&source, &target);
        assert_eq!(lut[0], 20, "half the source sits at or below the target's second value");
        assert_eq!(lut[255], 40, "the top of the source reaches the top of the target");
        assert_eq!(specification_lut(&source, &[0u32; 256]), identity_lut());
    }

    #[test]
    fn a_lookup_table_maps_every_sample() {
        let mut lut = identity_lut();
        lut[1] = 9;
        assert_eq!(apply_lut(&[0, 1, 2], &lut), vec![0, 9, 2]);
    }
}
