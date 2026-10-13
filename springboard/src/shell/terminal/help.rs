//! Terminal help: the keys, gestures and menu items of the Terminal on one scrolling page.
use super::{Panel, Shell, TermKey};

/// Keys with a Cmd chord: the label shown, the evdev key code and Shift state `TermKey::from_chord` takes, and the key
/// it must give. A test keeps the page and the key table in step.
const KEYS: &[(&str, &str, u16, bool, TermKey)] = &[
    ("\u{2318}T", "New tab", 20, false, TermKey::NewTab),
    ("\u{2318}W", "Close tab", 17, false, TermKey::CloseTab),
    ("Shift \u{2318}]", "Next tab", 27, true, TermKey::NextTab),
    ("Shift \u{2318}[", "Previous tab", 26, true, TermKey::PrevTab),
    ("\u{2318}C", "Copy the selection", 46, false, TermKey::Copy),
    ("\u{2318}A", "Select all visible text", 30, false, TermKey::SelectAll),
    ("\u{2318}F", "Find in the scrollback", 33, false, TermKey::Find),
    ("\u{2318}G", "Find next", 34, false, TermKey::FindNext),
    ("Shift \u{2318}G", "Find previous", 34, true, TermKey::FindPrev),
    ("\u{2318}E", "Use the selection for Find", 18, false, TermKey::UseSelectionForFind),
    ("\u{2318}K", "Clear the scrollback", 37, false, TermKey::ClearScrollback),
    ("\u{2318}U", "Mark the current line", 22, false, TermKey::Mark),
    ("\u{2318}\u{2191}", "Jump to the previous mark", 103, false, TermKey::PreviousMark),
    ("\u{2318}\u{2193}", "Jump to the next mark", 108, false, TermKey::NextMark),
    ("\u{2318}0", "Actual size", 11, false, TermKey::ActualSize),
    ("\u{2318}=", "Bigger text", 13, false, TermKey::Bigger),
    ("\u{2318}-", "Smaller text", 12, false, TermKey::Smaller),
];

/// The help page as `(is_heading, text)` lines.
pub fn lines() -> Vec<(bool, String)> {
    let mut out = vec![(true, "Keys".to_string())];
    out.extend(KEYS.iter().map(|(label, what, ..)| (false, format!("{label}   {what}"))));
    out.push((false, "\u{2318}1 to \u{2318}9   Go to that tab".into()));
    let rest: &[(bool, &str)] = &[
        (true, "Touch"),
        (false, "Drag to scroll back through the output."),
        (false, "Touch and hold, then drag, to select text."),
        (false, "Double tap selects a word, triple tap a line."),
        (false, "Tap a web link to open it in the browser."),
        (true, "Tabs"),
        (false, "The + button opens a tab; the cross closes it."),
        (false, "More has Edit tab title, Close other tabs,"),
        (false, "Move tab left and Move tab right."),
        (false, "A tab running a program asks before it closes."),
        (true, "Marks"),
        (false, "Prompts the shell reports get a green tick."),
        (false, "Mark line adds a tick of your own."),
        (false, "Marks are kept until the scrollback is full."),
        (true, "Links and commands"),
        (false, "ssh, telnet and x-man-page links open a tab."),
        (false, "Open man page shows the page of the selection."),
        (false, "Paste escaped text puts a backslash before"),
        (false, "spaces and shell symbols."),
        (false, "Login shell sets the shell new tabs run."),
    ];
    out.extend(rest.iter().map(|&(heading, text)| (heading, text.to_string())));
    out
}

impl Shell {
    /// Opens the help page at its top.
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
    fn every_listed_chord_gives_its_key() {
        for (label, what, code, shift, key) in KEYS {
            assert_eq!(TermKey::from_chord(*code, *shift), Some(*key), "{label} {what}");
        }
    }

    #[test]
    fn the_page_has_headings_and_short_lines() {
        let lines = lines();
        assert!(lines[0].0, "it starts with a heading");
        assert!(lines.iter().filter(|(heading, _)| *heading).count() >= 4);
        for (_, text) in &lines {
            assert!(!text.is_empty() && text.chars().count() <= 52, "{text}");
        }
        assert!(lines.iter().any(|(_, text)| text.contains("New tab")));
        assert!(lines.iter().any(|(_, text)| text.contains("next mark")));
    }
}
