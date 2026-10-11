//! Steps and distance from the acceleration the user gives the device: a step detector counts the peaks of the
//! acceleration magnitude, with a refractory interval so one stride counts once, and a distance comes from a stride
//! length. Step times are kept, so the caller can ask how many steps fell in a time range.
//!
//! Input is the magnitude of the user acceleration in g, the gravity-free reading of
//! [`crate::motion::DeviceMotion::user_acceleration`] (its norm). Times are seconds on the caller's clock. The
//! thresholds are a starting point for a hand-carried device, not a calibrated gait model. The module owns no sensor
//! and no clock.

/// A magnitude has to rise above this many g to be a step.
const DEFAULT_THRESHOLD_G: f64 = 0.12;
/// Two steps are at least this far apart, in seconds (at most 240 steps a minute).
const MIN_STEP_INTERVAL_S: f64 = 0.25;
/// After a step, the magnitude has to fall below this share of the threshold before the next step can count.
const REARM_FRACTION: f64 = 0.6;
/// A typical stride in metres, used when the caller gives none that is plausible.
const DEFAULT_STRIDE_M: f64 = 0.75;
const MIN_STRIDE_M: f64 = 0.2;
const MAX_STRIDE_M: f64 = 2.5;

/// Counts steps as rising peaks of the acceleration magnitude above a threshold.
#[derive(Clone, Debug)]
pub struct StepDetector {
    threshold_g: f64,
    min_interval_s: f64,
    armed: bool,
    last_step: Option<f64>,
}

impl StepDetector {
    /// A threshold or interval that is not finite and positive is replaced by the default.
    pub fn new(threshold_g: f64, min_interval_s: f64) -> Self {
        let threshold_g = if threshold_g.is_finite() && threshold_g > 0.0 { threshold_g } else { DEFAULT_THRESHOLD_G };
        let min_interval_s = if min_interval_s.is_finite() && min_interval_s > 0.0 { min_interval_s } else { MIN_STEP_INTERVAL_S };
        StepDetector { threshold_g, min_interval_s, armed: true, last_step: None }
    }

    /// Forgets the last step and re-arms the detector.
    pub fn reset(&mut self) {
        self.armed = true;
        self.last_step = None;
    }

    /// Feeds one magnitude in g taken at time `t`. Returns `true` when this sample is a step.
    pub fn feed(&mut self, t: f64, magnitude_g: f64) -> bool {
        if !t.is_finite() || !magnitude_g.is_finite() {
            return false;
        }
        if magnitude_g < self.threshold_g * REARM_FRACTION {
            self.armed = true;
        }
        if !self.armed || magnitude_g < self.threshold_g {
            return false;
        }
        if let Some(last) = self.last_step {
            if t - last < self.min_interval_s {
                return false;
            }
        }
        self.armed = false;
        self.last_step = Some(t);
        true
    }
}

/// Steps, distance and the times of the steps, from a stream of acceleration magnitudes.
#[derive(Clone, Debug)]
pub struct Pedometer {
    detector: StepDetector,
    stride_m: f64,
    steps: Vec<f64>,
}

impl Pedometer {
    /// `stride_m` is the length of one step in metres. A value outside 0.2 to 2.5 m is replaced by 0.75 m.
    pub fn new(stride_m: f64) -> Self {
        let stride_m = if stride_m.is_finite() && (MIN_STRIDE_M..=MAX_STRIDE_M).contains(&stride_m) {
            stride_m
        } else {
            DEFAULT_STRIDE_M
        };
        Pedometer {
            detector: StepDetector::new(DEFAULT_THRESHOLD_G, MIN_STEP_INTERVAL_S),
            stride_m,
            steps: Vec::new(),
        }
    }

    /// Feeds one magnitude in g at time `t`. Returns `true` when it counted a step.
    pub fn feed(&mut self, t: f64, magnitude_g: f64) -> bool {
        if self.detector.feed(t, magnitude_g) {
            self.steps.push(t);
            true
        } else {
            false
        }
    }

    /// The steps counted since the last reset.
    pub fn steps(&self) -> usize {
        self.steps.len()
    }

    /// The distance walked in metres: the steps times the stride length.
    pub fn distance_m(&self) -> f64 {
        self.steps.len() as f64 * self.stride_m
    }

    /// The steps taken from `from` to `to`, both ends included. Zero when `from` is after `to`.
    pub fn steps_between(&self, from: f64, to: f64) -> usize {
        if !(from <= to) {
            return 0;
        }
        let lo = self.steps.partition_point(|&s| s < from);
        let hi = self.steps.partition_point(|&s| s <= to);
        hi - lo
    }

    /// Clears the count and the step times.
    pub fn reset(&mut self) {
        self.detector.reset();
        self.steps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One acceleration peak at `t`, then the magnitude falls back below the re-arm level.
    fn pulse(p: &mut Pedometer, t: f64) -> bool {
        let counted = p.feed(t, 0.3);
        p.feed(t + 0.05, 0.02);
        counted
    }

    #[test]
    fn each_peak_spaced_by_a_stride_is_one_step() {
        let mut p = Pedometer::new(0.75);
        for &t in &[0.0, 0.5, 1.0] {
            assert!(pulse(&mut p, t));
        }
        assert_eq!(p.steps(), 3);
    }

    #[test]
    fn peaks_inside_the_refractory_interval_do_not_count() {
        // six peaks 0.1 s apart: only the first and the one at 0.3 s clear the 0.25 s interval
        let mut p = Pedometer::new(0.75);
        for k in 0..6 {
            pulse(&mut p, k as f64 * 0.1);
        }
        assert_eq!(p.steps(), 2);
    }

    #[test]
    fn distance_is_the_step_count_times_the_stride() {
        let mut p = Pedometer::new(0.8);
        for &t in &[0.0, 0.5, 1.0, 1.5] {
            pulse(&mut p, t);
        }
        assert_eq!(p.steps(), 4);
        assert!((p.distance_m() - 3.2).abs() < 1e-9);
    }

    #[test]
    fn steps_are_queried_by_time_range() {
        let mut p = Pedometer::new(0.75);
        for &t in &[0.0, 0.5, 1.0] {
            pulse(&mut p, t);
        }
        // the steps at 0.5 and 1.0 fall in [0.4, 1.0], both ends included
        assert_eq!(p.steps_between(0.4, 1.0), 2);
        assert_eq!(p.steps_between(2.0, 3.0), 0);
        assert_eq!(p.steps_between(1.0, 0.0), 0, "a backwards range holds nothing");
        assert_eq!(p.steps_between(f64::NAN, 1.0), 0);
    }

    #[test]
    fn values_that_are_not_finite_or_a_stride_out_of_range_fall_back() {
        let mut p = Pedometer::new(f64::NAN);
        assert!(!p.feed(f64::NAN, 0.3));
        assert!(!p.feed(0.0, f64::NAN));
        pulse(&mut p, 0.0);
        assert_eq!(p.steps(), 1);
        assert!((p.distance_m() - DEFAULT_STRIDE_M).abs() < 1e-9);
        // a stride of 10 m is not a step
        let far = Pedometer::new(10.0);
        assert!((far.stride_m - DEFAULT_STRIDE_M).abs() < 1e-9);
    }

    #[test]
    fn reset_clears_the_count_and_re_arms_the_detector() {
        let mut p = Pedometer::new(0.75);
        pulse(&mut p, 0.0);
        p.reset();
        assert_eq!(p.steps(), 0);
        // a peak at the same time as the old step counts again, since the last step was forgotten
        assert!(pulse(&mut p, 0.0));
        assert_eq!(p.steps_between(0.0, 0.0), 1);
    }
}
