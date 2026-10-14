//! Settings panes as App Intents: one intent per pane a Settings extension offers to Shortcuts and Search.
//! The table is plain data. Each intent has a stable id (the name a deep link or the control socket uses), a
//! title for a Shortcuts list, and the pane it opens. `find` resolves an id to its intent, and `PaneIntent::page`
//! is the page the shell pushes when the intent runs.

use super::pages::PageId;

/// One intent that opens a Settings pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneIntent {
    /// lower-case id with dashes and no spaces, such as "control-centre"
    pub id: &'static str,
    /// the title a Shortcuts list shows
    pub title: &'static str,
    /// the page the intent opens
    pub page: PageId,
}

/// The pane intents, in the order Shortcuts lists them.
pub const PANE_INTENTS: &[PaneIntent] = &[
    PaneIntent { id: "about", title: "About", page: PageId::About },
    PaneIntent { id: "account", title: "BifrOSt Account", page: PageId::Root },
    PaneIntent { id: "accessibility", title: "Accessibility", page: PageId::Accessibility },
    PaneIntent { id: "accounts", title: "Users & Groups", page: PageId::Users },
    PaneIntent { id: "appearance", title: "Appearance", page: PageId::Appearance },
    PaneIntent { id: "battery", title: "Battery", page: PageId::Battery },
    PaneIntent { id: "bluetooth", title: "Bluetooth", page: PageId::Bluetooth },
    PaneIntent { id: "control-centre", title: "Control Centre", page: PageId::ControlCentre },
    PaneIntent { id: "passcode", title: "Passcode", page: PageId::Passcode },
    PaneIntent { id: "passwords", title: "Passwords", page: PageId::Passwords },
];

/// The intent with this id. The id is matched without case and surrounding spaces.
pub fn find(id: &str) -> Option<&'static PaneIntent> {
    let key = id.trim().to_ascii_lowercase();
    PANE_INTENTS.iter().find(|p| p.id == key)
}

/// The page the intent with this id opens, if there is one.
pub fn page_for(id: &str) -> Option<PageId> {
    find(id).map(|p| p.page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_lower_case_and_dash_separated() {
        for (i, p) in PANE_INTENTS.iter().enumerate() {
            assert!(!p.id.is_empty());
            assert!(p.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "bad id {}", p.id);
            assert!(!PANE_INTENTS[..i].iter().any(|q| q.id == p.id), "duplicate id {}", p.id);
        }
    }

    #[test]
    fn every_intent_has_a_title() {
        assert!(PANE_INTENTS.iter().all(|p| !p.title.trim().is_empty()));
    }

    #[test]
    fn find_ignores_case_and_surrounding_spaces() {
        assert_eq!(find(" Battery ").map(|p| p.page), Some(PageId::Battery));
        assert_eq!(find("CONTROL-CENTRE").map(|p| p.page), Some(PageId::ControlCentre));
        assert_eq!(find("nope"), None);
        assert_eq!(find(""), None);
    }

    #[test]
    fn page_for_maps_each_pane() {
        assert_eq!(page_for("about"), Some(PageId::About));
        assert_eq!(page_for("appearance"), Some(PageId::Appearance));
        assert_eq!(page_for("bluetooth"), Some(PageId::Bluetooth));
        assert_eq!(page_for("accessibility"), Some(PageId::Accessibility));
        assert_eq!(page_for("passwords"), Some(PageId::Passwords));
        assert_eq!(page_for("account"), Some(PageId::Root), "the account card sits on the root page");
        assert_eq!(page_for("optical"), None);
    }
}
