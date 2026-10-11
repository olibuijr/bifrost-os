//! CIE L*a*b* colour for sRGB values, the perceptual space that colour conversion and palette matching
//! use. A colour goes sRGB to linear light, then to CIE XYZ under the D65 white, then to L*a*b* with the
//! standard cube-root law. The reverse path returns clamped sRGB. `delta_e` is the CIE 1976 difference,
//! the Euclidean distance in L*a*b*. Only sRGB is modelled; ICC profiles and rendering intents are not.

use crate::colorfx::Rgb;
use crate::colorspace::Space;

/// A colour in CIE L*a*b* under the D65 white point. `l` runs 0..=100; `a` and `b` are unbounded
/// opponent axes (about -128..=127 for sRGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

/// XYZ of the D65 reference white, with Y = 1.
const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];

/// Linear sRGB to XYZ (D65), row-major.
const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

/// XYZ (D65) to linear sRGB, row-major.
const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0556434, -0.2040259, 1.0572252],
];

/// Cube-root threshold of the L*a*b* law (6/29 cubed).
const EPSILON: f64 = 0.008856;

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn forward(t: f64) -> f64 {
    if t > EPSILON {
        t.cbrt()
    } else {
        7.787 * t + 16.0 / 116.0
    }
}

fn inverse(t: f64) -> f64 {
    if t * t * t > EPSILON {
        t * t * t
    } else {
        (t - 16.0 / 116.0) / 7.787
    }
}

impl Lab {
    /// The L*a*b* value of an sRGB colour. Channels are clamped to 0..=1 first.
    pub fn from_rgb(c: Rgb) -> Lab {
        let c = c.clamped();
        let lin = [c.r, c.g, c.b].map(|v| Space::Srgb.to_linear(v as f64));
        let xyz = mul(&SRGB_TO_XYZ, lin);
        let fx = forward(xyz[0] / WHITE[0]);
        let fy = forward(xyz[1] / WHITE[1]);
        let fz = forward(xyz[2] / WHITE[2]);
        Lab { l: 116.0 * fy - 16.0, a: 500.0 * (fx - fy), b: 200.0 * (fy - fz) }
    }

    /// The sRGB colour of this L*a*b* value. Out-of-gamut results are clamped to 0..=1.
    pub fn to_rgb(self) -> Rgb {
        let fy = (self.l + 16.0) / 116.0;
        let fx = fy + self.a / 500.0;
        let fz = fy - self.b / 200.0;
        let xyz = [inverse(fx) * WHITE[0], inverse(fy) * WHITE[1], inverse(fz) * WHITE[2]];
        let lin = mul(&XYZ_TO_SRGB, xyz);
        let enc = |v: f64| Space::Srgb.from_linear(v) as f32;
        Rgb::new(enc(lin[0]), enc(lin[1]), enc(lin[2])).clamped()
    }
}

/// The CIE 1976 colour difference between two colours: the Euclidean distance in L*a*b*. A difference
/// near 2.3 is about one just-noticeable step.
pub fn delta_e(x: Lab, y: Lab) -> f64 {
    let (dl, da, db) = (x.l - y.l, x.a - y.a, x.b - y.b);
    (dl * dl + da * da + db * db).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn white_and_black_sit_at_the_ends_of_the_lightness_axis() {
        let white = Lab::from_rgb(Rgb::new(1.0, 1.0, 1.0));
        assert!(close(white.l, 100.0, 0.01) && close(white.a, 0.0, 0.01) && close(white.b, 0.0, 0.01), "{white:?}");
        let black = Lab::from_rgb(Rgb::new(0.0, 0.0, 0.0));
        assert!(close(black.l, 0.0, 1e-9) && close(black.a, 0.0, 1e-9) && close(black.b, 0.0, 1e-9));
    }

    #[test]
    fn pure_red_matches_the_published_lab_values() {
        // sRGB red under D65 is L 53.24, a 80.09, b 67.20.
        let red = Lab::from_rgb(Rgb::new(1.0, 0.0, 0.0));
        assert!(close(red.l, 53.24, 0.05) && close(red.a, 80.09, 0.05) && close(red.b, 67.20, 0.05), "{red:?}");
    }

    #[test]
    fn rgb_survives_a_round_trip_through_lab() {
        for c in [Rgb::new(0.2, 0.4, 0.6), Rgb::new(0.9, 0.1, 0.5), Rgb::new(0.5, 0.5, 0.5), Rgb::new(0.0, 1.0, 0.25)] {
            let back = Lab::from_rgb(c).to_rgb();
            assert!(close(back.r as f64, c.r as f64, 1e-4), "{c:?} -> {back:?}");
            assert!(close(back.g as f64, c.g as f64, 1e-4), "{c:?} -> {back:?}");
            assert!(close(back.b as f64, c.b as f64, 1e-4), "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn grey_ramp_has_rising_lightness_and_no_chroma() {
        let mut last = -1.0;
        for i in 0..=8 {
            let v = i as f32 / 8.0;
            let lab = Lab::from_rgb(Rgb::new(v, v, v));
            assert!(lab.l > last, "lightness must rise: {lab:?}");
            assert!(close(lab.a, 0.0, 0.05) && close(lab.b, 0.0, 0.05), "{lab:?}");
            last = lab.l;
        }
    }

    #[test]
    fn out_of_gamut_lab_is_clamped_into_srgb() {
        let rgb = Lab { l: 50.0, a: 150.0, b: 0.0 }.to_rgb();
        for v in [rgb.r, rgb.g, rgb.b] {
            assert!((0.0..=1.0).contains(&v), "{rgb:?}");
        }
    }

    #[test]
    fn delta_e_is_zero_for_equal_colours_and_symmetric() {
        let red = Lab::from_rgb(Rgb::new(1.0, 0.0, 0.0));
        let blue = Lab::from_rgb(Rgb::new(0.0, 0.0, 1.0));
        assert!(close(delta_e(red, red), 0.0, 1e-12));
        assert!(close(delta_e(red, blue), delta_e(blue, red), 1e-12));
        assert!(delta_e(red, blue) > 100.0, "red and blue are far apart");
    }
}
