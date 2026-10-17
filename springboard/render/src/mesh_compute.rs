//! Geometry processing kernels for 3D scene editing (the compute-shader jobs of a USD authoring tool), as pure
//! functions over plain arrays (std only): per-face normals, per-vertex tangents from texture coordinates, vertex
//! welding, vertex-clustering decimation, Laplacian smoothing with fixed boundaries and a bounding sphere.
//!
//! Positions are `[f32; 3]`, texture coordinates `[f32; 2]` and triangle lists are flat `u32` index arrays with
//! three entries per triangle. Smooth vertex normals and one-level midpoint subdivision live in
//! `bifrost-ui` (`mesh_process`); skinning and morph targets in `morph_skin`. Every function is the CPU reference
//! for a one-thread-per-vertex or one-thread-per-triangle GPU kernel.

use std::collections::HashMap;

/// A 3-vector.
pub type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: V3) -> V3 {
    let l = len(a);
    if l < 1e-20 {
        [0.0; 3]
    } else {
        scale(a, 1.0 / l)
    }
}

/// True when every index is in range and the count is a multiple of three.
fn valid(vertex_count: usize, indices: &[u32]) -> bool {
    indices.len() % 3 == 0 && indices.iter().all(|&i| (i as usize) < vertex_count)
}

/// One unit normal per triangle (counter-clockwise winding faces the viewer). A degenerate triangle gets the zero
/// vector. `None` when an index is out of range or the index count is not a multiple of three.
pub fn face_normals(positions: &[V3], indices: &[u32]) -> Option<Vec<V3>> {
    if !valid(positions.len(), indices) {
        return None;
    }
    Some(
        indices
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
                normalize(cross(sub(b, a), sub(c, a)))
            })
            .collect(),
    )
}

/// Per-vertex tangents `[x, y, z, w]` for normal mapping. The tangent points along increasing `u`, is made
/// perpendicular to the vertex normal, and `w` is -1 where the texture is mirrored (the bitangent then points
/// against `cross(normal, tangent)`) and +1 otherwise. A vertex with no usable texture mapping gets an arbitrary
/// tangent perpendicular to its normal. `None` when the array lengths disagree or an index is out of range.
pub fn compute_tangents(positions: &[V3], uvs: &[[f32; 2]], normals: &[V3], indices: &[u32]) -> Option<Vec<[f32; 4]>> {
    let n = positions.len();
    if uvs.len() != n || normals.len() != n || !valid(n, indices) {
        return None;
    }
    let mut tan = vec![[0.0f32; 3]; n];
    let mut bit = vec![[0.0f32; 3]; n];
    for t in indices.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let e1 = sub(positions[i1], positions[i0]);
        let e2 = sub(positions[i2], positions[i0]);
        let (du1, dv1) = (uvs[i1][0] - uvs[i0][0], uvs[i1][1] - uvs[i0][1]);
        let (du2, dv2) = (uvs[i2][0] - uvs[i0][0], uvs[i2][1] - uvs[i0][1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let tangent = scale(sub(scale(e1, dv2), scale(e2, dv1)), r);
        let bitangent = scale(sub(scale(e2, du1), scale(e1, du2)), r);
        for &i in &[i0, i1, i2] {
            tan[i] = add(tan[i], tangent);
            bit[i] = add(bit[i], bitangent);
        }
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let nrm = normalize(normals[i]);
        let mut t = normalize(sub(tan[i], scale(nrm, dot(nrm, tan[i]))));
        if len(t) == 0.0 {
            // Any vector perpendicular to the normal.
            let axis = if nrm[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            t = normalize(cross(nrm, axis));
        }
        let w = if dot(cross(nrm, t), bit[i]) < 0.0 { -1.0 } else { 1.0 };
        out.push([t[0], t[1], t[2], w]);
    }
    Some(out)
}

/// Merges vertices closer than `epsilon` on a grid of that size and drops triangles that collapse to a line or a
/// point. Returns the new positions (first occurrence of each merged vertex wins) and indices. `None` for invalid
/// input or a non-positive epsilon.
pub fn weld_vertices(positions: &[V3], indices: &[u32], epsilon: f32) -> Option<(Vec<V3>, Vec<u32>)> {
    if !valid(positions.len(), indices) || !(epsilon > 0.0) {
        return None;
    }
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut out_pos: Vec<V3> = Vec::new();
    let mut remap: Vec<u32> = Vec::with_capacity(positions.len());
    for p in positions {
        let key = ((p[0] / epsilon).round() as i64, (p[1] / epsilon).round() as i64, (p[2] / epsilon).round() as i64);
        let id = *map.entry(key).or_insert_with(|| {
            out_pos.push(*p);
            (out_pos.len() - 1) as u32
        });
        remap.push(id);
    }
    let mut out_idx = Vec::with_capacity(indices.len());
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a != b && b != c && a != c {
            out_idx.extend_from_slice(&[a, b, c]);
        }
    }
    Some((out_pos, out_idx))
}

/// Vertex-clustering decimation (Rossignac and Low): vertices in the same cubic cell of side `cell` collapse to the
/// average of their positions, and triangles with two corners in one cell vanish. Cells are numbered in order of
/// first use, so the result is deterministic. `None` for invalid input or a non-positive cell size.
pub fn decimate_clusters(positions: &[V3], indices: &[u32], cell: f32) -> Option<(Vec<V3>, Vec<u32>)> {
    if !valid(positions.len(), indices) || !(cell > 0.0) {
        return None;
    }
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut sums: Vec<V3> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();
    let mut remap: Vec<u32> = Vec::with_capacity(positions.len());
    for p in positions {
        let key = ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64, (p[2] / cell).floor() as i64);
        let next = sums.len() as u32;
        let id = *map.entry(key).or_insert(next);
        if id == next {
            sums.push([0.0; 3]);
            counts.push(0);
        }
        sums[id as usize] = add(sums[id as usize], *p);
        counts[id as usize] += 1;
        remap.push(id);
    }
    let out_pos: Vec<V3> = sums.iter().zip(&counts).map(|(s, &c)| scale(*s, 1.0 / c as f32)).collect();
    let mut out_idx = Vec::new();
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a != b && b != c && a != c {
            out_idx.extend_from_slice(&[a, b, c]);
        }
    }
    Some((out_pos, out_idx))
}

/// Uniform Laplacian smoothing: each pass moves every interior vertex `lambda` of the way to the mean of its edge
/// neighbours. Vertices on a boundary edge (an edge of exactly one triangle) stay put, so an open surface keeps its
/// outline. `None` for invalid input or `lambda` outside 0..=1.
pub fn laplacian_smooth(positions: &[V3], indices: &[u32], iterations: usize, lambda: f32) -> Option<Vec<V3>> {
    if !valid(positions.len(), indices) || !(0.0..=1.0).contains(&lambda) {
        return None;
    }
    let n = positions.len();
    let mut edge_use: HashMap<(u32, u32), u32> = HashMap::new();
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in indices.chunks_exact(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            *edge_use.entry(key).or_insert(0) += 1;
            if !neighbours[a as usize].contains(&b) {
                neighbours[a as usize].push(b);
            }
            if !neighbours[b as usize].contains(&a) {
                neighbours[b as usize].push(a);
            }
        }
    }
    let mut fixed = vec![false; n];
    for (&(a, b), &c) in &edge_use {
        if c == 1 {
            fixed[a as usize] = true;
            fixed[b as usize] = true;
        }
    }
    let mut cur = positions.to_vec();
    for _ in 0..iterations {
        let mut next = cur.clone();
        for i in 0..n {
            if fixed[i] || neighbours[i].is_empty() {
                continue;
            }
            let mut mean = [0.0f32; 3];
            for &j in &neighbours[i] {
                mean = add(mean, cur[j as usize]);
            }
            mean = scale(mean, 1.0 / neighbours[i].len() as f32);
            next[i] = add(cur[i], scale(sub(mean, cur[i]), lambda));
        }
        cur = next;
    }
    Some(cur)
}

/// A sphere containing every point, by Ritter's method (within a few percent of the minimum). `None` for no points.
pub fn bounding_sphere(points: &[V3]) -> Option<(V3, f32)> {
    let first = *points.first()?;
    let far = |from: V3| -> V3 {
        let mut best = from;
        let mut best_d = -1.0f32;
        for p in points {
            let d = dot(sub(*p, from), sub(*p, from));
            if d > best_d {
                best_d = d;
                best = *p;
            }
        }
        best
    };
    let y = far(first);
    let z = far(y);
    let mut center = scale(add(y, z), 0.5);
    let mut radius = len(sub(z, y)) * 0.5;
    for p in points {
        let d = len(sub(*p, center));
        if d > radius {
            let new_radius = (radius + d) * 0.5;
            let k = (new_radius - radius) / d;
            center = add(center, scale(sub(*p, center), k));
            radius = new_radius;
        }
    }
    // Guard against rounding: grow until every point is inside.
    let slack = points.iter().map(|p| len(sub(*p, center))).fold(0.0f32, f32::max);
    Some((center, radius.max(slack)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize, step: f32) -> (Vec<V3>, Vec<u32>) {
        let mut pos = Vec::new();
        for y in 0..n {
            for x in 0..n {
                pos.push([x as f32 * step, y as f32 * step, 0.0]);
            }
        }
        let mut idx = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let a = (y * n + x) as u32;
                let b = a + 1;
                let c = a + n as u32;
                let d = c + 1;
                idx.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        (pos, idx)
    }

    #[test]
    fn face_normals_follow_winding() {
        let (pos, idx) = grid(2, 1.0);
        let n = face_normals(&pos, &idx).unwrap();
        assert_eq!(n.len(), 2);
        for v in n {
            assert!((v[2] - 1.0).abs() < 1e-6);
        }
        assert!(face_normals(&pos, &[0, 1]).is_none());
        assert!(face_normals(&pos, &[0, 1, 9]).is_none());
        assert_eq!(face_normals(&[[0.0; 3]; 3], &[0, 1, 2]).unwrap()[0], [0.0; 3]);
    }

    #[test]
    fn tangent_follows_u_and_sign_flags_mirroring() {
        let (pos, idx) = grid(2, 1.0);
        let normals = vec![[0.0, 0.0, 1.0]; 4];
        // u along +x, v along +y.
        let uv = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let t = compute_tangents(&pos, &uv, &normals, &idx).unwrap();
        for v in &t {
            assert!((v[0] - 1.0).abs() < 1e-5 && v[1].abs() < 1e-5 && v[2].abs() < 1e-5);
            assert_eq!(v[3], 1.0);
        }
        // v flipped: the bitangent now points against cross(n, t).
        let uv = vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]];
        let t = compute_tangents(&pos, &uv, &normals, &idx).unwrap();
        for v in &t {
            assert!((v[0] - 1.0).abs() < 1e-5);
            assert_eq!(v[3], -1.0);
        }
        assert!(compute_tangents(&pos, &uv[..3], &normals, &idx).is_none());
    }

    #[test]
    fn unmapped_vertices_get_a_perpendicular_tangent() {
        let (pos, idx) = grid(2, 1.0);
        let normals = vec![[0.0, 0.0, 1.0]; 4];
        let uv = vec![[0.5, 0.5]; 4];
        let t = compute_tangents(&pos, &uv, &normals, &idx).unwrap();
        for v in &t {
            assert!(v[2].abs() < 1e-6);
            assert!((v[0] * v[0] + v[1] * v[1] - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn welding_merges_duplicates_and_drops_slivers() {
        let pos = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0001]];
        let idx = vec![0, 1, 2, 3, 4, 5, 2, 5, 6];
        let (p, i) = weld_vertices(&pos, &idx, 0.01).unwrap();
        assert_eq!(p.len(), 4);
        // The third triangle collapses once 2, 5 and 6 are one vertex.
        assert_eq!(i.len(), 6);
        assert!(i.iter().all(|&v| (v as usize) < p.len()));
        assert!(weld_vertices(&pos, &idx, 0.0).is_none());
    }

    #[test]
    fn clustering_reduces_triangles_and_keeps_indices_valid() {
        let (pos, idx) = grid(9, 0.125);
        let (p, i) = decimate_clusters(&pos, &idx, 0.5).unwrap();
        assert_eq!(p.len(), 9);
        assert!(!i.is_empty() && i.len() < idx.len());
        assert!(i.iter().all(|&v| (v as usize) < p.len()));
        assert!(decimate_clusters(&pos, &idx, -1.0).is_none());
        // A cell larger than the mesh collapses everything.
        let (p, i) = decimate_clusters(&pos, &idx, 10.0).unwrap();
        assert_eq!(p.len(), 1);
        assert!(i.is_empty());
    }

    #[test]
    fn smoothing_flattens_a_bump_and_keeps_the_outline() {
        let (mut pos, idx) = grid(3, 1.0);
        pos[4][2] = 1.0;
        let s = laplacian_smooth(&pos, &idx, 1, 1.0).unwrap();
        assert!(s[4][2].abs() < 1e-6);
        for i in [0usize, 1, 2, 3, 5, 6, 7, 8] {
            assert_eq!(s[i], pos[i]);
        }
        let half = laplacian_smooth(&pos, &idx, 1, 0.5).unwrap();
        assert!((half[4][2] - 0.5).abs() < 1e-6);
        assert!(laplacian_smooth(&pos, &idx, 1, 2.0).is_none());
    }

    #[test]
    fn bounding_sphere_contains_all_points() {
        let pts = vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, -0.5], [0.3, 0.3, 0.3]];
        let (c, r) = bounding_sphere(&pts).unwrap();
        for p in &pts {
            assert!(len(sub(*p, c)) <= r + 1e-5);
        }
        let (c, r) = bounding_sphere(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]).unwrap();
        assert!((r - 1.0).abs() < 1e-6 && (c[0] - 1.0).abs() < 1e-6);
        assert!(bounding_sphere(&[]).is_none());
    }
}
