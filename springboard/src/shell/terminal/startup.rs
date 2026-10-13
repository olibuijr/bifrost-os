//! Login shell and startup: which shell new tabs run, and whether the saved window group opens when the Terminal starts.
use super::*;
use std::os::unix::fs::PermissionsExt;

/// The pref that holds the chosen shell.
pub const SHELL_KEY: &str = "terminal.shell";
/// The pref that switches the window group on at launch.
pub const LAUNCH_KEY: &str = "terminal.group_launch";

/// The shell a typed path names. Ok(None) for an empty line (the account's own login shell); an error says what is wrong.
pub fn validate_shell(typed: &str) -> Result<Option<String>, String> {
    let path = typed.trim();
    if path.is_empty() {
        return Ok(None);
    }
    if !path.starts_with('/') || path.chars().any(char::is_control) {
        return Err("Type the full path, for example /bin/bash".into());
    }
    let meta = fs::metadata(path).map_err(|_| format!("{path} does not exist"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(format!("{path} is not a program"));
    }
    Ok(Some(path.to_string()))
}

/// The folders of the tabs to open when the Terminal starts: those of the saved window group when that is switched on
/// and a group is saved, else one tab in the home folder (None).
pub fn launch_dirs(group_on: bool, saved_group: &str) -> Vec<Option<String>> {
    let dirs = groups::decode(saved_group);
    if group_on && !dirs.is_empty() {
        dirs.into_iter().map(Some).collect()
    } else {
        vec![None]
    }
}

impl Shell {
    /// The shell new tabs run: the chosen one while it is still a program, else None for the account's login shell.
    /// Only the default command is replaced; a Terminal started on another command keeps it.
    pub(crate) fn terminal_login_shell(&self) -> Option<String> {
        if self.term_cmd != crate::term::SHELL {
            return None;
        }
        validate_shell(self.terminal_ui.login_shell.as_deref()?).ok().flatten()
    }

    /// Opens the first tabs of a Terminal start: the saved window group when it is switched on, else a single tab.
    pub(crate) fn terminal_launch_tabs(&mut self) {
        let dirs = launch_dirs(self.terminal_ui.group_launch, self.prefs.get(groups::KEY).unwrap_or(""));
        for dir in dirs {
            self.new_terminal_tab(dir);
        }
    }

    /// Opens the Login shell panel with the chosen path (empty: the account's shell).
    pub(super) fn terminal_open_shell_panel(&mut self, t: f64) {
        let current = self.terminal_ui.login_shell.clone().unwrap_or_default();
        let mut field = TextField::new(&self.ui, "Shell path (empty: account shell)", 14.0, false, 512);
        field.set_text(&self.ui, &current);
        self.terminal_ui.entry = Some(field);
        self.terminal_ui.panel = Panel::Shell;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Saves the typed shell; new tabs run it from now on. A path that is not a program stays open with the reason.
    pub(super) fn terminal_apply_shell(&mut self, t: f64) {
        let typed = self.terminal_ui.entry.as_ref().map(|field| field.text().to_string()).unwrap_or_default();
        match validate_shell(&typed) {
            Ok(shell) => {
                let saved = self.prefs.set(SHELL_KEY, shell.as_deref().unwrap_or(""));
                self.terminal_ui.login_shell = shell.clone();
                self.terminal_ui.message = match (saved, shell) {
                    (Err(e), _) => format!("Could not save the shell: {e}"),
                    (Ok(()), Some(path)) => format!("New tabs run {path}"),
                    (Ok(()), None) => "New tabs run the account shell".into(),
                };
                self.terminal_ui.entry = None;
                self.terminal_close_panel(t);
            }
            Err(why) => self.terminal_ui.message = why,
        }
    }

    /// Turns "open the window group at launch" on or off and saves it.
    pub(super) fn terminal_toggle_group_launch(&mut self) {
        self.terminal_ui.group_launch = !self.terminal_ui.group_launch;
        let on = self.terminal_ui.group_launch;
        self.terminal_ui.message = match self.prefs.set(LAUNCH_KEY, if on { "1" } else { "0" }) {
            Ok(()) if on => "The window group opens when the Terminal starts".into(),
            Ok(()) => "The Terminal starts with one tab".into(),
            Err(e) => format!("Could not save: {e}"),
        };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_must_be_a_full_path_to_a_program() {
        assert_eq!(validate_shell(""), Ok(None), "empty means the account's shell");
        assert_eq!(validate_shell("  \t "), Ok(None));
        assert_eq!(validate_shell(" /bin/sh "), Ok(Some("/bin/sh".to_string())));
        assert!(validate_shell("bash").unwrap_err().starts_with("Type the full path"));
        assert!(validate_shell("/bin/sh\n-c").is_err());
        assert!(validate_shell("/no/such/shell").unwrap_err().contains("does not exist"));
        assert!(validate_shell("/etc/passwd").unwrap_err().contains("not a program"), "a file without the execute bit");
        assert!(validate_shell("/tmp").unwrap_err().contains("not a program"), "a folder");
    }

    #[test]
    fn the_window_group_opens_at_launch_only_when_switched_on_and_saved() {
        let group = groups::encode(&["/a".to_string(), "/b".to_string()]);
        assert_eq!(launch_dirs(true, &group), vec![Some("/a".to_string()), Some("/b".to_string())]);
        assert_eq!(launch_dirs(false, &group), vec![None], "off: one tab");
        assert_eq!(launch_dirs(true, ""), vec![None], "nothing saved: one tab");
    }
}
