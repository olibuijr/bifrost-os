//! Force-feedback effects for a wheel or a joystick: the effect kinds, their value ranges, and the output level an
//! effect produces at a given time.
//!
//! Ranges follow the usual force-feedback convention: levels and gain run from 0 to 10000, and signed magnitudes run
//! from -10000 to 10000. Times are microseconds. The module drives nothing; the device layer (see
//! [`crate::ff_device`]) owns the effect slots and turns a level into a motor command.
//!
//! Envelope rule used here: the attack runs linearly from the attack level to the sustain magnitude over the attack
//! time, the sustain holds, and the fade runs linearly from the sustain magnitude to the fade level over the last
//! fade time of the effect. Attack and fade levels are magnitudes; the sign comes from the effect.

/// The largest gain and level, and the largest unsigned amplitude.
pub const LEVEL_MAX: u32 = 10_000;

/// The largest signed magnitude; the range is `-MAGNITUDE_MAX..=MAGNITUDE_MAX`.
pub const MAGNITUDE_MAX: i32 = 10_000;

/// The largest periodic phase, in degrees of a cycle times 100 (0 to 35999).
pub const PHASE_MAX: u32 = 35_999;

/// The duration that means "until stopped".
pub const INFINITE_US: u32 = u32::MAX;

/// Why an effect is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectError {
    /// a value is outside its range; the name is the field that is out of range
    OutOfRange(&'static str),
    /// a periodic effect needs a period above zero
    ZeroPeriod,
    /// the attack and the fade together are longer than the effect
    EnvelopeTooLong,
}

/// Optional shaping of an effect's amplitude over its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// magnitude at the start, 0 to 10000
    pub attack_level: u32,
    /// time to rise from the attack level to the sustain, in microseconds
    pub attack_us: u32,
    /// magnitude at the end, 0 to 10000
    pub fade_level: u32,
    /// time at the end over which the sustain falls to the fade level, in microseconds
    pub fade_us: u32,
}

/// The type-specific part of an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a steady force; `magnitude` is -10000 to 10000
    Constant { magnitude: i32 },
    /// a force that moves linearly from `start` to `end` over the duration; both are -10000 to 10000
    Ramp { start: i32, end: i32 },
    /// a repeating wave: `magnitude` 0 to 10000, `offset` -10000 to 10000, `phase` 0 to 35999, `period_us` above 0
    Periodic { magnitude: u32, offset: i32, phase: u32, period_us: u32 },
    /// a force that depends on displacement from `offset`, with coefficients -10000 to 10000, saturations and a
    /// dead band 0 to 10000
    Condition {
        offset: i32,
        positive: i32,
        negative: i32,
        positive_saturation: u32,
        negative_saturation: u32,
        dead_band: u32,
    },
}

/// One effect: its kind, gain, timing and optional envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub kind: Kind,
    /// scaling applied to every magnitude of the effect, 0 to 10000 (10000 is full)
    pub gain: u32,
    /// how long the effect runs; [`INFINITE_US`] runs until stopped
    pub duration_us: u32,
    /// the wait after a start before the effect plays, in microseconds
    pub start_delay_us: u32,
    pub envelope: Option<Envelope>,
}

fn check_magnitude(v: i32, field: &'static str) -> Result<(), EffectError> {
    if (-MAGNITUDE_MAX..=MAGNITUDE_MAX).contains(&v) {
        Ok(())
    } else {
        Err(EffectError::OutOfRange(field))
    }
}

fn check_level(v: u32, field: &'static str) -> Result<(), EffectError> {
    if v <= LEVEL_MAX {
        Ok(())
    } else {
        Err(EffectError::OutOfRange(field))
    }
}

impl Effect {
    /// A full-gain effect of this kind that runs for `duration_us`, with no delay and no envelope.
    pub fn new(kind: Kind, duration_us: u32) -> Self {
        Effect { kind, gain: LEVEL_MAX, duration_us, start_delay_us: 0, envelope: None }
    }

    /// Checks every value against its range and the timing rules. An effect that passes can be played.
    pub fn validate(&self) -> Result<(), EffectError> {
        check_level(self.gain, "gain")?;
        match self.kind {
            Kind::Constant { magnitude } => check_magnitude(magnitude, "magnitude")?,
            Kind::Ramp { start, end } => {
                check_magnitude(start, "start")?;
                check_magnitude(end, "end")?;
            }
            Kind::Periodic { magnitude, offset, phase, period_us } => {
                check_level(magnitude, "magnitude")?;
                check_magnitude(offset, "offset")?;
                if phase > PHASE_MAX {
                    return Err(EffectError::OutOfRange("phase"));
                }
                if period_us == 0 {
                    return Err(EffectError::ZeroPeriod);
                }
            }
            Kind::Condition { offset, positive, negative, positive_saturation, negative_saturation, dead_band } => {
                check_magnitude(offset, "offset")?;
                check_magnitude(positive, "positive")?;
                check_magnitude(negative, "negative")?;
                check_level(positive_saturation, "positive_saturation")?;
                check_level(negative_saturation, "negative_saturation")?;
                check_level(dead_band, "dead_band")?;
            }
        }
        if let Some(env) = self.envelope {
            check_level(env.attack_level, "attack_level")?;
            check_level(env.fade_level, "fade_level")?;
            if self.duration_us != INFINITE_US
                && env.attack_us as u64 + env.fade_us as u64 > self.duration_us as u64
            {
                return Err(EffectError::EnvelopeTooLong);
            }
        }
        Ok(())
    }

    /// The signed output level at `t_us` after the effect starts, from -10000 to 10000, with gain and envelope applied.
    ///
    /// Returns `Some(0)` once the duration has passed. Returns `None` for kinds that have no level curve in this model
    /// (periodic and condition).
    pub fn level_at(&self, t_us: u32) -> Option<i32> {
        if self.duration_us != INFINITE_US && t_us >= self.duration_us {
            return Some(0);
        }
        let t = t_us as i64;
        let base: i64 = match self.kind {
            Kind::Constant { magnitude } => magnitude as i64,
            Kind::Ramp { start, end } => {
                if self.duration_us == INFINITE_US {
                    start as i64
                } else {
                    let d = self.duration_us as i64;
                    start as i64 + (end as i64 - start as i64) * t / d
                }
            }
            Kind::Periodic { .. } | Kind::Condition { .. } => return None,
        };
        let shaped = match self.envelope {
            Some(env) => self.shape(base, env, t),
            None => base,
        };
        let scaled = shaped * self.gain as i64 / LEVEL_MAX as i64;
        Some(scaled.clamp(-(MAGNITUDE_MAX as i64), MAGNITUDE_MAX as i64) as i32)
    }

    /// Applies the envelope to a signed base value at time `t` (microseconds, already known to be inside the effect).
    fn shape(&self, base: i64, env: Envelope, t: i64) -> i64 {
        let sign = if base < 0 { -1 } else { 1 };
        let sustain = base.abs();
        let attack = env.attack_us as i64;
        let level = if attack > 0 && t < attack {
            env.attack_level as i64 + (sustain - env.attack_level as i64) * t / attack
        } else if self.duration_us != INFINITE_US && env.fade_us > 0 {
            let fade_start = self.duration_us as i64 - env.fade_us as i64;
            if t >= fade_start {
                sustain + (env.fade_level as i64 - sustain) * (t - fade_start) / env.fade_us as i64
            } else {
                sustain
            }
        } else {
            sustain
        };
        sign * level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnitudes_and_gain_are_range_checked() {
        let ok = Effect::new(Kind::Constant { magnitude: -10_000 }, 1000);
        assert_eq!(ok.validate(), Ok(()));
        let big = Effect::new(Kind::Constant { magnitude: 10_001 }, 1000);
        assert_eq!(big.validate(), Err(EffectError::OutOfRange("magnitude")));
        let gain = Effect { gain: 10_001, ..ok };
        assert_eq!(gain.validate(), Err(EffectError::OutOfRange("gain")));
    }

    #[test]
    fn periodic_needs_a_period_and_a_phase_inside_one_cycle() {
        let kind = |phase, period_us| Kind::Periodic { magnitude: 5000, offset: 0, phase, period_us };
        assert_eq!(Effect::new(kind(0, 0), 1000).validate(), Err(EffectError::ZeroPeriod));
        assert_eq!(Effect::new(kind(36_000, 100), 1000).validate(), Err(EffectError::OutOfRange("phase")));
        assert_eq!(Effect::new(kind(35_999, 100), 1000).validate(), Ok(()));
    }

    #[test]
    fn attack_and_fade_must_fit_inside_a_finite_effect() {
        let env = Envelope { attack_level: 0, attack_us: 600, fade_level: 0, fade_us: 500 };
        let mut e = Effect::new(Kind::Constant { magnitude: 5000 }, 1000);
        e.envelope = Some(env);
        assert_eq!(e.validate(), Err(EffectError::EnvelopeTooLong));
        e.duration_us = INFINITE_US;
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn constant_level_follows_gain_and_stops_at_the_end() {
        let mut e = Effect::new(Kind::Constant { magnitude: 8000 }, 1000);
        e.gain = 5000;
        assert_eq!(e.level_at(0), Some(4000));
        assert_eq!(e.level_at(999), Some(4000));
        assert_eq!(e.level_at(1000), Some(0));
    }

    #[test]
    fn ramp_moves_linearly_across_the_duration() {
        let e = Effect::new(Kind::Ramp { start: 0, end: 10_000 }, 1000);
        assert_eq!(e.level_at(0), Some(0));
        assert_eq!(e.level_at(250), Some(2500));
        assert_eq!(e.level_at(500), Some(5000));
    }

    #[test]
    fn envelope_rises_holds_and_falls() {
        let mut e = Effect::new(Kind::Constant { magnitude: 10_000 }, 1000);
        e.envelope = Some(Envelope { attack_level: 0, attack_us: 100, fade_level: 2000, fade_us: 200 });
        assert_eq!(e.level_at(0), Some(0));
        assert_eq!(e.level_at(50), Some(5000));
        assert_eq!(e.level_at(100), Some(10_000));
        assert_eq!(e.level_at(900), Some(6000));
        assert_eq!(e.level_at(999), Some(2040));
    }

    #[test]
    fn negative_force_keeps_its_sign_and_periodic_has_no_curve() {
        let neg = Effect::new(Kind::Constant { magnitude: -6000 }, 1000);
        assert_eq!(neg.level_at(10), Some(-6000));
        let wave = Effect::new(Kind::Periodic { magnitude: 5000, offset: 0, phase: 0, period_us: 100 }, 1000);
        assert_eq!(wave.level_at(10), None);
    }
}
