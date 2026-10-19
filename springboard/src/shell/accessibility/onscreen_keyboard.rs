//! Accessibility Keyboard: Settings > Accessibility > Keyboard > Accessibility Keyboard opens the on-screen keyboard on
//! purpose, the same keyboard the Input menu's Show On-Screen Keyboard row opens (`Shell::keyboard_open_explicit` and
//! `keyboard_close_explicit`). A desktop never raises it by itself (`FormFactor::allows_onscreen_keyboard`), so this switch and
//! that menu row are its two entry points.
//!
//! The switch and the keyboard stay in step both ways: switching it on opens the keyboard on the next shell tick, switching
//! it off closes it, and closing the keyboard any other way (its hide key, the Input menu row) switches the switch off.
//! The preference is `a11y_accessibility_keyboard`; a keyboard still open when the shell stops is opened again at the next start.

use super::AccessibilitySettings;
use crate::shell::Shell;

impl Shell {
    /// Whether the Accessibility Keyboard switch is on, from the open Settings copy or the preferences.
    pub(in crate::shell) fn accessibility_keyboard_on(&self) -> bool {
        self.settings
            .as_ref()
            .map_or_else(|| AccessibilitySettings::load(&self.prefs).accessibility_keyboard, |st| st.accessibility().accessibility_keyboard)
    }

    /// Records that the on-screen keyboard was opened (`on`) or closed on purpose: the preference, and the open Settings
    /// page, so the switch shows what the keyboard does. Nothing is written when the switch already says so.
    pub(in crate::shell) fn remember_accessibility_keyboard(&mut self, on: bool) {
        let mut a = self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs), |st| st.accessibility().clone());
        if a.accessibility_keyboard == on {
            return;
        }
        a.accessibility_keyboard = on;
        a.save(&mut self.prefs);
        if let Some(st) = &mut self.settings {
            st.refresh_accessibility(&self.ui, a);
        }
        self.dirty = true;
    }

    /// One shell tick: a switch that disagrees with the keyboard moves the keyboard (the switch was flipped in Settings, or
    /// the preference was set at start). Nothing happens while they agree, so a tablet that never uses the switch is untouched.
    pub(in crate::shell) fn accessibility_keyboard_tick(&mut self, t: f64) {
        let wanted = self.accessibility_keyboard_on();
        if wanted && !self.osk_pinned {
            self.keyboard_open_explicit(t);
        } else if !wanted && self.osk_pinned {
            self.keyboard_close_explicit(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::keyboard;
    use crate::shell::tests::desktop_shell;

    #[test]
    fn switching_the_setting_on_opens_the_keyboard_and_off_closes_it() {
        keyboard::fake_process(500.0);
        let mut sh = desktop_shell();
        crate::shell::tests::unlock(&mut sh);
        assert!(!sh.accessibility_keyboard_on() && !sh.keyboard_opened_explicitly());
        sh.tick(1000.0);
        assert!(!sh.keyboard.shown(), "a desktop does not raise it by itself");
        sh.prefs.set("a11y_accessibility_keyboard", "1").unwrap();
        sh.tick(1100.0);
        assert!(sh.keyboard_opened_explicitly() && sh.keyboard.shown(), "the switch opens it");
        assert!(sh.desk_keyboard_visible(), "and the desktop draws it");
        sh.prefs.set("a11y_accessibility_keyboard", "0").unwrap();
        sh.tick(1200.0);
        assert!(!sh.keyboard_opened_explicitly() && !sh.keyboard.shown(), "switching it off closes it");
    }

    #[test]
    fn closing_the_keyboard_with_its_hide_key_switches_the_setting_off() {
        keyboard::fake_process(500.0);
        let mut sh = desktop_shell();
        sh.prefs.set("a11y_accessibility_keyboard", "1").unwrap();
        sh.tick(1000.0);
        assert!(sh.keyboard_opened_explicitly());
        keyboard::fake_typed(keyboard::Out::Hide);
        sh.tick(1100.0);
        assert!(!sh.keyboard_opened_explicitly() && !sh.keyboard.shown());
        assert_eq!(sh.prefs.get("a11y_accessibility_keyboard"), Some("0"), "the preference follows the keyboard");
        sh.tick(1200.0);
        assert!(!sh.keyboard.shown(), "and the next tick does not open it again");
    }

    #[test]
    fn the_input_menu_row_and_the_setting_stay_in_step() {
        keyboard::fake_process(500.0);
        let mut sh = desktop_shell();
        sh.keyboard_open_explicit(1000.0);
        assert_eq!(sh.prefs.get("a11y_accessibility_keyboard"), Some("1"), "opening it from the menu turns the switch on");
        sh.tick(1100.0);
        assert!(sh.keyboard_opened_explicitly(), "the tick leaves a keyboard the switch agrees with alone");
        sh.keyboard_close_explicit(1200.0);
        assert_eq!(sh.prefs.get("a11y_accessibility_keyboard"), Some("0"));
        sh.tick(1300.0);
        assert!(!sh.keyboard.shown());
    }

    #[test]
    fn closing_with_no_keyboard_open_leaves_the_setting_alone() {
        let mut sh = desktop_shell();
        sh.keyboard_close_explicit(1000.0);
        assert_eq!(sh.prefs.get("a11y_accessibility_keyboard"), None, "nothing was open, nothing is written");
    }
}
