//! Face grouping for `FaceGroupAnalyzer`: a store of face embeddings per image asset, and grouping of the faces into
//! entities by cosine similarity. Pure data, no I/O, no model.
//!
//! * There is no face detector here. The caller supplies one embedding per face, produced elsewhere.
//! * Two faces are linked when the cosine similarity of their embeddings is at least the threshold. An entity is a
//!   connected group of linked faces (single linkage), so a chain of similar faces joins into one entity.
//! * A zero vector, or two vectors of different length, have similarity 0 with everything.
//! * Face ids count up from 1 in insertion order and are never reused, even after a delete.
//! * Entities are listed in the order of their first face, and each lists its face ids in ascending order.
//! * `try_insert_face` and `try_entities` refuse bad inputs with a typed error instead of storing or grouping them.

use crate::media_intelligence_error::{check_embedding, check_threshold, MediaIntelligenceError};

/// A stored face: its id, the image asset it came from, and its embedding.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub id: u64,
    pub asset: u64,
    pub embedding: Vec<f64>,
}

/// The faces of all assets in one analyzer.
#[derive(Clone, Debug)]
pub struct FaceGroups {
    faces: Vec<Face>,
    next_id: u64,
}

impl Default for FaceGroups {
    fn default() -> Self {
        Self::new()
    }
}

impl FaceGroups {
    pub fn new() -> Self {
        FaceGroups { faces: Vec::new(), next_id: 1 }
    }

    /// Stores a face of `asset` with `embedding` and returns its id.
    pub fn insert_face(&mut self, asset: u64, embedding: Vec<f64>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.faces.push(Face { id, asset, embedding });
        id
    }

    /// Removes every face of `asset`. Returns how many faces were removed.
    pub fn delete_asset(&mut self, asset: u64) -> usize {
        let before = self.faces.len();
        self.faces.retain(|f| f.asset != asset);
        before - self.faces.len()
    }

    /// Removes every face of every asset.
    pub fn delete_all(&mut self) {
        self.faces.clear();
    }

    /// `insert_face` with the embedding checked first: it must be non-empty, finite, and as long as the faces already
    /// stored. A refused face is not stored.
    pub fn try_insert_face(&mut self, asset: u64, embedding: Vec<f64>) -> Result<u64, MediaIntelligenceError> {
        let expected = self.faces.first().map(|f| f.embedding.len());
        check_embedding(&embedding, expected)?;
        Ok(self.insert_face(asset, embedding))
    }

    /// `entities` with the threshold checked first: it must be finite and within -1 to 1.
    pub fn try_entities(&self, threshold: f64) -> Result<Vec<Vec<u64>>, MediaIntelligenceError> {
        check_threshold(threshold)?;
        Ok(self.entities(threshold))
    }

    /// The ids of the faces of `asset`, in insertion order.
    pub fn faces_in_asset(&self, asset: u64) -> Vec<u64> {
        self.faces.iter().filter(|f| f.asset == asset).map(|f| f.id).collect()
    }

    /// The entities at `threshold`: each one is the ascending list of its face ids.
    pub fn entities(&self, threshold: f64) -> Vec<Vec<u64>> {
        let n = self.faces.len();
        let mut parent: Vec<usize> = (0..n).collect();
        for i in 0..n {
            for j in i + 1..n {
                if cosine(&self.faces[i].embedding, &self.faces[j].embedding) >= threshold {
                    let a = find(&mut parent, i);
                    let b = find(&mut parent, j);
                    if a != b {
                        // the smaller index stays the root, so each group is named by its first face
                        parent[a.max(b)] = a.min(b);
                    }
                }
            }
        }
        let mut groups: Vec<Vec<u64>> = Vec::new();
        let mut slot: Vec<Option<usize>> = vec![None; n];
        for i in 0..n {
            let root = find(&mut parent, i);
            let group = slot[root];
            match group {
                Some(g) => groups[g].push(self.faces[i].id),
                None => {
                    slot[root] = Some(groups.len());
                    groups.push(vec![self.faces[i].id]);
                }
            }
        }
        groups
    }
}

/// The root of `i` in the union-find forest, with path halving.
fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Cosine similarity of `a` and `b`. 0 when the lengths differ, when either vector is empty or zero.
fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f64 = a.iter().map(|x| x * x).sum();
    let nb: f64 = b.iter().map(|y| y * y).sum();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Face 1 [1,0] and face 2 [0,1] on asset 1. Face 3 [1,0] and face 4 [1,1] on asset 2.
    /// Similarities: 1-3 = 1, 1-2 = 0, 1-4 = 2-4 = 3-4 = 1/sqrt(2) = 0.70710678.
    fn sample() -> FaceGroups {
        let mut g = FaceGroups::new();
        assert_eq!(g.insert_face(1, vec![1.0, 0.0]), 1);
        assert_eq!(g.insert_face(1, vec![0.0, 1.0]), 2);
        assert_eq!(g.insert_face(2, vec![1.0, 0.0]), 3);
        assert_eq!(g.insert_face(2, vec![1.0, 1.0]), 4);
        g
    }

    #[test]
    fn high_threshold_keeps_only_identical_faces_together() {
        assert_eq!(sample().entities(0.9), vec![vec![1, 3], vec![2], vec![4]]);
    }

    #[test]
    fn the_threshold_is_inclusive_at_the_similarity_value() {
        // 0.70710678 is not at least 0.71, so the 1/sqrt(2) links are not made
        assert_eq!(sample().entities(0.71), vec![vec![1, 3], vec![2], vec![4]]);
        // 0.70710678 is at least 0.7, so face 4 joins 1 and 2, and 3 joins through 1
        assert_eq!(sample().entities(0.7), vec![vec![1, 2, 3, 4]]);
    }

    #[test]
    fn exact_match_passes_a_threshold_of_one() {
        assert_eq!(sample().entities(1.0), vec![vec![1, 3], vec![2], vec![4]]);
    }

    #[test]
    fn zero_vectors_and_mismatched_lengths_never_link() {
        let mut g = FaceGroups::new();
        g.insert_face(1, vec![0.0, 0.0]);
        g.insert_face(1, vec![1.0, 0.0]);
        assert_eq!(g.entities(0.5), vec![vec![1], vec![2]]);
        let mut h = FaceGroups::new();
        h.insert_face(1, vec![1.0, 0.0]);
        h.insert_face(2, vec![1.0, 0.0, 0.0]);
        assert_eq!(h.entities(0.5), vec![vec![1], vec![2]]);
    }

    #[test]
    fn deleting_an_asset_removes_its_faces_and_ids_are_not_reused() {
        let mut g = sample();
        assert_eq!(g.faces_in_asset(1), vec![1, 2]);
        assert_eq!(g.delete_asset(2), 2);
        assert_eq!(g.entities(0.9), vec![vec![1], vec![2]]);
        assert_eq!(g.insert_face(3, vec![0.0, 1.0]), 5);
    }

    #[test]
    fn delete_all_leaves_no_entities() {
        let mut g = sample();
        g.delete_all();
        assert!(g.entities(0.5).is_empty());
        assert!(g.faces_in_asset(1).is_empty());
    }
}
