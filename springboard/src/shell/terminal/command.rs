//! New command: a command line typed in the Terminal runs in a new tab. The tab closes when the command ends.
use super::*;

impl Shell {
    /// Opens the New command panel with an empty command line and the keyboard up.
    pub(super) fn terminal_open_command(&mut self, t: f64) {
        self.terminal_ui.command = Some(TextField::new(&self.ui, "Command", 14.0, false, 512));
        self.terminal_ui.panel = Panel::Command;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Runs the typed line through `/bin/sh -c` in a new tab, then closes the panel. An empty line stays open.
    pub(super) fn terminal_run_command(&mut self, t: f64) {
        let line = self
            .terminal_ui
            .command
            .as_ref()
            .map(|field| field.text().trim().to_string())
            .unwrap_or_default();
        if line.is_empty() {
            self.terminal_ui.message = "Type a command to run".into();
            return;
        }
        let argv = ["/bin/sh", "-c", line.as_str()];
        if self.spawn_terminal_tab(&argv, None) {
            self.terminal_ui.command = None;
            self.terminal_close_panel(t);
        }
    }
}
