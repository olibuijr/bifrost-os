//! Built-in 3D shapes as vertex data: a box, a UV sphere and a plane (`NativeBoxGeometry`, `NativeSphereGeometry`,
//! `NativePlaneGeometry`). Each builder returns positions, normals, texture coordinates and triangle indices in plain
//! vectors. The layout is the one `mesh.rs` describes; this module owns no GPU memory.
//!
//! Winding: every triangle is counter-clockwise when seen from outside the shape, so its cross product points outward.
//! Degenerate triangles at the poles of a sphere are kept, so the index count is the same for any segment count.

use crate::linalg::Vector3;

/// Vertex data for one built-in shape. `indices` holds three entries per triangle and fits `IndexType::U32`.
#[derive(Clone, Debug, PartialEq)]
pub struct PrimitiveMesh {
    pub positions: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    pub texture_coordinates: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl PrimitiveMesh {
    fn empty() -> Self {
        PrimitiveMesh { positions: Vec::new(), normals: Vec::new(), texture_coordinates: Vec::new(), indices: Vec::new() }
    }

    /// Number of triangles (the index count divided by three).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Appends one rectangular grid of `(segments_u + 1) x (segments_v + 1)` vertices on the face through `center`.
    /// Along `u` and `v` the face spans `half_u` and `half_v` either side of `center`; `u x v` must equal `normal`.
    #[allow(clippy::too_many_arguments)]
    fn push_grid(
        &mut self,
        center: Vector3,
        normal: Vector3,
        u: Vector3,
        v: Vector3,
        half_u: f32,
        half_v: f32,
        segments_u: usize,
        segments_v: usize,
    ) {
        let base = self.positions.len() as u32;
        let (nu, nv) = (segments_u.max(1), segments_v.max(1));
        for i in 0..=nu {
            let s = -1.0 + 2.0 * i as f32 / nu as f32;
            for j in 0..=nv {
                let t = -1.0 + 2.0 * j as f32 / nv as f32;
                self.positions.push(center + u * (s * half_u) + v * (t * half_v));
                self.normals.push(normal);
                self.texture_coordinates.push([i as f32 / nu as f32, j as f32 / nv as f32]);
            }
        }
        let columns = (nv + 1) as u32;
        for i in 0..nu as u32 {
            for j in 0..nv as u32 {
                let a = base + i * columns + j;
                let b = a + columns;
                let c = b + 1;
                let d = a + 1;
                // a, b, c and a, c, d: both counter-clockwise from the normal side.
                self.indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
}

fn valid_extent(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

/// A rectangular box centred on the origin (`NativeBoxGeometry`). `width` runs along x, `height` along y and `length`
/// along z. Each face is subdivided into `*_segments` cells along its two axes (at least one each). Chamfered edges
/// are not built. Returns `None` for a negative or non-finite size.
pub fn box_mesh(
    width: f32,
    height: f32,
    length: f32,
    width_segments: usize,
    height_segments: usize,
    length_segments: usize,
) -> Option<PrimitiveMesh> {
    if !(valid_extent(width) && valid_extent(height) && valid_extent(length)) {
        return None;
    }
    let (hw, hh, hl) = (width * 0.5, height * 0.5, length * 0.5);
    let mut m = PrimitiveMesh::empty();
    let x = Vector3::new(1.0, 0.0, 0.0);
    let y = Vector3::new(0.0, 1.0, 0.0);
    let z = Vector3::new(0.0, 0.0, 1.0);
    // +X and -X: the faces span z (length) and y (height).
    m.push_grid(x * hw, x, -z, y, hl, hh, length_segments, height_segments);
    m.push_grid(x * -hw, -x, z, y, hl, hh, length_segments, height_segments);
    // +Y and -Y: the faces span x (width) and z (length).
    m.push_grid(y * hh, y, x, -z, hw, hl, width_segments, length_segments);
    m.push_grid(y * -hh, -y, x, z, hw, hl, width_segments, length_segments);
    // +Z and -Z: the faces span x (width) and y (height).
    m.push_grid(z * hl, z, x, y, hw, hh, width_segments, height_segments);
    m.push_grid(z * -hl, -z, -x, y, hw, hh, width_segments, height_segments);
    Some(m)
}

/// A rectangle in the x-y plane centred on the origin, facing +z (`NativePlaneGeometry`). Corner rounding is not built.
/// Returns `None` for a negative or non-finite size.
pub fn plane_mesh(width: f32, height: f32, width_segments: usize, height_segments: usize) -> Option<PrimitiveMesh> {
    if !(valid_extent(width) && valid_extent(height)) {
        return None;
    }
    let mut m = PrimitiveMesh::empty();
    m.push_grid(
        Vector3::ZERO,
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        width * 0.5,
        height * 0.5,
        width_segments,
        height_segments,
    );
    Some(m)
}

/// A sphere centred on the origin (`NativeSphereGeometry`), built as a latitude and longitude grid. `segments` is the
/// number of divisions around the equator (at least three); the number of rings from pole to pole is `segments / 2`
/// (at least two). Returns `None` for a negative or non-finite radius.
pub fn sphere_mesh(radius: f32, segments: usize) -> Option<PrimitiveMesh> {
    if !valid_extent(radius) {
        return None;
    }
    let longitude = segments.max(3);
    let latitude = (segments / 2).max(2);
    let mut m = PrimitiveMesh::empty();
    for i in 0..=latitude {
        let theta = std::f32::consts::PI * i as f32 / latitude as f32;
        let (st, ct) = theta.sin_cos();
        for j in 0..=longitude {
            let phi = 2.0 * std::f32::consts::PI * j as f32 / longitude as f32;
            let (sp, cp) = phi.sin_cos();
            let n = Vector3::new(st * cp, ct, st * sp);
            m.positions.push(n * radius);
            m.normals.push(n);
            m.texture_coordinates.push([j as f32 / longitude as f32, i as f32 / latitude as f32]);
        }
    }
    let columns = (longitude + 1) as u32;
    for i in 0..latitude as u32 {
        for j in 0..longitude as u32 {
            let a = i * columns + j;
            let b = a + 1;
            let c = a + columns;
            let d = c + 1;
            // Outward winding: (a, b, c) and (c, b, d).
            m.indices.extend_from_slice(&[a, b, c, c, b, d]);
        }
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn cross_of_triangle(m: &PrimitiveMesh, t: usize) -> Vector3 {
        let p0 = m.positions[m.indices[3 * t] as usize];
        let p1 = m.positions[m.indices[3 * t + 1] as usize];
        let p2 = m.positions[m.indices[3 * t + 2] as usize];
        (p1 - p0).cross(p2 - p0)
    }

    fn centroid_of_triangle(m: &PrimitiveMesh, t: usize) -> Vector3 {
        let p0 = m.positions[m.indices[3 * t] as usize];
        let p1 = m.positions[m.indices[3 * t + 1] as usize];
        let p2 = m.positions[m.indices[3 * t + 2] as usize];
        (p0 + p1 + p2) * (1.0 / 3.0)
    }

    #[test]
    fn plane_has_corners_at_half_extents_and_faces_plus_z() {
        let p = plane_mesh(2.0, 4.0, 1, 1).expect("valid size");
        assert_eq!(p.positions.len(), 4);
        assert_eq!(p.triangle_count(), 2);
        let max_x = p.positions.iter().map(|v| v.x).fold(f32::MIN, f32::max);
        let max_y = p.positions.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        assert!(close(max_x, 1.0) && close(max_y, 2.0), "x {max_x}, y {max_y}");
        assert!(p.normals.iter().all(|n| *n == Vector3::new(0.0, 0.0, 1.0)));
        assert!(cross_of_triangle(&p, 0).z > 0.0, "first triangle must face +z");
    }

    #[test]
    fn plane_segments_multiply_the_grid() {
        let p = plane_mesh(1.0, 1.0, 3, 2).expect("valid size");
        assert_eq!(p.positions.len(), 4 * 3);
        assert_eq!(p.triangle_count(), 3 * 2 * 2);
        assert_eq!(p.texture_coordinates[0], [0.0, 0.0]);
    }

    #[test]
    fn box_with_unit_segments_has_six_quad_faces() {
        let b = box_mesh(2.0, 4.0, 6.0, 1, 1, 1).expect("valid size");
        assert_eq!(b.positions.len(), 24);
        assert_eq!(b.triangle_count(), 12);
        let max_x = b.positions.iter().map(|v| v.x).fold(f32::MIN, f32::max);
        let max_y = b.positions.iter().map(|v| v.y).fold(f32::MIN, f32::max);
        let max_z = b.positions.iter().map(|v| v.z).fold(f32::MIN, f32::max);
        assert!(close(max_x, 1.0) && close(max_y, 2.0) && close(max_z, 3.0));
    }

    #[test]
    fn every_box_triangle_winds_outward() {
        let b = box_mesh(2.0, 4.0, 6.0, 2, 3, 4).expect("valid size");
        for t in 0..b.triangle_count() {
            let n = cross_of_triangle(&b, t);
            assert!(n.dot(centroid_of_triangle(&b, t)) > 0.0, "triangle {t} faces inward");
        }
    }

    #[test]
    fn box_segment_counts_set_vertex_and_index_totals() {
        // Faces: x pair 5x4 vertices (length 4, height 3), y pair 3x5 (width 2, length 4), z pair 3x4 (width 2, height 3).
        let b = box_mesh(1.0, 1.0, 1.0, 2, 3, 4).expect("valid size");
        assert_eq!(b.positions.len(), 2 * 20 + 2 * 15 + 2 * 12);
        assert_eq!(b.indices.len(), 6 * (2 * 12 + 2 * 8 + 2 * 6));
    }

    #[test]
    fn negative_or_non_finite_sizes_are_rejected() {
        assert!(box_mesh(-1.0, 1.0, 1.0, 1, 1, 1).is_none());
        assert!(plane_mesh(1.0, f32::NAN, 1, 1).is_none());
        assert!(sphere_mesh(-0.5, 8).is_none());
    }

    #[test]
    fn sphere_vertices_lie_on_the_radius_and_the_top_pole_is_up() {
        let s = sphere_mesh(2.0, 8).expect("valid radius");
        // 8 around and 4 rings: 5 x 9 vertices, 4 x 8 cells of two triangles each.
        assert_eq!(s.positions.len(), 5 * 9);
        assert_eq!(s.indices.len(), 4 * 8 * 6);
        assert!(s.positions.iter().all(|p| close(p.length(), 2.0)));
        assert!(close(s.positions[0].y, 2.0));
    }

    #[test]
    fn sphere_triangles_wind_outward_except_degenerate_pole_faces() {
        let s = sphere_mesh(1.0, 12).expect("valid radius");
        let mut checked = 0;
        for t in 0..s.triangle_count() {
            let n = cross_of_triangle(&s, t);
            if n.length() > 1e-6 {
                assert!(n.dot(centroid_of_triangle(&s, t)) > 0.0, "triangle {t} faces inward");
                checked += 1;
            }
        }
        assert!(checked > s.triangle_count() / 2);
    }
}
