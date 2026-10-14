//! Edit Title: a name for the front tab. An empty name restores the automatic title (the program's
//! title, else the running job, else the shell number).
use super::*;

impl Shell {
    /// Opens the Edit Title panel with the tab's current name (empty when it has none) and the keyboard up.
    pub(super) fn terminal_open_title(&mut self, t: f64) {
        let current = self
            .term
            .as_ref()
            .and_then(|term| term.custom_title().map(str::to_string))
            .unwrap_or_default();
        let mut field = TextField::new(&self.ui, "Tab title", 14.0, false, 256);
        field.set_text(&self.ui, &current);
        self.terminal_ui.tab_title = Some(field);
        self.terminal_ui.panel = Panel::Title;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Gives the front tab the typed name (trimmed; empty restores the automatic title) and closes the panel.
    pub(super) fn terminal_save_title(&mut self, t: f64) {
        let name = self
            .terminal_ui
            .tab_title
            .as_ref()
            .map(|field| field.text().trim().to_string());
        if let (Some(name), Some(term)) = (name, self.term.as_mut()) {
            term.set_custom_title(Some(name));
        }
        self.terminal_ui.tab_title = None;
        self.terminal_close_panel(t);
    }
}
