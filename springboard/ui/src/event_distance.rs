//! Distance models and directivity for a sound source: how loud a source is at a distance, and how loud it is from
//! each direction. Gains are linear factors (1.0 is unchanged, 0.0 is silent). Pure Rust: the caller supplies the
//! distance and the angles, and no audio device is opened.
//!
//! The geometric spreading model loses `20 * rolloff * log10(distance / reference)` dB, which gives the familiar
//! 6 dB per doubling at roll-off 1.0. Directivity works on the angle between the source's forward axis and the
//! direction to the listener, in radians (0 is straight ahead, pi is directly behind).

use std::f64::consts::PI;

use crate::event_envelope::Envelope;

/// Geometric spreading: the gain falls as a power of the distance beyond a reference distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometricSpreading {
    /// How strongly distance dissipates the sound. 1.0 halves the amplitude per doubling of distance; 2.0 quarters it;
    /// 0.5 falls by about 0.707 per doubling.
    pub rolloff_factor: f64,
    /// Distance in metres inside which the sound is not attenuated.
    pub reference_distance: f64,
}

impl GeometricSpreading {
    /// Both values must be finite; the roll-off must be at least zero and the reference distance positive.
    pub fn new(rolloff_factor: f64, reference_distance: f64) -> Option<Self> {
        if rolloff_factor.is_finite() && rolloff_factor >= 0.0 && reference_distance.is_finite() && reference_distance > 0.0 {
            Some(Self { rolloff_factor, reference_distance })
        } else {
            None
        }
    }

    /// The linear gain at `distance` metres. Inside the reference distance (or at a negative distance) it is 1.0.
    pub fn gain(&self, distance: f64) -> f64 {
        let d = distance.max(self.reference_distance);
        (self.reference_distance / d).powf(self.rolloff_factor)
    }
}

/// Envelope distance model: the gain is read from an envelope whose input is the distance in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvelopeDistance {
    pub envelope: Envelope,
}

impl EnvelopeDistance {
    /// The gain at `distance`, clamped to 0.0..=1.0 so an envelope cannot boost a sound above its level.
    pub fn gain(&self, distance: f64) -> f64 {
        self.envelope.evaluate(distance).clamp(0.0, 1.0)
    }
}

/// Cardioid directivity: a heart-shaped pattern, full gain straight ahead and silent directly behind.
/// `angle` is in radians from the forward axis. The gain is `0.5 * (1 + cos(angle))`.
pub fn cardioid_gain(angle: f64) -> f64 {
    0.5 * (1.0 + angle.cos())
}

/// Cone directivity: full gain inside the inner cone, a fixed gain outside the outer cone and a straight-line fade
/// between them. Angles are in radians from the forward axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeDirectivity {
    pub inner_angle: f64,
    pub outer_angle: f64,
    /// Gain outside the outer cone, 0.0..=1.0.
    pub outer_gain: f64,
}

impl ConeDirectivity {
    /// Needs `0 <= inner < outer <= pi` and an outer gain in `0..=1`.
    pub fn new(inner_angle: f64, outer_angle: f64, outer_gain: f64) -> Option<Self> {
        let angles_ok = inner_angle.is_finite() && outer_angle.is_finite() && inner_angle >= 0.0 && inner_angle < outer_angle && outer_angle <= PI;
        if angles_ok && (0.0..=1.0).contains(&outer_gain) {
            Some(Self { inner_angle, outer_angle, outer_gain })
        } else {
            None
        }
    }

    /// The gain for a listener at `angle` radians from the forward axis.
    pub fn gain(&self, angle: f64) -> f64 {
        let a = angle.abs();
        if a <= self.inner_angle {
            1.0
        } else if a >= self.outer_angle {
            self.outer_gain
        } else {
            let t = (a - self.inner_angle) / (self.outer_angle - self.inner_angle);
            1.0 + (self.outer_gain - 1.0) * t
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_envelope::{CurveType, EnvelopeSegment};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn geometric_spreading_halves_per_doubling_at_roll_off_one() {
        let g = GeometricSpreading::new(1.0, 1.0).unwrap();
        assert!(close(g.gain(1.0), 1.0));
        assert!(close(g.gain(2.0), 0.5), "6 dB per doubling");
        assert!(close(g.gain(4.0), 0.25));
        assert!(close(g.gain(0.2), 1.0), "inside the reference distance nothing is lost");
    }

    #[test]
    fn roll_off_scales_the_loss_as_its_power() {
        // Roll-off 2 loses twice the decibels: a quarter at a doubling. Roll-off 0.5 halves the decibels.
        let steep = GeometricSpreading::new(2.0, 1.0).unwrap();
        let soft = GeometricSpreading::new(0.5, 1.0).unwrap();
        assert!(close(steep.gain(2.0), 0.25));
        assert!(close(soft.gain(4.0), 0.5), "(1/4) to the power 0.5");
    }

    #[test]
    fn bad_spreading_parameters_are_refused() {
        assert!(GeometricSpreading::new(-1.0, 1.0).is_none());
        assert!(GeometricSpreading::new(1.0, 0.0).is_none());
        assert!(GeometricSpreading::new(f64::NAN, 1.0).is_none());
    }

    #[test]
    fn envelope_distance_reads_the_envelope_and_clamps_it() {
        // Full volume to 1 m, then a linear fade to silence at 11 m.
        let env = Envelope::new(
            (0.0, 1.0),
            vec![
                EnvelopeSegment { end_x: 1.0, end_y: 1.0, curve: CurveType::HoldStartValue },
                EnvelopeSegment { end_x: 11.0, end_y: 0.0, curve: CurveType::Linear },
            ],
        )
        .unwrap();
        let m = EnvelopeDistance { envelope: env };
        assert!(close(m.gain(0.5), 1.0));
        assert!(close(m.gain(6.0), 0.5), "half way down the fade");
        assert!(close(m.gain(20.0), 0.0));
    }

    #[test]
    fn cardioid_is_full_ahead_half_at_the_side_and_silent_behind() {
        assert!(close(cardioid_gain(0.0), 1.0));
        assert!(close(cardioid_gain(PI / 2.0), 0.5));
        assert!(close(cardioid_gain(PI), 0.0));
    }

    #[test]
    fn cone_holds_inside_fades_between_and_clamps_outside() {
        let c = ConeDirectivity::new(1.0, 2.0, 0.2).unwrap();
        assert!(close(c.gain(0.5), 1.0));
        assert!(close(c.gain(-0.5), 1.0), "the pattern is symmetric about the axis");
        assert!(close(c.gain(1.5), 0.6), "1 + (0.2 - 1) * 0.5");
        assert!(close(c.gain(2.5), 0.2));
    }

    #[test]
    fn cone_angles_must_be_ordered_and_within_a_half_turn() {
        assert!(ConeDirectivity::new(2.0, 1.0, 0.5).is_none());
        assert!(ConeDirectivity::new(1.0, 1.0, 0.5).is_none());
        assert!(ConeDirectivity::new(0.0, PI + 0.1, 0.5).is_none());
        assert!(ConeDirectivity::new(0.0, PI, 1.5).is_none());
    }
}
