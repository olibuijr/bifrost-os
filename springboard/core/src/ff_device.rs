//! The state of one force-feedback device, such as a racing wheel or a joystick with motors: the device gain and
//! autocenter properties, the commands that pause, reset or stop it, and a fixed set of effect slots. An app
//! downloads an effect into a free slot, starts and stops it, and reads the level the device should output.
//!
//! The model is pure: it sends nothing to the kernel. A caller that talks to an evdev node uploads the effects it
//! gets from here. Gain scales every effect, and a paused device, a device with its actuators off, or a stopped
//! effect outputs zero. Effects wait out their start delay before they play.

use crate::ff_effect::{Effect, EffectError, LEVEL_MAX};

/// How many effects one device holds at once.
pub const SLOTS: usize = 16;

/// The device-wide settings an app can read and write. Both run from 0 to 10000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    /// scales every effect; 10000 is full
    Gain,
    /// the strength of the spring that returns a wheel or stick to centre
    Autocenter,
}

/// A device-wide command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// unloads every effect and restores the default properties and states
    Reset,
    /// stops every playing effect; the effects stay loaded
    StopAll,
    /// holds every effect at zero until the device continues
    Pause,
    /// lets paused effects play again
    Continue,
    /// turns the motors on
    ActuatorsOn,
    /// turns the motors off, so every effect outputs zero
    ActuatorsOff,
}

/// Why a device call is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// the effect is not valid
    Effect(EffectError),
    /// every slot holds an effect
    Full,
    /// the slot holds no effect
    NoEffect(usize),
}

impl From<EffectError> for DeviceError {
    fn from(e: EffectError) -> Self {
        DeviceError::Effect(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    effect: Effect,
    playing: bool,
}

/// One force-feedback device and its effect slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    gain: u32,
    autocenter: u32,
    paused: bool,
    actuators_on: bool,
    slots: Vec<Option<Slot>>,
}

impl Default for Device {
    fn default() -> Self {
        Device::new()
    }
}

impl Device {
    /// A device with full gain, no autocenter, the motors on, and every slot free.
    pub fn new() -> Self {
        Device { gain: LEVEL_MAX, autocenter: 0, paused: false, actuators_on: true, slots: vec![None; SLOTS] }
    }

    pub fn property(&self, property: Property) -> u32 {
        match property {
            Property::Gain => self.gain,
            Property::Autocenter => self.autocenter,
        }
    }

    /// Sets a property. A value above 10000 is clamped to 10000. Returns the value the device now holds.
    pub fn set_property(&mut self, property: Property, value: u32) -> u32 {
        let v = value.min(LEVEL_MAX);
        match property {
            Property::Gain => self.gain = v,
            Property::Autocenter => self.autocenter = v,
        }
        v
    }

    /// Stores a new effect in the first free slot and returns the slot number. The effect is not playing yet.
    pub fn download(&mut self, effect: Effect) -> Result<usize, DeviceError> {
        effect.validate()?;
        let index = self.slots.iter().position(|s| s.is_none()).ok_or(DeviceError::Full)?;
        self.slots[index] = Some(Slot { effect, playing: false });
        Ok(index)
    }

    /// Replaces the effect in a slot, keeping whether it is playing.
    pub fn set_parameters(&mut self, slot: usize, effect: Effect) -> Result<(), DeviceError> {
        effect.validate()?;
        let s = self.slot_mut(slot)?;
        s.effect = effect;
        Ok(())
    }

    /// Starts the effect in a slot from its beginning.
    pub fn start(&mut self, slot: usize) -> Result<(), DeviceError> {
        self.slot_mut(slot)?.playing = true;
        Ok(())
    }

    /// Stops the effect in a slot. It stays loaded.
    pub fn stop(&mut self, slot: usize) -> Result<(), DeviceError> {
        self.slot_mut(slot)?.playing = false;
        Ok(())
    }

    /// Removes the effect from a slot and frees it.
    pub fn unload(&mut self, slot: usize) -> Result<(), DeviceError> {
        self.slot_mut(slot)?;
        self.slots[slot] = None;
        Ok(())
    }

    /// Whether the app has started the effect in a slot and not stopped it. A missing slot is not playing.
    pub fn is_playing(&self, slot: usize) -> bool {
        matches!(self.slots.get(slot), Some(Some(s)) if s.playing)
    }

    /// Applies a device-wide command.
    pub fn command(&mut self, command: Command) {
        match command {
            Command::Reset => *self = Device::new(),
            Command::StopAll => {
                for s in self.slots.iter_mut().flatten() {
                    s.playing = false;
                }
            }
            Command::Pause => self.paused = true,
            Command::Continue => self.paused = false,
            Command::ActuatorsOn => self.actuators_on = true,
            Command::ActuatorsOff => self.actuators_on = false,
        }
    }

    /// The signed output level of the effect in a slot, `t_us` microseconds after the device was asked to start it,
    /// from -10000 to 10000. The device gain is applied. Returns `None` for a missing slot or for an effect kind the
    /// effect model has no curve for.
    pub fn output_level(&self, slot: usize, t_us: u32) -> Option<i32> {
        let s = self.slots.get(slot)?.as_ref()?;
        if !s.playing || self.paused || !self.actuators_on {
            return Some(0);
        }
        let delay = s.effect.start_delay_us;
        if t_us < delay {
            return Some(0);
        }
        let level = s.effect.level_at(t_us - delay)?;
        Some((level as i64 * self.gain as i64 / LEVEL_MAX as i64) as i32)
    }

    fn slot_mut(&mut self, slot: usize) -> Result<&mut Slot, DeviceError> {
        match self.slots.get_mut(slot) {
            Some(Some(s)) => Ok(s),
            _ => Err(DeviceError::NoEffect(slot)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ff_effect::Kind;

    fn constant(magnitude: i32) -> Effect {
        Effect::new(Kind::Constant { magnitude }, 1000)
    }

    #[test]
    fn download_start_stop_and_unload_track_one_slot() {
        let mut d = Device::new();
        assert_eq!(d.download(constant(100)), Ok(0));
        assert_eq!(d.download(constant(200)), Ok(1));
        d.start(0).unwrap();
        assert!(d.is_playing(0));
        assert!(!d.is_playing(1));
        d.stop(0).unwrap();
        assert!(!d.is_playing(0));
        d.unload(0).unwrap();
        assert_eq!(d.start(0), Err(DeviceError::NoEffect(0)));
        assert_eq!(d.download(constant(300)), Ok(0));
    }

    #[test]
    fn a_full_device_refuses_a_seventeenth_effect_and_an_invalid_effect_takes_no_slot() {
        let mut d = Device::new();
        for i in 0..SLOTS {
            assert_eq!(d.download(constant(100)), Ok(i));
        }
        assert_eq!(d.download(constant(100)), Err(DeviceError::Full));
        d.unload(5).unwrap();
        assert_eq!(d.download(constant(100)), Ok(5));

        let mut bad = Device::new();
        let mut e = constant(100);
        e.gain = 10_001;
        assert_eq!(bad.download(e), Err(DeviceError::Effect(EffectError::OutOfRange("gain"))));
        assert_eq!(bad.download(constant(100)), Ok(0));
    }

    #[test]
    fn device_gain_scales_the_output_and_is_clamped() {
        let mut d = Device::new();
        assert_eq!(d.set_property(Property::Gain, 5000), 5000);
        let slot = d.download(Effect::new(Kind::Constant { magnitude: 8000 }, 1000)).unwrap();
        d.start(slot).unwrap();
        assert_eq!(d.output_level(slot, 0), Some(4000));
        assert_eq!(d.set_property(Property::Gain, 20_000), 10_000);
        assert_eq!(d.output_level(slot, 0), Some(8000));
        assert_eq!(d.output_level(9, 0), None);
    }

    #[test]
    fn pause_and_actuators_hold_the_output_at_zero_without_stopping_the_effect() {
        let mut d = Device::new();
        d.download(constant(8000)).unwrap();
        d.start(0).unwrap();
        d.command(Command::Pause);
        assert_eq!(d.output_level(0, 0), Some(0));
        assert!(d.is_playing(0));
        d.command(Command::Continue);
        assert_eq!(d.output_level(0, 0), Some(8000));
        d.command(Command::ActuatorsOff);
        assert_eq!(d.output_level(0, 0), Some(0));
        d.command(Command::ActuatorsOn);
        assert_eq!(d.output_level(0, 0), Some(8000));
    }

    #[test]
    fn start_delay_is_waited_out_before_the_effect_plays() {
        let mut d = Device::new();
        let mut e = constant(8000);
        e.start_delay_us = 300;
        d.download(e).unwrap();
        d.start(0).unwrap();
        assert_eq!(d.output_level(0, 200), Some(0));
        assert_eq!(d.output_level(0, 300), Some(8000));
        assert_eq!(d.output_level(0, 1300), Some(0));
    }

    #[test]
    fn reset_clears_slots_and_properties_and_stop_all_keeps_the_effects() {
        let mut d = Device::new();
        d.set_property(Property::Gain, 5000);
        d.set_property(Property::Autocenter, 3000);
        d.download(constant(100)).unwrap();
        d.download(constant(200)).unwrap();
        d.start(0).unwrap();
        d.start(1).unwrap();
        d.command(Command::StopAll);
        assert!(!d.is_playing(0) && !d.is_playing(1));
        assert_eq!(d.start(1), Ok(()));
        d.command(Command::Reset);
        assert_eq!(d.property(Property::Gain), LEVEL_MAX);
        assert_eq!(d.property(Property::Autocenter), 0);
        assert_eq!(d.start(0), Err(DeviceError::NoEffect(0)));
    }
}
