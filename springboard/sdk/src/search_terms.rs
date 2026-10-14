//! Term-level index over an app's searchable items: which items contain a word, and which words start with a prefix.
//!
//! The index holds the words of each item's title, description and keywords. Words are lowercase runs of letters
//! and digits (the same split as [`crate::spellcheck::words`]). A multi-word query matches the items that contain
//! every word. Prefix lookups serve type-ahead. The index is in memory and built by the app from its own items;
//! the shell's Core Spotlight store is not read here.
//!
//! ```ignore
//! let mut terms = TermIndex::new();
//! terms.add_item(&item);                    // title, description and keywords of one CSSearchableItem
//! terms.ids_with_all("quarterly report");   // items that contain both words
//! terms.ids_with_prefix("qua");             // items with a word that starts with "qua"
//! ```
//!
//! Limits: no summarization (it needs a language model), no stemming ("reports" does not find "report"), no ranking
//! beyond the sorted id order, and no persistence.

use std::collections::{BTreeMap, BTreeSet};

use crate::quicksearch::CSSearchableItem;
/// Lowercase words of a text, split at every character that is not alphanumeric.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Words of indexed items. `postings` maps a word to the item ids that contain it; `words_of` is the reverse map,
/// kept so that an item can be removed or replaced.
#[derive(Clone, Debug, Default)]
pub struct TermIndex {
    postings: BTreeMap<String, BTreeSet<String>>,
    words_of: BTreeMap<String, BTreeSet<String>>,
}

impl TermIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of indexed items.
    pub fn len(&self) -> usize {
        self.words_of.len()
    }

    /// True when no item is indexed.
    pub fn is_empty(&self) -> bool {
        self.words_of.is_empty()
    }

    /// Indexes `text` under `id`. An id that is already indexed is replaced.
    pub fn insert(&mut self, id: &str, text: &str) {
        self.remove(id);
        let item_words: BTreeSet<String> = words(text).into_iter().collect();
        for word in &item_words {
            self.postings.entry(word.clone()).or_default().insert(id.to_string());
        }
        self.words_of.insert(id.to_string(), item_words);
    }

    /// Indexes the title, description and keywords of a searchable item, under its unique identifier.
    pub fn add_item(&mut self, item: &CSSearchableItem) {
        let set = &item.attribute_set;
        let mut text = String::new();
        for part in std::iter::once(set.title.as_str()).chain(std::iter::once(set.content_description.as_str())) {
            text.push_str(part);
            text.push(' ');
        }
        for keyword in &set.keywords {
            text.push_str(keyword);
            text.push(' ');
        }
        self.insert(&item.unique_identifier, &text);
    }

    /// Drops an item. An unknown id does nothing.
    pub fn remove(&mut self, id: &str) {
        let Some(item_words) = self.words_of.remove(id) else { return };
        for word in item_words {
            if let Some(ids) = self.postings.get_mut(&word) {
                ids.remove(id);
                if ids.is_empty() {
                    self.postings.remove(&word);
                }
            }
        }
    }

    /// The ids of the items that contain every word of `query`, in id order. An empty query matches nothing.
    pub fn ids_with_all(&self, query: &str) -> Vec<&str> {
        let query_words: BTreeSet<String> = words(query).into_iter().collect();
        let mut sets = query_words.iter().map(|w| self.postings.get(w));
        let Some(Some(first)) = sets.next() else { return Vec::new() };
        let mut result: BTreeSet<&str> = first.iter().map(String::as_str).collect();
        for set in sets {
            let Some(set) = set else { return Vec::new() };
            result.retain(|id| set.contains(*id));
        }
        result.into_iter().collect()
    }

    /// The ids of the items that have a word starting with `prefix`, in id order. The prefix is lowercased.
    pub fn ids_with_prefix(&self, prefix: &str) -> Vec<&str> {
        let prefix = prefix.to_lowercase();
        if prefix.is_empty() {
            return Vec::new();
        }
        let mut result: BTreeSet<&str> = BTreeSet::new();
        for (_, ids) in self.postings.range(prefix.clone()..).take_while(|(word, _)| word.starts_with(&prefix)) {
            result.extend(ids.iter().map(String::as_str));
        }
        result.into_iter().collect()
    }

    /// The distinct words of one item, sorted. Empty when the id is not indexed.
    pub fn words_of(&self, id: &str) -> Vec<&str> {
        self.words_of.get(id).map(|set| set.iter().map(String::as_str).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quicksearch::CSSearchableItemAttributeSet;

    fn item(id: &str, title: &str, description: &str, keywords: &[&str]) -> CSSearchableItem {
        let set = CSSearchableItemAttributeSet::new(title).content_description(description).keywords(keywords);
        CSSearchableItem::new(id, "docs", set)
    }

    #[test]
    fn a_word_finds_the_items_that_contain_it_in_any_case() {
        let mut t = TermIndex::new();
        t.insert("a", "Quarterly Report");
        t.insert("b", "Meeting notes");
        assert_eq!(t.ids_with_all("REPORT"), ["a"]);
        assert_eq!(t.ids_with_all("notes"), ["b"]);
        assert!(t.ids_with_all("missing").is_empty());
    }

    #[test]
    fn a_multi_word_query_needs_every_word() {
        let mut t = TermIndex::new();
        t.insert("a", "budget plan 2026");
        t.insert("b", "budget draft");
        assert_eq!(t.ids_with_all("budget plan"), ["a"]);
        assert_eq!(t.ids_with_all("budget"), ["a", "b"]);
        assert!(t.ids_with_all("plan draft").is_empty());
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let mut t = TermIndex::new();
        t.insert("a", "text");
        assert!(t.ids_with_all("").is_empty());
        assert!(t.ids_with_all("  -- ").is_empty());
        assert!(t.ids_with_prefix("").is_empty());
    }

    #[test]
    fn a_prefix_finds_items_with_a_word_that_starts_with_it() {
        let mut t = TermIndex::new();
        t.insert("a", "quarterly");
        t.insert("b", "quartz rock");
        t.insert("c", "quiet");
        assert_eq!(t.ids_with_prefix("quarter"), ["a"]);
        assert_eq!(t.ids_with_prefix("qua"), ["a", "b"]);
        assert!(t.ids_with_prefix("zeta").is_empty());
    }

    #[test]
    fn an_item_is_replaced_and_removed_cleanly() {
        let mut t = TermIndex::new();
        t.insert("a", "old words");
        t.insert("a", "new words");
        assert!(t.ids_with_all("old").is_empty());
        assert_eq!(t.ids_with_all("new"), ["a"]);
        assert_eq!(t.len(), 1);
        t.remove("a");
        assert!(t.is_empty());
        assert!(t.ids_with_prefix("new").is_empty());
        t.remove("a");
    }

    #[test]
    fn a_searchable_item_is_indexed_by_title_description_and_keywords() {
        let mut t = TermIndex::new();
        t.add_item(&item("n1", "Shopping", "milk and eggs", &["groceries"]));
        assert_eq!(t.ids_with_all("milk"), ["n1"]);
        assert_eq!(t.ids_with_all("groceries shopping"), ["n1"]);
        assert_eq!(t.words_of("n1"), ["and", "eggs", "groceries", "milk", "shopping"]);
        assert!(t.words_of("unknown").is_empty());
    }
}
