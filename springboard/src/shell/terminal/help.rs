//! Terminal help: a reference of the keyboard shortcuts the Terminal answers while it is in front,
//! opened from the More menu. Each row is what the key does and its key equivalent.
use super::*;

/// The Terminal's own shortcuts, in the order the Shell menu lists them (see `hwkeys::TermKey::from_chord`).
pub(super) const SHORTCUTS: &[(&str, &str)] = &[
    ("Copy", "\u{2318}C"),
    ("Paste", "\u{2318}V"),
    ("Select All", "\u{2318}A"),
    ("Find in scrollback", "\u{2318}F"),
    ("Find Next", "\u{2318}G"),
    ("Find Previous", "\u{21e7}\u{2318}G"),
    ("Clear Scrollback", "\u{2318}K"),
    ("Larger text", "\u{2318}="),
    ("Smaller text", "\u{2318}-"),
    ("Actual size", "\u{2318}0"),
    ("New tab", "\u{2318}T"),
    ("Close tab", "\u{2318}W"),
    ("Next tab", "\u{21e7}\u{2318}]"),
    ("Previous tab", "\u{21e7}\u{2318}["),
    ("Go to tab 1 to 9", "\u{2318}1 to \u{2318}9"),
];

impl Shell {
    /// Opens the Terminal help panel: the shortcut list, read only, closed with Done.
    pub(super) fn terminal_open_help(&mut self, t: f64) {
        self.terminal_ui.panel = Panel::Help;
        self.terminal_ui.scroll = 0.0;
        self.terminal_ui.message.clear();
        self.set_keyboard(false, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_row_names_an_action_and_a_key() {
        assert!(SHORTCUTS.len() >= 10);
        for (action, keys) in SHORTCUTS {
            assert!(!action.is_empty() && !keys.is_empty(), "{action}: {keys}");
            assert!(keys.contains('\u{2318}'), "every Terminal shortcut is a Cmd chord: {keys}");
        }
    }
}
