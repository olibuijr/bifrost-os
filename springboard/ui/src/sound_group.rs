//! Sound groups and ducking. A group scales every sound in it by one gain. A ducker lowers a group's gain while a
//! trigger level is above a threshold, moving toward the duck depth with an attack time and back to unity with a
//! release time, both as one-pole smoothing. Pure gain math: the caller supplies the trigger level and the block
//! length in seconds, so the result is deterministic.

/// A set of sounds that share one gain, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    gain: f32,
}

impl Group {
    /// Returns None unless `gain` is finite and in 0..=1.
    pub fn new(gain: f32) -> Option<Self> {
        (gain.is_finite() && (0.0..=1.0).contains(&gain)).then_some(Self { gain })
    }

    /// The group's gain.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Gain of one sound in the group after the group gain. A negative sound gain counts as zero.
    pub fn apply(&self, sound_gain: f32) -> f32 {
        sound_gain.max(0.0) * self.gain
    }
}

/// Lowers a group's gain while a trigger is loud. Starts at unity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ducker {
    threshold: f32,
    depth: f32,
    attack: f32,
    release: f32,
    gain: f32,
}

impl Ducker {
    /// `threshold` is the linear trigger level above which the ducker ducks (at least 0). `depth` is the gain while
    /// ducked (0 to 1). `attack` and `release` are the smoothing times in seconds for going down and coming back up
    /// (at least 0; 0 means instant). Returns None for any value that is not finite or is out of range.
    pub fn new(threshold: f32, depth: f32, attack: f32, release: f32) -> Option<Self> {
        let finite = threshold.is_finite() && depth.is_finite() && attack.is_finite() && release.is_finite();
        let valid = finite
            && threshold >= 0.0
            && (0.0..=1.0).contains(&depth)
            && attack >= 0.0
            && release >= 0.0;
        valid.then_some(Self { threshold, depth, attack, release, gain: 1.0 })
    }

    /// The current gain of the ducked group.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Advances by `dt` seconds with the trigger's `level` for that block, and returns the new gain.
    /// The target is the duck depth while `level` is above the threshold and 1 otherwise. Moving down uses the
    /// attack time and moving up uses the release time: gain = target + (gain - target) * exp(-dt / time).
    /// A negative or non-finite `dt` counts as zero, which leaves the gain where it is.
    pub fn step(&mut self, level: f32, dt: f32) -> f32 {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let target = if level > self.threshold { self.depth } else { 1.0 };
        let time = if target < self.gain { self.attack } else { self.release };
        let keep = if time <= 0.0 { 0.0 } else { (-dt / time).exp() };
        self.gain = target + (self.gain - target) * keep;
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn group_gain_multiplies_sound_gain() {
        let g = Group::new(0.8).unwrap();
        assert!(close(g.apply(0.5), 0.4, 1e-6));
        assert!(close(g.apply(-1.0), 0.0, 1e-9));
        assert!(Group::new(1.2).is_none());
        assert!(Group::new(f32::NAN).is_none());
    }

    #[test]
    fn quiet_trigger_keeps_unity_gain() {
        let mut d = Ducker::new(0.5, 0.25, 0.01, 0.1).unwrap();
        assert!(close(d.step(0.1, 0.01), 1.0, 1e-6));
        assert!(close(d.step(0.5, 0.01), 1.0, 1e-6));
    }

    #[test]
    fn loud_trigger_ducks_with_the_attack_time() {
        // Attack 0.01 s, block 0.01 s: exp(-1) = 0.367879. gain = 0.25 + (1 - 0.25) * 0.367879 = 0.525909.
        let mut d = Ducker::new(0.5, 0.25, 0.01, 0.1).unwrap();
        assert!(close(d.step(0.9, 0.01), 0.525909, 1e-4));
        // Second block: 0.25 + (0.525909 - 0.25) * 0.367879 = 0.351493.
        assert!(close(d.step(0.9, 0.01), 0.351493, 1e-4));
    }

    #[test]
    fn released_trigger_returns_with_the_release_time() {
        // Release 0.1 s, block 0.1 s: gain = 1 + (0.25 - 1) * 0.367879 = 0.724091.
        let mut d = Ducker::new(0.5, 0.25, 0.01, 0.1).unwrap();
        d.step(0.9, 1.0);
        assert!(close(d.gain(), 0.25, 1e-4));
        assert!(close(d.step(0.0, 0.1), 0.724091, 1e-4));
    }

    #[test]
    fn zero_time_and_zero_step_behave_exactly() {
        let mut instant = Ducker::new(0.5, 0.25, 0.0, 0.0).unwrap();
        assert!(close(instant.step(0.9, 0.001), 0.25, 1e-6));
        assert!(close(instant.step(0.0, 0.001), 1.0, 1e-6));
        let mut held = Ducker::new(0.5, 0.25, 0.01, 0.1).unwrap();
        assert!(close(held.step(0.9, 0.0), 1.0, 1e-6));
    }

    #[test]
    fn rejects_bad_ducker_settings() {
        assert!(Ducker::new(0.5, 1.5, 0.01, 0.1).is_none());
        assert!(Ducker::new(-0.1, 0.25, 0.01, 0.1).is_none());
        assert!(Ducker::new(0.5, 0.25, -0.01, 0.1).is_none());
    }
}
