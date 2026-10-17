//! Mesh vertex-stage kernels: what a vertex shader does for 3D content before rasterisation. 4x4 matrix
//! algebra (row-major, column vectors, `m[row][col]`), a Vulkan-style perspective projection (right
//! handed view looking down -Z, depth 0 at the near plane and 1 at the far plane), projection to
//! screen pixels, the normal matrix, linear-blend skinning of positions and normals from a joint palette,
//! morph-target (blend shape) accumulation, area-weighted smooth vertex normals and a tangent frame from
//! UV derivatives. The mesh layout types are in `bifrost-ui` `mesh`, the shading function is `pbr`, and
//! the triangle rasteriser is `raster3d`. The GPU twin of the skinning and morph step is
//! `springboard/gpu/shaders/skin_blend.comp`. Written from the standard formulations.

pub type Vec3 = [f32; 3];
pub type Mat4 = [[f32; 4]; 4];

/// The identity matrix.
pub fn identity() -> Mat4 {
    let mut m = [[0.0f32; 4]; 4];
    for i in 0..4 {
        m[i][i] = 1.0;
    }
    m
}

/// The product `a * b` (apply `b` first, then `a`).
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[r][k] * b[k][c];
            }
            out[r][c] = s;
        }
    }
    out
}

/// A translation.
pub fn translation(t: Vec3) -> Mat4 {
    let mut m = identity();
    m[0][3] = t[0];
    m[1][3] = t[1];
    m[2][3] = t[2];
    m
}

/// A non-uniform scale.
pub fn scaling(s: Vec3) -> Mat4 {
    let mut m = identity();
    m[0][0] = s[0];
    m[1][1] = s[1];
    m[2][2] = s[2];
    m
}

/// A rotation about the Z axis by `radians` (counter-clockwise looking down -Z).
pub fn rotation_z(radians: f32) -> Mat4 {
    let (s, c) = radians.sin_cos();
    let mut m = identity();
    m[0][0] = c;
    m[0][1] = -s;
    m[1][0] = s;
    m[1][1] = c;
    m
}

/// Perspective projection for a right-handed view looking down -Z with depth 0..1. `None` for a
/// non-positive field of view, aspect or near plane, or `far <= near`.
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Option<Mat4> {
    if !(fov_y > 0.0 && fov_y < std::f32::consts::PI && aspect > 0.0 && near > 0.0 && far > near) {
        return None;
    }
    let f = 1.0 / (fov_y * 0.5).tan();
    let mut m = [[0.0f32; 4]; 4];
    m[0][0] = f / aspect;
    m[1][1] = f;
    m[2][2] = far / (near - far);
    m[2][3] = near * far / (near - far);
    m[3][2] = -1.0;
    Some(m)
}

/// A point (w = 1) through a matrix to homogeneous clip coordinates.
pub fn transform_point(m: &Mat4, p: Vec3) -> [f32; 4] {
    let v = [p[0], p[1], p[2], 1.0];
    let mut out = [0.0f32; 4];
    for r in 0..4 {
        out[r] = m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2] + m[r][3] * v[3];
    }
    out
}

/// A direction (w = 0) through the upper 3x3 of a matrix.
pub fn transform_direction(m: &Mat4, d: Vec3) -> Vec3 {
    let mut out = [0.0f32; 3];
    for r in 0..3 {
        out[r] = m[r][0] * d[0] + m[r][1] * d[1] + m[r][2] * d[2];
    }
    out
}

/// Clip coordinates to screen pixels (origin top-left, y down) and depth 0..1. `None` for a point at or
/// behind the eye plane (`w <= 0`) or a degenerate viewport.
pub fn to_screen(clip: [f32; 4], width: f32, height: f32) -> Option<(f32, f32, f32)> {
    if !(clip[3] > 1e-9) || !(width > 0.0 && height > 0.0) {
        return None;
    }
    let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
    Some(((ndc[0] * 0.5 + 0.5) * width, (0.5 - ndc[1] * 0.5) * height, ndc[2]))
}

fn normalise(v: Vec3) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0; 3]
    }
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The normal matrix (inverse transpose of the upper 3x3), so normals stay perpendicular under
/// non-uniform scale. `None` for a singular matrix.
pub fn normal_matrix(m: &Mat4) -> Option<[[f32; 3]; 3]> {
    let a = |r: usize, c: usize| m[r][c];
    let cof = [
        [
            a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1),
            -(a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0)),
            a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0),
        ],
        [
            -(a(0, 1) * a(2, 2) - a(0, 2) * a(2, 1)),
            a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0),
            -(a(0, 0) * a(2, 1) - a(0, 1) * a(2, 0)),
        ],
        [
            a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1),
            -(a(0, 0) * a(1, 2) - a(0, 2) * a(1, 0)),
            a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0),
        ],
    ];
    let det = a(0, 0) * cof[0][0] + a(0, 1) * cof[0][1] + a(0, 2) * cof[0][2];
    if det.abs() < 1e-12 {
        return None;
    }
    let mut out = [[0.0f32; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = cof[r][c] / det;
        }
    }
    Some(out)
}

/// A unit normal through a normal matrix.
pub fn transform_normal(n: &[[f32; 3]; 3], v: Vec3) -> Vec3 {
    normalise([
        n[0][0] * v[0] + n[0][1] * v[1] + n[0][2] * v[2],
        n[1][0] * v[0] + n[1][1] * v[1] + n[1][2] * v[2],
        n[2][0] * v[0] + n[2][1] * v[1] + n[2][2] * v[2],
    ])
}

/// Linear-blend skinning of one position: the weighted sum of the point through each of up to four joint
/// matrices. Weights are renormalised to sum to 1 (all-zero weights leave the point unmoved). `None`
/// when a joint index is outside `palette`.
pub fn skin_position(p: Vec3, joints: [usize; 4], weights: [f32; 4], palette: &[Mat4]) -> Option<Vec3> {
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    if total <= 1e-9 {
        return Some(p);
    }
    let mut out = [0.0f32; 3];
    for i in 0..4 {
        let w = weights[i].max(0.0) / total;
        if w == 0.0 {
            continue;
        }
        let m = palette.get(joints[i])?;
        let q = transform_point(m, p);
        for c in 0..3 {
            out[c] += w * q[c];
        }
    }
    Some(out)
}

/// Linear-blend skinning of a normal: blended through the joint rotations, then renormalised (exact for
/// rotations; an approximation under joint scale, as the usual implementations are).
pub fn skin_normal(n: Vec3, joints: [usize; 4], weights: [f32; 4], palette: &[Mat4]) -> Option<Vec3> {
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    if total <= 1e-9 {
        return Some(n);
    }
    let mut out = [0.0f32; 3];
    for i in 0..4 {
        let w = weights[i].max(0.0) / total;
        if w == 0.0 {
            continue;
        }
        let m = palette.get(joints[i])?;
        let q = transform_direction(m, n);
        for c in 0..3 {
            out[c] += w * q[c];
        }
    }
    Some(normalise(out))
}

/// Morph targets: `base + sum(weight_i * delta_i)` per vertex. Every delta set must have the base's
/// length and there must be one weight per set, otherwise `None`.
pub fn apply_blend_shapes(base: &[Vec3], deltas: &[&[Vec3]], weights: &[f32]) -> Option<Vec<Vec3>> {
    if deltas.len() != weights.len() || deltas.iter().any(|d| d.len() != base.len()) {
        return None;
    }
    let mut out = base.to_vec();
    for (d, w) in deltas.iter().zip(weights.iter()) {
        for (o, dv) in out.iter_mut().zip(d.iter()) {
            for c in 0..3 {
                o[c] += w * dv[c];
            }
        }
    }
    Some(out)
}

/// Smooth vertex normals: each triangle adds its (area-weighted, unnormalised) face normal to its three
/// vertices, then every vertex normal is normalised. Triangles are index triples; `None` when an index
/// is out of range or the index count is not a multiple of three.
pub fn smooth_normals(positions: &[Vec3], indices: &[u32]) -> Option<Vec<Vec3>> {
    if indices.len() % 3 != 0 || indices.iter().any(|i| *i as usize >= positions.len()) {
        return None;
    }
    let mut acc = vec![[0.0f32; 3]; positions.len()];
    for tri in indices.chunks(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let n = cross(sub(positions[b], positions[a]), sub(positions[c], positions[a]));
        for v in [a, b, c] {
            for k in 0..3 {
                acc[v][k] += n[k];
            }
        }
    }
    Some(acc.into_iter().map(normalise).collect())
}

/// The tangent and bitangent of a triangle from its positions and UVs (the directions in which U and V
/// grow). `None` for a degenerate UV mapping.
pub fn tangent_frame(p: [Vec3; 3], uv: [[f32; 2]; 3]) -> Option<(Vec3, Vec3)> {
    let e1 = sub(p[1], p[0]);
    let e2 = sub(p[2], p[0]);
    let (du1, dv1) = (uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]);
    let (du2, dv2) = (uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]);
    let det = du1 * dv2 - du2 * dv1;
    if det.abs() < 1e-12 {
        return None;
    }
    let r = 1.0 / det;
    let mut t = [0.0f32; 3];
    let mut b = [0.0f32; 3];
    for k in 0..3 {
        t[k] = (e1[k] * dv2 - e2[k] * dv1) * r;
        b[k] = (e2[k] * du1 - e1[k] * du2) * r;
    }
    Some((normalise(t), normalise(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close3(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn matrices_compose_right_to_left() {
        let m = mul(&translation([1.0, 0.0, 0.0]), &scaling([2.0, 2.0, 2.0]));
        let p = transform_point(&m, [1.0, 1.0, 1.0]);
        assert_eq!(&p[0..3], &[3.0, 2.0, 2.0]);
        assert_eq!(mul(&identity(), &m), m);
        let r = transform_point(&rotation_z(std::f32::consts::FRAC_PI_2), [1.0, 0.0, 0.0]);
        assert!(close3([r[0], r[1], r[2]], [0.0, 1.0, 0.0]));
    }

    #[test]
    fn perspective_maps_near_to_zero_and_far_to_one() {
        let p = perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 10.0).unwrap();
        let near = transform_point(&p, [0.0, 0.0, -1.0]);
        let far = transform_point(&p, [0.0, 0.0, -10.0]);
        assert!((near[2] / near[3]).abs() < 1e-5);
        assert!((far[2] / far[3] - 1.0).abs() < 1e-5);
        assert!(perspective(0.0, 1.0, 1.0, 10.0).is_none());
        assert!(perspective(1.0, 1.0, 5.0, 5.0).is_none());
    }

    #[test]
    fn the_view_axis_lands_in_the_middle_of_the_screen() {
        let p = perspective(1.0, 2.0, 0.1, 100.0).unwrap();
        let clip = transform_point(&p, [0.0, 0.0, -5.0]);
        let (x, y, z) = to_screen(clip, 800.0, 400.0).unwrap();
        assert!((x - 400.0).abs() < 1e-3 && (y - 200.0).abs() < 1e-3);
        assert!(z > 0.0 && z < 1.0);
        let up = to_screen(transform_point(&p, [0.0, 1.0, -5.0]), 800.0, 400.0).unwrap();
        assert!(up.1 < 200.0, "world up is screen up (smaller y)");
        assert!(to_screen(transform_point(&p, [0.0, 0.0, 5.0]), 800.0, 400.0).is_none());
    }

    #[test]
    fn normal_matrix_keeps_normals_perpendicular_under_stretch() {
        let n = normal_matrix(&scaling([2.0, 1.0, 1.0])).unwrap();
        let out = transform_normal(&n, [1.0, 1.0, 0.0]);
        assert!(close3(out, normalise([0.5, 1.0, 0.0])));
        assert!(normal_matrix(&scaling([0.0, 1.0, 1.0])).is_none());
    }

    #[test]
    fn skinning_blends_joint_transforms() {
        let palette = [identity(), translation([2.0, 0.0, 0.0])];
        let p = [1.0, 1.0, 1.0];
        let rigid = skin_position(p, [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0], &palette).unwrap();
        assert!(close3(rigid, p));
        let half = skin_position(p, [0, 1, 0, 0], [0.5, 0.5, 0.0, 0.0], &palette).unwrap();
        assert!(close3(half, [2.0, 1.0, 1.0]));
        let unnormalised = skin_position(p, [0, 1, 0, 0], [1.0, 1.0, 0.0, 0.0], &palette).unwrap();
        assert!(close3(unnormalised, [2.0, 1.0, 1.0]));
        assert_eq!(skin_position(p, [0, 0, 0, 0], [0.0; 4], &palette), Some(p));
        assert!(skin_position(p, [5, 0, 0, 0], [1.0, 0.0, 0.0, 0.0], &palette).is_none());
    }

    #[test]
    fn skinned_normals_blend_rotations() {
        let palette = [identity(), rotation_z(std::f32::consts::FRAC_PI_2)];
        let n = skin_normal([1.0, 0.0, 0.0], [0, 1, 0, 0], [0.5, 0.5, 0.0, 0.0], &palette).unwrap();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close3(n, [s, s, 0.0]));
    }

    #[test]
    fn blend_shapes_add_weighted_deltas() {
        let base = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let up = [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let right = [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let out = apply_blend_shapes(&base, &[&up, &right], &[1.0, 0.5]).unwrap();
        assert!(close3(out[0], [0.5, 1.0, 0.0]) && close3(out[1], [1.5, 1.0, 0.0]));
        assert_eq!(apply_blend_shapes(&base, &[], &[]).unwrap(), base.to_vec());
        assert!(apply_blend_shapes(&base, &[&up], &[1.0, 2.0]).is_none());
        assert!(apply_blend_shapes(&base, &[&up[..1]], &[1.0]).is_none());
    }

    #[test]
    fn smooth_normals_of_a_triangle_and_a_shared_edge() {
        let pos = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let n = smooth_normals(&pos, &[0, 1, 2]).unwrap();
        for v in &n {
            assert!(close3(*v, [0.0, 0.0, 1.0]));
        }
        assert!(smooth_normals(&pos, &[0, 1]).is_none());
        assert!(smooth_normals(&pos, &[0, 1, 3]).is_none());
        // Two triangles folded 90 degrees share vertices 0 and 1: their normals there tilt halfway.
        let fold = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let n = smooth_normals(&fold, &[0, 1, 2, 0, 3, 1]).unwrap();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close3(n[0], [0.0, s, s]) || close3(n[0], [0.0, -s, s]) || close3(n[0], [0.0, s, -s]) || close3(n[0], [0.0, -s, -s]));
    }

    #[test]
    fn tangent_frame_follows_the_uv_axes() {
        let p = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let (t, b) = tangent_frame(p, [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]).unwrap();
        assert!(close3(t, [1.0, 0.0, 0.0]) && close3(b, [0.0, 1.0, 0.0]));
        // Flip V: the bitangent flips with it.
        let (_, b) = tangent_frame(p, [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]]).unwrap();
        assert!(close3(b, [0.0, -1.0, 0.0]));
        assert!(tangent_frame(p, [[0.0, 0.0]; 3]).is_none());
    }
}
