//! The Terminal's smaller panels: the Profiles picker, saved window groups, All Tabs, the Startup command, the name of a
//! window group, export names for the scrollback and the selection, and the Find bar's options.
use super::groups;
use super::profiles::{self, Choice};
use super::startup;
use super::*;

/// The pref that holds the Find bar's ignore-case option ("0" when off).
pub const IGNORE_CASE_KEY: &str = "terminal.find_ignore_case";
/// The pref that holds the Find bar's wrap-around option ("0" when off).
pub const WRAP_KEY: &str = "terminal.find_wrap";

/// The file stem an export name gives: blanks trimmed, a `.txt` ending dropped, no folders and no hidden names. Err
/// says what is wrong with the name.
pub fn export_stem(typed: &str) -> Result<String, String> {
    let name = typed.trim();
    let name = name.strip_suffix(".txt").unwrap_or(name).trim_end();
    if name.is_empty() {
        return Err("Type a name for the file".into());
    }
    if name.len() > 100 || name.starts_with('.') || name.chars().any(|c| c == '/' || c.is_control()) {
        return Err("Use a file name without / or control characters".into());
    }
    Ok(name.to_string())
}

impl Shell {
    /// The text of the open entry field (Title, Shell, Colors, Startup, the group name and the export name).
    pub(super) fn terminal_entry_text(&self) -> String {
        self.terminal_ui.entry.as_ref().map(|field| field.text().to_string()).unwrap_or_default()
    }

    /// Applies the entry panel that is open: its Enter key, or its button.
    pub(super) fn terminal_apply_entry(&mut self, t: f64) {
        match self.terminal_ui.panel {
            Panel::Title => self.terminal_apply_title(t),
            Panel::Shell => self.terminal_apply_shell(t),
            Panel::Startup => self.terminal_apply_startup(t),
            Panel::GroupName => self.terminal_apply_group_name(t),
            Panel::Export => self.terminal_apply_export(t),
            _ => self.terminal_apply_colors(t),
        }
    }

    /// Opens a one-line entry panel with `value` in its field.
    fn terminal_open_entry(&mut self, panel: Panel, label: &str, value: &str, max: usize, t: f64) {
        let mut field = TextField::new(&self.ui, label, 14.0, false, max);
        field.set_text(&self.ui, value);
        self.terminal_ui.entry = Some(field);
        self.terminal_ui.panel = panel;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Opens the Profiles panel: every colour theme and profile, with the one in use marked.
    pub(super) fn terminal_open_profiles(&mut self, t: f64) {
        self.terminal_ui.panel = Panel::Profiles;
        self.terminal_ui.scroll = 0.0;
        self.terminal_ui.message.clear();
        self.set_keyboard(false, t);
    }

    /// The Profiles rows, in order.
    pub(super) fn terminal_profile_choices(&self) -> Vec<Choice> {
        let custom = usize::from(self.terminal_ui.custom_colors.is_some());
        profiles::choices(THEMES.len(), profile_table::PROFILES.len() + custom)
    }

    /// Uses the chosen theme or profile in every tab and keeps it.
    pub(super) fn terminal_apply_choice(&mut self, choice: Choice) {
        match choice {
            Choice::Theme(theme) => {
                self.terminal_ui.settings.theme = theme.min(THEMES.len() - 1);
                self.terminal_ui.profile = None;
            }
            Choice::Profile(profile) => self.terminal_ui.profile = Some(profile),
        }
        self.terminal_appearance();
        self.terminal_ui.message = format!("Using {}", profiles::choice_name(choice));
        self.dirty = true;
    }

    /// Makes the profile in use the one a Terminal starts with (the numbered theme when no profile is in use).
    pub(super) fn terminal_set_default_profile(&mut self) {
        let value = self.terminal_ui.profile.map(|p| p.to_string()).unwrap_or_default();
        self.terminal_ui.message = match self.prefs.set(profiles::DEFAULT_KEY, &value) {
            Ok(()) => match self.terminal_ui.profile {
                Some(p) => format!("The Terminal starts with {}", profiles::scheme_name(self.terminal_ui.settings.theme, Some(p))),
                None => "The Terminal starts with the numbered theme".into(),
            },
            Err(e) => format!("Could not save the default: {e}"),
        };
        self.dirty = true;
    }

    /// Opens the Window groups panel, or says that none is saved.
    pub(super) fn terminal_open_groups(&mut self, t: f64) {
        if self.terminal_groups().is_empty() {
            self.terminal_ui.message = "No window group saved".into();
            return;
        }
        self.terminal_ui.panel = Panel::Groups;
        self.terminal_ui.scroll = 0.0;
        self.terminal_ui.message.clear();
        self.terminal_ui.group_delete = None;
        self.set_keyboard(false, t);
    }

    /// A tap on a group row opens the group; a tap on its cross asks once and then deletes it.
    pub(super) fn terminal_group_tapped(&mut self, index: usize, delete: bool, t: f64) {
        let groups = self.terminal_groups();
        let Some(group) = groups.get(index).cloned() else { return };
        if !delete {
            self.terminal_ui.group_delete = None;
            if self.terminal_open_group_tabs(index) {
                self.terminal_close_panel(t);
                self.terminal_ui.message = format!("Opened {}", group.name);
            }
        } else if self.terminal_ui.group_delete != Some(index) {
            self.terminal_ui.group_delete = Some(index);
            self.terminal_ui.message = format!("Tap the cross again to delete {}", group.name);
        } else {
            self.terminal_ui.group_delete = None;
            self.terminal_ui.message = self.terminal_delete_group(index);
            if self.terminal_groups().is_empty() {
                self.terminal_close_panel(t);
            }
        }
        self.dirty = true;
    }

    /// Opens the All Tabs panel: one row per open tab.
    pub(super) fn terminal_open_tabs(&mut self, t: f64) {
        self.terminal_ui.panel = Panel::Tabs;
        self.terminal_ui.scroll = 0.0;
        self.terminal_ui.message.clear();
        self.set_keyboard(false, t);
    }

    /// Shows tab `index` and closes the panel.
    pub(super) fn terminal_show_tab(&mut self, index: usize, t: f64) {
        if index < self.term.len() {
            self.term.select(index);
            self.terminal_close_panel(t);
            self.terminal_ui.message.clear();
        }
    }

    /// Opens the Startup command panel with the command in use.
    pub(super) fn terminal_open_startup(&mut self, t: f64) {
        let current = self.terminal_ui.startup_command.clone().unwrap_or_default();
        self.terminal_open_entry(Panel::Startup, "Command (empty: none)", &current, 512, t);
    }

    /// Saves the typed command; the first tab runs it each time the Terminal starts. Control characters are dropped.
    pub(super) fn terminal_apply_startup(&mut self, t: f64) {
        let typed = self.terminal_entry_text();
        let command: String = typed.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
        let saved = self.prefs.set(startup::COMMAND_KEY, &command);
        self.terminal_ui.message = match saved {
            Err(e) => format!("Could not save the command: {e}"),
            Ok(()) if command.is_empty() => "No command runs at launch".into(),
            Ok(()) => "The command runs when the Terminal starts".into(),
        };
        self.terminal_ui.startup_command = (!command.is_empty()).then_some(command);
        self.terminal_ui.entry = None;
        self.terminal_close_panel(t);
    }

    /// Opens the panel that names a window group; the open tabs are saved under that name.
    pub(super) fn terminal_open_group_name(&mut self, t: f64) {
        let suggested = format!("Window group {}", self.terminal_groups().len() + 1);
        self.terminal_open_entry(Panel::GroupName, "Group name", &suggested, 64, t);
    }

    /// Saves the open tabs as the typed group; a name that is taken replaces that group.
    pub(super) fn terminal_apply_group_name(&mut self, t: f64) {
        let typed = self.terminal_entry_text();
        match groups::group_name(&typed) {
            Ok(name) => {
                self.terminal_ui.message = self.terminal_save_group_named(&name);
                self.terminal_ui.entry = None;
                self.terminal_close_panel(t);
            }
            Err(why) => self.terminal_ui.message = why,
        }
    }

    /// Opens the panel that names an export: the scrollback (`selection` false) or the selected text.
    pub(super) fn terminal_open_export(&mut self, selection: bool, t: f64) {
        let (stem, text) = if selection {
            match self.term.as_ref().and_then(|tab| tab.selected_text()).filter(|text| !text.is_empty()) {
                Some(text) => ("terminal-selection", text),
                None => {
                    self.terminal_ui.message = "Select terminal text first".into();
                    return;
                }
            }
        } else {
            ("terminal-scrollback", self.term.as_ref().map(|tab| tab.transcript()).unwrap_or_default())
        };
        self.terminal_ui.export = Some((stem.to_string(), text));
        self.terminal_open_entry(Panel::Export, "File name", stem, 128, t);
    }

    /// Writes the pending export to Files under the typed name; an earlier file of that name is kept and the new one
    /// gets a number.
    pub(super) fn terminal_apply_export(&mut self, t: f64) {
        let typed = self.terminal_entry_text();
        let Some((_, text)) = self.terminal_ui.export.clone() else {
            self.terminal_close_panel(t);
            return;
        };
        match export_stem(&typed) {
            Ok(stem) => {
                self.terminal_ui.message = self.terminal_save_text(&stem, &text);
                self.terminal_ui.export = None;
                self.terminal_ui.entry = None;
                self.terminal_close_panel(t);
            }
            Err(why) => self.terminal_ui.message = why,
        }
    }

    /// Turns the Find bar's ignore-case option on or off and saves it.
    pub(super) fn terminal_toggle_ignore_case(&mut self) {
        self.terminal_ui.find_ignore_case = !self.terminal_ui.find_ignore_case;
        let on = self.terminal_ui.find_ignore_case;
        self.terminal_ui.message = match self.prefs.set(IGNORE_CASE_KEY, if on { "1" } else { "0" }) {
            Ok(()) if on => "Find ignores case".into(),
            Ok(()) => "Find matches the case exactly".into(),
            Err(e) => format!("Could not save: {e}"),
        };
        self.dirty = true;
    }

    /// Turns the Find bar's wrap-around option on or off and saves it.
    pub(super) fn terminal_toggle_wrap(&mut self) {
        self.terminal_ui.find_wrap = !self.terminal_ui.find_wrap;
        let on = self.terminal_ui.find_wrap;
        self.terminal_ui.message = match self.prefs.set(WRAP_KEY, if on { "1" } else { "0" }) {
            Ok(()) if on => "Find wraps around the scrollback".into(),
            Ok(()) => "Find stops at the ends of the scrollback".into(),
            Err(e) => format!("Could not save: {e}"),
        };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_name_is_a_plain_file_name() {
        assert_eq!(export_stem("  build log.txt ").as_deref(), Ok("build log"));
        assert_eq!(export_stem("notes").as_deref(), Ok("notes"));
        assert!(export_stem("  ").is_err());
        assert!(export_stem(".txt").is_err(), "nothing but the ending");
        assert!(export_stem(".hidden").is_err(), "a hidden file");
        assert!(export_stem("a/b").is_err(), "no folders");
        assert!(export_stem("a\nb").is_err());
        assert!(export_stem(&"x".repeat(101)).is_err());
    }
}
