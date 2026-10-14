//! Things done with the selected text or the clipboard: paste with the shell's special characters escaped, paste the
//! selection, search the web for it and save it to Files.
use super::*;

/// Characters a POSIX shell treats as syntax; each gets a backslash in Paste Escaped Text.
const SPECIAL: &str = " !\"#$&'()*;<>?[\\]^`{|}~";

/// `text` as shell words: every special character is backslash-escaped, and each line break or tab becomes an escaped
/// space so a pasted file name or snippet cannot run anything. Other control characters are dropped.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.replace("\r\n", "\n").chars() {
        match c {
            '\n' | '\r' | '\t' => out.push_str("\\ "),
            c if c.is_control() => {}
            c if SPECIAL.contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// The web search query for a selection: blanks collapsed, at most 200 characters. None when nothing is selected.
pub fn search_query(selection: &str) -> Option<String> {
    let query: String = selection.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    let query = query.trim().to_string();
    (!query.is_empty()).then_some(query)
}

/// A file name `stem` + `ext` that `taken` does not report: `stem.ext`, then `stem 2.ext`, `stem 3.ext`, ...
pub fn free_name(stem: &str, ext: &str, taken: impl Fn(&str) -> bool) -> String {
    let first = format!("{stem}.{ext}");
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{stem} {n}.{ext}"))
        .find(|name| !taken(name))
        .expect("an unbounded range always yields a free name")
}

impl Shell {
    /// Pastes the clipboard with the shell's special characters escaped (Paste Escaped Text).
    pub(super) fn terminal_paste_escaped(&mut self, t: f64) {
        match clipboard::text().filter(|text| !text.is_empty()) {
            Some(text) => {
                if let Some(term) = self.term.as_mut() {
                    term.paste(&escape_text(&text));
                }
                self.terminal_close_panel(t);
            }
            None => self.terminal_ui.message = "The clipboard has no text".into(),
        }
    }

    /// Types the selected text into the shell (Paste Selection).
    pub(super) fn terminal_paste_selection(&mut self, t: f64) {
        match self.term.as_ref().and_then(|term| term.selected_text()).filter(|text| !text.is_empty()) {
            Some(text) => {
                if let Some(term) = self.term.as_mut() {
                    term.paste(&text);
                }
                self.terminal_close_panel(t);
            }
            None => self.terminal_ui.message = "Select terminal text first".into(),
        }
    }

    /// Opens a web search for the selected text in the browser.
    pub(super) fn terminal_search_web(&mut self, t: f64) {
        let query = self.term.as_ref().and_then(|term| term.selected_text()).and_then(|text| search_query(&text));
        self.terminal_ui.message = match query {
            Some(query) => match self.desktop_open_url(&super::super::browser::nav::search_url(&query), t) {
                Ok(()) => "Searching the web".into(),
                Err(e) => format!("Could not search: {e}"),
            },
            None => "Select terminal text first".into(),
        };
    }

    /// Writes `text` to a new file in Files, named from `stem` so an earlier export is never overwritten; the result
    /// is the message to show.
    pub(super) fn terminal_save_text(&self, stem: &str, text: &str) -> String {
        let dir = self.data_dir.join("Files");
        let name = free_name(stem, "txt", |name| dir.join(name).exists());
        match fs::create_dir_all(&dir).and_then(|_| fs::write(dir.join(&name), text)) {
            Ok(()) => format!("Saved to Files/{name}"),
            Err(e) => format!("Could not save: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_characters_get_a_backslash() {
        assert_eq!(escape_text("My File (1).txt"), "My\\ File\\ \\(1\\).txt");
        assert_eq!(escape_text("a;b|c&d$e`f"), "a\\;b\\|c\\&d\\$e\\`f");
        assert_eq!(escape_text("it's \"x\" \\ #1"), "it\\'s\\ \\\"x\\\"\\ \\\\\\ \\#1");
        assert_eq!(escape_text("plain-text_1.2/3"), "plain-text_1.2/3", "path characters stay");
        assert_eq!(escape_text(""), "");
    }

    #[test]
    fn line_breaks_and_control_characters_cannot_run_a_command() {
        assert_eq!(escape_text("rm a\nrm b"), "rm\\ a\\ rm\\ b");
        assert_eq!(escape_text("a\r\nb"), "a\\ b", "a CRLF is one break");
        assert_eq!(escape_text("a\x1b[201~b\x07"), "a\\[201\\~b", "the escape and bell are dropped, the rest is escaped");
        assert_eq!(escape_text("a\tb"), "a\\ b");
        assert!(!escape_text("x\ny\rz").contains(['\n', '\r']));
    }

    #[test]
    fn a_search_query_is_one_short_line() {
        assert_eq!(search_query("  rust\n  lifetimes  ").as_deref(), Some("rust lifetimes"));
        assert_eq!(search_query("\n \t"), None);
        assert_eq!(search_query(""), None);
        assert_eq!(search_query(&"abcd ".repeat(100)).map(|q| q.chars().count()), Some(199), "cut at 200, trailing blank trimmed");
    }

    #[test]
    fn exports_never_take_a_name_that_exists() {
        assert_eq!(free_name("terminal-scrollback", "txt", |_| false), "terminal-scrollback.txt");
        let taken = ["terminal-scrollback.txt", "terminal-scrollback 2.txt"];
        assert_eq!(free_name("terminal-scrollback", "txt", |n| taken.contains(&n)), "terminal-scrollback 3.txt");
    }
}
