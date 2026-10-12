//! Dictionary lookup: a store of term definitions and a lookup that finds the term at a position in a text. Mirrors the
//! system dictionary lookup of the core services layer: a definition for a term, and the range of the term that covers
//! a position in a string, preferring the longest matching (possibly multi-word) term.
//!
//! * Terms are matched case-insensitively over the word boundaries of `spellcheck::words`, so "e-mail" is two words
//!   "e" and "mail", and a term is stored with the same normalization.
//! * The definitions are supplied by the caller. No dictionary is bundled, so a lookup of an unknown word finds nothing.
//! * Ranges are byte ranges on char boundaries, as elsewhere in the SDK.
//!
//! Limits: no part-of-speech or pronunciation data, no inflection (a lookup of "ran" does not find "run"), and no
//! definition lookup across languages.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

use crate::spellcheck::words;

/// `DefinitionError`: a term that cannot be stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionError {
    /// The term has no words after normalization.
    EmptyTerm,
}

impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefinitionError::EmptyTerm => write!(f, "the term has no words"),
        }
    }
}

/// The lowercase words of a term joined by single spaces, or `None` when it has no words.
fn normalize(term: &str) -> Option<(String, usize)> {
    let spans = words(term);
    if spans.is_empty() {
        return None;
    }
    let key: Vec<String> = spans.iter().map(|r| term[r.start..r.end].to_lowercase()).collect();
    Some((key.join(" "), spans.len()))
}

/// `DefinitionIndex`: definitions keyed by normalized term, with the longest term length kept for lookups.
#[derive(Clone, Debug, Default)]
pub struct DefinitionIndex {
    terms: BTreeMap<String, String>,
    max_words: usize,
}

impl DefinitionIndex {
    /// An empty index.
    pub fn new() -> Self {
        DefinitionIndex::default()
    }

    /// Stores a definition for a term, replacing any earlier one for the same term.
    pub fn insert(&mut self, term: &str, definition: &str) -> Result<(), DefinitionError> {
        let (key, count) = normalize(term).ok_or(DefinitionError::EmptyTerm)?;
        if count > self.max_words {
            self.max_words = count;
        }
        self.terms.insert(key, definition.to_string());
        Ok(())
    }

    /// The number of stored terms.
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Whether no term is stored.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The definition of a term, matched case-insensitively.
    pub fn definition(&self, term: &str) -> Option<&str> {
        let (key, _) = normalize(term)?;
        self.terms.get(&key).map(|d| d.as_str())
    }

    /// The longest stored term that covers byte `offset` in `text`, as its byte range.
    pub fn term_range(&self, text: &str, offset: usize) -> Option<Range<usize>> {
        let spans = words(text);
        let mut best: Option<Range<usize>> = None;
        for i in 0..spans.len() {
            if spans[i].start > offset {
                break;
            }
            let mut key = String::new();
            for j in i..spans.len().min(i + self.max_words) {
                // A multi-word term may only span whitespace between its words.
                if j > i && !text[spans[j - 1].end..spans[j].start].chars().all(char::is_whitespace) {
                    break;
                }
                if j > i {
                    key.push(' ');
                }
                key.push_str(&text[spans[j].start..spans[j].end].to_lowercase());
                if spans[j].end > offset && self.terms.contains_key(&key) {
                    let range = spans[i].start..spans[j].end;
                    if best.as_ref().map_or(true, |b| range.len() > b.len()) {
                        best = Some(range);
                    }
                }
            }
        }
        best
    }

    /// The longest stored term at byte `offset` in `text`, with its range and its definition.
    pub fn definition_at(&self, text: &str, offset: usize) -> Option<(Range<usize>, &str)> {
        let range = self.term_range(text, offset)?;
        let (key, _) = normalize(&text[range.start..range.end])?;
        let definition = self.terms.get(&key)?;
        Some((range, definition.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_stored_and_found_case_insensitively() {
        let mut index = DefinitionIndex::new();
        index.insert("Rust", "A systems language").unwrap();
        assert_eq!(index.definition("RUST"), Some("A systems language"));
        assert_eq!(index.definition("  rust "), Some("A systems language"));
        assert_eq!(index.definition("go"), None);
        assert_eq!(index.len(), 1);
        assert_eq!(index.insert("  ... ", "nothing"), Err(DefinitionError::EmptyTerm));
    }

    #[test]
    fn a_single_word_is_found_by_its_range() {
        let mut index = DefinitionIndex::new();
        index.insert("rust", "A language").unwrap();
        // "I like rust a lot": "rust" occupies bytes 7..11.
        assert_eq!(index.term_range("I like rust a lot", 9), Some(7..11));
        // The same offset in a different word finds nothing.
        assert_eq!(index.term_range("I like python", 9), None);
    }

    #[test]
    fn the_longest_term_covering_the_offset_wins() {
        let mut index = DefinitionIndex::new();
        index.insert("rust", "A metal").unwrap();
        index.insert("rust language", "A systems language").unwrap();
        // "the rust language is": "rust" is 4..8, "language" is 9..17.
        assert_eq!(index.term_range("the rust language is", 5), Some(4..17));
        assert_eq!(index.term_range("the rust language is", 10), Some(4..17));
    }

    #[test]
    fn an_offset_in_whitespace_between_words_finds_no_term() {
        let mut index = DefinitionIndex::new();
        index.insert("rust", "A metal").unwrap();
        // The space after "rust" is byte 4 of "rust is nice"; no term covers it.
        assert_eq!(index.term_range("rust is nice", 4), None);
    }

    #[test]
    fn definition_at_returns_range_and_definition() {
        let mut index = DefinitionIndex::new();
        index.insert("rust", "A language").unwrap();
        assert_eq!(index.definition_at("use rust", 5), Some((4..8, "A language")));
        assert_eq!(index.definition_at("use rust", 1), None);
        assert!(!index.is_empty());
    }
}
