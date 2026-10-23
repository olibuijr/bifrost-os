//! Focus silences notification delivery without changing alarm behavior.
//! The Focus that is on is a mode of `notify::modes`: Do Not Disturb, Work,
//! Personal, Sleep or one the person added. A mode is on by hand or by its
//! schedule; Do Not Disturb's schedule and allowed apps stay in the
//! notification store (see `notify::Schedule`). A muted app is silenced the
//! same way on its own.

use super::{notify, Shell};

impl Shell {
    /// Whether Do Not Disturb currently silences notification banners and wakes.
    pub fn dnd(&self) -> bool {
        self.notify.dnd()
    }

    /// Changes Focus from Control Centre and keeps a retained Settings page in sync.
    pub fn set_dnd(&mut self, on: bool) {
        self.set_focus_mode(on.then_some(notify::modes::DND_ID));
    }

    /// The Focus mode names, in list order, and the index of the mode that is on by hand: what Settings > Focus lists.
    pub(super) fn focus_mode_view(&self) -> (Vec<String>, Option<usize>) {
        let modes = &self.notify.modes;
        let names = modes.list.iter().map(|m| m.name.clone()).collect();
        let on = modes.manual.as_deref().and_then(|id| modes.list.iter().position(|m| m.id == id));
        (names, on)
    }

    /// Switches the Focus mode `id` on by hand, or Focus off (None), and saves the choice. Returns false for a mode
    /// that does not exist.
    pub fn set_focus_mode(&mut self, id: Option<&str>) -> bool {
        if !self.notify.set_mode(id) {
            return false;
        }
        // the Settings switch tracks Do Not Disturb only; another mode is saved in focus_modes.ini
        let on = id == Some(notify::modes::DND_ID);
        let mut opts = self.options();
        opts.dnd = on;
        opts.save(&mut self.prefs);
        let (names, mode_on) = self.focus_mode_view();
        let active = self.notify.active_mode().map(|m| m.name);
        if let Some(st) = &mut self.settings {
            st.set_focus_modes(names, mode_on, active);
            st.set_dnd(&self.ui, on);
        }
        self.notify.save_modes();
        self.dirty = true;
        true
    }

    /// Settings > Focus > Add Focus…: adds a mode named `name` (off until it is chosen), saves the list and shows it
    /// in a retained Settings page. Returns false when the list refused it (empty name, or full).
    pub fn add_focus_mode(&mut self, name: &str) -> bool {
        if self.notify.modes.add(name).is_none() {
            return false;
        }
        self.notify.save_modes();
        let (names, mode_on) = self.focus_mode_view();
        let active = self.notify.active_mode().map(|m| m.name);
        if let Some(st) = &mut self.settings {
            st.set_focus_modes(names, mode_on, active);
        }
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::notify::Notification;
    use crate::shell::prefs::Prefs;
    use crate::shell::tests::{settle, shell, unlock};
    use crate::shell::{AppId, Effect, Event};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-focus").join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn settings_lists_the_modes_and_follows_the_one_on_by_hand() {
        let mut sh = shell();
        unlock(&mut sh);
        let (names, on) = sh.focus_mode_view();
        assert_eq!(names, ["Do Not Disturb", "Work", "Personal", "Sleep"]);
        assert_eq!(on, None);
        assert!(sh.set_focus_mode(Some("work")));
        assert_eq!(sh.focus_mode_view().1, Some(1));
        assert!(sh.set_focus_mode(None));
        assert_eq!(sh.focus_mode_view().1, None);
    }

    /// E2E W5-9: Settings > Focus > Add Focus… adds a mode that stays off until it is chosen.
    #[test]
    fn add_focus_appends_a_mode_that_is_off() {
        let mut sh = shell();
        unlock(&mut sh);
        assert!(sh.add_focus_mode("Reading"));
        let (names, on) = sh.focus_mode_view();
        assert_eq!(names.last().map(String::as_str), Some("Reading"));
        assert_eq!(on, None, "a new mode is off");
        assert!(!sh.add_focus_mode("  "), "an empty name is refused");
    }

    #[test]
    fn dnd_keeps_notifications_and_badges_without_banners_or_screen_wakes() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.set_dnd(true);
        sh.notify(Notification::new(AppId::Settings, "Low Battery", "20% remaining", 1));
        assert!(sh.notify.banner().is_none());
        assert_eq!(sh.notify.store().count(AppId::Settings), 1);
        assert!(sh.notify.badge_gen() > 0);
        sh.event(Event::Power, 2000.0);
        sh.notify(Notification::new(AppId::Settings, "Wi-Fi", "Disconnected", 2));
        assert_eq!(sh.notify.next_wake(2100.0), None);
        assert_eq!(sh.next_wake(2100.0), None);
        assert!(sh.tick(2100.0).is_empty());
        assert!(!sh.screen_on());
        assert_eq!(sh.notify.store().count(AppId::Settings), 2);
    }

    #[test]
    fn enabling_dnd_clears_an_existing_banner_and_pending_notification_wake() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.notify(Notification::new(AppId::Settings, "Wi-Fi", "Connected", 1));
        assert!(sh.notify.banner().is_some());
        sh.set_dnd(true);
        assert!(sh.notify.banner().is_none());
        assert!(sh.needs_frame());
        sh.set_dnd(false);
        assert!(sh.notify.banner().is_none(), "old notifications are not replayed");
        sh.notify(Notification::new(AppId::Settings, "Wi-Fi", "Disconnected", 2));
        assert!(sh.notify.banner().is_some(), "new notifications deliver after Focus ends");
        sh.event(Event::Power, 2000.0);
        sh.notify(Notification::new(AppId::Settings, "Wi-Fi", "Connected", 3));
        assert_eq!(sh.notify.next_wake(2100.0), Some(2100.0));
        sh.set_dnd(true);
        assert_eq!(sh.notify.next_wake(2100.0), None);
        assert!(sh.tick(2100.0).is_empty());
        assert_eq!(sh.notify.store().len(), 3);
    }

    #[test]
    fn focus_persists_across_restart_and_syncs_retained_settings() {
        let path = scratch("settings.ini");
        let _ = std::fs::remove_file(&path);
        let mut sh = shell();
        sh.prefs = Prefs::load(&path);
        unlock(&mut sh);
        sh.open(AppId::Settings, 1000.0);
        settle(&mut sh, 1000.0);
        sh.set_dnd(true);
        assert!(sh.settings.as_ref().unwrap().options().dnd);
        let mut again = shell();
        again.prefs = Prefs::load(&path);
        again.apply_options();
        assert!(again.dnd());
        again.set_dnd(false);
        let mut third = shell();
        third.prefs = Prefs::load(&path);
        third.apply_options();
        assert!(!third.dnd());
    }

    #[test]
    fn alarm_still_sounds_and_wakes_a_dark_screen_during_dnd() {
        let path = scratch("clock.ini");
        std::fs::write(&path, "alarm.0.time=00:01\nalarm.0.on=1\nalarm.0.label=Wake up\n").unwrap();
        let mut sh = shell();
        sh.set_wall(1_704_067_200, 0, 1000.0);
        sh.clock.load(path.to_str().unwrap(), 1000.0);
        sh.set_dnd(true);
        sh.event(Event::Power, 2000.0);
        let fx = sh.tick(61_000.0);
        assert!(fx.contains(&Effect::Alarm), "{fx:?}");
        assert!(fx.contains(&Effect::ScreenOn), "{fx:?}");
        assert!(sh.screen_on());
        assert!(sh.clock.alert().is_some());
        assert!(sh.notify.banner().is_none());
    }

    #[test]
    fn a_mode_chosen_by_hand_survives_a_restart_and_the_settings_switch_follows_do_not_disturb_only() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-focus-restart").join(std::process::id().to_string());
        let notes = dir.join("notifications.ini").to_string_lossy().into_owned();
        let prefs_path = dir.join("settings.ini");
        let _ = std::fs::remove_file(&prefs_path);
        let _ = std::fs::remove_file(dir.join("focus_modes.ini"));
        let mut sh = shell();
        sh.notify.load_modes(&notes);
        sh.prefs = Prefs::load(&prefs_path);
        assert!(sh.set_focus_mode(Some("personal")));
        assert!(sh.dnd(), "a mode on by hand is Focus on");
        assert!(!sh.set_focus_mode(Some("nope")), "an unknown mode is refused");
        let mut again = shell();
        again.notify.load_modes(&notes);
        again.prefs = Prefs::load(&prefs_path);
        again.apply_options();
        assert_eq!(again.notify.modes.manual.as_deref(), Some("personal"));
        assert!(again.dnd());
        again.set_dnd(true);
        assert_eq!(again.notify.modes.manual.as_deref(), Some("do-not-disturb"), "Do Not Disturb replaces the other mode");
        again.set_dnd(false);
        assert!(!again.dnd());
    }
}
