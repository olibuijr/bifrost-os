//! Keyboard shortcuts: Cmd or Ctrl chords and keys typed while a name is being edited.
use crate::*;

impl Files {
    /// Keys while a name is being typed. Returns false for keys it does not use.
    pub(crate) fn naming_key(&mut self, code: u32) -> bool {
        match code {
            13 => {
                self.commit_rename();
                return true;
            }
            27 => {
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
    /// Cmd or Ctrl chords. Returns false when the chord is not one of ours.
    pub(crate) fn chord(&mut self, key: u32, mods: u32) -> bool {
        let shift = mods & CHORD_SHIFT != 0;
        if mods & CHORD_META != 0 && self.naming.is_none() && self.go_chord(key, shift, mods & CHORD_ALT != 0) {
            return true;
        }
        if mods & CHORD_ALT != 0 || mods & (CHORD_CTRL | CHORD_META) == 0 || self.naming.is_some() {
            return false;
        }
        if key == 31 && mods & CHORD_CTRL != 0 && mods & CHORD_META != 0 {
            self.toggle_sidebar(); // ⌃⌘S: View > Show/Hide Sidebar
            return true;
        }
        match (key, shift) {
            (14, false) => self.trash_selected(),  // Backspace
            (28, false) => self.start_rename(),    // Return
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
            (20, false) => self.open_tags(),       // T
            (2, false) => self.set_view(true, false),  // 1: icons
            (3, false) => self.set_view(false, false), // 2: list
            (4, false) => self.set_view(false, true),  // 3: columns
            (5, false) => self.set_gallery(),          // 4: gallery
            (36, false) => {                           // J: Show View Options
                self.vo.show = !self.vo.show;
                self.revision += 1;
            }
            _ => return false,
        }
        true
    }
}
