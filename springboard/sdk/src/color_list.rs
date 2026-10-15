//! Named colour lists: an ordered set of colours, each under a key, that an app can build, edit and
//! look up by name. [`ColorLists`] is the set of lists an app has registered.
//!
//! ```ignore
//! let mut brand = ColorList::new("Brand");
//! brand.set_color("Accent", Color::from_rgba8(10, 132, 255, 255));
//! let mut lists = ColorLists::new();
//! lists.add(brand);
//! let accent = lists.get("Brand").and_then(|l| l.color("Accent"));
//! ```
//!
//! Keys keep the order they were first set in, so a picker can show the list the way the author wrote it. Setting an
//! existing key replaces its colour in place. Empty keys are refused. Colours are [`graphics_color::Color`] values, so
//! a list can hold colours in any colour space.

use crate::graphics_color::Color;

/// One named list of colours.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorList {
    name: String,
    entries: Vec<(String, Color)>,
}

impl ColorList {
    /// An empty list with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        ColorList { name: name.into(), entries: Vec::new() }
    }

    /// The list's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets `color` under `key`, replacing the colour if the key exists. Returns `false` (and changes nothing) for an
    /// empty key.
    pub fn set_color(&mut self, key: &str, color: Color) -> bool {
        if key.is_empty() {
            return false;
        }
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = color,
            None => self.entries.push((key.to_string(), color)),
        }
        true
    }

    /// The colour stored under `key`.
    pub fn color(&self, key: &str) -> Option<&Color> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    /// Removes `key`. Returns whether it was present.
    pub fn remove_color(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(k, _)| k != key);
        self.entries.len() != before
    }

    /// Moves `key` to position `index` (0 is the first), shifting the colours in between. Returns `false` and changes
    /// nothing when the key is absent or `index` is not a position of the list.
    pub fn move_color(&mut self, key: &str, index: usize) -> bool {
        let Some(from) = self.entries.iter().position(|(k, _)| k == key) else {
            return false;
        };
        if index >= self.entries.len() {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(index, entry);
        true
    }

    /// Keys in the order they were first set, or moved to.
    pub fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// The number of colours in the list.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list has no colours.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The colour lists an app has registered, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorLists {
    lists: Vec<ColorList>,
}

impl ColorLists {
    /// An empty registry.
    pub fn new() -> Self {
        ColorLists::default()
    }

    /// Registers `list`. Returns `false` and keeps the existing list when a list with the same name is already
    /// registered.
    pub fn add(&mut self, list: ColorList) -> bool {
        if self.lists.iter().any(|l| l.name == list.name) {
            return false;
        }
        self.lists.push(list);
        true
    }

    /// The list named `name`.
    pub fn get(&self, name: &str) -> Option<&ColorList> {
        self.lists.iter().find(|l| l.name == name)
    }

    /// The list named `name`, for editing.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut ColorList> {
        self.lists.iter_mut().find(|l| l.name == name)
    }

    /// Removes the list named `name`. Returns whether it was registered.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.lists.len();
        self.lists.retain(|l| l.name != name);
        self.lists.len() != before
    }

    /// Names of the registered lists, in registration order.
    pub fn names(&self) -> Vec<&str> {
        self.lists.iter().map(|l| l.name()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Color {
        Color::from_rgba8(255, 0, 0, 255)
    }

    fn blue() -> Color {
        Color::from_rgba8(0, 0, 255, 255)
    }

    #[test]
    fn new_list_is_empty_and_keeps_its_name() {
        let list = ColorList::new("Brand");
        assert_eq!(list.name(), "Brand");
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert!(list.keys().is_empty());
    }

    #[test]
    fn keys_keep_first_insertion_order() {
        let mut list = ColorList::new("Brand");
        assert!(list.set_color("Zebra", red()));
    assert!(list.set_color("Azure", blue()));
    assert_eq!(list.keys(), vec!["Zebra", "Azure"]);
    }

    #[test]
    fn setting_an_existing_key_replaces_the_colour_in_place() {
        let mut list = ColorList::new("Brand");
        list.set_color("Accent", red());
        list.set_color("Other", blue());
        assert!(list.set_color("Accent", blue()));
        assert_eq!(list.len(), 2);
        assert_eq!(list.keys(), vec!["Accent", "Other"]);
        assert_eq!(list.color("Accent"), Some(&blue()));
    }

    #[test]
    fn empty_key_is_refused() {
        let mut list = ColorList::new("Brand");
        assert!(!list.set_color("", red()));
        assert!(list.is_empty());
    }

    #[test]
    fn unknown_key_has_no_colour() {
        let list = ColorList::new("Brand");
        assert!(list.color("Missing").is_none());
    }

    #[test]
    fn remove_reports_whether_the_key_was_present() {
        let mut list = ColorList::new("Brand");
        list.set_color("Accent", red());
        assert!(list.remove_color("Accent"));
        assert!(!list.remove_color("Accent"));
        assert!(list.is_empty());
    }

    #[test]
    fn registry_refuses_duplicate_names_and_keeps_the_first() {
        let mut lists = ColorLists::new();
        let mut first = ColorList::new("Brand");
        first.set_color("Accent", red());
        assert!(lists.add(first));
        assert!(!lists.add(ColorList::new("Brand")));
        assert_eq!(lists.get("Brand").and_then(|l| l.color("Accent")), Some(&red()));
    }

    #[test]
    fn registry_lists_names_in_registration_order_and_removes_by_name() {
        let mut lists = ColorLists::new();
        lists.add(ColorList::new("Project"));
        lists.add(ColorList::new("Brand"));
        assert_eq!(lists.names(), vec!["Project", "Brand"]);
        assert!(lists.remove("Project"));
        assert!(!lists.remove("Project"));
        assert_eq!(lists.names(), vec!["Brand"]);
    }

    #[test]
    fn get_mut_edits_the_registered_list() {
        let mut lists = ColorLists::new();
        lists.add(ColorList::new("Brand"));
        lists.get_mut("Brand").unwrap().set_color("Accent", blue());
        assert_eq!(lists.get("Brand").unwrap().keys(), vec!["Accent"]);
        assert!(lists.get_mut("Missing").is_none());
    }
}
