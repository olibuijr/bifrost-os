//! Ambient occlusion baking per vertex. For each vertex, a fixed set of directions covers the hemisphere above its
//! normal. A direction counts as open when the ray from the vertex reaches no triangle within `max_distance`. The
//! baked value is the open share: 1.0 for a vertex with nothing above it, 0.0 for one fully enclosed. The directions
//! are spread evenly by area (uniform in the height above the surface) along a golden-angle spiral, so the bake is
//! deterministic. The cost is vertices times samples times triangles: suited to small meshes and previews.
//! Light probes and lightmap packing are not modelled here.

use crate::linalg::Vector3;
use crate::mesh_primitives::PrimitiveMesh;
use crate::mesh_process::smooth_normals;

/// The golden angle in radians, which spreads successive spiral directions evenly around the normal.
const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// Bakes one occlusion value per vertex. The mesh normals are used when there is one per vertex; otherwise normals
/// are computed from the triangles. Returns `None` for zero samples, a non-positive or non-finite distance, a
/// triangle index list that is not a whole number of triangles, or an index past the vertex list.
pub fn bake_vertex_occlusion(mesh: &PrimitiveMesh, samples: usize, max_distance: f32) -> Option<Vec<f32>> {
    if samples == 0 || !(max_distance.is_finite() && max_distance > 0.0) {
        return None;
    }
    if mesh.indices.len() % 3 != 0 || mesh.indices.iter().any(|&i| i as usize >= mesh.positions.len()) {
        return None;
    }
    let normals = if mesh.normals.len() == mesh.positions.len() { mesh.normals.clone() } else { smooth_normals(mesh) };
    let triangles: Vec<[Vector3; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| [mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]])
        .collect();
    let offset = max_distance * 1e-4;
    let mut baked = Vec::with_capacity(mesh.positions.len());
    for (position, normal) in mesh.positions.iter().zip(&normals) {
        let n = normal.normalized();
        if n.length() == 0.0 {
            baked.push(1.0);
            continue;
        }
        let (t, b) = tangent_frame(n);
        let origin = *position + n * offset;
        let mut open = 0usize;
        for k in 0..samples {
            let dir = hemisphere_direction(k, samples, n, t, b);
            if !triangles.iter().any(|tri| hits(origin, dir, tri, max_distance)) {
                open += 1;
            }
        }
        baked.push(open as f32 / samples as f32);
    }
    Some(baked)
}

/// Two unit vectors that, with `n`, form a right-handed frame.
fn tangent_frame(n: Vector3) -> (Vector3, Vector3) {
    let helper = if n.x.abs() > 0.9 { Vector3::new(0.0, 1.0, 0.0) } else { Vector3::new(1.0, 0.0, 0.0) };
    let t = helper.cross(n).normalized();
    let b = n.cross(t);
    (t, b)
}

/// Direction `k` of `samples` over the hemisphere around `n`. The height is `(k + 0.5) / samples`, which gives equal
/// solid angle per sample, and the azimuth advances by the golden angle.
fn hemisphere_direction(k: usize, samples: usize, n: Vector3, t: Vector3, b: Vector3) -> Vector3 {
    let z = (k as f32 + 0.5) / samples as f32;
    let radius = (1.0 - z * z).max(0.0).sqrt();
    let phi = k as f32 * GOLDEN_ANGLE;
    t * (radius * phi.cos()) + b * (radius * phi.sin()) + n * z
}

/// Whether the ray from `origin` along `dir` (unit length) hits `tri` at a distance in `(0, max_t]`. This is the
/// Moller-Trumbore test.
fn hits(origin: Vector3, dir: Vector3, tri: &[Vector3; 3], max_t: f32) -> bool {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return false;
    }
    let inv = 1.0 / det;
    let s = origin - tri[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = e2.dot(q) * inv;
    t > 0.0 && t <= max_t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(positions: Vec<Vector3>, normals: Vec<Vector3>, indices: Vec<u32>) -> PrimitiveMesh {
        PrimitiveMesh { positions, normals, texture_coordinates: Vec::new(), indices }
    }

    fn up() -> Vector3 {
        Vector3::new(0.0, 1.0, 0.0)
    }

    /// A small floor at y = 0 (half size 1) and, when `ceiling` is true, a large ceiling at y = 0.5 (half size 10).
    fn floor_with_ceiling(ceiling: bool) -> PrimitiveMesh {
        let mut positions = vec![
            Vector3::new(-1.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, -1.0),
            Vector3::new(1.0, 0.0, 1.0),
            Vector3::new(-1.0, 0.0, 1.0),
        ];
        let mut indices = vec![0, 2, 1, 0, 3, 2];
        if ceiling {
            let y = 0.5;
            positions.extend([
                Vector3::new(-10.0, y, -10.0),
                Vector3::new(10.0, y, -10.0),
                Vector3::new(10.0, y, 10.0),
                Vector3::new(-10.0, y, 10.0),
            ]);
            indices.extend([4, 6, 5, 4, 7, 6]);
        }
        let normals = vec![up(); positions.len()];
        mesh(positions, normals, indices)
    }

    #[test]
    fn an_open_floor_is_fully_visible() {
        let baked = bake_vertex_occlusion(&floor_with_ceiling(false), 32, 1.0).expect("valid input");
        assert_eq!(baked, vec![1.0; 4]);
    }

    #[test]
    fn a_ceiling_in_reach_blocks_the_steep_half_of_the_hemisphere() {
        // With 64 samples, exactly the 32 directions with a height of at least 0.5 hit the ceiling, which is within
        // reach (the ray is at most 1 unit long). The corner vertex then sees half the hemisphere.
        let baked = bake_vertex_occlusion(&floor_with_ceiling(true), 64, 1.0).expect("valid input");
        assert!((baked[0] - 0.5).abs() < 0.02, "got {}", baked[0]);
    }

    #[test]
    fn a_ceiling_out_of_reach_blocks_nothing() {
        let baked = bake_vertex_occlusion(&floor_with_ceiling(true), 64, 0.2).expect("valid input");
        assert_eq!(baked[0], 1.0);
    }

    #[test]
    fn normals_are_computed_when_the_mesh_has_none() {
        let mut flat = floor_with_ceiling(false);
        flat.normals.clear();
        let baked = bake_vertex_occlusion(&flat, 16, 1.0).expect("valid input");
        assert_eq!(baked, vec![1.0; 4]);
    }

    #[test]
    fn bad_input_gives_no_bake() {
        let m = floor_with_ceiling(false);
        assert!(bake_vertex_occlusion(&m, 0, 1.0).is_none());
        assert!(bake_vertex_occlusion(&m, 8, 0.0).is_none());
        assert!(bake_vertex_occlusion(&m, 8, f32::NAN).is_none());
        let mut torn = m.clone();
        torn.indices.pop();
        assert!(bake_vertex_occlusion(&torn, 8, 1.0).is_none());
        let mut bad = m.clone();
        bad.indices[0] = 99;
        assert!(bake_vertex_occlusion(&bad, 8, 1.0).is_none());
    }

    #[test]
    fn the_hemisphere_directions_stay_above_the_surface() {
        let n = Vector3::new(0.3, 0.8, -0.5).normalized();
        let (t, b) = tangent_frame(n);
        for k in 0..40 {
            let d = hemisphere_direction(k, 40, n, t, b);
            assert!(d.dot(n) > 0.0);
            assert!((d.length() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn moller_trumbore_hits_only_within_the_distance() {
        let tri = [Vector3::new(-1.0, 2.0, -1.0), Vector3::new(1.0, 2.0, -1.0), Vector3::new(0.0, 2.0, 1.0)];
        assert!(hits(Vector3::ZERO, up(), &tri, 3.0));
        assert!(!hits(Vector3::ZERO, up(), &tri, 1.0));
        assert!(!hits(Vector3::ZERO, -up(), &tri, 3.0));
    }
}
