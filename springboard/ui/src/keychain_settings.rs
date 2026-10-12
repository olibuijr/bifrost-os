//! Keychain settings: when a keychain locks by itself, and the panel that edits those settings.
//!
//! [`KeychainSettings`] holds two choices: whether the keychain locks when the machine sleeps, and how many minutes
//! of idle time pass before it locks (`None` means it never locks on idle). [`KeychainSettingsPanel`] keeps the
//! settings a user is editing next to the saved ones. Changes stay pending until [`KeychainSettingsPanel::commit`],
//! and [`KeychainSettingsPanel::revert`] drops them. The model is pure and reads no clock.

/// Shortest idle time the settings accept, in minutes.
pub const MIN_IDLE_MINUTES: u32 = 1;
/// Longest idle time the settings accept, in minutes (one day).
pub const MAX_IDLE_MINUTES: u32 = 1440;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeychainSettings {
    /// Lock the keychain when the machine sleeps.
    pub lock_on_sleep: bool,
    /// Minutes of idle time before the keychain locks; `None` never locks on idle.
    pub lock_after_idle: Option<u32>,
}

impl Default for KeychainSettings {
    /// Locks on sleep and never on idle.
    fn default() -> Self {
        KeychainSettings { lock_on_sleep: true, lock_after_idle: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// The idle time is outside [`MIN_IDLE_MINUTES`] to [`MAX_IDLE_MINUTES`]. Holds the rejected value.
    IdleOutOfRange(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeychainSettingsPanel {
    keychain: String,
    current: KeychainSettings,
    saved: KeychainSettings,
}

impl KeychainSettingsPanel {
    /// A panel for the named keychain, showing its saved settings.
    pub fn new(keychain: &str, settings: KeychainSettings) -> Self {
        KeychainSettingsPanel { keychain: keychain.to_string(), current: settings, saved: settings }
    }

    pub fn keychain(&self) -> &str {
        &self.keychain
    }

    /// The settings as the user is editing them, including changes not yet committed.
    pub fn settings(&self) -> KeychainSettings {
        self.current
    }

    pub fn set_lock_on_sleep(&mut self, lock: bool) {
        self.current.lock_on_sleep = lock;
    }

    /// Sets the idle time. A value outside the limits is refused and the current value stays.
    pub fn set_lock_after_idle(&mut self, minutes: Option<u32>) -> Result<(), SettingsError> {
        if let Some(m) = minutes {
            if !(MIN_IDLE_MINUTES..=MAX_IDLE_MINUTES).contains(&m) {
                return Err(SettingsError::IdleOutOfRange(m));
            }
        }
        self.current.lock_after_idle = minutes;
        Ok(())
    }

    /// `true` when the edited settings differ from the saved ones.
    pub fn is_dirty(&self) -> bool {
        self.current != self.saved
    }

    /// Drops the pending changes and shows the saved settings again.
    pub fn revert(&mut self) {
        self.current = self.saved;
    }

    /// Saves the pending changes. Returns the new settings when there were any, or `None` when nothing changed.
    pub fn commit(&mut self) -> Option<KeychainSettings> {
        if !self.is_dirty() {
            return None;
        }
        self.saved = self.current;
        Some(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> KeychainSettingsPanel {
        KeychainSettingsPanel::new("Work", KeychainSettings::default())
    }

    #[test]
    fn default_locks_on_sleep_and_never_on_idle() {
        let s = KeychainSettings::default();
        assert!(s.lock_on_sleep);
        assert_eq!(s.lock_after_idle, None);
    }

    #[test]
    fn idle_limits_are_one_minute_to_one_day() {
        let mut p = panel();
        assert_eq!(p.set_lock_after_idle(Some(0)), Err(SettingsError::IdleOutOfRange(0)));
        assert_eq!(p.set_lock_after_idle(Some(1441)), Err(SettingsError::IdleOutOfRange(1441)));
        assert_eq!(p.set_lock_after_idle(Some(1)), Ok(()));
        assert_eq!(p.set_lock_after_idle(Some(1440)), Ok(()));
        assert_eq!(p.settings().lock_after_idle, Some(1440));
    }

    #[test]
    fn refused_idle_value_leaves_the_setting_unchanged() {
        let mut p = panel();
        p.set_lock_after_idle(Some(15)).unwrap();
        assert!(p.set_lock_after_idle(Some(2000)).is_err());
        assert_eq!(p.settings().lock_after_idle, Some(15));
    }

    #[test]
    fn commit_saves_once_and_reports_no_change_after() {
        let mut p = panel();
        assert_eq!(p.commit(), None);
        p.set_lock_on_sleep(false);
        assert!(p.is_dirty());
        assert_eq!(p.commit(), Some(KeychainSettings { lock_on_sleep: false, lock_after_idle: None }));
        assert!(!p.is_dirty());
        assert_eq!(p.commit(), None);
    }

    #[test]
    fn revert_restores_the_saved_settings() {
        let mut p = panel();
        p.set_lock_on_sleep(false);
        p.set_lock_after_idle(Some(10)).unwrap();
        p.revert();
        assert_eq!(p.settings(), KeychainSettings::default());
        assert!(!p.is_dirty());
    }

    #[test]
    fn revert_after_commit_returns_to_the_committed_values() {
        let mut p = panel();
        p.set_lock_after_idle(Some(30)).unwrap();
        p.commit();
        p.set_lock_after_idle(None).unwrap();
        p.revert();
        assert_eq!(p.settings().lock_after_idle, Some(30));
    }

    #[test]
    fn keychain_name_is_kept() {
        assert_eq!(panel().keychain(), "Work");
    }
}
