//! Document search for apps that keep their own text: the term-level index of Search Kit (`SKIndexCreate`,
//! `SKIndexAddDocument`, `SKSearchCreate`, `SKSearchFindMatches`, `SKSummaryCreateWithString`).
//!
//! An app adds documents (an id and its text), searches them by words, and gets a short summary of the sentence that
//! holds the match. Words are lowercase runs of letters and digits; a query word matches a document word that starts
//! with it (type-ahead), and a multi-word query needs every word. The index is in memory and lives in the app; the
//! app keeps it on disk with [`SearchIndex::to_text`] and [`SearchIndex::from_text`]. Nothing here reaches the shell.
//!
//! ```ignore
//! let mut index = SearchIndex::new();
//! index.add_document("note-1", "Buy oat milk. Call the dentist.");
//! let hits = index.find_matches("milk", 10);            // [Match { id: "note-1", score: 1 }]
//! let line = index.summary("note-1", "milk", 80);      // Some("Buy oat milk.")
//! ```
//!
//! Ids and text are stored with tabs and line breaks turned to spaces, so an id reads back as the form it was stored in.

use std::collections::{BTreeMap, BTreeSet};

/// One document a query matched: its id and the number of query-word occurrences in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub id: String,
    pub score: usize,
}

/// An in-memory word index over documents: each word maps to the documents that contain it, with the counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchIndex {
    docs: BTreeMap<String, String>,
    terms: BTreeMap<String, BTreeMap<String, usize>>,
}

/// Lowercase runs of letters and digits.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Tabs and line breaks become spaces, so an id or text fits one line of the text form.
fn clean(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\t' | '\r' | '\n') { ' ' } else { c }).collect()
}

/// Whether any word of `sentence` starts with one of the query words.
fn sentence_matches(sentence: &str, query: &[String]) -> bool {
    words(sentence).iter().any(|w| query.iter().any(|q| w.starts_with(q.as_str())))
}

impl SearchIndex {
    /// An empty index.
    pub fn new() -> SearchIndex {
        SearchIndex::default()
    }

    /// How many documents the index holds.
    pub fn document_count(&self) -> usize {
        self.docs.len()
    }

    /// Adds a document, or replaces the document with the same id.
    pub fn add_document(&mut self, id: &str, text: &str) {
        let id = clean(id);
        self.remove_document(&id);
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for w in words(text) {
            *counts.entry(w).or_insert(0) += 1;
        }
        for (word, n) in counts {
            self.terms.entry(word).or_default().insert(id.clone(), n);
        }
        self.docs.insert(id, clean(text));
    }

    /// Removes a document; `false` when no document has that id.
    pub fn remove_document(&mut self, id: &str) -> bool {
        let id = clean(id);
        if self.docs.remove(&id).is_none() {
            return false;
        }
        self.terms.retain(|_, postings| {
            postings.remove(&id);
            !postings.is_empty()
        });
        true
    }

    /// The documents that contain every query word (a word matches a document word it starts), best first: by the
    /// number of matching occurrences, then by id. At most `limit` results. An empty query matches nothing.
    pub fn find_matches(&self, query: &str, limit: usize) -> Vec<Match> {
        let query = words(query);
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut per_word: Vec<BTreeMap<String, usize>> = Vec::new();
        for q in &query {
            let mut hits: BTreeMap<String, usize> = BTreeMap::new();
            for (word, postings) in self.terms.range(q.clone()..) {
                if !word.starts_with(q.as_str()) {
                    break;
                }
                for (id, n) in postings {
                    *hits.entry(id.clone()).or_insert(0) += n;
                }
            }
            if hits.is_empty() {
                return Vec::new();
            }
            per_word.push(hits);
        }
        let mut ids: BTreeSet<String> = per_word[0].keys().cloned().collect();
        for hits in &per_word[1..] {
            ids.retain(|id| hits.contains_key(id));
        }
        let mut out: Vec<Match> = ids
            .into_iter()
            .map(|id| {
                let score = per_word.iter().map(|hits| hits[&id]).sum();
                Match { id, score }
            })
            .collect();
        out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        out.truncate(limit);
        out
    }

    /// The sentence of document `id` that holds the first query word (the first sentence when none does), trimmed and
    /// cut to `max_chars` characters with an ellipsis. `None` for an unknown id.
    pub fn summary(&self, id: &str, query: &str, max_chars: usize) -> Option<String> {
        let text = self.docs.get(&clean(id))?;
        let query = words(query);
        let sentences: Vec<&str> = text.split_inclusive(|c: char| matches!(c, '.' | '!' | '?')).collect();
        let sentence = sentences.iter().copied().find(|s| sentence_matches(s, &query)).or_else(|| sentences.first().copied())?;
        let s = sentence.trim();
        if s.chars().count() <= max_chars {
            return Some(s.to_string());
        }
        let cut: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        Some(format!("{}\u{2026}", cut.trim_end()))
    }

    /// The index as text: one document per line, `id<TAB>text`. Documents are in id order.
    pub fn to_text(&self) -> String {
        self.docs.iter().map(|(id, text)| format!("{id}\t{text}\n")).collect()
    }

    /// Rebuilds an index from [`SearchIndex::to_text`]; lines without a tab are skipped.
    pub fn from_text(text: &str) -> SearchIndex {
        let mut index = SearchIndex::new();
        for line in text.lines() {
            if let Some((id, body)) = line.split_once('\t') {
                index.add_document(id, body);
            }
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SearchIndex {
        let mut i = SearchIndex::new();
        i.add_document("a", "Buy oat milk. Call the dentist.");
        i.add_document("b", "Milk the cow, milk again.");
        i.add_document("c", "Dentist appointment on Monday.");
        i
    }

    #[test]
    fn a_word_finds_each_document_that_holds_it_with_its_count() {
        let hits = sample().find_matches("milk", 10);
        assert_eq!(hits, vec![Match { id: "b".into(), score: 2 }, Match { id: "a".into(), score: 1 }]);
    }

    #[test]
    fn every_query_word_is_needed_and_a_prefix_matches() {
        let i = sample();
        assert_eq!(i.find_matches("dentist monday", 10), vec![Match { id: "c".into(), score: 2 }]);
        assert_eq!(i.find_matches("mil", 10).len(), 2, "a prefix finds the words that start with it");
        assert!(i.find_matches("dentist cow", 10).is_empty());
        assert!(i.find_matches("  ", 10).is_empty());
    }

    #[test]
    fn results_are_limited_and_replacing_or_removing_a_document_updates_the_words() {
        let mut i = sample();
        assert_eq!(i.find_matches("milk", 1).len(), 1);
        i.add_document("b", "Nothing to see.");
        assert_eq!(i.find_matches("milk", 10), vec![Match { id: "a".into(), score: 1 }]);
        assert!(i.remove_document("a"));
        assert!(i.find_matches("milk", 10).is_empty());
        assert!(!i.remove_document("a"));
        assert_eq!(i.document_count(), 2);
    }

    #[test]
    fn a_summary_is_the_sentence_with_the_match_cut_to_the_limit() {
        let i = sample();
        assert_eq!(i.summary("a", "dentist", 80).as_deref(), Some("Call the dentist."));
        assert_eq!(i.summary("a", "nothing", 80).as_deref(), Some("Buy oat milk."), "no match gives the first sentence");
        assert_eq!(i.summary("a", "dentist", 8).as_deref(), Some("Call th\u{2026}"));
        assert_eq!(i.summary("zz", "dentist", 80), None);
    }

    #[test]
    fn the_text_form_round_trips_and_keeps_one_document_per_line() {
        let mut i = SearchIndex::new();
        i.add_document("n\t1", "Two\nlines\there");
        let text = i.to_text();
        assert_eq!(text.lines().count(), 1);
        let back = SearchIndex::from_text(&text);
        assert_eq!(back, i);
        assert_eq!(back.find_matches("lines", 5)[0].id, "n 1");
        assert_eq!(SearchIndex::from_text("no tab here\n").document_count(), 0);
    }
}
