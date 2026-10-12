//! The tree: model layers, running animations, and the presentation tree evaluated at a time.
//!
//! Owner: wave G1 agent `layers-model` (structure, apply) with `layers-animation` (presentation evaluation).

use crate::animation::{Animation, AnimationId};
use crate::geometry::{Affine, Rect};
use crate::layer::{Layer, LayerId};
use crate::transaction::Transaction;
use std::collections::BTreeMap;

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
}
