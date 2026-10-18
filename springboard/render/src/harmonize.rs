//! Image harmonisation: makes a pasted-in foreground take on the colour mood of the scene it sits in, by statistical
//! colour transfer in CIE L*a*b* (Reinhard, Ashikhmin, Gooch and Shirley, "Color Transfer between Images"). The mean and
//! spread of the foreground's L, a and b are moved to the background's, with the spread ratio limited so a flat
//! foreground does not blow up. A soft mask says which pixels are foreground, and `strength` scales the effect.
//! Pure code (std only); the same per-channel affine map runs as a shader once the two sets of statistics are known.

use crate::colorfx::Rgb;
use crate::colorlab::Lab;

/// Weighted mean and standard deviation of L, a and b.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabStats {
    pub mean: [f64; 3],
    pub std: [f64; 3],
}

fn lab_array(c: Rgb) -> [f64; 3] {
    let l = Lab::from_rgb(c);
    [l.l, l.a, l.b]
}

/// Statistics of `pixels` weighted by `weights` (same length). `None` when the lengths differ or the total weight is
/// negligible.
pub fn lab_stats(pixels: &[Rgb], weights: &[f32]) -> Option<LabStats> {
    if pixels.len() != weights.len() {
        return None;
    }
    let total: f64 = weights.iter().map(|w| f64::from(w.max(0.0))).sum();
    if total < 1e-9 {
        return None;
    }
    let labs: Vec<[f64; 3]> = pixels.iter().map(|c| lab_array(*c)).collect();
    let mut mean = [0.0f64; 3];
    for (l, w) in labs.iter().zip(weights) {
        let w = f64::from(w.max(0.0));
        for c in 0..3 {
            mean[c] += w * l[c];
        }
    }
    for m in mean.iter_mut() {
        *m /= total;
    }
    let mut var = [0.0f64; 3];
    for (l, w) in labs.iter().zip(weights) {
        let w = f64::from(w.max(0.0));
        for c in 0..3 {
            var[c] += w * (l[c] - mean[c]) * (l[c] - mean[c]);
        }
    }
    Some(LabStats { mean, std: [(var[0] / total).sqrt(), (var[1] / total).sqrt(), (var[2] / total).sqrt()] })
}

/// Moves one colour from the `from` statistics to the `to` statistics: per channel `(v - mean_from) * gain + mean_to`
/// with `gain = std_to / std_from` kept within `1 / max_gain ..= max_gain` (`max_gain` below 1 is treated as 1).
pub fn transfer(c: Rgb, from: &LabStats, to: &LabStats, max_gain: f64) -> Rgb {
    let g = max_gain.max(1.0);
    let v = lab_array(c);
    let mut out = [0.0f64; 3];
    for i in 0..3 {
        let gain = (to.std[i] / from.std[i].max(1e-3)).clamp(1.0 / g, g);
        out[i] = (v[i] - from.mean[i]) * gain + to.mean[i];
    }
    Lab { l: out[0], a: out[1], b: out[2] }.to_rgb()
}

/// Harmonises the foreground of `image`. `mask` is 1 on the foreground, 0 on the background, soft in between; the
/// foreground statistics are weighted by the mask and the background ones by `1 - mask`. Each pixel moves toward its
/// transferred colour by `mask * strength`, so the background is untouched. `None` when the lengths differ. When
/// either region is empty the image is returned unchanged.
pub fn harmonize(image: &[Rgb], mask: &[f32], strength: f32) -> Option<Vec<Rgb>> {
    if image.len() != mask.len() {
        return None;
    }
    let fg: Vec<f32> = mask.iter().map(|m| m.clamp(0.0, 1.0)).collect();
    let bg: Vec<f32> = fg.iter().map(|m| 1.0 - m).collect();
    let (Some(from), Some(to)) = (lab_stats(image, &fg), lab_stats(image, &bg)) else {
        return Some(image.to_vec());
    };
    let s = strength.clamp(0.0, 1.0);
    Some(
        image
            .iter()
            .zip(&fg)
            .map(|(c, m)| {
                let t = m * s;
                if t <= 0.0 {
                    *c
                } else {
                    c.mix(transfer(*c, &from, &to, 2.0), t)
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> (Vec<Rgb>, Vec<f32>) {
        let mut img = Vec::new();
        let mut mask = Vec::new();
        for i in 0..8 {
            let d = 0.01 * i as f32;
            img.push(Rgb::new(0.25 + d, 0.23 + d, 0.21 + d));
            mask.push(1.0);
        }
        for i in 0..8 {
            let d = 0.01 * i as f32;
            img.push(Rgb::new(0.6 + d, 0.65 + d, 0.7 + d));
            mask.push(0.0);
        }
        (img, mask)
    }

    fn mean_l(px: &[Rgb]) -> f64 {
        px.iter().map(|c| Lab::from_rgb(*c).l).sum::<f64>() / px.len() as f64
    }

    #[test]
    fn stats_of_a_constant_colour_have_no_spread() {
        let px = vec![Rgb::new(0.5, 0.5, 0.5); 4];
        let s = lab_stats(&px, &[1.0; 4]).unwrap();
        assert!(s.std.iter().all(|v| v.abs() < 1e-9));
        assert!((s.mean[0] - Lab::from_rgb(px[0]).l).abs() < 1e-9);
        assert!(lab_stats(&px, &[0.0; 4]).is_none());
        assert!(lab_stats(&px, &[1.0; 3]).is_none());
    }

    #[test]
    fn weights_select_the_pixels() {
        let px = vec![Rgb::new(0.1, 0.1, 0.1), Rgb::new(0.9, 0.9, 0.9)];
        let only_first = lab_stats(&px, &[1.0, 0.0]).unwrap();
        assert!((only_first.mean[0] - Lab::from_rgb(px[0]).l).abs() < 1e-9);
    }

    #[test]
    fn the_foreground_takes_on_the_background_brightness() {
        let (img, mask) = scene();
        let before = mean_l(&img[..8]);
        let bg_l = mean_l(&img[8..]);
        let out = harmonize(&img, &mask, 1.0).unwrap();
        let after = mean_l(&out[..8]);
        assert!((after - bg_l).abs() < 3.0, "{after} vs {bg_l}");
        assert!((after - bg_l).abs() < (before - bg_l).abs());
        // The background does not change at all.
        assert_eq!(&out[8..], &img[8..]);
    }

    #[test]
    fn strength_scales_the_effect() {
        let (img, mask) = scene();
        assert_eq!(harmonize(&img, &mask, 0.0).unwrap(), img);
        let half = harmonize(&img, &mask, 0.5).unwrap();
        let full = harmonize(&img, &mask, 1.0).unwrap();
        let l0 = mean_l(&img[..8]);
        let lh = mean_l(&half[..8]);
        let lf = mean_l(&full[..8]);
        assert!(lh > l0 && lh < lf, "{l0} {lh} {lf}");
    }

    #[test]
    fn degenerate_inputs() {
        let (img, mask) = scene();
        assert!(harmonize(&img, &mask[..4], 1.0).is_none());
        let all_fg = vec![1.0; img.len()];
        assert_eq!(harmonize(&img, &all_fg, 1.0).unwrap(), img);
        let all_bg = vec![0.0; img.len()];
        assert_eq!(harmonize(&img, &all_bg, 1.0).unwrap(), img);
    }

    #[test]
    fn transfer_to_the_same_statistics_is_the_identity() {
        let (img, _) = scene();
        let s = lab_stats(&img, &vec![1.0; img.len()]).unwrap();
        for c in &img[..4] {
            let t = transfer(*c, &s, &s, 2.0);
            assert!((t.r - c.r).abs() < 1e-3 && (t.g - c.g).abs() < 1e-3 && (t.b - c.b).abs() < 1e-3);
        }
    }
}
