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
