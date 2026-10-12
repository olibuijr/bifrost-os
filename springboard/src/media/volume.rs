//! The media volume: sixteen steps, linear in dB over 30 dB. The level lives on the object model: the volume
//! and mute controls of the default output device (`VolumeTarget`, implemented for the HAL
//! `Device`). Pure apart from `Volume::apply`, which a test drives through a fake target.
//!
//! There is no process-wide software gain: a gain written by the shell never reached the players in other
//! processes. A player scales by its own volume (`Volume::new(v).linear()`, see `AudioPlayer`).

use bifrost_audio_types::{props, AudioError, FourCC, PropertyAddress, PropertyValue, Scope};
use bifrost_audio_types::address::MAIN_ELEMENT;

/// Volume steps between silence and full.
pub const STEPS: u32 = 16;
/// The curve spans this many dB: step 1 sits `RANGE_DB - STEP_DB` below full. 30 dB keeps the
/// middle of the slider audible on the small speakers (48 dB buried everything below 70 %).
pub const RANGE_DB: f32 = 30.0;
/// dB between neighbouring steps.
pub const STEP_DB: f32 = RANGE_DB / STEPS as f32;
/// Level when nothing is persisted yet: muted, so the machine never makes a
/// sound until the user turns the volume up (the user's standing rule).
pub const DEFAULT_LEVEL: f32 = 0.0;
/// The settings.ini key.
pub const PREF: &str = "volume";

/// A volume level, 0 (mute) ..= 1 (full).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    level: f32,
}

impl Default for Volume {
    fn default() -> Self {
        Volume::new(DEFAULT_LEVEL)
    }
}

impl Volume {
    pub fn new(level: f32) -> Self {
        Volume { level: if level.is_nan() { 0.0 } else { level.clamp(0.0, 1.0) } }
    }

    /// Exactly on step `step` of `STEPS`.
    pub fn at_step(step: u32) -> Self {
        Volume::new(step.min(STEPS) as f32 / STEPS as f32)
    }

    pub fn level(self) -> f32 {
        self.level
    }

    /// The nearest step, 0..=16.
    pub fn step(self) -> u32 {
        (self.level * STEPS as f32).round() as u32
    }

    /// One step louder (from the nearest step, so a slider value snaps).
    pub fn up(self) -> Self {
        Volume::at_step(self.step() + 1)
    }

    /// One step quieter.
    pub fn down(self) -> Self {
        Volume::at_step(self.step().saturating_sub(1))
    }

    pub fn muted(self) -> bool {
        self.step() == 0
    }

    /// Gain in dB below full scale, None when muted; linear in the level.
    pub fn gain_db(self) -> Option<f32> {
        (!self.muted()).then(|| -(1.0 - self.level) * RANGE_DB)
    }

    /// The sample multiplier for the software path.
    pub fn linear(self) -> f32 {
        self.gain_db().map_or(0.0, |db| 10f32.powf(db / 20.0))
    }

    /// The persisted level, `DEFAULT_LEVEL` when missing or unreadable.
    pub fn from_pref(value: Option<&str>) -> Self {
        value.and_then(|v| v.trim().parse().ok()).map_or_else(Volume::default, Volume::new)
    }

    /// The text to persist.
    pub fn pref(self) -> String {
        format!("{:.3}", self.level)
    }
}

/// Kept for callers that still print it: the gain is per player now, so this is always unity.
pub fn software_gain() -> f32 {
    1.0
}

/// Kept so existing call sites compile: a no-op (see `software_gain`).
pub fn set_software_gain(_gain: f32) {}

/// Scales interleaved S16 samples by `gain`, saturating; a no-op at 1.0.
pub fn scale(samples: &mut [i16], gain: f32) {
    if gain >= 1.0 {
        return;
    }
    if gain <= 0.0 {
        samples.fill(0);
        return;
    }
    // 16.16 fixed point: exact enough, and no float per sample
    let g = (gain * 65536.0) as i64;
    for s in samples {
        *s = ((*s as i64 * g + 32768) >> 16).clamp(i16::MIN as i64, i16::MAX as i64) as i16;
    }
}

/// Where a volume level is applied: the volume and mute controls of an output device.
pub trait VolumeTarget {
    fn set_scalar(&self, scalar: f32) -> Result<(), AudioError>;
    fn set_mute(&self, mute: bool) -> Result<(), AudioError>;
    fn scalar(&self) -> Result<f32, AudioError>;
}

fn output_address(selector: FourCC) -> PropertyAddress {
    PropertyAddress { selector, scope: Scope::Output, element: MAIN_ELEMENT }
}

impl VolumeTarget for bifrost_audio_hal::Device {
    fn set_scalar(&self, scalar: f32) -> Result<(), AudioError> {
        self.set(output_address(props::VOLUME_SCALAR), PropertyValue::F32(scalar))
    }

    fn set_mute(&self, mute: bool) -> Result<(), AudioError> {
        self.set(output_address(props::MUTE), PropertyValue::Bool(mute))
    }

    fn scalar(&self) -> Result<f32, AudioError> {
        match self.get(output_address(props::VOLUME_SCALAR))? {
            PropertyValue::F32(v) => Ok(v),
            _ => Err(AudioError::BadValueType),
        }
    }
}

/// The default output device, or None when the audio server is not running or does not answer within 100 ms.
/// Call from a worker thread only.
pub fn default_output() -> Option<bifrost_audio_hal::Device> {
    let system = bifrost_audio_hal::System::connect_within(super::speaker::CLIENT, std::time::Duration::from_millis(100)).ok()?;
    system.default_device(bifrost_audio_hal::Role::Output).ok()
}

impl Volume {
    /// Applies the level to the output device's volume and mute controls (returns the scalar set).
    /// Without a device (`None`, the audio server is not there) nothing is applied here and the result
    /// is `Ok(None)`; players scale by their own volume. A refused write reports the error.
    pub fn apply(self, target: Option<&dyn VolumeTarget>) -> std::io::Result<Option<f32>> {
        let Some(t) = target else { return Ok(None) };
        t.set_scalar(self.level).and_then(|()| t.set_mute(self.muted())).map_err(|e| std::io::Error::other(format!("device volume: {e:?}")))?;
        Ok(Some(self.level))
    }

    /// The level the device reports, for syncing the sliders with a change made elsewhere.
    pub fn from_device(target: &dyn VolumeTarget) -> Option<Volume> {
        target.scalar().ok().map(Volume::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<String>>,
        fail: bool,
        value: f32,
    }

    impl VolumeTarget for Fake {
        fn set_scalar(&self, v: f32) -> Result<(), AudioError> {
            self.calls.borrow_mut().push(format!("scalar {v}"));
            if self.fail { Err(AudioError::NotSettable) } else { Ok(()) }
        }
        fn set_mute(&self, m: bool) -> Result<(), AudioError> {
            self.calls.borrow_mut().push(format!("mute {m}"));
            Ok(())
        }
        fn scalar(&self) -> Result<f32, AudioError> {
            Ok(self.value)
        }
    }

    #[test]
    fn apply_sets_the_device_controls() {
        let f = Fake { calls: RefCell::new(vec![]), fail: false, value: 0.25 };
        assert_eq!(Volume::at_step(8).apply(Some(&f)).unwrap(), Some(0.5));
        assert_eq!(*f.calls.borrow(), ["scalar 0.5", "mute false"]);
        assert_eq!(Volume::from_device(&f), Some(Volume::new(0.25)));
        Volume::at_step(0).apply(Some(&f)).unwrap();
        assert_eq!(f.calls.borrow()[3], "mute true", "step 0 mutes");
    }

    #[test]
    fn apply_without_a_device_does_nothing_and_a_refusal_reports_the_error() {
        assert_eq!(Volume::at_step(8).apply(None).unwrap(), None);
        let f = Fake { calls: RefCell::new(vec![]), fail: true, value: 0.0 };
        assert!(Volume::at_step(8).apply(Some(&f)).is_err());
    }

    #[test]
    fn sixteen_even_steps_over_thirty_db_with_mute_at_zero() {
        assert_eq!(Volume::new(1.0).gain_db(), Some(0.0));
        assert_eq!(Volume::at_step(15).gain_db(), Some(-1.875));
        assert_eq!(Volume::at_step(8).gain_db(), Some(-15.0));
        assert_eq!(Volume::at_step(1).gain_db(), Some(-28.125));
        assert_eq!(Volume::at_step(0).gain_db(), None);
        assert!(Volume::at_step(0).muted());
        assert!(!Volume::at_step(1).muted());
        assert_eq!(Volume::new(0.75).gain_db(), Some(-7.5), "slider values are continuous");
    }

    #[test]
    fn keys_step_from_the_nearest_step_and_stop_at_the_ends() {
        assert_eq!(Volume::new(0.5).up().level(), 9.0 / 16.0);
        assert_eq!(Volume::new(0.5).down().level(), 7.0 / 16.0);
        assert_eq!(Volume::new(0.52).up().step(), 9, "a slider value snaps to the grid");
        assert_eq!(Volume::new(1.0).up(), Volume::new(1.0));
        assert_eq!(Volume::new(0.0).down(), Volume::new(0.0));
        assert_eq!(Volume::at_step(1).down().step(), 0);
        assert_eq!(Volume::at_step(0).up().step(), 1);
        assert_eq!(Volume::new(f32::NAN).level(), 0.0);
        assert_eq!(Volume::new(7.0).level(), 1.0);
    }

    #[test]
    fn linear_gain_follows_the_db_curve() {
        assert_eq!(Volume::new(1.0).linear(), 1.0);
        assert!((Volume::at_step(15).linear() - 0.8059).abs() < 1e-3);
        assert!((Volume::at_step(8).linear() - 0.1778).abs() < 1e-3);
        // the 56 % slider position (step 9) that was too quiet: now about -13 dB, not -31
        assert!(Volume::new(0.56).linear() > 0.2);
        assert_eq!(Volume::at_step(0).linear(), 0.0);
    }
    #[test]
    fn prefs_round_trip_with_a_default() {
        assert_eq!(Volume::from_pref(None), Volume::new(DEFAULT_LEVEL));
        assert_eq!(Volume::from_pref(Some("junk")), Volume::new(DEFAULT_LEVEL));
        assert_eq!(Volume::from_pref(Some(" 0.75 ")).level(), 0.75);
        assert_eq!(Volume::at_step(11).pref(), "0.688");
        assert_eq!(Volume::from_pref(Some(&Volume::at_step(11).pref())).step(), 11);
        assert_eq!(Volume::from_pref(Some("3")).level(), 1.0);
    }

    #[test]
    fn scale_scales_samples_saturating() {
        let mut s = [1000, -1000, i16::MAX, i16::MIN];
        scale(&mut s, 1.0);
        assert_eq!(s, [1000, -1000, i16::MAX, i16::MIN], "unity leaves samples alone");
        scale(&mut s, 0.5);
        assert_eq!(s, [500, -500, 16384, -16384]);
        scale(&mut s, 0.0);
        assert_eq!(s, [0; 4]);
        let mut s = [i16::MAX, i16::MIN];
        scale(&mut s, 0.999);
        assert!(s[0] > 32700 && s[1] < -32700);
    }
}
