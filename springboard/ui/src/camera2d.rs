//! Two-dimensional camera: the point of view on a scene. It has a centre in scene points, a zoom factor and a rotation
//! in radians. The centre of the view shows the camera's position. Zoom and rotation turn the view, not the scene, so
//! the scene's own coordinates never change.
//!
//! Pure: no clock and no drawing. It maps points both ways and gives the part of the scene the view shows, so the
//! renderer can cull nodes outside it. Rotation is counter-clockwise in the scene's Y-up coordinates, and the camera
//! turns the view the opposite way, as a camera does.
//!
//! Rules: a non-finite position or rotation falls back to 0. A zoom that is not a positive finite number falls back to
//! 1.

/// A camera over a two-dimensional scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera2d {
    x: f32,
    y: f32,
    scale: f32,
    rotation: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

impl Camera2d {
    /// A camera centred on the scene point (`x`, `y`), with zoom 1 and no rotation.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: finite_or(x, 0.0),
            y: finite_or(y, 0.0),
            scale: 1.0,
            rotation: 0.0,
        }
    }

    /// The scene point the camera is centred on.
    pub fn position(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    /// Moves the camera to the scene point (`x`, `y`).
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = finite_or(x, self.x);
        self.y = finite_or(y, self.y);
    }

    /// The zoom factor: above 1 the scene looks bigger.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Sets the zoom factor. A value that is not a positive finite number is ignored.
    pub fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
        }
    }

    /// The rotation in radians.
    pub fn rotation(&self) -> f32 {
        self.rotation
    }

    /// Sets the rotation in radians. A non-finite value is ignored.
    pub fn set_rotation(&mut self, radians: f32) {
        self.rotation = finite_or(radians, self.rotation);
    }

    /// Maps a scene point to a point in a view of `view_width` by `view_height` points, origin at the bottom-left.
    pub fn scene_to_view(&self, view_width: f32, view_height: f32, point: (f32, f32)) -> (f32, f32) {
        let dx = point.0 - self.x;
        let dy = point.1 - self.y;
        let (sin, cos) = self.rotation.sin_cos();
        let rx = dx * cos + dy * sin;
        let ry = -dx * sin + dy * cos;
        (
            view_width / 2.0 + rx * self.scale,
            view_height / 2.0 + ry * self.scale,
        )
    }

    /// Maps a view point back to the scene. It is the inverse of [`Camera2d::scene_to_view`].
    pub fn view_to_scene(&self, view_width: f32, view_height: f32, point: (f32, f32)) -> (f32, f32) {
        let x = (point.0 - view_width / 2.0) / self.scale;
        let y = (point.1 - view_height / 2.0) / self.scale;
        let (sin, cos) = self.rotation.sin_cos();
        (self.x + cos * x - sin * y, self.y + sin * x + cos * y)
    }

    /// The smallest axis-aligned box of scene points the view shows, as (min x, min y, max x, max y). With rotation
    /// this box is larger than the visible area; it is a safe bound for culling.
    pub fn visible_bounds(&self, view_width: f32, view_height: f32) -> (f32, f32, f32, f32) {
        let corners = [
            self.view_to_scene(view_width, view_height, (0.0, 0.0)),
            self.view_to_scene(view_width, view_height, (view_width, 0.0)),
            self.view_to_scene(view_width, view_height, (0.0, view_height)),
            self.view_to_scene(view_width, view_height, (view_width, view_height)),
        ];
        let mut bounds = (corners[0].0, corners[0].1, corners[0].0, corners[0].1);
        for (x, y) in corners.iter().skip(1) {
            bounds.0 = bounds.0.min(*x);
            bounds.1 = bounds.1.min(*y);
            bounds.2 = bounds.2.max(*x);
            bounds.3 = bounds.3.max(*y);
        }
        bounds
    }

    /// Whether a scene point lies inside the view.
    pub fn contains(&self, view_width: f32, view_height: f32, point: (f32, f32)) -> bool {
        let (x, y) = self.scene_to_view(view_width, view_height, point);
        (0.0..=view_width).contains(&x) && (0.0..=view_height).contains(&y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn the_camera_centre_maps_to_the_view_centre() {
        let cam = Camera2d::new(500.0, 300.0);
        assert!(close(cam.scene_to_view(800.0, 600.0, (500.0, 300.0)), (400.0, 300.0)));
    }

    #[test]
    fn zoom_scales_distances_from_the_centre() {
        let mut cam = Camera2d::new(0.0, 0.0);
        cam.set_scale(2.0);
        assert!(close(cam.scene_to_view(100.0, 100.0, (10.0, -5.0)), (70.0, 40.0)));
    }

    #[test]
    fn invalid_zoom_is_ignored() {
        let mut cam = Camera2d::new(0.0, 0.0);
        cam.set_scale(0.0);
        cam.set_scale(-3.0);
        cam.set_scale(f32::NAN);
        assert_eq!(cam.scale(), 1.0);
    }

    #[test]
    fn view_to_scene_inverts_scene_to_view_with_rotation() {
        let mut cam = Camera2d::new(40.0, -20.0);
        cam.set_scale(1.5);
        cam.set_rotation(0.7);
        let p = (13.0, 91.0);
        let v = cam.scene_to_view(640.0, 480.0, p);
        assert!(close(cam.view_to_scene(640.0, 480.0, v), p));
    }

    #[test]
    fn a_quarter_turn_moves_the_point_along_the_view_x_axis() {
        let mut cam = Camera2d::new(0.0, 0.0);
        cam.set_rotation(std::f32::consts::FRAC_PI_2);
        assert!(close(cam.scene_to_view(100.0, 100.0, (0.0, 10.0)), (60.0, 50.0)));
    }

    #[test]
    fn visible_bounds_of_an_unrotated_unzoomed_camera_are_the_view_rect() {
        let cam = Camera2d::new(0.0, 0.0);
        assert_eq!(cam.visible_bounds(100.0, 50.0), (-50.0, -25.0, 50.0, 25.0));
    }

    #[test]
    fn contains_reports_points_inside_and_outside_the_view() {
        let cam = Camera2d::new(0.0, 0.0);
        assert!(cam.contains(100.0, 100.0, (0.0, 0.0)));
        assert!(cam.contains(100.0, 100.0, (49.0, -49.0)));
        assert!(!cam.contains(100.0, 100.0, (51.0, 0.0)));
    }

    #[test]
    fn non_finite_position_keeps_the_previous_value() {
        let mut cam = Camera2d::new(3.0, 4.0);
        cam.set_position(f32::INFINITY, 9.0);
        assert_eq!(cam.position(), (3.0, 9.0));
    }
}
