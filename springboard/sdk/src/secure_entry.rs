//! Secure text entry (the SecurityUI framework): a password field an app can use without its text reaching the screen
//! or a log. The entry keeps the characters itself, shows only a mask with one dot per character, refuses control
//! characters and input past its maximum length, and wipes its buffer when it is cleared or dropped. The text leaves
//! the entry only through [`SecureEntry::take`], which empties it, or through the [`SecureEntry::matches`] check.
//!
//! The entry has no view of its own: the app draws [`SecureEntry::mask`] in its field.

/// The character drawn for each typed character.
pub const MASK: char = '\u{2022}';

/// The longest entry when the app does not choose one.
pub const DEFAULT_MAX_LEN: usize = 256;

/// The text of a secure field, kept apart from the screen.
pub struct SecureEntry {
    chars: Vec<char>,
    max_len: usize,
}

impl SecureEntry {
    /// An empty entry that accepts up to `max_len` characters (at least one).
    pub fn new(max_len: usize) -> Self {
        SecureEntry { chars: Vec::new(), max_len: max_len.max(1) }
    }

    /// Types one character at the end. Returns `false` when the character is refused: a control character (a newline,
    /// a tab) or the entry is full.
    pub fn type_char(&mut self, c: char) -> bool {
        if c.is_control() || self.chars.len() >= self.max_len {
            return false;
        }
        self.chars.push(c);
        true
    }

    /// Types each character of `text` in turn and returns how many were accepted.
    pub fn type_str(&mut self, text: &str) -> usize {
        text.chars().filter(|&c| self.type_char(c)).count()
    }

    /// Deletes the last character. Returns `false` when the entry was empty.
    pub fn delete_back(&mut self) -> bool {
        self.chars.pop().is_some()
    }

    /// The number of characters typed.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether nothing is typed.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The text the field shows: one mask character per typed character, never the characters themselves.
    pub fn mask(&self) -> String {
        std::iter::repeat(MASK).take(self.chars.len()).collect()
    }

    /// Compares the typed text with `expected`. Every character of the longer side is looked at, so the work does not
    /// depend on where the first difference is.
    pub fn matches(&self, expected: &str) -> bool {
        let want: Vec<char> = expected.chars().collect();
        let len_ok = want.len() == self.chars.len();
        let mut diff: u32 = 0;
        for i in 0..self.chars.len().max(want.len()) {
            let a = self.chars.get(i).copied().unwrap_or('\0') as u32;
            let b = want.get(i).copied().unwrap_or('\0') as u32;
            diff |= a ^ b;
        }
        len_ok && diff == 0
    }

    /// Returns the typed text and empties the entry. The caller owns the returned string and is responsible for it.
    pub fn take(&mut self) -> String {
        let text: String = self.chars.iter().collect();
        self.clear();
        text
    }

    /// Empties the entry and overwrites the characters it held.
    pub fn clear(&mut self) {
        for c in self.chars.iter_mut() {
            *c = '\0';
        }
        self.chars.clear();
    }
}

impl Drop for SecureEntry {
    fn drop(&mut self) {
        self.clear();
    }
}

impl std::fmt::Debug for SecureEntry {
    /// Shows the length only, never the text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecureEntry({} characters)", self.chars.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_shows_only_masks() {
        let mut e = SecureEntry::new(DEFAULT_MAX_LEN);
        assert_eq!(e.type_str("hunter2"), 7);
        assert_eq!(e.mask(), MASK.to_string().repeat(7));
        assert!(!e.mask().contains('h'));
        assert_eq!(format!("{e:?}"), "SecureEntry(7 characters)");
    }

    #[test]
    fn control_characters_and_overflow_are_refused() {
        let mut e = SecureEntry::new(3);
        assert!(!e.type_char('\n'));
        assert!(!e.type_char('\t'));
        assert_eq!(e.type_str("abcdef"), 3, "only three fit");
        assert!(!e.type_char('g'));
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn delete_back_removes_the_last_character() {
        let mut e = SecureEntry::new(8);
        assert!(!e.delete_back(), "nothing to delete");
        e.type_str("ab");
        assert!(e.delete_back());
        assert_eq!(e.len(), 1);
        assert!(!e.is_empty());
    }

    #[test]
    fn matches_compares_the_whole_text() {
        let mut e = SecureEntry::new(DEFAULT_MAX_LEN);
        e.type_str("correct horse");
        assert!(e.matches("correct horse"));
        assert!(!e.matches("correct hors"), "a prefix is not a match");
        assert!(!e.matches("correct horse!"), "a longer text is not a match");
        assert!(!e.matches("Correct horse"));
    }

    #[test]
    fn take_returns_the_text_once_and_empties_the_entry() {
        let mut e = SecureEntry::new(DEFAULT_MAX_LEN);
        e.type_str("secret");
        assert_eq!(e.take(), "secret");
        assert!(e.is_empty());
        assert_eq!(e.take(), "", "the second take has nothing");
    }

    #[test]
    fn a_zero_maximum_still_takes_one_character() {
        let mut e = SecureEntry::new(0);
        assert!(e.type_char('x'));
        assert!(!e.type_char('y'));
    }
}
