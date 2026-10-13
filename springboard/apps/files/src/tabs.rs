//! Tabs: several folder views in one window (File > New Tab, ⌘T; ⌘W closes; ⌃Tab and ⌃⇧Tab step through).
//! The open tab's navigation (path, history) lives in `Files`; the other tabs are parked as `TabState` in the
//! `TabStrip`. Switching parks the open tab's state, then shows the chosen one through `navigate`.
use crate::*;

/// The navigation a parked tab keeps: where it is and the history to go back and forward through.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabState {
    pub(crate) path: PathBuf,
    pub(crate) history: Vec<PathBuf>,
    pub(crate) at: usize,
}

/// The tab strip model: the tabs in order and which one is open. Holds the open tab too, kept current by `park`.
#[derive(Debug)]
pub(crate) struct TabStrip<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> TabStrip<T> {
    /// A strip with one tab, which is open.
    pub(crate) fn new(first: T) -> Self {
        Self { items: vec![first], active: 0 }
    }
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
    pub(crate) fn active(&self) -> usize {
        self.active
    }
    pub(crate) fn items(&self) -> &[T] {
        &self.items
    }
    /// Replaces the open tab's stored state with `item` (called before the open tab is left).
    pub(crate) fn park(&mut self, item: T) {
        self.items[self.active] = item;
    }
    /// Adds a tab right after the open one and makes it the open tab. Returns its index.
    pub(crate) fn insert_after_active(&mut self, item: T) -> usize {
        let at = self.active + 1;
        self.items.insert(at, item);
        self.active = at;
        at
    }
    /// Makes tab `i` the open tab. Returns false when there is no such tab.
    pub(crate) fn select(&mut self, i: usize) -> bool {
        if i >= self.items.len() {
            return false;
        }
        self.active = i;
        true
    }
    /// Steps to the next (or previous) tab, wrapping around. Returns the new open tab.
    pub(crate) fn step(&mut self, forward: bool) -> usize {
        let n = self.items.len();
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
        self.active
    }
    /// Closes tab `i`; the last tab cannot be closed. The open tab moves to the tab that takes its place,
    /// or to the one before it when the last tab closes. Returns the removed item.
    pub(crate) fn close(&mut self, i: usize) -> Option<T> {
        if self.items.len() <= 1 || i >= self.items.len() {
            return None;
        }
        let item = self.items.remove(i);
        if i < self.active {
            self.active -= 1;
        } else if i == self.active {
            self.active = i.min(self.items.len() - 1);
        }
        Some(item)
    }
}

impl Files {
    /// The open tab's navigation, as the strip stores it.
    fn tab_state(&self) -> TabState {
        TabState { path: self.path.clone(), history: self.history.clone(), at: self.at }
    }
    /// Shows the open tab's stored navigation: its folder with its own history.
    fn show_tab(&mut self) {
        let Some(tab) = self.tabs.items().get(self.tabs.active()).cloned() else {
            return;
        };
        self.history = tab.history;
        self.at = tab.at;
        self.navigate(tab.path, false);
    }
    /// File > New Tab (⌘T): a tab at the open folder, which becomes the open tab.
    pub(crate) fn new_tab(&mut self) {
        let state = self.tab_state();
        self.tabs.park(state);
        let fresh = TabState { path: self.path.clone(), history: vec![self.path.clone()], at: 0 };
        self.tabs.insert_after_active(fresh);
        self.show_tab();
        self.revision += 1;
    }
    /// Closes the open tab (⌘W). The last tab stays open.
    pub(crate) fn close_tab(&mut self) {
        let open = self.tabs.active();
        if self.tabs.close(open).is_some() {
            self.show_tab();
            self.revision += 1;
        }
    }
    /// Steps to the next or previous tab (⌃Tab and ⌃⇧Tab).
    pub(crate) fn step_tab(&mut self, forward: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let state = self.tab_state();
        self.tabs.park(state);
        self.tabs.step(forward);
        self.show_tab();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tab_opens_after_the_open_one_and_becomes_open() {
        let mut strip = TabStrip::new("a");
        assert_eq!(strip.insert_after_active("b"), 1);
        assert_eq!(strip.active(), 1);
        strip.select(0);
        assert_eq!(strip.insert_after_active("c"), 1);
        assert_eq!(strip.items(), &["a", "c", "b"]);
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut strip = TabStrip::new(1);
        strip.insert_after_active(2);
        strip.insert_after_active(3);
        assert_eq!(strip.active(), 2);
        assert_eq!(strip.step(true), 0);
        assert_eq!(strip.step(false), 2);
        assert_eq!(strip.step(false), 1);
    }

    #[test]
    fn the_last_tab_cannot_close_and_the_open_tab_moves_to_a_neighbour() {
        let mut strip = TabStrip::new('x');
        assert_eq!(strip.close(0), None);
        strip.insert_after_active('y');
        strip.insert_after_active('z');
        // open is 'z' (index 2); closing an earlier tab keeps 'z' open
        assert_eq!(strip.close(0), Some('x'));
        assert_eq!(strip.items(), &['y', 'z']);
        assert_eq!(strip.active(), 1);
        // closing the open last tab falls back to the one before it
        assert_eq!(strip.close(1), Some('z'));
        assert_eq!(strip.active(), 0);
        assert!(!strip.select(5));
    }

    #[test]
    fn parking_keeps_each_tabs_own_state() {
        let mut strip = TabStrip::new(TabState { path: "/a".into(), history: vec!["/a".into()], at: 0 });
        strip.park(TabState { path: "/a/b".into(), history: vec!["/a".into(), "/a/b".into()], at: 1 });
        assert_eq!(strip.items()[0].path, PathBuf::from("/a/b"));
        assert_eq!(strip.items()[0].at, 1);
    }
}
