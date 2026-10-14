//! Tab model: closing and replacing tabs.
use super::*;

impl Shell {

    /// Closes tab `i`; the last one is replaced by a fresh shell.
    pub(super) fn close_terminal_tab(&mut self, i: usize) {
        self.term.close(i);
        if self.term.is_empty() {
            self.new_terminal_tab(None);
        }
    }

}
