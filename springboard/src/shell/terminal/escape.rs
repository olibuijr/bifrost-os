//! Text for the shell: Paste Escaped Text quotes the characters the shell reads specially, and a
//! selection opened with man is checked first.

/// The text with every character the shell reads specially escaped by a backslash, so that it
/// arrives as one word. Tabs and line breaks become spaces (a line break would run the command);
/// other control characters are dropped.
pub fn shell_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let c = match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if !(c.is_alphanumeric() || "@%+=:,./_-".contains(c)) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The manual page a selection names: its first word, when it is a plausible command name (letters,
/// digits and `. _ + -`, at most 64 characters, no leading dash). None for anything else.
pub fn man_topic(selection: &str) -> Option<&str> {
    let word = selection.split_whitespace().next()?;
    let plausible = word.len() <= 64
        && !word.starts_with('-')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    plausible.then_some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_metacharacters_are_escaped() {
        assert_eq!(shell_escape("a b/c-d_e.f"), r"a\ b/c-d_e.f");
        assert_eq!(shell_escape("it's $HOME"), r"it\'s\ \$HOME");
        assert_eq!(shell_escape("x;rm -rf ~"), r"x\;rm\ -rf\ \~");
        assert_eq!(shell_escape("a\\b"), r"a\\b");
    }

    #[test]
    fn line_breaks_become_escaped_spaces_and_other_controls_vanish() {
        assert_eq!(shell_escape("a\nb\r\nc\td"), r"a\ b\ \ c\ d", "a CRLF is two spaces");
        assert_eq!(shell_escape("x\u{7}\u{1b}y"), "xy");
        assert_eq!(shell_escape(""), "");
    }

    #[test]
    fn letters_beyond_ascii_pass_through() {
        assert_eq!(shell_escape("Ólafur"), "Ólafur");
    }

    #[test]
    fn man_topic_is_the_first_plausible_word() {
        assert_eq!(man_topic("ls -la"), Some("ls"));
        assert_eq!(man_topic("  printf\n"), Some("printf"));
        assert_eq!(man_topic("g++"), Some("g++"), "plus signs are allowed in topic names");
        assert_eq!(man_topic("openssl.cnf"), Some("openssl.cnf"));
    }

    #[test]
    fn man_topic_refuses_options_shell_syntax_and_empty_text() {
        assert_eq!(man_topic("--help"), None);
        assert_eq!(man_topic("a;rm"), None);
        assert_eq!(man_topic("$(id)"), None);
        assert_eq!(man_topic("   "), None);
        assert_eq!(man_topic(&"a".repeat(65)), None);
        assert_eq!(man_topic(&"a".repeat(64)).map(str::len), Some(64));
    }
}
