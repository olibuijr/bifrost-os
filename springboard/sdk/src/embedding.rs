//! On-device text embeddings for search: a hashed bag-of-words vector per text, compared by cosine similarity.
//!
//! This is not a neural model. Each lowercase word (the same split as [`crate::search_kit`]) is hashed into one of
//! [`DIMENSIONS`] buckets with a sign taken from a second hash bit, the counts are normalised to unit length, and two
//! texts are compared by the cosine of their vectors. Texts that share words score higher than texts that do not, which
//! is what a first system-search reranker needs. It does not know synonyms, word order or meaning, and hash collisions
//! add small noise. Nothing is downloaded or loaded: the vectors are computed from the text alone.
//!
//! ```ignore
//! let mut index = EmbeddingIndex::new();
//! index.insert("note-1", "Buy oat milk");
//! index.insert("note-2", "Call the dentist");
//! let hits = index.nearest("milk", 5);   // [("note-1", score)]
//! ```

use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Length of every vector. A fixed size keeps vectors from different texts comparable.
pub const DIMENSIONS: usize = 256;

/// FNV-1a over the bytes of a word.
fn fnv1a(word: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in word.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Lowercase runs of letters and digits.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The unit-length hashed bag-of-words vector of `text`. Text with no words gives the zero vector.
pub fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIMENSIONS];
    for w in words(text) {
        let h = fnv1a(&w);
        let bucket = (h % DIMENSIONS as u64) as usize;
        let sign = if (h >> 32) & 1 == 1 { 1.0 } else { -1.0 };
        v[bucket] += sign;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Cosine similarity of two vectors, from -1 to 1. Zero when the lengths differ or either vector is zero.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// In-memory vectors of an app's documents, keyed by document id.
#[derive(Clone, Debug, Default)]
pub struct EmbeddingIndex {
    vectors: BTreeMap<String, Vec<f32>>,
}

impl EmbeddingIndex {
    /// An empty index.
    pub fn new() -> EmbeddingIndex {
        EmbeddingIndex::default()
    }

    /// How many documents the index holds.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Whether the index holds no documents.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Adds a document, or replaces the vector of the document with the same id.
    pub fn insert(&mut self, id: &str, text: &str) {
        self.vectors.insert(id.to_string(), embed(text));
    }

    /// Removes a document; `false` when no document has that id.
    pub fn remove(&mut self, id: &str) -> bool {
        self.vectors.remove(id).is_some()
    }

    /// The documents whose vector is similar to the query text, best first (by score, then by id). Only documents
    /// with a positive score are returned, at most `limit` of them.
    pub fn nearest(&self, query: &str, limit: usize) -> Vec<(String, f32)> {
        if limit == 0 {
            return Vec::new();
        }
        let q = embed(query);
        let mut out: Vec<(String, f32)> = self
            .vectors
            .iter()
            .map(|(id, v)| (id.clone(), cosine(&q, v)))
            .filter(|(_, score)| *score > 0.0)
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_have_fixed_length_and_unit_norm() {
        let v = embed("buy oat milk");
        assert_eq!(v.len(), DIMENSIONS);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn text_without_words_is_the_zero_vector() {
        assert!(embed("  ,, ..  ").iter().all(|x| *x == 0.0));
        assert_eq!(cosine(&embed(""), &embed("milk")), 0.0);
    }

    #[test]
    fn words_are_case_insensitive() {
        assert_eq!(embed("Oat MILK"), embed("oat milk"));
    }

    #[test]
    fn identical_text_scores_one_and_shared_words_score_higher_than_none() {
        let a = embed("buy oat milk");
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
        assert!(cosine(&a, &embed("oat milk for coffee")) > cosine(&a, &embed("call the dentist tomorrow")));
    }

    #[test]
    fn nearest_ranks_the_related_document_first() {
        let mut index = EmbeddingIndex::new();
        index.insert("note-1", "Buy oat milk and bread");
        index.insert("note-2", "Call the dentist on Friday");
        index.insert("note-3", "Quarterly report draft");
        let hits = index.nearest("oat milk", 5);
        assert_eq!(hits.first().map(|h| h.0.as_str()), Some("note-1"));
        assert!(hits.iter().all(|h| h.0 != "note-3"));
    }

    #[test]
    fn nearest_respects_the_limit_and_an_empty_query_matches_nothing() {
        let mut index = EmbeddingIndex::new();
        index.insert("a", "oat milk");
        index.insert("b", "oat flakes");
        assert_eq!(index.nearest("oat", 1).len(), 1);
        assert!(index.nearest("oat", 0).is_empty());
        assert!(index.nearest("", 5).is_empty());
    }

    #[test]
    fn insert_replaces_and_remove_forgets() {
        let mut index = EmbeddingIndex::new();
        index.insert("a", "oat milk");
        index.insert("a", "dentist");
        assert_eq!(index.len(), 1);
        assert!(index.nearest("oat", 5).is_empty());
        assert!(index.remove("a"));
        assert!(!index.remove("a"));
        assert!(index.is_empty());
    }
}
