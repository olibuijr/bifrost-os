//! The retained layer tree: a [`Layer`] is a shared handle (clones are the same layer) with geometry (bounds,
//! position, anchor point, z position, transform), content (an image placed by a gravity, or what a delegate drew),
//! simple style (opacity, hidden, masks-to-bounds, background, border), sublayers and a superlayer.
//!
//! Every property is also reachable by key path through [`Layer::value`] and [`Layer::set_value`]
//! (`"position.x"`, `"opacity"`, `"transform.rotation.z"`, ...). Keys the layer does not know are kept in a table of
//! extra values, so layer styles, layer subclasses and animations can hang their own properties on a layer and read
//! them back by key. [`Layer::add_observer`] reports every change (old and new value) after it applied.
//!
//! Geometry follows the usual layer model: a point `p` in a layer's own space (bounds coordinates, y down) lies at
//! `position + transform * (p - anchor)` in the superlayer's space, where `anchor` is the anchor point (a unit
//! fraction) of the bounds. [`Layer::render_list`] flattens the tree, parents first and sublayers by z position, into
//! [`LayerCmd`]s a renderer replays; the SDK does so for layer-backed views.
//!
//! Single-threaded: layers use `Rc`. Pure model with unit tests; the compositor is not involved.
use crate::cg::path::Affine as AffineTransform;
use crate::layer_layout::{Constraint, DrawOp, LayerAction, LayerContext, LayerDelegate, LayoutManager};
use crate::transform3d::Transform3D;
use crate::Rect;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};

/// A bitmap used as layer content: premultiplied RGBA packed in `u32`s (R in the low byte), `id` stable for the same
/// pixels so a renderer can cache textures.
#[derive(Clone, Debug)]
pub struct LayerImage {
    pub id: u64,
    pub w: usize,
    pub h: usize,
    pub px: Rc<Vec<u32>>,
}

impl LayerImage {
    pub fn new(w: usize, h: usize, px: Vec<u32>) -> Self {
        let mut hasher = DefaultHasher::new();
        w.hash(&mut hasher);
        h.hash(&mut hasher);
        px.hash(&mut hasher);
        LayerImage { id: hasher.finish(), w, h, px: Rc::new(px) }
    }
}

impl PartialEq for LayerImage {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.w == o.w && self.h == o.h
    }
}

/// How contents are placed in the bounds (`contentsGravity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentsGravity {
    /// Stretched to the bounds.
    Resize,
    /// Scaled to fit, keeping the aspect ratio.
    ResizeAspect,
    /// Scaled to cover, keeping the aspect ratio.
    ResizeAspectFill,
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ContentsGravity {
    pub fn name(self) -> &'static str {
        match self {
            ContentsGravity::Resize => "resize",
            ContentsGravity::ResizeAspect => "resizeAspect",
            ContentsGravity::ResizeAspectFill => "resizeAspectFill",
            ContentsGravity::Center => "center",
            ContentsGravity::Top => "top",
            ContentsGravity::Bottom => "bottom",
            ContentsGravity::Left => "left",
            ContentsGravity::Right => "right",
            ContentsGravity::TopLeft => "topLeft",
            ContentsGravity::TopRight => "topRight",
            ContentsGravity::BottomLeft => "bottomLeft",
            ContentsGravity::BottomRight => "bottomRight",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        [
            ContentsGravity::Resize,
            ContentsGravity::ResizeAspect,
            ContentsGravity::ResizeAspectFill,
            ContentsGravity::Center,
            ContentsGravity::Top,
            ContentsGravity::Bottom,
            ContentsGravity::Left,
            ContentsGravity::Right,
            ContentsGravity::TopLeft,
            ContentsGravity::TopRight,
            ContentsGravity::BottomLeft,
            ContentsGravity::BottomRight,
        ]
        .into_iter()
        .find(|g| g.name() == name)
    }
}

/// Where an image of `iw` x `ih` points lands in `bounds` for `gravity`.
pub fn gravity_rect(gravity: ContentsGravity, bounds: Rect, iw: f32, ih: f32) -> Rect {
    let (bx, by, bw, bh) = (bounds.x, bounds.y, bounds.w, bounds.h);
    if iw <= 0.0 || ih <= 0.0 {
        return Rect { x: bx, y: by, w: 0.0, h: 0.0 };
    }
    let aspect = |fill: bool| {
        let s = if fill { (bw / iw).max(bh / ih) } else { (bw / iw).min(bh / ih) };
        let (w, h) = (iw * s, ih * s);
        Rect { x: bx + (bw - w) / 2.0, y: by + (bh - h) / 2.0, w, h }
    };
    let at = |hx: f32, vy: f32| Rect { x: bx + (bw - iw) * hx, y: by + (bh - ih) * vy, w: iw, h: ih };
    match gravity {
        ContentsGravity::Resize => bounds,
        ContentsGravity::ResizeAspect => aspect(false),
        ContentsGravity::ResizeAspectFill => aspect(true),
        ContentsGravity::Center => at(0.5, 0.5),
        ContentsGravity::Top => at(0.5, 0.0),
        ContentsGravity::Bottom => at(0.5, 1.0),
        ContentsGravity::Left => at(0.0, 0.5),
        ContentsGravity::Right => at(1.0, 0.5),
        ContentsGravity::TopLeft => at(0.0, 0.0),
        ContentsGravity::TopRight => at(1.0, 0.0),
        ContentsGravity::BottomLeft => at(0.0, 1.0),
        ContentsGravity::BottomRight => at(1.0, 1.0),
    }
}

/// A value of a layer property, as read and written by key path.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerValue {
    Float(f32),
    Bool(bool),
    Point(f32, f32),
    Size(f32, f32),
    Rect(Rect),
    /// Premultiplied RGBA packed in a `u32`; 0 is clear.
    Color(u32),
    Transform(Transform3D),
    Text(String),
    Image(Option<LayerImage>),
}

/// Why [`Layer::set_value`] refused a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerError {
    /// The value's type does not fit the key.
    TypeMismatch(String),
    /// The text is not a valid contents gravity.
    BadValue(String),
}

/// The keys of the built-in properties (key paths below them, like `position.x`, work as well).
pub const BUILTIN_KEYS: &[&str] = &[
    "name",
    "bounds",
    "position",
    "anchorPoint",
    "zPosition",
    "frame",
    "transform",
    "opacity",
    "hidden",
    "masksToBounds",
    "backgroundColor",
    "borderWidth",
    "borderColor",
    "contents",
    "contentsGravity",
    "contentsScale",
    "contentsRect",
];

/// Observer of property changes: layer, key, old value, new value.
pub type LayerObserver = Rc<dyn Fn(&Layer, &str, &LayerValue, &LayerValue)>;

struct Node {
    id: u64,
    name: String,
    bounds: Rect,
    position: (f32, f32),
    anchor: (f32, f32),
    z: f32,
    transform: Transform3D,
    opacity: f32,
    hidden: bool,
    masks: bool,
    background: u32,
    border_width: f32,
    border_color: u32,
    contents: Option<LayerImage>,
    gravity: ContentsGravity,
    contents_scale: f32,
    contents_rect: Rect,
    sublayers: Vec<Layer>,
    superlayer: Weak<RefCell<Node>>,
    delegate: Option<Rc<dyn LayerDelegate>>,
    layout: Option<Rc<dyn LayoutManager>>,
    constraints: Vec<Constraint>,
    actions: HashMap<String, LayerAction>,
    extras: BTreeMap<String, LayerValue>,
    observers: Vec<LayerObserver>,
    needs_display: bool,
    needs_layout: bool,
    needs_display_on_bounds_change: bool,
    /// something visible changed since the last [`Layer::clear_dirty_tree`]
    dirty: bool,
    drawn: Vec<DrawOp>,
}

/// A shared handle to a layer; clones refer to the same layer. Equality is identity.
#[derive(Clone)]
pub struct Layer(Rc<RefCell<Node>>);

impl PartialEq for Layer {
    fn eq(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.borrow();
        write!(f, "Layer#{}({:?} {:?} sub={})", n.id, n.name, n.bounds, n.sublayers.len())
    }
}

impl Default for Layer {
    fn default() -> Self {
        Layer::new()
    }
}

static NEXT_LAYER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn r_unit() -> Rect {
    Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
}

fn to_affine(t: &Transform3D) -> AffineTransform {
    AffineTransform { a: t.m[0][0], b: t.m[0][1], c: t.m[1][0], d: t.m[1][1], tx: t.m[3][0], ty: t.m[3][1] }
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h
}

impl Layer {
    pub fn new() -> Layer {
        Layer(Rc::new(RefCell::new(Node {
            id: NEXT_LAYER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name: String::new(),
            bounds: Rect::default(),
            position: (0.0, 0.0),
            anchor: (0.5, 0.5),
            z: 0.0,
            transform: Transform3D::IDENTITY,
            opacity: 1.0,
            hidden: false,
            masks: false,
            background: 0,
            border_width: 0.0,
            border_color: 0xff00_0000,
            contents: None,
            gravity: ContentsGravity::Resize,
            contents_scale: 1.0,
            contents_rect: r_unit(),
            sublayers: Vec::new(),
            superlayer: Weak::new(),
            delegate: None,
            layout: None,
            constraints: Vec::new(),
            actions: HashMap::new(),
            extras: BTreeMap::new(),
            observers: Vec::new(),
            needs_display: true,
            needs_layout: false,
            needs_display_on_bounds_change: false,
            dirty: true,
            drawn: Vec::new(),
        })))
    }

    /// A layer with `bounds` and the `position` that puts its frame at the bounds' size from the origin.
    pub fn with_frame(frame: Rect) -> Layer {
        let l = Layer::new();
        l.set_frame(frame);
        l
    }

    /// Process-unique id.
    pub fn id(&self) -> u64 {
        self.0.borrow().id
    }

    // ---- change plumbing -------------------------------------------------------------------------------------

    /// Applies `f`, and when the value under `key` changed: marks the layer changed, flags layout where geometry
    /// moved, and tells the observers.
    fn change(&self, key: &str, f: impl FnOnce(&mut Node)) {
        let old = self.value(key);
        f(&mut self.0.borrow_mut());
        let new = self.value(key);
        if old == new {
            return;
        }
        let (old, new) = (old.unwrap_or(LayerValue::Bool(false)), new.unwrap_or(LayerValue::Bool(false)));
        let geometry = matches!(key, "bounds" | "position" | "anchorPoint" | "transform" | "frame");
        let (observers, parent) = {
            let mut n = self.0.borrow_mut();
            n.dirty = true;
            if key == "bounds" {
                n.needs_layout = true;
                if n.needs_display_on_bounds_change {
                    n.needs_display = true;
                }
            }
            (n.observers.clone(), n.superlayer.upgrade())
        };
        if geometry {
            if let Some(p) = parent {
                let mut pn = p.borrow_mut();
                if pn.layout.as_ref().map_or(false, |m| m.reacts_to_sublayer_changes()) {
                    pn.needs_layout = true;
                }
            }
        }
        for o in observers {
            o(self, key, &old, &new);
        }
    }

    /// Registers a change observer; it runs after the change applied. Observers must not add observers.
    pub fn add_observer(&self, observer: LayerObserver) {
        self.0.borrow_mut().observers.push(observer);
    }

    /// Something visible in this layer or a sublayer changed since the last [`Layer::clear_dirty_tree`].
    pub fn tree_dirty(&self) -> bool {
        let n = self.0.borrow();
        n.dirty || n.sublayers.iter().any(|s| s.tree_dirty())
    }

    pub fn clear_dirty_tree(&self) {
        let subs = {
            let mut n = self.0.borrow_mut();
            n.dirty = false;
            n.sublayers.clone()
        };
        subs.iter().for_each(|s| s.clear_dirty_tree());
    }

    // ---- plain properties ------------------------------------------------------------------------------------

    pub fn name(&self) -> String {
        self.0.borrow().name.clone()
    }
    pub fn set_name(&self, name: &str) {
        self.change("name", |n| n.name = name.to_string());
    }

    pub fn bounds(&self) -> Rect {
        self.0.borrow().bounds
    }
    pub fn set_bounds(&self, bounds: Rect) {
        self.change("bounds", |n| n.bounds = bounds);
    }

    pub fn position(&self) -> (f32, f32) {
        self.0.borrow().position
    }
    pub fn set_position(&self, x: f32, y: f32) {
        self.change("position", |n| n.position = (x, y));
    }

    /// The anchor point as a fraction of the bounds (0.5, 0.5 by default).
    pub fn anchor_point(&self) -> (f32, f32) {
        self.0.borrow().anchor
    }
    /// Changing the anchor moves the layer's frame unless `position` is adjusted too.
    pub fn set_anchor_point(&self, x: f32, y: f32) {
        self.change("anchorPoint", |n| n.anchor = (x, y));
    }

    pub fn z_position(&self) -> f32 {
        self.0.borrow().z
    }
    pub fn set_z_position(&self, z: f32) {
        self.change("zPosition", |n| n.z = z);
    }

    pub fn transform(&self) -> Transform3D {
        self.0.borrow().transform
    }
    pub fn set_transform(&self, t: Transform3D) {
        self.change("transform", |n| n.transform = t);
    }

    pub fn opacity(&self) -> f32 {
        self.0.borrow().opacity
    }
    pub fn set_opacity(&self, v: f32) {
        self.change("opacity", |n| n.opacity = v);
    }

    pub fn is_hidden(&self) -> bool {
        self.0.borrow().hidden
    }
    pub fn set_hidden(&self, v: bool) {
        self.change("hidden", |n| n.hidden = v);
    }

    pub fn masks_to_bounds(&self) -> bool {
        self.0.borrow().masks
    }
    pub fn set_masks_to_bounds(&self, v: bool) {
        self.change("masksToBounds", |n| n.masks = v);
    }

    /// The background fill; `None` paints nothing.
    pub fn background_color(&self) -> Option<u32> {
        let c = self.0.borrow().background;
        (c != 0).then_some(c)
    }
    pub fn set_background_color(&self, color: Option<u32>) {
        self.change("backgroundColor", |n| n.background = color.unwrap_or(0));
    }

    pub fn border_width(&self) -> f32 {
        self.0.borrow().border_width
    }
    pub fn set_border_width(&self, w: f32) {
        self.change("borderWidth", |n| n.border_width = w);
    }
    pub fn border_color(&self) -> u32 {
        self.0.borrow().border_color
    }
    pub fn set_border_color(&self, c: u32) {
        self.change("borderColor", |n| n.border_color = c);
    }

    pub fn contents(&self) -> Option<LayerImage> {
        self.0.borrow().contents.clone()
    }
    pub fn set_contents(&self, image: Option<LayerImage>) {
        self.change("contents", |n| n.contents = image);
    }
    pub fn contents_gravity(&self) -> ContentsGravity {
        self.0.borrow().gravity
    }
    pub fn set_contents_gravity(&self, g: ContentsGravity) {
        self.change("contentsGravity", |n| n.gravity = g);
    }
    /// Pixels per point of the contents image.
    pub fn contents_scale(&self) -> f32 {
        self.0.borrow().contents_scale
    }
    pub fn set_contents_scale(&self, s: f32) {
        self.change("contentsScale", |n| n.contents_scale = s);
    }
    /// The part of the contents image shown, as a rectangle in unit coordinates.
    pub fn contents_rect(&self) -> Rect {
        self.0.borrow().contents_rect
    }
    pub fn set_contents_rect(&self, r: Rect) {
        self.change("contentsRect", |n| n.contents_rect = r);
    }

    // ---- frame and geometry ----------------------------------------------------------------------------------

    /// Local space to superlayer space.
    pub fn affine_to_superlayer(&self) -> AffineTransform {
        let n = self.0.borrow();
        let ax = n.bounds.x + n.anchor.0 * n.bounds.w;
        let ay = n.bounds.y + n.anchor.1 * n.bounds.h;
        AffineTransform::translate(-ax, -ay).then(to_affine(&n.transform)).then(AffineTransform::translate(n.position.0, n.position.1))
    }

    /// The bounds in the superlayer's space (the bounding box when the transform rotates or skews).
    pub fn frame(&self) -> Rect {
        self.affine_to_superlayer().apply_rect(self.bounds().into()).into()
    }

    /// Moves and sizes the layer so its frame is `f`. With a transform other than the identity only the position
    /// changes (the frame is then the bounding box of the transformed bounds).
    pub fn set_frame(&self, f: Rect) {
        if self.transform().is_identity() {
            let b = self.bounds();
            self.set_bounds(Rect { x: b.x, y: b.y, w: f.w, h: f.h });
            let (ax, ay) = self.anchor_point();
            self.set_position(f.x + ax * f.w, f.y + ay * f.h);
        } else {
            let cur = self.frame();
            let (px, py) = self.position();
            self.set_position(px + f.x - cur.x, py + f.y - cur.y);
        }
    }

    // ---- tree --------------------------------------------------------------------------------------------------

    pub fn superlayer(&self) -> Option<Layer> {
        self.0.borrow().superlayer.upgrade().map(Layer)
    }

    pub fn sublayers(&self) -> Vec<Layer> {
        self.0.borrow().sublayers.clone()
    }

    /// True when `self` is `other` or one of its ancestors.
    fn is_ancestor_or_self_of(&self, other: &Layer) -> bool {
        let mut cur = Some(other.clone());
        while let Some(l) = cur {
            if l == *self {
                return true;
            }
            cur = l.superlayer();
        }
        false
    }

    /// Adds `layer` as the last sublayer (in front). It leaves its current superlayer first. A layer cannot become
    /// its own ancestor: such a call does nothing.
    pub fn add_sublayer(&self, layer: &Layer) {
        let at = self.0.borrow().sublayers.len();
        self.insert_sublayer(layer, at);
    }

    /// Inserts `layer` at `index` (clamped) among the sublayers.
    pub fn insert_sublayer(&self, layer: &Layer, index: usize) {
        if layer.is_ancestor_or_self_of(self) {
            return;
        }
        let same = layer.superlayer().as_ref() == Some(self);
        let mut index = index;
        if same {
            let pos = self.0.borrow().sublayers.iter().position(|s| s == layer);
            if let Some(p) = pos {
                if p < index {
                    index -= 1;
                }
            }
        }
        layer.detach();
        let mut n = self.0.borrow_mut();
        let index = index.min(n.sublayers.len());
        n.sublayers.insert(index, layer.clone());
        n.needs_layout = true;
        n.dirty = true;
        drop(n);
        let mut c = layer.0.borrow_mut();
        c.superlayer = Rc::downgrade(&self.0);
        c.dirty = true;
    }

    /// Inserts `layer` just above (after) `sibling`; at the end when `sibling` is not a sublayer.
    pub fn insert_sublayer_above(&self, layer: &Layer, sibling: &Layer) {
        let at = self.index_of(sibling).map(|i| i + 1).unwrap_or_else(|| self.0.borrow().sublayers.len());
        self.insert_sublayer(layer, at);
    }

    /// Inserts `layer` just below (before) `sibling`; at the end when `sibling` is not a sublayer.
    pub fn insert_sublayer_below(&self, layer: &Layer, sibling: &Layer) {
        let at = self.index_of(sibling).unwrap_or_else(|| self.0.borrow().sublayers.len());
        self.insert_sublayer(layer, at);
    }

    fn index_of(&self, sub: &Layer) -> Option<usize> {
        self.0.borrow().sublayers.iter().position(|s| s == sub)
    }

    fn detach(&self) {
        if let Some(parent) = self.superlayer() {
            let mut p = parent.0.borrow_mut();
            p.sublayers.retain(|s| s != self);
            p.needs_layout = true;
            p.dirty = true;
        }
        self.0.borrow_mut().superlayer = Weak::new();
    }

    pub fn remove_from_superlayer(&self) {
        self.detach();
    }

    /// Puts `new` where `old` is; nothing happens when `old` is not a sublayer.
    pub fn replace_sublayer(&self, old: &Layer, new: &Layer) {
        let Some(i) = self.index_of(old) else { return };
        if old == new {
            return;
        }
        old.detach();
        self.insert_sublayer(new, i);
    }

    /// Replaces all sublayers.
    pub fn set_sublayers(&self, layers: &[Layer]) {
        for s in self.sublayers() {
            s.detach();
        }
        for l in layers {
            self.add_sublayer(l);
        }
    }

    /// The sublayers back to front: stable sort by z position.
    fn sublayers_by_z(&self) -> Vec<Layer> {
        let mut v = self.sublayers();
        v.sort_by(|a, b| a.z_position().partial_cmp(&b.z_position()).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    pub fn root(&self) -> Layer {
        let mut cur = self.clone();
        while let Some(p) = cur.superlayer() {
            cur = p;
        }
        cur
    }

    // ---- hit testing and coordinates ---------------------------------------------------------------------------

    /// True when `(x, y)`, in this layer's own space, lies inside its bounds.
    pub fn contains_point(&self, x: f32, y: f32) -> bool {
        contains(self.bounds(), x, y)
    }

    /// The front-most visible layer under `p` (in the superlayer's space) in this layer's subtree, or `None`.
    /// Hidden and fully transparent layers and their sublayers are skipped; a sublayer outside its superlayer's
    /// bounds cannot be hit.
    pub fn hit_test(&self, p: (f32, f32)) -> Option<Layer> {
        let inv = self.affine_to_superlayer().invert()?;
        let local = inv.apply(p.0, p.1);
        self.hit_test_local(local)
    }

    /// [`Layer::hit_test`] for a point already in this layer's own space.
    pub fn hit_test_local(&self, local: (f32, f32)) -> Option<Layer> {
        if self.is_hidden() || self.opacity() <= 0.0 || !self.contains_point(local.0, local.1) {
            return None;
        }
        for s in self.sublayers_by_z().iter().rev() {
            if let Some(hit) = s.hit_test(local) {
                return Some(hit);
            }
        }
        Some(self.clone())
    }

    /// Local space to the root layer's superlayer space.
    fn affine_to_root(&self) -> AffineTransform {
        let mut t = self.affine_to_superlayer();
        let mut cur = self.superlayer();
        while let Some(l) = cur {
            t = t.then(l.affine_to_superlayer());
            cur = l.superlayer();
        }
        t
    }

    /// Converts a point in this layer's space to `other`'s space.
    pub fn convert_point_to(&self, p: (f32, f32), other: &Layer) -> (f32, f32) {
        let root = self.affine_to_root().apply(p.0, p.1);
        match other.affine_to_root().invert() {
            Some(inv) => inv.apply(root.0, root.1),
            None => root,
        }
    }

    /// Converts a point in `other`'s space to this layer's space.
    pub fn convert_point_from(&self, p: (f32, f32), other: &Layer) -> (f32, f32) {
        other.convert_point_to(p, self)
    }

    /// Converts a rectangle in this layer's space to `other`'s space (the bounding box of its corners).
    pub fn convert_rect_to(&self, r: Rect, other: &Layer) -> Rect {
        let pts = [(r.x, r.y), (r.x + r.w, r.y), (r.x, r.y + r.h), (r.x + r.w, r.y + r.h)].map(|p| self.convert_point_to(p, other));
        let x0 = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let y0 = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let x1 = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        let y1 = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    pub fn convert_rect_from(&self, r: Rect, other: &Layer) -> Rect {
        other.convert_rect_to(r, self)
    }

    // ---- delegate, layout, actions ---------------------------------------------------------------------------

    pub fn set_delegate(&self, delegate: Option<Rc<dyn LayerDelegate>>) {
        let mut n = self.0.borrow_mut();
        n.delegate = delegate;
        n.needs_display = true;
        n.needs_layout = true;
    }
    pub fn delegate(&self) -> Option<Rc<dyn LayerDelegate>> {
        self.0.borrow().delegate.clone()
    }

    pub fn set_layout_manager(&self, manager: Option<Rc<dyn LayoutManager>>) {
        let mut n = self.0.borrow_mut();
        n.layout = manager;
        n.needs_layout = true;
    }
    pub fn layout_manager(&self) -> Option<Rc<dyn LayoutManager>> {
        self.0.borrow().layout.clone()
    }

    pub fn constraints(&self) -> Vec<Constraint> {
        self.0.borrow().constraints.clone()
    }
    pub fn add_constraint(&self, c: Constraint) {
        self.0.borrow_mut().constraints.push(c);
        self.mark_parent_layout();
    }
    pub fn set_constraints(&self, cs: Vec<Constraint>) {
        self.0.borrow_mut().constraints = cs;
        self.mark_parent_layout();
    }
    fn mark_parent_layout(&self) {
        if let Some(p) = self.superlayer() {
            p.set_needs_layout();
        }
    }

    /// The action table consulted after the delegate by [`Layer::action_for_key`].
    pub fn set_action(&self, key: &str, action: Option<LayerAction>) {
        let mut n = self.0.borrow_mut();
        match action {
            Some(a) => {
                n.actions.insert(key.to_string(), a);
            }
            None => {
                n.actions.remove(key);
            }
        }
    }

    /// What a change of `key` should do: the delegate's answer, else this layer's `actions` table, else `None`.
    pub fn action_for_key(&self, key: &str) -> Option<LayerAction> {
        let (delegate, table) = {
            let n = self.0.borrow();
            (n.delegate.clone(), n.actions.get(key).cloned())
        };
        delegate.and_then(|d| d.action_for(self, key)).or(table)
    }

    // ---- display and layout cycle ------------------------------------------------------------------------------

    pub fn set_needs_display(&self) {
        let mut n = self.0.borrow_mut();
        n.needs_display = true;
        n.dirty = true;
    }
    pub fn needs_display(&self) -> bool {
        self.0.borrow().needs_display
    }
    /// Redraw through the delegate whenever the bounds change (off by default).
    pub fn set_needs_display_on_bounds_change(&self, on: bool) {
        self.0.borrow_mut().needs_display_on_bounds_change = on;
    }

    pub fn set_needs_layout(&self) {
        self.0.borrow_mut().needs_layout = true;
    }
    pub fn needs_layout(&self) -> bool {
        self.0.borrow().needs_layout
    }

    /// Lays out this layer's sublayers now when they need it: the layout manager if set, else the delegate. Then the
    /// same for every sublayer. When an ancestor also needs layout, the topmost one runs first.
    pub fn layout_if_needed(&self) {
        let mut top = self.clone();
        let mut cur = self.superlayer();
        while let Some(l) = cur {
            if l.needs_layout() {
                top = l.clone();
            }
            cur = l.superlayer();
        }
        top.layout_pass();
    }

    fn layout_pass(&self) {
        if self.needs_layout() {
            let (manager, delegate) = {
                let n = self.0.borrow();
                (n.layout.clone(), n.delegate.clone())
            };
            if let Some(m) = manager {
                m.layout_sublayers(self);
            } else if let Some(d) = delegate {
                d.layout_sublayers(self);
            }
            // cleared after the pass: the pass itself moves sublayers, which would flag this layer again
            self.0.borrow_mut().needs_layout = false;
        }
        for s in self.sublayers() {
            s.layout_pass();
        }
    }

    /// Runs the delegate's drawing for every layer in the subtree that needs display.
    pub fn display_if_needed(&self) {
        if self.needs_display() {
            let (delegate, size) = {
                let n = self.0.borrow();
                (n.delegate.clone(), (n.bounds.w, n.bounds.h))
            };
            let mut ctx = LayerContext::new(size.0, size.1);
            if let Some(d) = delegate {
                d.draw_layer(self, &mut ctx);
            }
            let mut n = self.0.borrow_mut();
            n.drawn = ctx.into_ops();
            n.needs_display = false;
            n.dirty = true;
        }
        for s in self.sublayers() {
            s.display_if_needed();
        }
    }

    // ---- key paths ---------------------------------------------------------------------------------------------

    /// True for the built-in keys (and key paths below them).
    pub fn is_builtin_key(key: &str) -> bool {
        let head = key.split('.').next().unwrap_or("");
        BUILTIN_KEYS.contains(&head)
    }

    /// Reads a property by key path. Built-ins: `bounds` (`.origin.x`, `.size.width`, ...), `position` (`.x`, `.y`),
    /// `anchorPoint`, `zPosition`, `frame`, `transform` (`.scale`, `.scale.x|y|z`, `.rotation.z`,
    /// `.translation.x|y|z`), `opacity`, `hidden`, `masksToBounds`, `backgroundColor`, `borderWidth`, `borderColor`,
    /// `contents`, `contentsGravity`, `contentsScale`, `contentsRect`, `name`. Anything else is read from the extra
    /// values; `None` when nothing is stored.
    pub fn value(&self, key: &str) -> Option<LayerValue> {
        let n = self.0.borrow();
        let f = LayerValue::Float;
        Some(match key {
            "name" => LayerValue::Text(n.name.clone()),
            "bounds" => LayerValue::Rect(n.bounds),
            "bounds.origin" => LayerValue::Point(n.bounds.x, n.bounds.y),
            "bounds.origin.x" => f(n.bounds.x),
            "bounds.origin.y" => f(n.bounds.y),
            "bounds.size" => LayerValue::Size(n.bounds.w, n.bounds.h),
            "bounds.size.width" => f(n.bounds.w),
            "bounds.size.height" => f(n.bounds.h),
            "position" => LayerValue::Point(n.position.0, n.position.1),
            "position.x" => f(n.position.0),
            "position.y" => f(n.position.1),
            "anchorPoint" => LayerValue::Point(n.anchor.0, n.anchor.1),
            "anchorPoint.x" => f(n.anchor.0),
            "anchorPoint.y" => f(n.anchor.1),
            "zPosition" => f(n.z),
            "frame" => {
                drop(n);
                return Some(LayerValue::Rect(self.frame()));
            }
            "transform" => LayerValue::Transform(n.transform),
            "transform.scale" | "transform.scale.x" => f(decompose(&n.transform).sx),
            "transform.scale.y" => f(decompose(&n.transform).sy),
            "transform.scale.z" => f(decompose(&n.transform).sz),
            "transform.rotation" | "transform.rotation.z" => f(decompose(&n.transform).rot),
            "transform.translation.x" => f(n.transform.m[3][0]),
            "transform.translation.y" => f(n.transform.m[3][1]),
            "transform.translation.z" => f(n.transform.m[3][2]),
            "opacity" => f(n.opacity),
            "hidden" => LayerValue::Bool(n.hidden),
            "masksToBounds" => LayerValue::Bool(n.masks),
            "backgroundColor" => LayerValue::Color(n.background),
            "borderWidth" => f(n.border_width),
            "borderColor" => LayerValue::Color(n.border_color),
            "contents" => LayerValue::Image(n.contents.clone()),
            "contentsGravity" => LayerValue::Text(n.gravity.name().to_string()),
            "contentsScale" => f(n.contents_scale),
            "contentsRect" => LayerValue::Rect(n.contents_rect),
            _ => return n.extras.get(key).cloned(),
        })
    }

    /// Writes a property by key path (see [`Layer::value`]). A key the layer does not know is stored as an extra
    /// value, which observers see like any other change. Errors leave the layer unchanged.
    pub fn set_value(&self, key: &str, value: LayerValue) -> Result<(), LayerError> {
        use LayerValue as V;
        let mismatch = || LayerError::TypeMismatch(key.to_string());
        let float = |v: &V| if let V::Float(x) = v { Ok(*x) } else { Err(mismatch()) };
        match key {
            "name" => match value {
                V::Text(t) => self.set_name(&t),
                _ => return Err(mismatch()),
            },
            "bounds" => match value {
                V::Rect(r) => self.set_bounds(r),
                _ => return Err(mismatch()),
            },
            "bounds.origin" => match value {
                V::Point(x, y) => {
                    let b = self.bounds();
                    self.set_bounds(Rect { x, y, ..b });
                }
                _ => return Err(mismatch()),
            },
            "bounds.origin.x" => {
                let b = self.bounds();
                self.set_bounds(Rect { x: float(&value)?, ..b });
            }
            "bounds.origin.y" => {
                let b = self.bounds();
                self.set_bounds(Rect { y: float(&value)?, ..b });
            }
            "bounds.size" => match value {
                V::Size(w, h) => {
                    let b = self.bounds();
                    self.set_bounds(Rect { w, h, ..b });
                }
                _ => return Err(mismatch()),
            },
            "bounds.size.width" => {
                let b = self.bounds();
                self.set_bounds(Rect { w: float(&value)?, ..b });
            }
            "bounds.size.height" => {
                let b = self.bounds();
                self.set_bounds(Rect { h: float(&value)?, ..b });
            }
            "position" => match value {
                V::Point(x, y) => self.set_position(x, y),
                _ => return Err(mismatch()),
            },
            "position.x" => {
                let p = self.position();
                self.set_position(float(&value)?, p.1);
            }
            "position.y" => {
                let p = self.position();
                self.set_position(p.0, float(&value)?);
            }
            "anchorPoint" => match value {
                V::Point(x, y) => self.set_anchor_point(x, y),
                _ => return Err(mismatch()),
            },
            "anchorPoint.x" => {
                let a = self.anchor_point();
                self.set_anchor_point(float(&value)?, a.1);
            }
            "anchorPoint.y" => {
                let a = self.anchor_point();
                self.set_anchor_point(a.0, float(&value)?);
            }
            "zPosition" => self.set_z_position(float(&value)?),
            "frame" => match value {
                V::Rect(r) => self.set_frame(r),
                _ => return Err(mismatch()),
            },
            "transform" => match value {
                V::Transform(t) => self.set_transform(t),
                _ => return Err(mismatch()),
            },
            "transform.scale" => {
                let v = float(&value)?;
                self.update_transform(|d| {
                    d.sx = v;
                    d.sy = v;
                    d.sz = v;
                });
            }
            "transform.scale.x" => {
                let v = float(&value)?;
                self.update_transform(|d| d.sx = v);
            }
            "transform.scale.y" => {
                let v = float(&value)?;
                self.update_transform(|d| d.sy = v);
            }
            "transform.scale.z" => {
                let v = float(&value)?;
                self.update_transform(|d| d.sz = v);
            }
            "transform.rotation" | "transform.rotation.z" => {
                let v = float(&value)?;
                self.update_transform(|d| d.rot = v);
            }
            "transform.translation.x" => {
                let v = float(&value)?;
                self.update_transform(|d| d.tx = v);
            }
            "transform.translation.y" => {
                let v = float(&value)?;
                self.update_transform(|d| d.ty = v);
            }
            "transform.translation.z" => {
                let v = float(&value)?;
                self.update_transform(|d| d.tz = v);
            }
            "opacity" => self.set_opacity(float(&value)?),
            "hidden" => match value {
                V::Bool(b) => self.set_hidden(b),
                _ => return Err(mismatch()),
            },
            "masksToBounds" => match value {
                V::Bool(b) => self.set_masks_to_bounds(b),
                _ => return Err(mismatch()),
            },
            "backgroundColor" => match value {
                V::Color(c) => self.set_background_color((c != 0).then_some(c)),
                _ => return Err(mismatch()),
            },
            "borderWidth" => self.set_border_width(float(&value)?),
            "borderColor" => match value {
                V::Color(c) => self.set_border_color(c),
                _ => return Err(mismatch()),
            },
            "contents" => match value {
                V::Image(i) => self.set_contents(i),
                _ => return Err(mismatch()),
            },
            "contentsGravity" => match value {
                V::Text(t) => self.set_contents_gravity(ContentsGravity::from_name(&t).ok_or_else(|| LayerError::BadValue(t.clone()))?),
                _ => return Err(mismatch()),
            },
            "contentsScale" => self.set_contents_scale(float(&value)?),
            "contentsRect" => match value {
                V::Rect(r) => self.set_contents_rect(r),
                _ => return Err(mismatch()),
            },
            _ => self.set_extra(key, value),
        }
        Ok(())
    }

    fn update_transform(&self, f: impl FnOnce(&mut Decomposed)) {
        let mut d = decompose(&self.transform());
        f(&mut d);
        self.set_transform(d.build());
    }

    fn set_extra(&self, key: &str, value: LayerValue) {
        let old = self.0.borrow().extras.get(key).cloned();
        if old.as_ref() == Some(&value) {
            return;
        }
        let observers = {
            let mut n = self.0.borrow_mut();
            n.extras.insert(key.to_string(), value.clone());
            n.dirty = true;
            n.observers.clone()
        };
        let old = old.unwrap_or(LayerValue::Bool(false));
        for o in observers {
            o(self, key, &old, &value);
        }
    }

    /// The extra values stored under keys the layer does not know, in key order.
    pub fn extra_keys(&self) -> Vec<String> {
        self.0.borrow().extras.keys().cloned().collect()
    }

    // ---- flattening for a renderer -----------------------------------------------------------------------------

    /// Lays out and displays the tree as needed, then flattens it into render commands, parents first, sublayers
    /// back to front. Coordinates in the commands are in each layer's own space; a `Push` carries the transform to
    /// the superlayer's space (for the root, to the space the caller draws in).
    pub fn render_list(&self) -> Vec<LayerCmd> {
        self.layout_if_needed();
        self.display_if_needed();
        let mut out = Vec::new();
        self.emit(&mut out);
        out
    }

    fn emit(&self, out: &mut Vec<LayerCmd>) {
        let (hidden, opacity) = (self.is_hidden(), self.opacity());
        if hidden || opacity <= 0.0 {
            return;
        }
        let transform = self.affine_to_superlayer();
        let n = self.0.borrow();
        let b = n.bounds;
        out.push(LayerCmd::Push { transform, opacity });
        if n.masks {
            out.push(LayerCmd::PushClip(b));
        }
        if n.background != 0 {
            out.push(LayerCmd::Fill { rect: b, color: n.background });
        }
        if let Some(img) = &n.contents {
            let s = if n.contents_scale > 0.0 { n.contents_scale } else { 1.0 };
            let dst = gravity_rect(n.gravity, b, img.w as f32 / s, img.h as f32 / s);
            if !n.masks {
                out.push(LayerCmd::PushClip(b));
            }
            out.push(LayerCmd::Image { image: img.clone(), src: n.contents_rect, dst, alpha: 1.0 });
            if !n.masks {
                out.push(LayerCmd::PopClip);
            }
        }
        for op in &n.drawn {
            out.push(match op {
                DrawOp::FillRect { rect, color } => LayerCmd::Fill { rect: *rect, color: *color },
                DrawOp::StrokeRect { rect, width, color } => LayerCmd::Stroke { rect: *rect, width: *width, color: *color },
                DrawOp::Image { image, dst, alpha } => LayerCmd::Image { image: image.clone(), src: r_unit(), dst: *dst, alpha: *alpha },
            });
        }
        let (border_w, border_c, masks) = (n.border_width, n.border_color, n.masks);
        drop(n);
        for s in self.sublayers_by_z() {
            s.emit(out);
        }
        if border_w > 0.0 {
            out.push(LayerCmd::Stroke { rect: b, width: border_w, color: border_c });
        }
        if masks {
            out.push(LayerCmd::PopClip);
        }
        out.push(LayerCmd::Pop);
    }
}

/// One step of a flattened layer tree.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerCmd {
    /// Enter a layer: concatenate `transform` (local to superlayer space) and fade the layer as a group.
    Push { transform: AffineTransform, opacity: f32 },
    /// Leave the layer entered by the matching `Push`.
    Pop,
    /// Clip to a rectangle in the current space until the matching `PopClip`.
    PushClip(Rect),
    PopClip,
    Fill { rect: Rect, color: u32 },
    /// An outline of `width` drawn inside `rect`.
    Stroke { rect: Rect, width: f32, color: u32 },
    /// `src` is the shown part of the image in unit coordinates; `dst` is in the current space.
    Image { image: LayerImage, src: Rect, dst: Rect, alpha: f32 },
}

/// A transform split into scale, z rotation and translation (the 2D-style transforms layers use in practice).
struct Decomposed {
    sx: f32,
    sy: f32,
    sz: f32,
    rot: f32,
    tx: f32,
    ty: f32,
    tz: f32,
}

fn decompose(t: &Transform3D) -> Decomposed {
    let m = &t.m;
    Decomposed {
        sx: (m[0][0] * m[0][0] + m[0][1] * m[0][1]).sqrt(),
        sy: (m[1][0] * m[1][0] + m[1][1] * m[1][1]).sqrt(),
        sz: m[2][2],
        rot: m[0][1].atan2(m[0][0]),
        tx: m[3][0],
        ty: m[3][1],
        tz: m[3][2],
    }
}

impl Decomposed {
    fn build(&self) -> Transform3D {
        Transform3D::make_scale(self.sx, self.sy, self.sz)
            .concat(Transform3D::make_rotation(self.rot, 0.0, 0.0, 1.0))
            .concat(Transform3D::make_translation(self.tx, self.ty, self.tz))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn frame_round_trips_through_position_and_bounds() {
        let l = Layer::new();
        l.set_frame(r(10.0, 20.0, 100.0, 50.0));
        assert_eq!(l.bounds(), r(0.0, 0.0, 100.0, 50.0));
        assert_eq!(l.position(), (60.0, 45.0));
        assert_eq!(l.frame(), r(10.0, 20.0, 100.0, 50.0));
        l.set_anchor_point(0.0, 0.0);
        assert_eq!(l.frame(), r(60.0, 45.0, 100.0, 50.0));
    }

    #[test]
    fn transform_changes_the_frame_but_not_the_bounds() {
        let l = Layer::with_frame(r(0.0, 0.0, 100.0, 50.0));
        l.set_transform(Transform3D::make_scale(2.0, 2.0, 1.0));
        let f = l.frame();
        assert!(close(f.x, -50.0) && close(f.y, -25.0) && close(f.w, 200.0) && close(f.h, 100.0));
        assert_eq!(l.bounds(), r(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn sublayer_add_insert_remove_replace() {
        let p = Layer::new();
        let (a, b, c, d) = (Layer::new(), Layer::new(), Layer::new(), Layer::new());
        p.add_sublayer(&a);
        p.add_sublayer(&b);
        p.insert_sublayer(&c, 1);
        assert_eq!(p.sublayers(), vec![a.clone(), c.clone(), b.clone()]);
        assert_eq!(c.superlayer(), Some(p.clone()));
        p.insert_sublayer_above(&d, &a);
        p.insert_sublayer_below(&d, &b);
        assert_eq!(p.sublayers(), vec![a.clone(), c.clone(), d.clone(), b.clone()]);
        let e = Layer::new();
        p.replace_sublayer(&c, &e);
        assert_eq!(p.sublayers(), vec![a.clone(), e.clone(), d.clone(), b.clone()]);
        assert_eq!(c.superlayer(), None);
        b.remove_from_superlayer();
        assert_eq!(p.sublayers().len(), 3);
        // moving a layer to another parent removes it from the first
        let q = Layer::new();
        q.add_sublayer(&a);
        assert_eq!(p.sublayers().len(), 2);
        assert_eq!(a.superlayer(), Some(q));
        // re-adding at its own index keeps the order
        p.insert_sublayer(&e, 2);
        assert_eq!(p.sublayers(), vec![d.clone(), e.clone()]);
    }

    #[test]
    fn a_layer_cannot_become_its_own_ancestor() {
        let a = Layer::new();
        let b = Layer::new();
        a.add_sublayer(&b);
        b.add_sublayer(&a);
        assert_eq!(a.superlayer(), None);
        a.add_sublayer(&a);
        assert_eq!(a.sublayers(), vec![b]);
    }

    #[test]
    fn hit_testing_prefers_the_front_most_and_skips_hidden() {
        let root = Layer::with_frame(r(0.0, 0.0, 200.0, 200.0));
        let back = Layer::with_frame(r(10.0, 10.0, 100.0, 100.0));
        let front = Layer::with_frame(r(50.0, 50.0, 100.0, 100.0));
        root.add_sublayer(&back);
        root.add_sublayer(&front);
        assert_eq!(root.hit_test_local((60.0, 60.0)), Some(front.clone()));
        assert_eq!(root.hit_test_local((20.0, 20.0)), Some(back.clone()));
        front.set_hidden(true);
        assert_eq!(root.hit_test_local((60.0, 60.0)), Some(back.clone()));
        front.set_hidden(false);
        back.set_z_position(5.0);
        assert_eq!(root.hit_test_local((60.0, 60.0)), Some(back.clone()));
        assert_eq!(root.hit_test_local((300.0, 300.0)), None);
        // sublayer outside the parent's bounds is not hit
        let out = Layer::with_frame(r(250.0, 0.0, 10.0, 10.0));
        root.add_sublayer(&out);
        assert_eq!(root.hit_test_local((255.0, 5.0)), None);
        // hit_test takes superlayer coordinates
        assert_eq!(back.hit_test((20.0, 20.0)), Some(back.clone()));
    }

    #[test]
    fn coordinate_conversion_between_layers() {
        let root = Layer::with_frame(r(0.0, 0.0, 400.0, 400.0));
        let a = Layer::with_frame(r(100.0, 100.0, 100.0, 100.0));
        let b = Layer::with_frame(r(10.0, 20.0, 30.0, 30.0));
        root.add_sublayer(&a);
        a.add_sublayer(&b);
        let p = b.convert_point_to((1.0, 2.0), &root);
        assert!(close(p.0, 111.0) && close(p.1, 122.0));
        let q = b.convert_point_from((111.0, 122.0), &root);
        assert!(close(q.0, 1.0) && close(q.1, 2.0));
        let sib = Layer::with_frame(r(300.0, 0.0, 50.0, 50.0));
        root.add_sublayer(&sib);
        let s = b.convert_point_to((0.0, 0.0), &sib);
        assert!(close(s.0, -190.0) && close(s.1, 120.0));
        let rr = b.convert_rect_to(r(0.0, 0.0, 5.0, 5.0), &root);
        assert!(close(rr.x, 110.0) && close(rr.w, 5.0));
        // a scaled parent scales the child's points
        a.set_transform(Transform3D::make_scale(2.0, 2.0, 1.0));
        let p = b.convert_point_to((0.0, 0.0), &root);
        assert!(close(p.0, 100.0 - 50.0 + 20.0) && close(p.1, 100.0 - 50.0 + 40.0));
    }

    #[test]
    fn key_paths_read_and_write() {
        let l = Layer::with_frame(r(0.0, 0.0, 10.0, 10.0));
        l.set_value("position.x", LayerValue::Float(42.0)).unwrap();
        assert_eq!(l.position().0, 42.0);
        assert_eq!(l.value("position.x"), Some(LayerValue::Float(42.0)));
        l.set_value("bounds.size.width", LayerValue::Float(30.0)).unwrap();
        assert_eq!(l.bounds().w, 30.0);
        l.set_value("opacity", LayerValue::Float(0.5)).unwrap();
        assert_eq!(l.opacity(), 0.5);
        l.set_value("hidden", LayerValue::Bool(true)).unwrap();
        assert!(l.is_hidden());
        l.set_value("backgroundColor", LayerValue::Color(0xff11_2233)).unwrap();
        assert_eq!(l.background_color(), Some(0xff11_2233));
        l.set_value("contentsGravity", LayerValue::Text("center".into())).unwrap();
        assert_eq!(l.contents_gravity(), ContentsGravity::Center);
        assert!(matches!(l.set_value("contentsGravity", LayerValue::Text("nope".into())), Err(LayerError::BadValue(_))));
        assert!(matches!(l.set_value("opacity", LayerValue::Bool(true)), Err(LayerError::TypeMismatch(_))));
        assert_eq!(l.opacity(), 0.5);
        assert_eq!(l.value("missing"), None);
    }

    #[test]
    fn transform_key_paths_round_trip() {
        let l = Layer::new();
        l.set_value("transform.scale.x", LayerValue::Float(2.0)).unwrap();
        l.set_value("transform.rotation.z", LayerValue::Float(0.5)).unwrap();
        l.set_value("transform.translation.x", LayerValue::Float(7.0)).unwrap();
        let get = |k: &str| match l.value(k) {
            Some(LayerValue::Float(v)) => v,
            other => panic!("{k}: {other:?}"),
        };
        assert!(close(get("transform.scale.x"), 2.0));
        assert!(close(get("transform.scale.y"), 1.0));
        assert!(close(get("transform.rotation.z"), 0.5));
        assert!(close(get("transform.translation.x"), 7.0));
        assert!(close(get("transform.translation.y"), 0.0));
    }

    #[test]
    fn unknown_keys_are_kept_as_extras_and_observed() {
        let l = Layer::new();
        let seen: Rc<RefCell<Vec<(String, LayerValue, LayerValue)>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        l.add_observer(Rc::new(move |_: &Layer, k: &str, o: &LayerValue, n: &LayerValue| sink.borrow_mut().push((k.to_string(), o.clone(), n.clone()))));
        l.set_value("cornerRadius", LayerValue::Float(8.0)).unwrap();
        l.set_value("cornerRadius", LayerValue::Float(8.0)).unwrap();
        l.set_opacity(0.25);
        l.set_opacity(0.25);
        assert_eq!(l.value("cornerRadius"), Some(LayerValue::Float(8.0)));
        assert_eq!(l.extra_keys(), vec!["cornerRadius".to_string()]);
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1], ("opacity".to_string(), LayerValue::Float(1.0), LayerValue::Float(0.25)));
        assert!(Layer::is_builtin_key("position.x"));
        assert!(!Layer::is_builtin_key("cornerRadius"));
    }

    struct Painter {
        draws: Cell<u32>,
        laid_out: Cell<u32>,
    }
    impl LayerDelegate for Painter {
        fn draw_layer(&self, _l: &Layer, ctx: &mut LayerContext) {
            self.draws.set(self.draws.get() + 1);
            let (w, h) = ctx.size();
            ctx.fill_rect(r(0.0, 0.0, w, h), 0xff00_00ff);
        }
        fn layout_sublayers(&self, layer: &Layer) {
            self.laid_out.set(self.laid_out.get() + 1);
            for s in layer.sublayers() {
                s.set_frame(layer.bounds());
            }
        }
        fn action_for(&self, _l: &Layer, key: &str) -> Option<LayerAction> {
            (key == "position").then_some(LayerAction::Null)
        }
    }

    #[test]
    fn delegate_draws_lays_out_and_answers_actions() {
        let painter = Rc::new(Painter { draws: Cell::new(0), laid_out: Cell::new(0) });
        let l = Layer::with_frame(r(0.0, 0.0, 40.0, 20.0));
        l.set_delegate(Some(painter.clone()));
        let child = Layer::new();
        l.add_sublayer(&child);
        let cmds = l.render_list();
        assert_eq!(painter.draws.get(), 1);
        assert_eq!(painter.laid_out.get(), 1);
        assert_eq!(child.frame(), r(0.0, 0.0, 40.0, 20.0));
        assert!(cmds.iter().any(|c| matches!(c, LayerCmd::Fill { color: 0xff00_00ff, .. })));
        // nothing to redo until asked
        l.render_list();
        assert_eq!(painter.draws.get(), 1);
        assert_eq!(painter.laid_out.get(), 1);
        l.set_needs_display();
        l.set_needs_layout();
        assert!(l.needs_display() && l.needs_layout());
        l.layout_if_needed();
        l.display_if_needed();
        assert_eq!(painter.draws.get(), 2);
        assert_eq!(painter.laid_out.get(), 2);
        assert_eq!(l.action_for_key("position"), Some(LayerAction::Null));
        assert_eq!(l.action_for_key("opacity"), None);
        l.set_action("opacity", Some(LayerAction::Named("fade".into())));
        assert_eq!(l.action_for_key("opacity"), Some(LayerAction::Named("fade".into())));
        // growing the bounds asks for a layout
        l.set_bounds(r(0.0, 0.0, 80.0, 20.0));
        assert!(l.needs_layout());
    }

    #[test]
    fn render_list_orders_by_z_and_skips_hidden() {
        let root = Layer::with_frame(r(0.0, 0.0, 100.0, 100.0));
        root.set_background_color(Some(1));
        let a = Layer::with_frame(r(0.0, 0.0, 10.0, 10.0));
        a.set_background_color(Some(2));
        a.set_z_position(1.0);
        let b = Layer::with_frame(r(0.0, 0.0, 10.0, 10.0));
        b.set_background_color(Some(3));
        let c = Layer::with_frame(r(0.0, 0.0, 10.0, 10.0));
        c.set_background_color(Some(4));
        c.set_hidden(true);
        for l in [&a, &b, &c] {
            root.add_sublayer(l);
        }
        let colors: Vec<u32> = root.render_list().iter().filter_map(|c| if let LayerCmd::Fill { color, .. } = c { Some(*color) } else { None }).collect();
        assert_eq!(colors, vec![1, 3, 2]);
        let cmds = root.render_list();
        let pushes = cmds.iter().filter(|c| matches!(c, LayerCmd::Push { .. })).count();
        let pops = cmds.iter().filter(|c| matches!(c, LayerCmd::Pop)).count();
        assert_eq!((pushes, pops), (3, 3));
    }

    #[test]
    fn masks_border_and_image_contents() {
        let l = Layer::with_frame(r(0.0, 0.0, 20.0, 10.0));
        l.set_masks_to_bounds(true);
        l.set_border_width(2.0);
        l.set_contents(Some(LayerImage::new(2, 2, vec![0xffff_ffff; 4])));
        l.set_contents_gravity(ContentsGravity::Center);
        let cmds = l.render_list();
        assert!(matches!(cmds[1], LayerCmd::PushClip(_)));
        let img = cmds.iter().find_map(|c| if let LayerCmd::Image { dst, .. } = c { Some(*dst) } else { None }).unwrap();
        assert_eq!(img, r(9.0, 4.0, 2.0, 2.0));
        assert!(cmds.iter().any(|c| matches!(c, LayerCmd::Stroke { width, .. } if *width == 2.0)));
        assert!(matches!(cmds[cmds.len() - 2], LayerCmd::PopClip));
    }

    #[test]
    fn gravity_rects() {
        let b = r(0.0, 0.0, 100.0, 50.0);
        assert_eq!(gravity_rect(ContentsGravity::Resize, b, 10.0, 10.0), b);
        assert_eq!(gravity_rect(ContentsGravity::ResizeAspect, b, 10.0, 10.0), r(25.0, 0.0, 50.0, 50.0));
        assert_eq!(gravity_rect(ContentsGravity::ResizeAspectFill, b, 10.0, 10.0), r(0.0, -25.0, 100.0, 100.0));
        assert_eq!(gravity_rect(ContentsGravity::BottomRight, b, 10.0, 10.0), r(90.0, 40.0, 10.0, 10.0));
        assert_eq!(gravity_rect(ContentsGravity::TopLeft, b, 10.0, 10.0), r(0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn tree_dirty_tracks_changes() {
        let root = Layer::with_frame(r(0.0, 0.0, 10.0, 10.0));
        let child = Layer::new();
        root.add_sublayer(&child);
        assert!(root.tree_dirty());
        root.clear_dirty_tree();
        assert!(!root.tree_dirty());
        child.set_opacity(0.5);
        assert!(root.tree_dirty());
        root.clear_dirty_tree();
        child.set_opacity(0.5);
        assert!(!root.tree_dirty());
    }
}
