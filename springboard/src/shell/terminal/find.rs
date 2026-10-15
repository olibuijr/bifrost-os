//! The Find bar.
use super::*;

impl Shell {
    /// Cmd+E: the selected text becomes the Find query and the match is shown.
    pub(super) fn terminal_use_selection_for_find(&mut self, t: f64) {
        let line = self
            .term
            .as_ref()
            .and_then(|term| term.selected_text())
            .and_then(|text| text.lines().next().map(|l| l.trim().to_string()))
            .unwrap_or_default();
        if line.is_empty() {
            self.terminal_ui.message = "Select terminal text first".into();
            return;
        }
        if !matches!(self.terminal_ui.panel, Panel::None | Panel::Find) {
            self.terminal_close_panel(t);
        }
        self.terminal_action(1, t);
        if let Some(field) = self.terminal_ui.query.as_mut() {
            field.set_text(&self.ui, &line);
        }
        self.terminal_find(Direction::Older, true);
    }

    pub(super) fn terminal_find(&mut self, dir: Direction, restart: bool) {
        let query = self
            .terminal_ui
            .query
            .as_ref()
            .map(|q| q.text())
            .unwrap_or_default();
        let found = self
            .term
            .as_mut()
            .is_some_and(|term| term.find(query, dir, restart));
        self.terminal_ui.message = if query.is_empty() {
            String::new()
        } else if found {
            let (at, total) = self.term.as_ref().map_or((0, 0), |term| term.match_position(&query));
            if total == 0 { String::new() } else { format!("{at} of {total}") }
        } else {
            "No matches".into()
        };
    }
}
