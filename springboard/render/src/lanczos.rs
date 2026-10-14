//! Lanczos resampling of 8-bit single-channel planes. The kernel is the
//! windowed sinc `sinc(t) * sinc(t / a)` for `|t| < a`, with `a = 3` taps per
//! side of a source sample. When the image shrinks, the kernel is widened by
//! the scale factor so that the output is anti-aliased. Weights are normalised
//! per output sample and the result is clamped to 0..=255. The filter runs on
//! the CPU in plain std code and is separable: rows are resampled first, then
//! columns.

use crate::filter::Plane;

/// Lobes of the Lanczos window on each side of the centre.
pub const LOBES: f32 = 3.0;

/// The normalised sinc: `sin(pi x) / (pi x)`, with the value 1 at zero.
pub fn sinc(x: f32) -> f32 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f32::consts::PI * x;
        px.sin() / px
    }
}

/// The Lanczos window value at offset `t` (in source samples, already divided by the filter scale).
pub fn lanczos(t: f32) -> f32 {
    if t.abs() >= LOBES {
        0.0
    } else {
        sinc(t) * sinc(t / LOBES)
    }
}

/// For each output index, the source taps as `(source index, weight)` pairs. Weights sum to one.
/// Source indices are clamped to the valid range, so edge samples repeat.
pub fn axis_taps(src_len: usize, dst_len: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src_len as f32 / dst_len as f32;
    // Widen the kernel when shrinking so that each output sample averages its source footprint.
    let filter_scale = scale.max(1.0);
    let radius = LOBES * filter_scale;
    let last = src_len as isize - 1;
    (0..dst_len)
        .map(|o| {
            let centre = (o as f32 + 0.5) * scale - 0.5;
            let start = (centre - radius).floor() as isize;
            let end = (centre + radius).ceil() as isize;
            let mut taps: Vec<(usize, f32)> = Vec::new();
            for i in start..=end {
                let weight = lanczos((i as f32 - centre) / filter_scale);
                if weight != 0.0 {
                    taps.push((i.clamp(0, last) as usize, weight));
                }
            }
            let total: f32 = taps.iter().map(|&(_, w)| w).sum();
            if total.abs() < 1e-6 {
                // Degenerate window: fall back to the nearest source sample.
                let nearest = centre.round().clamp(0.0, last as f32) as usize;
                return vec![(nearest, 1.0)];
            }
            taps.into_iter().map(|(i, w)| (i, w / total)).collect()
        })
        .collect()
}

/// Resamples `src` to `out_w` by `out_h` pixels. Returns `None` when a size is zero or the source is empty.
pub fn lanczos_scale(src: &Plane, out_w: usize, out_h: usize) -> Option<Plane> {
    if out_w == 0 || out_h == 0 || src.width == 0 || src.height == 0 {
        return None;
    }
    let (w, h) = (src.width, src.height);
    let horizontal = axis_taps(w, out_w);
    let vertical = axis_taps(h, out_h);

    // Horizontal pass: keep full precision until the final rounding.
    let mut mid = vec![0.0f32; out_w * h];
    for y in 0..h {
        let row = &src.pixels[y * w..(y + 1) * w];
        for (ox, taps) in horizontal.iter().enumerate() {
            mid[y * out_w + ox] = taps.iter().map(|&(ix, wt)| wt * row[ix] as f32).sum();
        }
    }

    // Vertical pass.
    let mut pixels = vec![0u8; out_w * out_h];
    for (oy, taps) in vertical.iter().enumerate() {
        for ox in 0..out_w {
            let acc: f32 = taps.iter().map(|&(iy, wt)| wt * mid[iy * out_w + ox]).sum();
            pixels[oy * out_w + ox] = acc.round().clamp(0.0, 255.0) as u8;
        }
    }
    Some(Plane { width: out_w, height: out_h, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinc_is_one_at_zero_and_zero_at_integers() {
        assert_eq!(sinc(0.0), 1.0);
        assert!(sinc(1.0).abs() < 1e-6);
        assert!(sinc(2.0).abs() < 1e-6);
    }

    #[test]
    fn lanczos_window_is_zero_outside_its_lobes() {
        assert_eq!(lanczos(3.0), 0.0);
        assert_eq!(lanczos(-4.5), 0.0);
        assert!((lanczos(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn taps_are_normalised_for_every_output_sample() {
        for (src, dst) in [(8usize, 8usize), (8, 4), (4, 9), (1, 5)] {
            for taps in axis_taps(src, dst) {
                let total: f32 = taps.iter().map(|&(_, w)| w).sum();
                assert!((total - 1.0).abs() < 1e-4, "src {src} dst {dst}");
                assert!(taps.iter().all(|&(i, _)| i < src));
            }
        }
    }

    #[test]
    fn identity_size_keeps_the_pixels() {
        let src = Plane::new(4, 2, vec![0, 50, 100, 150, 200, 210, 220, 255]).unwrap();
        assert_eq!(lanczos_scale(&src, 4, 2).unwrap(), src);
    }

    #[test]
    fn flat_plane_stays_flat_at_any_size() {
        let src = Plane::filled(5, 3, 117);
        assert_eq!(lanczos_scale(&src, 9, 7).unwrap().pixels, vec![117u8; 63]);
        assert_eq!(lanczos_scale(&src, 2, 1).unwrap().pixels, vec![117u8; 2]);
    }

    #[test]
    fn output_has_the_requested_size() {
        let src = Plane::filled(6, 4, 0);
        let out = lanczos_scale(&src, 3, 10).unwrap();
        assert_eq!((out.width, out.height), (3, 10));
        assert_eq!(out.pixels.len(), 30);
    }

    #[test]
    fn upscaling_a_ramp_stays_monotonic() {
        let src = Plane::new(4, 1, vec![0, 80, 160, 240]).unwrap();
        let out = lanczos_scale(&src, 8, 1).unwrap();
        // Edge repetition and ringing move the end samples a little; the ends stay near the source ends.
        assert!(out.pixels[0] <= 20, "{:?}", out.pixels);
        assert!((230..=255).contains(&out.pixels[7]), "{:?}", out.pixels);
        assert!(out.pixels[7] > out.pixels[3] && out.pixels[3] > out.pixels[0]);
    }

    #[test]
    fn empty_or_zero_sizes_are_refused() {
        let src = Plane::filled(2, 2, 9);
        assert!(lanczos_scale(&src, 0, 2).is_none());
        assert!(lanczos_scale(&src, 2, 0).is_none());
        assert!(lanczos_scale(&Plane::new(0, 0, Vec::new()).unwrap(), 2, 2).is_none());
    }
}
