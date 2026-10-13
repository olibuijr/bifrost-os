//! Spell checking for the page: the SDK's checker over the on-screen keyboard's English word list, the misspelled
//! words of a document as ranges of cells (drawn with a wavy rule), and the words the user taught the checker, kept in
//! the app's preferences.

use crate::text::Cell;
use bifrost_app::document::DocumentHost;
use bifrost_app::spellcheck::{words, CheckingTypes, KeyValues, SpellChecker, WordList};
use std::ops::Range;

/// The English word list the checker starts from: a word and its count per line.
const EN_WORDS: &str = include_str!("../../keyboard/src/words/en.txt");

/// The preferences key of the learned words (one line, words separated by spaces).
const LEARNED_PREF: &str = "spelling.learned";

/// The app's preferences as the checker's key-value store. The preference is one line, so words are joined with spaces.
struct Prefs<'a>(&'a dyn DocumentHost);

impl KeyValues for Prefs<'_> {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.pref(key).map(|v| v.replace(' ', "\n"))
    }
    fn put_string(&mut self, key: &str, value: &str) {
        self.0.set_pref(key, &value.replace('\n', " "));
    }
}

pub struct Spelling {
    checker: SpellChecker,
    /// Whether misspelled words are marked while typing.
    pub enabled: bool,
    loaded: bool,
}

impl Spelling {
    pub fn new() -> Self {
        let list = EN_WORDS.lines().filter_map(|l| l.split_whitespace().next());
        Spelling { checker: SpellChecker::new(Box::new(WordList::new("en", list))), enabled: true, loaded: false }
    }

    /// Reads the learned words once, when the app first has a host.
    pub fn load(&mut self, host: &dyn DocumentHost) {
        if !self.loaded {
            self.loaded = true;
            if let Some(v) = host.pref(LEARNED_PREF) {
                self.checker.load(&Memory(v));
            }
        }
    }

    /// The misspelled words of `cells`, as ranges of cell indices.
    pub fn misspelled(&self, cells: &[Cell]) -> Vec<Range<usize>> {
        let text: String = cells.iter().map(|c| c.ch).collect();
        let starts = byte_starts(&text);
        self.checker
            .check(&text, CheckingTypes::SPELLING)
            .into_iter()
            .map(|r| char_index(&starts, r.range.start)..char_index(&starts, r.range.end))
            .collect()
    }

    /// The best correction for the word in `range`, when the dictionary has one.
    pub fn correction(&self, cells: &[Cell], range: Range<usize>) -> Option<String> {
        let text: String = cells.iter().map(|c| c.ch).collect();
        let starts = byte_starts(&text);
        let bytes = starts[range.start.min(starts.len() - 1)]..starts[range.end.min(starts.len() - 1)];
        self.checker.correction(&text, bytes)
    }

    /// Correct Spelling Automatically: when the word that ends right before the cell `boundary` (a space or punctuation
    /// just typed) is misspelled and the dictionary has a correction, the word's range and the correction.
    pub fn autocorrect(&self, cells: &[Cell], boundary: usize) -> Option<(Range<usize>, String)> {
        let range = word_at(cells, boundary)?;
        if range.end != boundary {
            return None;
        }
        let word = &cells[range.clone()];
        let len = word.len();
        if self.misspelled(word) != vec![0..len] {
            return None;
        }
        let guess = self.correction(word, 0..len)?;
        Some((range, guess))
    }

    /// Adds the word to the user's dictionary and keeps it.
    pub fn learn(&mut self, host: &dyn DocumentHost, word: &str) {
        self.checker.learn_word(word, &mut Prefs(host));
    }

    /// Stops flagging the word until the app quits.
    pub fn ignore(&mut self, word: &str) {
        self.checker.ignore_word(word);
    }
}

/// A read-only store holding the preference's value, to load the learned words.
struct Memory(String);

impl KeyValues for Memory {
    fn get_string(&self, _key: &str) -> Option<String> {
        Some(self.0.replace(' ', "\n"))
    }
    fn put_string(&mut self, _key: &str, _value: &str) {}
}

/// The byte offset of every char of `text`, then the text's length.
fn byte_starts(text: &str) -> Vec<usize> {
    text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect()
}

fn char_index(starts: &[usize], byte: usize) -> usize {
    starts.partition_point(|&s| s < byte)
}

/// The word at (or just before) the cell index `pos`: the range of its cells.
pub fn word_at(cells: &[Cell], pos: usize) -> Option<Range<usize>> {
    let text: String = cells.iter().map(|c| c.ch).collect();
    let starts = byte_starts(&text);
    words(&text)
        .into_iter()
        .map(|r| char_index(&starts, r.start)..char_index(&starts, r.end))
        .find(|r| pos >= r.start && pos <= r.end)
}

/// The first misspelled range that starts at or after `from`, wrapping around to the start.
pub fn next_misspelled(ranges: &[Range<usize>], from: usize) -> Option<Range<usize>> {
    ranges.iter().find(|r| r.start >= from).or_else(|| ranges.first()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Style;

    fn cells(s: &str) -> Vec<Cell> {
        s.chars().map(|ch| Cell { ch, style: Style::default() }).collect()
    }

    fn small() -> Spelling {
        Spelling { checker: SpellChecker::new(Box::new(WordList::new("en", ["hello", "world", "the", "word"]))), enabled: true, loaded: true }
    }

    #[test]
    fn misspelled_words_are_ranges_of_cells() {
        let c = cells("hello wrold, éa the");
        let ranges = small().misspelled(&c);
        assert_eq!(ranges[0], 6..11);
        assert_eq!(ranges[1], 13..15, "a word after a multi-byte character keeps its cell index");
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn a_correction_comes_from_the_dictionary() {
        let c = cells("hello wrld");
        assert_eq!(small().correction(&c, 6..10), Some("world".to_string()));
    }

    #[test]
    fn a_misspelled_word_before_a_boundary_is_corrected_automatically() {
        let c = cells("hello wrld ");
        assert_eq!(small().autocorrect(&c, 10), Some((6..10, "world".to_string())));
        let right = cells("hello world ");
        assert_eq!(small().autocorrect(&right, 11), None, "a correct word is left alone");
        assert_eq!(small().autocorrect(&c, 5), None, "hello is spelled right");
    }

    #[test]
    fn word_at_finds_the_word_around_the_caret() {
        let c = cells("hello  world");
        assert_eq!(word_at(&c, 2), Some(0..5));
        assert_eq!(word_at(&c, 5), Some(0..5), "the end of a word still belongs to it");
        assert_eq!(word_at(&c, 6), None, "between words there is none");
    }

    #[test]
    fn ignored_words_stop_being_flagged_and_the_next_one_wraps() {
        let mut s = small();
        let c = cells("zzz hello qqq");
        assert_eq!(s.misspelled(&c).len(), 2);
        s.ignore("zzz");
        let left = s.misspelled(&c);
        assert_eq!(left, vec![10..13]);
        assert_eq!(next_misspelled(&left, 11), Some(10..13), "wraps to the first");
        assert_eq!(next_misspelled(&[], 0), None);
    }
}
