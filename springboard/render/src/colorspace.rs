//! Named RGB working spaces and the matrix conversions between them.
//!
//! Each space is defined by the CIE xy chromaticities of its primaries and white point and by its transfer curve.
//! A conversion decodes the channels to linear light, maps them through XYZ with a 3x3 matrix (adapting the white
//! point with the Bradford transform when the two spaces use different whites) and encodes them again. Colours
//! outside the target gamut come out negative or above 1; `Transform::apply_u8` clamps them. Rendering intents and
//! black point compensation are in `rendering_intent`. ICC profile tables are not modelled here.

/// A 3x3 matrix, row-major.
pub type Mat3 = [[f64; 3]; 3];

/// CIE xy white point of the D65 illuminant.
const D65: (f64, f64) = (0.3127, 0.3290);
/// CIE xy white point of the D50 illuminant.
const D50: (f64, f64) = (0.3457, 0.3585);

/// Bradford cone response matrix used for chromatic adaptation.
const BRADFORD: Mat3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

/// A named RGB working space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// sRGB (IEC 61966-2-1): D65 white, piecewise sRGB transfer curve.
    Srgb,
    /// Display P3: D65 white, the sRGB transfer curve and wide P3 primaries.
    DisplayP3,
    /// Adobe RGB (1998): D65 white, gamma 563/256.
    AdobeRgb,
    /// ROMM RGB (ProPhoto): D50 white, gamma 1.8 with a linear toe.
    RommRgb,
}

impl Space {
    /// Every space, in declaration order.
    pub const ALL: [Space; 4] = [Space::Srgb, Space::DisplayP3, Space::AdobeRgb, Space::RommRgb];

    /// The name a settings page or a colour picker shows.
    pub fn name(self) -> &'static str {
        match self {
            Space::Srgb => "sRGB",
            Space::DisplayP3 => "Display P3",
            Space::AdobeRgb => "Adobe RGB (1998)",
            Space::RommRgb => "ROMM RGB",
        }
    }

    /// CIE xy chromaticities of the red, green and blue primaries and of the white point.
    pub fn chromaticities(self) -> ([(f64, f64); 3], (f64, f64)) {
        match self {
            Space::Srgb => ([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65),
            Space::DisplayP3 => ([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], D65),
            Space::AdobeRgb => ([(0.64, 0.33), (0.21, 0.71), (0.15, 0.06)], D65),
            Space::RommRgb => ([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], D50),
        }
    }

    /// Decodes an encoded channel (nominally 0..=1) to linear light. Negative input is mirrored around zero.
    pub fn to_linear(self, v: f64) -> f64 {
        if v < 0.0 {
            return -self.to_linear(-v);
        }
        match self {
            Space::Srgb | Space::DisplayP3 => {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            Space::AdobeRgb => v.powf(563.0 / 256.0),
            Space::RommRgb => {
                if v < 0.03125 {
                    v / 16.0
                } else {
                    v.powf(1.8)
                }
            }
        }
    }

    /// Encodes a linear-light channel. Negative input is mirrored around zero.
    pub fn from_linear(self, l: f64) -> f64 {
        if l < 0.0 {
            return -self.from_linear(-l);
        }
        match self {
            Space::Srgb | Space::DisplayP3 => {
                if l <= 0.0031308 {
                    12.92 * l
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }
            Space::AdobeRgb => l.powf(256.0 / 563.0),
            Space::RommRgb => {
                if l < 1.0 / 512.0 {
                    16.0 * l
                } else {
                    l.powf(1.0 / 1.8)
                }
            }
        }
    }
}

fn mul(a: Mat3, b: Mat3) -> Mat3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

/// Applies a 3x3 matrix to a column vector.
pub fn apply(m: Mat3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Inverse of a 3x3 matrix by the adjugate. A singular matrix gives infinities; the callers never pass one.
pub fn inverse(m: Mat3) -> Mat3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        [(e * i - f * h) * k, (c * h - b * i) * k, (b * f - c * e) * k],
        [(f * g - d * i) * k, (a * i - c * g) * k, (c * d - a * f) * k],
        [(d * h - e * g) * k, (b * g - a * h) * k, (a * e - b * d) * k],
    ]
}

/// XYZ of a chromaticity with luminance 1.
fn xyz_of((x, y): (f64, f64)) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// XYZ of the D50 connection-space white with luminance 1.
pub fn d50_xyz() -> [f64; 3] {
    xyz_of(D50)
}

/// The matrix from linear RGB of `space` to XYZ, relative to the space's own white.
fn rgb_to_xyz(space: Space) -> Mat3 {
    let (prim, white) = space.chromaticities();
    rgb_to_xyz_of(prim, white)
}

/// The matrix from linear RGB with the given primaries and white (CIE xy) to XYZ relative to that white.
fn rgb_to_xyz_of(prim: [(f64, f64); 3], white: (f64, f64)) -> Mat3 {
    let p = prim.map(xyz_of);
    // Columns are the primaries; scale each column so that RGB (1, 1, 1) lands on the white point.
    let cols: Mat3 = [[p[0][0], p[1][0], p[2][0]], [p[0][1], p[1][1], p[2][1]], [p[0][2], p[1][2], p[2][2]]];
    let s = apply(inverse(cols), xyz_of(white));
    let mut m = cols;
    for row in m.iter_mut() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell *= s[j];
        }
    }
    m
}

/// Bradford chromatic adaptation from one white point to another (both as xy).
pub fn adaptation(from: (f64, f64), to: (f64, f64)) -> Mat3 {
    if from == to {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let src = apply(BRADFORD, xyz_of(from));
    let dst = apply(BRADFORD, xyz_of(to));
    let mut scale = [[0.0; 3]; 3];
    for (i, row) in scale.iter_mut().enumerate() {
        row[i] = dst[i] / src[i];
    }
    mul(inverse(BRADFORD), mul(scale, BRADFORD))
}

/// The colorant matrix of an RGB profile for the given primaries and white (CIE xy): the columns are the XYZ of the
/// red, green and blue colorants relative to the D50 connection space, as an ICC matrix-shaper profile stores them.
pub fn colorants_d50(prim: [(f64, f64); 3], white: (f64, f64)) -> Mat3 {
    mul(adaptation(white, D50), rgb_to_xyz_of(prim, white))
}

/// Linear RGB to linear RGB matrix from `from` to `to`.
pub fn matrix(from: Space, to: Space) -> Mat3 {
    let (_, from_white) = from.chromaticities();
    let (_, to_white) = to.chromaticities();
    mul(inverse(rgb_to_xyz(to)), mul(adaptation(from_white, to_white), rgb_to_xyz(from)))
}

/// Linear RGB to linear RGB matrix without chromatic adaptation. The XYZ of each input colour is kept as it is, so a
/// white in `from` does not become a white in `to`. This is the absolute colorimetric case; luminance is relative
/// to the source white.
pub fn matrix_absolute(from: Space, to: Space) -> Mat3 {
    mul(inverse(rgb_to_xyz(to)), rgb_to_xyz(from))
}

/// Matrix from XYZ relative to the D50 connection space of ICC profiles to linear RGB of `to`. The white point is
/// adapted from D50 to the space's own white, so the D50 white maps to RGB (1, 1, 1). This is the step that lets a
/// matrix-shaper profile (see `icc::MatrixShaper`) convert into any named space.
pub fn from_xyz_d50(to: Space) -> Mat3 {
    let (_, to_white) = to.chromaticities();
    mul(inverse(rgb_to_xyz(to)), adaptation(D50, to_white))
}

/// A conversion between two working spaces, with its matrix computed once.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    from: Space,
    to: Space,
    m: Mat3,
}

impl Transform {
    /// Builds the conversion from `from` to `to`.
    pub fn new(from: Space, to: Space) -> Self {
        Transform { from, to, m: matrix(from, to) }
    }

    /// The source space.
    pub fn source(&self) -> Space {
        self.from
    }

    /// The destination space.
    pub fn destination(&self) -> Space {
        self.to
    }

    /// The linear-light matrix between the two spaces.
    pub fn matrix(&self) -> Mat3 {
        self.m
    }

    /// Converts one colour given as encoded channels (nominally 0..=1). The result may leave 0..=1.
    pub fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        let lin = rgb.map(|c| self.from.to_linear(c));
        apply(self.m, lin).map(|c| self.to.from_linear(c))
    }

    /// Converts one 8-bit colour, clamping out-of-gamut results to the 0..=255 range.
    pub fn apply_u8(&self, rgb: [u8; 3]) -> [u8; 3] {
        let out = self.apply(rgb.map(|c| c as f64 / 255.0));
        out.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

#[cfg(test)]
mod bradford_tests {
    use super::*;

    #[test]
    fn bradford_d65_to_d50_matches_the_published_matrix() {
        // Bradford D65 -> D50 as published (Lindbloom); the white points here come from their xy values, so the
        // match is to about 1e-3.
        let want: Mat3 = [[1.0478112, 0.0228866, -0.0501270], [0.0295424, 0.9904844, -0.0170491], [-0.0092345, 0.0150436, 0.7521316]];
        let got = adaptation(D65, D50);
        for i in 0..3 {
            for j in 0..3 {
                assert!((got[i][j] - want[i][j]).abs() < 1e-3, "[{i}][{j}] {} vs {}", got[i][j], want[i][j]);
            }
        }
    }

    #[test]
    fn srgb_colorants_relative_to_d50_match_the_profile_values() {
        // The sRGB colorants adapted to D50, as every sRGB ICC profile lists them (rXYZ, gXYZ, bXYZ columns).
        let (prim, white) = Space::Srgb.chromaticities();
        let m = colorants_d50(prim, white);
        let col = |j: usize| [m[0][j], m[1][j], m[2][j]];
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 2e-3);
        assert!(close(col(0), [0.4361, 0.2225, 0.0139]));
        assert!(close(col(1), [0.3851, 0.7169, 0.0971]));
        assert!(close(col(2), [0.1431, 0.0606, 0.7141]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
    }

    #[test]
    fn inverse_undoes_a_matrix() {
        let m: Mat3 = [[2.0, 1.0, 0.5], [0.0, 3.0, 1.0], [1.0, 0.0, 4.0]];
        let id = mul(m, inverse(m));
        for i in 0..3 {
            for j in 0..3 {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((id[i][j] - want).abs() < 1e-12, "cell {i},{j} = {}", id[i][j]);
            }
        }
    }

    #[test]
    fn transfer_curves_round_trip() {
        for space in Space::ALL {
            for step in 0..=100 {
                let v = step as f64 / 100.0;
                let back = space.from_linear(space.to_linear(v));
                assert!((back - v).abs() < 1e-9, "{:?} at {v}: {back}", space);
            }
        }
    }

    #[test]
    fn srgb_midgrey_decodes_to_known_linear_value() {
        // IEC 61966-2-1: encoded 0.5 is 0.2140 in linear light.
        assert!((Space::Srgb.to_linear(0.5) - 0.2140).abs() < 1e-4);
        assert!((Space::Srgb.from_linear(0.2140) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn negative_channels_are_mirrored() {
        for space in Space::ALL {
            let pos = space.to_linear(0.4);
            assert!((space.to_linear(-0.4) + pos).abs() < 1e-12);
            assert!((space.from_linear(-0.2) + space.from_linear(0.2)).abs() < 1e-12);
        }
    }

    #[test]
    fn white_maps_to_white_between_all_spaces() {
        for from in Space::ALL {
            for to in Space::ALL {
                let out = Transform::new(from, to).apply([1.0, 1.0, 1.0]);
                assert!(close(out, [1.0, 1.0, 1.0], 1e-9), "{:?} to {:?}: {out:?}", from, to);
            }
        }
    }

    #[test]
    fn srgb_red_in_display_p3_matches_the_published_value() {
        // sRGB red expressed in Display P3 is about (0.9175, 0.2003, 0.1387) once encoded.
        let out = Transform::new(Space::Srgb, Space::DisplayP3).apply([1.0, 0.0, 0.0]);
        assert!(close(out, [0.9175, 0.2003, 0.1387], 0.005), "{out:?}");
    }

    #[test]
    fn conversion_round_trips_within_float_error() {
        let a = Transform::new(Space::DisplayP3, Space::RommRgb);
        let b = Transform::new(Space::RommRgb, Space::DisplayP3);
        for colour in [[0.2, 0.5, 0.8], [0.9, 0.1, 0.4], [0.0, 1.0, 0.25]] {
            let back = b.apply(a.apply(colour));
            assert!(close(back, colour, 1e-9), "{colour:?} came back as {back:?}");
        }
    }

    #[test]
    fn wide_gamut_red_leaves_srgb_and_clamps_to_8_bits() {
        let p3_to_srgb = Transform::new(Space::DisplayP3, Space::Srgb);
        let raw = p3_to_srgb.apply([1.0, 0.0, 0.0]);
        assert!(raw[1] < 0.0 && raw[2] < 0.0, "expected out-of-gamut green and blue, got {raw:?}");
        assert_eq!(p3_to_srgb.apply_u8([255, 0, 0]), [255, 0, 0]);
    }

    #[test]
    fn the_d50_white_maps_to_white_in_every_space() {
        // The D50 white of the ICC connection space, as XYZ with Y = 1.
        let d50_white = [0.9642, 1.0, 0.8249];
        for space in Space::ALL {
            let lin = apply(from_xyz_d50(space), d50_white);
            assert!(close(lin, [1.0, 1.0, 1.0], 2e-3), "{:?}: {lin:?}", space);
        }
    }

    #[test]
    fn same_space_is_identity_for_8_bit_colours() {
        let t = Transform::new(Space::Srgb, Space::Srgb);
        for c in [[0u8, 1, 128], [200, 17, 255], [255, 255, 255], [9, 99, 180]] {
            assert_eq!(t.apply_u8(c), c);
        }
        assert_eq!(t.source(), Space::Srgb);
        assert_eq!(t.destination(), Space::Srgb);
    }
}
