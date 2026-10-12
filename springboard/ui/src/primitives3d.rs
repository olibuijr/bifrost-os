//! Built-in 3D shapes as plain vertex data: a cuboid, a UV sphere and a flat plane.
//! Each shape returns a `Geometry3` with one position and one unit normal per vertex, and a triangle list.
//! Triangle winding is not enforced. Every builder returns None for a degenerate size, so no shape is empty.

use crate::linalg::Vector3;
use std::f32::consts::PI;

/// Vertex data for one shape: positions and normals in parallel, and `u32` indices in triples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry3 {
    pub positions: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub indices: Vec<u32>,
}

impl Geometry3 {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The min and max corners of the vertex positions, or None when there are no vertices.
    pub fn bounds(&self) -> Option<(Vector3, Vector3)> {
        let first = *self.positions.first()?;
        let bounds = self.positions.iter().fold((first, first), |(lo, hi), p| {
            (
                Vector3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                Vector3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
            )
        });
        Some(bounds)
    }
}

/// A box centred on the origin with the given full extents along X, Y and Z. Each face has four
/// vertices with its own normal, so there are 24 vertices and 12 triangles.
pub fn cuboid_geometry(width: f32, height: f32, length: f32) -> Option<Geometry3> {
    if width <= 0.0 || height <= 0.0 || length <= 0.0 {
        return None;
    }
    let h = [width * 0.5, height * 0.5, length * 0.5];
    let mut g = Geometry3::default();
    for k in 0..3 {
        let i = (k + 1) % 3;
        let j = (k + 2) % 3;
        for &sign in &[-1.0f32, 1.0] {
            let base = g.positions.len() as u32;
            let mut n = [0.0f32; 3];
            n[k] = sign;
            for &(s1, s2) in &[(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut p = [0.0f32; 3];
                p[k] = sign * h[k];
                p[i] = s1 * h[i];
                p[j] = s2 * h[j];
                g.positions.push(Vector3::new(p[0], p[1], p[2]));
                g.normals.push(Vector3::new(n[0], n[1], n[2]));
            }
            g.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    Some(g)
}

/// A UV sphere centred on the origin, with `segments` slices around the Y axis and `rings` bands from
/// pole to pole. Needs at least 3 segments, 2 rings and a positive radius. The pole rows are kept as
/// degenerate triangles so the index layout stays regular. Vertices number `(segments + 1) * (rings + 1)`
/// and triangles `segments * rings * 2`.
pub fn sphere_geometry(radius: f32, segments: usize, rings: usize) -> Option<Geometry3> {
    if radius <= 0.0 || segments < 3 || rings < 2 {
        return None;
    }
    let mut g = Geometry3::default();
    for r in 0..=rings {
        let theta = PI * r as f32 / rings as f32;
        let (st, ct) = theta.sin_cos();
        for s in 0..=segments {
            let phi = 2.0 * PI * s as f32 / segments as f32;
            let n = Vector3::new(st * phi.cos(), ct, st * phi.sin());
            g.positions.push(n * radius);
            g.normals.push(n);
        }
    }
    let row = (segments + 1) as u32;
    for r in 0..rings as u32 {
        for s in 0..segments as u32 {
            let a = r * row + s;
            let b = a + row;
            g.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Some(g)
}

/// A flat rectangle in the XZ plane at y = 0, facing +Y, with the given width (X) and length (Z).
pub fn plane_geometry(width: f32, length: f32) -> Option<Geometry3> {
    if width <= 0.0 || length <= 0.0 {
        return None;
    }
    let (hw, hl) = (width * 0.5, length * 0.5);
    let positions = vec![
        Vector3::new(-hw, 0.0, -hl),
        Vector3::new(hw, 0.0, -hl),
        Vector3::new(hw, 0.0, hl),
        Vector3::new(-hw, 0.0, hl),
    ];
    Some(Geometry3 {
        positions,
        normals: vec![Vector3::new(0.0, 1.0, 0.0); 4],
        indices: vec![0, 1, 2, 0, 2, 3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_vec(actual: Vector3, x: f32, y: f32, z: f32) {
        assert!(
            close(actual.x, x) && close(actual.y, y) && close(actual.z, z),
            "expected ({x}, {y}, {z}), got {actual:?}"
        );
    }

    #[test]
    fn cuboid_has_24_vertices_and_12_triangles() {
        let g = cuboid_geometry(2.0, 4.0, 6.0).unwrap();
        assert_eq!(g.vertex_count(), 24);
        assert_eq!(g.indices.len(), 36);
        assert_eq!(g.triangle_count(), 12);
        assert_eq!(g.normals.len(), 24);
    }

    #[test]
    fn cuboid_bounds_match_its_extents() {
        let g = cuboid_geometry(2.0, 4.0, 6.0).unwrap();
        let (lo, hi) = g.bounds().unwrap();
        assert_vec(lo, -1.0, -2.0, -3.0);
        assert_vec(hi, 1.0, 2.0, 3.0);
    }

    #[test]
    fn cuboid_faces_carry_their_axis_normals() {
        let g = cuboid_geometry(1.0, 1.0, 1.0).unwrap();
        // The first face is at -X; the last face is at +Z.
        assert_vec(g.normals[0], -1.0, 0.0, 0.0);
        assert_vec(g.normals[23], 0.0, 0.0, 1.0);
        assert!(cuboid_geometry(0.0, 1.0, 1.0).is_none());
    }

    #[test]
    fn sphere_counts_follow_segments_and_rings() {
        let g = sphere_geometry(1.0, 8, 4).unwrap();
        assert_eq!(g.vertex_count(), 45);
        assert_eq!(g.indices.len(), 4 * 8 * 6);
        assert_eq!(g.triangle_count(), 64);
    }

    #[test]
    fn sphere_poles_and_equator_lie_on_the_radius() {
        let g = sphere_geometry(2.0, 8, 4).unwrap();
        // Index 0 is the +Y pole; index 2 * 9 is the equator point at azimuth 0.
        assert_vec(g.positions[0], 0.0, 2.0, 0.0);
        assert_vec(g.positions[18], 2.0, 0.0, 0.0);
        assert_vec(g.normals[18], 1.0, 0.0, 0.0);
    }

    #[test]
    fn sphere_rejects_degenerate_settings() {
        assert!(sphere_geometry(1.0, 2, 4).is_none());
        assert!(sphere_geometry(1.0, 8, 1).is_none());
        assert!(sphere_geometry(0.0, 8, 4).is_none());
    }

    #[test]
    fn plane_is_a_flat_quad_facing_up() {
        let g = plane_geometry(4.0, 2.0).unwrap();
        assert_eq!(g.vertex_count(), 4);
        assert_eq!(g.triangle_count(), 2);
        assert!(g.normals.iter().all(|n| *n == Vector3::new(0.0, 1.0, 0.0)));
        let (lo, hi) = g.bounds().unwrap();
        assert_vec(lo, -2.0, 0.0, -1.0);
        assert_vec(hi, 2.0, 0.0, 1.0);
        assert!(plane_geometry(0.0, 2.0).is_none());
    }
}
