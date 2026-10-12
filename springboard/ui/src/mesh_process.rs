//! Mesh processing on `PrimitiveMesh` data: vertex normals from the triangles and one level of midpoint subdivision.
//! Both are pure functions over plain vectors. Neither bakes lighting, places light probes or builds normal maps.

use std::collections::HashMap;

use crate::linalg::Vector3;
use crate::mesh_primitives::PrimitiveMesh;

/// Area-weighted vertex normals, one per position. Each vertex sums the cross products of its triangles' edges, so a
/// triangle counts in proportion to its area, and the sum is normalised. A vertex used by no triangle gets zero.
/// Triangles that refer to a missing position are skipped.
pub fn smooth_normals(mesh: &PrimitiveMesh) -> Vec<Vector3> {
    let n = mesh.positions.len();
    let p = &mesh.positions;
    let mut sums = vec![Vector3::ZERO; n];
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if a >= n || b >= n || c >= n {
            continue;
        }
        let face = (p[b] - p[a]).cross(p[c] - p[a]);
        sums[a] = sums[a] + face;
        sums[b] = sums[b] + face;
        sums[c] = sums[c] + face;
    }
    sums.into_iter().map(|s| s.normalized()).collect()
}

/// Splits every triangle into four at its edge midpoints (one level). A new vertex sits at the midpoint of its edge, and
/// its normal and texture coordinates are the midpoints of the edge's ends, with the normal renormalised. Each shared
/// edge gets one new vertex, so neighbouring triangles stay welded. Triangle winding is kept.
///
/// Returns `None` when the arrays differ in length, the index count is not a multiple of three, or an index is out of
/// range. A mesh built by `mesh_primitives` always has equal-length arrays.
pub fn subdivide_midpoints(mesh: &PrimitiveMesh) -> Option<PrimitiveMesh> {
    let n = mesh.positions.len();
    if mesh.normals.len() != n || mesh.texture_coordinates.len() != n || mesh.indices.len() % 3 != 0 {
        return None;
    }
    if mesh.indices.iter().any(|&i| i as usize >= n) {
        return None;
    }
    let mut out = PrimitiveMesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        texture_coordinates: mesh.texture_coordinates.clone(),
        indices: Vec::with_capacity(mesh.indices.len() * 4),
    };
    let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let ab = edge_midpoint(&mut out, &mut cache, a, b);
        let bc = edge_midpoint(&mut out, &mut cache, b, c);
        let ca = edge_midpoint(&mut out, &mut cache, c, a);
        out.indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }
    Some(out)
}

/// Index of the vertex at the midpoint of edge `a`-`b`, appending it to `out` the first time the edge is seen.
/// The original vertices are the first entries of `out`, so their values are read from there.
fn edge_midpoint(out: &mut PrimitiveMesh, cache: &mut HashMap<(u32, u32), u32>, a: u32, b: u32) -> u32 {
    let key = if a < b { (a, b) } else { (b, a) };
    if let Some(&id) = cache.get(&key) {
        return id;
    }
    let (ia, ib) = (a as usize, b as usize);
    let id = out.positions.len() as u32;
    let position = (out.positions[ia] + out.positions[ib]) * 0.5;
    let normal = ((out.normals[ia] + out.normals[ib]) * 0.5).normalized();
    let (ta, tb) = (out.texture_coordinates[ia], out.texture_coordinates[ib]);
    out.positions.push(position);
    out.normals.push(normal);
    out.texture_coordinates.push([(ta[0] + tb[0]) * 0.5, (ta[1] + tb[1]) * 0.5]);
    cache.insert(key, id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn mesh(positions: Vec<Vector3>, indices: Vec<u32>) -> PrimitiveMesh {
        let n = positions.len();
        PrimitiveMesh {
            positions,
            normals: vec![Vector3::new(0.0, 0.0, 1.0); n],
            texture_coordinates: vec![[0.0, 0.0]; n],
            indices,
        }
    }

    #[test]
    fn unit_square_has_upward_normals() {
        let m = mesh(
            vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(1.0, 1.0, 0.0),
                Vector3::new(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );
        let normals = smooth_normals(&m);
        assert_eq!(normals.len(), 4);
        for n in normals {
            assert!(close(n.x, 0.0) && close(n.y, 0.0) && close(n.z, 1.0), "{:?}", n);
        }
    }

    #[test]
    fn normals_are_weighted_by_face_area() {
        // Triangle 0-1-2 lies in z = 0 with normal +z and cross length 1. Triangle 0-3-4 lies in x = 0 with normal +x
        // and cross length 3. Vertex 0 sums (0,0,1) and (3,0,0) to (3,0,1), so its normal is (3, 0, 1) / sqrt(10).
        let m = mesh(
            vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(0.0, 3.0, 0.0),
                Vector3::new(0.0, 0.0, 1.0),
            ],
            vec![0, 1, 2, 0, 3, 4],
        );
        let s = 10.0f32.sqrt();
        let n0 = smooth_normals(&m)[0];
        assert!(close(n0.x, 3.0 / s) && close(n0.y, 0.0) && close(n0.z, 1.0 / s), "{:?}", n0);
        // Vertex 1 belongs to one triangle only, so it keeps that triangle's normal.
        let n1 = smooth_normals(&m)[1];
        assert!(close(n1.z, 1.0) && close(n1.x, 0.0), "{:?}", n1);
    }

    #[test]
    fn unused_vertex_gets_zero_normal() {
        let m = mesh(
            vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(9.0, 9.0, 9.0),
            ],
            vec![0, 1, 2],
        );
        assert_eq!(smooth_normals(&m)[3], Vector3::ZERO);
    }

    #[test]
    fn one_level_makes_four_triangles_per_triangle() {
        // Right triangle with legs 2: each of the four children has legs 1, so area 0.5, and they sum to the parent area 2.
        let m = PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(2.0, 0.0, 0.0), Vector3::new(0.0, 2.0, 0.0)],
            normals: vec![Vector3::new(0.0, 0.0, 1.0); 3],
            texture_coordinates: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        };
        let out = subdivide_midpoints(&m).unwrap();
        assert_eq!(out.positions.len(), 6);
        assert_eq!(out.indices.len(), 12);
        assert_eq!(out.positions[3], Vector3::new(1.0, 0.0, 0.0));
        assert_eq!(out.positions[4], Vector3::new(1.0, 1.0, 0.0));
        assert_eq!(out.positions[5], Vector3::new(0.0, 1.0, 0.0));
        let mut area = 0.0;
        for t in out.indices.chunks_exact(3) {
            let p = |i: u32| out.positions[i as usize];
            area += (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length() * 0.5;
        }
        assert!(close(area, 2.0), "{}", area);
    }

    #[test]
    fn midpoint_texture_coordinates_and_normals_are_averaged() {
        let m = PrimitiveMesh {
            positions: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(2.0, 0.0, 0.0), Vector3::new(0.0, 2.0, 0.0)],
            normals: vec![Vector3::new(1.0, 0.0, 0.0), Vector3::new(0.0, 1.0, 0.0), Vector3::new(0.0, 0.0, 1.0)],
            texture_coordinates: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        };
        let out = subdivide_midpoints(&m).unwrap();
        // Midpoint of edge 0-1 has uv (0.5, 0) and normal (1,1,0) / sqrt(2), renormalised.
        assert_eq!(out.texture_coordinates[3], [0.5, 0.0]);
        let r = 0.5f32.sqrt();
        let n = out.normals[3];
        assert!(close(n.x, r) && close(n.y, r) && close(n.z, 0.0), "{:?}", n);
    }

    #[test]
    fn subdivision_rejects_bad_input() {
        let good = PrimitiveMesh {
            positions: vec![Vector3::ZERO; 3],
            normals: vec![Vector3::new(0.0, 0.0, 1.0); 3],
            texture_coordinates: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        };
        let mut short_normals = good.clone();
        short_normals.normals.pop();
        assert!(subdivide_midpoints(&short_normals).is_none());
        let mut out_of_range = good.clone();
        out_of_range.indices = vec![0, 1, 3];
        assert!(subdivide_midpoints(&out_of_range).is_none());
        let mut partial = good.clone();
        partial.indices = vec![0, 1];
        assert!(subdivide_midpoints(&partial).is_none());
        assert!(subdivide_midpoints(&good).is_some());
    }
}
