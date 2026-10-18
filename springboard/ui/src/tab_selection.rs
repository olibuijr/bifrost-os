//! `TabView` selection: the ordered tab identifiers and the one selected tab. Pure model; the view draws the tab bar
//! and calls these methods. Selecting an unknown tab is refused, and removing the selected tab selects the tab that
//! takes its place (or the last tab when the removed one was last), as a tab bar does.

/// The tabs of a `TabView` and which one is selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabSelection {
    ids: Vec<String>,
    selected: usize,
}

impl TabSelection {
    /// The tabs in the given order, the first one selected. Repeated identifiers are dropped after their first use.
    pub fn new<S: AsRef<str>>(ids: &[S]) -> Self {
        let mut out: Vec<String> = Vec::new();
        for id in ids {
            let id = id.as_ref();
            if !out.iter().any(|o| o == id) {
                out.push(id.to_string());
            }
        }
        Self { ids: out, selected: 0 }
    }

    /// The tab identifiers, in tab-bar order.
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// The identifier of the selected tab, or `None` when there are no tabs.
    pub fn selected_id(&self) -> Option<&str> {
        self.ids.get(self.selected).map(String::as_str)
    }

    /// Selects the tab `id`. Returns false, and keeps the current selection, when there is no such tab.
    pub fn select(&mut self, id: &str) -> bool {
        match self.ids.iter().position(|t| t == id) {
            Some(index) => {
                self.selected = index;
                true
            }
            None => false,
        }
    }

    /// Removes the tab `id`. Returns false when there is no such tab. The selection follows the tab it was on, or
    /// moves to the neighbour that takes its place.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.ids.iter().position(|t| t == id) else {
            return false;
        };
        self.ids.remove(index);
        if self.ids.is_empty() {
            self.selected = 0;
        } else if index < self.selected {
            self.selected -= 1;
        } else if index == self.selected && self.selected >= self.ids.len() {
            self.selected = self.ids.len() - 1;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_the_first_tab_and_drops_repeated_identifiers() {
        let t = TabSelection::new(&["a", "b", "a"]);
        assert_eq!(t.ids().len(), 2);
        assert_eq!(t.selected_id(), Some("a"));
        assert_eq!(TabSelection::new::<&str>(&[]).selected_id(), None);
    }

    #[test]
    fn select_moves_to_a_known_tab_only() {
        let mut t = TabSelection::new(&["a", "b"]);
        assert!(t.select("b"));
        assert_eq!(t.selected_id(), Some("b"));
        assert!(!t.select("z"));
        assert_eq!(t.selected_id(), Some("b"));
    }

    #[test]
    fn removing_the_selected_tab_selects_its_successor_or_the_last_tab() {
        let mut t = TabSelection::new(&["a", "b", "c"]);
        assert!(t.select("c"));
        assert!(t.remove("c"));
        assert_eq!(t.selected_id(), Some("b"));
        assert!(t.select("a"));
        assert!(t.remove("a"));
        assert_eq!(t.selected_id(), Some("b"));
        assert!(t.remove("b"));
        assert_eq!(t.selected_id(), None);
        assert!(!t.remove("b"));
    }

    #[test]
    fn removing_a_tab_before_the_selected_one_keeps_the_selection() {
        let mut t = TabSelection::new(&["a", "b", "c"]);
        assert!(t.select("c"));
        assert!(t.remove("a"));
        assert_eq!(t.selected_id(), Some("c"));
    }
}
