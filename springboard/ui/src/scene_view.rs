//! A view that shows a `Scene` through one of its cameras, as a pure model of the reference framework's scene
//! renderer queries (`SCNSceneRenderer` `projectPoint`, `unprojectPoint` and `hitTest`). The view holds the size of
//! the area it draws into and the node whose camera it uses as the point of view. It draws nothing: the renderer
//! reads the same matrices, and an app or the shell maps the screen coordinates it gets here onto its own surface.
//!
//! Screen coordinates are in points with the origin at the top left and y growing downward, like the window
//! coordinates used across the SDK. Depth is `0` at the near plane and `1` at the far plane.
//!
//! The camera's own aspect is replaced by the view's width over height, so a horizontal field of view keeps its
//! angle as the window is resized. Hit testing uses a bounding sphere per candidate node, because nodes carry no
//! geometry in this model; a per-triangle test belongs to the renderer.

use crate::linalg::{Matrix4, Vector3, Vector4};
use crate::scene3d::{Camera, NodeId, Scene};

/// A point on the view: screen position in points and normalized depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedPoint {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
}

/// A ray in world space. `direction` is unit length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vector3,
    pub direction: Vector3,
}

/// A node that a hit test found, with the distance along the ray to where the ray enters its bounding sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitResult {
    pub node: NodeId,
    pub distance: f32,
}

/// The view itself: its size and the camera node it looks through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneView {
    width: f32,
    height: f32,
    point_of_view: Option<NodeId>,
}

impl SceneView {
    pub fn new(width: f32, height: f32) -> SceneView {
        SceneView { width, height, point_of_view: None }
    }

    pub fn size(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn set_size(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
    }

    /// The camera node the view looks through. `None` until one is set; the views then project nothing.
    pub fn point_of_view(&self) -> Option<NodeId> {
        self.point_of_view
    }

    pub fn set_point_of_view(&mut self, camera: Option<NodeId>) {
        self.point_of_view = camera;
    }

    /// The projection matrix with the view's aspect, and the camera's view matrix, multiplied together.
    fn view_projection(&self, scene: &Scene) -> Option<Matrix4> {
        let node = self.point_of_view?;
        let camera: Camera = scene.camera(node)?;
        if !(self.width > 0.0 && self.height > 0.0) {
            return None;
        }
        let fitted = Camera { aspect: self.width / self.height, ..camera };
        let view = scene.camera_view(node)?;
        Some(fitted.projection() * view)
    }

    /// Where a world point lands on the view. `None` when there is no point of view, or when the point is on or
    /// behind the camera's eye plane (and so has no screen position).
    pub fn project_point(&self, scene: &Scene, world: Vector3) -> Option<ProjectedPoint> {
        let m = self.view_projection(scene)?;
        let clip = m.transform(world.to_vector4(1.0));
        if clip.w <= 0.0 {
            return None;
        }
        let (nx, ny, nz) = (clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
        Some(ProjectedPoint {
            x: (nx + 1.0) * 0.5 * self.width,
            y: (1.0 - ny) * 0.5 * self.height,
            depth: (nz + 1.0) * 0.5,
        })
    }

    /// The world point at a screen position and depth. `None` when there is no point of view or the view-projection
    /// is singular.
    pub fn unproject_point(&self, scene: &Scene, point: ProjectedPoint) -> Option<Vector3> {
        let m = self.view_projection(scene)?;
        let inverse = m.inverse()?;
        let nx = point.x / self.width * 2.0 - 1.0;
        let ny = 1.0 - point.y / self.height * 2.0;
        let nz = point.depth * 2.0 - 1.0;
        let v: Vector4 = inverse.transform(Vector4::new(nx, ny, nz, 1.0));
        if v.w == 0.0 {
            return None;
        }
        Some(v.xyz() * (1.0 / v.w))
    }

    /// The ray from the near plane through a screen position.
    pub fn ray(&self, scene: &Scene, x: f32, y: f32) -> Option<Ray> {
        let near = self.unproject_point(scene, ProjectedPoint { x, y, depth: 0.0 })?;
        let far = self.unproject_point(scene, ProjectedPoint { x, y, depth: 1.0 })?;
        let delta = far - near;
        if delta.length() < 1e-9 {
            return None;
        }
        Some(Ray { origin: near, direction: delta.normalized() })
    }

    /// The candidate nodes whose bounding spheres the ray through a screen position passes through, nearest first.
    /// Each candidate is a node with its bounding radius in world units. Nodes that do not exist are skipped.
    pub fn hit_test(&self, scene: &Scene, x: f32, y: f32, candidates: &[(NodeId, f32)]) -> Vec<HitResult> {
        let Some(ray) = self.ray(scene, x, y) else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        for &(node, radius) in candidates {
            let Some(center) = scene.world_position(node) else {
                continue;
            };
            let radius = radius.max(0.0);
            let to_center = center - ray.origin;
            let along = to_center.dot(ray.direction);
            let perpendicular_sq = to_center.dot(to_center) - along * along;
            if perpendicular_sq > radius * radius {
                continue;
            }
            let half_chord = (radius * radius - perpendicular_sq.max(0.0)).sqrt();
            let exit = along + half_chord;
            if exit < 0.0 {
                continue;
            }
            let enter = (along - half_chord).max(0.0);
            hits.push(HitResult { node, distance: enter });
        }
        hits.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::Pose;

    fn v(x: f32, y: f32, z: f32) -> Vector3 {
        Vector3::new(x, y, z)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// A scene with a camera five units down +Z, looking down -Z, and a view of 800 by 600 points.
    fn setup() -> (Scene, NodeId, SceneView) {
        let mut scene = Scene::new();
        let cam = scene.add_child(scene.root(), "camera").unwrap();
        scene.set_camera(cam, Some(Camera::new(60.0, 800.0 / 600.0, 0.1, 100.0)));
        let mut pose = Pose::identity();
        pose.position = v(0.0, 0.0, 5.0);
        scene.set_pose(cam, pose);
        let mut view = SceneView::new(800.0, 600.0);
        view.set_point_of_view(Some(cam));
        (scene, cam, view)
    }

    #[test]
    fn no_point_of_view_projects_nothing() {
        let (scene, _cam, _) = setup();
        let view = SceneView::new(800.0, 600.0);
        assert_eq!(view.point_of_view(), None);
        assert!(view.project_point(&scene, v(0.0, 0.0, 0.0)).is_none());
        assert!(view.ray(&scene, 400.0, 300.0).is_none());
    }

    #[test]
    fn the_origin_projects_to_the_centre_of_the_view() {
        let (scene, _cam, view) = setup();
        let p = view.project_point(&scene, v(0.0, 0.0, 0.0)).unwrap();
        assert!(close(p.x, 400.0), "{:?}", p);
        assert!(close(p.y, 300.0), "{:?}", p);
        assert!(p.depth > 0.0 && p.depth < 1.0);
    }

    #[test]
    fn screen_y_grows_downward_and_x_grows_rightward() {
        let (scene, _cam, view) = setup();
        let right = view.project_point(&scene, v(1.0, 0.0, 0.0)).unwrap();
        let up = view.project_point(&scene, v(0.0, 1.0, 0.0)).unwrap();
        assert!(right.x > 400.0);
        assert!(up.y < 300.0);
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        let (scene, _cam, view) = setup();
        assert!(view.project_point(&scene, v(0.0, 0.0, 10.0)).is_none());
    }

    #[test]
    fn unproject_inverts_project() {
        let (scene, _cam, view) = setup();
        let world = v(0.7, -0.4, 0.2);
        let screen = view.project_point(&scene, world).unwrap();
        let back = view.unproject_point(&scene, screen).unwrap();
        assert!(close(back.x, world.x) && close(back.y, world.y) && close(back.z, world.z), "{:?}", back);
    }

    #[test]
    fn the_ray_through_the_centre_points_down_the_view_axis() {
        let (scene, _cam, view) = setup();
        let ray = view.ray(&scene, 400.0, 300.0).unwrap();
        assert!(close(ray.direction.x, 0.0) && close(ray.direction.y, 0.0));
        assert!(close(ray.direction.z, -1.0));
        assert!((ray.direction.length() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn hit_test_finds_a_sphere_under_the_centre_and_misses_one_off_to_the_side() {
        let (mut scene, _cam, view) = setup();
        let root = scene.root();
        let on_axis = scene.add_child(root, "on-axis").unwrap();
        let mut p = Pose::identity();
        p.position = v(0.0, 0.0, 0.0);
        scene.set_pose(on_axis, p);
        let aside = scene.add_child(root, "aside").unwrap();
        let mut q = Pose::identity();
        q.position = v(5.0, 0.0, 0.0);
        scene.set_pose(aside, q);

        let hits = view.hit_test(&scene, 400.0, 300.0, &[(on_axis, 1.0), (aside, 1.0)]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node, on_axis);
        // The ray starts on the near plane (z = 4.9, the camera at z = 5 less z_near 0.1) and the sphere's front
        // is at z = 1, so the ray enters after 3.9 units.
        assert!(close(hits[0].distance, 3.9), "{:?}", hits[0]);
    }

    #[test]
    fn hit_test_orders_results_nearest_first() {
        let (mut scene, _cam, view) = setup();
        let root = scene.root();
        let near = scene.add_child(root, "near").unwrap();
        let far = scene.add_child(root, "far").unwrap();
        let mut a = Pose::identity();
        a.position = v(0.0, 0.0, 2.0);
        scene.set_pose(near, a);
        let mut b = Pose::identity();
        b.position = v(0.0, 0.0, -2.0);
        scene.set_pose(far, b);

        let hits = view.hit_test(&scene, 400.0, 300.0, &[(far, 0.5), (near, 0.5)]);
        let order: Vec<NodeId> = hits.iter().map(|h| h.node).collect();
        assert_eq!(order, vec![near, far]);
    }

    #[test]
    fn a_zero_sized_view_projects_nothing() {
        let (scene, cam, _) = setup();
        let mut view = SceneView::new(0.0, 0.0);
        view.set_point_of_view(Some(cam));
        assert!(view.project_point(&scene, v(0.0, 0.0, 0.0)).is_none());
    }
}
