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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerId;

    #[test]
    fn new_transaction_carries_its_sequence_and_no_changes() {
        let tx = Transaction::new(7);
        assert_eq!(tx.seq, 7);
        assert!(tx.is_empty());
        assert_eq!(tx.implicit, None);
    }

    #[test]
    fn default_transaction_is_sequence_zero_and_empty() {
        let tx = Transaction::default();
        assert_eq!(tx.seq, 0);
        assert!(tx.is_empty());
    }

    #[test]
    fn push_appends_in_order_and_chains() {
        let mut tx = Transaction::new(1);
        tx.push(Change::Create(LayerId(1))).push(Change::SetRoot(LayerId(1)));
        assert!(!tx.is_empty());
        assert_eq!(tx.changes, vec![Change::Create(LayerId(1)), Change::SetRoot(LayerId(1))]);
    }

    #[test]
    fn implicit_animation_settings_are_kept_on_the_transaction() {
        let mut tx = Transaction::new(2);
        tx.implicit = Some((0.3, TimingFunction::EaseInEaseOut));
        let copy = tx.clone();
        assert_eq!(copy, tx);
        assert_eq!(copy.implicit, Some((0.3, TimingFunction::EaseInEaseOut)));
    }

    #[test]
    fn changes_compare_by_their_fields() {
        let a = Change::Insert { parent: LayerId(1), child: LayerId(2), index: 0 };
        let b = Change::Insert { parent: LayerId(1), child: LayerId(2), index: 1 };
        assert_ne!(a, b);
        assert_eq!(a, a.clone());
    }

    #[test]
    fn an_empty_transaction_is_still_a_transaction() {
        let mut tx = Transaction::new(3);
        assert!(tx.is_empty());
        tx.push(Change::RemoveAllAnimations(LayerId(4)));
        assert!(!tx.is_empty());
        assert_eq!(tx.changes.len(), 1);
    }
}
