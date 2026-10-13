//! Window groups: the folders of the open Terminal tabs, saved as one set and opened again as one tab per folder.
//! One group is kept (the last one saved), in the `terminal.window_group` pref.
use super::super::prefs::{escape_field, unescape_field};
use super::*;

/// The pref that holds the saved group.
pub const KEY: &str = "terminal.window_group";

/// The folders as one single-line pref value: each escaped, joined by tabs, in tab order.
pub fn encode(dirs: &[String]) -> String {
    dirs.iter().filter(|dir| !dir.is_empty()).map(|dir| escape_field(dir)).collect::<Vec<_>>().join("\t")
}

/// The folders of a saved group, in order. A folder whose escapes do not decode is dropped.
pub fn decode(value: &str) -> Vec<String> {
    value.split('\t').filter(|field| !field.is_empty()).filter_map(|field| unescape_field(field, 4096)).collect()
}

impl Shell {
    /// Saves the folder of every open tab as the window group.
    pub(super) fn terminal_save_group(&mut self) {
        let dirs: Vec<String> = self.term.iter().filter_map(|tab| tab.cwd()).collect();
        self.terminal_ui.message = match self.prefs.set(KEY, &encode(&dirs)) {
            Ok(()) => format!("Saved {} tab{} to the window group", dirs.len(), if dirs.len() == 1 { "" } else { "s" }),
            Err(e) => format!("Could not save window group: {e}"),
        };
        self.dirty = true;
    }

    /// Opens one tab in each folder of the saved group and closes the panel; no saved group says so.
    pub(super) fn terminal_open_group(&mut self, t: f64) {
        let dirs = decode(self.prefs.get(KEY).unwrap_or(""));
        if dirs.is_empty() {
            self.terminal_ui.message = "No window group saved".into();
            return;
        }
        for dir in dirs {
            self.new_terminal_tab(Some(dir));
        }
        self.terminal_close_panel(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_round_trips_folders_with_tabs_and_backslashes() {
        let dirs = vec!["/home/a b".to_string(), "/tmp/with\ttab".to_string(), "/x\\y".to_string()];
        let value = encode(&dirs);
        assert!(!value.contains('\n') && !value.contains('\r'), "one pref line");
        assert_eq!(decode(&value), dirs);
    }

    #[test]
    fn an_empty_group_has_no_folders_and_blank_folders_are_skipped() {
        assert_eq!(encode(&[]), "");
        assert!(decode("").is_empty());
        assert_eq!(decode(&encode(&["".to_string(), "/a".to_string()])), vec!["/a".to_string()]);
    }
}
