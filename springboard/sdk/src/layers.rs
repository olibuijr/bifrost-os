//! Layer-backed views for the direct path (docs/graphics-architecture.md, "SDK"; wave G2 row `sdk-layer-views`).
//!
//! A view that wants a layer (`LayerTree::sync` with `wants = true`) owns one `bifrost_layers` layer under the window's
//! root layer. Changes made during a frame are recorded in a `LayerBatch` and leave as transactions at the end of the
//! frame (`LayerTree::flush`). `LayerTree::animate` (and `LayerBatch::animate`) runs a closure with the transaction's
//! implicit animation set, so frame, opacity and background changes made inside it are interpolated by the window
//! server, not by the app. Hover and press are property changes (`set_highlight`), not redraws.
//!
//! On the shared-memory path (`Path::Shared`) nothing is recorded: `sync` returns `None`, `flush` returns nothing, and
//! every view redraws as it does today.
//!
//! The root layer is `ROOT`; the frame loop creates it (`wl::frame::frame_transaction`), so this module hands out ids
//! from 2 upwards. Owner: wave G2 agent `sdk-layer-views`.

use bifrost_layers::{Change, Color, LayerId, Point, Property, Rect, TimingFunction, Transaction, Value};
use std::collections::HashMap;

/// The window's root layer, created by the frame loop's first transaction.
pub const ROOT: LayerId = LayerId(1);

/// The first id this module hands out (`ROOT` is 1).
const FIRST_ID: u32 = 2;

/// Which path a view tree draws on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// Shared-memory frames through the shell: views redraw, no layers.
    Shared,
    /// Direct path: the window server composites the layer tree.
    Direct,
}

impl Path {
    /// `Direct` only when the direct path is requested (`BIFROST_DIRECT=1`) and the compositor offers
    /// `bifrost_layer_manager_v1`; anything else falls back to `Shared`.
    pub fn choose(requested: bool, layer_protocol: bool) -> Path {
        if requested && layer_protocol {
            Path::Direct
        } else {
            Path::Shared
        }
    }
}

/// The pointer state of a view. The tint strength is the fraction of the tint colour drawn over the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    Normal,
    Hover,
    Pressed,
}

impl Highlight {
    /// 0 at rest, 6% under the pointer, 12% while pressed.
    pub fn strength(self) -> f32 {
        match self {
            Highlight::Normal => 0.0,
            Highlight::Hover => 0.06,
            Highlight::Pressed => 0.12,
        }
    }
}

/// One frame's recorded changes. Each change keeps the implicit animation in force when it was recorded, so a single
/// frame can mix animated and immediate changes; `into_transactions` splits them where the setting changes.
#[derive(Debug, Default)]
pub struct LayerBatch {
    entries: Vec<(Option<(f64, TimingFunction)>, Change)>,
    implicit: Option<(f64, TimingFunction)>,
}

impl LayerBatch {
    pub fn new() -> LayerBatch {
        LayerBatch::default()
    }

    /// Records a change under the implicit animation in force now.
    pub fn push(&mut self, change: Change) {
        self.entries.push((self.implicit, change));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The implicit animation (duration in seconds, curve) that changes recorded now would carry.
    pub fn implicit(&self) -> Option<(f64, TimingFunction)> {
        self.implicit
    }

    /// Sets the implicit animation and returns the previous setting, so callers can restore it.
    pub fn set_implicit(&mut self, implicit: Option<(f64, TimingFunction)>) -> Option<(f64, TimingFunction)> {
        std::mem::replace(&mut self.implicit, implicit)
    }

    /// Runs `f` with the implicit animation set to `duration` and `curve`. The previous setting is restored afterwards,
    /// so nested blocks each apply their own timing and an outer block's timing returns after an inner one.
    pub fn animate<R>(&mut self, duration: f64, curve: TimingFunction, f: impl FnOnce(&mut LayerBatch) -> R) -> R {
        let outer = self.set_implicit(Some((duration, curve)));
        let result = f(self);
        self.set_implicit(outer);
        result
    }

    /// Groups the recorded changes into transactions in order. A new transaction starts whenever the implicit
    /// animation changes, because a transaction carries one setting. Sequence numbers run from `first_seq`.
    pub fn into_transactions(self, first_seq: u64) -> Vec<Transaction> {
        let mut out: Vec<Transaction> = Vec::new();
        for (implicit, change) in self.entries {
            let reuse = out.last().is_some_and(|tx| tx.implicit == implicit);
            if !reuse {
                let mut tx = Transaction::new(first_seq + out.len() as u64);
                tx.implicit = implicit;
                out.push(tx);
            }
            if let Some(tx) = out.last_mut() {
                tx.changes.push(change);
            }
        }
        out
    }
}

/// What the client last committed for one view's layer, so unchanged values record nothing.
#[derive(Debug)]
struct Entry {
    id: LayerId,
    frame: Option<Rect>,
    opacity: Option<f32>,
    background: Option<Color>,
}

/// The client side of the layer tree: one layer per view that wants one, with a per-frame batch.
#[derive(Debug)]
pub struct LayerTree {
    path: Path,
    next_id: u32,
    seq: u64,
    views: HashMap<u64, Entry>,
    batch: LayerBatch,
}

impl LayerTree {
    pub fn new(path: Path) -> LayerTree {
        LayerTree { path, next_id: FIRST_ID, seq: 0, views: HashMap::new(), batch: LayerBatch::new() }
    }

    pub fn path(&self) -> Path {
        self.path
    }

    pub fn is_direct(&self) -> bool {
        self.path == Path::Direct
    }

    /// The layer a view owns, if it has one.
    pub fn layer_of(&self, view: u64) -> Option<LayerId> {
        self.views.get(&view).map(|e| e.id)
    }

    /// The number of view layers alive.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// The batch of this frame, for changes the helpers below do not cover.
    pub fn batch(&mut self) -> &mut LayerBatch {
        &mut self.batch
    }

    /// Runs `f` with the implicit animation set (an implicit transaction): frame, opacity and background changes inside
    /// it are animated by the window server over `duration` seconds with `curve`.
    pub fn animate<R>(&mut self, duration: f64, curve: TimingFunction, f: impl FnOnce(&mut LayerTree) -> R) -> R {
        let outer = self.batch.set_implicit(Some((duration, curve)));
        let result = f(self);
        self.batch.set_implicit(outer);
        result
    }

    /// Makes a view's layer match `wants`: creates it (under `parent`, or the root) on first use, updates its frame,
    /// and destroys it when the view stops wanting one. Returns the view's layer, or `None` on the shared path or
    /// when the view does not want a layer.
    pub fn sync(&mut self, view: u64, parent: Option<LayerId>, frame: Rect, wants: bool) -> Option<LayerId> {
        if !self.is_direct() {
            return None;
        }
        if !wants {
            self.remove(view);
            return None;
        }
        if !self.views.contains_key(&view) {
            let id = LayerId(self.next_id);
            self.next_id += 1;
            self.batch.push(Change::Create(id));
            self.batch.push(Change::Insert { parent: parent.unwrap_or(ROOT), child: id, index: u32::MAX });
            self.views.insert(view, Entry { id, frame: None, opacity: None, background: None });
        }
        self.set_frame(view, frame);
        self.layer_of(view)
    }

    /// Sets a view's frame in its superlayer: bounds at the origin and position at the frame's centre (anchor 0.5).
    /// Records nothing when the frame is unchanged; returns whether it recorded a change.
    pub fn set_frame(&mut self, view: u64, frame: Rect) -> bool {
        let Some(entry) = self.views.get_mut(&view) else {
            return false;
        };
        if entry.frame == Some(frame) {
            return false;
        }
        entry.frame = Some(frame);
        let id = entry.id;
        self.batch.push(Change::Set {
            layer: id,
            key: Property::Bounds,
            value: Value::Rect(Rect { x: 0.0, y: 0.0, w: frame.w, h: frame.h }),
        });
        self.batch.push(Change::Set {
            layer: id,
            key: Property::Position,
            value: Value::Point(Point { x: frame.x + frame.w / 2.0, y: frame.y + frame.h / 2.0 }),
        });
        true
    }

    /// Sets a view's opacity; records nothing when unchanged.
    pub fn set_opacity(&mut self, view: u64, opacity: f32) -> bool {
        let Some(entry) = self.views.get_mut(&view) else {
            return false;
        };
        if entry.opacity == Some(opacity) {
            return false;
        }
        entry.opacity = Some(opacity);
        let id = entry.id;
        self.batch.push(Change::Set { layer: id, key: Property::Opacity, value: Value::Scalar(opacity) });
        true
    }

    /// Sets a view's background colour; records nothing when unchanged.
    pub fn set_background(&mut self, view: u64, color: Color) -> bool {
        let Some(entry) = self.views.get_mut(&view) else {
            return false;
        };
        if entry.background == Some(color) {
            return false;
        }
        entry.background = Some(color);
        let id = entry.id;
        self.batch.push(Change::Set { layer: id, key: Property::BackgroundColor, value: Value::Color(color) });
        true
    }

    /// Hover and press as a property change: the background becomes `tint` at the state's strength. Returns whether a
    /// change was recorded.
    pub fn set_highlight(&mut self, view: u64, highlight: Highlight, tint: Color) -> bool {
        let color = Color { r: tint.r, g: tint.g, b: tint.b, a: highlight.strength() };
        self.set_background(view, color)
    }

    /// Destroys a view's layer (and the layers below it). Returns whether the view had one.
    pub fn remove(&mut self, view: u64) -> bool {
        match self.views.remove(&view) {
            Some(entry) => {
                self.batch.push(Change::Destroy(entry.id));
                true
            }
            None => false,
        }
    }

    /// Ends the frame: the recorded changes as transactions, numbered after the last one sent. Empty when nothing
    /// changed or on the shared path.
    pub fn flush(&mut self) -> Vec<Transaction> {
        if self.batch.is_empty() {
            return Vec::new();
        }
        let batch = std::mem::take(&mut self.batch);
        let txs = batch.into_transactions(self.seq + 1);
        self.seq += txs.len() as u64;
        txs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn the_shared_path_records_nothing() {
        let mut tree = LayerTree::new(Path::Shared);
        assert!(!tree.is_direct());
        assert_eq!(tree.sync(1, None, frame(0.0, 0.0, 10.0, 10.0), true), None);
        assert!(!tree.set_frame(1, frame(0.0, 0.0, 20.0, 20.0)));
        assert!(tree.flush().is_empty());
        assert!(tree.is_empty());
    }

    #[test]
    fn the_path_needs_both_the_request_and_the_protocol() {
        assert_eq!(Path::choose(true, true), Path::Direct);
        assert_eq!(Path::choose(true, false), Path::Shared);
        assert_eq!(Path::choose(false, true), Path::Shared);
        assert_eq!(Path::choose(false, false), Path::Shared);
    }

    #[test]
    fn first_sync_creates_one_layer_under_the_root() {
        let mut tree = LayerTree::new(Path::Direct);
        let id = tree.sync(7, None, frame(10.0, 20.0, 100.0, 50.0), true);
        assert_eq!(id, Some(LayerId(2)));
        let txs = tree.flush();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].seq, 1);
        assert_eq!(
            txs[0].changes,
            vec![
                Change::Create(LayerId(2)),
                Change::Insert { parent: ROOT, child: LayerId(2), index: u32::MAX },
                Change::Set { layer: LayerId(2), key: Property::Bounds, value: Value::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)) },
                Change::Set { layer: LayerId(2), key: Property::Position, value: Value::Point(Point { x: 60.0, y: 45.0 }) },
            ]
        );
    }

    #[test]
    fn an_unchanged_frame_records_nothing() {
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(7, None, frame(0.0, 0.0, 10.0, 10.0), true);
        tree.flush();
        assert_eq!(tree.sync(7, None, frame(0.0, 0.0, 10.0, 10.0), true), Some(LayerId(2)));
        assert!(tree.flush().is_empty());
        assert!(tree.set_opacity(7, 0.5));
        assert!(!tree.set_opacity(7, 0.5), "a repeated opacity records nothing");
        assert!(tree.set_background(7, Color::CLEAR));
        assert!(!tree.set_background(7, Color::CLEAR));
        assert!(tree.set_frame(7, frame(5.0, 0.0, 10.0, 10.0)));
        assert_eq!(tree.flush().len(), 1);
    }

    #[test]
    fn a_view_that_stops_wanting_a_layer_loses_it() {
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(3, None, frame(0.0, 0.0, 4.0, 4.0), true);
        tree.flush();
        assert_eq!(tree.layer_of(3), Some(LayerId(2)));
        assert_eq!(tree.sync(3, None, frame(0.0, 0.0, 4.0, 4.0), false), None);
        assert_eq!(tree.layer_of(3), None);
        let txs = tree.flush();
        assert_eq!(txs[0].changes, vec![Change::Destroy(LayerId(2))]);
        assert!(!tree.remove(3));
    }

    #[test]
    fn ids_are_handed_out_in_order_and_children_go_under_their_parent() {
        let mut tree = LayerTree::new(Path::Direct);
        let parent = tree.sync(1, None, frame(0.0, 0.0, 100.0, 100.0), true).unwrap();
        let child = tree.sync(2, Some(parent), frame(0.0, 0.0, 10.0, 10.0), true).unwrap();
        assert_eq!((parent, child), (LayerId(2), LayerId(3)));
        let txs = tree.flush();
        assert!(txs[0].changes.contains(&Change::Insert { parent, child, index: u32::MAX }));
    }

    #[test]
    fn animate_sets_the_implicit_animation_only_inside_the_block() {
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(1, None, frame(0.0, 0.0, 10.0, 10.0), true);
        tree.animate(0.25, TimingFunction::EaseInEaseOut, |t| {
            t.set_frame(1, frame(50.0, 0.0, 10.0, 10.0));
        });
        tree.set_opacity(1, 0.5);
        let txs = tree.flush();
        // Create/Insert/Bounds/Position immediate, then the animated frame, then the immediate opacity.
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].implicit, None);
        assert_eq!(txs[1].implicit, Some((0.25, TimingFunction::EaseInEaseOut)));
        assert_eq!(txs[1].changes.len(), 2);
        assert_eq!(txs[2].implicit, None);
        assert_eq!(txs[2].changes, vec![Change::Set { layer: LayerId(2), key: Property::Opacity, value: Value::Scalar(0.5) }]);
        assert_eq!(tree.batch().implicit(), None);
    }

    #[test]
    fn nested_blocks_restore_the_outer_timing() {
        let mut batch = LayerBatch::new();
        batch.animate(0.3, TimingFunction::Linear, |b| {
            b.animate(0.1, TimingFunction::EaseIn, |b| {
                assert_eq!(b.implicit(), Some((0.1, TimingFunction::EaseIn)));
            });
            assert_eq!(b.implicit(), Some((0.3, TimingFunction::Linear)));
        });
        assert_eq!(batch.implicit(), None);
    }

    #[test]
    fn hover_and_press_are_background_property_changes() {
        let tint = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(9, None, frame(0.0, 0.0, 10.0, 10.0), true);
        tree.flush();
        // one frame per state: transactions group consecutive immediate changes, so hover and press in the same
        // frame would share one transaction (see `into_transactions`)
        assert!(tree.set_highlight(9, Highlight::Hover, tint));
        assert!(!tree.set_highlight(9, Highlight::Hover, tint), "the same state records nothing");
        let hover = tree.flush();
        assert!(tree.set_highlight(9, Highlight::Pressed, tint));
        let pressed = tree.flush();
        assert_eq!(hover.len(), 1);
        assert_eq!(pressed.len(), 1);
        let alpha = |tx: &Transaction| match tx.changes[0] {
            Change::Set { key: Property::BackgroundColor, value: Value::Color(c), .. } => c.a,
            _ => panic!("expected a background change"),
        };
        assert!((alpha(&hover[0]) - 0.06).abs() < 1e-6);
        assert!((alpha(&pressed[0]) - 0.12).abs() < 1e-6);
    }

    #[test]
    fn highlight_strengths_are_zero_six_and_twelve_percent() {
        assert_eq!(Highlight::Normal.strength(), 0.0);
        assert!((Highlight::Hover.strength() - 0.06).abs() < 1e-6);
        assert!((Highlight::Pressed.strength() - 0.12).abs() < 1e-6);
    }

    #[test]
    fn sequence_numbers_continue_across_frames() {
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(1, None, frame(0.0, 0.0, 10.0, 10.0), true);
        let first = tree.flush();
        tree.animate(0.2, TimingFunction::Default, |t| {
            t.set_opacity(1, 0.0);
        });
        let second = tree.flush();
        assert_eq!(first[0].seq, 1);
        assert_eq!(second[0].seq, 2);
        assert_eq!(second[0].implicit, Some((0.2, TimingFunction::Default)));
        assert!(tree.flush().is_empty());
    }

    #[test]
    fn grouping_splits_only_where_the_implicit_setting_changes() {
        let mut batch = LayerBatch::new();
        batch.push(Change::RemoveAllAnimations(LayerId(1)));
        batch.push(Change::RemoveAllAnimations(LayerId(2)));
        batch.animate(0.1, TimingFunction::Linear, |b| b.push(Change::RemoveAllAnimations(LayerId(3))));
        batch.animate(0.1, TimingFunction::Linear, |b| b.push(Change::RemoveAllAnimations(LayerId(4))));
        batch.push(Change::RemoveAllAnimations(LayerId(5)));
        let txs = batch.into_transactions(10);
        let lens: Vec<usize> = txs.iter().map(|t| t.changes.len()).collect();
        assert_eq!(lens, vec![2, 2, 1]);
        let seqs: Vec<u64> = txs.iter().map(|t| t.seq).collect();
        assert_eq!(seqs, vec![10, 11, 12]);
    }

    #[test]
    fn a_destroyed_view_frees_its_layer_only_for_new_views() {
        let mut tree = LayerTree::new(Path::Direct);
        tree.sync(1, None, frame(0.0, 0.0, 1.0, 1.0), true);
        tree.remove(1);
        // ids are never reused within one tree, so a late change for an old layer cannot hit a new view
        assert_eq!(tree.sync(2, None, frame(0.0, 0.0, 1.0, 1.0), true), Some(LayerId(3)));
    }
}
