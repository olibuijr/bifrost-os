//! Motion cues: a ring of dots at the edge of the screen that shows the sustained motion of a moving vehicle, so the eyes
//! have a fixed reference while the inner ear feels the motion. The model low-passes the acceleration the caller measures
//! (gravity already removed), so a short jolt fades out and a steady push lights the dots on the side it pushes toward.
//! Pure: the caller supplies the samples and the time step, and draws the dots it gets back. Nothing is drawn while the
//! cues are off.

use std::f32::consts::TAU;

/// Dots on the ring.
pub const DOT_COUNT: usize = 12;
/// Ring radius in screen units: the dots sit at this distance from the centre, on a square of -1 to 1.
pub const RING_RADIUS: f32 = 0.9;
/// Time constant of the low-pass filter, in seconds. Acceleration that lasts well under this is mostly removed.
pub const FILTER_TIME_CONSTANT: f32 = 0.5;
/// Below this filtered acceleration, in m/s², no dot shows.
pub const CUE_THRESHOLD: f32 = 0.05;
/// At this filtered acceleration, in m/s², the dots on the side of the push are fully opaque.
pub const CUE_FULL: f32 = 0.3;

/// One dot of the ring. `x` and `y` are in screen units, `opacity` from 0 (hidden) to 1 (full).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
}

/// The cue state: whether cues are on, and the filtered acceleration in the device's x and y axes.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionCues {
    enabled: bool,
    filtered: (f32, f32),
}

impl MotionCues {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, filtered: (0.0, 0.0) }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turns the cues on or off. Turning them off forgets the filtered motion, so they start from rest when turned on.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.filtered = (0.0, 0.0);
        }
    }

    /// The filtered acceleration, in m/s², as (x, y).
    pub fn filtered(&self) -> (f32, f32) {
        self.filtered
    }

    /// Feeds one measurement: the acceleration in m/s² (x, y) and the time since the last one, in seconds.
    /// A time step that is not positive, or not finite, is ignored.
    pub fn sample(&mut self, accel: (f32, f32), dt: f32) {
        if !dt.is_finite() || dt <= 0.0 || !accel.0.is_finite() || !accel.1.is_finite() {
            return;
        }
        let alpha = dt / (FILTER_TIME_CONSTANT + dt);
        self.filtered.0 += alpha * (accel.0 - self.filtered.0);
        self.filtered.1 += alpha * (accel.1 - self.filtered.1);
    }

    /// The dots to draw now. Empty while the cues are off. Each dot's opacity follows how hard the motion pushes
    /// toward its place on the ring: a dot on the far side of the push stays hidden.
    pub fn dots(&self) -> Vec<Dot> {
        if !self.enabled {
            return Vec::new();
        }
        let (fx, fy) = self.filtered;
        let magnitude = fx.hypot(fy);
        let strength = ((magnitude - CUE_THRESHOLD) / (CUE_FULL - CUE_THRESHOLD)).clamp(0.0, 1.0);
        let direction = fy.atan2(fx);
        (0..DOT_COUNT)
            .map(|i| {
                let angle = TAU * i as f32 / DOT_COUNT as f32;
                let weight = (angle - direction).cos().max(0.0);
                Dot {
                    x: RING_RADIUS * angle.cos(),
                    y: RING_RADIUS * angle.sin(),
                    opacity: strength * weight,
                }
            })
            .collect()
    }
}

/// Standard gravity in m/s², the factor that turns the accelerometer's readings in g into m/s².
pub const STANDARD_GRAVITY: f32 = 9.80665;

/// Takes gravity out of the accelerometer, so the cues see only the motion of the vehicle. The constant part of the
/// reading is a slow low-pass of the readings (gravity), and each sample's motion is its difference from that estimate.
/// The estimate follows a long push too, after about `time_constant` seconds, so a sustained turn fades from the cues
/// the way the eye stops noticing a steady ride.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GravityEstimate {
    time_constant: f32,
    value: Option<[f32; 3]>,
}

impl GravityEstimate {
    /// An estimate with the given time constant in seconds. `None` unless it is finite and above zero.
    pub fn new(time_constant: f32) -> Option<Self> {
        if time_constant.is_finite() && time_constant > 0.0 {
            Some(GravityEstimate { time_constant, value: None })
        } else {
            None
        }
    }

    /// Adds one reading in g (x, y, z), taken `dt` seconds after the last. Returns the motion on x and y in m/s², with
    /// the gravity estimate from before this reading removed. The first reading only sets the estimate, so it returns
    /// zero. A reading that is not finite, or a `dt` that is not positive, returns `None` and changes nothing.
    pub fn linear(&mut self, sample_g: [f32; 3], dt: f32) -> Option<(f32, f32)> {
        if !dt.is_finite() || dt <= 0.0 || sample_g.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let (residual, next) = match self.value {
            None => ([0.0; 3], sample_g),
            Some(prev) => {
                let alpha = dt / (self.time_constant + dt);
                let residual = [sample_g[0] - prev[0], sample_g[1] - prev[1], sample_g[2] - prev[2]];
                let next = [prev[0] + alpha * residual[0], prev[1] + alpha * residual[1], prev[2] + alpha * residual[2]];
                (residual, next)
            }
        };
        self.value = Some(next);
        Some((residual[0] * STANDARD_GRAVITY, residual[1] * STANDARD_GRAVITY))
    }
}

#[cfg(test)]
mod gravity_tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_first_reading_is_taken_as_gravity() {
        let mut g = GravityEstimate::new(2.0).unwrap();
        assert_eq!(g.linear([0.0, 0.0, 1.0], 0.1), Some((0.0, 0.0)));
    }

    #[test]
    fn a_jolt_shows_as_motion_and_a_still_device_shows_none() {
        let mut g = GravityEstimate::new(2.0).unwrap();
        g.linear([0.0, 0.0, 1.0], 0.1);
        let (x, y) = g.linear([0.5, 0.0, 1.0], 0.1).unwrap();
        assert!(near(x, 0.5 * STANDARD_GRAVITY));
        assert!(near(y, 0.0));
        let (x, _) = g.linear([0.5, 0.0, 1.0], 0.1).unwrap();
        assert!(x.abs() < 0.5 * STANDARD_GRAVITY, "the estimate starts to absorb a steady push");
    }

    #[test]
    fn a_sustained_push_is_absorbed_into_gravity() {
        let mut g = GravityEstimate::new(1.0).unwrap();
        g.linear([0.0, 0.0, 1.0], 0.1);
        let mut last = (f32::MAX, f32::MAX);
        for _ in 0..200 {
            last = g.linear([0.2, 0.0, 1.0], 0.1).unwrap();
        }
        assert!(last.0.abs() < 0.01, "after many seconds the steady push has faded: {last:?}");
    }

    #[test]
    fn a_bad_reading_or_time_step_changes_nothing() {
        let mut g = GravityEstimate::new(2.0).unwrap();
        g.linear([0.0, 0.0, 1.0], 0.1);
        let before = g;
        assert_eq!(g.linear([f32::NAN, 0.0, 1.0], 0.1), None);
        assert_eq!(g.linear([0.0, 0.0, 1.0], 0.0), None);
        assert_eq!(g.linear([0.0, 0.0, 1.0], f32::INFINITY), None);
        assert_eq!(g, before);
    }

    #[test]
    fn a_time_constant_must_be_positive_and_finite() {
        assert!(GravityEstimate::new(0.0).is_none());
        assert!(GravityEstimate::new(-1.0).is_none());
        assert!(GravityEstimate::new(f32::NAN).is_none());
        assert!(GravityEstimate::new(2.0).is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(cues: &mut MotionCues, accel: (f32, f32)) {
        // Ten seconds at 60 Hz is far past the filter's time constant.
        for _ in 0..600 {
            cues.sample(accel, 1.0 / 60.0);
        }
    }

    #[test]
    fn nothing_is_drawn_while_the_cues_are_off() {
        let mut cues = MotionCues::new(false);
        settle(&mut cues, (2.0, 0.0));
        assert!(cues.dots().is_empty());
        assert!(!cues.is_enabled());
    }

    #[test]
    fn a_still_device_draws_hidden_dots() {
        let mut cues = MotionCues::new(true);
        settle(&mut cues, (0.0, 0.0));
        let dots = cues.dots();
        assert_eq!(dots.len(), DOT_COUNT);
        assert!(dots.iter().all(|d| d.opacity == 0.0));
    }

    #[test]
    fn a_steady_push_lights_the_dots_on_its_side() {
        let mut cues = MotionCues::new(true);
        settle(&mut cues, (1.0, 0.0));
        let dots = cues.dots();
        // The dot at angle 0 sits on the +x side of the ring and is fully lit.
        let right = dots[0];
        assert!((right.x - RING_RADIUS).abs() < 1e-5);
        assert!((right.opacity - 1.0).abs() < 1e-5);
        // The dot on the far side stays hidden.
        let left = dots[DOT_COUNT / 2];
        assert!(left.opacity.abs() < 1e-5);
    }

    #[test]
    fn a_short_jolt_is_mostly_filtered_out() {
        let mut cues = MotionCues::new(true);
        cues.sample((5.0, 0.0), 0.02);
        // One 20 ms spike of 5 m/s² moves the filtered value by under a tenth of its size.
        assert!(cues.filtered().0 < 0.5);
        for _ in 0..200 {
            cues.sample((0.0, 0.0), 0.02);
        }
        assert!(cues.filtered().0.abs() < 0.01);
    }

    #[test]
    fn a_bad_time_step_or_sample_is_ignored() {
        let mut cues = MotionCues::new(true);
        cues.sample((1.0, 1.0), 0.0);
        cues.sample((1.0, 1.0), -0.1);
        cues.sample((f32::NAN, 1.0), 0.1);
        cues.sample((1.0, 1.0), f32::INFINITY);
        assert_eq!(cues.filtered(), (0.0, 0.0));
    }

    #[test]
    fn turning_the_cues_off_forgets_the_motion() {
        let mut cues = MotionCues::new(true);
        settle(&mut cues, (1.0, 1.0));
        cues.set_enabled(false);
        assert_eq!(cues.filtered(), (0.0, 0.0));
        cues.set_enabled(true);
        assert!(cues.dots().iter().all(|d| d.opacity == 0.0));
    }
}
