//! Ducking: while a notification or speech is active, other streams drop to `DUCK_GAIN`; the gain ramps by a fixed
//! step per tick so there is no click. Ticks come from the caller (one per quantum batch), so tests need no clock.

pub const DUCK_GAIN: f32 = 0.3;
/// Gain change per tick: attack is faster than release.
pub const ATTACK: f32 = 0.2;
pub const RELEASE: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Notification,
    Speech,
}

#[derive(Clone, Debug)]
pub struct Ducker {
    active: [bool; 2],
    gain: f32,
}

impl Default for Ducker {
    fn default() -> Self {
        Ducker { active: [false; 2], gain: 1.0 }
    }
}

impl Ducker {
    pub fn set(&mut self, r: Reason, on: bool) {
        self.active[r as usize] = on;
    }

    pub fn ducking(&self) -> bool {
        self.active.iter().any(|a| *a)
    }

    /// The gain to apply to the other streams.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Advances one tick and returns the new gain.
    pub fn tick(&mut self) -> f32 {
        if self.ducking() {
            self.gain = (self.gain - ATTACK).max(DUCK_GAIN);
        } else {
            self.gain = (self.gain + RELEASE).min(1.0);
        }
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_down_to_floor_and_back() {
        let mut d = Ducker::default();
        assert_eq!(d.tick(), 1.0);
        d.set(Reason::Speech, true);
        assert!((d.tick() - 0.8).abs() < 1e-6);
        for _ in 0..10 {
            d.tick();
        }
        assert_eq!(d.gain(), DUCK_GAIN);
        d.set(Reason::Notification, true);
        d.set(Reason::Speech, false);
        assert!(d.ducking());
        d.set(Reason::Notification, false);
        assert!((d.tick() - 0.35).abs() < 1e-6);
        for _ in 0..20 {
            d.tick();
        }
        assert_eq!(d.gain(), 1.0);
    }

    #[test]
    fn ramps_are_monotonic_and_stay_in_range() {
        let mut d = Ducker::default();
        d.set(Reason::Notification, true);
        let mut last = d.gain();
        for _ in 0..10 {
            let g = d.tick();
            assert!(g <= last && g >= DUCK_GAIN);
            last = g;
        }
        d.set(Reason::Notification, false);
        let mut ticks = 0;
        while d.gain() < 1.0 {
            let g = d.tick();
            assert!(g >= last && g <= 1.0);
            last = g;
            ticks += 1;
            assert!(ticks < 100, "release never finished");
        }
        // (1.0 - 0.3) / 0.05 = 14 ticks, allowing one for float rounding
        assert!((14..=15).contains(&ticks), "{ticks}");
    }

    #[test]
    fn ducking_again_mid_release_turns_around_at_attack_speed() {
        let mut d = Ducker::default();
        d.set(Reason::Speech, true);
        for _ in 0..5 {
            d.tick();
        }
        d.set(Reason::Speech, false);
        for _ in 0..4 {
            d.tick();
        }
        let g = d.gain();
        assert!((g - 0.5).abs() < 1e-5, "{g}");
        d.set(Reason::Speech, true);
        assert!((d.tick() - DUCK_GAIN).abs() < 1e-5);
    }
}
