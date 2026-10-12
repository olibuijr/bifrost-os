//! Keyframed values over time (`NativeAnimatedValue`, `NativeAnimatedScalar`, `NativeAnimatedVector3`,
//! `NativeAnimatedQuaternion`). A track holds samples sorted by time in seconds. Scalars and vectors are blended
//! linearly between samples, and rotations are blended with `Quaternion::slerp`. A time before the first sample or
//! after the last sample gives the nearest sample. A track is data only: a renderer samples it each frame.

use crate::linalg::{Quaternion, Vector3};

/// A value that can be blended between two keyframes.
pub trait Interpolate: Copy {
    /// `t` is the position between the two keys, in 0..=1.
    fn interpolate(from: Self, to: Self, t: f32) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(from: f32, to: f32, t: f32) -> f32 {
        from + (to - from) * t
    }
}

impl Interpolate for Vector3 {
    fn interpolate(from: Vector3, to: Vector3, t: f32) -> Vector3 {
        from + (to - from) * t
    }
}

impl Interpolate for Quaternion {
    fn interpolate(from: Quaternion, to: Quaternion, t: f32) -> Quaternion {
        Quaternion::slerp(from, to, t)
    }
}

/// A sampled track of values keyed by time in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeTrack<T: Interpolate> {
    samples: Vec<(f64, T)>,
}

/// A track of scalar values (`NativeAnimatedScalar`).
pub type AnimatedScalar = KeyframeTrack<f32>;
/// A track of three-component vectors (`NativeAnimatedVector3`).
pub type AnimatedVector3 = KeyframeTrack<Vector3>;
/// A track of rotations (`NativeAnimatedQuaternion`).
pub type AnimatedQuaternion = KeyframeTrack<Quaternion>;

impl<T: Interpolate> KeyframeTrack<T> {
    pub fn new() -> Self {
        KeyframeTrack { samples: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Sets the value at `time`, replacing a sample already there. `false` when the time is not finite.
    pub fn set(&mut self, time: f64, value: T) -> bool {
        if !time.is_finite() {
            return false;
        }
        match self.samples.binary_search_by(|s| s.0.total_cmp(&time)) {
            Ok(i) => self.samples[i].1 = value,
            Err(i) => self.samples.insert(i, (time, value)),
        }
        true
    }

    /// The value at `time`. `None` when the track is empty or the time is not finite.
    pub fn value_at(&self, time: f64) -> Option<T> {
        if !time.is_finite() {
            return None;
        }
        let first = *self.samples.first()?;
        let last = *self.samples.last()?;
        if time <= first.0 {
            return Some(first.1);
        }
        if time >= last.0 {
            return Some(last.1);
        }
        match self.samples.binary_search_by(|s| s.0.total_cmp(&time)) {
            Ok(i) => Some(self.samples[i].1),
            Err(i) => {
                let (t0, v0) = self.samples[i - 1];
                let (t1, v1) = self.samples[i];
                let u = ((time - t0) / (t1 - t0)) as f32;
                Some(T::interpolate(v0, v1, u))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn scalar_is_linear_between_keys() {
        let mut t = AnimatedScalar::new();
        assert!(t.set(0.0, 0.0));
        assert!(t.set(2.0, 10.0));
        assert!(close(t.value_at(1.0).unwrap(), 5.0));
        assert!(close(t.value_at(0.5).unwrap(), 2.5));
    }

    #[test]
    fn times_outside_the_track_hold_the_end_values() {
        let mut t = AnimatedScalar::new();
        t.set(1.0, 4.0);
        t.set(3.0, 8.0);
        assert!(close(t.value_at(-3.0).unwrap(), 4.0));
        assert!(close(t.value_at(9.0).unwrap(), 8.0));
        assert!(close(t.value_at(3.0).unwrap(), 8.0));
    }

    #[test]
    fn keys_sort_on_insert_and_replace_at_the_same_time() {
        let mut t = AnimatedScalar::new();
        t.set(2.0, 4.0);
        t.set(0.0, 0.0);
        t.set(1.0, 2.0);
        assert_eq!(t.len(), 3);
        assert!(close(t.value_at(1.5).unwrap(), 3.0));
        t.set(1.0, 6.0);
        assert_eq!(t.len(), 3);
        assert!(close(t.value_at(1.5).unwrap(), 5.0));
    }

    #[test]
    fn vector3_blends_each_component() {
        let mut t = AnimatedVector3::new();
        t.set(0.0, Vector3::ZERO);
        t.set(1.0, Vector3::new(2.0, 4.0, 6.0));
        let v = t.value_at(0.25).unwrap();
        assert!(close(v.x, 0.5) && close(v.y, 1.0) && close(v.z, 1.5));
    }

    #[test]
    fn quaternion_blends_on_the_short_arc() {
        // From no turn to a 90 degree turn about z. At the halfway time the turn is 45 degrees about z.
        let mut t = AnimatedQuaternion::new();
        let z = Vector3::new(0.0, 0.0, 1.0);
        t.set(0.0, Quaternion::identity());
        t.set(1.0, Quaternion::from_axis_angle(z, PI / 2.0));
        let q = t.value_at(0.5).unwrap();
        let expect = Quaternion::from_axis_angle(z, PI / 4.0);
        assert!(close(q.x, expect.x) && close(q.y, expect.y));
        assert!(close(q.z, expect.z) && close(q.w, expect.w));
    }

    #[test]
    fn empty_track_and_bad_times_give_nothing() {
        let mut t = AnimatedScalar::new();
        assert!(t.is_empty());
        assert_eq!(t.value_at(0.0), None);
        assert!(!t.set(f64::NAN, 1.0));
        assert!(!t.set(f64::INFINITY, 1.0));
        assert!(t.is_empty());
        t.set(0.0, 1.0);
        assert_eq!(t.value_at(f64::NAN), None);
    }
}
