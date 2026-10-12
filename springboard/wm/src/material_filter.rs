//! The colour filter stages of a material, as pure math (host-testable): the saturation matrix, the brightness
//! offset and the luminance mapping. The compose shader (`gpu/shaders/compose.frag`) and the GLES glass path
//! (`glass.rs`) implement the same arithmetic; the tests here are the hand-computed reference both are held to.
//! Measured values and the evidence: `docs/reference/materials-macos27.md`.
//!
//! Order of the stages on a blurred backdrop pixel (straight RGB, 0..=1), each stage clamped to 0..=1:
//! 1. saturation: `mix(luma, rgb, s)` with the Rec.709 luma weights (the weights are confirmed by the glass colour
//!    matrix of the platform, whose off-diagonal terms are in the ratio of these weights);
//! 2. luminance mapping: the pixel's luma `l` is pulled towards a curve of itself, `rgb += amount * (curve(l) - l)`;
//!    `curve` is the cubic Bezier through the four recipe values at the input luma (the four values are control
//!    values equally spaced on the luma axis; INFERRED, see the document);
//! 3. brightness: `rgb += brightness`.

/// Rec.709 luma weights.
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The luminance mapping and brightness of a material (saturation is separate: it differs per appearance and is
/// already carried by `Resolved::Glass`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    /// Added to every channel after the luminance mapping (-1..1).
    pub brightness: f32,
    /// 0 skips the mapping, 1 replaces the luma with the curve.
    pub luminance_amount: f32,
    /// The four control values of the luminance curve (they may leave 0..1).
    pub luminance_values: [f32; 4],
}

impl Tone {
    /// No mapping, no offset.
    pub const NONE: Tone = Tone { brightness: 0.0, luminance_amount: 0.0, luminance_values: [0.0; 4] };

    pub const fn new(brightness: f32, luminance_amount: f32, luminance_values: [f32; 4]) -> Tone {
        Tone { brightness, luminance_amount, luminance_values }
    }

    pub fn is_none(&self) -> bool {
        self.brightness == 0.0 && self.luminance_amount == 0.0
    }

    /// The tone stages (2 and 3) on one pixel.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mapped = map_luminance(rgb, self.luminance_amount, self.luminance_values);
        brighten(mapped, self.brightness)
    }
}

impl Default for Tone {
    fn default() -> Tone {
        Tone::NONE
    }
}

fn clamp01(c: [f32; 3]) -> [f32; 3] {
    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
}

/// The luma of a pixel.
pub fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2]
}

/// The saturation matrix applied to a pixel: `s` 1 keeps it, 0 is grey, above 1 pushes away from grey.
pub fn saturate(rgb: [f32; 3], s: f32) -> [f32; 3] {
    let l = luma(rgb);
    clamp01([l + (rgb[0] - l) * s, l + (rgb[1] - l) * s, l + (rgb[2] - l) * s])
}

/// The row-major 3x3 saturation matrix for `s` (what a shader multiplies by).
pub fn saturation_matrix(s: f32) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = (1.0 - s) * LUMA[c] + if r == c { s } else { 0.0 };
        }
    }
    m
}

/// The luminance curve at input `x` (0..=1): the cubic Bezier with the four control values.
pub fn curve(values: [f32; 4], x: f32) -> f32 {
    let u = 1.0 - x;
    u * u * u * values[0] + 3.0 * u * u * x * values[1] + 3.0 * u * x * x * values[2] + x * x * x * values[3]
}

/// The luminance mapping stage: shifts every channel by the amount the pixel's luma moves towards the curve.
pub fn map_luminance(rgb: [f32; 3], amount: f32, values: [f32; 4]) -> [f32; 3] {
    if amount == 0.0 {
        return rgb;
    }
    let l = luma(rgb);
    let d = amount * (curve(values, l.clamp(0.0, 1.0)) - l);
    clamp01([rgb[0] + d, rgb[1] + d, rgb[2] + d])
}

/// The brightness stage.
pub fn brighten(rgb: [f32; 3], b: f32) -> [f32; 3] {
    clamp01([rgb[0] + b, rgb[1] + b, rgb[2] + b])
}

/// All three stages in order: saturation, luminance mapping, brightness.
pub fn filter(rgb: [f32; 3], saturation: f32, tone: Tone) -> [f32; 3] {
    tone.apply(saturate(rgb, saturation))
}

/// A 5x4 colour matrix (rows R, G, B, A; the fifth column is the offset) applied to straight RGBA.
pub fn apply_color_matrix(m: &[[f32; 5]; 4], c: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (i, row) in m.iter().enumerate() {
        out[i] = (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3] * c[3] + row[4]).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn saturation_one_is_identity_and_zero_is_the_luma() {
        let c = [0.8, 0.4, 0.2];
        assert_eq!(saturate(c, 1.0), c);
        let l = luma(c);
        let g = saturate(c, 0.0);
        assert!(g.iter().all(|&v| near(v, l)));
    }

    #[test]
    fn saturation_matrix_rows_sum_to_one_and_match_the_per_pixel_form() {
        let m = saturation_matrix(1.5);
        for row in m {
            assert!(near(row.iter().sum::<f32>(), 1.0));
        }
        // hand-computed: luma of (0.5, 0.25, 0.25) = 0.10630 + 0.17880 + 0.01805 = 0.30315; R = 0.30315 + 0.19685 * 1.5
        let c = [0.5, 0.25, 0.25];
        let p = saturate(c, 1.5);
        let by_matrix: Vec<f32> = m.iter().map(|r| r[0] * c[0] + r[1] * c[1] + r[2] * c[2]).collect();
        assert!(near(p[0], 0.598_425) && near(p[1], by_matrix[1]) && near(p[0], by_matrix[0]));
    }

    #[test]
    fn the_curve_hits_its_end_values_and_the_hand_computed_midpoint() {
        let v = [0.23, 0.52, 0.27, 0.255];
        assert!(near(curve(v, 0.0), 0.23) && near(curve(v, 1.0), 0.255));
        // t = 0.5: (0.23 + 3 * 0.52 + 3 * 0.27 + 0.255) / 8 = 2.855 / 8
        assert!(near(curve(v, 0.5), 0.356_875));
    }

    #[test]
    fn the_dark_chrome_tone_on_black_and_on_mid_grey() {
        // dark chrome: amount 0.75, brightness -0.1, values [0.23 0.52 0.27 0.255]
        let t = Tone::new(-0.1, 0.75, [0.23, 0.52, 0.27, 0.255]);
        // black: l = 0, curve = 0.23, shift = 0.75 * 0.23 = 0.1725, brightness -> 0.0725
        assert!(t.apply([0.0; 3]).iter().all(|&v| near(v, 0.0725)));
        // mid grey 0.5: shift = 0.75 * (0.356875 - 0.5) = -0.10734375, brightness -> 0.29265625
        assert!(t.apply([0.5; 3]).iter().all(|&v| near(v, 0.292_656)));
    }

    #[test]
    fn the_light_chrome_tone_on_white_and_black() {
        let t = Tone::new(0.1, 0.75, [0.8, 0.9, 1.1, 0.825]);
        // white: curve(1) = 0.825, shift 0.75 * (0.825 - 1) = -0.13125, brightness -> 0.96875
        assert!(t.apply([1.0; 3]).iter().all(|&v| near(v, 0.968_75)));
        // a brightness push past white clamps
        assert_eq!(brighten([0.95; 3], 0.2), [1.0; 3]);
        // black: curve = 0.8, shift 0.6, brightness 0.1 -> 0.7
        assert!(t.apply([0.0; 3]).iter().all(|&v| near(v, 0.7)));
    }

    #[test]
    fn an_amount_of_zero_skips_the_mapping_and_keeps_chroma() {
        let t = Tone::new(0.05, 0.0, [9.0; 4]);
        let out = t.apply([0.2, 0.4, 0.6]);
        assert!(near(out[0], 0.25) && near(out[1], 0.45) && near(out[2], 0.65));
        assert!(Tone::NONE.is_none() && !t.is_none());
    }

    #[test]
    fn the_stages_run_in_order() {
        // saturate first (grey stays grey), then tone
        let t = Tone::new(0.1, 0.0, [0.0; 4]);
        let p = filter([0.5; 3], 2.0, t);
        assert!(p.iter().all(|&v| near(v, 0.6)));
    }

    #[test]
    fn the_glass_colour_matrix_is_a_saturation_gain_and_offset() {
        // the platform's glass matrix, rows R G B A (its measured entries)
        let m = [
            [0.921, -0.265, -0.027, 0.0, 0.235],
            [-0.079, 0.735, -0.027, 0.0, 0.235],
            [-0.079, -0.265, 0.973, 0.0, 0.235],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ];
        // our decomposition: saturation 1.5885, then a grey veil of 0.6343 at alpha 0.3705 (gain 0.6295, offset 0.235)
        for c in [[0.2f32, 0.5, 0.8], [0.9, 0.1, 0.3], [0.0, 0.0, 0.0], [0.5, 0.5, 0.5]] {
            let want = apply_color_matrix(&m, [c[0], c[1], c[2], 1.0]);
            let s = saturate(c, 1.5885);
            let a = 0.3705;
            let veil = 0.6343;
            for i in 0..3 {
                let got = s[i] * (1.0 - a) + veil * a;
                // saturate clamps where the matrix does too, so compare unclamped interior pixels only
                if want[i] > 0.001 && want[i] < 0.999 {
                    assert!((got - want[i]).abs() < 0.01, "{c:?} ch {i}: {got} vs {}", want[i]);
                }
            }
        }
    }
}
