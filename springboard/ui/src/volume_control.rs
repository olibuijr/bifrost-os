//! A device's volume control: a level from 0 to 1 and a mute flag, kept apart. Muting does not change the level, so
//! unmuting returns to the level that was set before, and setting a level while muted leaves the device muted. The
//! effective gain is 0 while muted and the level otherwise. Pure: no mixer is touched and no sample is scaled.

/// Why a volume change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeError {
    /// The level is NaN or infinite.
    NotFinite,
}

/// The volume control of one device, one channel group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeControl {
    level: f32,
    muted: bool,
}

impl Default for VolumeControl {
    /// Full level and not muted.
    fn default() -> Self {
        VolumeControl { level: 1.0, muted: false }
    }
}

impl VolumeControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// The level as set, 0.0 to 1.0, whether or not the control is muted.
    pub fn level(&self) -> f32 {
        self.level
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// The gain the device applies: 0.0 while muted, the level otherwise.
    pub fn effective(&self) -> f32 {
        if self.muted { 0.0 } else { self.level }
    }

    /// Sets the level, clamped to 0.0..=1.0. The mute flag does not change. Refuses a value that is not finite.
    pub fn set_level(&mut self, level: f32) -> Result<(), VolumeError> {
        if !level.is_finite() {
            return Err(VolumeError::NotFinite);
        }
        self.level = level.clamp(0.0, 1.0);
        Ok(())
    }

    /// Moves the level by `delta`, clamped to 0.0..=1.0. Refuses a delta that is not finite.
    pub fn nudge(&mut self, delta: f32) -> Result<f32, VolumeError> {
        if !delta.is_finite() {
            return Err(VolumeError::NotFinite);
        }
        self.level = (self.level + delta).clamp(0.0, 1.0);
        Ok(self.level)
    }

    /// Sets the mute flag. Returns true when the flag changed.
    pub fn set_muted(&mut self, muted: bool) -> bool {
        let changed = self.muted != muted;
        self.muted = muted;
        changed
    }

    /// Flips the mute flag and returns the new value.
    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        self.muted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muting_keeps_the_level_and_unmuting_restores_it() {
        let mut v = VolumeControl::new();
        v.set_level(0.4).unwrap();
        assert!(v.set_muted(true));
        assert_eq!(v.level(), 0.4);
        assert_eq!(v.effective(), 0.0);
        assert!(v.set_muted(false));
        assert_eq!(v.effective(), 0.4);
    }

    #[test]
    fn setting_a_level_while_muted_stays_muted() {
        let mut v = VolumeControl::new();
        v.set_muted(true);
        v.set_level(0.25).unwrap();
        assert!(v.is_muted());
        assert_eq!(v.level(), 0.25);
        assert_eq!(v.effective(), 0.0);
    }

    #[test]
    fn levels_are_clamped_and_non_finite_values_are_refused() {
        let mut v = VolumeControl::new();
        v.set_level(1.7).unwrap();
        assert_eq!(v.level(), 1.0);
        v.set_level(-0.5).unwrap();
        assert_eq!(v.level(), 0.0);
        assert_eq!(v.set_level(f32::NAN), Err(VolumeError::NotFinite));
        assert_eq!(v.set_level(f32::INFINITY), Err(VolumeError::NotFinite));
        assert_eq!(v.level(), 0.0);
    }

    #[test]
    fn nudge_moves_the_level_within_range() {
        let mut v = VolumeControl::new();
        v.set_level(0.5).unwrap();
        assert_eq!(v.nudge(0.25), Ok(0.75));
        assert_eq!(v.nudge(0.5), Ok(1.0));
        assert_eq!(v.nudge(-1.5), Ok(0.0));
        assert_eq!(v.nudge(f32::NAN), Err(VolumeError::NotFinite));
    }

    #[test]
    fn toggle_flips_the_flag_and_set_muted_reports_changes() {
        let mut v = VolumeControl::default();
        assert_eq!(v.level(), 1.0);
        assert!(v.toggle_mute());
        assert!(!v.toggle_mute());
        assert!(!v.set_muted(false));
        assert!(v.set_muted(true));
        assert!(!v.set_muted(true));
    }
}
