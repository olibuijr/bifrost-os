//! Text replacements (Keyboard > Text Replacements): a list of shortcuts and the phrases they expand to. A shortcut is
//! one word, matched exactly. The list is kept sorted by shortcut (without case) and is saved as one line of text,
//! so it fits in a settings value: records are separated by U+001E and each shortcut is followed by U+001F and its
//! phrase. Both separators are control characters, which a shortcut or phrase may not contain.

/// One replacement: typing `shortcut` inserts `phrase`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub shortcut: String,
    pub phrase: String,
}

/// Why a replacement was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    EmptyShortcut,
    /// A shortcut is one word.
    ShortcutHasSpace,
    EmptyPhrase,
    /// A control character (a line break or a separator of the saved form).
    ControlCharacter,
}

const RECORD: char = '\u{1e}';
const FIELD: char = '\u{1f}';

/// The replacement list. Shortcuts are unique.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replacements {
    entries: Vec<Entry>,
}

impl Replacements {
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries, sorted by shortcut without case.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds the replacement, or changes the phrase of an existing shortcut. Both ends are trimmed first.
    pub fn set(&mut self, shortcut: &str, phrase: &str) -> Result<(), Refused> {
        let (s, p) = (shortcut.trim(), phrase.trim());
        if s.is_empty() {
            return Err(Refused::EmptyShortcut);
        }
        if p.is_empty() {
            return Err(Refused::EmptyPhrase);
        }
        if s.chars().any(char::is_whitespace) {
            return Err(Refused::ShortcutHasSpace);
        }
        if s.chars().chain(p.chars()).any(char::is_control) {
            return Err(Refused::ControlCharacter);
        }
        match self.entries.iter().position(|e| e.shortcut == s) {
            Some(i) => self.entries[i].phrase = p.to_string(),
            None => self.entries.push(Entry { shortcut: s.to_string(), phrase: p.to_string() }),
        }
        self.entries.sort_by_key(|e| e.shortcut.to_lowercase());
        Ok(())
    }

    /// Removes the replacement for `shortcut`. False when there was none.
    pub fn remove(&mut self, shortcut: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.shortcut != shortcut);
        self.entries.len() != before
    }

    /// The phrase that the whole word `word` expands to, matched exactly.
    pub fn expand(&self, word: &str) -> Option<&str> {
        self.entries.iter().find(|e| e.shortcut == word).map(|e| e.phrase.as_str())
    }

    /// The list as one line of text (see the module notes).
    pub fn to_line(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("{}{}{}", e.shortcut, FIELD, e.phrase))
            .collect::<Vec<_>>()
            .join("\u{1e}")
    }

    /// The list read back from [`Replacements::to_line`]. A record without a separator is skipped, and a shortcut that
    /// appears twice keeps its last phrase.
    pub fn from_line(text: &str) -> Self {
        let mut out = Self::new();
        for record in text.split(RECORD) {
            if let Some((shortcut, phrase)) = record.split_once(FIELD) {
                let _ = out.set(shortcut, phrase);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(r: &Replacements) -> Vec<(&str, &str)> {
        r.entries().iter().map(|e| (e.shortcut.as_str(), e.phrase.as_str())).collect()
    }

    #[test]
    fn set_adds_trims_and_keeps_the_list_sorted_without_case() {
        let mut r = Replacements::new();
        r.set("omw", "On my way").unwrap();
        r.set("  brb ", " be right back ").unwrap();
        r.set("Zed", "zed").unwrap();
        r.set("abc", "a b c").unwrap();
        assert_eq!(pairs(&r), [("abc", "a b c"), ("brb", "be right back"), ("omw", "On my way"), ("Zed", "zed")]);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn setting_an_existing_shortcut_changes_its_phrase() {
        let mut r = Replacements::new();
        r.set("omw", "On my way").unwrap();
        r.set("omw", "On my way!").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.expand("omw"), Some("On my way!"));
    }

    #[test]
    fn refusals_name_the_broken_part() {
        let mut r = Replacements::new();
        assert_eq!(r.set("", "x"), Err(Refused::EmptyShortcut));
        assert_eq!(r.set("   ", "x"), Err(Refused::EmptyShortcut));
        assert_eq!(r.set("a", "  "), Err(Refused::EmptyPhrase));
        assert_eq!(r.set("a b", "x"), Err(Refused::ShortcutHasSpace));
        assert_eq!(r.set("ab", "x\ny"), Err(Refused::ControlCharacter));
        assert_eq!(r.set("a\u{1f}b", "x"), Err(Refused::ControlCharacter));
        assert!(r.is_empty(), "a refused entry changes nothing");
    }

    #[test]
    fn expand_matches_the_whole_shortcut_exactly() {
        let mut r = Replacements::new();
        r.set("omw", "On my way").unwrap();
        assert_eq!(r.expand("omw"), Some("On my way"));
        assert_eq!(r.expand("OMW"), None);
        assert_eq!(r.expand("omwx"), None);
    }

    #[test]
    fn remove_reports_whether_something_was_removed() {
        let mut r = Replacements::new();
        r.set("brb", "be right back").unwrap();
        r.set("omw", "On my way").unwrap();
        assert!(r.remove("brb"));
        assert!(!r.remove("brb"));
        assert_eq!(pairs(&r), [("omw", "On my way")]);
    }

    #[test]
    fn the_saved_line_round_trips() {
        let mut r = Replacements::new();
        r.set("brb", "be right back").unwrap();
        r.set("omw", "On my way = soon").unwrap();
        let line = r.to_line();
        assert_eq!(line, "brb\u{1f}be right back\u{1e}omw\u{1f}On my way = soon");
        assert_eq!(Replacements::from_line(&line), r);
        assert!(Replacements::from_line("").is_empty());
    }

    #[test]
    fn a_malformed_record_is_skipped_and_a_repeat_keeps_the_last() {
        let text = "junk\u{1e}ok\u{1f}first\u{1e}ok\u{1f}second";
        let r = Replacements::from_line(text);
        assert_eq!(pairs(&r), [("ok", "second")]);
    }
}
