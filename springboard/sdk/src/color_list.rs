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
use bifrost_render::appearance::Pair;
use bifrost_render::ui::system;
use bifrost_ui::panels::ColorPanel;

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

    /// The built-in System list: the named system colours (red, orange, yellow, green, mint, teal, cyan, blue, indigo,
    /// purple, pink, brown, gray) with their light or dark values.
    pub fn system(dark: bool) -> ColorList {
        let pick = |p: Pair| {
            let v = if dark { p.dark } else { p.light };
            Color::from_rgba8((v & 0xff) as u8, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8, (v >> 24) as u8)
        };
        let mut list = ColorList::new("System");
        for (name, pair) in [
            ("Red", system::SYSTEM_RED),
            ("Orange", system::SYSTEM_ORANGE),
            ("Yellow", system::SYSTEM_YELLOW),
            ("Green", system::SYSTEM_GREEN),
            ("Mint", system::SYSTEM_MINT),
            ("Teal", system::SYSTEM_TEAL),
            ("Cyan", system::SYSTEM_CYAN),
            ("Blue", system::SYSTEM_BLUE),
            ("Indigo", system::SYSTEM_INDIGO),
            ("Purple", system::SYSTEM_PURPLE),
            ("Pink", system::SYSTEM_PINK),
            ("Brown", system::SYSTEM_BROWN),
            ("Gray", system::SYSTEM_GRAY),
        ] {
            list.set_color(name, pick(pair));
        }
        list
    }

    /// The swatches a colour well shows for this list: each key with its straight 8-bit sRGB colour and alpha, in the
    /// list's order.
    pub fn swatches(&self) -> Vec<(String, [u8; 4])> {
        self.entries.iter().map(|(k, c)| (k.clone(), c.to_rgba8())).collect()
    }

    /// Puts the colour under `key` into a colour panel (its colour and its opacity), as picking a swatch does. Returns
    /// false and leaves the panel alone when the key is absent.
    pub fn apply_to_panel(&self, key: &str, panel: &mut ColorPanel) -> bool {
        let Some(color) = self.color(key) else { return false };
        let [r, g, b, a] = color.to_rgba8();
        panel.set_rgb((r, g, b));
        panel.set_alpha(a as f32 / 255.0);
        true
    }

    /// The list as text for a file: a `list <name>` line, then one `<key><TAB>#rrggbbaa` line per colour in order.
    /// Colours are written in 8-bit sRGB, so a colour in another space is converted.
    pub fn to_text(&self) -> String {
        let mut out = format!("list {}\n", self.name);
        for (key, color) in &self.entries {
            let [r, g, b, a] = color.to_rgba8();
            out.push_str(&format!("{key}\t#{r:02x}{g:02x}{b:02x}{a:02x}\n"));
        }
        out
    }

    /// Reads text written by `to_text`. Returns None when the first line is not `list <name>` with a name. A line that
    /// is not `<key><TAB>#rrggbbaa` (or has an empty key) is skipped, so a damaged file keeps what can be read.
    pub fn from_text(text: &str) -> Option<ColorList> {
        let mut lines = text.lines();
        let name = lines.next()?.strip_prefix("list ")?.trim();
        if name.is_empty() {
            return None;
        }
        let mut list = ColorList::new(name);
        for line in lines {
            let Some((key, hex)) = line.split_once('\t') else { continue };
            let Some(hex) = hex.trim().strip_prefix('#') else { continue };
            if hex.len() != 8 || !hex.is_ascii() {
                continue;
            }
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            if let (Some(r), Some(g), Some(b), Some(a)) = (byte(0), byte(2), byte(4), byte(6)) {
                list.set_color(key, Color::from_rgba8(r, g, b, a));
            }
        }
        Some(list)
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

    /// A registry that holds the built-in System list, as every app starts with.
    pub fn standard(dark: bool) -> ColorLists {
        let mut lists = ColorLists::new();
        lists.add(ColorList::system(dark));
        lists
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

    #[test]
    fn the_system_list_holds_the_named_colours_in_both_appearances() {
        let dark = ColorList::system(true);
        let light = ColorList::system(false);
        assert_eq!(dark.len(), 13);
        assert_eq!(dark.keys()[0], "Red");
        assert_eq!(dark.color("Red").map(|c| c.to_rgba8()), Some([255, 69, 58, 255]));
        assert_eq!(light.color("Red").map(|c| c.to_rgba8()), Some([255, 59, 48, 255]));
        assert_eq!(light.color("Blue").map(|c| c.to_rgba8()), Some([0, 122, 255, 255]));
        assert_eq!(ColorLists::standard(false).names(), vec!["System"]);
    }

    #[test]
    fn a_list_round_trips_through_text_in_order() {
        let mut list = ColorList::new("Brand colours");
        list.set_color("Zebra", Color::from_rgba8(1, 2, 3, 255));
        list.set_color("Azure", Color::from_rgba8(10, 132, 255, 128));
        let back = ColorList::from_text(&list.to_text()).expect("reads back");
        assert_eq!(back.name(), "Brand colours");
        assert_eq!(back.keys(), vec!["Zebra", "Azure"]);
        assert_eq!(back.color("Azure").map(|c| c.to_rgba8()), Some([10, 132, 255, 128]));
        assert_eq!(back.color("Zebra").map(|c| c.to_rgba8()), Some([1, 2, 3, 255]));
    }

    #[test]
    fn damaged_text_keeps_what_can_be_read() {
        assert!(ColorList::from_text("").is_none());
        assert!(ColorList::from_text("colours\nA\t#ff000000").is_none(), "no header");
        assert!(ColorList::from_text("list \n").is_none(), "no name");
        let list = ColorList::from_text("list Mixed\nGood\t#00ff00ff\nbroken line\nShort\t#fff\nBadHex\t#zzzzzzzz\n\t#ff0000ff\nAlso good\t#0000ffff\n").unwrap();
        assert_eq!(list.keys(), vec!["Good", "Also good"]);
        assert_eq!(list.color("Good").map(|c| c.to_rgba8()), Some([0, 255, 0, 255]));
    }

    #[test]
    fn picking_a_swatch_sets_the_colour_panel() {
        use std::{cell::Cell, rc::Rc};
        let list = ColorList::system(false);
        let mut panel = ColorPanel::new((0, 0, 0));
        let fired = Rc::new(Cell::new(0));
        let seen = fired.clone();
        panel.on_change(move |_, _| seen.set(seen.get() + 1));
        assert!(list.apply_to_panel("Green", &mut panel));
        assert_eq!(panel.color(), (52, 199, 89));
        assert_eq!(panel.alpha(), 1.0);
        assert!(fired.get() >= 1, "the panel reports the change");
        assert!(!list.apply_to_panel("Missing", &mut panel));
        assert_eq!(panel.color(), (52, 199, 89), "an absent key leaves the panel alone");
        let swatches = list.swatches();
        assert_eq!(swatches.len(), 13);
        assert_eq!(swatches[0], ("Red".to_string(), [255, 59, 48, 255]));
    }
}
