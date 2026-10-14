//! Marks in the Terminal: set a mark on a line and jump between marks (the shell's prompt reports mark themselves, see
//! `term::marks`).
use super::*;

/// What the message says after a jump found nothing.
pub fn no_mark_message(marks: usize, forward: bool) -> &'static str {
    match (marks, forward) {
        (0, _) => "No marks yet. Mark a line from More or with Cmd+U",
        (_, true) => "No later mark",
        (_, false) => "No earlier mark",
    }
}

impl Shell {
    /// Marks the line the caret is on, or removes the mark there.
    pub(super) fn terminal_toggle_mark(&mut self) {
        self.terminal_ui.message = match self.term.as_mut().and_then(|tab| tab.toggle_mark()) {
            Some(true) => "Marked this line".into(),
            Some(false) => "Mark removed".into(),
            None => "Full-screen programs keep no marks".into(),
        };
        self.dirty = true;
    }

    /// Scrolls to the previous or next mark.
    pub(super) fn terminal_jump_mark(&mut self, forward: bool) {
        let moved = self.term.as_mut().is_some_and(|tab| tab.jump_mark(forward));
        if moved {
            self.terminal_ui.message.clear();
        } else {
            let marks = self.term.as_ref().map_or(0, |tab| tab.mark_count());
            self.terminal_ui.message = no_mark_message(marks, forward).into();
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_jump_says_why() {
        assert!(no_mark_message(0, true).starts_with("No marks yet"));
        assert_eq!(no_mark_message(0, true), no_mark_message(0, false));
        assert_eq!(no_mark_message(3, true), "No later mark");
        assert_eq!(no_mark_message(3, false), "No earlier mark");
    }
}
