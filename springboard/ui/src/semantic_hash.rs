//! A local semantic score for system search: [`HashedSemantic`] plugs into [`SemanticHook`](crate::search_rank::SemanticHook)
//! so [`rank`](crate::search_rank::rank) can reorder candidates by how close their text is to the query.
//!
//! Not a neural model (none is bundled). Each text becomes a unit-length vector of [`DIMENSIONS`] hashed features: its
//! folded words (the search tokenizer, so "Café" is "cafe") and the character trigrams of each word, padded with `^` and
//! `$`. The trigrams make word forms close ("running" and "run" share `^ru` and `run`), which whole-word hashing cannot.
//! Two texts score the cosine of their vectors, clamped to 0..=1. There are no synonyms, no word order and no learned
//! meaning; hash collisions add noise of about 1/sqrt(DIMENSIONS), which the ranker's weight keeps small.
//!
//! The ranker only scores items that already hold a query word, so this changes the order of matches, not which items match.

use crate::search_rank::{tokenize, SemanticHook};
use std::sync::Mutex;

/// Length of every vector.
pub const DIMENSIONS: usize = 512;
/// A whole word counts this much; each of its trigrams counts [`TRIGRAM_WEIGHT`].
const WORD_WEIGHT: f32 = 1.5;
const TRIGRAM_WEIGHT: f32 = 0.5;
/// The longest text embedded (in characters), so a huge document body costs a bounded amount.
const MAX_CHARS: usize = 4096;

/// FNV-1a over the bytes of `word`.
pub fn fnv1a(word: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in word.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn add(v: &mut [f32], feature: &str, weight: f32) {
    let h = fnv1a(feature);
    let sign = if (h >> 32) & 1 == 1 { weight } else { -weight };
    v[(h % DIMENSIONS as u64) as usize] += sign;
}

/// The unit-length vector of `text` (words and word trigrams). Text with no words gives the zero vector.
pub fn embed_subword(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIMENSIONS];
    let clipped: String = text.chars().take(MAX_CHARS).collect();
    for word in tokenize(&clipped) {
        add(&mut v, &word, WORD_WEIGHT);
        let padded: Vec<char> = std::iter::once('^').chain(word.chars()).chain(std::iter::once('$')).collect();
        for gram in padded.windows(3) {
            let g: String = gram.iter().collect();
            add(&mut v, &g, TRIGRAM_WEIGHT);
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Cosine of two vectors, 0 when either is the zero vector or the lengths differ.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let (na, nb) = (a.iter().map(|x| x * x).sum::<f32>().sqrt(), b.iter().map(|x| x * x).sum::<f32>().sqrt());
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// The hashed semantic score as a [`SemanticHook`]. It remembers the last query's vector, since one query is scored
/// against many items.
#[derive(Default)]
pub struct HashedSemantic {
    last: Mutex<Option<(String, Vec<f32>)>>,
}

impl HashedSemantic {
    pub fn new() -> HashedSemantic {
        HashedSemantic::default()
    }

    fn query_vector(&self, query: &str) -> Vec<f32> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((q, v)) = last.as_ref() {
            if q == query {
                return v.clone();
            }
        }
        let v = embed_subword(query);
        *last = Some((query.to_string(), v.clone()));
        v
    }
}

impl SemanticHook for HashedSemantic {
    /// `None` when the query or the text has no words; otherwise the cosine, never below 0.
    fn similarity(&self, query: &str, text: &str) -> Option<f32> {
        let q = self.query_vector(query);
        let t = embed_subword(text);
        if q.iter().all(|x| *x == 0.0) || t.iter().all(|x| *x == 0.0) {
            return None;
        }
        Some(cosine(&q, &t).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search_rank::{rank, Item, Kind, NoSemantic, Usage};

    #[test]
    fn vectors_are_unit_length_and_folded() {
        let v = embed_subword("Buy oat milk");
        assert_eq!(v.len(), DIMENSIONS);
        assert!((v.iter().map(|x| x * x).sum::<f32>().sqrt() - 1.0).abs() < 1e-5);
        assert_eq!(embed_subword("Café"), embed_subword("cafe"), "diacritics and case are folded like the search index");
        assert!(embed_subword(" ,, ").iter().all(|x| *x == 0.0));
    }

    #[test]
    fn word_forms_are_close_and_unrelated_words_are_not() {
        let h = HashedSemantic::new();
        let near = h.similarity("running", "run").unwrap();
        let far = h.similarity("running", "dentist").unwrap();
        assert!(near > 0.2, "{near}");
        assert!(far < 0.15, "{far}");
        assert!((h.similarity("oat milk", "Oat Milk").unwrap() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn nothing_to_compare_gives_no_score() {
        let h = HashedSemantic::new();
        assert_eq!(h.similarity("", "milk"), None);
        assert_eq!(h.similarity("milk", "  "), None);
    }

    #[test]
    fn the_cached_query_vector_follows_the_query() {
        let h = HashedSemantic::new();
        let a = h.similarity("milk", "oat milk").unwrap();
        let b = h.similarity("dentist", "oat milk").unwrap();
        let a2 = h.similarity("milk", "oat milk").unwrap();
        assert!(a > b);
        assert_eq!(a, a2);
    }

    #[test]
    fn very_long_text_is_clipped() {
        let long = "milk ".repeat(100_000);
        assert!((embed_subword(&long).iter().map(|x| x * x).sum::<f32>().sqrt() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_hook_raises_the_score_of_close_text_and_never_changes_what_matches() {
        let mut a = Item::new("a", "Notes", Kind::Document);
        a.text = "grocery run oat milk".into();
        let mut b = Item::new("b", "Notes", Kind::Document);
        b.text = "grocery oat quarterly report appendix tables charts".into();
        let items = [b, a];
        let with = rank("oat grocery", &items, &Usage::default(), 0, &HashedSemantic::new());
        let without = rank("oat grocery", &items, &Usage::default(), 0, &NoSemantic);
        assert_eq!(with.len(), without.len());
        let score = |r: &[(usize, f32)], id: &str| r.iter().find(|(i, _)| items[*i].id == id).unwrap().1;
        assert!(score(&with, "a") > score(&without, "a"));
        assert!(score(&with, "a") > score(&with, "b"), "the shorter, closer text ranks above the other");
    }
}
