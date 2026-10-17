//! Scene nodes as a pure model: a tree of positioned nodes with z order, alpha, shape, label and image content, an
//! optional camera node, and an action runner per node.
//!
//! Each node sits at a position in its parent's coordinates and has a rotation, a scale and an alpha. A child's world
//! transform is its parent's world transform followed by its own, so rotations and scales compose down the tree. The
//! content of a node is centred on the node (its anchor is the middle), and its frame is the bounds of that content in
//! scene coordinates after the world transform.
//!
//! Without a camera the scene origin maps to the centre of the view. A camera node maps the view onto the scene: the
//! camera's world position is shown at the view centre, and its world scale zooms, so a scale of two shows twice the
//! area. The camera's world rotation turns the view. Coordinates are not flipped: the renderer decides the y direction.
//!
//! The tree draws nothing. [`NodeTree::draw_list`] returns the visible items in paint order with their frames in view
//! coordinates, and a renderer turns them into pixels. Each node owns a runner, so an action runs on one node and its
//! reports carry that node's id. Time is a step the caller supplies; no clock is read here.

use std::collections::BTreeMap;

use crate::sk_action::{Action, Handle, Runner};

/// A point in scene or view coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// An axis-aligned rectangle given by its top-left corner and its size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub origin: Point,
    pub size: Size,
}

/// What a node shows. Shapes, labels and images are centred on the node. A label or image takes its size from the
/// caller, because measuring text and decoding images belong to the renderer.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Holds children and draws nothing.
    Empty,
    /// A filled rectangle.
    Rect(Size),
    /// A filled circle with the given radius.
    Circle(f64),
    /// A line of text in the given box.
    Label { text: String, size: Size },
    /// A named image drawn in the given box.
    Image { name: String, size: Size },
}

impl Content {
    /// The box the content is drawn in, or `None` for content that draws nothing. A circle has its diameter as both
    /// width and height.
    fn bounding_size(&self) -> Option<Size> {
        match self {
            Content::Empty => None,
            Content::Rect(size) | Content::Label { size, .. } | Content::Image { size, .. } => Some(*size),
            Content::Circle(radius) => Some(Size {
                width: 2.0 * radius,
                height: 2.0 * radius,
            }),
        }
    }
}

/// A handle to a node in a [`NodeTree`]. Ids are not reused, so a stale id is refused rather than pointing at a new
/// node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

/// One item of the draw list: a node with visible content, its frame in view coordinates and its effective alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawItem {
    pub node: NodeId,
    pub content: Content,
    pub frame: Frame,
    pub alpha: f64,
}

/// One item of the quad list: a node with visible content and the four corners of its box in view coordinates. The
/// corners follow the node's own axes after its world transform and the camera, so a rotated node is a rotated
/// quad, where [`DrawItem`] holds only its bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawQuad {
    pub node: NodeId,
    pub content: Content,
    /// The box corners in the order of the node's own axes: (-x, -y), (+x, -y), (+x, +y), (-x, +y). With y pointing up
    /// that is bottom-left, bottom-right, top-right, top-left. A circle's box is its bounding square.
    pub corners: [Point; 4],
    pub alpha: f64,
}

/// An affine map of the plane: a 2x2 linear part and a translation. A point `p` maps to
/// `(m11 x + m12 y + tx, m21 x + m22 y + ty)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine {
    m11: f64,
    m12: f64,
    m21: f64,
    m22: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    const IDENTITY: Affine = Affine {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// The transform of a node in its parent: scale, then rotation, then translation.
    fn local(position: Point, rotation: f64, scale: Point) -> Affine {
        let (sin, cos) = rotation.sin_cos();
        Affine {
            m11: cos * scale.x,
            m12: -sin * scale.y,
            m21: sin * scale.x,
            m22: cos * scale.y,
            tx: position.x,
            ty: position.y,
        }
    }

    /// The map that applies `child` first and then `self`.
    fn then(self, child: Affine) -> Affine {
        Affine {
            m11: self.m11 * child.m11 + self.m12 * child.m21,
            m12: self.m11 * child.m12 + self.m12 * child.m22,
            m21: self.m21 * child.m11 + self.m22 * child.m21,
            m22: self.m21 * child.m12 + self.m22 * child.m22,
            tx: self.m11 * child.tx + self.m12 * child.ty + self.tx,
            ty: self.m21 * child.tx + self.m22 * child.ty + self.ty,
        }
    }

    fn apply(self, p: Point) -> Point {
        Point {
            x: self.m11 * p.x + self.m12 * p.y + self.tx,
            y: self.m21 * p.x + self.m22 * p.y + self.ty,
        }
    }

    fn origin(self) -> Point {
        Point {
            x: self.tx,
            y: self.ty,
        }
    }

    /// Applies the inverse of the linear part to a vector. `None` when the map collapses a direction (a zero scale).
    fn unapply_vector(self, v: Point) -> Option<Point> {
        let det = self.m11 * self.m22 - self.m12 * self.m21;
        if det.abs() < 1e-12 {
            return None;
        }
        Some(Point {
            x: (self.m22 * v.x - self.m12 * v.y) / det,
            y: (-self.m21 * v.x + self.m11 * v.y) / det,
        })
    }
}

/// A node and its state. Only [`NodeTree`] builds and edits these.
#[derive(Debug)]
struct Node {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    content: Content,
    position: Point,
    z_position: f64,
    rotation: f64,
    scale: Point,
    alpha: f64,
    hidden: bool,
    actions: Runner,
}

impl Node {
    fn new(parent: Option<NodeId>, content: Content) -> Node {
        Node {
            parent,
            children: Vec::new(),
            content,
            position: Point::default(),
            z_position: 0.0,
            rotation: 0.0,
            scale: Point { x: 1.0, y: 1.0 },
            alpha: 1.0,
            hidden: false,
            actions: Runner::new(),
        }
    }

    fn local(&self) -> Affine {
        Affine::local(self.position, self.rotation, self.scale)
    }
}

/// A tree of scene nodes with an optional camera.
#[derive(Debug)]
pub struct NodeTree {
    nodes: BTreeMap<NodeId, Node>,
    next: u32,
    camera: Option<NodeId>,
}

const ROOT: NodeId = NodeId(0);

impl Default for NodeTree {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeTree {
    /// A tree holding only the scene root, which has no content and cannot be removed.
    pub fn new() -> NodeTree {
        let mut nodes = BTreeMap::new();
        nodes.insert(ROOT, Node::new(None, Content::Empty));
        NodeTree {
            nodes,
            next: 1,
            camera: None,
        }
    }

    /// The scene root.
    pub fn root(&self) -> NodeId {
        ROOT
    }

    /// Adds a child to `parent` and returns its id, or `None` when `parent` is not in the tree.
    pub fn add(&mut self, parent: NodeId, content: Content) -> Option<NodeId> {
        if !self.nodes.contains_key(&parent) {
            return None;
        }
        let id = NodeId(self.next);
        self.next += 1;
        self.nodes.insert(id, Node::new(Some(parent), content));
        if let Some(owner) = self.nodes.get_mut(&parent) {
            owner.children.push(id);
        }
        Some(id)
    }

    /// Removes a node with its subtree and their running actions. The root cannot be removed. Returns whether a node
    /// was removed.
    pub fn remove(&mut self, id: NodeId) -> bool {
        if id == ROOT {
            return false;
        }
        let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) else {
            return false;
        };
        if let Some(owner) = self.nodes.get_mut(&parent) {
            owner.children.retain(|child| *child != id);
        }
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            if let Some(node) = self.nodes.remove(&current) {
                pending.extend(node.children);
            }
        }
        if self.camera.is_some_and(|c| !self.nodes.contains_key(&c)) {
            self.camera = None;
        }
        true
    }

    /// The parent of a node; `None` for the root and for unknown ids.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(|n| n.parent)
    }

    /// The children of a node in the order they were added, or `None` for an unknown id.
    pub fn children(&self, id: NodeId) -> Option<&[NodeId]> {
        self.nodes.get(&id).map(|n| n.children.as_slice())
    }

    /// Sets the position of a node in its parent's coordinates. Returns `false` for an unknown id.
    pub fn set_position(&mut self, id: NodeId, position: Point) -> bool {
        self.edit(id, |n| n.position = position)
    }

    /// Sets the rotation in radians, counter-clockwise in scene coordinates.
    pub fn set_rotation(&mut self, id: NodeId, radians: f64) -> bool {
        self.edit(id, |n| n.rotation = radians)
    }

    pub fn set_scale(&mut self, id: NodeId, scale: Point) -> bool {
        self.edit(id, |n| n.scale = scale)
    }

    /// Sets the depth among siblings. Siblings with a lower value are painted first.
    pub fn set_z_position(&mut self, id: NodeId, z: f64) -> bool {
        self.edit(id, |n| n.z_position = z)
    }

    /// Sets the alpha, clamped to 0 through 1.
    pub fn set_alpha(&mut self, id: NodeId, alpha: f64) -> bool {
        self.edit(id, |n| n.alpha = alpha.clamp(0.0, 1.0))
    }

    /// Hides a node and everything below it.
    pub fn set_hidden(&mut self, id: NodeId, hidden: bool) -> bool {
        self.edit(id, |n| n.hidden = hidden)
    }

    /// Chooses the camera node, or none. Returns `false` when the camera id is not in the tree.
    pub fn set_camera(&mut self, camera: Option<NodeId>) -> bool {
        if let Some(id) = camera {
            if !self.nodes.contains_key(&id) {
                return false;
            }
        }
        self.camera = camera;
        true
    }

    /// The position of a node in scene coordinates.
    pub fn world_position(&self, id: NodeId) -> Option<Point> {
        self.world(id).map(Affine::origin)
    }

    /// The alpha of a node: its own alpha multiplied by the alpha of every ancestor.
    pub fn world_alpha(&self, id: NodeId) -> Option<f64> {
        let chain = self.chain(id)?;
        Some(chain.iter().map(|c| self.nodes[c].alpha).product())
    }

    /// The bounds of a node's content in scene coordinates. `None` for content that draws nothing.
    pub fn world_frame(&self, id: NodeId) -> Option<Frame> {
        let node = self.nodes.get(&id)?;
        let world = self.world(id)?;
        content_frame(&node.content, world)
    }

    /// Maps a scene point to view coordinates. `None` when the camera collapses a direction.
    pub fn view_point(&self, scene: Point, view: Size) -> Option<Point> {
        let center = view_center(view);
        match self.camera_transform() {
            None => Some(Point {
                x: center.x + scene.x,
                y: center.y + scene.y,
            }),
            Some(camera) => {
                let offset = camera.unapply_vector(Point {
                    x: scene.x - camera.tx,
                    y: scene.y - camera.ty,
                })?;
                Some(Point {
                    x: center.x + offset.x,
                    y: center.y + offset.y,
                })
            }
        }
    }

    /// Maps a view point to scene coordinates, the inverse of [`NodeTree::view_point`].
    pub fn scene_point(&self, view: Point, size: Size) -> Option<Point> {
        let center = view_center(size);
        let offset = Point {
            x: view.x - center.x,
            y: view.y - center.y,
        };
        match self.camera_transform() {
            None => Some(Point {
                x: offset.x,
                y: offset.y,
            }),
            Some(camera) => Some(camera.apply(offset)),
        }
    }

    /// The visible items in paint order, with frames in view coordinates. A node is visible when it is not hidden,
    /// its alpha is above zero and its content draws something; a hidden node hides its subtree. A parent is painted
    /// before its children, and siblings are painted by ascending z position, keeping insertion order for equal values.
    /// Returns an empty list when the camera collapses a direction.
    pub fn draw_list(&self, view: Size) -> Vec<DrawItem> {
        let mut items = Vec::new();
        self.collect(ROOT, Affine::IDENTITY, 1.0, view, &mut items);
        items
    }

    /// The visible items in paint order, as [`DrawQuad`]s: the same nodes in the same order as [`NodeTree::draw_list`],
    /// each with its box corners in view coordinates instead of the bounds of the box.
    pub fn draw_quads(&self, view: Size) -> Vec<DrawQuad> {
        let mut items = Vec::new();
        self.collect_quads(ROOT, Affine::IDENTITY, 1.0, view, &mut items);
        items
    }

    /// Starts an action on a node. Returns `None` for an unknown id.
    pub fn run_action(&mut self, id: NodeId, action: &Action) -> Option<Handle> {
        self.nodes.get_mut(&id).map(|n| n.actions.run(action))
    }

    /// Removes a running action from a node. Returns whether it was still running.
    pub fn remove_action(&mut self, id: NodeId, handle: Handle) -> bool {
        self.nodes.get_mut(&id).is_some_and(|n| n.actions.remove(handle))
    }

    /// Advances the actions of every node by `dt` seconds. Returns the report ids that fired, paired with the node that
    /// ran them, in node order and then in the order the reports fired.
    pub fn advance_actions(&mut self, dt: f64) -> Vec<(NodeId, u32)> {
        let mut fired = Vec::new();
        for (id, node) in self.nodes.iter_mut() {
            for report in node.actions.advance(dt) {
                fired.push((*id, report));
            }
        }
        fired
    }

    fn edit(&mut self, id: NodeId, change: impl FnOnce(&mut Node)) -> bool {
        match self.nodes.get_mut(&id) {
            Some(node) => {
                change(node);
                true
            }
            None => false,
        }
    }

    /// The ids from the root down to `id`, or `None` for an unknown id.
    fn chain(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(c) = current {
            let node = self.nodes.get(&c)?;
            chain.push(c);
            current = node.parent;
        }
        chain.reverse();
        Some(chain)
    }

    /// The world transform of a node: the composition of its chain of local transforms.
    fn world(&self, id: NodeId) -> Option<Affine> {
        let chain = self.chain(id)?;
        Some(
            chain
                .iter()
                .fold(Affine::IDENTITY, |acc, c| acc.then(self.nodes[c].local())),
        )
    }

    fn camera_transform(&self) -> Option<Affine> {
        self.camera.and_then(|c| self.world(c))
    }

    fn collect(&self, id: NodeId, parent_world: Affine, parent_alpha: f64, view: Size, out: &mut Vec<DrawItem>) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        if node.hidden {
            return;
        }
        let world = parent_world.then(node.local());
        let alpha = parent_alpha * node.alpha;
        if alpha > 0.0 {
            if let Some(frame) = content_frame(&node.content, world).and_then(|f| self.view_frame(f, view)) {
                out.push(DrawItem {
                    node: id,
                    content: node.content.clone(),
                    frame,
                    alpha,
                });
            }
        }
        let mut children: Vec<NodeId> = node.children.clone();
        children.sort_by(|a, b| self.nodes[a].z_position.total_cmp(&self.nodes[b].z_position));
        for child in children {
            self.collect(child, world, alpha, view, out);
        }
    }

    fn collect_quads(&self, id: NodeId, parent_world: Affine, parent_alpha: f64, view: Size, out: &mut Vec<DrawQuad>) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        if node.hidden {
            return;
        }
        let world = parent_world.then(node.local());
        let alpha = parent_alpha * node.alpha;
        if alpha > 0.0 {
            if let Some(size) = node.content.bounding_size() {
                let (hw, hh) = (size.width / 2.0, size.height / 2.0);
                let local = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];
                let mapped: Option<Vec<Point>> = local
                    .iter()
                    .map(|&(x, y)| self.view_point(world.apply(Point { x, y }), view))
                    .collect();
                if let Some(m) = mapped {
                    out.push(DrawQuad {
                        node: id,
                        content: node.content.clone(),
                        corners: [m[0], m[1], m[2], m[3]],
                        alpha,
                    });
                }
            }
        }
        let mut children: Vec<NodeId> = node.children.clone();
        children.sort_by(|a, b| self.nodes[a].z_position.total_cmp(&self.nodes[b].z_position));
        for child in children {
            self.collect_quads(child, world, alpha, view, out);
        }
    }

    /// The bounds of a scene frame in view coordinates, or `None` when the camera collapses a direction.
    fn view_frame(&self, frame: Frame, view: Size) -> Option<Frame> {
        let corners = [
            Point {
                x: frame.origin.x,
                y: frame.origin.y,
            },
            Point {
                x: frame.origin.x + frame.size.width,
                y: frame.origin.y,
            },
            Point {
                x: frame.origin.x,
                y: frame.origin.y + frame.size.height,
            },
            Point {
                x: frame.origin.x + frame.size.width,
                y: frame.origin.y + frame.size.height,
            },
        ];
        let mapped = corners
            .iter()
            .map(|c| self.view_point(*c, view))
            .collect::<Option<Vec<Point>>>()?;
        Some(bounds(&mapped))
    }
}

fn view_center(view: Size) -> Point {
    Point {
        x: view.width / 2.0,
        y: view.height / 2.0,
    }
}

/// The scene bounds of content drawn at a world transform. A circle stays a circle under rotation and scale, so its
/// bounds come from the linear part of the transform; a box takes the bounds of its four transformed corners.
fn content_frame(content: &Content, world: Affine) -> Option<Frame> {
    let size = content.bounding_size()?;
    if let Content::Circle(radius) = content {
        let half_width = radius * (world.m11 * world.m11 + world.m12 * world.m12).sqrt();
        let half_height = radius * (world.m21 * world.m21 + world.m22 * world.m22).sqrt();
        return Some(Frame {
            origin: Point {
                x: world.tx - half_width,
                y: world.ty - half_height,
            },
            size: Size {
                width: 2.0 * half_width,
                height: 2.0 * half_height,
            },
        });
    }
    let (hw, hh) = (size.width / 2.0, size.height / 2.0);
    let corners = [
        world.apply(Point { x: -hw, y: -hh }),
        world.apply(Point { x: hw, y: -hh }),
        world.apply(Point { x: -hw, y: hh }),
        world.apply(Point { x: hw, y: hh }),
    ];
    Some(bounds(&corners))
}

/// The smallest frame that holds every point. Points must not be empty.
fn bounds(points: &[Point]) -> Frame {
    let min_x = points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    Frame {
        origin: Point { x: min_x, y: min_y },
        size: Size {
            width: max_x - min_x,
            height: max_y - min_y,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, x: f64, y: f64) -> bool {
        (a.x - x).abs() < 1e-9 && (a.y - y).abs() < 1e-9
    }

    #[test]
    fn a_rotated_node_is_a_rotated_quad_with_corners_in_the_order_of_its_axes() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let n = tree.add(root, Content::Rect(Size { width: 40.0, height: 20.0 })).unwrap();
        tree.set_position(n, Point { x: 10.0, y: 0.0 });
        let view = Size { width: 100.0, height: 100.0 };
        let q = tree.draw_quads(view);
        assert_eq!(q.len(), 1);
        assert!(close(q[0].corners[0], 40.0, 40.0), "(-x, -y) corner: {:?}", q[0].corners);
        assert!(close(q[0].corners[1], 80.0, 40.0));
        assert!(close(q[0].corners[2], 80.0, 60.0));
        assert!(close(q[0].corners[3], 40.0, 60.0));
        tree.set_rotation(n, std::f64::consts::FRAC_PI_2);
        let q = tree.draw_quads(view);
        assert!(close(q[0].corners[0], 70.0, 30.0), "the (-x, -y) corner turned a quarter: {:?}", q[0].corners);
        assert!(close(q[0].corners[1], 70.0, 70.0));
        let frame = tree.draw_list(view)[0].frame;
        assert!((frame.size.width - 20.0).abs() < 1e-9 && (frame.size.height - 40.0).abs() < 1e-9, "the bounds hold the same box");
    }

    #[test]
    fn quads_come_in_the_paint_order_of_the_draw_list_and_skip_what_does_not_draw() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let size = Size { width: 10.0, height: 10.0 };
        let back = tree.add(root, Content::Rect(size)).unwrap();
        let front = tree.add(root, Content::Circle(5.0)).unwrap();
        let hidden = tree.add(root, Content::Rect(size)).unwrap();
        let empty = tree.add(root, Content::Empty).unwrap();
        tree.set_z_position(front, 2.0);
        tree.set_z_position(back, 1.0);
        tree.set_hidden(hidden, true);
        let view = Size { width: 50.0, height: 50.0 };
        let quads: Vec<NodeId> = tree.draw_quads(view).iter().map(|q| q.node).collect();
        let items: Vec<NodeId> = tree.draw_list(view).iter().map(|i| i.node).collect();
        assert_eq!(quads, items);
        assert_eq!(quads, [back, front]);
        assert!(!quads.contains(&empty));
    }

    #[test]
    fn the_camera_moves_and_zooms_the_quads_like_the_draw_list() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let n = tree.add(root, Content::Rect(Size { width: 10.0, height: 10.0 })).unwrap();
        let cam = tree.add(root, Content::Empty).unwrap();
        tree.set_camera(Some(cam));
        tree.set_position(cam, Point { x: 5.0, y: 0.0 });
        tree.set_scale(cam, Point { x: 0.5, y: 0.5 });
        let view = Size { width: 100.0, height: 100.0 };
        let q = &tree.draw_quads(view)[0];
        let f = tree.draw_list(view)[0].frame;
        assert_eq!(q.node, n);
        assert!(close(q.corners[0], f.origin.x, f.origin.y));
        assert!(close(q.corners[2], f.origin.x + f.size.width, f.origin.y + f.size.height));
        assert!((f.size.width - 20.0).abs() < 1e-9, "a camera scale of one half zooms in: the box is twice as wide on screen");
    }
}
