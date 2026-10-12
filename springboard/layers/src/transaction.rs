//! Transactions: every change to a tree, applied atomically (the reference platform's transaction).
//!
//! Owner: wave G1 agent `layers-model`.

use crate::animation::{Animation, AnimationId};
use crate::layer::{LayerId, Property};
use crate::animation::Value;
use crate::layer::{Border, ContentsGravity, LayerContents, Material, Shadow};
use crate::timing::TimingFunction;

/// One change.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Create(LayerId),
    /// Destroys the layer and every sublayer below it.
    Destroy(LayerId),
    SetRoot(LayerId),
    /// Inserts `child` into `parent`'s sublayers at `index` (clamped), removing it from any previous parent.
    Insert { parent: LayerId, child: LayerId, index: u32 },
    RemoveFromSuperlayer(LayerId),
    /// Sets an animatable property; with an implicit animation set on the transaction, the change animates.
    Set { layer: LayerId, key: Property, value: Value },
    SetHidden(LayerId, bool),
    SetMasksToBounds(LayerId, bool),
    SetBorder(LayerId, Border),
    SetShadow(LayerId, Shadow),
    SetMaterial(LayerId, Material),
    SetContents(LayerId, LayerContents),
    SetContentsGravity(LayerId, ContentsGravity),
    AddAnimation { layer: LayerId, animation: Animation },
    RemoveAnimation { layer: LayerId, id: AnimationId },
    RemoveAllAnimations(LayerId),
}

/// An atomic batch of changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transaction {
    /// Increases with every commit of a context; echoed in `presented`.
    pub seq: u64,
    pub changes: Vec<Change>,
    /// Implicit animation: property `Set`s in this transaction animate from the presentation value over this
    /// duration (seconds) with this function. `None` disables implicit animation (the default for apps; the SDK
    /// enables it in animation blocks).
    pub implicit: Option<(f64, TimingFunction)>,
}

impl Transaction {
    pub fn new(seq: u64) -> Transaction {
        Transaction { seq, ..Transaction::default() }
    }

    pub fn push(&mut self, change: Change) -> &mut Transaction {
        self.changes.push(change);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}
