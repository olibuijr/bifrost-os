//! Object hierarchy for 3D assets (`NativeObjectContainer`, `NativeObject`, `NativeNamed`, `NativeTransform`).
//! A scene is an arena of named nodes. Each node has an ordered list of transform operations and a hidden flag.
//! Nodes are added under a parent and never re-parented, so the hierarchy stays acyclic.
//! This module holds the hierarchy and the transform maths only; it owns no mesh or GPU data.

use crate::linalg::{Matrix4, Vector3};

/// Identifies a node in one `SceneObjects` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(pub usize);

/// One operation of a local transform (`NativeTransformOperation` and its translate, rotate, scale and matrix forms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOp {
    Translate(Vector3),
    /// Rotation by `radians` about `axis`.
    Rotate { radians: f32, axis: Vector3 },
    Scale(Vector3),
    Matrix(Matrix4),
}

impl TransformOp {
    fn matrix(self) -> Matrix4 {
        match self {
            TransformOp::Translate(v) => Matrix4::translation(v.x, v.y, v.z),
            TransformOp::Rotate { radians, axis } => Matrix4::rotation(radians, axis),
            TransformOp::Scale(v) => Matrix4::scaling(v.x, v.y, v.z),
            TransformOp::Matrix(m) => m,
        }
    }
}

/// One named node of the hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub name: String,
    /// A hidden node, and everything under it, is left out of rendering.
    pub hidden: bool,
    /// The local transform. The first operation is the outermost one: `[Translate, Scale]` moves a point by the
    /// scale first and then by the translation.
    pub ops: Vec<TransformOp>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

/// An arena of named nodes with a parent and child structure (`NativeObjectContainer`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneObjects {
    nodes: Vec<SceneNode>,
    roots: Vec<NodeId>,
}

/// A name is usable in a path when it is not empty and holds no `/`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/')
}

impl SceneObjects {
    pub fn new() -> Self {
        SceneObjects::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, id: NodeId) -> Option<&SceneNode> {
        self.nodes.get(id.0)
    }

    /// Adds a node with no parent. `None` when the name is empty or holds a `/`.
    pub fn add_root(&mut self, name: &str) -> Option<NodeId> {
        if !valid_name(name) {
            return None;
        }
        let id = self.push_node(name, None);
        self.roots.push(id);
        Some(id)
    }

    /// Adds a child under `parent`. `None` when the parent does not exist or the name is not valid.
    pub fn add_child(&mut self, parent: NodeId, name: &str) -> Option<NodeId> {
        if self.node(parent).is_none() || !valid_name(name) {
            return None;
        }
        let id = self.push_node(name, Some(parent));
        self.nodes[parent.0].children.push(id);
        Some(id)
    }

    fn push_node(&mut self, name: &str, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(SceneNode {
            name: name.to_string(),
            hidden: false,
            ops: Vec::new(),
            parent,
            children: Vec::new(),
        });
        id
    }

    /// Appends a transform operation to the end of the node's list. `false` when the node does not exist.
    pub fn push_op(&mut self, id: NodeId, op: TransformOp) -> bool {
        match self.nodes.get_mut(id.0) {
            Some(node) => {
                node.ops.push(op);
                true
            }
            None => false,
        }
    }

    /// Sets the hidden flag. `false` when the node does not exist.
    pub fn set_hidden(&mut self, id: NodeId, hidden: bool) -> bool {
        match self.nodes.get_mut(id.0) {
            Some(node) => {
                node.hidden = hidden;
                true
            }
            None => false,
        }
    }

    /// The transform of this node relative to its parent. `None` when the node does not exist.
    pub fn local_matrix(&self, id: NodeId) -> Option<Matrix4> {
        let node = self.node(id)?;
        let mut m = Matrix4::identity();
        for op in &node.ops {
            m = m * op.matrix();
        }
        Some(m)
    }

    /// The transform of this node relative to the root: the parents' local transforms, root first.
    pub fn world_matrix(&self, id: NodeId) -> Option<Matrix4> {
        let mut m = Matrix4::identity();
        for n in self.chain(id)? {
            m = m * self.local_matrix(n)?;
        }
        Some(m)
    }

    /// The ids from the root down to `id`, both included.
    fn chain(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let node = self.node(c)?;
            out.push(c);
            cur = node.parent;
        }
        out.reverse();
        Some(out)
    }

    /// The path of names from the root to this node, joined with `/`.
    pub fn path(&self, id: NodeId) -> Option<String> {
        let names: Vec<&str> = self
            .chain(id)?
            .into_iter()
            .map(|n| self.nodes[n.0].name.as_str())
            .collect();
        Some(names.join("/"))
    }

    /// The node at a `/` separated path of names, matching the first node of each name. `None` when a segment
    /// matches nothing or the path is empty.
    pub fn find(&self, path: &str) -> Option<NodeId> {
        let mut parts = path.split('/');
        let first = parts.next()?;
        let mut cur = self
            .roots
            .iter()
            .copied()
            .find(|id| self.nodes[id.0].name == first)?;
        for part in parts {
            cur = self.nodes[cur.0]
                .children
                .iter()
                .copied()
                .find(|id| self.nodes[id.0].name == part)?;
        }
        Some(cur)
    }

    /// Every node under `id`, depth first and in child order. The node itself is not included.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = match self.node(id) {
            Some(node) => node.children.iter().rev().copied().collect(),
            None => return out,
        };
        while let Some(c) = stack.pop() {
            out.push(c);
            stack.extend(self.nodes[c.0].children.iter().rev().copied());
        }
        out
    }

    /// Whether the node and all its ancestors are not hidden. `None` when the node does not exist.
    pub fn is_rendered(&self, id: NodeId) -> Option<bool> {
        let chain = self.chain(id)?;
        Some(chain.iter().all(|n| !self.nodes[n.0].hidden))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const EPS: f32 = 1e-4;

    fn near(a: Vector3, b: Vector3) -> bool {
        (a - b).length() < EPS
    }

    #[test]
    fn child_transform_composes_with_parent() {
        let mut s = SceneObjects::new();
        let r = s.add_root("a").unwrap();
        s.push_op(r, TransformOp::Translate(Vector3::new(1.0, 0.0, 0.0)));
        let c = s.add_child(r, "b").unwrap();
        s.push_op(c, TransformOp::Translate(Vector3::new(0.0, 2.0, 0.0)));
        let world = s.world_matrix(c).unwrap();
        let origin = world.transform_point(Vector3::ZERO).unwrap();
        assert!(near(origin, Vector3::new(1.0, 2.0, 0.0)));
        // The local transform of the child alone only moves it by its own translation.
        let local = s.local_matrix(c).unwrap().transform_point(Vector3::ZERO).unwrap();
        assert!(near(local, Vector3::new(0.0, 2.0, 0.0)));
    }

    #[test]
    fn first_listed_operation_is_outermost() {
        let mut s = SceneObjects::new();
        let r = s.add_root("a").unwrap();
        s.push_op(r, TransformOp::Translate(Vector3::new(1.0, 0.0, 0.0)));
        s.push_op(r, TransformOp::Scale(Vector3::new(2.0, 2.0, 2.0)));
        // The scale applies first: (1,0,0) becomes (2,0,0), then the translation gives (3,0,0).
        let p = s.local_matrix(r).unwrap().transform_point(Vector3::new(1.0, 0.0, 0.0)).unwrap();
        assert!(near(p, Vector3::new(3.0, 0.0, 0.0)));
    }

    #[test]
    fn rotation_about_z_turns_x_into_y() {
        let mut s = SceneObjects::new();
        let r = s.add_root("spin").unwrap();
        s.push_op(r, TransformOp::Rotate { radians: PI / 2.0, axis: Vector3::new(0.0, 0.0, 1.0) });
        let p = s.local_matrix(r).unwrap().transform_point(Vector3::new(1.0, 0.0, 0.0)).unwrap();
        assert!(near(p, Vector3::new(0.0, 1.0, 0.0)));
    }

    #[test]
    fn matrix_operation_is_used_as_given() {
        let mut s = SceneObjects::new();
        let r = s.add_root("m").unwrap();
        s.push_op(r, TransformOp::Matrix(Matrix4::translation(5.0, 0.0, 0.0)));
        let p = s.local_matrix(r).unwrap().transform_point(Vector3::ZERO).unwrap();
        assert!(near(p, Vector3::new(5.0, 0.0, 0.0)));
    }

    #[test]
    fn paths_resolve_both_ways() {
        let mut s = SceneObjects::new();
        let world = s.add_root("world").unwrap();
        let car = s.add_child(world, "car").unwrap();
        let wheel = s.add_child(car, "wheel").unwrap();
        assert_eq!(s.path(wheel).as_deref(), Some("world/car/wheel"));
        assert_eq!(s.find("world/car/wheel"), Some(wheel));
        assert_eq!(s.find("world/missing"), None);
        assert_eq!(s.find(""), None);
        assert_eq!(s.find("world//car"), None);
    }

    #[test]
    fn hidden_parent_hides_its_children() {
        let mut s = SceneObjects::new();
        let r = s.add_root("room").unwrap();
        let lamp = s.add_child(r, "lamp").unwrap();
        assert_eq!(s.is_rendered(lamp), Some(true));
        s.set_hidden(r, true);
        assert_eq!(s.is_rendered(lamp), Some(false));
        s.set_hidden(r, false);
        assert_eq!(s.is_rendered(lamp), Some(true));
    }

    #[test]
    fn descendants_are_depth_first_in_child_order() {
        let mut s = SceneObjects::new();
        let r = s.add_root("r").unwrap();
        let c1 = s.add_child(r, "c1").unwrap();
        let g = s.add_child(c1, "g").unwrap();
        let c2 = s.add_child(r, "c2").unwrap();
        assert_eq!(s.descendants(r), vec![c1, g, c2]);
        assert!(s.descendants(c2).is_empty());
    }

    #[test]
    fn invalid_names_and_parents_are_refused() {
        let mut s = SceneObjects::new();
        assert_eq!(s.add_root(""), None);
        assert_eq!(s.add_root("a/b"), None);
        let r = s.add_root("ok").unwrap();
        assert_eq!(s.add_child(NodeId(99), "x"), None);
        assert_eq!(s.add_child(r, ""), None);
        assert_eq!(s.len(), 1);
        assert!(!s.push_op(NodeId(7), TransformOp::Scale(Vector3::new(1.0, 1.0, 1.0))));
    }
}
