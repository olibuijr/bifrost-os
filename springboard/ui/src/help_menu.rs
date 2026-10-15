//! Help menu entries contributed by a settings pane. A pane adds its entries when it is selected and removes them when
//! it is unselected, so the Help menu lists the help of the pane in front and nothing else. Each entry has a title and
//! an optional anchor into the pane's help. The model is pure: the shell draws the menu rows and opens the anchor.

/// One Help menu entry of a pane: the title shown in the menu and the anchor in the pane's help that it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpEntry {
    pub title: String,
    pub anchor: Option<String>,
}

impl HelpEntry {
    /// An entry with a trimmed, non-empty title and an optional anchor. None when the title is blank or the anchor is
    /// present but blank.
    pub fn new(title: &str, anchor: Option<&str>) -> Option<Self> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let anchor = match anchor.map(str::trim) {
            None => None,
            Some("") => return None,
            Some(a) => Some(a.to_string()),
        };
        Some(HelpEntry { title: title.to_string(), anchor })
    }
}

/// The Help menu entries of the panes that are selected. Entries are listed in the order their pane was last updated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneHelpMenu {
    panes: Vec<(String, Vec<HelpEntry>)>,
}

impl PaneHelpMenu {
    /// A menu with no pane entries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the entries of `pane`, replacing any it already had. The pane moves to the end of the menu order. Returns
    /// false, and changes nothing, when the pane identifier is blank.
    pub fn update(&mut self, pane: &str, entries: Vec<HelpEntry>) -> bool {
        let pane = pane.trim();
        if pane.is_empty() {
            return false;
        }
        self.panes.retain(|(p, _)| p != pane);
        self.panes.push((pane.to_string(), entries));
        true
    }

    /// Removes the entries of `pane`. Returns false when the pane had none.
    pub fn remove(&mut self, pane: &str) -> bool {
        let pane = pane.trim();
        let before = self.panes.len();
        self.panes.retain(|(p, _)| p != pane);
        self.panes.len() != before
    }

    /// Every entry, pane by pane in menu order, then in the order each pane gave them.
    pub fn entries(&self) -> impl Iterator<Item = &HelpEntry> {
        self.panes.iter().flat_map(|(_, entries)| entries.iter())
    }

    /// The first entry whose title equals `title`, ignoring ASCII case.
    pub fn entry_titled(&self, title: &str) -> Option<&HelpEntry> {
        self.entries().find(|e| e.title.eq_ignore_ascii_case(title.trim()))
    }

    /// True when no selected pane contributes an entry.
    pub fn is_empty(&self) -> bool {
        self.entries().next().is_none()
    }
}
