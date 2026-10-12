//! Force-feedback effects for a wheel or a joystick: the effect kinds such a device takes (constant, periodic, ramp and
//! condition), an envelope that shapes each one, and a device that holds a fixed number of effects, starts and stops
//! them, and sums their output into one signed force.
//!
//! The module drives nothing. The caller reads the effect capability of the input node (see
//! [`crate::gamepad::has_bit`]), uploads each effect to the node, and feeds in the times, the wheel's displacement and
//! its velocity. Forces are fractions from -1 to 1, and anything outside that range is clamped; a value that is not
//! finite is silent. The effect durations and delays are milliseconds, and [`INFINITE_MS`] runs until stopped.

use std::fmt;

/// The duration that means "until stopped", as in [`crate::rumble::INFINITE_MS`].
pub const INFINITE_MS: u32 = u32::MAX;

/// The most effects one device holds at once.
pub const MAX_EFFECTS: usize = 16;

/// A signed force level, clamped to -1 to 1. A value that is not finite is silent.
fn level(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// A fraction from 0 to 1. A value that is not finite is zero.
fn unit(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The waveform of a periodic effect. Each one runs from -1 to 1 over a period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    /// +1 for the first half of the period, -1 for the second.
    Square,
    /// -1 at the start, rising to +1 at the middle, back to -1 at the end.
    Triangle,
    /// A full sine cycle, starting at zero.
    Sine,
    /// Rising from -1 to +1 across the period.
    SawUp,
    /// Falling from +1 to -1 across the period.
    SawDown,
}

/// The reaction a condition effect applies: a spring pulls back toward the centre by displacement, a damper resists
/// the velocity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionKind {
    Spring,
    Damper,
}

/// What an effect does while it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A steady force.
    Constant { level: f64 },
    /// A waveform: `offset + magnitude * wave`, with the wave phase shifted by `phase_ms`.
    Periodic { wave: Wave, magnitude: f64, offset: f64, phase_ms: u32, period_ms: u32 },
    /// A force that moves linearly from `start` at the first millisecond to `end` at the duration.
    Ramp { start: f64, end: f64 },
    /// A force that depends on the wheel. `positive` and `negative` are the coefficients on each side of the centre,
    /// `deadband` is the band around the centre that gives no force, and the force is clamped to `saturation`.
    Condition {
        kind: ConditionKind,
        positive: f64,
        negative: f64,
        deadband: f64,
        center: f64,
        saturation: f64,
    },
}

/// How an effect fades in and out. The level at the start of the attack, and at the end of the fade, is a fraction of
/// full strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub attack_ms: u32,
    pub attack_level: f64,
    pub fade_ms: u32,
    pub fade_level: f64,
}

impl Envelope {
    /// No attack and no fade: the effect is at full strength from its first millisecond to its last.
    pub const NONE: Envelope = Envelope { attack_ms: 0, attack_level: 1.0, fade_ms: 0, fade_level: 1.0 };
}

/// One force-feedback effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    pub shape: Shape,
    /// How long the effect runs after its delay, in milliseconds; [`INFINITE_MS`] runs until stopped.
    pub duration_ms: u32,
    /// How long the device waits after a start before the effect begins.
    pub delay_ms: u32,
    pub envelope: Envelope,
}

/// A rejected effect or device call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A periodic effect with a period of zero has no waveform.
    ZeroPeriod,
    /// An effect with a duration of zero never plays.
    ZeroDuration,
    /// The device already holds [`MAX_EFFECTS`] effects.
    Full,
    /// No effect has this id, or it was unloaded.
    UnknownEffect,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::ZeroPeriod => "periodic effect has a zero period",
            Error::ZeroDuration => "effect has a zero duration",
            Error::Full => "device holds the most effects it can",
            Error::UnknownEffect => "no effect has this id",
        };
        f.write_str(text)
    }
}

impl Effect {
    /// Checks the parts that make an effect unplayable.
    pub fn validate(&self) -> Result<(), Error> {
        if self.duration_ms == 0 {
            return Err(Error::ZeroDuration);
        }
        if let Shape::Periodic { period_ms: 0, .. } = self.shape {
            return Err(Error::ZeroPeriod);
        }
        Ok(())
    }

    /// The force at `elapsed_ms` after the effect was started, for a wheel at `displacement` (from -1 to 1) moving at
    /// `velocity` (per second, in the same fraction units). It is zero during the delay and after the duration.
    pub fn sample(&self, elapsed_ms: u64, displacement: f64, velocity: f64) -> f64 {
        let delay = u64::from(self.delay_ms);
        if elapsed_ms < delay {
            return 0.0;
        }
        let t = elapsed_ms - delay;
        if self.duration_ms != INFINITE_MS && t >= u64::from(self.duration_ms) {
            return 0.0;
        }
        let raw = match self.shape {
            Shape::Constant { level: l } => level(l),
            Shape::Periodic { wave, magnitude, offset, phase_ms, period_ms } => {
                // Integer modulo keeps the phase exact at every millisecond.
                let period = u64::from(period_ms);
                let p = ((t + u64::from(phase_ms)) % period) as f64 / period as f64;
                let w = match wave {
                    Wave::Square => {
                        if p < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
                    Wave::Sine => (2.0 * std::f64::consts::PI * p).sin(),
                    Wave::SawUp => 2.0 * p - 1.0,
                    Wave::SawDown => 1.0 - 2.0 * p,
                };
                level(level(offset) + level(magnitude) * w)
            }
            Shape::Ramp { start, end } => {
                let progress = t as f64 / f64::from(self.duration_ms);
                level(level(start) + (level(end) - level(start)) * progress)
            }
            Shape::Condition { kind, positive, negative, deadband, center, saturation } => {
                let x = match kind {
                    ConditionKind::Spring => displacement - center,
                    ConditionKind::Damper => velocity,
                };
                let x = if x.is_finite() { x } else { 0.0 };
                let band = unit(deadband);
                if x.abs() <= band {
                    0.0
                } else {
                    let coefficient = if x > 0.0 { level(positive) } else { level(negative) };
                    let outside = x - x.signum() * band;
                    let force = -coefficient * outside;
                    force.clamp(-unit(saturation), unit(saturation))
                }
            }
        };
        raw * self.envelope_scale(t)
    }

    /// The envelope's multiplier at `t` milliseconds into the duration. It is the lower of the attack and fade values.
    fn envelope_scale(&self, t: u64) -> f64 {
        let env = self.envelope;
        let mut scale = 1.0_f64;
        if env.attack_ms > 0 && t < u64::from(env.attack_ms) {
            let a = t as f64 / f64::from(env.attack_ms);
            let start = unit(env.attack_level);
            scale = scale.min(start + (1.0 - start) * a);
        }
        if env.fade_ms > 0 && self.duration_ms != INFINITE_MS {
            let fade_start = u64::from(self.duration_ms).saturating_sub(u64::from(env.fade_ms));
            if t >= fade_start {
                let f = ((t - fade_start) as f64 / f64::from(env.fade_ms)).min(1.0);
                scale = scale.min(1.0 + (unit(env.fade_level) - 1.0) * f);
            }
        }
        scale
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    id: u16,
    effect: Effect,
    /// The millisecond the effect was started, or `None` while it is stopped.
    started_at: Option<u64>,
}

/// A force-feedback device: up to [`MAX_EFFECTS`] effects, each stopped or playing, with a gain that scales the sum and
/// an autocenter level for the centring spring the device applies on its own.
#[derive(Debug)]
pub struct Device {
    slots: Vec<Slot>,
    next_id: u16,
    gain: f64,
    autocenter: f64,
}

impl Default for Device {
    fn default() -> Self {
        Device::new()
    }
}

impl Device {
    pub fn new() -> Self {
        Device { slots: Vec::new(), next_id: 1, gain: 1.0, autocenter: 0.0 }
    }

    /// Stores a new effect, stopped, and returns its id. Ids count up from 1.
    pub fn create(&mut self, effect: Effect) -> Result<u16, Error> {
        effect.validate()?;
        if self.slots.len() >= MAX_EFFECTS {
            return Err(Error::Full);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.slots.push(Slot { id, effect, started_at: None });
        Ok(id)
    }

    /// Replaces the parameters of an effect. A playing effect keeps its start time.
    pub fn set_parameters(&mut self, id: u16, effect: Effect) -> Result<(), Error> {
        effect.validate()?;
        let slot = self.slot_mut(id)?;
        slot.effect = effect;
        Ok(())
    }

    /// Starts an effect at `now_ms`. Starting a playing effect restarts it.
    pub fn start(&mut self, id: u16, now_ms: u64) -> Result<(), Error> {
        let slot = self.slot_mut(id)?;
        slot.started_at = Some(now_ms);
        Ok(())
    }

    /// Stops an effect. Stopping a stopped effect does nothing.
    pub fn stop(&mut self, id: u16) -> Result<(), Error> {
        let slot = self.slot_mut(id)?;
        slot.started_at = None;
        Ok(())
    }

    /// Removes an effect and frees its slot.
    pub fn unload(&mut self, id: u16) -> Result<(), Error> {
        let index = self.slots.iter().position(|s| s.id == id).ok_or(Error::UnknownEffect)?;
        self.slots.remove(index);
        Ok(())
    }

    /// Whether an effect was started and not stopped. It says nothing about whether its duration has run out.
    pub fn is_playing(&self, id: u16) -> bool {
        self.slots.iter().any(|s| s.id == id && s.started_at.is_some())
    }

    /// The number of effects the device holds.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Sets the device gain, a fraction from 0 to 1 that scales the whole output. Anything outside is clamped.
    pub fn set_gain(&mut self, gain: f64) {
        self.gain = unit(gain);
    }

    pub fn gain(&self) -> f64 {
        self.gain
    }

    /// Sets the autocenter level, a fraction from 0 to 1: how strongly the device centres the wheel without an effect.
    pub fn set_autocenter(&mut self, level: f64) {
        self.autocenter = unit(level);
    }

    pub fn autocenter(&self) -> f64 {
        self.autocenter
    }

    /// The summed force of every playing effect at `now_ms`, scaled by the gain and clamped to -1 to 1.
    pub fn output(&self, now_ms: u64, displacement: f64, velocity: f64) -> f64 {
        let total: f64 = self
            .slots
            .iter()
            .filter_map(|slot| {
                let started = slot.started_at?;
                if now_ms < started {
                    return None;
                }
                Some(slot.effect.sample(now_ms - started, displacement, velocity))
            })
            .sum();
        (total * self.gain).clamp(-1.0, 1.0)
    }

    /// The same force as [`Device::output`] as a signed 16-bit value, for a device that takes integer levels.
    pub fn output_s16(&self, now_ms: u64, displacement: f64, velocity: f64) -> i16 {
        (self.output(now_ms, displacement, velocity) * 32_767.0).round() as i16
    }

    fn slot_mut(&mut self, id: u16) -> Result<&mut Slot, Error> {
        self.slots.iter_mut().find(|s| s.id == id).ok_or(Error::UnknownEffect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn plain(shape: Shape, duration_ms: u32) -> Effect {
        Effect { shape, duration_ms, delay_ms: 0, envelope: Envelope::NONE }
    }

    #[test]
    fn a_constant_effect_is_silent_during_its_delay_and_after_its_duration() {
        let e = Effect {
            shape: Shape::Constant { level: 0.5 },
            duration_ms: 100,
            delay_ms: 20,
            envelope: Envelope::NONE,
        };
        assert_eq!(e.sample(10, 0.0, 0.0), 0.0);
        assert_eq!(e.sample(20, 0.0, 0.0), 0.5);
        assert_eq!(e.sample(119, 0.0, 0.0), 0.5);
        assert_eq!(e.sample(120, 0.0, 0.0), 0.0);
    }

    #[test]
    fn a_square_wave_flips_at_half_its_period() {
        let e = plain(
            Shape::Periodic { wave: Wave::Square, magnitude: 1.0, offset: 0.0, phase_ms: 0, period_ms: 100 },
            1000,
        );
        assert_eq!(e.sample(0, 0.0, 0.0), 1.0);
        assert_eq!(e.sample(49, 0.0, 0.0), 1.0);
        assert_eq!(e.sample(50, 0.0, 0.0), -1.0);
        assert_eq!(e.sample(99, 0.0, 0.0), -1.0);
        assert_eq!(e.sample(100, 0.0, 0.0), 1.0);
    }

    #[test]
    fn a_triangle_and_a_sine_peak_at_the_stated_points() {
        let tri = plain(
            Shape::Periodic { wave: Wave::Triangle, magnitude: 1.0, offset: 0.0, phase_ms: 0, period_ms: 100 },
            1000,
        );
        assert!(close(tri.sample(0, 0.0, 0.0), -1.0));
        assert!(close(tri.sample(25, 0.0, 0.0), 0.0));
        assert!(close(tri.sample(50, 0.0, 0.0), 1.0));
        let sine = plain(
            Shape::Periodic { wave: Wave::Sine, magnitude: 1.0, offset: 0.0, phase_ms: 0, period_ms: 100 },
            1000,
        );
        assert!(close(sine.sample(0, 0.0, 0.0), 0.0));
        assert!(close(sine.sample(25, 0.0, 0.0), 1.0));
        assert!(close(sine.sample(75, 0.0, 0.0), -1.0));
    }

    #[test]
    fn a_ramp_moves_linearly_from_start_to_end_over_its_duration() {
        let e = plain(Shape::Ramp { start: -1.0, end: 1.0 }, 200);
        assert!(close(e.sample(0, 0.0, 0.0), -1.0));
        assert!(close(e.sample(50, 0.0, 0.0), -0.5));
        assert!(close(e.sample(100, 0.0, 0.0), 0.0));
    }

    #[test]
    fn the_envelope_fades_an_effect_in_and_out() {
        let e = Effect {
            shape: Shape::Constant { level: 1.0 },
            duration_ms: 1000,
            delay_ms: 0,
            envelope: Envelope { attack_ms: 100, attack_level: 0.0, fade_ms: 200, fade_level: 0.0 },
        };
        assert!(close(e.sample(0, 0.0, 0.0), 0.0));
        assert!(close(e.sample(50, 0.0, 0.0), 0.5));
        assert!(close(e.sample(200, 0.0, 0.0), 1.0));
        assert!(close(e.sample(900, 0.0, 0.0), 0.5));
    }

    #[test]
    fn a_spring_pushes_back_outside_the_deadband_and_is_clamped() {
        let spring = |positive, negative, deadband, saturation| {
            plain(
                Shape::Condition {
                    kind: ConditionKind::Spring,
                    positive,
                    negative,
                    deadband,
                    center: 0.0,
                    saturation,
                },
                1000,
            )
        };
        let e = spring(0.5, 0.5, 0.1, 1.0);
        assert!(close(e.sample(0, 0.4, 0.0), -0.15));
        assert!(close(e.sample(0, -0.4, 0.0), 0.15));
        assert_eq!(e.sample(0, 0.05, 0.0), 0.0);
        let hard = spring(10.0, 10.0, 0.0, 0.25);
        assert!(close(hard.sample(0, 1.0, 0.0), -0.25));
    }

    #[test]
    fn a_damper_resists_velocity_not_displacement() {
        let e = plain(
            Shape::Condition {
                kind: ConditionKind::Damper,
                positive: 0.5,
                negative: 0.5,
                deadband: 0.0,
                center: 0.0,
                saturation: 1.0,
            },
            1000,
        );
        assert!(close(e.sample(0, 0.9, 0.4), -0.2));
        assert!(close(e.sample(0, 0.9, -0.4), 0.2));
    }

    #[test]
    fn a_device_caps_its_effects_and_rejects_bad_ones() {
        let mut d = Device::new();
        let constant = plain(Shape::Constant { level: 0.1 }, 10);
        for _ in 0..MAX_EFFECTS {
            d.create(constant).unwrap();
        }
        assert_eq!(d.create(constant), Err(Error::Full));
        let zero_period = plain(
            Shape::Periodic { wave: Wave::Sine, magnitude: 1.0, offset: 0.0, phase_ms: 0, period_ms: 0 },
            10,
        );
        assert_eq!(Device::new().create(zero_period), Err(Error::ZeroPeriod));
        assert_eq!(Device::new().create(plain(Shape::Constant { level: 1.0 }, 0)), Err(Error::ZeroDuration));
        assert_eq!(Device::new().start(7, 0), Err(Error::UnknownEffect));
    }

    #[test]
    fn playing_effects_sum_and_scale_by_gain_and_clamp_to_full_scale() {
        let mut d = Device::new();
        let a = d.create(plain(Shape::Constant { level: 0.5 }, 100)).unwrap();
        let b = d.create(plain(Shape::Constant { level: -0.25 }, 100)).unwrap();
        d.start(a, 0).unwrap();
        d.start(b, 0).unwrap();
        assert!(close(d.output(0, 0.0, 0.0), 0.25));
        d.set_gain(0.5);
        assert!(close(d.output(0, 0.0, 0.0), 0.125));
        d.set_gain(1.0);
        d.stop(b).unwrap();
        assert!(close(d.output(0, 0.0, 0.0), 0.5));
        assert_eq!(d.output_s16(0, 0.0, 0.0), 16_384);
        let loud = d.create(plain(Shape::Constant { level: 1.0 }, 100)).unwrap();
        d.start(loud, 0).unwrap();
        assert!(close(d.output(0, 0.0, 0.0), 1.0));
        assert_eq!(d.output_s16(0, 0.0, 0.0), 32_767);
        d.unload(a).unwrap();
        assert_eq!(d.len(), 2);
        assert!(!d.is_playing(a));
    }

    #[test]
    fn an_effect_starts_at_its_start_time_and_a_stop_silences_it() {
        let mut d = Device::new();
        let id = d.create(plain(Shape::Constant { level: 0.8 }, 100)).unwrap();
        d.start(id, 100).unwrap();
        assert!(d.is_playing(id));
        assert_eq!(d.output(50, 0.0, 0.0), 0.0);
        assert!(close(d.output(100, 0.0, 0.0), 0.8));
        d.stop(id).unwrap();
        assert!(!d.is_playing(id));
        assert_eq!(d.output(120, 0.0, 0.0), 0.0);
    }
}
