//! 3D transforms: a 4x4 matrix in the layout of the reference SDK's layer transform (`m11`..`m44`, row vectors).
//! A point `(x, y, z)` is the row vector `[x, y, z, 1]` times the matrix, so the translation sits in the fourth row
//! (`m41`, `m42`, `m43`) and the perspective terms sit in the fourth column (`m14`, `m24`, `m34`).
//! `concat(a, b)` applies `a` first and then `b`, which is the order the Make/Concat/Rotate helpers of the reference
//! SDK use. Pure maths: the renderer applies the result to its own layers and the spatial types read it directly.
//! The matrix is row-major internally: `m[r][c]` is `m{r+1}{c+1}`.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform3D {
    pub m: [[f32; 4]; 4],
}

impl Default for Transform3D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform3D {
    pub const IDENTITY: Transform3D = Transform3D {
        m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
    };

    pub fn identity() -> Self {
        Self::IDENTITY
    }

    /// Moves points by `(tx, ty, tz)`.
    pub fn make_translation(tx: f32, ty: f32, tz: f32) -> Self {
        let mut t = Self::IDENTITY;
        t.m[3][0] = tx;
        t.m[3][1] = ty;
        t.m[3][2] = tz;
        t
    }

    /// Scales each axis about the origin.
    pub fn make_scale(sx: f32, sy: f32, sz: f32) -> Self {
        let mut t = Self::IDENTITY;
        t.m[0][0] = sx;
        t.m[1][1] = sy;
        t.m[2][2] = sz;
        t
    }

    /// Rotates by `radians` about the axis `(x, y, z)` through the origin, right-handed. The axis is normalised;
    /// a zero axis gives the identity.
    pub fn make_rotation(radians: f32, x: f32, y: f32, z: f32) -> Self {
        let len = (x * x + y * y + z * z).sqrt();
        if len == 0.0 || !len.is_finite() {
            return Self::IDENTITY;
        }
        let (x, y, z) = (x / len, y / len, z / len);
        let (s, c) = radians.sin_cos();
        let k = 1.0 - c;
        // Column-vector rotation R; the row-vector matrix is its transpose.
        let r = [
            [c + x * x * k, x * y * k - z * s, x * z * k + y * s],
            [y * x * k + z * s, c + y * y * k, y * z * k - x * s],
            [z * x * k - y * s, z * y * k + x * s, c + z * z * k],
        ];
        let mut t = Self::IDENTITY;
        for (i, row) in r.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                t.m[j][i] = *v;
            }
        }
        t
    }

    /// The transform that applies `self` first and then `next` (the row-vector product `self * next`).
    pub fn concat(self, next: Transform3D) -> Transform3D {
        let mut out = [[0.0f32; 4]; 4];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.m[i][k] * next.m[k][j]).sum();
            }
        }
        Transform3D { m: out }
    }

    /// `self` followed by a translation.
    pub fn translated(self, tx: f32, ty: f32, tz: f32) -> Transform3D {
        self.concat(Self::make_translation(tx, ty, tz))
    }

    /// `self` followed by a scale.
    pub fn scaled(self, sx: f32, sy: f32, sz: f32) -> Transform3D {
        self.concat(Self::make_scale(sx, sy, sz))
    }

    /// `self` followed by a rotation about `(x, y, z)`.
    pub fn rotated(self, radians: f32, x: f32, y: f32, z: f32) -> Transform3D {
        self.concat(Self::make_rotation(radians, x, y, z))
    }

    /// A perspective transform with the eye at `distance` along the z axis (`m34 = -1 / distance`). Points are
    /// divided by the resulting `w` by [`Transform3D::apply`]. A zero distance gives the identity.
    pub fn make_perspective(distance: f32) -> Transform3D {
        let mut t = Self::IDENTITY;
        if distance != 0.0 && distance.is_finite() {
            t.m[2][3] = -1.0 / distance;
        }
        t
    }

    /// The inverse, or `None` when the matrix is singular (no unique inverse).
    pub fn invert(self) -> Option<Transform3D> {
        // Gauss-Jordan elimination with partial pivoting on [m | I].
        let mut a = self.m;
        let mut inv = Self::IDENTITY.m;
        for col in 0..4 {
            let pivot = (col..4).max_by(|&p, &q| a[p][col].abs().total_cmp(&a[q][col].abs()))?;
            if a[pivot][col].abs() < 1e-12 {
                return None;
            }
            a.swap(col, pivot);
            inv.swap(col, pivot);
            let p = a[col][col];
            for j in 0..4 {
                a[col][j] /= p;
                inv[col][j] /= p;
            }
            for row in 0..4 {
                if row != col {
                    let f = a[row][col];
                    if f != 0.0 {
                        for j in 0..4 {
                            a[row][j] -= f * a[col][j];
                            inv[row][j] -= f * inv[col][j];
                        }
                    }
                }
            }
        }
        Some(Transform3D { m: inv })
    }

    /// True when the fourth column is `(0, 0, 0, 1)`, so the transform keeps parallel lines parallel and never
    /// divides by `w`.
    pub fn is_affine(self) -> bool {
        self.m[0][3] == 0.0 && self.m[1][3] == 0.0 && self.m[2][3] == 0.0 && self.m[3][3] == 1.0
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    /// Maps a point. Returns `None` when the homogeneous `w` is zero (the point maps to infinity).
    pub fn apply(self, x: f32, y: f32, z: f32) -> Option<(f32, f32, f32)> {
        let m = &self.m;
        let px = x * m[0][0] + y * m[1][0] + z * m[2][0] + m[3][0];
        let py = x * m[0][1] + y * m[1][1] + z * m[2][1] + m[3][1];
        let pz = x * m[0][2] + y * m[1][2] + z * m[2][2] + m[3][2];
        let w = x * m[0][3] + y * m[1][3] + z * m[2][3] + m[3][3];
        if w == 0.0 {
            return None;
        }
        Some((px / w, py / w, pz / w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32, f32), b: (f32, f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4 && (a.2 - b.2).abs() < 1e-4
    }

    #[test]
    fn translation_moves_points_and_keeps_the_fourth_row() {
        let t = Transform3D::make_translation(3.0, -2.0, 5.0);
        assert!(close(t.apply(1.0, 1.0, 1.0).unwrap(), (4.0, -1.0, 6.0)));
        assert_eq!(t.m[3], [3.0, -2.0, 5.0, 1.0]);
        assert!(t.is_affine());
    }

    #[test]
    fn concat_applies_the_first_transform_first() {
        // Scale by 2 then move by 10: x = 1 becomes 12.
        let scale_then_move = Transform3D::make_scale(2.0, 1.0, 1.0).concat(Transform3D::make_translation(10.0, 0.0, 0.0));
        assert!(close(scale_then_move.apply(1.0, 0.0, 0.0).unwrap(), (12.0, 0.0, 0.0)));
        // Move by 10 then scale by 2: x = 1 becomes 22.
        let move_then_scale = Transform3D::make_translation(10.0, 0.0, 0.0).concat(Transform3D::make_scale(2.0, 1.0, 1.0));
        assert!(close(move_then_scale.apply(1.0, 0.0, 0.0).unwrap(), (22.0, 0.0, 0.0)));
        assert_ne!(scale_then_move, move_then_scale);
    }

    #[test]
    fn quarter_turn_about_z_maps_x_to_y_and_matches_the_row_layout() {
        let r = Transform3D::make_rotation(std::f32::consts::FRAC_PI_2, 0.0, 0.0, 1.0);
        assert!(close(r.apply(1.0, 0.0, 0.0).unwrap(), (0.0, 1.0, 0.0)));
        // Row-vector layout: m11 = cos, m12 = sin, m21 = -sin, m22 = cos.
        assert!(r.m[0][0].abs() < 1e-6 && (r.m[0][1] - 1.0).abs() < 1e-6 && (r.m[1][0] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn rotation_axis_is_normalised_and_a_zero_axis_is_the_identity() {
        let a = Transform3D::make_rotation(0.7, 0.0, 0.0, 1.0);
        let b = Transform3D::make_rotation(0.7, 0.0, 0.0, 4.0);
        assert!(close(a.apply(1.0, 2.0, 3.0).unwrap(), b.apply(1.0, 2.0, 3.0).unwrap()));
        assert!(Transform3D::make_rotation(1.0, 0.0, 0.0, 0.0).is_identity());
    }

    #[test]
    fn invert_round_trips_and_singular_matrices_have_no_inverse() {
        let t = Transform3D::identity()
            .rotated(0.4, 1.0, 1.0, 0.0)
            .scaled(2.0, 3.0, 0.5)
            .translated(7.0, -1.0, 2.0);
        let back = t.invert().expect("invertible");
        let p = t.apply(1.5, -2.0, 0.25).unwrap();
        assert!(close(back.apply(p.0, p.1, p.2).unwrap(), (1.5, -2.0, 0.25)));
        assert!(Transform3D::make_scale(1.0, 0.0, 1.0).invert().is_none());
    }

    #[test]
    fn perspective_sets_m34_and_divides_by_w() {
        let p = Transform3D::make_perspective(100.0);
        assert!(!p.is_affine());
        assert!((p.m[2][3] + 0.01).abs() < 1e-7);
        // A point at z = 50 has w = 1 - 50/100 = 0.5, so x doubles.
        let (x, _, z) = p.apply(1.0, 0.0, 50.0).unwrap();
        assert!((x - 2.0).abs() < 1e-5 && (z - 100.0).abs() < 1e-3);
        // A point at z = 100 has w = 0, so it maps to infinity.
        assert!(p.apply(1.0, 0.0, 100.0).is_none());
        assert!(Transform3D::make_perspective(0.0).is_identity());
    }
}
