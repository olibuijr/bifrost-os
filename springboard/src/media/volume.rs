//! The media volume: sixteen steps, linear in dB over 30 dB, and the software gain the player
//! applies when the sound card offers no volume control. On a desktop the card's own playback
//! volume carries the level when it has one (`desktop_control`). Pure apart from `Volume::apply`.
//!
//! The software gain is the only place the level is applied when there is no card control: a
//! player must not scale by its own volume as well, or the two attenuations add up.

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
/// The card's control device.
pub const CONTROL_DEVICE: &str = "/dev/snd/controlC0";

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

/// Playback volume controls a desktop sound card may offer, best first.
pub const DESKTOP_CONTROLS: [&str; 4] = ["Master", "PCM", "Speaker", "Headphone"];

/// The standard ALSA playback volume to drive on a desktop: the first of `DESKTOP_CONTROLS`
/// that exists as an integer "<name> Playback Volume" with a usable range.
pub fn desktop_control(controls: &[super::mixer::Control]) -> Option<&super::mixer::Control> {
    DESKTOP_CONTROLS.iter().find_map(|base| {
        let name = format!("{base} Playback Volume");
        controls.iter().find(|c| c.name == name && c.kind == super::mixer::Kind::Int && c.max > c.min)
    })
}

/// The raw value for `level` (0..=1) on a control spanning `min..=max`; linear, and level 0
/// lands on `min` exactly.
pub fn range_value(level: f32, min: i64, max: i64) -> i64 {
    min + (level.clamp(0.0, 1.0) * (max - min) as f32).round() as i64
}

#[cfg(target_os = "linux")]
impl Volume {
    /// Applies the level: the sound card's own volume carries it when there is a mixer with a
    /// playback control, else the software gain does (every player applies it).
    pub fn apply(self, mixer: Option<&super::mixer::Mixer>) -> std::io::Result<Option<i64>> {
        let hw = mixer.map(|m| self.apply_desktop(m)).transpose();
        set_software_gain(if matches!(hw, Ok(Some(_))) { 1.0 } else { self.linear() });
        hw
    }

    fn apply_desktop(self, m: &super::mixer::Mixer) -> std::io::Result<i64> {
        let controls = m.list()?;
        let c = desktop_control(&controls).ok_or_else(|| std::io::Error::other("no playback volume control"))?;
        let v = range_value(if self.muted() { 0.0 } else { self.level }, c.min, c.max);
        m.set(&c.name, &vec![v; c.count.max(1) as usize])?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn ctl(name: &str, kind: super::super::mixer::Kind, max: i64) -> super::super::mixer::Control {
        super::super::mixer::Control { numid: 1, name: name.into(), kind, count: 2, min: 0, max, step: 1, items: vec![], values: vec![], db: None }
    }

    #[test]
    fn desktop_picks_the_first_existing_playback_volume() {
        use super::super::mixer::Kind::{Bool, Int};
        let all = [ctl("Headphone Playback Volume", Int, 87), ctl("PCM Playback Volume", Int, 255), ctl("Master Playback Switch", Bool, 1)];
        assert_eq!(desktop_control(&all).unwrap().name, "PCM Playback Volume", "PCM outranks Headphone; a switch is no volume");
        let rev = [ctl("Speaker Playback Volume", Int, 10), ctl("Master Playback Volume", Int, 64)];
        assert_eq!(desktop_control(&rev).unwrap().name, "Master Playback Volume", "list order does not matter");
        assert!(desktop_control(&[ctl("Master Playback Volume", Bool, 1), ctl("Mic Capture Volume", Int, 9), ctl("Master Playback Volume", Int, 0)]).is_none());
        assert!(desktop_control(&[]).is_none());
    }

    #[test]
    fn range_value_maps_linearly_onto_the_control() {
        assert_eq!(range_value(0.0, 0, 87), 0);
        assert_eq!(range_value(1.0, 0, 87), 87);
        assert_eq!(range_value(0.5, 0, 100), 50);
        assert_eq!(range_value(0.5, 10, 20), 15);
        assert_eq!(range_value(2.0, 0, 87), 87);
        assert_eq!(range_value(-1.0, 4, 87), 4);
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
