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

/// The Terminal help line shown in the More menu.
pub const HELP: &str = "Cmd+T new tab, Cmd+W close, Cmd+F find, Cmd+K clear, Cmd+/- text size";

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
}
