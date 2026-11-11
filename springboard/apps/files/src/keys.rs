//! Keyboard shortcuts: Cmd or Ctrl chords, keys typed while a name is being edited, and type-to-select.
use crate::*;

/// Type-to-select: a letter typed within this long of the one before goes on with the same name; after it a new name
/// starts.
pub(crate) const TYPE_SELECT_GAP: Duration = Duration::from_millis(1000);

/// Type-to-select: the place among `names` (in the order they are shown) of the first one that starts with `typed`,
/// letter case aside. None when no name does, or nothing was typed.
pub(crate) fn type_select_match<'a>(mut names: impl Iterator<Item = &'a str>, typed: &str) -> Option<usize> {
    let typed = typed.to_lowercase();
    if typed.is_empty() {
        return None;
    }
    names.position(|name| name.to_lowercase().starts_with(&typed))
}

impl Files {
    /// Keys while a name is being typed. Returns false for keys it does not use.
    pub(crate) fn naming_key(&mut self, code: u32) -> bool {
        match code {
            13 => {
                self.commit_rename();
                return true;
            }
            KEY_ESCAPE | KEY_ESCAPE_ASCII => {
                self.naming = None;
                return true;
            }
            _ => {}
        }
        let Some((_, field)) = self.naming.as_mut() else {
            return false;
        };
        match code {
            8 => field.pop(),
            46 => field.delete(),
            37 | 39 => field.move_caret(code == 39),
            36 => field.home(),
            35 => field.end(),
            _ => return false,
        }
        true
    }
    /// Whether a space typed at `now` is a letter of the name being typed for type-to-select ("New Folder") and not
    /// the preview key: a name is being typed, and an item shown goes on with a space after what was typed so far.
    pub(crate) fn typing_a_name(&self, now: Instant) -> bool {
        let Some((typed, at)) = self.fx.typed.as_ref() else { return false };
        if now.duration_since(*at) >= TYPE_SELECT_GAP {
            return false;
        }
        let with_space = format!("{typed} ");
        let visible = self.visible();
        type_select_match(visible.iter().map(|&i| self.rows[i].name.as_str()), &with_space).is_some()
    }
    /// Type-to-select, in every item view (icons, list, columns and gallery): `text` typed at `now` goes on with the
    /// name being typed, or starts one after a pause, and the first item shown whose name starts with it becomes the
    /// selection and is scrolled into view (the gallery shows it large). True when an item was found; with none the
    /// selection stays.
    pub(crate) fn type_select(&mut self, text: &str, now: Instant) -> bool {
        if text.is_empty() || text.chars().any(char::is_control) {
            return false;
        }
        let mut typed = match self.fx.typed.take() {
            Some((typed, at)) if now.duration_since(at) < TYPE_SELECT_GAP => typed,
            _ => String::new(),
        };
        typed.push_str(text);
        let visible = self.visible();
        let hit = type_select_match(visible.iter().map(|&i| self.rows[i].name.as_str()), &typed).map(|n| visible[n]);
        self.fx.typed = Some((typed, now));
        let Some(i) = hit else { return false };
        self.select_only(i);
        self.reveal(i);
        self.revision += 1;
        true
    }
    /// Cmd or Ctrl chords. Returns false when the chord is not one of ours.
    pub(crate) fn chord(&mut self, key: u32, mods: u32) -> bool {
        let shift = mods & CHORD_SHIFT != 0;
        if mods & CHORD_META != 0 && self.naming.is_none() && self.go_chord(key, shift, mods & CHORD_ALT != 0) {
            return true;
        }
        if key == 46 && mods & CHORD_META != 0 && mods & CHORD_ALT != 0 && self.naming.is_none() {
            self.copy_item_path(); // Option+Cmd+C: Copy Item Path
            return true;
        }
        if key == 47 && mods & CHORD_META != 0 && mods & CHORD_ALT != 0 && !shift && self.naming.is_none() {
            self.paste_move(); // Option+Cmd+V: move the copied items here
            return true;
        }
        if mods & CHORD_ALT != 0 || mods & (CHORD_CTRL | CHORD_META) == 0 || self.naming.is_some() {
            return false;
        }
        if key == 31 && mods & CHORD_CTRL != 0 && mods & CHORD_META != 0 {
            self.toggle_sidebar(); // ⌃⌘S: View > Show/Hide Sidebar
            return true;
        }
        if key == 30 && mods & CHORD_CTRL != 0 && mods & CHORD_META != 0 && !shift {
            self.make_alias(); // ⌃⌘A: Make Alias, the key the context menu shows (⌘L still works)
            return true;
        }
        match (key, shift) {
            (14, false) => self.trash_selected(),  // Backspace
            (14, true) => self.ask_empty_trash(),  // Shift+Backspace: Empty Trash, after the confirmation sheet
            (28, false) => self.start_rename(),    // Return
            (24, false) => self.open_selected(),   // O: File > Open
            (108, false) => self.open_selected(),  // Down: Open (a chord only while Cmd or Ctrl is held)
            (46, false) => self.copy_selected(false), // C
            (45, false) => self.copy_selected(true),  // X
            (47, false) => self.paste(),           // V
            (32, false) => self.duplicate(),       // D
            (30, false) => self.select_all(),      // A
            (44, false) => self.undo_file_op(),    // Z
            (49, true) => self.new_folder(),       // Shift+N
            (33, false) => self.searching = true,  // F
            (103, false) => self.go_up(),          // Up
            (26, false) => self.go_back(),         // [
            (27, false) => self.go_forward(),      // ]
            (52, true) => self.toggle_hidden(),    // Shift+.
            (53, false) => self.toggle_status_bar(), // /: View > Show Status Bar
            (23, false) => self.get_info(),        // I
            (20, false) => self.new_tab(),         // T: File > New Tab
            (20, true) => self.open_tags(),        // Shift+T: Tags
            (17, false) => self.close_tab(),       // W: Close Tab
            (15, false) => self.step_tab(true),    // Ctrl+Tab: next tab (Tab key)
            (15, true) => self.step_tab(false),    // Ctrl+Shift+Tab: previous tab
            (2, false) => self.set_view(true, false),  // 1: icons
            (3, false) => self.set_view(false, false), // 2: list
            (4, false) => self.set_view(false, true),  // 3: columns
            (5, false) => self.set_gallery(),          // 4: gallery
            (38, false) => self.make_alias(),          // L: Make Alias
            (36, false) => {                           // J: Show View Options
                self.vo.show = !self.vo.show;
                self.revision += 1;
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, folder};

    #[test]
    fn a_typed_name_matches_the_first_item_that_starts_with_it() {
        let names = ["Alpha", "beta", "Bravo"];
        assert_eq!(type_select_match(names.iter().copied(), "b"), Some(1));
        assert_eq!(type_select_match(names.iter().copied(), "BR"), Some(2), "letter case aside");
        assert_eq!(type_select_match(names.iter().copied(), "x"), None);
        assert_eq!(type_select_match(names.iter().copied(), ""), None);
    }

    #[test]
    fn typing_a_name_selects_the_item_in_every_view() {
        let names = ["Alpha", "beta", "Bravo", "charlie", "New Folder"];
        for view in ["icons", "list", "columns", "gallery"] {
            let mut files = fixture(names.iter().map(|n| folder(n)).collect());
            files.grid = view == "icons";
            files.columns = view == "columns";
            files.vo.gallery = view == "gallery";
            let t0 = Instant::now();
            assert!(files.type_select("b", t0));
            assert_eq!(files.selection, vec![1], "{view}: b is beta");
            assert!(!files.typing_a_name(t0), "{view}: no name goes on with a space after b: the space is the preview key");
            assert!(files.type_select("r", t0 + Duration::from_millis(200)));
            assert_eq!(files.selection, vec![2], "{view}: br is Bravo");
            assert!(!files.type_select("z", t0 + Duration::from_millis(400)));
            assert_eq!(files.selection, vec![2], "{view}: no name starts with brz, the selection stays");
            // after a pause a new name starts
            let later = t0 + Duration::from_millis(400) + TYPE_SELECT_GAP;
            assert!(!files.typing_a_name(later));
            assert!(files.type_select("c", later));
            assert_eq!(files.selection, vec![3], "{view}: a new name after the pause");
            // a space is a letter of a name being typed
            let again = later + TYPE_SELECT_GAP;
            for letter in ["n", "e", "w"] {
                assert!(files.type_select(letter, again));
            }
            assert!(files.typing_a_name(again));
            assert!(files.type_select(" ", again));
            assert_eq!(files.selection, vec![4], "{view}: New Folder");
            // a control character is no letter
            assert!(!files.type_select("\u{8}", again));
            if view == "gallery" {
                let visible = files.visible();
                assert_eq!(files.gallery_current(&visible), Some(4), "the gallery shows the item large");
            }
        }
    }
}
