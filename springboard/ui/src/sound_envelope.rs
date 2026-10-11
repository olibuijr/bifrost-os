//! Envelopes: a curve over a linear input (seconds, distance or a parameter value) built from segments. Each segment
//! runs from the previous point to its end point along one `CurveType`. Sound uses them to shape volume over a
//! distance, to fade a blend between child sounds and to change a parameter while a sound plays. Pure Rust, no audio
//! device.
//!
//! Curve formulas are this module's own choice: the reference framework names the curve types but does not publish
//! their maths. Every curve maps 0.0..=1.0 onto 0.0..=1.0. The inverse of a curve is `1 - f(1 - t)`, the mirror image
//! of the forward curve.

use std::f64::consts::FRAC_PI_2;

/// The curve a segment follows (`CurveType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveType {
    Linear,
    Squared,
    InverseSquared,
    Cubed,
    InverseCubed,
    Sine,
    InverseSine,
    /// Smoothstep: starts and ends flat, rises through the middle.
    Sigmoid,
    InverseSigmoid,
    /// Stays at the segment's start value for its whole length.
    HoldStartValue,
    /// Stays at the segment's end value for its whole length.
    JumpToEndValue,
}

impl CurveType {
    /// The fraction of the way through the segment (`t` in 0.0..=1.0) that the curve has reached.
    pub fn shape(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Squared => t * t,
            Self::InverseSquared => 1.0 - (1.0 - t) * (1.0 - t),
            Self::Cubed => t * t * t,
            Self::InverseCubed => 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t),
            Self::Sine => (t * FRAC_PI_2).sin(),
            Self::InverseSine => 1.0 - ((1.0 - t) * FRAC_PI_2).sin(),
            Self::Sigmoid => t * t * (3.0 - 2.0 * t),
            Self::InverseSigmoid => {
                let u = 1.0 - t;
                1.0 - u * u * (3.0 - 2.0 * u)
            }
            Self::HoldStartValue => 0.0,
            Self::JumpToEndValue => 1.0,
        }
    }
}

/// One segment: it ends at (`end_x`, `end_y`) and follows `curve` from the previous point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeSegment {
    pub end_x: f64,
    pub end_y: f64,
    pub curve: CurveType,
}

/// Why an envelope cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    /// No segment, so there is no curve.
    Empty,
    /// A value is not finite, or a segment does not move forward on the input axis.
    InvalidPoint,
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("the envelope has no segments"),
            Self::InvalidPoint => f.write_str("an envelope point is not finite or does not advance the input"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// A curve over a linear input. Inputs before the start clamp to the start value and inputs past the last segment
/// clamp to its end value.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub start_x: f64,
    pub start_y: f64,
    pub segments: Vec<EnvelopeSegment>,
}

impl Envelope {
    /// Builds an envelope. Every value must be finite, and each segment's `end_x` must be greater than the one before
    /// it (the first must be greater than `start_x`).
    pub fn new(start: (f64, f64), segments: Vec<EnvelopeSegment>) -> Result<Self, EnvelopeError> {
        if segments.is_empty() {
            return Err(EnvelopeError::Empty);
        }
        let (start_x, start_y) = start;
        if !start_x.is_finite() || !start_y.is_finite() {
            return Err(EnvelopeError::InvalidPoint);
        }
        let mut last_x = start_x;
        for s in &segments {
            if !s.end_x.is_finite() || !s.end_y.is_finite() || s.end_x <= last_x {
                return Err(EnvelopeError::InvalidPoint);
            }
            last_x = s.end_x;
        }
        Ok(Self { start_x, start_y, segments })
    }

    /// The input range the envelope covers: from the start point to the last segment's end.
    pub fn domain(&self) -> (f64, f64) {
        let last = self.segments.last().map(|s| s.end_x).unwrap_or(self.start_x);
        (self.start_x, last)
    }

    /// The smallest and largest output value over the envelope's points.
    pub fn range(&self) -> (f64, f64) {
        let mut lo = self.start_y;
        let mut hi = self.start_y;
        for s in &self.segments {
            lo = lo.min(s.end_y);
            hi = hi.max(s.end_y);
        }
        (lo, hi)
    }

    /// The output value at input `x`.
    pub fn evaluate(&self, x: f64) -> f64 {
        if x <= self.start_x {
            return self.start_y;
        }
        let (mut px, mut py) = (self.start_x, self.start_y);
        for s in &self.segments {
            if x <= s.end_x {
                let t = (x - px) / (s.end_x - px);
                return py + (s.end_y - py) * s.curve.shape(t);
            }
            px = s.end_x;
            py = s.end_y;
        }
        py
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(end_x: f64, end_y: f64, curve: CurveType) -> EnvelopeSegment {
        EnvelopeSegment { end_x, end_y, curve }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn linear_ramp_and_clamping_outside_the_domain() {
        let e = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Linear)]).unwrap();
        assert!(close(e.evaluate(0.5), 0.5));
        assert!(close(e.evaluate(-3.0), 0.0), "before the start the start value holds");
        assert!(close(e.evaluate(2.0), 1.0), "past the last segment the end value holds");
        assert_eq!(e.domain(), (0.0, 1.0));
        assert_eq!(e.range(), (0.0, 1.0));
    }

    #[test]
    fn power_curves_at_half_way() {
        let sq = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Squared)]).unwrap();
        let cu = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Cubed)]).unwrap();
        let isq = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::InverseSquared)]).unwrap();
        assert!(close(sq.evaluate(0.5), 0.25));
        assert!(close(cu.evaluate(0.5), 0.125));
        assert!(close(isq.evaluate(0.5), 0.75), "the inverse of a square is 1 - (1 - t)^2");
    }

    #[test]
    fn sigmoid_is_flat_at_both_ends_and_half_way_at_the_middle() {
        let e = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Sigmoid)]).unwrap();
        assert!(close(e.evaluate(0.5), 0.5));
        assert!(close(e.evaluate(0.25), 0.15625), "3 * 0.0625 - 2 * 0.015625");
    }

    #[test]
    fn sine_matches_its_closed_form() {
        let e = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Sine)]).unwrap();
        assert!(close(e.evaluate(0.5), (std::f64::consts::FRAC_PI_4).sin()));
        let inv = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::InverseSine)]).unwrap();
        assert!(close(inv.evaluate(0.5), 1.0 - (std::f64::consts::FRAC_PI_4).sin()));
    }

    #[test]
    fn hold_and_jump_curves_ignore_the_position_in_the_segment() {
        let hold = Envelope::new((0.0, 2.0), vec![seg(1.0, 5.0, CurveType::HoldStartValue)]).unwrap();
        let jump = Envelope::new((0.0, 2.0), vec![seg(1.0, 5.0, CurveType::JumpToEndValue)]).unwrap();
        assert!(close(hold.evaluate(0.5), 2.0));
        assert!(close(jump.evaluate(0.5), 5.0));
        assert_eq!(hold.range(), (2.0, 5.0));
    }

    #[test]
    fn two_segments_meet_at_the_shared_point() {
        // Up from (0,0) to (1,1), then linearly down to (3,0). At x = 2 the second segment is half way.
        let e = Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Linear), seg(3.0, 0.0, CurveType::Linear)]).unwrap();
        assert!(close(e.evaluate(2.0), 0.5));
        assert!(close(e.evaluate(1.0), 1.0));
        assert_eq!(e.domain(), (0.0, 3.0));
        assert_eq!(e.range(), (0.0, 1.0));
    }

    #[test]
    fn bad_envelopes_are_refused() {
        assert_eq!(Envelope::new((0.0, 0.0), Vec::new()).unwrap_err(), EnvelopeError::Empty);
        assert_eq!(
            Envelope::new((0.0, 0.0), vec![seg(1.0, 1.0, CurveType::Linear), seg(1.0, 0.0, CurveType::Linear)]).unwrap_err(),
            EnvelopeError::InvalidPoint,
            "the input must strictly advance"
        );
        assert_eq!(
            Envelope::new((0.0, f64::NAN), vec![seg(1.0, 1.0, CurveType::Linear)]).unwrap_err(),
            EnvelopeError::InvalidPoint
        );
    }
}
