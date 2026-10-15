//! ColorSync rendering intents and black point compensation: how an out-of-gamut colour is mapped when converting
//! between spaces. Perceptual compresses the whole source gamut smoothly, relative colorimetric clips to the
//! destination gamut, saturation keeps chroma, and absolute keeps the white point. Black point compensation scales
//! luminance so the source black lands on the destination black. Operates on linear RGB triples in 0..1.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn clip(c: [f32; 3]) -> [f32; 3] {
    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
}

pub fn in_gamut(c: [f32; 3]) -> bool {
    c.iter().all(|v| (0.0..=1.0).contains(v))
}

/// Pulls an out-of-gamut colour toward its own luminance (grey) until every channel fits, keeping hue.
pub fn compress_to_gamut(c: [f32; 3]) -> [f32; 3] {
    if in_gamut(c) {
        return c;
    }
    let y = luma(c).clamp(0.0, 1.0);
    // find the largest t in 0..=1 so that grey + t*(c - grey) is in gamut
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let t = (lo + hi) / 2.0;
        let m = [y + t * (c[0] - y), y + t * (c[1] - y), y + t * (c[2] - y)];
        if in_gamut(m) {
            lo = t;
        } else {
            hi = t;
        }
    }
    [y + lo * (c[0] - y), y + lo * (c[1] - y), y + lo * (c[2] - y)]
}

/// Maps `c` (already expressed in the destination space, possibly out of its gamut) according to the intent.
/// `white_scale` is destination white over source white per channel, used only by absolute colorimetric.
pub fn apply(c: [f32; 3], intent: Intent, white_scale: [f32; 3]) -> [f32; 3] {
    match intent {
        Intent::RelativeColorimetric => clip(c),
        Intent::Perceptual => {
            // smooth compression: first desaturate what does not fit, then clip rounding noise
            clip(compress_to_gamut(c))
        }
        Intent::Saturation => {
            // keep the colour as saturated as possible: scale the largest channel to 1 when out of gamut by excess
            let max = c.iter().cloned().fold(f32::MIN, f32::max);
            if max > 1.0 { clip([c[0] / max, c[1] / max, c[2] / max]) } else { clip(c) }
        }
        Intent::AbsoluteColorimetric => clip([c[0] * white_scale[0], c[1] * white_scale[1], c[2] * white_scale[2]]),
    }
}

/// Black point compensation: scales luminance so the source black level `src_black` maps to `dst_black`.
pub fn compensate_black(c: [f32; 3], src_black: f32, dst_black: f32) -> [f32; 3] {
    if (1.0 - src_black).abs() < 1e-6 {
        return c;
    }
    let scale = (1.0 - dst_black) / (1.0 - src_black);
    let offset = (dst_black - src_black) / (1.0 - src_black);
    [c[0] * scale + offset, c[1] * scale + offset, c[2] * scale + offset]
}

/// Convert in one step: compensate black if asked, then map by intent.
pub fn convert(c: [f32; 3], intent: Intent, bpc: bool, src_black: f32, dst_black: f32, white_scale: [f32; 3]) -> [f32; 3] {
    let c = if bpc { compensate_black(c, src_black, dst_black) } else { c };
    apply(c, intent, white_scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: [f32; 3] = [1.0, 1.0, 1.0];

    #[test]
    fn in_gamut_colours_are_untouched() {
        let c = [0.2, 0.5, 0.9];
        for i in [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation] {
            assert_eq!(apply(c, i, ONE), c);
        }
    }

    #[test]
    fn relative_clips_and_perceptual_keeps_hue() {
        let c = [1.3, 0.4, -0.1];
        assert_eq!(apply(c, Intent::RelativeColorimetric, ONE), [1.0, 0.4, 0.0]);
        let p = apply(c, Intent::Perceptual, ONE);
        assert!(in_gamut(p));
        // the ordering of channels (the hue) is kept
        assert!(p[0] > p[1] && p[1] > p[2]);
        // and the result differs from a hard clip
        assert_ne!(p, apply(c, Intent::RelativeColorimetric, ONE));
    }

    #[test]
    fn saturation_scales_by_the_largest_channel() {
        let s = apply([1.5, 0.75, 0.0], Intent::Saturation, ONE);
        assert_eq!(s, [1.0, 0.5, 0.0]);
    }

    #[test]
    fn absolute_applies_the_white_scale() {
        let a = apply([0.5, 0.5, 0.5], Intent::AbsoluteColorimetric, [1.0, 0.9, 0.8]);
        assert!((a[0] - 0.5).abs() < 1e-6 && (a[1] - 0.45).abs() < 1e-6 && (a[2] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn black_point_compensation_maps_black_to_black() {
        let b = compensate_black([0.05, 0.05, 0.05], 0.05, 0.0);
        assert!(b[0].abs() < 1e-6);
        let w = compensate_black([1.0, 1.0, 1.0], 0.05, 0.0);
        assert!((w[0] - 1.0).abs() < 1e-6);
        assert_eq!(compensate_black([0.3, 0.3, 0.3], 1.0, 0.0), [0.3, 0.3, 0.3]);
        let c = convert([0.05, 0.05, 0.05], Intent::RelativeColorimetric, true, 0.05, 0.0, ONE);
        assert!(c[0] < 1e-6);
    }
}
