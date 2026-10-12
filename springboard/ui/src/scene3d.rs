//! Scene tree for 3D content: nodes with a position, an orientation and a scale relative to their parent, a camera
//! attachment that yields a view and a projection matrix, and a look-at constraint that turns a node toward another
//! node. Pure maths on the `linalg` types: nothing is drawn here, the renderer reads world poses and matrices.
//! Nodes live in one arena and refer to each other by `NodeId`; nodes are added but never removed, so a handle stays
//! valid. Local space follows `linalg::Matrix4::look_at`: a node looks down its local -Z axis with +Y up.
//! Skew is not represented. Scale composes per axis, so a non-uniform scale under a rotated parent is approximate.

use crate::linalg::{degrees_to_radians, Matrix4, Quaternion, Vector3};

/// Handle of a node in a `Scene`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(usize);

/// Position, orientation and scale of a node, relative to its parent (or to the world for a child of the root).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: Vector3,
    pub orientation: Quaternion,
    pub scale: Vector3,
}

impl Pose {
    pub fn identity() -> Pose {
        Pose { position: Vector3::ZERO, orientation: Quaternion::identity(), scale: Vector3::new(1.0, 1.0, 1.0) }
    }

    /// Places `child`, given in this pose's local space, into the space this pose lives in.
    pub fn compose(self, child: Pose) -> Pose {
        let offset = Vector3::new(
            child.position.x * self.scale.x,
            child.position.y * self.scale.y,
            child.position.z * self.scale.z,
        );
        Pose {
            position: self.position + self.orientation.rotate(offset),
            orientation: (self.orientation * child.orientation).normalized(),
            scale: Vector3::new(self.scale.x * child.scale.x, self.scale.y * child.scale.y, self.scale.z * child.scale.z),
        }
    }

    /// The matrix that maps local points into the parent space: scale, then rotation, then translation.
    pub fn matrix(self) -> Matrix4 {
        let p = self.position;
        let s = self.scale;
        Matrix4::translation(p.x, p.y, p.z) * self.orientation.to_matrix4() * Matrix4::scaling(s.x, s.y, s.z)
    }
}

/// A constraint that overrides the orientation of a node each time the scene is resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constraint {
    /// Turns the node so its front (local -Z) axis points at `target`, keeping `world_up` as the up direction.
    LookAt { target: NodeId, world_up: Vector3 },
}

/// Which axis the field of view is measured along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProjectionDirection {
    Horizontal,
    Vertical,
}

/// A perspective camera attached to a node. The camera looks down its node's local -Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Viewing angle in degrees, measured along `direction`.
    pub field_of_view: f32,
    pub direction: ProjectionDirection,
    /// Width over height of the image.
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Camera {
    /// A camera whose field of view is measured horizontally.
    pub fn new(field_of_view: f32, aspect: f32, z_near: f32, z_far: f32) -> Camera {
        Camera { field_of_view, direction: ProjectionDirection::Horizontal, aspect, z_near, z_far }
    }

    /// The vertical viewing angle in radians, converted from the configured direction.
    pub fn vertical_fov_radians(&self) -> f32 {
        let half = degrees_to_radians(self.field_of_view) * 0.5;
        match self.direction {
            ProjectionDirection::Vertical => 2.0 * half,
            ProjectionDirection::Horizontal => 2.0 * (half.tan() / self.aspect.max(f32::EPSILON)).atan(),
        }
    }

    /// The perspective projection for this camera.
    pub fn projection(&self) -> Matrix4 {
        Matrix4::perspective(self.vertical_fov_radians(), self.aspect, self.z_near, self.z_far)
    }
}

/// The rotation whose local X, Y and Z axes point along the given directions (the columns of a rotation matrix).
/// The directions must be unit length and at right angles.
pub fn quaternion_from_axes(x: Vector3, y: Vector3, z: Vector3) -> Quaternion {
    // Matrix entries, row then column: m01 is row 0, column 1.
    let (m00, m01, m02) = (x.x, y.x, z.x);
    let (m10, m11, m12) = (x.y, y.y, z.y);
    let (m20, m21, m22) = (x.z, y.z, z.z);
    let trace = m00 + m11 + m22;
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        Quaternion::new((m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, s * 0.25)
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        Quaternion::new(s * 0.25, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s)
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        Quaternion::new((m01 + m10) / s, s * 0.25, (m12 + m21) / s, (m02 - m20) / s)
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        Quaternion::new((m02 + m20) / s, (m12 + m21) / s, s * 0.25, (m10 - m01) / s)
    }
}

/// The orientation that points the front (local -Z) axis from `eye` at `target`, with `up` kept as the up direction.
/// `None` when the two points coincide or when `up` is parallel to the view direction.
pub fn look_at_orientation(eye: Vector3, target: Vector3, up: Vector3) -> Option<Quaternion> {
    let f = (target - eye).normalized();
    if f.length() < 1e-6 {
        return None;
    }
    let s = f.cross(up).normalized();
    if s.length() < 1e-6 {
        return None;
    }
    let u = s.cross(f);
    Some(quaternion_from_axes(s, u, -f))
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    pose: Pose,
    constraint: Option<Constraint>,
    camera: Option<Camera>,
}

impl Node {
    fn new(name: &str, parent: Option<NodeId>) -> Node {
        Node {
            name: name.to_string(),
            parent,
            children: Vec::new(),
            pose: Pose::identity(),
            constraint: None,
            camera: None,
        }
    }
}

/// A tree of nodes. The root node (`root()`) is the world; every other node is relative to its parent.
#[derive(Clone, Debug)]
pub struct Scene {
    nodes: Vec<Node>,
}

impl Scene {
    pub fn new() -> Scene {
        Scene { nodes: vec![Node::new("root", None)] }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Adds a child under `parent`. `None` when `parent` is not in this scene.
    pub fn add_child(&mut self, parent: NodeId, name: &str) -> Option<NodeId> {
        if parent.0 >= self.nodes.len() {
            return None;
        }
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node::new(name, Some(parent)));
        self.nodes[parent.0].children.push(id);
        Some(id)
    }

    /// The first node with this name, in the order the nodes were added.
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.name == name).map(NodeId)
    }

    pub fn name(&self, id: NodeId) -> Option<&str> {
        self.nodes.get(id.0).map(|n| n.name.as_str())
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(id.0).and_then(|n| n.parent)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id.0).map(|n| n.children.as_slice()).unwrap_or(&[])
    }

    /// The pose of a node relative to its parent, before constraints.
    pub fn local_pose(&self, id: NodeId) -> Option<Pose> {
        self.nodes.get(id.0).map(|n| n.pose)
    }

    pub fn set_pose(&mut self, id: NodeId, pose: Pose) -> bool {
        match self.nodes.get_mut(id.0) {
            Some(n) => {
                n.pose = pose;
                true
            }
            None => false,
        }
    }

    pub fn set_constraint(&mut self, id: NodeId, constraint: Option<Constraint>) -> bool {
        match self.nodes.get_mut(id.0) {
            Some(n) => {
                n.constraint = constraint;
                true
            }
            None => false,
        }
    }

    pub fn set_camera(&mut self, id: NodeId, camera: Option<Camera>) -> bool {
        match self.nodes.get_mut(id.0) {
            Some(n) => {
                n.camera = camera;
                true
            }
            None => false,
        }
    }

    pub fn camera(&self, id: NodeId) -> Option<Camera> {
        self.nodes.get(id.0).and_then(|n| n.camera)
    }

    /// The pose of a node in world space, with every ancestor's pose and constraint applied.
    pub fn world_pose(&self, id: NodeId) -> Option<Pose> {
        if id.0 < self.nodes.len() {
            Some(self.resolve(id, 0))
        } else {
            None
        }
    }

    pub fn world_position(&self, id: NodeId) -> Option<Vector3> {
        self.world_pose(id).map(|p| p.position)
    }

    /// The matrix that maps the node's local points into world space.
    pub fn world_matrix(&self, id: NodeId) -> Option<Matrix4> {
        self.world_pose(id).map(|p| p.matrix())
    }

    /// The view matrix of a camera node: the inverse of its world matrix. `None` when the node has no camera.
    pub fn camera_view(&self, id: NodeId) -> Option<Matrix4> {
        self.camera(id)?;
        self.world_matrix(id)?.inverse()
    }

    /// Resolves a node's world pose. Constraint targets are resolved in turn, so a cycle of constraints stops
    /// applying once the recursion is deeper than the node count.
    fn resolve(&self, id: NodeId, depth: usize) -> Pose {
        let node = &self.nodes[id.0];
        let parent = match node.parent {
            Some(p) => self.resolve(p, depth + 1),
            None => Pose::identity(),
        };
        let mut local = node.pose;
        if let Some(Constraint::LookAt { target, world_up }) = node.constraint {
            if depth <= self.nodes.len() && target.0 < self.nodes.len() {
                let eye = parent.compose(local).position;
                let aim = self.resolve(target, depth + 1).position;
                if let Some(world) = look_at_orientation(eye, aim, world_up) {
                    local.orientation = parent.orientation.normalized().conjugate() * world;
                }
            }
        }
        parent.compose(local)
    }
}

impl Default for Scene {
    fn default() -> Self {
        Scene::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn v(x: f32, y: f32, z: f32) -> Vector3 {
        Vector3::new(x, y, z)
    }

    fn near(a: Vector3, b: Vector3) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4
    }

    fn at(position: Vector3) -> Pose {
        Pose { position, ..Pose::identity() }
    }

    #[test]
    fn child_offsets_add_to_the_parent_position() {
        let mut s = Scene::new();
        let root = s.root();
        let a = s.add_child(root, "a").unwrap();
        s.set_pose(a, at(v(10.0, 0.0, 0.0)));
        let b = s.add_child(a, "b").unwrap();
        s.set_pose(b, at(v(0.0, 5.0, 0.0)));
        assert!(near(s.world_position(b).unwrap(), v(10.0, 5.0, 0.0)));
    }

    #[test]
    fn parent_scale_multiplies_the_child_offset() {
        let mut s = Scene::new();
        let root = s.root();
        let a = s.add_child(root, "a").unwrap();
        s.set_pose(a, Pose { scale: v(2.0, 2.0, 2.0), ..Pose::identity() });
        let b = s.add_child(a, "b").unwrap();
        s.set_pose(b, at(v(1.0, 0.0, 0.0)));
        assert!(near(s.world_position(b).unwrap(), v(2.0, 0.0, 0.0)));
        // The parent scale also reaches the child's own matrix: a unit step along X inside b is doubled and then
        // offset by b's world position (2, 0, 0), so the local point (1, 0, 0) lands at (4, 0, 0).
        let p = s.world_matrix(b).unwrap().transform_point(v(1.0, 0.0, 0.0)).unwrap();
        assert!(near(p, v(4.0, 0.0, 0.0)));
    }

    #[test]
    fn parent_rotation_turns_the_child_offset() {
        let mut s = Scene::new();
        let root = s.root();
        let a = s.add_child(root, "a").unwrap();
        let turn = Quaternion::from_axis_angle(v(0.0, 0.0, 1.0), FRAC_PI_2);
        s.set_pose(a, Pose { orientation: turn, ..Pose::identity() });
        let b = s.add_child(a, "b").unwrap();
        s.set_pose(b, at(v(1.0, 0.0, 0.0)));
        assert!(near(s.world_position(b).unwrap(), v(0.0, 1.0, 0.0)));
    }

    #[test]
    fn nested_rotations_compose_parent_last() {
        // c sits at (1, 0, 0) in b. b turns 90 degrees about Y, then a turns 90 degrees about Z.
        // (1, 0, 0) turned about Y becomes (0, 0, -1), which is unchanged by the turn about Z.
        let mut s = Scene::new();
        let root = s.root();
        let a = s.add_child(root, "a").unwrap();
        s.set_pose(a, Pose { orientation: Quaternion::from_axis_angle(v(0.0, 0.0, 1.0), FRAC_PI_2), ..Pose::identity() });
        let b = s.add_child(a, "b").unwrap();
        s.set_pose(b, Pose { orientation: Quaternion::from_axis_angle(v(0.0, 1.0, 0.0), FRAC_PI_2), ..Pose::identity() });
        let c = s.add_child(b, "c").unwrap();
        s.set_pose(c, at(v(1.0, 0.0, 0.0)));
        assert!(near(s.world_position(c).unwrap(), v(0.0, 0.0, -1.0)));
    }

    #[test]
    fn look_at_turns_the_front_axis_toward_the_target() {
        let mut s = Scene::new();
        let root = s.root();
        let n = s.add_child(root, "n").unwrap();
        let t = s.add_child(root, "t").unwrap();
        s.set_pose(t, at(v(10.0, 0.0, 0.0)));
        s.set_constraint(n, Some(Constraint::LookAt { target: t, world_up: v(0.0, 1.0, 0.0) }));
        let q = s.world_pose(n).unwrap().orientation;
        assert!(near(q.rotate(v(0.0, 0.0, -1.0)), v(1.0, 0.0, 0.0)));
        // The up direction stays up.
        assert!(near(q.rotate(v(0.0, 1.0, 0.0)), v(0.0, 1.0, 0.0)));
    }

    #[test]
    fn look_at_accounts_for_the_parent_rotation() {
        // The parent turns 90 degrees about Z. The constrained child must still face world +X.
        let mut s = Scene::new();
        let root = s.root();
        let p = s.add_child(root, "p").unwrap();
        s.set_pose(p, Pose { orientation: Quaternion::from_axis_angle(v(0.0, 0.0, 1.0), FRAC_PI_2), ..Pose::identity() });
        let t = s.add_child(root, "t").unwrap();
        s.set_pose(t, at(v(10.0, 0.0, 0.0)));
        let c = s.add_child(p, "c").unwrap();
        s.set_constraint(c, Some(Constraint::LookAt { target: t, world_up: v(0.0, 1.0, 0.0) }));
        let q = s.world_pose(c).unwrap().orientation;
        assert!(near(q.rotate(v(0.0, 0.0, -1.0)), v(1.0, 0.0, 0.0)));
        assert!(near(q.rotate(v(0.0, 1.0, 0.0)), v(0.0, 1.0, 0.0)));
    }

    #[test]
    fn look_at_leaves_the_orientation_alone_when_target_coincides() {
        let mut s = Scene::new();
        let root = s.root();
        let n = s.add_child(root, "n").unwrap();
        let t = s.add_child(root, "t").unwrap();
        s.set_constraint(n, Some(Constraint::LookAt { target: t, world_up: v(0.0, 1.0, 0.0) }));
        let q = s.world_pose(n).unwrap().orientation;
        assert!(near(q.rotate(v(0.0, 0.0, -1.0)), v(0.0, 0.0, -1.0)));
    }

    #[test]
    fn horizontal_field_of_view_gives_the_vertical_focal_length() {
        // A 90 degree horizontal angle at aspect 2 gives tan(vertical half angle) = tan(45) / 2 = 0.5, so the
        // vertical focal term is 2 and the horizontal term is 2 / aspect = 1.
        let c = Camera::new(90.0, 2.0, 0.1, 100.0);
        let p = c.projection();
        assert!((p.m[5] - 2.0).abs() < 1e-4);
        assert!((p.m[0] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn vertical_direction_is_used_as_given() {
        let c = Camera { field_of_view: 60.0, direction: ProjectionDirection::Vertical, aspect: 1.5, z_near: 0.1, z_far: 50.0 };
        assert!((c.vertical_fov_radians() - std::f32::consts::FRAC_PI_3).abs() < 1e-5);
        let h = Camera::new(90.0, 1.0, 0.1, 50.0);
        assert!((h.vertical_fov_radians() - FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn camera_view_moves_the_world_into_camera_space() {
        // A camera 10 units up the +Z axis sees the origin 10 units in front of it, at -Z.
        let mut s = Scene::new();
        let root = s.root();
        let cam = s.add_child(root, "cam").unwrap();
        s.set_pose(cam, at(v(0.0, 0.0, 10.0)));
        s.set_camera(cam, Some(Camera::new(60.0, 1.5, 0.1, 50.0)));
        let view = s.camera_view(cam).unwrap();
        let p = view.transform_point(v(0.0, 0.0, 0.0)).unwrap();
        assert!(near(p, v(0.0, 0.0, -10.0)));
        // A node without a camera has no view.
        assert!(s.camera_view(root).is_none());
    }
}
