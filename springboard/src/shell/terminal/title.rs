//! Edit Title: a name typed for the active tab replaces the title the program sets.
use super::*;

impl Shell {
    /// Opens the Edit title panel with the tab's current title and the keyboard up.
    pub(super) fn terminal_open_title(&mut self, t: f64) {
        // only a title the user typed is offered for editing; the automatic one stays automatic unless replaced
        let current = self.term.as_ref().and_then(|tab| tab.custom_title()).unwrap_or_default().to_string();
        let mut field = TextField::new(&self.ui, "Tab title (empty: automatic)", 14.0, false, 64);
        field.set_text(&self.ui, &current);
        self.terminal_ui.entry = Some(field);
        self.terminal_ui.panel = Panel::Title;
        self.terminal_ui.message.clear();
        self.terminal_ui.scroll = 0.0;
        self.set_keyboard(true, t);
    }

    /// Gives the active tab the typed title; an empty line puts the automatic title back.
    pub(super) fn terminal_apply_title(&mut self, t: f64) {
        let typed = self.terminal_ui.entry.as_ref().map(|field| field.text().to_string()).unwrap_or_default();
        if let Some(tab) = self.term.as_mut() {
            tab.set_custom_title(&typed);
        }
        let named = self.term.as_ref().is_some_and(|tab| tab.custom_title().is_some());
        self.terminal_ui.entry = None;
        self.terminal_close_panel(t);
        self.terminal_ui.message = if named { "Tab title set".into() } else { "Tab title is automatic".into() };
    }
}
