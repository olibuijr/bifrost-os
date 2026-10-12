//! Rotations, Euler angles, and poses (a position with a rotation, optionally scaled) for 3D content.
//! Built on `linalg::Quaternion`. Angles are radians. Rotations are right-handed: a positive angle turns
//! counter-clockwise when looking back along the axis towards the origin.

use crate::linalg::{Quaternion, Vector3};
use crate::spatial::Point3;

/// Euler angles applied in a fixed order: roll about X first, then pitch about Y, then yaw about Z.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EulerAngles {
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// A rotation stored as a unit quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation3 {
    pub quaternion: Quaternion,
}

impl Default for Rotation3 {
    fn default() -> Self {
        Rotation3::identity()
    }
}

impl Rotation3 {
    pub fn identity() -> Self {
        Rotation3 { quaternion: Quaternion::identity() }
    }

    /// The rotation by `radians` about `axis`. A zero axis gives the identity.
    pub fn from_axis_angle(axis: Vector3, radians: f32) -> Self {
        if axis.length() == 0.0 {
            return Rotation3::identity();
        }
        Rotation3 { quaternion: Quaternion::from_axis_angle(axis, radians).normalized() }
    }

    pub fn from_euler(e: EulerAngles) -> Self {
        let x = Quaternion::from_axis_angle(Vector3::new(1.0, 0.0, 0.0), e.roll);
        let y = Quaternion::from_axis_angle(Vector3::new(0.0, 1.0, 0.0), e.pitch);
        let z = Quaternion::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), e.yaw);
        Rotation3 { quaternion: (z * y * x).normalized() }
    }

    /// This rotation followed by `next`.
    pub fn then(self, next: Rotation3) -> Rotation3 {
        Rotation3 { quaternion: (next.quaternion * self.quaternion).normalized() }
    }

    /// The rotation that undoes this one.
    pub fn inverse(self) -> Rotation3 {
        Rotation3 { quaternion: self.quaternion.normalized().conjugate() }
    }

    pub fn rotate(self, v: Vector3) -> Vector3 {
        self.quaternion.normalized().rotate(v)
    }

    /// The unit axis and the angle in radians, from the quaternion. The identity gives a zero axis and angle 0.
    pub fn axis_angle(self) -> (Vector3, f32) {
        let q = self.quaternion.normalized();
        let v = Vector3::new(q.x, q.y, q.z);
        let s = v.length();
        if s == 0.0 {
            return (Vector3::ZERO, 0.0);
        }
        (v * (1.0 / s), 2.0 * s.atan2(q.w))
    }
}

/// A position and a rotation. `apply` maps a point from local space to parent space: rotate, then translate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose3 {
    pub position: Point3,
    pub rotation: Rotation3,
}

impl Pose3 {
    pub fn new(position: Point3, rotation: Rotation3) -> Self {
        Pose3 { position, rotation }
    }

    /// Local point to parent space.
    pub fn apply(self, p: Point3) -> Point3 {
        self.position.offset(self.rotation.rotate(p.to_vector()))
    }

    /// Parent point to local space, the inverse of `apply`.
    pub fn to_local(self, p: Point3) -> Point3 {
        let d = p.to_vector() - self.position.to_vector();
        Point3::from_vector(self.rotation.inverse().rotate(d))
    }

    /// This pose expressed in the space of `parent`, so that `result.apply(p)` equals `parent.apply(self.apply(p))`.
    pub fn in_parent(self, parent: Pose3) -> Pose3 {
        Pose3 {
            position: parent.apply(self.position),
            rotation: self.rotation.then(parent.rotation),
        }
    }
}

/// A pose with a uniform scale applied to the local point before the rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaledPose3 {
    pub pose: Pose3,
    pub scale: f32,
}

impl ScaledPose3 {
    /// Local point to parent space: scale, rotate, then translate.
    pub fn apply(self, p: Point3) -> Point3 {
        self.pose.apply(Point3::from_vector(p.to_vector() * self.scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

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

    fn assert_point(actual: Point3, x: f32, y: f32, z: f32) {
        assert_vec(actual.to_vector(), x, y, z);
    }

    #[test]
    fn identity_leaves_vectors_unchanged() {
        assert_vec(Rotation3::identity().rotate(Vector3::new(1.0, 2.0, 3.0)), 1.0, 2.0, 3.0);
        assert_vec(Rotation3::from_axis_angle(Vector3::ZERO, 1.0).rotate(Vector3::new(0.0, 1.0, 0.0)), 0.0, 1.0, 0.0);
    }

    #[test]
    fn quarter_turn_about_z_maps_x_to_y() {
        let r = Rotation3::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), FRAC_PI_2);
        assert_vec(r.rotate(Vector3::new(1.0, 0.0, 0.0)), 0.0, 1.0, 0.0);
    }

    #[test]
    fn axis_angle_round_trips() {
        let r = Rotation3::from_axis_angle(Vector3::new(0.0, 0.0, 2.0), 0.5);
        let (axis, angle) = r.axis_angle();
        assert_vec(axis, 0.0, 0.0, 1.0);
        assert!(close(angle, 0.5));
        let (none, zero) = Rotation3::identity().axis_angle();
        assert_vec(none, 0.0, 0.0, 0.0);
        assert!(close(zero, 0.0));
    }

    #[test]
    fn two_quarter_turns_make_a_half_turn() {
        let quarter = Rotation3::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), FRAC_PI_2);
        let half = quarter.then(quarter);
        assert_vec(half.rotate(Vector3::new(1.0, 0.0, 0.0)), -1.0, 0.0, 0.0);
        assert_vec(half.inverse().rotate(Vector3::new(1.0, 0.0, 0.0)), -1.0, 0.0, 0.0);
    }

    #[test]
    fn euler_roll_and_yaw_match_axis_turns() {
        let yaw = Rotation3::from_euler(EulerAngles { roll: 0.0, pitch: 0.0, yaw: FRAC_PI_2 });
        assert_vec(yaw.rotate(Vector3::new(1.0, 0.0, 0.0)), 0.0, 1.0, 0.0);

        let roll = Rotation3::from_euler(EulerAngles { roll: FRAC_PI_2, pitch: 0.0, yaw: 0.0 });
        assert_vec(roll.rotate(Vector3::new(0.0, 1.0, 0.0)), 0.0, 0.0, 1.0);

        let half_turn = Rotation3::from_euler(EulerAngles { roll: 0.0, pitch: PI, yaw: 0.0 });
        assert_vec(half_turn.rotate(Vector3::new(1.0, 0.0, 0.0)), -1.0, 0.0, 0.0);
    }

    #[test]
    fn pose_applies_rotation_then_translation_and_inverts() {
        let pose = Pose3::new(
            Point3::new(1.0, 0.0, 0.0),
            Rotation3::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), FRAC_PI_2),
        );
        let world = pose.apply(Point3::new(1.0, 0.0, 0.0));
        assert_point(world, 1.0, 1.0, 0.0);
        assert_point(pose.to_local(world), 1.0, 0.0, 0.0);
    }

    #[test]
    fn nested_pose_matches_applying_parent_after_child() {
        let child = Pose3::new(Point3::new(1.0, 0.0, 0.0), Rotation3::identity());
        let parent = Pose3::new(
            Point3::ORIGIN,
            Rotation3::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), FRAC_PI_2),
        );
        let combined = child.in_parent(parent);
        assert_point(combined.apply(Point3::ORIGIN), 0.0, 1.0, 0.0);
        assert_point(parent.apply(child.apply(Point3::ORIGIN)), 0.0, 1.0, 0.0);
    }

    #[test]
    fn scaled_pose_scales_before_moving() {
        let scaled = ScaledPose3 {
            pose: Pose3::new(Point3::new(1.0, 0.0, 0.0), Rotation3::identity()),
            scale: 2.0,
        };
        assert_point(scaled.apply(Point3::new(1.0, 1.0, 0.0)), 3.0, 2.0, 0.0);
    }
}
