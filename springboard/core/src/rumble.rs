//! Rumble for a game controller: a two-motor effect with a level for the heavy (strong) motor and one for the light
//! (weak) motor, a duration, and the 16-bit motor magnitudes a force-feedback rumble effect takes on the input node.
//!
//! The levels are fractions from 0 to 1; anything outside is clamped, and a value that is not finite is silent. The
//! module drives nothing: the caller uploads the magnitudes to the device, and checks that the device reports the
//! rumble effect (see [`crate::gamepad::supports_rumble`]).

/// The duration that means "until stopped". It is the largest millisecond count the effect can carry.
pub const INFINITE_MS: u32 = u32::MAX;

/// Full scale of a 16-bit motor magnitude.
const FULL_SCALE: f64 = 65_535.0;

fn level(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One rumble: the motor levels and how long they run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rumble {
    /// the heavy, low-frequency motor, 0 to 1
    pub strong: f64,
    /// the light, high-frequency motor, 0 to 1
    pub weak: f64,
    /// how long it runs, in milliseconds; [`INFINITE_MS`] runs until stopped
    pub duration_ms: u32,
}

impl Rumble {
    pub fn new(strong: f64, weak: f64, duration_ms: u32) -> Self {
        Rumble { strong: level(strong), weak: level(weak), duration_ms }
    }

    /// A rumble that runs until it is stopped.
    pub fn until_stopped(strong: f64, weak: f64) -> Self {
        Rumble::new(strong, weak, INFINITE_MS)
    }

    /// The rumble that stops every motor.
    pub fn stop() -> Self {
        Rumble::new(0.0, 0.0, 0)
    }

    pub fn is_infinite(&self) -> bool {
        self.duration_ms == INFINITE_MS
    }

    /// Whether both motors are off.
    pub fn is_silent(&self) -> bool {
        self.strong == 0.0 && self.weak == 0.0
    }

    /// The motor magnitudes as a rumble effect takes them, `(strong, weak)`, each from 0 to 65535.
    pub fn magnitudes(&self) -> (u16, u16) {
        (to_u16(self.strong), to_u16(self.weak))
    }
}

fn to_u16(v: f64) -> u16 {
    (level(v) * FULL_SCALE).round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_clamped_and_a_value_that_is_not_finite_is_silent() {
        let r = Rumble::new(1.5, -0.2, 100);
        assert_eq!((r.strong, r.weak), (1.0, 0.0));
        let nan = Rumble::new(f64::NAN, f64::INFINITY, 100);
        assert!(nan.is_silent());
    }

    #[test]
    fn full_and_half_levels_map_onto_the_sixteen_bit_range() {
        assert_eq!(Rumble::new(1.0, 0.0, 10).magnitudes(), (65_535, 0));
        assert_eq!(Rumble::new(0.0, 0.5, 10).magnitudes(), (0, 0x8000));
        assert_eq!(Rumble::new(2.0, 9.0, 10).magnitudes(), (65_535, 65_535));
    }

    #[test]
    fn an_until_stopped_rumble_is_infinite_and_a_timed_one_is_not() {
        assert!(Rumble::until_stopped(0.5, 0.5).is_infinite());
        assert!(!Rumble::new(0.5, 0.5, 250).is_infinite());
        assert_eq!(Rumble::until_stopped(0.5, 0.5).duration_ms, INFINITE_MS);
    }

    #[test]
    fn stop_silences_both_motors() {
        let stop = Rumble::stop();
        assert!(stop.is_silent());
        assert_eq!(stop.magnitudes(), (0, 0));
        assert!(!Rumble::new(0.0, 0.1, 10).is_silent());
    }

    #[test]
    fn a_rumble_keeps_the_duration_it_was_given() {
        let r = Rumble::new(0.3, 0.6, 1500);
        assert_eq!(r.duration_ms, 1500);
        assert_eq!(r.magnitudes().0, (0.3 * FULL_SCALE).round() as u16);
    }
}
