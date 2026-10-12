//! The `bifrost_layer_v1` server: a `bifrost_layers::Tree` per context (per surface), bindings of contents ids to
//! imported textures, pending transactions applied at frame start (atomically with the pending bindings and
//! damage), presentation trees evaluated at the frame's predicted presentation time, `presented` and
//! `animation_finished` events after the flip. Trees are kept in `ContextStore`, free of Smithay types for tests;
//! the Wayland dispatch glue lives next to it.
//!
//! Owner: wave G1 agent `wm-layers`.

use bifrost_layers::tree::{ApplyError, PresentedLayer};
use bifrost_layers::{ContentsId, Transaction, Tree};
use std::collections::HashMap;

/// One context's state.
#[derive(Debug, Default)]
pub struct Context {
    pub tree: Tree,
    /// contents id -> the key of the bound buffer's import (`import::Importer`).
    pub bindings: HashMap<ContentsId, u64>,
    pub pending: Vec<Transaction>,
    pub pending_bindings: Vec<(ContentsId, Option<u64>)>,
    /// Sequence numbers applied in the frame being composed, reported as `presented` after its flip.
    pub applied: Vec<u64>,
}

/// Every context, by an id the dispatch glue assigns.
#[derive(Debug, Default)]
pub struct ContextStore {
    pub contexts: HashMap<u32, Context>,
}

impl ContextStore {
    /// Queues a commit (decoded by the glue with `bifrost_layers::codec::decode`).
    pub fn commit(&mut self, ctx: u32, tx: Transaction) {
        let _ = (ctx, tx);
        todo!("wm-layers")
    }

    /// Applies every pending commit at tree time `now`; returns the contexts that failed (the glue posts a protocol
    /// error) and leaves the others applied.
    pub fn apply_pending(&mut self, now: f64) -> Vec<(u32, ApplyError)> {
        let _ = now;
        todo!("wm-layers")
    }

    /// The layers to draw for a context at the predicted presentation time.
    pub fn presentation(&self, ctx: u32, t: f64) -> Vec<PresentedLayer> {
        let _ = (ctx, t);
        todo!("wm-layers")
    }

    /// Whether any context animates at `t` (the clock keeps scheduling frames).
    pub fn animating(&self, t: f64) -> bool {
        let _ = t;
        todo!("wm-layers")
    }
}
