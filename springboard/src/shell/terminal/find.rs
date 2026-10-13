//! The Find bar.
use super::*;

/// The Find query a selection gives (Use Selection for Find): its first non-blank line, trimmed, at most 256 characters.
pub fn query_from_selection(selection: &str) -> Option<String> {
    let line = selection.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(256).collect())
}

impl Shell {
    /// Searches the scrollback for the selected text: the Find bar opens with it as the query and jumps to a match.
    pub(super) fn terminal_use_selection_for_find(&mut self, t: f64) {
        let query = self.term.as_ref().and_then(|term| term.selected_text()).and_then(|text| query_from_selection(&text));
        let Some(query) = query else {
            self.terminal_ui.message = "Select terminal text first".into();
            return;
        };
        if self.terminal_ui.panel != Panel::None && self.terminal_ui.panel != Panel::Find {
            self.terminal_close_panel(t);
        }
        self.terminal_action(1, t);
        if let Some(field) = &mut self.terminal_ui.query {
            field.set_text(&self.ui, &query);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_filled_line_of_a_selection_is_the_query() {
        assert_eq!(query_from_selection("  needle  
second").as_deref(), Some("needle"));
        assert_eq!(query_from_selection("
 
  hay stack 
").as_deref(), Some("hay stack"));
        assert_eq!(query_from_selection(""), None);
        assert_eq!(query_from_selection(" 
	"), None);
        assert_eq!(query_from_selection(&"x".repeat(300)).map(|q| q.len()), Some(256));
    }
}
