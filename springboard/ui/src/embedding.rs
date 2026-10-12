//! Text embeddings: a vocabulary that maps strings to fixed-length vectors, with distances between strings, the nearest
//! strings to a term or to a vector, and a sentence vector made from the mean of the known words. Mirrors the text
//! embedding API of the natural language framework (word and sentence embeddings, neighbours, distances), with the
//! vectors supplied by the caller. Pure data, no I/O.
//!
//! * Terms are compared case-insensitively after trimming surrounding whitespace.
//! * `DistanceType::Cosine` is `1 - cos(a, b)`, so identical directions are 0 and orthogonal ones are 1. A zero
//!   vector has no direction and is treated as distance 1 from everything.
//! * `DistanceType::Euclidean` is the straight-line distance.
//! * `neighbors` sorts by distance, then by term (the vocabulary is ordered), and never returns the query term itself.
//!
//! Limits: there is no model file reader and no bundled vectors, so a vocabulary is only as good as what the caller
//! inserts. Sentence vectors are a plain mean with no weighting, and tokens are split on non-alphanumeric characters
//! only, so a sentence with no known words has no vector.

use std::collections::BTreeMap;
use std::fmt;

/// `DistanceType`: how two vectors are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceType {
    /// `1 - cos(a, b)`.
    Cosine,
    /// Straight-line distance.
    Euclidean,
}

/// `EmbeddingError`: why a vocabulary change or query was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingError {
    /// The dimension must be at least 1.
    ZeroDimension,
    /// The term is empty after trimming.
    EmptyTerm,
    /// A vector does not have the vocabulary's dimension.
    DimensionMismatch { expected: usize, found: usize },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbeddingError::ZeroDimension => write!(f, "the dimension must be at least 1"),
            EmbeddingError::EmptyTerm => write!(f, "the term is empty"),
            EmbeddingError::DimensionMismatch { expected, found } => {
                write!(f, "expected {} dimensions, found {}", expected, found)
            }
        }
    }
}

/// `Embedding`: a vocabulary of equal-length vectors keyed by normalized term.
#[derive(Clone, Debug, Default)]
pub struct Embedding {
    dimension: usize,
    vectors: BTreeMap<String, Vec<f32>>,
}

fn normalize(term: &str) -> String {
    term.trim().to_lowercase()
}

fn dot_and_norms(a: &[f32], b: &[f32]) -> (f64, f64, f64) {
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    (dot, na.sqrt(), nb.sqrt())
}

/// Distance between two vectors of the same length. Returns `None` when the lengths differ.
pub fn distance(a: &[f32], b: &[f32], kind: DistanceType) -> Option<f64> {
    if a.len() != b.len() {
        return None;
    }
    match kind {
        DistanceType::Euclidean => {
            let sum: f64 = a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| {
                    let d = *x as f64 - *y as f64;
                    d * d
                })
                .sum();
            Some(sum.sqrt())
        }
        DistanceType::Cosine => {
            let (dot, na, nb) = dot_and_norms(a, b);
            if na == 0.0 || nb == 0.0 {
                return Some(1.0);
            }
            Some(1.0 - dot / (na * nb))
        }
    }
}

impl Embedding {
    /// An empty vocabulary whose vectors all have `dimension` components.
    pub fn new(dimension: usize) -> Result<Self, EmbeddingError> {
        if dimension == 0 {
            return Err(EmbeddingError::ZeroDimension);
        }
        Ok(Embedding { dimension, vectors: BTreeMap::new() })
    }

    /// The number of components in each vector.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// The number of terms in the vocabulary.
    pub fn vocabulary_size(&self) -> usize {
        self.vectors.len()
    }

    /// Adds a term, or replaces its vector when the term is already known.
    pub fn insert(&mut self, term: &str, vector: &[f32]) -> Result<(), EmbeddingError> {
        let key = normalize(term);
        if key.is_empty() {
            return Err(EmbeddingError::EmptyTerm);
        }
        if vector.len() != self.dimension {
            return Err(EmbeddingError::DimensionMismatch { expected: self.dimension, found: vector.len() });
        }
        self.vectors.insert(key, vector.to_vec());
        Ok(())
    }

    /// Whether the term is in the vocabulary.
    pub fn contains(&self, term: &str) -> bool {
        self.vectors.contains_key(&normalize(term))
    }

    /// The vector of a term, if it is in the vocabulary.
    pub fn vector(&self, term: &str) -> Option<&[f32]> {
        self.vectors.get(&normalize(term)).map(|v| v.as_slice())
    }

    /// Distance between two terms, or `None` when either is unknown.
    pub fn distance_between(&self, a: &str, b: &str, kind: DistanceType) -> Option<f64> {
        let va = self.vector(a)?;
        let vb = self.vector(b)?;
        distance(va, vb, kind)
    }

    /// The nearest terms to `term`, nearest first, at most `max_count`. Empty when the term is unknown.
    pub fn neighbors(&self, term: &str, max_count: usize, kind: DistanceType) -> Vec<(String, f64)> {
        let key = normalize(term);
        let query = match self.vectors.get(&key) {
            Some(v) => v,
            None => return Vec::new(),
        };
        let mut scored: Vec<(String, f64)> = self
            .vectors
            .iter()
            .filter(|(k, _)| **k != key)
            .filter_map(|(k, v)| distance(query, v, kind).map(|d| (k.clone(), d)))
            .collect();
        // Stable sort: equal distances keep the vocabulary's alphabetical order.
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_count);
        scored
    }

    /// The nearest terms to an arbitrary vector, nearest first, at most `max_count`.
    pub fn neighbors_of_vector(
        &self,
        query: &[f32],
        max_count: usize,
        kind: DistanceType,
    ) -> Result<Vec<(String, f64)>, EmbeddingError> {
        if query.len() != self.dimension {
            return Err(EmbeddingError::DimensionMismatch { expected: self.dimension, found: query.len() });
        }
        let mut scored: Vec<(String, f64)> = self
            .vectors
            .iter()
            .filter_map(|(k, v)| distance(query, v, kind).map(|d| (k.clone(), d)))
            .collect();
        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_count);
        Ok(scored)
    }

    /// The mean vector of the known words in `text`, or `None` when no word is in the vocabulary.
    pub fn sentence_vector(&self, text: &str) -> Option<Vec<f32>> {
        let mut sum = vec![0.0f32; self.dimension];
        let mut count = 0usize;
        for token in text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()) {
            if let Some(v) = self.vector(token) {
                for (s, x) in sum.iter_mut().zip(v.iter()) {
                    *s += *x;
                }
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        let n = count as f32;
        Some(sum.into_iter().map(|s| s / n).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn sample() -> Embedding {
        let mut e = Embedding::new(2).unwrap();
        e.insert("cat", &[1.0, 0.0]).unwrap();
        e.insert("dog", &[0.0, 1.0]).unwrap();
        e.insert("bird", &[1.0, 1.0]).unwrap();
        e
    }

    #[test]
    fn insert_normalizes_terms_and_checks_dimension() {
        let mut e = Embedding::new(2).unwrap();
        e.insert("  Cat ", &[1.0, 0.0]).unwrap();
        assert!(e.contains("CAT"));
        assert_eq!(e.vector("cat"), Some(&[1.0f32, 0.0][..]));
        assert_eq!(e.vocabulary_size(), 1);
        assert_eq!(
            e.insert("dog", &[1.0, 0.0, 0.0]),
            Err(EmbeddingError::DimensionMismatch { expected: 2, found: 3 })
        );
        assert_eq!(e.insert("   ", &[1.0, 0.0]), Err(EmbeddingError::EmptyTerm));
        assert_eq!(Embedding::new(0).unwrap_err(), EmbeddingError::ZeroDimension);
    }

    #[test]
    fn euclidean_distance_is_the_straight_line() {
        let mut e = Embedding::new(2).unwrap();
        e.insert("a", &[0.0, 0.0]).unwrap();
        e.insert("b", &[3.0, 4.0]).unwrap();
        let d = e.distance_between("a", "b", DistanceType::Euclidean).unwrap();
        assert!(close(d, 5.0));
        assert_eq!(e.distance_between("a", "missing", DistanceType::Euclidean), None);
    }

    #[test]
    fn cosine_distance_is_one_minus_similarity() {
        let mut e = Embedding::new(2).unwrap();
        e.insert("x", &[1.0, 0.0]).unwrap();
        e.insert("y", &[0.0, 1.0]).unwrap();
        e.insert("z", &[2.0, 0.0]).unwrap();
        e.insert("o", &[0.0, 0.0]).unwrap();
        // Orthogonal vectors are distance 1, the same direction is 0, a zero vector is 1.
        assert!(close(e.distance_between("x", "y", DistanceType::Cosine).unwrap(), 1.0));
        assert!(close(e.distance_between("x", "z", DistanceType::Cosine).unwrap(), 0.0));
        assert!(close(e.distance_between("o", "x", DistanceType::Cosine).unwrap(), 1.0));
    }

    #[test]
    fn neighbors_are_sorted_and_exclude_the_query() {
        let e = sample();
        // Cosine distance from cat: bird is 1 - 1/sqrt(2), dog is 1.
        let one = e.neighbors("cat", 1, DistanceType::Cosine);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].0, "bird");
        assert!(close(one[0].1, 1.0 - std::f64::consts::FRAC_1_SQRT_2));
        let all = e.neighbors("cat", 5, DistanceType::Cosine);
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["bird", "dog"]);
        assert!(close(all[1].1, 1.0));
        assert!(e.neighbors("unknown", 3, DistanceType::Cosine).is_empty());
    }

    #[test]
    fn neighbors_of_a_vector_need_the_right_dimension() {
        let e = sample();
        // Euclidean distance from [1, 1] is 0 to bird, 1 to cat and to dog.
        let near = e.neighbors_of_vector(&[1.0, 1.0], 1, DistanceType::Euclidean).unwrap();
        assert_eq!(near, vec![("bird".to_string(), 0.0)]);
        assert_eq!(
            e.neighbors_of_vector(&[1.0, 1.0, 1.0], 1, DistanceType::Euclidean),
            Err(EmbeddingError::DimensionMismatch { expected: 2, found: 3 })
        );
    }

    #[test]
    fn sentence_vector_is_the_mean_of_known_words() {
        let mut e = Embedding::new(2).unwrap();
        e.insert("a", &[1.0, 0.0]).unwrap();
        e.insert("b", &[3.0, 0.0]).unwrap();
        // "zzz" is unknown and does not count.
        assert_eq!(e.sentence_vector("A, b! zzz"), Some(vec![2.0f32, 0.0]));
        assert_eq!(e.sentence_vector("zzz qqq"), None);
    }
}
