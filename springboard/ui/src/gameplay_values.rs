//! Game-logic value types: the score bounds used by strategist models, axis-aligned boxes and rectangles for spatial
//! trees (`GKBox`, `GKQuad`), a triangle of a navigation mesh (`GKTriangle`), and the option sets for mesh
//! triangulation and tree splitting. Pure values with no I/O; the spatial and graph modules take these types as input.

/// Largest score a game model may report (`GKGameModelMaxScore`). The platform documentation gives no numeric value.
/// The bounds are symmetric and well inside `i64`, so a strategist can negate a score without overflow.
pub const GAME_MODEL_MAX_SCORE: i64 = i32::MAX as i64;

/// Smallest score a game model may report (`GKGameModelMinScore`).
pub const GAME_MODEL_MIN_SCORE: i64 = -GAME_MODEL_MAX_SCORE;

/// An axis-aligned box in three dimensions, from its lowest corner to its highest corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3 {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Box3 {
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    /// A box with no extent is valid when `min` does not exceed `max` on any axis.
    pub fn is_valid(&self) -> bool {
        (0..3).all(|i| self.min[i] <= self.max[i])
    }

    /// Whether `p` lies inside the box, with both faces included.
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }

    /// Whether the two boxes share any volume or touch on a face.
    pub fn intersects(&self, other: &Box3) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && other.min[i] <= self.max[i])
    }
}

/// An axis-aligned rectangle in two dimensions, from its lowest corner to its highest corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Quad {
    pub fn new(min: [f32; 2], max: [f32; 2]) -> Self {
        Self { min, max }
    }

    /// A rectangle is valid when `min` does not exceed `max` on either axis.
    pub fn is_valid(&self) -> bool {
        self.min[0] <= self.max[0] && self.min[1] <= self.max[1]
    }

    /// Whether `p` lies inside the rectangle, with its edges included.
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.min[0] && p[0] <= self.max[0] && p[1] >= self.min[1] && p[1] <= self.max[1]
    }

    /// Whether the two rectangles overlap or touch along an edge.
    pub fn intersects(&self, other: &Quad) -> bool {
        self.min[0] <= other.max[0]
            && other.min[0] <= self.max[0]
            && self.min[1] <= other.max[1]
            && other.min[1] <= self.max[1]
    }
}

/// A triangle of three 3D points, typically one polygon of a navigation mesh. Containment tests use the x and y
/// coordinates only, which is how a ground-plane mesh is queried.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub points: [[f32; 3]; 3],
}

impl Triangle {
    pub fn new(points: [[f32; 3]; 3]) -> Self {
        Self { points }
    }

    /// The cross product of the two edges leaving the first point. Its length is twice the triangle's area, and its
    /// direction follows the right-hand rule for the order of the points.
    pub fn normal(&self) -> [f32; 3] {
        let [a, b, c] = self.points;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    }

    /// The area of the triangle in 3D space.
    pub fn area(&self) -> f32 {
        let n = self.normal();
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
    }

    /// Twice the signed area of the triangle projected onto the x/y plane: positive when the points wind
    /// counter-clockwise seen from +z, negative when clockwise, zero when the projection is degenerate.
    pub fn signed_area2_xy(&self) -> f32 {
        self.normal()[2]
    }

    /// The centroid of the three points.
    pub fn center(&self) -> [f32; 3] {
        let [a, b, c] = self.points;
        [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0]
    }

    /// Whether the x/y position `p` lies inside the triangle's projection or on one of its edges, for either winding
    /// order. A triangle whose projection has no area contains nothing.
    pub fn contains_xy(&self, p: [f32; 2]) -> bool {
        if self.signed_area2_xy() == 0.0 {
            return false;
        }
        let [a, b, c] = self.points;
        let d1 = edge_sign([a[0], a[1]], [b[0], b[1]], p);
        let d2 = edge_sign([b[0], b[1]], [c[0], c[1]], p);
        let d3 = edge_sign([c[0], c[1]], [a[0], a[1]], p);
        let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(has_neg && has_pos)
    }
}

fn edge_sign(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (p[0] - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (p[1] - b[1])
}

/// Where graph nodes are placed when a mesh graph is generated (`GKMeshGraphTriangulationMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriangulationMode {
    /// One node at each vertex of the mesh.
    Vertices,
    /// One node at the center of each polygon.
    Centers,
    /// One node at the midpoint of each edge.
    EdgeMidpoints,
}

/// How a spatial tree splits a full group of elements (`GKRTreeSplitStrategy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitStrategy {
    /// Split the group in half by insertion order.
    Halve,
    /// Split along a line that puts half of the elements on each side.
    Linear,
    /// Split into the subgroups that occupy the least area.
    Quadratic,
    /// Split into the subgroups whose areas overlap the least.
    ReduceOverlap,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_bounds_are_symmetric() {
        assert!(GAME_MODEL_MAX_SCORE > 0);
        assert_eq!(GAME_MODEL_MIN_SCORE, -GAME_MODEL_MAX_SCORE);
        // A strategist negates scores, so negation must stay in range.
        assert_eq!(-GAME_MODEL_MIN_SCORE, GAME_MODEL_MAX_SCORE);
    }

    #[test]
    fn box_validity_and_containment_include_faces() {
        let b = Box3::new([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        assert!(b.is_valid());
        assert!(b.contains([0.0, 1.0, 2.0]));
        assert!(!b.contains([2.1, 1.0, 1.0]));
        assert!(!Box3::new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]).is_valid());
    }

    #[test]
    fn box_intersection_counts_touching_faces() {
        let a = Box3::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let touching = Box3::new([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let apart = Box3::new([1.5, 0.0, 0.0], [2.0, 1.0, 1.0]);
        assert!(a.intersects(&touching));
        assert!(!a.intersects(&apart));
    }

    #[test]
    fn quad_containment_and_intersection() {
        let q = Quad::new([0.0, 0.0], [4.0, 3.0]);
        assert!(q.is_valid());
        assert!(q.contains([4.0, 3.0]));
        assert!(!q.contains([4.0, 3.5]));
        assert!(q.intersects(&Quad::new([4.0, 0.0], [5.0, 1.0])));
        assert!(!q.intersects(&Quad::new([4.5, 0.0], [5.0, 1.0])));
        assert!(!Quad::new([1.0, 0.0], [0.0, 1.0]).is_valid());
    }

    #[test]
    fn triangle_area_sign_follows_winding() {
        let ccw = Triangle::new([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]]);
        let cw = Triangle::new([[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [2.0, 0.0, 0.0]]);
        let flat = Triangle::new([[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0, 0.0]]);
        assert_eq!(ccw.signed_area2_xy(), 4.0);
        assert_eq!(cw.signed_area2_xy(), -4.0);
        assert_eq!(flat.signed_area2_xy(), 0.0);
        assert_eq!(ccw.normal(), [0.0, 0.0, 4.0]);
    }

    #[test]
    fn triangle_area_is_measured_in_three_dimensions() {
        let flat = Triangle::new([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]]);
        let upright = Triangle::new([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]]);
        assert_eq!(flat.area(), 2.0);
        assert_eq!(upright.area(), 2.0);
        // An upright triangle has no area when seen from above.
        assert_eq!(upright.signed_area2_xy(), 0.0);
    }

    #[test]
    fn triangle_center_and_containment_work_for_both_windings() {
        let ccw = Triangle::new([[0.0, 0.0, 0.0], [3.0, 0.0, 3.0], [0.0, 3.0, 0.0]]);
        let cw = Triangle::new([[0.0, 0.0, 0.0], [0.0, 3.0, 0.0], [3.0, 0.0, 3.0]]);
        assert_eq!(ccw.center(), [1.0, 1.0, 1.0]);
        for t in [ccw, cw] {
            assert!(t.contains_xy([1.0, 1.0]));
            assert!(t.contains_xy([0.0, 0.0]));
            assert!(t.contains_xy([1.5, 1.5]));
            assert!(!t.contains_xy([2.0, 2.0]));
            assert!(!t.contains_xy([-0.1, 1.0]));
        }
    }

    #[test]
    fn upright_triangle_contains_no_ground_position() {
        let upright = Triangle::new([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]]);
        assert!(!upright.contains_xy([1.0, 0.0]));
    }

    #[test]
    fn option_sets_are_distinct_variants() {
        assert_ne!(TriangulationMode::Vertices, TriangulationMode::Centers);
        assert_ne!(TriangulationMode::Centers, TriangulationMode::EdgeMidpoints);
        assert_ne!(SplitStrategy::Halve, SplitStrategy::Linear);
        assert_ne!(SplitStrategy::Quadratic, SplitStrategy::ReduceOverlap);
    }
}
