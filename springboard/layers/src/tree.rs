//! The tree: model layers, running animations, and the presentation tree evaluated at a time.
//!
//! Owner: wave G1 agent `layers-model` (structure, apply) with `layers-animation` (presentation evaluation).

use crate::animation::{contributions, ended_at, fill_of, write, Animation, AnimationId, FillMode};
use crate::geometry::{Affine, Point, Rect};
use crate::layer::{Layer, LayerId};
use crate::transaction::Transaction;
use std::collections::BTreeMap;

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
}

/// Why a transaction was refused (nothing is applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    UnknownLayer(LayerId),
    DuplicateLayer(LayerId),
    /// Inserting would make a layer its own ancestor.
    Cycle(LayerId),
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
        let _ = (tx, now);
        todo!("layers-model")
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

#[cfg(test)]
mod tests {
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
