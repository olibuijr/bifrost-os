//! Camera for 3D views (`NativeCamera`, `NativeCameraProjection`): a position, a target and the projection parameters.
//! It builds the view and projection matrices from `linalg`. The orthographic extent is the visible height in
//! world units; the field of view applies to the perspective projection only.
//! It owns no render state.

use crate::linalg::{degrees_to_radians, Matrix4, Vector3};

/// The projection style (`NativeCameraProjection`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Perspective,
    Orthographic,
}

/// A point of view for rendering a 3D scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewCamera {
    pub position: Vector3,
    pub target: Vector3,
    pub up: Vector3,
    pub projection: Projection,
    /// Vertical field of view in degrees, for the perspective projection.
    pub field_of_view: f32,
    /// Visible height in world units, for the orthographic projection.
    pub orthographic_height: f32,
    /// Distances to the near and far clipping planes, both in front of the camera.
    pub near: f32,
    pub far: f32,
}

impl Default for ViewCamera {
    fn default() -> Self {
        ViewCamera {
            position: Vector3::new(0.0, 0.0, 5.0),
            target: Vector3::ZERO,
            up: Vector3::new(0.0, 1.0, 0.0),
            projection: Projection::Perspective,
            field_of_view: 60.0,
            orthographic_height: 2.0,
            near: 0.1,
            far: 100.0,
        }
    }
}

impl ViewCamera {
    /// Points the camera at `target` and keeps its position.
    pub fn look_at(&mut self, target: Vector3) {
        self.target = target;
    }

    /// Moves the camera to `position` and points it at `target` (`NativeCameraLookFrom`).
    pub fn look_from(&mut self, position: Vector3, target: Vector3) {
        self.position = position;
        self.target = target;
    }

    /// Whether the clipping range and the projection parameters are usable.
    pub fn is_valid(&self) -> bool {
        if !(self.near > 0.0 && self.far > self.near) {
            return false;
        }
        match self.projection {
            Projection::Perspective => self.field_of_view > 0.0 && self.field_of_view < 180.0,
            Projection::Orthographic => self.orthographic_height > 0.0,
        }
    }

    /// The world-to-eye transform (`NativeCameraViewMatrix`).
    pub fn view_matrix(&self) -> Matrix4 {
        Matrix4::look_at(self.position, self.target, self.up)
    }

    /// The eye-to-clip transform for a viewport with the given width over height. `None` when the camera is not
    /// valid or the aspect ratio is not positive.
    pub fn projection_matrix(&self, aspect: f32) -> Option<Matrix4> {
        if !self.is_valid() || !(aspect > 0.0) {
            return None;
        }
        match self.projection {
            Projection::Perspective => Some(Matrix4::perspective(
                degrees_to_radians(self.field_of_view),
                aspect,
                self.near,
                self.far,
            )),
            Projection::Orthographic => {
                let half_h = self.orthographic_height * 0.5;
                let half_w = half_h * aspect;
                let (n, f) = (self.near, self.far);
                let mut m = [0.0; 16];
                m[0] = 1.0 / half_w;
                m[5] = 1.0 / half_h;
                m[10] = -2.0 / (f - n);
                m[14] = -(f + n) / (f - n);
                m[15] = 1.0;
                Some(Matrix4 { m })
            }
        }
    }

    /// Projection times view: world space to clip space.
    pub fn view_projection(&self, aspect: f32) -> Option<Matrix4> {
        Some(self.projection_matrix(aspect)? * self.view_matrix())
    }

    /// Moves the camera so that the box from `min` to `max` lies inside the view, looking at its centre, and sets the
    /// clipping range around it (`NativeCameraFrameBoundingBox`). The camera keeps its direction from the centre. A
    /// box with no size leaves the camera unchanged and returns `false`.
    pub fn frame_bounding_box(&mut self, min: Vector3, max: Vector3) -> bool {
        let radius = (max - min).length() * 0.5;
        if !(radius > 0.0) {
            return false;
        }
        let center = (min + max) * 0.5;
        let distance = match self.projection {
            Projection::Perspective => {
                let half = degrees_to_radians(self.field_of_view) * 0.5;
                radius / half.sin()
            }
            Projection::Orthographic => {
                self.orthographic_height = radius * 2.0;
                radius
            }
        };
        let mut dir = (self.position - center).normalized();
        if dir == Vector3::ZERO {
            dir = Vector3::new(0.0, 0.0, 1.0);
        }
        self.position = center + dir * distance;
        self.target = center;
        self.near = (distance - radius).max(radius * 0.01);
        self.far = distance + radius;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-3;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn default_camera_is_valid_and_projects() {
        let cam = ViewCamera::default();
        assert!(cam.is_valid());
        assert!(cam.projection_matrix(16.0 / 9.0).is_some());
    }

    #[test]
    fn invalid_clipping_range_has_no_projection() {
        let mut cam = ViewCamera::default();
        cam.near = 0.0;
        assert!(!cam.is_valid());
        assert_eq!(cam.projection_matrix(1.0), None);
        let mut cam = ViewCamera::default();
        cam.far = cam.near;
        assert_eq!(cam.projection_matrix(1.0), None);
    }

    #[test]
    fn perspective_matrix_entries_for_90_degrees() {
        // fov 90 gives f = 1/tan(45 deg) = 1. near 1, far 3: m10 = 4/(1-3) = -2 and m14 = 2*3*1/(1-3) = -3.
        let mut cam = ViewCamera::default();
        cam.projection = Projection::Perspective;
        cam.field_of_view = 90.0;
        cam.near = 1.0;
        cam.far = 3.0;
        let m = cam.projection_matrix(1.0).unwrap().m;
        assert!(close(m[0], 1.0));
        assert!(close(m[5], 1.0));
        assert!(close(m[10], -2.0));
        assert!(close(m[14], -3.0));
        assert!(close(m[11], -1.0));
    }

    #[test]
    fn orthographic_maps_near_and_far_planes_to_clip_edges() {
        // Height 2 and aspect 2 give half extents 2 (wide) and 1 (tall). Near 1 and far 3: m10 = -1, m14 = -2.
        let mut cam = ViewCamera::default();
        cam.projection = Projection::Orthographic;
        cam.orthographic_height = 2.0;
        cam.near = 1.0;
        cam.far = 3.0;
        let m = cam.projection_matrix(2.0).unwrap().m;
        assert!(close(m[0], 0.5));
        assert!(close(m[5], 1.0));
        assert!(close(m[15], 1.0));
        // Eye-space z of -near must map to -1 and -far to +1.
        assert!(close(m[10] * -1.0 + m[14], -1.0));
        assert!(close(m[10] * -3.0 + m[14], 1.0));
        // A zero height is not a usable orthographic camera.
        cam.orthographic_height = 0.0;
        assert_eq!(cam.projection_matrix(2.0), None);
    }

    #[test]
    fn view_matrix_moves_the_eye_to_the_origin() {
        let mut cam = ViewCamera::default();
        cam.look_from(Vector3::new(0.0, 0.0, 5.0), Vector3::ZERO);
        let eye = cam.view_matrix().transform_point(cam.position).unwrap();
        assert!((eye - Vector3::ZERO).length() < 1e-4);
    }

    #[test]
    fn frame_bounding_box_places_the_camera_by_the_field_of_view() {
        // Box from (0,0,0) to (2,0,0): centre (1,0,0), radius 1. A 90 degree field gives distance 1/sin(45 deg).
        // The camera started on the +z side of the centre, so it stays there.
        let mut cam = ViewCamera::default();
        cam.look_from(Vector3::new(1.0, 0.0, 5.0), Vector3::ZERO);
        cam.field_of_view = 90.0;
        assert!(cam.frame_bounding_box(Vector3::ZERO, Vector3::new(2.0, 0.0, 0.0)));
        let d = std::f32::consts::SQRT_2;
        assert!(close(cam.position.x, 1.0));
        assert!(close(cam.position.z, d));
        assert!(close(cam.target.x, 1.0));
        assert!(close(cam.near, d - 1.0));
        assert!(close(cam.far, d + 1.0));
    }

    #[test]
    fn frame_bounding_box_refuses_an_empty_box() {
        let mut cam = ViewCamera::default();
        let before = cam;
        assert!(!cam.frame_bounding_box(Vector3::new(1.0, 1.0, 1.0), Vector3::new(1.0, 1.0, 1.0)));
        assert_eq!(cam, before);
    }
}
