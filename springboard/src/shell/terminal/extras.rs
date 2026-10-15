//! Terminal menu extras: Paste Escaped Text and the help line.

/// Text for Paste Escaped Text: every character the shell would treat specially gets a backslash,
/// so the paste lands as one literal word (a line break becomes a quoted line break).
pub fn escape_paste(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' | '\r' => out.push_str("'\n'"),
            c if c.is_alphanumeric() || "_-./,:@%+=~".contains(c) => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

/// The Terminal shortcuts: each key and what it does. The help line is built from this table.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Cmd+T", "new tab"),
    ("Cmd+W", "close tab"),
    ("Cmd+F", "find"),
    ("Cmd+E", "use selection for find"),
    ("Cmd+G", "find next"),
    ("Cmd+Shift+G", "find previous"),
    ("Cmd+K", "clear scrollback"),
    ("Cmd+/-", "text size"),
    ("Cmd+U", "mark line"),
    ("Cmd+Up / Cmd+Down", "previous / next mark"),
];

/// The Terminal help line shown in the More menu: every shortcut of `SHORTCUTS`, in table order.
pub fn help_line() -> String {
    SHORTCUTS
        .iter()
        .map(|(keys, action)| format!("{keys} {action}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_paste_backslashes_shell_specials() {
        assert_eq!(escape_paste("a b"), "a\\ b");
        assert_eq!(escape_paste("$(rm *);'x'"), "\\$\\(rm\\ \\*\\)\\;\\'x\\'");
        assert_eq!(escape_paste("/usr/bin/ls-1.txt"), "/usr/bin/ls-1.txt");
        assert_eq!(escape_paste(""), "");
    }

    #[test]
    fn escape_paste_quotes_line_breaks() {
        assert_eq!(escape_paste("a\nb"), "a'\n'b");
    }

    #[test]
    fn help_line_lists_every_shortcut_in_table_order() {
        let line = help_line();
        assert!(
            line.starts_with("Cmd+T new tab, Cmd+W close tab, Cmd+F find, "),
            "{line}"
        );
        assert!(
            line.ends_with("Cmd+Up / Cmd+Down previous / next mark"),
            "{line}"
        );
        assert_eq!(line.matches(", ").count(), SHORTCUTS.len() - 1);
    }

    #[test]
    fn every_shortcut_key_appears_once_in_the_table() {
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|(keys, _)| *keys).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), SHORTCUTS.len(), "no two actions share a key");
    }
}
