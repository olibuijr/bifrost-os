//! App Intents of the Settings panes: "Open Date & Time", "Open Keyboard" and the like. Each intent names the pane it
//! opens (`PageId`), so Search and the agent socket can list them and jump to the pane. Changing a value still goes
//! through the pane's own actions (`pages::Action`); an intent never writes a setting itself.
//!
//! The catalogue uses the same `[[intent]]` manifest spelling as app manifests (`manifest_lines`), so the shell can
//! list these in the same way it lists app shortcuts (`intents.rs`). Search lists them among its Shortcuts
//! (`quicksearch.rs`, `settings_intent_rows`) and opens the pane with `Settings::open_page_id`.

use super::pages::PageId;

/// One settings intent: a runnable shortcut that opens a pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsIntent {
    /// the id used in `intent.<id>` and in manifests; lower-case ASCII, digits, `-` and `_`
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// the pane the intent opens
    pub page: PageId,
}

/// Every settings intent, in the order Search lists them.
pub const CATALOGUE: &[SettingsIntent] = &[
    SettingsIntent { id: "open-sound", title: "Open Sound", description: "Show the sound output, input and alert settings", page: PageId::Sounds },
    SettingsIntent { id: "open-privacy", title: "Open Privacy", description: "Show the privacy and permission settings", page: PageId::Privacy },
    SettingsIntent { id: "open-screen-time", title: "Open Screen Time", description: "Show the screen time settings", page: PageId::ScreenTime },
    SettingsIntent { id: "open-storage", title: "Open Storage", description: "Show the storage use of this computer", page: PageId::Storage },
    SettingsIntent { id: "open-sharing", title: "Open Sharing", description: "Show the sharing settings", page: PageId::Sharing },
    SettingsIntent { id: "open-software-update", title: "Open Software Update", description: "Show the software update settings", page: PageId::SoftwareUpdate },
    SettingsIntent { id: "open-about", title: "Open About", description: "Show information about this computer", page: PageId::About },
    SettingsIntent { id: "open-account", title: "Open BifrOSt Account", description: "Show the account settings", page: PageId::Root },
    SettingsIntent { id: "open-accessibility", title: "Open Accessibility", description: "Show the accessibility settings", page: PageId::Accessibility },
    SettingsIntent { id: "open-users-groups", title: "Open Users & Groups", description: "Show the users and groups on this computer", page: PageId::Users },
    SettingsIntent { id: "open-appearance", title: "Open Appearance", description: "Show the light, dark and accent settings", page: PageId::Appearance },
    SettingsIntent { id: "open-battery", title: "Open Battery", description: "Show the battery and power settings", page: PageId::Battery },
    SettingsIntent { id: "open-bluetooth", title: "Open Bluetooth", description: "Show the Bluetooth devices and settings", page: PageId::Bluetooth },
    SettingsIntent { id: "open-control-centre", title: "Open Control Centre", description: "Show which controls the Control Centre holds", page: PageId::ControlCentre },
    SettingsIntent { id: "open-passcode", title: "Open Passcode", description: "Show the passcode and lock settings", page: PageId::Passcode },
    SettingsIntent { id: "open-passwords", title: "Open Passwords", description: "Show the saved passwords", page: PageId::Passwords },
    SettingsIntent { id: "open-date-time", title: "Open Date & Time", description: "Show the date, time and time zone settings", page: PageId::DateTime },
    SettingsIntent { id: "open-wallpaper", title: "Open Wallpaper", description: "Show the desktop picture settings", page: PageId::Wallpaper },
    SettingsIntent { id: "open-displays", title: "Open Displays", description: "Show the display and brightness settings", page: PageId::Display },
    SettingsIntent { id: "open-keyboard", title: "Open Keyboard", description: "Show the keyboard settings", page: PageId::Keyboard },
    SettingsIntent { id: "open-keyboard-shortcuts", title: "Open Keyboard Shortcuts", description: "Show the keyboard shortcuts the shell answers to", page: PageId::Shortcuts },
    SettingsIntent { id: "open-login-items", title: "Open Login Items", description: "Show the apps that start at login and the app extensions", page: PageId::LoginItems },
];

/// The intent with this id, if the catalogue has one.
pub fn find(id: &str) -> Option<&'static SettingsIntent> {
    CATALOGUE.iter().find(|intent| intent.id == id)
}

/// The pane an intent id opens, if it names a settings intent.
pub fn page_for(id: &str) -> Option<PageId> {
    find(id).map(|intent| intent.page)
}

/// Whether an id is a safe intent id: non-empty, lower-case ASCII letters, digits, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The catalogue as `[[intent]]` manifest tables, one per intent, for a manifest-style listing.
pub fn manifest_lines() -> Vec<String> {
    CATALOGUE
        .iter()
        .map(|intent| {
            format!(
                "[[intent]]\nid = \"{}\"\ntitle = \"{}\"\ndescription = \"{}\"\nshortcut = true\n",
                intent.id, intent.title, intent.description
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_valid_and_unique() {
        for intent in CATALOGUE {
            assert!(valid_id(intent.id), "bad id {}", intent.id);
            assert_eq!(CATALOGUE.iter().filter(|other| other.id == intent.id).count(), 1, "duplicate {}", intent.id);
        }
    }

    #[test]
    fn every_intent_has_a_title_and_description() {
        for intent in CATALOGUE {
            assert!(!intent.title.is_empty());
            assert!(!intent.description.is_empty());
        }
    }

    #[test]
    fn find_and_page_for_resolve_catalogue_entries() {
        assert_eq!(page_for("open-keyboard"), Some(PageId::Keyboard));
        assert_eq!(page_for("open-login-items"), Some(PageId::LoginItems));
        assert_eq!(find("open-displays").map(|i| i.title), Some("Open Displays"));
        assert_eq!(page_for("missing"), None);
        assert_eq!(find(""), None);
    }

    #[test]
    fn valid_id_rejects_unsafe_text() {
        assert!(valid_id("open-date-time"));
        assert!(valid_id("a_b1"));
        assert!(!valid_id(""));
        assert!(!valid_id("Open"));
        assert!(!valid_id("a\"b"));
        assert!(!valid_id("a b"));
    }

    #[test]
    fn manifest_lists_one_shortcut_table_per_intent() {
        let lines = manifest_lines();
        assert_eq!(lines.len(), CATALOGUE.len());
        assert!(lines[0].starts_with("[[intent]]\nid = \"open-sound\""));
        assert!(lines.iter().all(|t| t.contains("shortcut = true")));
    }
}
