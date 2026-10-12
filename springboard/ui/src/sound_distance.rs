//! Distance attenuation and directional emission for sound sources.
//! Pure gain functions with no audio output and no positions. The caller supplies the distance
//! from the listener, or the angle in radians between the source's facing axis and the direction
//! to the listener, and applies the returned gain (0 to 1) to a sound.
use crate::sound_envelope::Envelope;
use std::f32::consts::PI;

/// Gain that falls off with distance in metres. Full level up to `min_distance`, then
/// `min / (min + rolloff * (d - min))`. Distances past `max_distance` keep the gain of `max_distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometricSpreading {
    pub min_distance: f32,
    pub max_distance: f32,
    /// How fast the level falls. Zero means no falloff at all.
    pub rolloff: f32,
}

impl GeometricSpreading {
    /// Returns None unless every value is finite, `min_distance` is above zero, `max_distance`
    /// is at least `min_distance` and `rolloff` is not negative.
    pub fn new(min_distance: f32, max_distance: f32, rolloff: f32) -> Option<Self> {
        let finite = min_distance.is_finite() && max_distance.is_finite() && rolloff.is_finite();
        let valid = finite && min_distance > 0.0 && max_distance >= min_distance && rolloff >= 0.0;
        valid.then_some(Self { min_distance, max_distance, rolloff })
    }

    /// Gain at `distance`. The denominator is at least `min_distance`, so it never reaches zero.
    pub fn gain(&self, distance: f32) -> f32 {
        let d = distance.clamp(self.min_distance, self.max_distance);
        self.min_distance / (self.min_distance + self.rolloff * (d - self.min_distance))
    }
}

/// Gain from an envelope whose input is distance and whose output is the level. The result is
/// clamped to 0..=1 so an envelope cannot boost a sound above its source level.
pub fn envelope_gain(envelope: &Envelope, distance: f32) -> f32 {
    envelope.evaluate(distance).clamp(0.0, 1.0)
}

/// Cardioid emission: 1 straight ahead, 0.5 at 90 degrees, 0 directly behind.
/// `angle` is in radians between the source axis and the direction to the listener.
pub fn cardioid_gain(angle: f32) -> f32 {
    0.5 * (1.0 + angle.cos())
}

/// Cone emission. Full level within `inner` (a half-angle in radians), `outer_gain` at or beyond
/// `outer`, and a straight line between the two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    pub inner: f32,
    pub outer: f32,
    pub outer_gain: f32,
}

impl Cone {
    /// Returns None unless `0 <= inner <= outer <= PI` and `0 <= outer_gain <= 1`, all finite.
    pub fn new(inner: f32, outer: f32, outer_gain: f32) -> Option<Self> {
        let finite = inner.is_finite() && outer.is_finite() && outer_gain.is_finite();
        let valid = finite
            && inner >= 0.0
            && outer >= inner
            && outer <= PI
            && (0.0..=1.0).contains(&outer_gain);
        valid.then_some(Self { inner, outer, outer_gain })
    }

    /// Gain at `angle` in radians. The sign is ignored, so left and right of the axis match.
    pub fn gain(&self, angle: f32) -> f32 {
        let a = angle.abs();
        if a <= self.inner {
            1.0
        } else if a >= self.outer {
            self.outer_gain
        } else {
            // Here outer > inner, because a lies strictly between them. No division by zero.
            let t = (a - self.inner) / (self.outer - self.inner);
            1.0 + (self.outer_gain - 1.0) * t
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_envelope::{Curve, Segment};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn spreading_is_full_level_inside_the_minimum_distance() {
        let model = GeometricSpreading::new(1.0, 10.0, 1.0).unwrap();
        assert!(close(model.gain(0.5), 1.0));
        assert!(close(model.gain(1.0), 1.0));
    }

    #[test]
    fn spreading_falls_with_rolloff() {
        // min 1, rolloff 1: at d = 3 the denominator is 1 + 1 * 2 = 3, so gain is 1/3.
        let model = GeometricSpreading::new(1.0, 10.0, 1.0).unwrap();
        assert!(close(model.gain(3.0), 1.0 / 3.0));
    }

    #[test]
    fn spreading_holds_at_max_distance() {
        // Beyond max 10 the gain stays at the value for 10: 1 / (1 + 9) = 0.1.
        let model = GeometricSpreading::new(1.0, 10.0, 1.0).unwrap();
        assert!(close(model.gain(10.0), 0.1));
        assert!(close(model.gain(500.0), 0.1));
    }

    #[test]
    fn zero_rolloff_means_no_falloff() {
        let model = GeometricSpreading::new(1.0, 100.0, 0.0).unwrap();
        assert!(close(model.gain(50.0), 1.0));
    }

    #[test]
    fn invalid_spreading_is_rejected() {
        assert!(GeometricSpreading::new(0.0, 10.0, 1.0).is_none());
        assert!(GeometricSpreading::new(2.0, 1.0, 1.0).is_none());
        assert!(GeometricSpreading::new(1.0, 10.0, -0.5).is_none());
    }

    #[test]
    fn envelope_gain_reads_the_curve_and_clamps() {
        // Distance envelope: from 1.0 at distance 0 down to 0.0 at distance 10.
        let line = Envelope::new(
            (0.0, 1.0),
            vec![Segment { length: 10.0, target: 0.0, curve: Curve::Linear }],
        )
        .unwrap();
        assert!(close(envelope_gain(&line, 5.0), 0.5));
        assert!(close(envelope_gain(&line, 20.0), 0.0));
        // A target above 1 is clamped to 1.
        let boost = Envelope::new(
            (0.0, 1.0),
            vec![Segment { length: 1.0, target: 2.0, curve: Curve::Linear }],
        )
        .unwrap();
        assert!(close(envelope_gain(&boost, 1.0), 1.0));
    }

    #[test]
    fn cardioid_matches_the_heart_shape() {
        assert!(close(cardioid_gain(0.0), 1.0));
        assert!(close(cardioid_gain(PI / 2.0), 0.5));
        assert!(close(cardioid_gain(PI), 0.0));
    }

    #[test]
    fn cone_is_linear_between_inner_and_outer() {
        // inner 0.5, outer 1.0, outer gain 0.25. At 0.75 the position is 0.5 of the way,
        // so gain is 1 + (0.25 - 1) * 0.5 = 0.625.
        let cone = Cone::new(0.5, 1.0, 0.25).unwrap();
        assert!(close(cone.gain(0.3), 1.0));
        assert!(close(cone.gain(0.75), 0.625));
        assert!(close(cone.gain(1.5), 0.25));
        // The sign of the angle does not matter.
        assert!(close(cone.gain(-0.75), 0.625));
    }

    #[test]
    fn invalid_cone_is_rejected() {
        assert!(Cone::new(1.0, 0.5, 0.0).is_none());
        assert!(Cone::new(0.0, 0.5, 1.5).is_none());
        assert!(Cone::new(0.0, PI + 0.1, 0.5).is_none());
    }
}
