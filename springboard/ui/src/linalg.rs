//! Vectors, 4x4 matrices and unit quaternions for 3D content (`NativeVector3`, `NativeVector4`, `NativeMatrix4`, `NativeQuaternion`).
//! Plain `f32` values with no allocation. Matrices are column-major: element (row `r`, column `c`) is `m[c * 4 + r]`,
//! so the translation sits in `m[12..15]` and a point transforms as `M * (x, y, z, 1)`.

use std::ops::{Add, Mul, Neg, Sub};
use std::f32::consts::PI;

/// Converts an angle in degrees to radians (`NativeMathDegreesToRadians`).
pub fn degrees_to_radians(degrees: f32) -> f32 {
    degrees * PI / 180.0
}

/// Converts an angle in radians to degrees (`NativeMathRadiansToDegrees`).
pub fn radians_to_degrees(radians: f32) -> f32 {
    radians * 180.0 / PI
}

/// A three-component vector (`NativeVector3`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3 {
    pub const ZERO: Vector3 = Vector3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Vector3 { x, y, z }
    }

    pub fn dot(self, o: Vector3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vector3) -> Vector3 {
        Vector3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The unit vector in the same direction; a zero vector stays zero.
    pub fn normalized(self) -> Vector3 {
        let len = self.length();
        if len > 0.0 {
            self * (1.0 / len)
        } else {
            Vector3::ZERO
        }
    }

    /// Extends to a homogeneous point or direction with the given `w`.
    pub fn to_vector4(self, w: f32) -> Vector4 {
        Vector4 { x: self.x, y: self.y, z: self.z, w }
    }
}

impl Add for Vector3 {
    type Output = Vector3;
    fn add(self, o: Vector3) -> Vector3 {
        Vector3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vector3 {
    type Output = Vector3;
    fn sub(self, o: Vector3) -> Vector3 {
        Vector3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vector3 {
    type Output = Vector3;
    fn mul(self, s: f32) -> Vector3 {
        Vector3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vector3 {
    type Output = Vector3;
    fn neg(self) -> Vector3 {
        Vector3::new(-self.x, -self.y, -self.z)
    }
}

/// A homogeneous four-component vector (`NativeVector4`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vector4 {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Vector4 { x, y, z, w }
    }

    pub fn dot(self, o: Vector4) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// The first three components.
    pub fn xyz(self) -> Vector3 {
        Vector3::new(self.x, self.y, self.z)
    }
}

/// A 4x4 column-major matrix (`NativeMatrix4`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix4 {
    pub m: [f32; 16],
}

impl Default for Matrix4 {
    fn default() -> Self {
        Matrix4::identity()
    }
}

impl Matrix4 {
    /// `NativeMatrix4Identity`.
    pub fn identity() -> Self {
        let mut m = [0.0; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        Matrix4 { m }
    }

    /// `NativeMatrix4MakeTranslation`.
    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut out = Matrix4::identity();
        out.m[12] = x;
        out.m[13] = y;
        out.m[14] = z;
        out
    }

    /// `NativeMatrix4MakeScale`.
    pub fn scaling(x: f32, y: f32, z: f32) -> Self {
        let mut out = Matrix4::identity();
        out.m[0] = x;
        out.m[5] = y;
        out.m[10] = z;
        out
    }

    /// Rotation by `radians` about `axis` (`NativeMatrix4MakeRotation`). A zero axis gives the identity.
    pub fn rotation(radians: f32, axis: Vector3) -> Self {
        let a = axis.normalized();
        if a == Vector3::ZERO {
            return Matrix4::identity();
        }
        let (s, c) = radians.sin_cos();
        let t = 1.0 - c;
        let (x, y, z) = (a.x, a.y, a.z);
        Matrix4 {
            m: [
                t * x * x + c,
                t * x * y + s * z,
                t * x * z - s * y,
                0.0,
                t * x * y - s * z,
                t * y * y + c,
                t * y * z + s * x,
                0.0,
                t * x * z + s * y,
                t * y * z - s * x,
                t * z * z + c,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ],
        }
    }

    /// Right-handed perspective projection onto the -1..1 clip cube (`NativeMatrix4MakePerspective`).
    pub fn perspective(fovy_radians: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fovy_radians * 0.5).tan();
        let mut m = [0.0; 16];
        m[0] = f / aspect;
        m[5] = f;
        m[10] = (far + near) / (near - far);
        m[11] = -1.0;
        m[14] = 2.0 * far * near / (near - far);
        Matrix4 { m }
    }

    /// View matrix looking from `eye` toward `center` (`NativeMatrix4MakeLookAt`).
    pub fn look_at(eye: Vector3, center: Vector3, up: Vector3) -> Self {
        let f = (center - eye).normalized();
        let s = f.cross(up).normalized();
        let u = s.cross(f);
        let mut m = [0.0; 16];
        m[0] = s.x;
        m[4] = s.y;
        m[8] = s.z;
        m[1] = u.x;
        m[5] = u.y;
        m[9] = u.z;
        m[2] = -f.x;
        m[6] = -f.y;
        m[10] = -f.z;
        m[12] = -s.dot(eye);
        m[13] = -u.dot(eye);
        m[14] = f.dot(eye);
        m[15] = 1.0;
        Matrix4 { m }
    }

    /// Applies the matrix to a homogeneous vector (`NativeMatrix4MultiplyVector4`).
    pub fn transform(self, v: Vector4) -> Vector4 {
        let row = |r: usize| self.m[r] * v.x + self.m[4 + r] * v.y + self.m[8 + r] * v.z + self.m[12 + r] * v.w;
        Vector4 { x: row(0), y: row(1), z: row(2), w: row(3) }
    }

    /// Transforms a point and divides by the resulting `w`, as a projection needs. `None` when `w` is zero.
    pub fn transform_point(self, p: Vector3) -> Option<Vector3> {
        let v = self.transform(p.to_vector4(1.0));
        if v.w == 0.0 {
            None
        } else {
            Some(Vector3::new(v.x / v.w, v.y / v.w, v.z / v.w))
        }
    }

    pub fn transposed(self) -> Matrix4 {
        let mut m = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                m[r * 4 + c] = self.m[c * 4 + r];
            }
        }
        Matrix4 { m }
    }

    /// Inverse by Gauss-Jordan elimination with partial pivoting. `None` when the matrix is singular.
    pub fn inverse(self) -> Option<Matrix4> {
        // Row-major working copy: the left half is the matrix, the right half starts as the identity.
        let mut a = [[0.0f32; 8]; 4];
        for r in 0..4 {
            for c in 0..4 {
                a[r][c] = self.m[c * 4 + r];
            }
            a[r][4 + r] = 1.0;
        }
        for col in 0..4 {
            let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
            if a[pivot][col].abs() < 1e-12 {
                return None;
            }
            a.swap(col, pivot);
            let p = a[col][col];
            for v in a[col].iter_mut() {
                *v /= p;
            }
            let pivot_row = a[col];
            for (r, row) in a.iter_mut().enumerate() {
                if r != col {
                    let f = row[col];
                    for c in 0..8 {
                        row[c] -= f * pivot_row[c];
                    }
                }
            }
        }
        let mut m = [0.0; 16];
        for r in 0..4 {
            for c in 0..4 {
                m[c * 4 + r] = a[r][4 + c];
            }
        }
        Some(Matrix4 { m })
    }
}

impl Mul for Matrix4 {
    type Output = Matrix4;
    /// `self * rhs`: `rhs` is applied first when transforming a point.
    fn mul(self, rhs: Matrix4) -> Matrix4 {
        let mut m = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.m[k * 4 + r] * rhs.m[c * 4 + k];
                }
                m[c * 4 + r] = sum;
            }
        }
        Matrix4 { m }
    }
}

/// A quaternion `x i + y j + z k + w` (`NativeQuaternion`). Unit quaternions represent rotations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quaternion {
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Quaternion { x, y, z, w }
    }

    pub fn identity() -> Self {
        Quaternion::new(0.0, 0.0, 0.0, 1.0)
    }

    /// The rotation by `radians` about `axis` (`NativeQuaternionMakeWithAngleAndAxis`).
    pub fn from_axis_angle(axis: Vector3, radians: f32) -> Self {
        let a = axis.normalized();
        let (s, c) = (radians * 0.5).sin_cos();
        Quaternion::new(a.x * s, a.y * s, a.z * s, c)
    }

    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt()
    }

    /// The unit quaternion in the same direction; a zero quaternion becomes the identity.
    pub fn normalized(self) -> Quaternion {
        let len = self.length();
        if len > 0.0 {
            let s = 1.0 / len;
            Quaternion::new(self.x * s, self.y * s, self.z * s, self.w * s)
        } else {
            Quaternion::identity()
        }
    }

    pub fn conjugate(self) -> Quaternion {
        Quaternion::new(-self.x, -self.y, -self.z, self.w)
    }

    pub fn dot(self, o: Quaternion) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// Rotates a vector by this unit quaternion: `q v q*`.
    pub fn rotate(self, v: Vector3) -> Vector3 {
        let p = Quaternion::new(v.x, v.y, v.z, 0.0);
        let r = self * p * self.conjugate();
        Vector3::new(r.x, r.y, r.z)
    }

    /// Spherical interpolation from `from` to `to` (`NativeQuaternionSlerp`), `t` clamped to 0..=1.
    /// Takes the shorter arc and falls back to a normalized linear blend when the inputs are nearly equal.
    pub fn slerp(from: Quaternion, to: Quaternion, t: f32) -> Quaternion {
        let t = t.clamp(0.0, 1.0);
        let mut end = to;
        let mut d = from.dot(to);
        if d < 0.0 {
            end = Quaternion::new(-to.x, -to.y, -to.z, -to.w);
            d = -d;
        }
        if d > 0.9995 {
            return blend(from, 1.0 - t, end, t).normalized();
        }
        let theta0 = d.clamp(-1.0, 1.0).acos();
        let theta = theta0 * t;
        let sin0 = theta0.sin();
        let s0 = theta.cos() - d * theta.sin() / sin0;
        let s1 = theta.sin() / sin0;
        blend(from, s0, end, s1)
    }

    /// The rotation matrix of this quaternion (`NativeMatrix4MakeWithQuaternion`).
    pub fn to_matrix4(self) -> Matrix4 {
        let q = self.normalized();
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        Matrix4 {
            m: [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + z * w),
                2.0 * (x * z - y * w),
                0.0,
                2.0 * (x * y - z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + x * w),
                0.0,
                2.0 * (x * z + y * w),
                2.0 * (y * z - x * w),
                1.0 - 2.0 * (x * x + y * y),
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ],
        }
    }
}

/// `a * sa + b * sb`, component-wise.
fn blend(a: Quaternion, sa: f32, b: Quaternion, sb: f32) -> Quaternion {
    Quaternion::new(a.x * sa + b.x * sb, a.y * sa + b.y * sb, a.z * sa + b.z * sb, a.w * sa + b.w * sb)
}

impl Mul for Quaternion {
    type Output = Quaternion;
    /// Hamilton product. `(a * b).rotate(v)` equals `a.rotate(b.rotate(v))`.
    fn mul(self, b: Quaternion) -> Quaternion {
        let a = self;
        Quaternion::new(
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    fn assert_vec3(actual: Vector3, expected: Vector3) {
        assert!(
            close(actual.x, expected.x) && close(actual.y, expected.y) && close(actual.z, expected.z),
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn assert_matrix(actual: Matrix4, expected: Matrix4) {
        for i in 0..16 {
            assert!(close(actual.m[i], expected.m[i]), "element {i}: expected {:?}, got {:?}", expected.m, actual.m);
        }
    }

    #[test]
    fn identity_is_neutral_and_translation_moves_points() {
        let t = Matrix4::translation(1.0, 2.0, 3.0);
        assert_matrix(t * Matrix4::identity(), t);
        assert_matrix(Matrix4::identity() * t, t);
        assert_vec3(t.transform_point(Vector3::ZERO).unwrap(), Vector3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn rotation_about_z_turns_x_into_y() {
        let r = Matrix4::rotation(std::f32::consts::FRAC_PI_2, Vector3::new(0.0, 0.0, 2.0));
        let v = r.transform(Vector4::new(1.0, 0.0, 0.0, 0.0));
        assert!(close(v.x, 0.0) && close(v.y, 1.0) && close(v.z, 0.0), "got {v:?}");
        assert_matrix(Matrix4::rotation(1.0, Vector3::ZERO), Matrix4::identity());
    }

    #[test]
    fn inverse_round_trips_and_singular_matrices_have_none() {
        let m = Matrix4::translation(4.0, -1.0, 2.0)
            * Matrix4::rotation(0.7, Vector3::new(1.0, 1.0, 0.0))
            * Matrix4::scaling(2.0, 3.0, 0.5);
        let inv = m.inverse().expect("invertible");
        assert_matrix(m * inv, Matrix4::identity());
        assert!(Matrix4::scaling(1.0, 0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn look_at_puts_the_eye_at_the_origin_and_perspective_maps_near_to_minus_one() {
        let eye = Vector3::new(0.0, 0.0, 5.0);
        let view = Matrix4::look_at(eye, Vector3::ZERO, Vector3::new(0.0, 1.0, 0.0));
        assert_vec3(view.transform_point(eye).unwrap(), Vector3::ZERO);
        // The target lies 5 units in front of the eye, so view space puts it at z = -5.
        assert_vec3(view.transform_point(Vector3::ZERO).unwrap(), Vector3::new(0.0, 0.0, -5.0));

        let proj = Matrix4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
        let near = proj.transform_point(Vector3::new(0.0, 0.0, -1.0)).unwrap();
        assert!(close(near.z, -1.0), "near plane at z {}", near.z);
    }

    #[test]
    fn quaternion_rotation_matches_the_matrix_rotation() {
        let axis = Vector3::new(1.0, 2.0, 3.0);
        let angle = degrees_to_radians(70.0);
        let q = Quaternion::from_axis_angle(axis, angle);
        assert_matrix(q.to_matrix4(), Matrix4::rotation(angle, axis));
        let v = Vector3::new(0.3, -1.2, 2.0);
        let by_matrix = Matrix4::rotation(angle, axis).transform_point(v).unwrap();
        assert_vec3(q.rotate(v), by_matrix);
    }

    #[test]
    fn quaternion_product_composes_rotations_in_order() {
        let a = Quaternion::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), degrees_to_radians(90.0));
        let b = Quaternion::from_axis_angle(Vector3::new(1.0, 0.0, 0.0), degrees_to_radians(90.0));
        let v = Vector3::new(0.0, 1.0, 0.0);
        assert_vec3((a * b).rotate(v), a.rotate(b.rotate(v)));
    }

    #[test]
    fn slerp_halfway_is_half_the_turn_and_takes_the_short_arc() {
        let z = Vector3::new(0.0, 0.0, 1.0);
        let start = Quaternion::identity();
        let end = Quaternion::from_axis_angle(z, degrees_to_radians(90.0));
        let half = Quaternion::slerp(start, end, 0.5);
        assert_matrix(half.to_matrix4(), Matrix4::rotation(degrees_to_radians(45.0), z));
        // A negated quaternion is the same rotation, so slerp must not take the long way round.
        let negated = Quaternion::new(-end.x, -end.y, -end.z, -end.w);
        assert_matrix(Quaternion::slerp(start, negated, 0.5).to_matrix4(), half.to_matrix4());
    }

    #[test]
    fn degree_and_radian_conversions_round_trip() {
        assert!(close(degrees_to_radians(180.0), PI));
        assert!(close(radians_to_degrees(degrees_to_radians(37.5)), 37.5));
    }
}
