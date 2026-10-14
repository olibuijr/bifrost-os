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

    /// Closes every tab but the front one, except those running a job (a menu command never ends a
    /// job). Returns how many tabs with a running job stayed open.
    pub(super) fn close_other_terminal_tabs(&mut self) -> usize {
        let keep = self.term.active_index();
        let mut busy = 0;
        // from the last tab back, so the indexes still to visit do not move; closing a tab below the
        // front one moves the front one down, but the front tab is passed before any such close
        for i in (0..self.term.len()).rev() {
            if i == keep {
                continue;
            }
            if self.term.get(i).is_some_and(|tab| tab.job().is_some()) {
                busy += 1;
            } else {
                self.term.close(i);
            }
        }
        busy
    }

}
