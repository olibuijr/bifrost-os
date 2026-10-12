//! Breakpoint envelopes for audio parameters over a linear input (time, distance or a game value).
//! An envelope starts at a point and moves through segments; each segment reaches its target
//! value over its length along one curve. Pure model: no clock and no audio output. The caller
//! feeds the input and applies the output to a gain, filter or other parameter.
use std::f32::consts::{FRAC_PI_2, PI};

/// Shape of one segment. `shape` maps a position `t` in 0..=1 along the segment to a
/// fraction in 0..=1 of the way from the start value to the target value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Squared,
    InverseSquared,
    Cubed,
    InverseCubed,
    Sine,
    InverseSine,
    /// Keeps the start value for the whole segment.
    HoldStart,
    /// Takes the target value for the whole segment.
    JumpToEnd,
}

impl Curve {
    /// Fraction of the way from start to target at position `t` (clamped to 0..=1).
    pub fn shape(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::Squared => t * t,
            Curve::InverseSquared => t.sqrt(),
            Curve::Cubed => t * t * t,
            Curve::InverseCubed => t.cbrt(),
            Curve::Sine => (t * FRAC_PI_2).sin(),
            Curve::InverseSine => t.asin() * 2.0 / PI,
            Curve::HoldStart => 0.0,
            Curve::JumpToEnd => 1.0,
        }
    }
}

/// One step of an envelope: how far along the input it runs, where it ends and how it gets there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// Length along the input. Zero or more; a zero-length segment jumps to its target.
    pub length: f32,
    /// Output value at the end of the segment.
    pub target: f32,
    pub curve: Curve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    start_x: f32,
    start_y: f32,
    segments: Vec<Segment>,
}

impl Envelope {
    /// Builds an envelope from a start point `(input, output)` and its segments.
    /// Returns None when a start value is not finite, or a segment length is negative or not finite,
    /// or a target is not finite.
    pub fn new(start: (f32, f32), segments: Vec<Segment>) -> Option<Self> {
        if !start.0.is_finite() || !start.1.is_finite() {
            return None;
        }
        for segment in &segments {
            if !segment.length.is_finite() || segment.length < 0.0 || !segment.target.is_finite() {
                return None;
            }
        }
        Some(Self { start_x: start.0, start_y: start.1, segments })
    }

    /// The start point `(input, output)`.
    pub fn start_point(&self) -> (f32, f32) {
        (self.start_x, self.start_y)
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Input range covered: from the start point to the end of the last segment.
    pub fn domain(&self) -> (f32, f32) {
        let total: f32 = self.segments.iter().map(|segment| segment.length).sum();
        (self.start_x, self.start_x + total)
    }

    /// Smallest and largest output over the start value and every segment target.
    pub fn range(&self) -> (f32, f32) {
        let mut low = self.start_y;
        let mut high = self.start_y;
        for segment in &self.segments {
            low = low.min(segment.target);
            high = high.max(segment.target);
        }
        (low, high)
    }

    /// Output at input `x`. Input outside the domain is clamped to it, so the value before the
    /// start is the start value and the value after the end is the last target.
    pub fn evaluate(&self, x: f32) -> f32 {
        let (low, high) = self.domain();
        let x = x.clamp(low, high);
        let mut segment_x = self.start_x;
        let mut segment_y = self.start_y;
        for segment in &self.segments {
            let end_x = segment_x + segment.length;
            if x <= end_x {
                if segment.length <= 0.0 {
                    return segment.target;
                }
                let t = (x - segment_x) / segment.length;
                return segment_y + (segment.target - segment_y) * segment.curve.shape(t);
            }
            segment_x = end_x;
            segment_y = segment.target;
        }
        segment_y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn seg(length: f32, target: f32, curve: Curve) -> Segment {
        Segment { length, target, curve }
    }

    #[test]
    fn linear_segment_is_proportional_to_input() {
        // start (0, 0), length 2, target 10: at x = 1 the output is halfway, 5.
        let env = Envelope::new((0.0, 0.0), vec![seg(2.0, 10.0, Curve::Linear)]).unwrap();
        assert!(close(env.evaluate(1.0), 5.0));
        assert!(close(env.evaluate(0.5), 2.5));
    }

    #[test]
    fn squared_curve_bends_below_the_line() {
        // t = 0.5 gives shape 0.25, so output is 0 + 10 * 0.25 = 2.5 at x = 1.
        let env = Envelope::new((0.0, 0.0), vec![seg(2.0, 10.0, Curve::Squared)]).unwrap();
        assert!(close(env.evaluate(1.0), 2.5));
    }

    #[test]
    fn inverse_sine_at_half_is_one_third() {
        // asin(0.5) = pi/6, and (pi/6) * 2 / pi = 1/3. Start 0, target 3, so output 1.
        let env = Envelope::new((0.0, 0.0), vec![seg(1.0, 3.0, Curve::InverseSine)]).unwrap();
        assert!(close(env.evaluate(0.5), 1.0));
    }

    #[test]
    fn two_segments_join_at_the_first_target() {
        // start (0, 1); segment 1 runs 0..1 to 3; segment 2 runs 1..2 to 1.
        // At x = 1 the value is 3. At x = 1.5 it is halfway from 3 down to 1, which is 2.
        let env = Envelope::new(
            (0.0, 1.0),
            vec![seg(1.0, 3.0, Curve::Linear), seg(1.0, 1.0, Curve::Linear)],
        )
        .unwrap();
        assert!(close(env.evaluate(1.0), 3.0));
        assert!(close(env.evaluate(1.5), 2.0));
        assert!(close(env.evaluate(2.0), 1.0));
    }

    #[test]
    fn hold_and_jump_curves_ignore_the_slope() {
        let hold = Envelope::new((0.0, 4.0), vec![seg(2.0, 9.0, Curve::HoldStart)]).unwrap();
        assert!(close(hold.evaluate(1.5), 4.0));
        let jump = Envelope::new((0.0, 4.0), vec![seg(2.0, 9.0, Curve::JumpToEnd)]).unwrap();
        assert!(close(jump.evaluate(0.1), 9.0));
    }

    #[test]
    fn input_outside_the_domain_is_clamped() {
        let env = Envelope::new((10.0, 2.0), vec![seg(5.0, -4.0, Curve::Linear)]).unwrap();
        assert!(close(env.evaluate(0.0), 2.0));
        assert!(close(env.evaluate(100.0), -4.0));
        assert_eq!(env.domain(), (10.0, 15.0));
    }

    #[test]
    fn range_covers_start_and_every_target() {
        let env = Envelope::new(
            (0.0, 0.0),
            vec![seg(1.0, 5.0, Curve::Linear), seg(1.0, -2.0, Curve::Linear)],
        )
        .unwrap();
        assert_eq!(env.range(), (-2.0, 5.0));
    }

    #[test]
    fn zero_length_segment_jumps_to_its_target() {
        let env = Envelope::new((0.0, 0.0), vec![seg(0.0, 7.0, Curve::Linear)]).unwrap();
        assert!(close(env.evaluate(0.0), 7.0));
    }

    #[test]
    fn invalid_segments_are_rejected() {
        assert!(Envelope::new((0.0, 0.0), vec![seg(-1.0, 1.0, Curve::Linear)]).is_none());
        assert!(Envelope::new((0.0, 0.0), vec![seg(1.0, f32::NAN, Curve::Linear)]).is_none());
        assert!(Envelope::new((f32::INFINITY, 0.0), Vec::new()).is_none());
    }
}
