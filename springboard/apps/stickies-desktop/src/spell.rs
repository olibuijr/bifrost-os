//! Spelling for the notes: the SDK checker over the English word list the keyboard uses, the words the user taught it
//! (kept in the app's defaults), and the marks drawn under misspelled words and detected items. Every range is a byte
//! range of the note's text.

use bifrost_app::controls::detect;
use bifrost_app::defaults::UserDefaults;
use bifrost_app::spellcheck::{words, CheckingTypes, SpellChecker, WordList};
use std::ops::Range;

/// The English word list the checker starts from: a word and its count per line.
const EN_WORDS: &str = include_str!("../../keyboard/src/words/en.txt");

/// The checker, with the words the user taught it and the words ignored for this run.
pub struct Spelling {
    checker: SpellChecker,
}

impl Default for Spelling {
    fn default() -> Self {
        Spelling::new()
    }
}

impl Spelling {
    pub fn new() -> Spelling {
        let list = EN_WORDS.lines().filter_map(|l| l.split_whitespace().next());
        Spelling { checker: SpellChecker::new(Box::new(WordList::new("en", list))) }
    }

    /// Reads the words the user taught the checker from the app's defaults.
    pub fn load(&mut self, store: &UserDefaults) {
        self.checker.load(store);
    }

    /// The misspelled words of `text`, as byte ranges.
    pub fn misspelled(&self, text: &str) -> Vec<Range<usize>> {
        self.checker.check(text, CheckingTypes::SPELLING).into_iter().map(|r| r.range).collect()
    }

    /// The checker's best correction for the word at `range` of `text`, when it has one.
    pub fn correction(&self, text: &str, range: Range<usize>) -> Option<String> {
        self.checker.correction(text, range)
    }

    /// Adds `word` to the user's dictionary and keeps it in the app's defaults.
    pub fn learn(&mut self, store: &mut UserDefaults, word: &str) {
        self.checker.learn_word(word, store);
    }

    /// Stops flagging `word` until the app quits.
    pub fn ignore(&mut self, word: &str) {
        self.checker.ignore_word(word);
    }

    /// Correct Spelling Automatically: when a word ends at byte `boundary` (a space or punctuation was just typed after
    /// it), is misspelled and has a correction, the word's range and the correction.
    pub fn autocorrect(&self, text: &str, boundary: usize) -> Option<(Range<usize>, String)> {
        let range = word_ending_at(text, boundary)?;
        let word = &text[range.clone()];
        if self.checker.check(word, CheckingTypes::SPELLING).is_empty() {
            return None;
        }
        let guess = self.correction(word, 0..word.len())?;
        Some((range, guess))
    }
}

/// The word of `text` that ends at byte `end`.
pub fn word_ending_at(text: &str, end: usize) -> Option<Range<usize>> {
    words(text).into_iter().find(|r| r.end == end)
}

/// The word of `text` that contains byte `at`, or that ends there (the word before a caret).
pub fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    words(text).into_iter().find(|r| at >= r.start && at <= r.end)
}

/// The first misspelled range that starts at or after `from`, wrapping around to the first one.
pub fn next_misspelled(ranges: &[Range<usize>], from: usize) -> Option<Range<usize>> {
    ranges.iter().find(|r| r.start >= from).or_else(|| ranges.first()).cloned()
}

/// What a mark under the text says: a misspelled word, or an item the data detectors found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Misspelled,
    Detected,
}

/// A range of the text that the board marks with a rule under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub range: Range<usize>,
    pub kind: MarkKind,
}

/// The marks of `text`, in order: its misspelled words (when a checker is given) and the items of `kinds` the data
/// detectors find. A misspelling inside a detected item is dropped, so each piece of text is marked once.
pub fn marks(text: &str, spelling: Option<&Spelling>, kinds: &[detect::Kind]) -> Vec<Mark> {
    let detected: Vec<Range<usize>> = detect::find(text, kinds).into_iter().map(|m| m.range).collect();
    let mut out: Vec<Mark> = Vec::new();
    if let Some(sp) = spelling {
        let misspelled = sp.misspelled(text);
        out.extend(
            misspelled
                .into_iter()
                .filter(|r| !detected.iter().any(|d| r.start < d.end && d.start < r.end))
                .map(|range| Mark { range, kind: MarkKind::Misspelled }),
        );
    }
    out.extend(detected.into_iter().map(|range| Mark { range, kind: MarkKind::Detected }));
    out.sort_by_key(|m| m.range.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_word_before_a_boundary_and_the_word_at_a_caret() {
        assert_eq!(word_ending_at("hello wrold ", 11), Some(6..11));
        assert_eq!(word_ending_at("hello wrold", 8), None, "the boundary is not the word's end");
        assert_eq!(word_at("hello wrold", 7), Some(6..11));
        assert_eq!(word_at("hello wrold", 11), Some(6..11), "a caret after the word");
        assert_eq!(word_at("  ", 1), None);
    }

    #[test]
    fn the_next_misspelling_wraps_round_the_note() {
        let found = [2..4, 9..12];
        assert_eq!(next_misspelled(&found, 0), Some(2..4));
        assert_eq!(next_misspelled(&found, 3), Some(9..12));
        assert_eq!(next_misspelled(&found, 10), Some(2..4), "wraps to the first");
        assert_eq!(next_misspelled(&[], 0), None);
    }

    #[test]
    fn a_nonsense_word_is_marked_and_a_detected_date_is_marked_as_detected() {
        let sp = Spelling::new();
        let found = marks("hello wrold on 2026-10-09", Some(&sp), &[detect::Kind::Date]);
        assert!(found.iter().any(|m| m.kind == MarkKind::Misspelled && m.range == (6..11)));
        assert!(found.iter().any(|m| m.kind == MarkKind::Detected && m.range.start == 15), "the date after \"on \"");
        assert!(found.windows(2).all(|w| w[0].range.start <= w[1].range.start), "in order");
    }

    #[test]
    fn without_a_checker_only_the_detected_items_are_marked() {
        let found = marks("wrold 2026-10-09", None, &[detect::Kind::Date]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, MarkKind::Detected);
        assert!(marks("wrold", None, &[]).is_empty());
    }
}
