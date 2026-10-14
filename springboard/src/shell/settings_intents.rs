//! Settings intents: one App Intent per settings pane that Search, a Shortcut or an agent can open by name. The table
//! is data. Each intent id follows the manifest rule of `intents::ManifestIntent::runnable` (`[a-z0-9-_]`), so the
//! shell can list these intents as parameterless shortcuts. Opening the page is `PageId`, the same route the
//! Settings search uses (`settings_search`).
//!
//! Scope: the table and its lookup. The Settings app does not yet answer `intent.<id>` on its control socket.

use super::intents::ManifestIntent;
use super::pages::PageId;

/// One settings intent. The RPC method is `intent.<id>` and running it opens `page`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsIntent {
    pub id: &'static str,
    pub title: &'static str,
    pub page: PageId,
}

/// Every settings intent the shell knows, one per pane that has a page of its own.
pub const INTENTS: &[SettingsIntent] = &[
    SettingsIntent { id: "open-sound", title: "Open Sound Settings", page: PageId::Sounds },
    SettingsIntent { id: "open-privacy", title: "Open Privacy Settings", page: PageId::Privacy },
    SettingsIntent { id: "open-screen-time", title: "Open Screen Time Settings", page: PageId::ScreenTime },
    SettingsIntent { id: "open-storage", title: "Open Storage Settings", page: PageId::Storage },
    SettingsIntent { id: "open-sharing", title: "Open Sharing Settings", page: PageId::Sharing },
    SettingsIntent { id: "open-software-update", title: "Open Software Update Settings", page: PageId::SoftwareUpdate },
];

/// The page the intent `id` opens, or `None` when `id` is not a settings intent.
pub fn page_for(id: &str) -> Option<PageId> {
    INTENTS.iter().find(|i| i.id == id).map(|i| i.page)
}

/// The Search shortcut entry for an intent: no parameters, so the shell can run it without input.
pub fn manifest_intent(intent: &SettingsIntent) -> ManifestIntent {
    ManifestIntent { id: intent.id.into(), title: intent.title.into(), description: String::new(), shortcut: true, params: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_intent_id_is_a_runnable_shortcut() {
        for intent in INTENTS {
            let m = manifest_intent(intent);
            assert!(m.runnable(), "{} must be runnable from Search", intent.id);
        }
    }

    #[test]
    fn intent_ids_and_titles_are_unique_and_nonempty() {
        for (i, a) in INTENTS.iter().enumerate() {
            assert!(!a.title.is_empty());
            for b in &INTENTS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.title, b.title);
            }
        }
    }

    #[test]
    fn lookup_returns_the_page_of_a_known_intent() {
        assert_eq!(page_for("open-sound"), Some(PageId::Sounds));
        assert_eq!(page_for("open-storage"), Some(PageId::Storage));
        assert_eq!(page_for("open-software-update"), Some(PageId::SoftwareUpdate));
    }

    #[test]
    fn lookup_rejects_unknown_and_malformed_ids() {
        assert_eq!(page_for("open-spotlight"), None);
        assert_eq!(page_for(""), None);
        assert_eq!(page_for("Open-Sound"), None);
    }

    #[test]
    fn manifest_entry_carries_the_title_without_parameters() {
        let m = manifest_intent(&INTENTS[0]);
        assert_eq!(m.id, "open-sound");
        assert_eq!(m.title, "Open Sound Settings");
        assert!(m.shortcut && m.params.is_empty());
    }
}
