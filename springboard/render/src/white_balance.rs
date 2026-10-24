//! White balance as a white-point conversion (`CITemperatureAndTint`), replacing the gain model in
//! `colorfx::temperature_tint` for callers that want the reference behaviour.
//!
//! The reference filter takes two (temperature, tint) pairs, `inputNeutral` and `inputTargetNeutral`,
//! both defaulting to (6500, 0): the light the picture was taken under, and the light it should look
//! as if taken under. This model:
//!
//! 1. turns each pair into a chromaticity: the temperature in kelvin (clamped to 1667..=25000) goes
//!    through the published cubic fit of the Planckian locus (Kim et al., 2002); the tint moves the
//!    point across the locus in the CIE 1960 uv plane by `tint / 3000` (positive tint is magenta,
//!    negative is green, the scale raw converters use);
//! 2. adapts from the neutral white to the target white with the Bradford cone response;
//! 3. applies the adaptation to linear sRGB (D65 primaries), decoding and encoding the sRGB curve.
//!
//! A neutral above the target (a bluer light assumed) warms the picture, as in the reference. Equal
//! pairs leave it unchanged.
use crate::colorfx::Rgb;

/// The reference default for both pairs.
pub const DEFAULT_NEUTRAL: (f32, f32) = (6500.0, 0.0);
/// The temperature range of the locus fit, in kelvin.
pub const KELVIN_MIN: f64 = 1667.0;
pub const KELVIN_MAX: f64 = 25000.0;
/// Tint units per unit of distance in the uv plane.
pub const TINT_SCALE: f64 = 3000.0;

type M3 = [[f64; 3]; 3];

const SRGB_TO_XYZ: M3 = [[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]];
const BRADFORD: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

fn mul(a: &M3, b: &M3) -> M3 {
    let mut o = [[0.0; 3]; 3];
    for (i, row) in o.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn inverse(m: &M3) -> M3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ]
}

/// The CIE 1931 xy chromaticity of the Planckian locus at `kelvin` (clamped to the fit's range).
pub fn planckian_xy(kelvin: f64) -> (f64, f64) {
    let t = if kelvin.is_finite() { kelvin.clamp(KELVIN_MIN, KELVIN_MAX) } else { 6500.0 };
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    (x, y)
}

fn xy_to_uv(x: f64, y: f64) -> (f64, f64) {
    let d = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / d, 6.0 * y / d)
}

fn uv_to_xy(u: f64, v: f64) -> (f64, f64) {
    let d = 2.0 * u - 8.0 * v + 4.0;
    (3.0 * u / d, 2.0 * v / d)
}

/// The xy chromaticity of a (temperature, tint) pair.
pub fn neutral_xy(kelvin: f64, tint: f64) -> (f64, f64) {
    let (x, y) = planckian_xy(kelvin);
    let tint = if tint.is_finite() { tint } else { 0.0 };
    if tint == 0.0 {
        return (x, y);
    }
    // The locus direction from a small step in temperature, turned a quarter towards lower v (magenta).
    let k = if kelvin.is_finite() { kelvin.clamp(KELVIN_MIN, KELVIN_MAX) } else { 6500.0 };
    let (k0, k1) = ((k - 10.0).max(KELVIN_MIN), (k + 10.0).min(KELVIN_MAX));
    let (u0, v0) = xy_to_uv(planckian_xy(k0).0, planckian_xy(k0).1);
    let (u1, v1) = xy_to_uv(planckian_xy(k1).0, planckian_xy(k1).1);
    let (du, dv) = (u1 - u0, v1 - v0);
    let len = (du * du + dv * dv).sqrt();
    let (mut nu, mut nv) = (-dv / len, du / len);
    if nv > 0.0 {
        nu = -nu;
        nv = -nv;
    }
    let (u, v) = xy_to_uv(x, y);
    let s = tint / TINT_SCALE;
    uv_to_xy(u + nu * s, v + nv * s)
}

fn white_xyz(xy: (f64, f64)) -> [f64; 3] {
    let (x, y) = xy;
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// A precomputed white balance from `neutral` to `target` (each a temperature in kelvin and a tint).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WhiteBalance {
    /// Linear sRGB to linear sRGB.
    matrix: M3,
}

impl WhiteBalance {
    pub fn new(neutral: (f32, f32), target: (f32, f32)) -> Self {
        let src = white_xyz(neutral_xy(neutral.0 as f64, neutral.1 as f64));
        let dst = white_xyz(neutral_xy(target.0 as f64, target.1 as f64));
        let (cs, cd) = (apply(&BRADFORD, src), apply(&BRADFORD, dst));
        let gain: M3 = [[cd[0] / cs[0], 0.0, 0.0], [0.0, cd[1] / cs[1], 0.0], [0.0, 0.0, cd[2] / cs[2]]];
        let adapt = mul(&inverse(&BRADFORD), &mul(&gain, &BRADFORD));
        let matrix = mul(&inverse(&SRGB_TO_XYZ), &mul(&adapt, &SRGB_TO_XYZ));
        WhiteBalance { matrix }
    }

    /// The identity balance (both pairs at the default).
    pub fn identity() -> Self {
        WhiteBalance::new(DEFAULT_NEUTRAL, DEFAULT_NEUTRAL)
    }

    /// Balances one sRGB-encoded colour, clamped to 0..=1.
    pub fn apply(&self, c: Rgb) -> Rgb {
        let lin = [c.r, c.g, c.b].map(|v| decode(v as f64));
        let out = apply(&self.matrix, lin).map(|v| encode(v) as f32);
        Rgb::new(out[0], out[1], out[2]).clamped()
    }
}

fn decode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn encode(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locus_fit_matches_the_published_points() {
        let (x, y) = planckian_xy(6500.0);
        assert!((x - 0.3135).abs() < 1e-3 && (y - 0.3237).abs() < 1e-3, "{x} {y}");
        let (x, y) = planckian_xy(2856.0);
        // The incandescent point (illuminant A lies on the locus): about (0.4476, 0.4074).
        assert!((x - 0.4476).abs() < 2e-3 && (y - 0.4074).abs() < 2e-3, "{x} {y}");
        assert_eq!(planckian_xy(100.0), planckian_xy(KELVIN_MIN));
    }

    #[test]
    fn uv_conversion_round_trips() {
        let (u, v) = xy_to_uv(0.3127, 0.3290);
        let (x, y) = uv_to_xy(u, v);
        assert!((x - 0.3127).abs() < 1e-12 && (y - 0.3290).abs() < 1e-12);
    }

    #[test]
    fn equal_pairs_are_the_identity() {
        let grey = Rgb::new(0.5, 0.5, 0.5);
        for pair in [(6500.0, 0.0), (3200.0, 20.0)] {
            let out = WhiteBalance::new(pair, pair).apply(grey);
            assert!((out.r - 0.5).abs() < 1e-4 && (out.g - 0.5).abs() < 1e-4 && (out.b - 0.5).abs() < 1e-4, "{out:?}");
        }
    }

    #[test]
    fn a_bluer_assumed_light_warms_the_picture() {
        let grey = Rgb::new(0.5, 0.5, 0.5);
        let warm = WhiteBalance::new((9000.0, 0.0), DEFAULT_NEUTRAL).apply(grey);
        assert!(warm.r > 0.5 && warm.b < 0.5, "{warm:?}");
        let cool = WhiteBalance::new((4000.0, 0.0), DEFAULT_NEUTRAL).apply(grey);
        assert!(cool.r < 0.5 && cool.b > 0.5, "{cool:?}");
    }

    #[test]
    fn tint_moves_across_the_locus() {
        // Assuming a magenta light removes magenta: green rises.
        let grey = Rgb::new(0.5, 0.5, 0.5);
        let out = WhiteBalance::new((6500.0, 50.0), DEFAULT_NEUTRAL).apply(grey);
        assert!(out.g > out.r && out.g > out.b, "{out:?}");
        let (x0, y0) = neutral_xy(6500.0, 0.0);
        let (x1, y1) = neutral_xy(6500.0, 50.0);
        assert!(y1 < y0 && (x1 - x0).abs() < 0.02);
    }
}
