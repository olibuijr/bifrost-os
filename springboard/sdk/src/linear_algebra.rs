//! Vector, matrix and quaternion types for apps (GLKit's `GLKVector3`, `GLKMatrix4` and `GLKQuaternion`): the math in
//! `bifrost_ui::linalg`, re-exported so an app takes one import. Nothing here touches GPU memory; a GPU upload is a
//! separate API.

pub use bifrost_ui::linalg::{degrees_to_radians, radians_to_degrees, Matrix4, Quaternion, Vector3, Vector4};

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_translation_moves_a_point_and_the_identity_keeps_it() {
        let p = Vector3::new(1.0, 2.0, 3.0);
        let moved = Matrix4::translation(10.0, 0.0, -1.0).transform_point(p).expect("a translation is invertible");
        assert!(close(moved.x, 11.0) && close(moved.y, 2.0) && close(moved.z, 2.0));
        assert_eq!(Matrix4::identity().transform_point(p), Some(p));
    }

    #[test]
    fn a_quarter_turn_about_z_maps_x_onto_y() {
        let q = Quaternion::from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f32::consts::FRAC_PI_2);
        let r = q.rotate(Vector3::new(1.0, 0.0, 0.0));
        assert!(close(r.x, 0.0) && close(r.y, 1.0) && close(r.z, 0.0));
        assert!(close(degrees_to_radians(180.0), std::f32::consts::PI));
    }

    #[test]
    fn the_quaternion_matrix_agrees_with_the_quaternion_rotation() {
        let q = Quaternion::from_axis_angle(Vector3::new(0.0, 1.0, 0.0), std::f32::consts::FRAC_PI_2);
        let via_matrix = q.to_matrix4().transform_point(Vector3::new(0.0, 0.0, 1.0)).expect("a rotation is invertible");
        let direct = q.rotate(Vector3::new(0.0, 0.0, 1.0));
        assert!(close(via_matrix.x, direct.x) && close(via_matrix.y, direct.y) && close(via_matrix.z, direct.z));
    }
}
