//! Rendering intents and black point compensation for the working-space conversions in `colorspace`.
//!
//! A rendering intent decides how a conversion treats colours the destination cannot show, and whether the source
//! white is mapped onto the destination white. `colorspace::Transform` implements only the relative colorimetric
//! case, where out-of-gamut values are left out of range. This module adds the other three intents and an optional
//! black point compensation step. The formulas are the usual definitions, applied to linear light. There is no ICC
//! profile reader here, so the intent stored in a profile's lookup tables is not used; the caller picks the intent.

use super::cg::color::RenderingIntent;
use super::colorspace::{matrix, matrix_absolute, Mat3, Space};

/// The rendering intent of a conversion. This is the enum the colour model in `cg::color` already uses, so the
/// two conversion paths share one vocabulary. `Default` behaves as relative colorimetric here.
pub type Intent = RenderingIntent;

impl RenderingIntent {
    /// The four named intents, in the order a colour settings page lists them.
    pub const ALL: [RenderingIntent; 4] = [
        RenderingIntent::Perceptual,
        RenderingIntent::RelativeColorimetric,
        RenderingIntent::Saturation,
        RenderingIntent::AbsoluteColorimetric,
    ];

    /// The name a colour settings page shows.
    pub fn name(self) -> &'static str {
        match self {
            RenderingIntent::Default => "Default",
            RenderingIntent::Perceptual => "Perceptual",
            RenderingIntent::RelativeColorimetric => "Relative Colorimetric",
            RenderingIntent::Saturation => "Saturation",
            RenderingIntent::AbsoluteColorimetric => "Absolute Colorimetric",
        }
    }

    /// Whether the intent maps the source white onto the destination white. Perceptual compresses out-of-gamut
    /// colours toward grey, saturation clips them, relative colorimetric and the default leave them out of range,
    /// and absolute colorimetric keeps the source white as it is.
    pub fn adapts_white(self) -> bool {
        self != RenderingIntent::AbsoluteColorimetric
    }
}

/// Black points of the source and destination as linear luminance in 0..1. Zero is an ideal black; a real display
/// has a small positive black point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlackPoints {
    pub source: f64,
    pub destination: f64,
}

/// Maps one linear luminance value so that the source black lands on the destination black and the source white
/// stays at white. The map is affine, so values below the source black go below the destination black (they are
/// not clipped here). A source black of 1 or more has no range to scale and maps every value to the destination black.
pub fn compensate_black(v: f64, source: f64, destination: f64) -> f64 {
    if source >= 1.0 {
        return destination;
    }
    destination + (v - source) * (1.0 - destination) / (1.0 - source)
}

/// Multiplies a 3x3 matrix by a column vector.
fn mul_vec(m: Mat3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Perceptual gamut compression. A linear colour with a channel outside 0..=1 is moved toward the neutral grey of
/// its mean until every channel fits, so the hue order of the channels is kept. In-range colours are returned as
/// they are.
fn compress_to_gamut(c: [f64; 3]) -> [f64; 3] {
    if c.iter().all(|v| (0.0..=1.0).contains(v)) {
        return c;
    }
    let grey = ((c[0] + c[1] + c[2]) / 3.0).clamp(0.0, 1.0);
    let mut t: f64 = 1.0;
    for v in c {
        let d = v - grey;
        if v > 1.0 && d > 0.0 {
            t = t.min((1.0 - grey) / d);
        }
        if v < 0.0 && d < 0.0 {
            t = t.min(grey / -d);
        }
    }
    let t = t.clamp(0.0, 1.0);
    c.map(|v| (grey + (v - grey) * t).clamp(0.0, 1.0))
}

/// A conversion between two working spaces under one rendering intent, optionally with black point compensation.
#[derive(Clone, Copy, Debug)]
pub struct IntentTransform {
    from: Space,
    to: Space,
    intent: Intent,
    m: Mat3,
    black: Option<BlackPoints>,
}

impl IntentTransform {
    /// Builds the conversion from `from` to `to` under `intent`, without black point compensation.
    pub fn new(from: Space, to: Space, intent: Intent) -> Self {
        let m = if intent.adapts_white() { matrix(from, to) } else { matrix_absolute(from, to) };
        IntentTransform { from, to, intent, m, black: None }
    }

    /// Enables black point compensation with the given black points.
    pub fn with_black_points(mut self, black: BlackPoints) -> Self {
        self.black = Some(black);
        self
    }

    /// The rendering intent of this conversion.
    pub fn intent(&self) -> Intent {
        self.intent
    }

    /// Converts one colour given as encoded channels (nominally 0..=1). The result is encoded in the destination
    /// space and lies in 0..=1 for the perceptual and saturation intents; the other two may leave that range.
    pub fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        let lin = rgb.map(|c| self.from.to_linear(c));
        let mut out = mul_vec(self.m, lin);
        if let Some(b) = self.black {
            out = out.map(|c| compensate_black(c, b.source, b.destination));
        }
        out = match self.intent {
            Intent::Perceptual => compress_to_gamut(out),
            Intent::Saturation => out.map(|c| c.clamp(0.0, 1.0)),
            Intent::Default | Intent::RelativeColorimetric | Intent::AbsoluteColorimetric => out,
        };
        let encoded = out.map(|c| self.to.from_linear(c));
        match self.intent {
            // Encoding can overshoot 1 by a rounding step; these two intents promise an in-range result.
            Intent::Perceptual | Intent::Saturation => encoded.map(|c| c.clamp(0.0, 1.0)),
            _ => encoded,
        }
    }

    /// Converts one 8-bit colour, clamping the result to 0..=255.
    pub fn apply_u8(&self, rgb: [u8; 3]) -> [u8; 3] {
        let out = self.apply(rgb.map(|c| c as f64 / 255.0));
        out.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colorspace::Transform;

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
    }

    #[test]
    fn intent_names_and_white_adaptation() {
        assert_eq!(Intent::ALL.len(), 4);
        assert_eq!(Intent::RelativeColorimetric.name(), "Relative Colorimetric");
        assert!(Intent::Perceptual.adapts_white());
        assert!(Intent::Saturation.adapts_white());
        assert!(Intent::RelativeColorimetric.adapts_white());
        assert!(!Intent::AbsoluteColorimetric.adapts_white());
        assert!(Intent::Default.adapts_white());
        assert_eq!(Intent::Default.name(), "Default");
    }

    #[test]
    fn relative_matches_the_plain_transform() {
        let plain = Transform::new(Space::Srgb, Space::DisplayP3);
        let intent = IntentTransform::new(Space::Srgb, Space::DisplayP3, Intent::RelativeColorimetric);
        for c in [[0.2, 0.5, 0.8], [0.9, 0.1, 0.4], [1.0, 1.0, 1.0]] {
            assert!(close(intent.apply(c), plain.apply(c), 1e-9), "{c:?}");
        }
    }

    #[test]
    fn relative_keeps_white_white_and_absolute_does_not() {
        let relative = IntentTransform::new(Space::Srgb, Space::RommRgb, Intent::RelativeColorimetric);
        assert!(close(relative.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-9));

        // The D65 white of the source is not the D50 white of the destination, so its channels differ.
        let absolute = IntentTransform::new(Space::Srgb, Space::RommRgb, Intent::AbsoluteColorimetric);
        let out = absolute.apply([1.0, 1.0, 1.0]);
        let spread = out.iter().cloned().fold(f64::MIN, f64::max) - out.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread > 0.01, "absolute white should not be neutral: {out:?}");
    }

    #[test]
    fn perceptual_brings_wide_gamut_red_into_range_and_keeps_hue_order() {
        let t = IntentTransform::new(Space::DisplayP3, Space::Srgb, Intent::Perceptual);
        let out = t.apply([1.0, 0.0, 0.0]);
        assert!(out.iter().all(|c| (0.0..=1.0).contains(c)), "{out:?}");
        // Red stays the strongest channel and green stays the weakest of the two others.
        assert!(out[0] > out[1] && out[0] > out[2], "{out:?}");
        assert!(out[1] < out[2], "{out:?}");
    }

    #[test]
    fn saturation_clips_out_of_gamut_channels() {
        let t = IntentTransform::new(Space::DisplayP3, Space::Srgb, Intent::Saturation);
        assert_eq!(t.apply_u8([255, 0, 0]), [255, 0, 0]);
        let out = t.apply([1.0, 0.0, 0.0]);
        assert!(out[1].abs() < 1e-12 && out[2].abs() < 1e-12, "{out:?}");
        assert_eq!(t.intent(), Intent::Saturation);
    }

    #[test]
    fn compensate_black_maps_the_blacks_and_keeps_white() {
        let (src, dst) = (0.02, 0.05);
        assert!((compensate_black(src, src, dst) - dst).abs() < 1e-12);
        assert!((compensate_black(1.0, src, dst) - 1.0).abs() < 1e-12);
        assert!((compensate_black(0.0, 0.0, 0.0)).abs() < 1e-12);
        // Monotonic over the range.
        let mut last = f64::MIN;
        for step in 0..=100 {
            let v = compensate_black(step as f64 / 100.0, src, dst);
            assert!(v > last);
            last = v;
        }
    }

    #[test]
    fn black_point_compensation_lifts_black_and_keeps_white() {
        let black = BlackPoints { source: 0.0, destination: 0.02 };
        let t = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric).with_black_points(black);
        let lifted = Space::Srgb.from_linear(0.02);
        assert!(close(t.apply([0.0, 0.0, 0.0]), [lifted, lifted, lifted], 1e-6));
        assert!(close(t.apply([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0], 1e-6));
    }

    #[test]
    fn zero_black_points_are_the_identity() {
        let t = IntentTransform::new(Space::Srgb, Space::Srgb, Intent::RelativeColorimetric)
            .with_black_points(BlackPoints { source: 0.0, destination: 0.0 });
        for c in [[0u8, 1, 128], [200, 17, 255]] {
            assert_eq!(t.apply_u8(c), c);
        }
    }
}
