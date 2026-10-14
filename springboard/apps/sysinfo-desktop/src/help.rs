//! The in-app help of System Information: a short card that says what each part of the window does and which keys
//! and menu items reach it. Help > System Information Help shows the card in place of the facts; choosing a category
//! in the sidebar, or starting a find, brings the facts back.

use crate::model::{kv, Row};
use crate::window::SysInfo;

/// The line under the title while the help card is shown.
pub const SUBTITLE: &str = "Help for System Information. Choose a category in the sidebar to return to the facts.";

/// The rows of the help card, one key and one value each.
pub fn rows() -> Vec<Row> {
    vec![
        kv("Sidebar", "Choose a category. The arrow keys move the selection and Tab moves the focus between the sidebar, the item list and the card."),
        kv("Sections", "Click a section header to fold or unfold its categories. The choice is kept for the next launch."),
        kv("Item list", "Categories that list devices, disks or apps show an item list above the card. Click a column header to sort by it; a second click reverses the order."),
        kv("Card", "The facts of the picked item. Click a row to pick it; Edit > Copy copies the picked rows as text."),
        kv("Find", "Cmd+F opens the find bar and filters the rows as you type. Cmd+G and Cmd+Shift+G step through the categories that match."),
        kv("Selection", "Cmd+E searches for the picked value. Cmd+J scrolls the picked row or item into view."),
        kv("Refresh", "View > Refresh Information (Cmd+R) reads the computer again. While the profile is read, Escape cancels."),
        kv("Show Less Information", "View > Show Less Information keeps only the Overview in the sidebar and the report."),
        kv("Reports", "File > Save Report As writes text, rich text or JSON. File > Open Report reads a saved JSON report, and Open Recent lists the last reports."),
        kv("Serial number", "Edit > Speak Serial Number reads the serial number aloud when a speech engine is installed."),
    ]
}

impl SysInfo {
    /// Help > System Information Help: shows the help card, or hides it when it is already shown. True when the window
    /// changed.
    pub fn toggle_help(&mut self) -> bool {
        self.help = !self.help;
        self.item = 0;
        self.selection = crate::window::Selection::None;
        self.scroll = 0.0;
        self.list_scroll = 0.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Category;

    #[test]
    fn every_help_row_has_a_key_and_a_value() {
        let rows = rows();
        assert!(rows.len() >= 8, "the help names every part of the window");
        assert!(rows.iter().all(|(key, value)| !key.is_empty() && !value.is_empty()));
    }

    #[test]
    fn the_help_keys_are_unique() {
        let rows = rows();
        let mut keys: Vec<&str> = rows.iter().map(|(key, _)| key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), rows.len());
    }

    #[test]
    fn help_replaces_the_facts_until_a_category_is_chosen() {
        let mut w = SysInfo::new();
        assert!(w.toggle_help());
        assert!(w.help);
        assert_eq!(w.rows(), rows(), "the card shows the help rows");
        assert_eq!(w.subtitle(), SUBTITLE);

        assert!(w.select(Category::Memory));
        assert!(!w.help, "choosing a category leaves the help");
        assert_eq!(w.selected, Category::Memory);
    }

    #[test]
    fn choosing_the_shown_category_also_leaves_the_help() {
        let mut w = SysInfo::new();
        w.toggle_help();
        assert!(w.select(Category::Overview), "the same category still leaves the help");
        assert!(!w.help);
        assert!(!w.select(Category::Overview), "a second choice of the same category changes nothing");
    }

    #[test]
    fn a_second_help_choice_hides_the_card() {
        let mut w = SysInfo::new();
        w.toggle_help();
        assert!(w.toggle_help());
        assert!(!w.help);
        assert_ne!(w.rows(), rows());
    }

    #[test]
    fn starting_a_find_leaves_the_help() {
        let mut w = SysInfo::new();
        w.toggle_help();
        assert!(w.begin_find());
        assert!(!w.help, "the find filters the facts, so the help gives way");
    }
}
