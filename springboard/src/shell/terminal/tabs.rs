//! Tab model: closing, replacing and reordering tabs.
use super::*;
use crate::term::tab_order;

impl Shell {

    /// Closes tab `i`; the last one is replaced by a fresh shell.
    pub(super) fn close_terminal_tab(&mut self, i: usize) {
        self.term.close(i);
        if self.term.is_empty() {
            self.new_terminal_tab(None);
        }
    }

    /// More > Move Tab Left and Move Tab Right: the front tab trades places with its neighbour on the bar.
    pub(super) fn terminal_move_tab(&mut self, forward: bool, t: f64) {
        let at = self.term.active_index();
        match tab_order::neighbour(self.term.len(), at, forward) {
            Some(to) => {
                self.term.move_session(at, to);
                // a pending close names a tab index, which the move has changed
                self.terminal_ui.pending_close = None;
                self.terminal_close_panel(t);
            }
            None => {
                self.terminal_ui.message = if forward { "The tab is already last".into() } else { "The tab is already first".into() };
            }
        }
    }

}
