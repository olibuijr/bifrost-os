//! The commands of the Photos menus. The desktop bar shows them as rows of File, Edit, View and Image while Photos
//! is in front, and the hardware keyboard reaches them through the same key equivalents. Which rows are on, checked
//! or named differently depends on the app's state, which `MenuState` carries so the menu can be built (and tested)
//! without the app.

use super::rules::KindFilter;

/// A command of the Photos menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    NewAlbum,
    NewFolder,
    NewSmartAlbum,
    Import,
    Export,
    PlaySlideshow,
    Undo,
    Redo,
    CopyEdits,
    PasteEdits,
    SelectAll,
    DeselectAll,
    Find,
    ZoomIn,
    ZoomOut,
    AspectGrid,
    Histogram,
    ShowHidden,
    Filter(KindFilter),
    ClearFilters,
    ShowInfo,
    Library,
    Settings,
    Edit,
    /// the share sheet lists the apps that open the picture
    EditWith,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    AutoEnhance,
    Favourite,
    Hide,
    Duplicate,
    /// 0 clears the rating
    Rate(u8),
    AdjustDate,
    EditDetails,
    CopyLocation,
    Delete,
}

/// The menus Photos adds rows to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    File,
    Edit,
    View,
    Image,
}

/// What the menus need to know about the app.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// a photo is open in the viewer
    pub viewing: bool,
    /// the viewer or the selection gives the photos image commands act on
    pub has_targets: bool,
    /// the editor is open
    pub editing: bool,
    pub selecting: bool,
    pub undo_title: String,
    pub redo_title: String,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_paste_edits: bool,
    pub aspect_grid: bool,
    pub histogram: bool,
    pub show_hidden: bool,
    pub filter: KindFilter,
    pub favourite: bool,
    pub hidden: bool,
    pub rating: u8,
    pub has_location: bool,
    pub trash: bool,
    pub zoom_in: bool,
    pub zoom_out: bool,
}

/// One row of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub cmd: Option<Cmd>,
    pub label: String,
    /// key equivalent text for `KeyCombo::parse`, "" for none
    pub key: &'static str,
    pub checked: bool,
    pub enabled: bool,
    pub separator: bool,
}

impl Item {
    fn new(cmd: Cmd, label: &str, key: &'static str, enabled: bool) -> Item {
        Item { cmd: Some(cmd), label: label.to_string(), key, checked: false, enabled, separator: false }
    }

    fn checked(mut self, on: bool) -> Item {
        self.checked = on;
        self
    }

    fn sep() -> Item {
        Item { cmd: None, label: String::new(), key: "", checked: false, enabled: false, separator: true }
    }

    fn heading(label: &str) -> Item {
        Item { cmd: None, label: label.to_string(), key: "", checked: false, enabled: false, separator: false }
    }
}

/// The rows of `menu` for the app in state `s`.
pub fn items(menu: MenuKind, s: &MenuState) -> Vec<Item> {
    let acts = s.has_targets && !s.editing && !s.trash;
    let tweak = s.has_targets && !s.trash;
    match menu {
        MenuKind::File => vec![
            Item::new(Cmd::NewAlbum, "New Album", "cmd+n", !s.editing),
            Item::new(Cmd::NewFolder, "New Folder", "cmd+shift+n", !s.editing),
            Item::new(Cmd::NewSmartAlbum, "New Smart Album\u{2026}", "cmd+alt+n", !s.editing),
            Item::sep(),
            Item::new(Cmd::Import, "Import\u{2026}", "cmd+shift+i", !s.editing),
            Item::new(Cmd::Export, "Export Photos\u{2026}", "cmd+shift+e", s.has_targets && !s.editing),
            Item::sep(),
            Item::new(Cmd::PlaySlideshow, "Play Slideshow", "", !s.editing),
        ],
        MenuKind::Edit => vec![
            Item::new(Cmd::Undo, &s.undo_title, "cmd+z", s.can_undo),
            Item::new(Cmd::Redo, &s.redo_title, "cmd+shift+z", s.can_redo),
            Item::sep(),
            Item::new(Cmd::CopyEdits, "Copy Edits", "cmd+shift+c", s.has_targets && !s.editing),
            Item::new(Cmd::PasteEdits, "Paste Edits", "cmd+shift+v", acts && s.can_paste_edits),
            Item::sep(),
            Item::new(Cmd::SelectAll, "Select All", "cmd+a", !s.editing),
            Item::new(Cmd::DeselectAll, "Deselect All", "cmd+shift+a", s.selecting),
            Item::sep(),
            Item::new(Cmd::Find, "Find\u{2026}", "cmd+f", !s.editing),
        ],
        MenuKind::View => {
            let mut v = vec![
                Item::new(Cmd::ZoomIn, "Zoom In", "cmd+=", s.zoom_in),
                Item::new(Cmd::ZoomOut, "Zoom Out", "cmd+-", s.zoom_out),
                Item::sep(),
                Item::new(Cmd::AspectGrid, "Aspect Ratio Grid", "", true).checked(s.aspect_grid),
                Item::new(Cmd::Histogram, "Show Histogram", "", true).checked(s.histogram),
                Item::new(Cmd::ShowHidden, "Show Hidden Photo Album", "", true).checked(s.show_hidden),
                Item::sep(),
                Item::heading("Filter By"),
            ];
            v.extend(KindFilter::ALL.into_iter().map(|k| Item::new(Cmd::Filter(k), k.label(), "", true).checked(s.filter == k)));
            v.extend([Item::sep(), Item::new(Cmd::ShowInfo, "Show Info", "cmd+i", s.viewing), Item::new(Cmd::Library, "Library and Tools\u{2026}", "", true), Item::new(Cmd::Settings, "Settings\u{2026}", "", true)]);
            v
        }
        MenuKind::Image => {
            let mut v = vec![
                Item::new(Cmd::Edit, "Edit Photo", "", s.viewing && !s.editing && !s.trash),
                Item::new(Cmd::EditWith, "Edit With\u{2026}", "", s.viewing && !s.editing && !s.trash),
                Item::sep(),
                Item::new(Cmd::RotateLeft, "Rotate Left", "cmd+r", tweak),
                Item::new(Cmd::RotateRight, "Rotate Right", "cmd+alt+r", tweak),
                Item::new(Cmd::FlipHorizontal, "Flip Horizontal", "", tweak),
                Item::new(Cmd::FlipVertical, "Flip Vertical", "", tweak),
                Item::new(Cmd::AutoEnhance, "Auto Enhance", "", tweak),
                Item::sep(),
                Item::new(Cmd::Favourite, if s.favourite { "Unfavorite" } else { "Favorite" }, "cmd+.", acts),
                Item::new(Cmd::Hide, if s.hidden { "Unhide Photo" } else { "Hide Photo" }, "cmd+l", acts),
                Item::new(Cmd::Duplicate, "Duplicate", "cmd+d", acts),
                Item::sep(),
                Item::heading("Star Rating"),
            ];
            for r in 0..=5u8 {
                let label = if r == 0 { "None".to_string() } else { format!("{r} Star{}", if r == 1 { "" } else { "s" }) };
                v.push(Item::new(Cmd::Rate(r), &label, ["cmd+0", "cmd+1", "cmd+2", "cmd+3", "cmd+4", "cmd+5"][r as usize], acts).checked(s.rating == r));
            }
            v.extend([
                Item::sep(),
                Item::new(Cmd::AdjustDate, "Adjust Date and Time\u{2026}", "", s.viewing && !s.editing && !s.trash),
                Item::new(Cmd::EditDetails, "Title, Caption and Keywords\u{2026}", "", s.viewing && !s.editing && !s.trash),
                Item::new(Cmd::CopyLocation, "Copy Location", "", s.viewing && s.has_location),
                Item::sep(),
                Item::new(Cmd::Delete, "Delete Photo", "cmd+backspace", s.has_targets && !s.editing),
            ]);
            v
        }
    }
}

impl Cmd {
    /// The command of a Cmd chord: the evdev key code, with Shift and Alt held or not. None for chords Photos does
    /// not use (the shell's own, such as Cmd+W, are taken before this is asked).
    pub fn from_chord(code: u16, shift: bool, alt: bool) -> Option<Cmd> {
        Some(match (code, shift, alt) {
            (49, false, false) => Cmd::NewAlbum,
            (49, true, false) => Cmd::NewFolder,
            (49, false, true) => Cmd::NewSmartAlbum,
            (23, true, false) => Cmd::Import,
            (23, false, false) => Cmd::ShowInfo,
            (18, true, false) => Cmd::Export,
            (33, false, false) => Cmd::Find,
            (46, true, false) => Cmd::CopyEdits,
            (47, true, false) => Cmd::PasteEdits,
            (30, true, false) => Cmd::DeselectAll,
            // Cmd+= and Cmd++ both enlarge
            (13, _, false) => Cmd::ZoomIn,
            (12, false, false) => Cmd::ZoomOut,
            (19, false, false) => Cmd::RotateLeft,
            (19, false, true) => Cmd::RotateRight,
            (32, false, false) => Cmd::Duplicate,
            (38, false, false) => Cmd::Hide,
            (52, false, false) => Cmd::Favourite,
            (11, false, false) => Cmd::Rate(0),
            (2..=6, false, false) => Cmd::Rate((code - 1) as u8),
            (14, false, false) => Cmd::Delete,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> MenuState {
        MenuState { viewing: true, has_targets: true, undo_title: "Undo".into(), redo_title: "Redo".into(), zoom_in: true, zoom_out: true, ..MenuState::default() }
    }

    fn find(v: &[Item], cmd: Cmd) -> &Item {
        v.iter().find(|i| i.cmd == Some(cmd)).unwrap_or_else(|| panic!("no row for {cmd:?}"))
    }

    #[test]
    fn image_rows_need_a_photo_and_follow_its_state() {
        let mut s = state();
        let v = items(MenuKind::Image, &s);
        assert!(find(&v, Cmd::RotateLeft).enabled);
        assert_eq!(find(&v, Cmd::Favourite).label, "Favorite");
        s.favourite = true;
        s.hidden = true;
        s.rating = 3;
        let v = items(MenuKind::Image, &s);
        assert_eq!(find(&v, Cmd::Favourite).label, "Unfavorite");
        assert_eq!(find(&v, Cmd::Hide).label, "Unhide Photo");
        assert!(find(&v, Cmd::Rate(3)).checked && !find(&v, Cmd::Rate(2)).checked);
        s.has_targets = false;
        s.viewing = false;
        let v = items(MenuKind::Image, &s);
        assert!(!find(&v, Cmd::RotateLeft).enabled && !find(&v, Cmd::Delete).enabled);
        s.trash = true;
        s.has_targets = true;
        assert!(!find(&items(MenuKind::Image, &s), Cmd::Duplicate).enabled, "Recently Deleted photos only recover or go");
    }

    #[test]
    fn edit_menu_carries_the_undo_titles_and_selection_rows() {
        let mut s = state();
        s.undo_title = "Undo Favorite".into();
        s.can_undo = true;
        let v = items(MenuKind::Edit, &s);
        assert_eq!(find(&v, Cmd::Undo).label, "Undo Favorite");
        assert!(find(&v, Cmd::Undo).enabled && !find(&v, Cmd::Redo).enabled);
        assert!(!find(&v, Cmd::DeselectAll).enabled, "nothing is selected");
        s.selecting = true;
        assert!(find(&items(MenuKind::Edit, &s), Cmd::DeselectAll).enabled);
        assert!(!find(&v, Cmd::PasteEdits).enabled, "no copied edits yet");
        s.can_paste_edits = true;
        assert!(find(&items(MenuKind::Edit, &s), Cmd::PasteEdits).enabled);
    }

    #[test]
    fn view_menu_checks_the_current_filter_and_toggles() {
        let mut s = state();
        s.filter = KindFilter::Edited;
        s.aspect_grid = true;
        let v = items(MenuKind::View, &s);
        assert!(find(&v, Cmd::Filter(KindFilter::Edited)).checked);
        assert!(!find(&v, Cmd::Filter(KindFilter::All)).checked);
        assert!(find(&v, Cmd::AspectGrid).checked && !find(&v, Cmd::Histogram).checked);
        s.zoom_in = false;
        assert!(!find(&items(MenuKind::View, &s), Cmd::ZoomIn).enabled, "the grid is at its largest");
    }

    #[test]
    fn every_key_equivalent_is_a_valid_combo_and_unique() {
        let s = state();
        let mut seen: Vec<&str> = Vec::new();
        for m in [MenuKind::File, MenuKind::Edit, MenuKind::View, MenuKind::Image] {
            for i in items(m, &s) {
                if i.key.is_empty() {
                    continue;
                }
                assert!(bifrost_ui::keys::KeyCombo::parse(i.key).is_some(), "{} has a key the menu cannot parse", i.label);
                assert!(!seen.contains(&i.key), "{} repeats {}", i.label, i.key);
                seen.push(i.key);
            }
        }
    }

    #[test]
    fn chords_map_to_the_commands_the_menus_show() {
        assert_eq!(Cmd::from_chord(49, false, false), Some(Cmd::NewAlbum));
        assert_eq!(Cmd::from_chord(49, true, false), Some(Cmd::NewFolder));
        assert_eq!(Cmd::from_chord(13, true, false), Some(Cmd::ZoomIn), "Cmd++");
        assert_eq!(Cmd::from_chord(19, false, true), Some(Cmd::RotateRight));
        assert_eq!(Cmd::from_chord(11, false, false), Some(Cmd::Rate(0)));
        assert_eq!(Cmd::from_chord(2, false, false), Some(Cmd::Rate(1)));
        assert_eq!(Cmd::from_chord(6, false, false), Some(Cmd::Rate(5)));
        assert_eq!(Cmd::from_chord(7, false, false), None);
        assert_eq!(Cmd::from_chord(17, false, false), None, "Cmd+W stays the shell's");
    }
}
