//! Layer session: the app-facing entry point to a layer tree. It numbers transactions, applies each one atomically and
//! refuses a commit whose sequence number is not newer than the last committed one, so a duplicated or late batch cannot
//! overwrite newer state. Pure: the caller supplies the tree time, and the render side reads `presentation`.

use crate::transaction::Transaction;
use crate::tree::{ApplyError, PresentedLayer, Tree};

/// Why a commit was refused. A refused commit changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// The transaction's sequence number is not after the last one committed.
    Stale { last: u64, got: u64 },
    /// The tree refused one of the changes.
    Apply(ApplyError),
}

/// A layer tree with its commit order.
#[derive(Debug, Clone, Default)]
pub struct LayerSession {
    tree: Tree,
}

impl LayerSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sequence number the next transaction must carry.
    pub fn next_seq(&self) -> u64 {
        self.tree.seq + 1
    }

    /// An empty transaction numbered for the next commit.
    pub fn transaction(&self) -> Transaction {
        Transaction::new(self.next_seq())
    }

    /// Applies `tx` at tree time `now` and returns the sequence number now in effect.
    pub fn commit(&mut self, tx: &Transaction, now: f64) -> Result<u64, SessionError> {
        if tx.seq <= self.tree.seq {
            return Err(SessionError::Stale { last: self.tree.seq, got: tx.seq });
        }
        self.tree.apply(tx, now).map_err(SessionError::Apply)?;
        Ok(self.tree.seq)
    }

    /// The committed tree.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The layers to draw at tree time `t`, back to front.
    pub fn presentation(&self, t: f64) -> Vec<PresentedLayer> {
        self.tree.presentation(t)
    }
}
