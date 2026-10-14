//! Vibrance and a five-point tone curve for single colours. Colours are unpremultiplied RGB with
//! each channel in 0..=1, the convention of `colorfx`, so both functions can be passed to
//! `colorfx::map_pixel` for packed pixels.
use crate::colorfx::Rgb;

/// Raises the saturation of the less saturated colours more than the saturated ones. `amount` 0
/// leaves the colour unchanged; positive values boost, negative values reduce. The per-colour
/// weight is `amount * (1 - s)`, where `s` is the saturation (largest minus smallest channel,
/// divided by the largest). Grey has no chroma to scale, so it stays grey.
pub fn vibrance(c: Rgb, amount: f32) -> Rgb {
    let amount = if amount.is_finite() { amount } else { 0.0 };
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
    let scale = 1.0 + amount * (1.0 - sat);
    let l = c.luma();
    Rgb::new(l + (c.r - l) * scale, l + (c.g - l) * scale, l + (c.b - l) * scale).clamped()
}

/// A tone curve through five control points, each `(input, output)` in 0..=1. Between points the
/// curve is linear. The inputs must strictly increase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneCurve {
    points: [(f32, f32); 5],
}

impl ToneCurve {
    /// The identity curve: the diagonal through the five default points (0, 0.25, 0.5, 0.75, 1).
    pub fn identity() -> Self {
        ToneCurve { points: [(0.0, 0.0), (0.25, 0.25), (0.5, 0.5), (0.75, 0.75), (1.0, 1.0)] }
    }

    /// Builds a curve from five points. Returns `None` unless every value is finite and within
    /// 0..=1 and the inputs strictly increase.
    pub fn new(points: [(f32, f32); 5]) -> Option<Self> {
        let in_range = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        let valid = points.iter().all(|&(x, y)| in_range(x) && in_range(y))
            && points.windows(2).all(|w| w[0].0 < w[1].0);
        valid.then_some(ToneCurve { points })
    }

    /// The output for input `x`. Inputs outside 0..=1 are clamped to the ends of the curve.
    pub fn at(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        for w in self.points.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if x <= x1 {
                return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
            }
        }
        self.points[4].1
    }

    /// Applies the curve to each channel of `c`.
    pub fn apply(&self, c: Rgb) -> Rgb {
        Rgb::new(self.at(c.r), self.at(c.g), self.at(c.b)).clamped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn saturation_of(c: Rgb) -> f32 {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        if max > 0.0 {
            (max - min) / max
        } else {
            0.0
        }
    }

    #[test]
    fn vibrance_zero_is_identity_and_grey_stays_grey() {
        let c = Rgb::new(0.8, 0.4, 0.2);
        let same = vibrance(c, 0.0);
        assert!(close(same.r, c.r) && close(same.g, c.g) && close(same.b, c.b));
        let grey = Rgb::new(0.5, 0.5, 0.5);
        let out = vibrance(grey, 1.0);
        assert!(close(out.r, 0.5) && close(out.g, 0.5) && close(out.b, 0.5));
    }

    #[test]
    fn vibrance_boosts_muted_colours_more_than_vivid_ones() {
        let muted = Rgb::new(0.6, 0.5, 0.45);
        let vivid = Rgb::new(1.0, 0.2, 0.0);
        let muted_gain = saturation_of(vibrance(muted, 0.8)) - saturation_of(muted);
        let vivid_gain = saturation_of(vibrance(vivid, 0.8)) - saturation_of(vivid);
        assert!(muted_gain > 0.0);
        assert!(muted_gain > vivid_gain);
    }

    #[test]
    fn identity_tone_curve_leaves_values_unchanged() {
        let id = ToneCurve::identity();
        for &x in &[0.0, 0.1, 0.25, 0.37, 0.5, 0.9, 1.0] {
            assert!(close(id.at(x), x));
        }
        assert_eq!(ToneCurve::new([(0.0, 0.0), (0.25, 0.25), (0.5, 0.5), (0.75, 0.75), (1.0, 1.0)]), Some(id));
    }

    #[test]
    fn tone_curve_interpolates_between_points_and_clamps_inputs() {
        let curve = ToneCurve::new([(0.0, 0.0), (0.25, 0.5), (0.5, 0.5), (0.75, 0.5), (1.0, 1.0)]).unwrap();
        assert!(close(curve.at(0.125), 0.25));
        assert!(close(curve.at(0.5), 0.5));
        assert!(close(curve.at(-3.0), 0.0));
        assert!(close(curve.at(7.0), 1.0));
        let out = curve.apply(Rgb::new(0.125, 0.5, 1.0));
        assert!(close(out.r, 0.25) && close(out.g, 0.5) && close(out.b, 1.0));
    }

    #[test]
    fn tone_curve_rejects_unordered_or_out_of_range_points() {
        assert!(ToneCurve::new([(0.0, 0.0), (0.5, 0.5), (0.4, 0.4), (0.75, 0.75), (1.0, 1.0)]).is_none());
        assert!(ToneCurve::new([(0.0, 0.0), (0.25, 1.5), (0.5, 0.5), (0.75, 0.75), (1.0, 1.0)]).is_none());
        assert!(ToneCurve::new([(0.0, f32::NAN), (0.25, 0.25), (0.5, 0.5), (0.75, 0.75), (1.0, 1.0)]).is_none());
    }
}
