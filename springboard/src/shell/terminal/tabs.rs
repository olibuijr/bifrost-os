//! Tab model: closing, replacing and reordering tabs.
use super::*;

/// The warning shown before Close other tabs ends tabs that run programs: it names the first and counts the rest.
/// None when no other tab runs one.
pub fn others_warning(jobs: &[String]) -> Option<String> {
    let first = jobs.first()?;
    Some(match jobs.len() {
        1 => format!("{first} is running in another tab. Choose Close other tabs again to close it"),
        n => format!("{first} and {} more programs are running. Choose Close other tabs again to close them", n - 1),
    })
}

impl Shell {

    /// Closes tab `i`; the last one is replaced by a fresh shell.
    pub(super) fn close_terminal_tab(&mut self, i: usize) {
        self.term.close(i);
        if self.term.is_empty() {
            self.new_terminal_tab(None);
        }
    }

    /// Closes every tab but the active one. Tabs running programs ask first: the first choice names them and a second
    /// choice closes.
    pub(super) fn terminal_close_others(&mut self, t: f64) {
        let keep = self.term.active_index();
        if self.term.len() <= 1 {
            self.terminal_ui.message = "There are no other tabs".into();
            return;
        }
        let jobs: Vec<String> = self.term.iter().enumerate().filter(|&(i, _)| i != keep).filter_map(|(_, tab)| tab.job()).collect();
        if !self.terminal_ui.confirm_others {
            if let Some(warning) = others_warning(&jobs) {
                self.terminal_ui.confirm_others = true;
                self.terminal_ui.message = warning;
                return;
            }
        }
        let closed = self.term.close_others(keep);
        self.terminal_close_panel(t);
        self.terminal_ui.message = format!("Closed {closed} tab{}", if closed == 1 { "" } else { "s" });
    }

    /// Moves the active tab one place left or right in the tab bar.
    pub(super) fn terminal_move_tab(&mut self, left: bool) {
        let at = self.term.active_index();
        let to = if left { at.checked_sub(1) } else { Some(at + 1) };
        self.terminal_ui.pending_close = None;
        self.terminal_ui.message = match to {
            Some(to) if self.term.move_tab(at, to) => format!("Moved the tab {}", if left { "left" } else { "right" }),
            _ => format!("The tab is already the {}", if left { "first" } else { "last" }),
        };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_programs_are_named_before_other_tabs_close() {
        assert_eq!(others_warning(&[]), None);
        assert_eq!(
            others_warning(&["vim".to_string()]).as_deref(),
            Some("vim is running in another tab. Choose Close other tabs again to close it")
        );
        let many = others_warning(&["vim".to_string(), "top".to_string(), "ssh".to_string()]).unwrap();
        assert!(many.starts_with("vim and 2 more programs are running"), "{many}");
    }
}
