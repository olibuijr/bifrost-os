//! The media volume: sixteen steps, linear in dB over 30 dB, and the software gain the player
//! applies when the audio server is not there. The level lives on the object model: the volume
//! and mute controls of the default output device (`VolumeTarget`, implemented for the HAL
//! `Device`). Pure apart from `Volume::apply`, which a test drives through a fake target.
//!
//! The software gain is the only place the level is applied when there is no device control: a
//! player must not scale by its own volume as well, or the two attenuations add up.

use bifrost_audio_types::{props, AudioError, FourCC, PropertyAddress, PropertyValue, Scope};
use bifrost_audio_types::address::MAIN_ELEMENT;
use std::sync::atomic::{AtomicU32, Ordering};

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

/// The multiplier the player's audio thread applies, as f32 bits.
static SOFTWARE_GAIN: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// The multiplier the player applies to every sample (1.0 = untouched).
pub fn software_gain() -> f32 {
    f32::from_bits(SOFTWARE_GAIN.load(Ordering::Relaxed))
}

pub fn set_software_gain(gain: f32) {
    SOFTWARE_GAIN.store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

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

/// The default output device, or None when the audio server is not running.
pub fn default_output() -> Option<bifrost_audio_hal::Device> {
    let system = bifrost_audio_hal::System::connect(super::speaker::CLIENT).ok()?;
    system.default_device(bifrost_audio_hal::Role::Output).ok()
}

impl Volume {
    /// Applies the level: the output device's volume and mute controls carry it when the audio
    /// server is there (returns the scalar set), else the software gain does (every player applies
    /// it). A refused write falls back to the software gain and reports the error.
    pub fn apply(self, target: Option<&dyn VolumeTarget>) -> std::io::Result<Option<f32>> {
        let Some(t) = target else {
            set_software_gain(self.linear());
            return Ok(None);
        };
        match t.set_scalar(self.level).and_then(|()| t.set_mute(self.muted())) {
            Ok(()) => {
                set_software_gain(1.0);
                Ok(Some(self.level))
            }
            Err(e) => {
                set_software_gain(self.linear());
                Err(std::io::Error::other(format!("device volume: {e:?}")))
            }
        }
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
    use std::sync::Mutex;

    /// The software gain is process-wide: tests that set it take turns.
    static GAIN: Mutex<()> = Mutex::new(());

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
    fn apply_sets_the_device_controls_and_leaves_the_software_gain_at_unity() {
        let _g = GAIN.lock().unwrap();
        let f = Fake { calls: RefCell::new(vec![]), fail: false, value: 0.25 };
        assert_eq!(Volume::at_step(8).apply(Some(&f)).unwrap(), Some(0.5));
        assert_eq!(*f.calls.borrow(), ["scalar 0.5", "mute false"]);
        assert_eq!(software_gain(), 1.0);
        assert_eq!(Volume::from_device(&f), Some(Volume::new(0.25)));
        Volume::at_step(0).apply(Some(&f)).unwrap();
        assert_eq!(f.calls.borrow()[3], "mute true", "step 0 mutes");
    }

    #[test]
    fn apply_falls_back_to_software_gain_without_a_device_or_when_it_refuses() {
        let _g = GAIN.lock().unwrap();
        assert_eq!(Volume::at_step(8).apply(None).unwrap(), None);
        assert!((software_gain() - 0.1778).abs() < 1e-3);
        set_software_gain(1.0);
        let f = Fake { calls: RefCell::new(vec![]), fail: true, value: 0.0 };
        assert!(Volume::at_step(8).apply(Some(&f)).is_err());
        assert!((software_gain() - 0.1778).abs() < 1e-3);
        set_software_gain(1.0);
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
    fn software_gain_scales_samples_saturating() {
        let _g = GAIN.lock().unwrap();
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
        set_software_gain(0.25);
        assert_eq!(software_gain(), 0.25);
        set_software_gain(3.0);
        assert_eq!(software_gain(), 1.0, "clamped");
        set_software_gain(1.0);
    }
}
