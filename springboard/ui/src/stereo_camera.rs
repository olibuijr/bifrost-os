//! Stereoscopic cameras: a left and a right eye made from one `ViewCamera`. Each eye sits half the eye separation to
//! either side of the source camera, along its right axis, and looks in the same direction. Both eyes keep the
//! source projection, so each one renders with its own view and projection matrices. The renderer draws the two
//! images; this module only places the cameras. Composing the images (side by side, anaglyph) is left to the caller.

use crate::linalg::Vector3;
use crate::view_camera::ViewCamera;

/// The left and right eye cameras for one source camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyePair {
    pub left: ViewCamera,
    pub right: ViewCamera,
}

/// Splits `camera` into a left and a right eye. `separation` is the distance between the two eye positions in world
/// units. Returns `None` when the camera is not valid, the separation is negative or not finite, or the camera looks
/// along its own up axis (so the right axis is undefined).
pub fn eye_pair(camera: &ViewCamera, separation: f32) -> Option<EyePair> {
    if !camera.is_valid() || !separation.is_finite() || separation < 0.0 {
        return None;
    }
    let forward = (camera.target - camera.position).normalized();
    if forward.length() == 0.0 {
        return None;
    }
    let right = forward.cross(camera.up).normalized();
    if right.length() == 0.0 {
        return None;
    }
    let half = right * (separation * 0.5);
    Some(EyePair { left: shifted(camera, -half), right: shifted(camera, half) })
}

/// The eye camera at `offset` from the source camera. The target moves by the same offset, so the direction of view
/// does not change.
fn shifted(camera: &ViewCamera, offset: Vector3) -> ViewCamera {
    let mut eye = *camera;
    eye.position = camera.position + offset;
    eye.target = camera.target + offset;
    eye
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_camera::Projection;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn camera() -> ViewCamera {
        let mut cam = ViewCamera::default();
        cam.up = Vector3::new(0.0, 1.0, 0.0);
        cam.look_from(Vector3::new(0.0, 0.0, 5.0), Vector3::new(0.0, 0.0, 0.0));
        cam
    }

    #[test]
    fn eyes_sit_half_the_separation_either_side_along_the_right_axis() {
        let pair = eye_pair(&camera(), 0.2).expect("valid camera");
        // Looking down -z with +y up, the right axis is +x.
        assert!(close(pair.left.position.x, -0.1));
        assert!(close(pair.right.position.x, 0.1));
        assert!(close(pair.left.position.z, 5.0));
        assert!(close(pair.right.position.z, 5.0));
    }

    #[test]
    fn each_eye_looks_in_the_same_direction_as_the_source() {
        let source = camera();
        let pair = eye_pair(&source, 0.3).expect("valid camera");
        let source_dir = (source.target - source.position).normalized();
        for eye in [pair.left, pair.right] {
            let dir = (eye.target - eye.position).normalized();
            assert!(close(dir.x, source_dir.x) && close(dir.y, source_dir.y) && close(dir.z, source_dir.z));
        }
    }

    #[test]
    fn eyes_keep_the_source_projection() {
        let mut source = camera();
        source.projection = Projection::Orthographic;
        source.orthographic_height = 4.0;
        let pair = eye_pair(&source, 0.1).expect("valid camera");
        assert_eq!(pair.left.projection, Projection::Orthographic);
        assert_eq!(pair.right.orthographic_height, 4.0);
        assert_eq!(pair.left.near, source.near);
        assert_eq!(pair.right.far, source.far);
    }

    #[test]
    fn zero_separation_gives_two_identical_eyes() {
        let source = camera();
        let pair = eye_pair(&source, 0.0).expect("valid camera");
        assert_eq!(pair.left, source);
        assert_eq!(pair.right, source);
    }

    #[test]
    fn the_two_eyes_have_different_view_matrices() {
        let pair = eye_pair(&camera(), 0.5).expect("valid camera");
        assert_ne!(pair.left.view_matrix(), pair.right.view_matrix());
    }

    #[test]
    fn invalid_input_gives_no_pair() {
        assert!(eye_pair(&camera(), -1.0).is_none());
        assert!(eye_pair(&camera(), f32::NAN).is_none());
        let mut looking_up = camera();
        looking_up.look_from(Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 5.0, 0.0));
        assert!(eye_pair(&looking_up, 0.1).is_none());
        let mut bad = camera();
        bad.near = 0.0;
        assert!(eye_pair(&bad, 0.1).is_none());
    }
}
