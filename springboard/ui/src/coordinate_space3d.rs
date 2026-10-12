//! A tree of 3D coordinate spaces: each space has a parent and a transform from its own coordinates to the parent's.
//! Points convert between any two spaces in the tree by going through the world reference, the root that every
//! space eventually reaches. Pure matrix math, built on `linalg::Matrix4`. Matrices are column-major and a point
//! is transformed as `M * (x, y, z, 1)`, so `to_world(child) = to_world(parent) * to_parent(child)`.
//! The world reference is space 0 and always exists. A space's transform must be invertible, because a conversion
//! into that space uses the inverse.

use crate::linalg::{Matrix4, Vector3};

/// Index of a space in a `SpaceTree`. The world reference is `SpaceTree::WORLD`.
pub type SpaceId = usize;

#[derive(Clone, Debug)]
struct Space {
    parent: Option<SpaceId>,
    to_parent: Matrix4,
}

/// The spaces of a scene. Spaces are added below an existing one, so the parent of a space always has a smaller
/// index and the tree cannot contain a cycle.
#[derive(Clone, Debug)]
pub struct SpaceTree {
    spaces: Vec<Space>,
}

impl Default for SpaceTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SpaceTree {
    /// The world reference: the root every other space reaches.
    pub const WORLD: SpaceId = 0;

    /// A tree that holds only the world reference.
    pub fn new() -> Self {
        Self { spaces: vec![Space { parent: None, to_parent: Matrix4::identity() }] }
    }

    /// Adds a space below `parent`, whose coordinates reach the parent through `to_parent`. Returns None if the
    /// parent does not exist or the transform is not invertible.
    pub fn add(&mut self, parent: SpaceId, to_parent: Matrix4) -> Option<SpaceId> {
        if parent >= self.spaces.len() {
            return None;
        }
        to_parent.inverse()?;
        self.spaces.push(Space { parent: Some(parent), to_parent });
        Some(self.spaces.len() - 1)
    }

    /// Number of spaces, including the world reference.
    pub fn len(&self) -> usize {
        self.spaces.len()
    }

    /// Always false: the world reference is always present.
    pub fn is_empty(&self) -> bool {
        self.spaces.is_empty()
    }

    /// The parent of a space, or None for the world reference and unknown ids.
    pub fn parent(&self, id: SpaceId) -> Option<SpaceId> {
        self.spaces.get(id).and_then(|s| s.parent)
    }

    /// The transform that takes a point in `id` to the world reference. None if the space does not exist.
    pub fn to_world(&self, id: SpaceId) -> Option<Matrix4> {
        if id >= self.spaces.len() {
            return None;
        }
        let mut m = Matrix4::identity();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let s = &self.spaces[c];
            m = s.to_parent * m;
            cur = s.parent;
        }
        Some(m)
    }

    /// The transform that takes a point in `from` to the coordinates of `to`. None if either space is unknown.
    pub fn transform(&self, from: SpaceId, to: SpaceId) -> Option<Matrix4> {
        let from_world = self.to_world(from)?;
        let to_world_inverse = self.to_world(to)?.inverse()?;
        Some(to_world_inverse * from_world)
    }

    /// Converts a point from one space to another. None if a space is unknown or the point is at infinity.
    pub fn convert_point(&self, point: Vector3, from: SpaceId, to: SpaceId) -> Option<Vector3> {
        self.transform(from, to)?.transform_point(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vector3, b: Vector3, tol: f32) -> bool {
        (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.z - b.z).abs() <= tol
    }

    #[test]
    fn child_offset_by_a_translation_maps_its_origin_to_that_offset() {
        let mut t = SpaceTree::new();
        let child = t.add(SpaceTree::WORLD, Matrix4::translation(1.0, 0.0, 0.0)).unwrap();
        let origin = Vector3::new(0.0, 0.0, 0.0);
        assert!(close(t.convert_point(origin, child, SpaceTree::WORLD).unwrap(), Vector3::new(1.0, 0.0, 0.0), 1e-5));
        assert!(close(t.convert_point(Vector3::new(1.0, 0.0, 0.0), SpaceTree::WORLD, child).unwrap(), origin, 1e-5));
    }

    #[test]
    fn grandchild_point_goes_through_the_scale_and_the_child_offset() {
        let mut t = SpaceTree::new();
        let child = t.add(SpaceTree::WORLD, Matrix4::translation(1.0, 0.0, 0.0)).unwrap();
        let grand = t.add(child, Matrix4::scaling(2.0, 2.0, 2.0)).unwrap();
        // (1,1,1) doubles to (2,2,2) in the child, then moves by (1,0,0) to (3,2,2) in the world.
        let world = t.convert_point(Vector3::new(1.0, 1.0, 1.0), grand, SpaceTree::WORLD).unwrap();
        assert!(close(world, Vector3::new(3.0, 2.0, 2.0), 1e-5));
        assert_eq!(t.parent(grand), Some(child));
        assert_eq!(t.parent(SpaceTree::WORLD), None);
    }

    #[test]
    fn sibling_conversion_goes_through_the_world() {
        let mut t = SpaceTree::new();
        let a = t.add(SpaceTree::WORLD, Matrix4::translation(1.0, 0.0, 0.0)).unwrap();
        let b = t.add(SpaceTree::WORLD, Matrix4::translation(0.0, 5.0, 0.0)).unwrap();
        // The origin of a is at world (1,0,0), which is (1,-5,0) in b.
        let in_b = t.convert_point(Vector3::new(0.0, 0.0, 0.0), a, b).unwrap();
        assert!(close(in_b, Vector3::new(1.0, -5.0, 0.0), 1e-5));
    }

    #[test]
    fn transform_matrix_of_the_same_space_is_the_identity() {
        let mut t = SpaceTree::new();
        let a = t.add(SpaceTree::WORLD, Matrix4::translation(4.0, 4.0, 4.0)).unwrap();
        let m = t.transform(a, a).unwrap();
        assert!(close(m.transform_point(Vector3::new(2.0, 3.0, 5.0)).unwrap(), Vector3::new(2.0, 3.0, 5.0), 1e-5));
        assert_eq!(t.len(), 2);
        assert!(!t.is_empty());
    }

    #[test]
    fn unknown_parent_and_singular_transform_are_rejected() {
        let mut t = SpaceTree::new();
        assert!(t.add(7, Matrix4::identity()).is_none());
        // A zero scale on x flattens the space, so it has no inverse and cannot be a space.
        assert!(t.add(SpaceTree::WORLD, Matrix4::scaling(0.0, 1.0, 1.0)).is_none());
        assert_eq!(t.len(), 1);
        assert!(t.to_world(9).is_none());
        assert!(t.convert_point(Vector3::new(0.0, 0.0, 0.0), 9, SpaceTree::WORLD).is_none());
    }
}
