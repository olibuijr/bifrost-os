//! The form factor the system runs on: a desktop or laptop with pointer and hardware keyboard, or a touch tablet.
//! One shell and one SDK serve both; this model is the single switch. It is read once from `BIFROST_FORM` or the file
//! `/etc/bifrost/form` (one word, `desktop` or `tablet`) and defaults to desktop.

use std::sync::OnceLock;

/// Environment variable that names the form factor. It wins over the file.
pub const ENV: &str = "BIFROST_FORM";
/// System file holding the form factor as a plain word.
pub const FILE: &str = "/etc/bifrost/form";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FormFactor {
    /// Pointer and hardware keyboard: no on-screen keyboard.
    #[default]
    Desktop,
    /// Touch first: text fields raise the on-screen keyboard unless a hardware keyboard is attached.
    Tablet,
}

impl FormFactor {
    /// Parses the plain word, ignoring case and surrounding whitespace. Unknown words give `None`.
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "desktop" => Some(Self::Desktop),
            "tablet" => Some(Self::Tablet),
            _ => None,
        }
    }

    /// The word `parse` reads back; also what the shell passes to apps in `BIFROST_FORM`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Tablet => "tablet",
        }
    }

    /// Resolves the form from the environment value and the file contents. A value that does not parse is skipped, so a
    /// typo in the variable falls through to the file, and then to the desktop default.
    pub fn resolve(env: Option<&str>, file: Option<&str>) -> Self {
        env.and_then(Self::parse).or_else(|| file.and_then(Self::parse)).unwrap_or_default()
    }

    /// The form of this process, read once. Shell and apps call this; tests build a value and pass it instead.
    pub fn current() -> Self {
        static CURRENT: OnceLock<FormFactor> = OnceLock::new();
        *CURRENT.get_or_init(|| {
            let env = std::env::var(ENV).ok();
            let file = std::fs::read_to_string(FILE).ok();
            Self::resolve(env.as_deref(), file.as_deref())
        })
    }

    pub fn is_tablet(self) -> bool {
        self == Self::Tablet
    }

    pub fn is_desktop(self) -> bool {
        self == Self::Desktop
    }

    /// Whether the on-screen keyboard may be raised. `desktop_ui` is true while the windowed desktop UI is up.
    /// A desktop never raises it in that UI (the old touch-only presentation still can); a tablet raises it with or
    /// without the desktop UI, unless a hardware keyboard is attached.
    pub fn allows_onscreen_keyboard(self, desktop_ui: bool, hardware_keyboard: bool) -> bool {
        match self {
            Self::Desktop => !desktop_ui,
            Self::Tablet => !hardware_keyboard,
        }
    }

    /// Whether the session has no on-screen keyboard at all, so views and the keyboard process stay off.
    pub fn keyboard_disabled_globally(self) -> bool {
        self.is_desktop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_parse_ignoring_case_and_whitespace() {
        assert_eq!(FormFactor::parse("tablet\n"), Some(FormFactor::Tablet));
        assert_eq!(FormFactor::parse("  Desktop "), Some(FormFactor::Desktop));
        assert_eq!(FormFactor::parse("phone"), None);
        assert_eq!(FormFactor::parse(""), None);
    }

    #[test]
    fn as_str_round_trips() {
        for f in [FormFactor::Desktop, FormFactor::Tablet] {
            assert_eq!(FormFactor::parse(f.as_str()), Some(f));
        }
    }

    #[test]
    fn the_variable_wins_over_the_file_and_the_default_is_desktop() {
        assert_eq!(FormFactor::resolve(Some("tablet"), Some("desktop")), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(None, Some("tablet\n")), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(None, None), FormFactor::Desktop);
        assert_eq!(FormFactor::resolve(Some("garbage"), Some("tablet")), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(Some("garbage"), Some("also garbage")), FormFactor::Desktop);
    }

    #[test]
    fn a_desktop_never_raises_the_keyboard_on_the_desktop_ui() {
        for hw in [false, true] {
            assert!(!FormFactor::Desktop.allows_onscreen_keyboard(true, hw));
        }
        assert!(FormFactor::Desktop.allows_onscreen_keyboard(false, false));
    }

    #[test]
    fn a_tablet_raises_it_with_the_desktop_ui_up() {
        assert!(FormFactor::Tablet.allows_onscreen_keyboard(true, false));
        assert!(FormFactor::Tablet.allows_onscreen_keyboard(false, false));
    }

    #[test]
    fn a_hardware_keyboard_on_a_tablet_hides_it() {
        assert!(!FormFactor::Tablet.allows_onscreen_keyboard(true, true));
        assert!(!FormFactor::Tablet.allows_onscreen_keyboard(false, true));
    }

    #[test]
    fn only_a_desktop_turns_the_keyboard_off_for_the_session() {
        assert!(FormFactor::Desktop.keyboard_disabled_globally());
        assert!(!FormFactor::Tablet.keyboard_disabled_globally());
    }
}
