//! The tree: model layers, running animations, and the presentation tree evaluated at a time.
//!
//! Owner: wave G1 agent `layers-model` (structure, apply) with `layers-animation` (presentation evaluation).
//!
//! `apply` runs a whole transaction on a scratch copy of the tree and swaps the copy in only when every change was
//! accepted, so a refused transaction leaves nothing behind. Implicit animations are created here, from the value
//! the layer presents at the transaction's time to the new model value.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::discriminant;

use crate::animation::{contributions, ended_at, fill_of, write, Animation, AnimationId, FillMode, MediaTiming, Value};
use crate::geometry::{Affine, Point, Rect};
use crate::layer::{Layer, LayerId, Property};
use crate::timing::TimingFunction;
use crate::transaction::{Change, Transaction};

/// Ids from this value upwards belong to implicit animations; apps must keep their own ids below it.
pub const IMPLICIT_ID_BASE: u32 = 0x8000_0000;

/// Deepest sublayer nesting the presentation walks (a guard; `apply` already refuses cycles).
const MAX_DEPTH: usize = 512;

/// An animation running on a layer, with the tree time it began.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub layer: LayerId,
    pub animation: Animation,
    pub begin: f64,
}

/// A layer tree: the model (what the app last committed) and its running animations.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub layers: BTreeMap<LayerId, Layer>,
    pub root: Option<LayerId>,
    pub running: Vec<Running>,
    /// The last applied transaction's `seq`.
    pub seq: u64,
    /// Offset of the next implicit animation id from `IMPLICIT_ID_BASE`.
    implicit_seq: u32,
}

/// Why a transaction was refused (nothing is applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    UnknownLayer(LayerId),
    DuplicateLayer(LayerId),
    /// Inserting would make a layer its own ancestor.
    Cycle(LayerId),
    /// `LayerId(0)` is reserved and cannot be created.
    ZeroId,
    /// A value's kind does not match the property, or the property cannot be set or animated this way.
    BadValue { layer: LayerId, key: Property },
}

/// One layer as drawn: its model values with animations applied, its transform to the root, its clip.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedLayer {
    pub layer: Layer,
    /// From this layer's coordinate space to the root's.
    pub to_root: Affine,
    /// Accumulated opacity (own times every ancestor's).
    pub opacity: f32,
    /// The rounded clip of the nearest ancestor with `masks_to_bounds`, in root coordinates.
    pub clip: Option<(Rect, f32)>,
}

impl Tree {
    pub fn new() -> Tree {
        Tree::default()
    }

    /// Applies a transaction atomically at tree time `now` (implicit animations begin at `now`). On error nothing
    /// changes.
    pub fn apply(&mut self, tx: &Transaction, now: f64) -> Result<(), ApplyError> {
        // Validate and apply on a scratch copy; the copy replaces `self` only when every change was accepted.
        let mut scratch = self.clone();
        for change in &tx.changes {
            scratch.apply_change(change, tx.implicit, now)?;
        }
        scratch.seq = tx.seq;
        *self = scratch;
        Ok(())
    }

    /// The layers to draw at tree time `t`, back to front (painter's order: a layer, then its sublayers in order,
    /// sorted by `z_position` among siblings), hidden layers and their subtrees left out.
    pub fn presentation(&self, t: f64) -> Vec<PresentedLayer> {
        let Some(root) = self.root else { return Vec::new() };
        let anims = self.animations_by_layer();
        let mut out = Vec::new();
        if let Some(layer) = self.present_layer(root, &anims, t) {
            if !layer.hidden {
                self.emit(layer, Affine::IDENTITY, 1.0, None, &anims, t, 0, &mut out);
            }
        }
        out
    }

    /// Animations that ended by tree time `t`, removed from `running` (fill modes other than `Removed` keep their
    /// final value by writing it into the model first).
    pub fn take_finished(&mut self, t: f64) -> Vec<AnimationId> {
        let mut finished = Vec::new();
        let mut keep = Vec::with_capacity(self.running.len());
        for r in std::mem::take(&mut self.running) {
            if !ended_at(&r.animation, r.begin, t) {
                keep.push(r);
                continue;
            }
            if fill_of(&r.animation) != FillMode::Removed {
                if let Some(model) = self.layers.get(&r.layer).cloned() {
                    let values = contributions(&r.animation, r.begin, t, &model);
                    if let Some(layer) = self.layers.get_mut(&r.layer) {
                        for c in &values {
                            write(layer, c);
                        }
                    }
                }
            }
            finished.push(r.animation.id());
        }
        self.running = keep;
        finished
    }

    /// Whether anything animates at `t` (the renderer keeps scheduling frames while true). An animation that has
    /// not started yet counts; one that has ended does not, even while its fill holds a value.
    pub fn animating(&self, t: f64) -> bool {
        self.running.iter().any(|r| !ended_at(&r.animation, r.begin, t))
    }

    /// The root-space rectangle that changed between the presentations at `t0` and `t1` (damage).
    pub fn damage(&self, t0: f64, t1: f64) -> Rect {
        let before = self.presentation(t0);
        let after = self.presentation(t1);
        let before_by_id: BTreeMap<LayerId, &PresentedLayer> = before.iter().map(|p| (p.layer.id, p)).collect();
        let after_by_id: BTreeMap<LayerId, &PresentedLayer> = after.iter().map(|p| (p.layer.id, p)).collect();
        let mut out = Rect::ZERO;
        for (id, p) in &before_by_id {
            match after_by_id.get(id) {
                Some(q) if q == p => {}
                Some(q) => {
                    out = out.union(&presented_area(p)).union(&presented_area(q));
                }
                None => out = out.union(&presented_area(p)),
            }
        }
        for (id, q) in &after_by_id {
            if !before_by_id.contains_key(id) {
                out = out.union(&presented_area(q));
            }
        }
        out
    }

    /// Running animations grouped by the layer they act on, in the order they were added.
    fn animations_by_layer(&self) -> BTreeMap<LayerId, Vec<&Running>> {
        let mut map: BTreeMap<LayerId, Vec<&Running>> = BTreeMap::new();
        for r in &self.running {
            map.entry(r.layer).or_default().push(r);
        }
        map
    }

    /// A layer's model values with every running animation on it applied at `t`, in the order added (a later
    /// additive animation adds to the value so far; a later non-additive one replaces it).
    fn present_layer(&self, id: LayerId, anims: &BTreeMap<LayerId, Vec<&Running>>, t: f64) -> Option<Layer> {
        let model = self.layers.get(&id)?;
        let mut layer = model.clone();
        if let Some(list) = anims.get(&id) {
            for r in list {
                for c in contributions(&r.animation, r.begin, t, model) {
                    write(&mut layer, &c);
                }
            }
        }
        Some(layer)
    }

    /// Adds a presented layer and, after it, its visible sublayers (sorted by z among siblings).
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        layer: Layer,
        parent_to_root: Affine,
        parent_opacity: f32,
        ctx_clip: Option<(Rect, f32)>,
        anims: &BTreeMap<LayerId, Vec<&Running>>,
        t: f64,
        depth: usize,
        out: &mut Vec<PresentedLayer>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        let to_root = parent_to_root.concat(&layer.to_superlayer());
        let opacity = parent_opacity * layer.opacity;
        // The nearest masking ancestor clips this layer; a masking layer clips its own sublayers.
        let child_clip = if layer.masks_to_bounds { Some((bounds_in_root(&layer.bounds, &to_root), layer.corner_radius)) } else { ctx_clip };
        let sublayers = layer.sublayers.clone();
        out.push(PresentedLayer { layer, to_root, opacity, clip: ctx_clip });

        let mut kids: Vec<(f32, Layer)> = Vec::with_capacity(sublayers.len());
        for id in &sublayers {
            if let Some(kid) = self.present_layer(*id, anims, t) {
                if !kid.hidden {
                    kids.push((kid.z_position, kid));
                }
            }
        }
        // Stable sort: siblings with equal z keep their order.
        kids.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for (_, kid) in kids {
            self.emit(kid, to_root, opacity, child_clip, anims, t, depth + 1, out);
        }
    }

    fn apply_change(&mut self, change: &Change, implicit: Option<(f64, TimingFunction)>, now: f64) -> Result<(), ApplyError> {
        match change {
            Change::Create(id) => {
                if id.0 == 0 {
                    return Err(ApplyError::ZeroId);
                }
                if self.layers.contains_key(id) {
                    return Err(ApplyError::DuplicateLayer(*id));
                }
                self.layers.insert(*id, Layer::new(*id));
            }
            Change::Destroy(id) => {
                self.layer(*id)?;
                self.destroy(*id);
            }
            Change::SetRoot(id) => {
                self.layer(*id)?;
                self.root = Some(*id);
            }
            Change::Insert { parent, child, index } => self.insert(*parent, *child, *index)?,
            Change::RemoveFromSuperlayer(id) => {
                if let Some(sup) = self.layer(*id)?.superlayer {
                    if let Some(s) = self.layers.get_mut(&sup) {
                        s.sublayers.retain(|c| *c != *id);
                    }
                    self.layer_mut(*id)?.superlayer = None;
                }
            }
            Change::Set { layer, key, value } => self.set_animated(*layer, *key, *value, implicit, now)?,
            Change::SetHidden(id, v) => self.layer_mut(*id)?.hidden = *v,
            Change::SetMasksToBounds(id, v) => self.layer_mut(*id)?.masks_to_bounds = *v,
            Change::SetBorder(id, b) => self.layer_mut(*id)?.border = *b,
            Change::SetShadow(id, s) => self.set_animated(*id, Property::Shadow, Value::Shadow(*s), implicit, now)?,
            Change::SetMaterial(id, m) => self.layer_mut(*id)?.material = *m,
            Change::SetContents(id, c) => self.layer_mut(*id)?.contents = *c,
            Change::SetContentsGravity(id, g) => self.layer_mut(*id)?.contents_gravity = *g,
            Change::AddAnimation { layer, animation } => {
                self.layer(*layer)?;
                self.check_animation(*layer, animation)?;
                let begin = animation.timing().begin_time.unwrap_or(now);
                let aid = animation.id();
                // Adding an animation with an id the layer already runs replaces it.
                self.running.retain(|r| !(r.layer == *layer && r.animation.id() == aid));
                self.running.push(Running { layer: *layer, animation: animation.clone(), begin });
            }
            Change::RemoveAnimation { layer, id } => {
                self.layer(*layer)?;
                self.running.retain(|r| !(r.layer == *layer && r.animation.id() == *id));
            }
            Change::RemoveAllAnimations(layer) => {
                self.layer(*layer)?;
                self.running.retain(|r| r.layer != *layer);
            }
        }
        Ok(())
    }

    fn layer(&self, id: LayerId) -> Result<&Layer, ApplyError> {
        self.layers.get(&id).ok_or(ApplyError::UnknownLayer(id))
    }

    fn layer_mut(&mut self, id: LayerId) -> Result<&mut Layer, ApplyError> {
        self.layers.get_mut(&id).ok_or(ApplyError::UnknownLayer(id))
    }

    /// Removes a layer, its whole subtree, the animations on those layers, and any reference to them (root,
    /// superlayer's sublayer list).
    fn destroy(&mut self, id: LayerId) {
        let mut dead = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(l) = stack.pop() {
            if !dead.insert(l) {
                continue;
            }
            if let Some(layer) = self.layers.get(&l) {
                stack.extend(layer.sublayers.iter().copied());
            }
        }
        if let Some(sup) = self.layers.get(&id).and_then(|l| l.superlayer) {
            if let Some(s) = self.layers.get_mut(&sup) {
                s.sublayers.retain(|c| *c != id);
            }
        }
        for l in &dead {
            self.layers.remove(l);
        }
        self.running.retain(|r| !dead.contains(&r.layer));
        if self.root.is_some_and(|r| dead.contains(&r)) {
            self.root = None;
        }
    }

    /// Inserts `child` into `parent`'s sublayers at `index` (clamped), detaching it from its old superlayer first.
    fn insert(&mut self, parent: LayerId, child: LayerId, index: u32) -> Result<(), ApplyError> {
        self.layer(parent)?;
        self.layer(child)?;
        // A cycle is `child` standing at or above `parent`. Walking the superlayer chain terminates because the
        // tree is acyclic; the step bound only guards against a corrupted tree.
        let mut up = Some(parent);
        let mut steps = 0usize;
        while let Some(p) = up {
            if p == child {
                return Err(ApplyError::Cycle(child));
            }
            steps += 1;
            if steps > self.layers.len() {
                break;
            }
            up = self.layers.get(&p).and_then(|l| l.superlayer);
        }
        if let Some(old) = self.layer(child)?.superlayer {
            if let Some(o) = self.layers.get_mut(&old) {
                o.sublayers.retain(|c| *c != child);
            }
        }
        let p = self.layer_mut(parent)?;
        let at = (index as usize).min(p.sublayers.len());
        p.sublayers.insert(at, child);
        self.layer_mut(child)?.superlayer = Some(parent);
        Ok(())
    }

    /// Sets an animatable property. With an implicit animation on the transaction, the change animates from the
    /// presented value to the new one; the model holds the new value at once.
    fn set_animated(
        &mut self,
        id: LayerId,
        key: Property,
        to: Value,
        implicit: Option<(f64, TimingFunction)>,
        now: f64,
    ) -> Result<(), ApplyError> {
        let bad = ApplyError::BadValue { layer: id, key };
        if !key.animatable() {
            return Err(bad);
        }
        let model = self.layer(id)?.value(key).filter(|m| same_kind(m, &to)).ok_or(bad)?;
        if let Some((duration, function)) = implicit {
            let from = self.presented_value(id, key, now).unwrap_or(model);
            // A new implicit animation supersedes the layer's running animations of the same key.
            self.running.retain(|r| !(r.layer == id && top_key(&r.animation) == Some(key)));
            if from != to {
                let aid = AnimationId(IMPLICIT_ID_BASE.wrapping_add(self.implicit_seq));
                self.implicit_seq = self.implicit_seq.wrapping_add(1);
                let timing = MediaTiming { begin_time: Some(now), duration, ..MediaTiming::default() };
                let animation = Animation::Basic { id: aid, key, from: Some(from), to: Some(to), timing, function, additive: false };
                self.running.push(Running { layer: id, animation, begin: now });
            }
        }
        self.layer_mut(id)?.set_value(key, to);
        Ok(())
    }

    /// The value the layer shows at `t` (every running animation applied, as `presentation` draws it).
    fn presented_value(&self, id: LayerId, key: Property, t: f64) -> Option<Value> {
        self.present_layer(id, &self.animations_by_layer(), t)?.value(key)
    }

    /// Every animated value of an animation must be animatable and of the property's kind.
    fn check_animation(&self, layer: LayerId, anim: &Animation) -> Result<(), ApplyError> {
        match anim {
            Animation::Group { animations, .. } => {
                for a in animations {
                    self.check_animation(layer, a)?;
                }
                Ok(())
            }
            Animation::Basic { key, from, to, .. } | Animation::Spring { key, from, to, .. } => {
                self.check_values(layer, *key, from.iter().chain(to.iter()))
            }
            Animation::Keyframe { key, keyframes, .. } => self.check_values(layer, *key, keyframes.values.iter()),
        }
    }

    fn check_values<'a>(&self, layer: LayerId, key: Property, values: impl Iterator<Item = &'a Value>) -> Result<(), ApplyError> {
        let bad = ApplyError::BadValue { layer, key };
        if !key.animatable() {
            return Err(bad);
        }
        let model = self.layer(layer)?.value(key).ok_or(bad)?;
        if values.into_iter().all(|v| same_kind(v, &model)) {
            Ok(())
        } else {
            Err(ApplyError::BadValue { layer, key })
        }
    }
}

/// The axis-aligned bounds, in root space, of a layer-space rectangle under `m`.
fn bounds_in_root(r: &Rect, m: &Affine) -> Rect {
    let corners = [
        Point { x: r.x, y: r.y },
        Point { x: r.max_x(), y: r.y },
        Point { x: r.x, y: r.max_y() },
        Point { x: r.max_x(), y: r.max_y() },
    ]
    .map(|p| m.apply(p));
    let min_x = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let min_y = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_x = corners.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// The root-space area a presented layer paints: its bounds, padded by its shadow, cut to its clip.
fn presented_area(p: &PresentedLayer) -> Rect {
    let mut r = bounds_in_root(&p.layer.bounds, &p.to_root);
    if p.layer.shadow.opacity > 0.0 {
        let pad = p.layer.shadow.radius + p.layer.shadow.offset.x.abs().max(p.layer.shadow.offset.y.abs());
        r = Rect::new(r.x - pad, r.y - pad, r.w + 2.0 * pad, r.h + 2.0 * pad);
    }
    if let Some((clip, _)) = p.clip {
        r = intersect(&r, &clip);
    }
    r
}

fn intersect(a: &Rect, b: &Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let max_x = a.max_x().min(b.max_x());
    let max_y = a.max_y().min(b.max_y());
    if max_x <= x || max_y <= y {
        Rect::ZERO
    } else {
        Rect::new(x, y, max_x - x, max_y - y)
    }
}

/// Whether two values are of the same kind (the same `Value` variant).
fn same_kind(a: &Value, b: &Value) -> bool {
    discriminant(a) == discriminant(b)
}

/// The property a top-level (non-group) animation drives.
fn top_key(anim: &Animation) -> Option<Property> {
    match anim {
        Animation::Basic { key, .. } | Animation::Keyframe { key, .. } | Animation::Spring { key, .. } => Some(*key),
        Animation::Group { .. } => None,
    }
}



#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::geometry::{Color, Point};
    use crate::layer::{Border, ContentsGravity, ContentsId, LayerContents, Material, Shadow};

    fn lid(n: u32) -> LayerId {
        LayerId(n)
    }

    fn tx(seq: u64, changes: Vec<Change>) -> Transaction {
        Transaction { seq, changes, implicit: None }
    }

    /// Layers 1 (root) > 2 > 3, applied at seq 1.
    fn chain() -> Tree {
        let mut t = Tree::new();
        let t1 = tx(
            1,
            vec![
                Change::Create(lid(1)),
                Change::Create(lid(2)),
                Change::Create(lid(3)),
                Change::Insert { parent: lid(1), child: lid(2), index: 0 },
                Change::Insert { parent: lid(2), child: lid(3), index: 0 },
                Change::SetRoot(lid(1)),
            ],
        );
        t.apply(&t1, 0.0).unwrap();
        t
    }

    fn basic(id: u32, key: Property, to: Value) -> Animation {
        Animation::Basic {
            id: AnimationId(id),
            key,
            from: None,
            to: Some(to),
            timing: MediaTiming { duration: 1.0, ..MediaTiming::default() },
            function: TimingFunction::Linear,
            additive: false,
        }
    }

    #[test]
    fn create_insert_and_root_set_superlayer_bookkeeping() {
        let t = chain();
        assert_eq!(t.root, Some(lid(1)));
        assert_eq!(t.seq, 1);
        assert_eq!(t.layers.len(), 3);
        assert_eq!(t.layers[&lid(1)].sublayers, vec![lid(2)]);
        assert_eq!(t.layers[&lid(1)].superlayer, None);
        assert_eq!(t.layers[&lid(2)].superlayer, Some(lid(1)));
        assert_eq!(t.layers[&lid(2)].sublayers, vec![lid(3)]);
        assert_eq!(t.layers[&lid(3)].superlayer, Some(lid(2)));
    }

    #[test]
    fn a_duplicate_create_refuses_the_whole_transaction() {
        let mut t = Tree::new();
        let bad = tx(4, vec![Change::Create(lid(5)), Change::Create(lid(6)), Change::Create(lid(5))]);
        assert_eq!(t.apply(&bad, 0.0), Err(ApplyError::DuplicateLayer(lid(5))));
        assert!(t.layers.is_empty());
        assert_eq!(t.seq, 0);
    }

    #[test]
    fn an_unknown_layer_refuses_earlier_changes_in_the_same_transaction() {
        let mut t = Tree::new();
        let bad = tx(2, vec![Change::Create(lid(7)), Change::SetRoot(lid(9))]);
        assert_eq!(t.apply(&bad, 0.0), Err(ApplyError::UnknownLayer(lid(9))));
        assert!(!t.layers.contains_key(&lid(7)));
        assert_eq!(t.root, None);
    }

    #[test]
    fn layer_zero_is_reserved() {
        let mut t = Tree::new();
        assert_eq!(t.apply(&tx(1, vec![Change::Create(lid(0))]), 0.0), Err(ApplyError::ZeroId));
        assert!(t.layers.is_empty());
    }

    #[test]
    fn a_cycle_is_refused_and_leaves_the_tree_alone() {
        let mut t = chain();
        let before = t.clone();
        assert_eq!(t.apply(&tx(2, vec![Change::Insert { parent: lid(3), child: lid(1), index: 0 }]), 0.0), Err(ApplyError::Cycle(lid(1))));
        assert_eq!(t.apply(&tx(2, vec![Change::Insert { parent: lid(2), child: lid(2), index: 0 }]), 0.0), Err(ApplyError::Cycle(lid(2))));
        assert_eq!(t.layers, before.layers);
        assert_eq!(t.seq, 1);
    }

    #[test]
    fn destroy_is_recursive_and_detaches_from_the_superlayer() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::Destroy(lid(3))]), 0.0).unwrap();
        assert!(!t.layers.contains_key(&lid(3)));
        assert!(t.layers[&lid(2)].sublayers.is_empty());

        t.apply(&tx(3, vec![Change::Destroy(lid(2))]), 0.0).unwrap();
        assert_eq!(t.layers.keys().copied().collect::<Vec<_>>(), vec![lid(1)]);
        assert!(t.layers[&lid(1)].sublayers.is_empty());
        assert_eq!(t.root, Some(lid(1)));
    }

    #[test]
    fn destroying_the_root_clears_it_and_its_animations_go_too() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::AddAnimation { layer: lid(2), animation: basic(1, Property::Opacity, Value::Scalar(0.0)) }]), 0.0).unwrap();
        assert_eq!(t.running.len(), 1);
        t.apply(&tx(3, vec![Change::Destroy(lid(1))]), 0.0).unwrap();
        assert_eq!(t.root, None);
        assert!(t.layers.is_empty());
        assert!(t.running.is_empty());
    }

    #[test]
    fn moving_a_layer_detaches_it_and_clamps_the_index() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::Create(lid(4)), Change::Insert { parent: lid(1), child: lid(3), index: 99 }]), 0.0).unwrap();
        assert!(t.layers[&lid(2)].sublayers.is_empty());
        assert_eq!(t.layers[&lid(1)].sublayers, vec![lid(2), lid(3)]);
        assert_eq!(t.layers[&lid(3)].superlayer, Some(lid(1)));
        t.apply(&tx(3, vec![Change::Insert { parent: lid(1), child: lid(4), index: 0 }]), 0.0).unwrap();
        assert_eq!(t.layers[&lid(1)].sublayers, vec![lid(4), lid(2), lid(3)]);
    }

    #[test]
    fn remove_from_superlayer_keeps_the_layer() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::RemoveFromSuperlayer(lid(3))]), 0.0).unwrap();
        assert!(t.layers.contains_key(&lid(3)));
        assert_eq!(t.layers[&lid(3)].superlayer, None);
        assert!(t.layers[&lid(2)].sublayers.is_empty());
        // Removing a layer with no superlayer is a no-op.
        t.apply(&tx(3, vec![Change::RemoveFromSuperlayer(lid(1))]), 0.0).unwrap();
        assert_eq!(t.root, Some(lid(1)));
    }

    #[test]
    fn explicit_sets_write_the_model_without_animating() {
        let mut t = chain();
        let set = tx(
            2,
            vec![
                Change::Set { layer: lid(1), key: Property::Position, value: Value::Point(Point { x: 10.0, y: 20.0 }) },
                Change::Set { layer: lid(1), key: Property::Opacity, value: Value::Scalar(0.5) },
            ],
        );
        t.apply(&set, 3.0).unwrap();
        assert_eq!(t.layers[&lid(1)].position, Point { x: 10.0, y: 20.0 });
        assert_eq!(t.layers[&lid(1)].opacity, 0.5);
        assert!(t.running.is_empty());
    }

    #[test]
    fn a_wrong_value_kind_or_a_non_animatable_key_is_refused_atomically() {
        let mut t = chain();
        let mismatch = tx(
            2,
            vec![
                Change::Set { layer: lid(1), key: Property::Opacity, value: Value::Scalar(0.2) },
                Change::Set { layer: lid(1), key: Property::Position, value: Value::Scalar(1.0) },
            ],
        );
        assert_eq!(t.apply(&mismatch, 0.0), Err(ApplyError::BadValue { layer: lid(1), key: Property::Position }));
        assert_eq!(t.layers[&lid(1)].opacity, 1.0);
        let hidden = tx(3, vec![Change::Set { layer: lid(1), key: Property::Hidden, value: Value::Scalar(1.0) }]);
        assert_eq!(t.apply(&hidden, 0.0), Err(ApplyError::BadValue { layer: lid(1), key: Property::Hidden }));
        assert_eq!(t.seq, 1);
    }

    #[test]
    fn dedicated_setters_write_their_fields() {
        let mut t = chain();
        let border = Border { width: 1.0, color: Color::BLACK };
        let glass = Material::Glass { blur: 20.0, tint: Color::WHITE, saturation: 1.8 };
        let set = tx(
            2,
            vec![
                Change::SetHidden(lid(2), true),
                Change::SetMasksToBounds(lid(2), true),
                Change::SetBorder(lid(2), border),
                Change::SetMaterial(lid(2), glass),
                Change::SetContents(lid(2), LayerContents::Buffer(ContentsId(7))),
                Change::SetContentsGravity(lid(2), ContentsGravity::Center),
            ],
        );
        t.apply(&set, 0.0).unwrap();
        let l = &t.layers[&lid(2)];
        assert!(l.hidden);
        assert!(l.masks_to_bounds);
        assert_eq!(l.border, border);
        assert_eq!(l.material, glass);
        assert_eq!(l.contents, LayerContents::Buffer(ContentsId(7)));
        assert_eq!(l.contents_gravity, ContentsGravity::Center);
    }

    #[test]
    fn an_implicit_set_adds_a_basic_animation_from_the_old_value() {
        let mut t = chain();
        let mut implicit = tx(2, vec![Change::Set { layer: lid(1), key: Property::Opacity, value: Value::Scalar(0.5) }]);
        implicit.implicit = Some((0.25, TimingFunction::Linear));
        t.apply(&implicit, 10.0).unwrap();

        assert_eq!(t.layers[&lid(1)].opacity, 0.5);
        assert_eq!(t.running.len(), 1);
        let r = &t.running[0];
        assert_eq!(r.layer, lid(1));
        assert_eq!(r.begin, 10.0);
        assert!(r.animation.id().0 >= IMPLICIT_ID_BASE);
        match &r.animation {
            Animation::Basic { key, from, to, timing, function, additive, .. } => {
                assert_eq!(*key, Property::Opacity);
                assert_eq!(*from, Some(Value::Scalar(1.0)));
                assert_eq!(*to, Some(Value::Scalar(0.5)));
                assert_eq!(timing.begin_time, Some(10.0));
                assert_eq!(timing.duration, 0.25);
                assert_eq!(*function, TimingFunction::Linear);
                assert!(!additive);
            }
            other => panic!("expected a basic animation, got {other:?}"),
        }
    }

    #[test]
    fn an_implicit_set_to_the_same_value_adds_no_animation() {
        let mut t = chain();
        let mut same = tx(2, vec![Change::Set { layer: lid(1), key: Property::Opacity, value: Value::Scalar(1.0) }]);
        same.implicit = Some((0.25, TimingFunction::Linear));
        t.apply(&same, 0.0).unwrap();
        assert!(t.running.is_empty());
    }

    #[test]
    fn an_implicit_shadow_change_animates_the_shadow() {
        let mut t = chain();
        let s = Shadow { color: Color::BLACK, opacity: 0.5, radius: 8.0, offset: Point { x: 0.0, y: 2.0 } };
        let mut shadow = tx(2, vec![Change::SetShadow(lid(1), s)]);
        shadow.implicit = Some((0.2, TimingFunction::EaseOut));
        t.apply(&shadow, 3.0).unwrap();
        assert_eq!(t.layers[&lid(1)].shadow, s);
        assert_eq!(t.running.len(), 1);
        match &t.running[0].animation {
            Animation::Basic { key, from, to, function, .. } => {
                assert_eq!(*key, Property::Shadow);
                assert_eq!(*from, Some(Value::Shadow(Shadow::default())));
                assert_eq!(*to, Some(Value::Shadow(s)));
                assert_eq!(*function, TimingFunction::EaseOut);
            }
            other => panic!("expected a basic animation, got {other:?}"),
        }
    }

    #[test]
    fn a_dedicated_setter_without_implicit_does_not_animate() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::SetHidden(lid(1), true)]), 0.0).unwrap();
        assert!(t.layers[&lid(1)].hidden);
        assert!(t.running.is_empty());
    }

    #[test]
    fn add_replace_and_remove_animations() {
        let mut t = chain();
        let add1 = tx(2, vec![Change::AddAnimation { layer: lid(1), animation: basic(1, Property::Position, Value::Point(Point { x: 5.0, y: 5.0 })) }]);
        t.apply(&add1, 4.0).unwrap();
        assert_eq!(t.running.len(), 1);
        assert_eq!(t.running[0].begin, 4.0);

        // The same id on the same layer replaces the running entry.
        let replace = tx(3, vec![Change::AddAnimation { layer: lid(1), animation: basic(1, Property::Position, Value::Point(Point { x: 6.0, y: 6.0 })) }]);
        t.apply(&replace, 5.0).unwrap();
        assert_eq!(t.running.len(), 1);
        assert_eq!(t.running[0].begin, 5.0);
        assert_eq!(t.running[0].animation, basic(1, Property::Position, Value::Point(Point { x: 6.0, y: 6.0 })));

        t.apply(&tx(4, vec![Change::AddAnimation { layer: lid(2), animation: basic(1, Property::Opacity, Value::Scalar(0.0)) }]), 5.0).unwrap();
        assert_eq!(t.running.len(), 2);

        t.apply(&tx(5, vec![Change::RemoveAnimation { layer: lid(1), id: AnimationId(1) }]), 5.0).unwrap();
        assert_eq!(t.running.len(), 1);
        assert_eq!(t.running[0].layer, lid(2));

        // Removing an id the layer never ran is a no-op.
        t.apply(&tx(6, vec![Change::RemoveAnimation { layer: lid(2), id: AnimationId(99) }]), 5.0).unwrap();
        assert_eq!(t.running.len(), 1);

        t.apply(&tx(7, vec![Change::RemoveAllAnimations(lid(2))]), 5.0).unwrap();
        assert!(t.running.is_empty());
    }

    #[test]
    fn an_explicit_begin_time_sets_the_running_begin() {
        let mut t = chain();
        let timing = MediaTiming { begin_time: Some(12.5), ..Default::default() };
        let anim = Animation::Basic { id: AnimationId(3), key: Property::Opacity, from: None, to: Some(Value::Scalar(0.0)), timing, function: TimingFunction::Linear, additive: false };
        t.apply(&tx(2, vec![Change::AddAnimation { layer: lid(1), animation: anim }]), 1.0).unwrap();
        assert_eq!(t.running[0].begin, 12.5);
    }

    #[test]
    fn animations_with_the_wrong_kind_or_key_are_refused() {
        let mut t = chain();
        let wrong_kind = tx(2, vec![Change::AddAnimation { layer: lid(1), animation: basic(1, Property::Position, Value::Scalar(1.0)) }]);
        assert_eq!(t.apply(&wrong_kind, 0.0), Err(ApplyError::BadValue { layer: lid(1), key: Property::Position }));
        let hidden = tx(3, vec![Change::AddAnimation { layer: lid(1), animation: basic(1, Property::Hidden, Value::Scalar(1.0)) }]);
        assert_eq!(t.apply(&hidden, 0.0), Err(ApplyError::BadValue { layer: lid(1), key: Property::Hidden }));
        let missing = tx(4, vec![Change::AddAnimation { layer: lid(9), animation: basic(1, Property::Opacity, Value::Scalar(1.0)) }]);
        assert_eq!(t.apply(&missing, 0.0), Err(ApplyError::UnknownLayer(lid(9))));
        assert!(t.running.is_empty());
    }

    #[test]
    fn destroying_a_layer_drops_its_animations() {
        let mut t = chain();
        t.apply(&tx(2, vec![Change::AddAnimation { layer: lid(3), animation: basic(1, Property::Opacity, Value::Scalar(0.0)) }]), 0.0).unwrap();
        t.apply(&tx(3, vec![Change::Destroy(lid(2))]), 0.0).unwrap();
        assert!(t.running.is_empty());
    }

    #[test]
    fn implicit_ids_do_not_repeat() {
        let mut t = chain();
        let mut first = tx(2, vec![Change::Set { layer: lid(1), key: Property::Opacity, value: Value::Scalar(0.5) }]);
        first.implicit = Some((0.25, TimingFunction::Linear));
        t.apply(&first, 0.0).unwrap();
        let id1 = t.running[0].animation.id();
        // A second layer's implicit change gets a fresh id.
        let mut second = tx(3, vec![Change::Set { layer: lid(2), key: Property::Opacity, value: Value::Scalar(0.5) }]);
        second.implicit = Some((0.25, TimingFunction::Linear));
        t.apply(&second, 0.0).unwrap();
        let id2 = t.running[1].animation.id();
        assert_eq!(id1, AnimationId(IMPLICIT_ID_BASE));
        assert_eq!(id2, AnimationId(IMPLICIT_ID_BASE + 1));
    }

    #[test]
    fn an_empty_transaction_still_advances_the_sequence() {
        let mut t = chain();
        t.apply(&Transaction::new(9), 0.0).unwrap();
        assert_eq!(t.seq, 9);
        assert_eq!(t.layers.len(), 3);
    }
}


#[cfg(test)]
mod presentation_tests {
    use super::*;
    use crate::animation::{FillMode, MediaTiming, Value};
    use crate::layer::Property;
    use crate::timing::TimingFunction;

    fn layer(id: u32, sublayers: &[u32], f: impl FnOnce(&mut Layer)) -> Layer {
        let mut l = Layer::new(LayerId(id));
        l.sublayers = sublayers.iter().map(|s| LayerId(*s)).collect();
        f(&mut l);
        l
    }

    fn tree(layers: Vec<Layer>, root: u32) -> Tree {
        let mut t = Tree::new();
        for l in layers {
            t.layers.insert(l.id, l);
        }
        t.root = Some(LayerId(root));
        t
    }

    fn ids(list: &[PresentedLayer]) -> Vec<u32> {
        list.iter().map(|p| p.layer.id.0).collect()
    }

    fn find(list: &[PresentedLayer], id: u32) -> &PresentedLayer {
        list.iter().find(|p| p.layer.id == LayerId(id)).expect("layer presented")
    }

    fn opacity_running(layer: u32, from: f32, to: f32, timing: MediaTiming) -> Running {
        Running {
            layer: LayerId(layer),
            animation: Animation::Basic {
                id: AnimationId(100 + layer),
                key: Property::Opacity,
                from: Some(Value::Scalar(from)),
                to: Some(Value::Scalar(to)),
                timing,
                function: TimingFunction::Linear,
                additive: false,
            },
            begin: 0.0,
        }
    }

    fn position_running(layer: u32, from: Point, to: Point) -> Running {
        Running {
            layer: LayerId(layer),
            animation: Animation::Basic {
                id: AnimationId(200 + layer),
                key: Property::Position,
                from: Some(Value::Point(from)),
                to: Some(Value::Point(to)),
                timing: MediaTiming { duration: 1.0, ..MediaTiming::default() },
                function: TimingFunction::Linear,
                additive: false,
            },
            begin: 0.0,
        }
    }

    #[test]
    fn painter_order_is_parent_then_siblings_sorted_by_z() {
        // Root 1 has sublayers [2 (z 1), 3 (z -1), 4 (z 0)]; sorted by z: 3, 4, 2.
        let t = tree(
            vec![
                layer(1, &[2, 3, 4], |_| {}),
                layer(2, &[], |l| l.z_position = 1.0),
                layer(3, &[], |l| l.z_position = -1.0),
                layer(4, &[], |l| l.z_position = 0.0),
            ],
            1,
        );
        assert_eq!(ids(&t.presentation(0.0)), vec![1, 3, 4, 2]);
    }

    #[test]
    fn equal_z_keeps_the_sublayer_order_and_subtrees_follow_their_parent() {
        // 1 -> [2 -> [5], 3]; 2 and 3 both z 0, so 2 (with its child 5) comes first.
        let t = tree(vec![layer(1, &[2, 3], |_| {}), layer(2, &[5], |_| {}), layer(3, &[], |_| {}), layer(5, &[], |_| {})], 1);
        assert_eq!(ids(&t.presentation(0.0)), vec![1, 2, 5, 3]);
    }

    #[test]
    fn to_root_accumulates_positions_and_hidden_subtrees_are_left_out() {
        let t = tree(
            vec![
                layer(1, &[2, 6], |l| l.position = Point { x: 100.0, y: 0.0 }),
                layer(2, &[3], |l| {
                    l.position = Point { x: 10.0, y: 20.0 };
                    l.bounds = Rect::new(0.0, 0.0, 30.0, 30.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                }),
                layer(3, &[], |l| l.position = Point { x: 1.0, y: 1.0 }),
                layer(6, &[7], |l| l.hidden = true),
                layer(7, &[], |_| {}),
            ],
            1,
        );
        let p = t.presentation(0.0);
        assert_eq!(ids(&p), vec![1, 2, 3]);
        // Layer 2's origin lands at (110, 20); layer 3 is 1 further in each axis.
        assert_eq!(find(&p, 2).to_root.apply(Point::default()), Point { x: 110.0, y: 20.0 });
        assert_eq!(find(&p, 3).to_root.apply(Point::default()), Point { x: 111.0, y: 21.0 });
    }

    #[test]
    fn opacity_accumulates_down_the_tree() {
        let t = tree(
            vec![layer(1, &[2], |l| l.opacity = 0.5), layer(2, &[3], |l| l.opacity = 0.5), layer(3, &[], |l| l.opacity = 0.8)],
            1,
        );
        let p = t.presentation(0.0);
        assert_eq!(find(&p, 2).opacity, 0.25);
        assert!((find(&p, 3).opacity - 0.2).abs() < 1e-6);
    }

    #[test]
    fn masks_to_bounds_clips_the_sublayers_only_and_the_nearest_ancestor_wins() {
        // Layer 2 at (10, 10) masks its 100 x 50 bounds with radius 4; layer 3 and its child 4 inherit that clip.
        let t = tree(
            vec![
                layer(1, &[2], |_| {}),
                layer(2, &[3], |l| {
                    l.position = Point { x: 10.0, y: 10.0 };
                    l.bounds = Rect::new(0.0, 0.0, 100.0, 50.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                    l.masks_to_bounds = true;
                    l.corner_radius = 4.0;
                }),
                layer(3, &[4], |l| {
                    l.bounds = Rect::new(0.0, 0.0, 300.0, 300.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                }),
                layer(4, &[], |_| {}),
            ],
            1,
        );
        let p = t.presentation(0.0);
        assert_eq!(find(&p, 2).clip, None, "a layer is not clipped by its own mask");
        assert_eq!(find(&p, 3).clip, Some((Rect::new(10.0, 10.0, 100.0, 50.0), 4.0)));
        assert_eq!(find(&p, 4).clip, Some((Rect::new(10.0, 10.0, 100.0, 50.0), 4.0)));
    }

    #[test]
    fn basic_animation_is_evaluated_at_the_requested_time_and_removed_after() {
        let mut t = tree(vec![layer(1, &[2], |_| {}), layer(2, &[], |l| l.opacity = 1.0)], 1);
        t.running.push(opacity_running(2, 0.0, 1.0, MediaTiming { duration: 1.0, ..MediaTiming::default() }));
        assert_eq!(find(&t.presentation(0.5), 2).opacity, 0.5);
        assert_eq!(find(&t.presentation(0.25), 2).opacity, 0.25);
        // After the active time, the removed fill shows the model again (1.0).
        assert_eq!(find(&t.presentation(2.0), 2).opacity, 1.0);
        // Before the start, nothing is applied either.
        let mut late = t.clone();
        late.running[0].begin = 5.0;
        assert_eq!(find(&late.presentation(1.0), 2).opacity, 1.0);
    }

    #[test]
    fn animating_counts_pending_and_running_but_not_ended_animations() {
        let mut t = tree(vec![layer(1, &[], |_| {})], 1);
        let mut r = opacity_running(1, 0.0, 1.0, MediaTiming { duration: 1.0, ..MediaTiming::default() });
        r.begin = 2.0;
        t.running.push(r);
        assert!(t.animating(0.0), "not started yet");
        assert!(t.animating(2.5));
        assert!(!t.animating(3.5), "ended");
    }

    #[test]
    fn take_finished_writes_forwards_fill_into_the_model_and_drops_removed_ones() {
        let mut t = tree(vec![layer(1, &[2, 3], |_| {}), layer(2, &[], |l| l.opacity = 1.0), layer(3, &[], |l| l.opacity = 1.0)], 1);
        // Layer 2 fills forwards to 0.8; layer 3 is removed at its end and keeps its model value 1.0.
        t.running.push(opacity_running(2, 0.2, 0.8, MediaTiming { duration: 1.0, fill_mode: FillMode::Forwards, ..MediaTiming::default() }));
        t.running.push(opacity_running(3, 0.0, 0.0, MediaTiming { duration: 1.0, ..MediaTiming::default() }));
        assert!(t.take_finished(0.5).is_empty(), "nothing ends before 1 s");
        let done = t.take_finished(1.5);
        assert_eq!(done, vec![AnimationId(102), AnimationId(103)]);
        assert!(t.running.is_empty());
        assert!((t.layers[&LayerId(2)].opacity - 0.8).abs() < 1e-6, "forwards fill wrote the end value");
        assert_eq!(t.layers[&LayerId(3)].opacity, 1.0, "removed fill writes nothing");
        assert!(!t.animating(1.5));
    }

    #[test]
    fn damage_covers_the_old_and_new_place_of_a_moved_layer() {
        // Layer 2: 20 x 20 at anchor (0, 0), moving from (10, 10) to (50, 10) over 1 s.
        let mut t = tree(
            vec![
                layer(1, &[2], |_| {}),
                layer(2, &[], |l| {
                    l.bounds = Rect::new(0.0, 0.0, 20.0, 20.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                    l.position = Point { x: 10.0, y: 10.0 };
                }),
            ],
            1,
        );
        t.running.push(position_running(2, Point { x: 10.0, y: 10.0 }, Point { x: 50.0, y: 10.0 }));
        // At 0 the layer covers x 10..30; at 0.5 it covers x 30..50. Union: (10, 10) 40 x 20.
        assert_eq!(t.damage(0.0, 0.5), Rect::new(10.0, 10.0, 40.0, 20.0));
        // Nothing changes between two times after the animation has ended.
        assert_eq!(t.damage(2.0, 3.0), Rect::ZERO);
    }

    #[test]
    fn damage_of_a_changed_layer_is_cut_to_its_clip() {
        // Layer 3 (100 x 100, fading in) sits under layer 2, which clips to 10 x 10.
        let mut t = tree(
            vec![
                layer(1, &[2], |_| {}),
                layer(2, &[3], |l| {
                    l.bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                    l.masks_to_bounds = true;
                }),
                layer(3, &[], |l| {
                    l.bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
                    l.anchor_point = Point { x: 0.0, y: 0.0 };
                }),
            ],
            1,
        );
        t.running.push(opacity_running(3, 0.0, 1.0, MediaTiming { duration: 1.0, ..MediaTiming::default() }));
        assert_eq!(t.damage(0.0, 0.5), Rect::new(0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn empty_tree_presents_nothing() {
        assert!(Tree::new().presentation(0.0).is_empty());
        assert_eq!(Tree::new().damage(0.0, 1.0), Rect::ZERO);
        assert!(!Tree::new().animating(0.0));
    }
}
