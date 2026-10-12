//! The tree: model layers, running animations, and the presentation tree evaluated at a time.
//!
//! Owner: wave G1 agent `layers-model` (structure, apply) with `layers-animation` (presentation evaluation).
//!
//! `apply` runs a whole transaction on a scratch copy of the tree and swaps the copy in only when every change was
//! accepted, so a refused transaction leaves nothing behind. Implicit animations are created here, from the value
//! the layer presents at the transaction's time to the new model value.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::discriminant;

use crate::animation::{Animation, AnimationId, Keyframes, MediaTiming, Value};
use crate::geometry::{Affine, Rect};
use crate::layer::{Layer, LayerId, Property};
use crate::timing::TimingFunction;
use crate::transaction::{Change, Transaction};

/// Ids from this value upwards belong to implicit animations; apps must keep their own ids below it.
pub const IMPLICIT_ID_BASE: u32 = 0x8000_0000;

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
        let _ = t;
        todo!("layers-animation")
    }

    /// Animations that ended by tree time `t`, removed from `running` (fill modes other than `Removed` keep their
    /// final value by writing it into the model first).
    pub fn take_finished(&mut self, t: f64) -> Vec<AnimationId> {
        let _ = t;
        todo!("layers-animation")
    }

    /// Whether anything animates at `t` (the renderer keeps scheduling frames while true).
    pub fn animating(&self, t: f64) -> bool {
        let _ = t;
        todo!("layers-animation")
    }

    /// The root-space rectangle that changed between the presentations at `t0` and `t1` (damage).
    pub fn damage(&self, t0: f64, t1: f64) -> Rect {
        let _ = (t0, t1);
        todo!("layers-animation")
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

    /// The value the layer shows at `t`: the last running animation of `key` that is active, else the model value.
    fn presented_value(&self, id: LayerId, key: Property, t: f64) -> Option<Value> {
        let model = self.layers.get(&id)?.value(key);
        let mut value = None;
        for r in self.running.iter().filter(|r| r.layer == id) {
            if let Some(v) = sample(&r.animation, key, r.begin, t, model) {
                value = Some(v);
            }
        }
        value.or(model)
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

/// The value an animation gives `key` at tree time `t`, or `None` when it does not drive `key` or is not active.
/// A basic or spring animation without an explicit end uses the model value.
fn sample(anim: &Animation, key: Property, begin: f64, t: f64, model: Option<Value>) -> Option<Value> {
    match anim {
        Animation::Basic { key: k, from, to, timing, function, .. } if *k == key => {
            let p = timing.progress(begin, t)?;
            Some(from.or(model)?.lerp(&to.or(model)?, function.eval(p)))
        }
        Animation::Keyframe { key: k, keyframes, timing, .. } if *k == key => {
            let p = timing.progress(begin, t)?;
            keyframe_value(keyframes, p)
        }
        Animation::Spring { key: k, from, to, spring, .. } if *k == key => {
            let elapsed = t - begin;
            if elapsed < 0.0 || elapsed > f64::from(spring.settling_duration()) {
                return None;
            }
            let p = spring.progress(elapsed as f32);
            Some(from.or(model)?.lerp(&to.or(model)?, p))
        }
        Animation::Group { animations, .. } => animations.iter().fold(None, |acc, a| sample(a, key, begin, t, model).or(acc)),
        _ => None,
    }
}

/// The keyframe value at local progress `p` (0..1): the segment containing `p`, interpolated with its timing function.
fn keyframe_value(kf: &Keyframes, p: f32) -> Option<Value> {
    let n = kf.values.len().min(kf.key_times.len());
    if n == 0 {
        return None;
    }
    if n == 1 || p <= kf.key_times[0] {
        return Some(kf.values[0]);
    }
    let last = n - 1;
    for i in 0..last {
        let (t0, t1) = (kf.key_times[i], kf.key_times[i + 1]);
        if p <= t1 {
            let span = t1 - t0;
            let local = if span <= 0.0 { 1.0 } else { ((p - t0) / span).clamp(0.0, 1.0) };
            let f = kf.timing_functions.get(i).copied().unwrap_or(TimingFunction::Linear);
            return Some(kf.values[i].lerp(&kf.values[i + 1], f.eval(local)));
        }
    }
    Some(kf.values[last])
}

#[cfg(test)]
mod tests {
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
        let mut timing = MediaTiming::default();
        timing.begin_time = Some(12.5);
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
