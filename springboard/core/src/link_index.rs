//! Document link index: records which documents link to which, so an app can list the backlinks of a document.
//!
//! The index is in memory and pure. Persisting it and watching the files it describes are the caller's concern.
//! Links are written as `[[Target]]` in the text of a document, and `wiki_links` finds them.

use std::collections::{BTreeMap, BTreeSet};

/// The outgoing links of each document, with the reverse view that answers backlink queries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkIndex {
    outgoing: BTreeMap<String, BTreeSet<String>>,
}

impl LinkIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the outgoing links of `document` with `targets`. A document never links to itself, so that
    /// target is dropped. An empty set removes the document from the index.
    pub fn set_links<I, S>(&mut self, document: &str, targets: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let links: BTreeSet<String> = targets
            .into_iter()
            .map(|t| -> String { t.into() })
            .filter(|t| t.as_str() != document)
            .collect();
        if links.is_empty() {
            self.outgoing.remove(document);
        } else {
            self.outgoing.insert(document.to_string(), links);
        }
    }

    /// Removes `document` and its outgoing links. Returns whether it was indexed.
    pub fn remove(&mut self, document: &str) -> bool {
        self.outgoing.remove(document).is_some()
    }

    /// The targets `document` links to, in sorted order.
    pub fn links_from(&self, document: &str) -> Vec<&str> {
        self.outgoing
            .get(document)
            .map(|links| links.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// The documents that link to `target`, in sorted order.
    pub fn backlinks(&self, target: &str) -> Vec<&str> {
        self.outgoing
            .iter()
            .filter(|(_, links)| links.contains(target))
            .map(|(document, _)| document.as_str())
            .collect()
    }

    /// The number of documents that have at least one outgoing link.
    pub fn len(&self) -> usize {
        self.outgoing.len()
    }

    /// Whether no document has an outgoing link.
    pub fn is_empty(&self) -> bool {
        self.outgoing.is_empty()
    }
}

/// The `[[Target]]` links in `text`, in the order they first appear, without duplicates. Empty brackets and
/// unclosed brackets are ignored, and surrounding spaces are trimmed from each target.
pub fn wiki_links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let target = after[..end].trim();
        if !target.is_empty() && !found.iter().any(|t| t.as_str() == target) {
            found.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    found
}
