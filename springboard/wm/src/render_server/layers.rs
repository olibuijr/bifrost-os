//! The `bifrost_layer_v1` server: a `bifrost_layers::Tree` per context (per surface), bindings of contents ids to
//! imported textures, pending transactions applied at frame start (atomically with the pending bindings and
//! damage), presentation trees evaluated at the frame's predicted presentation time, `presented` and
//! `animation_finished` events after the flip. Trees are kept in `ContextStore`, free of Smithay types for tests;
//! the Wayland dispatch glue lives next to it.
//!
//! Owner: wave G1 agent `wm-layers`.

use bifrost_layers::tree::{ApplyError, PresentedLayer};
use bifrost_layers::{AnimationId, ContentsId, Transaction, Tree};
use std::collections::HashMap;

/// A damaged rectangle of a backing store, in buffer pixels: `(x, y, width, height)`.
pub type DamageRect = (i32, i32, i32, i32);

/// One context's state.
#[derive(Debug, Default)]
pub struct Context {
    pub tree: Tree,
    /// contents id -> the key of the bound buffer's import (`import::Importer`).
    pub bindings: HashMap<ContentsId, u64>,
    pub pending: Vec<Transaction>,
    /// Bindings received since the last applied commit; `None` unbinds. Applied with the next commit.
    pub pending_bindings: Vec<(ContentsId, Option<u64>)>,
    /// Damage received since the last applied commit, applied with it.
    pub pending_damage: Vec<(ContentsId, DamageRect)>,
    /// Sequence numbers applied in the frame being composed, reported as `presented` after its flip.
    pub applied: Vec<u64>,
    /// Damage applied in the frame being composed, drained by the compositor.
    pub damage: Vec<(ContentsId, DamageRect)>,
}

/// Every context, by an id the dispatch glue assigns.
#[derive(Debug, Default)]
pub struct ContextStore {
    pub contexts: HashMap<u32, Context>,
}

impl ContextStore {
    /// Creates a context. Returns `false` (and changes nothing) when `ctx` already exists, which the glue reports
    /// as `already_has_context`.
    pub fn create(&mut self, ctx: u32) -> bool {
        if self.contexts.contains_key(&ctx) {
            return false;
        }
        self.contexts.insert(ctx, Context::default());
        true
    }

    /// Removes a context (its surface was destroyed) and returns its state.
    pub fn remove(&mut self, ctx: u32) -> Option<Context> {
        self.contexts.remove(&ctx)
    }

    pub fn contains(&self, ctx: u32) -> bool {
        self.contexts.contains_key(&ctx)
    }

    /// Queues a bind (`Some(key)`) or unbind (`None`) of a contents id, applied with the next commit.
    /// Returns `false` for an unknown context.
    pub fn bind(&mut self, ctx: u32, contents: ContentsId, key: Option<u64>) -> bool {
        match self.contexts.get_mut(&ctx) {
            Some(c) => {
                c.pending_bindings.push((contents, key));
                true
            }
            None => false,
        }
    }

    /// Queues a damage rectangle for a backing store, applied with the next commit. Returns `false` for an unknown
    /// context.
    pub fn damage(&mut self, ctx: u32, contents: ContentsId, rect: DamageRect) -> bool {
        match self.contexts.get_mut(&ctx) {
            Some(c) => {
                c.pending_damage.push((contents, rect));
                true
            }
            None => false,
        }
    }

    /// The import key bound to a contents id of a context, if any.
    pub fn binding(&self, ctx: u32, contents: ContentsId) -> Option<u64> {
        self.contexts.get(&ctx).and_then(|c| c.bindings.get(&contents).copied())
    }

    /// Queues a commit (decoded by the glue with `bifrost_layers::codec::decode`). Commits to an unknown context
    /// create it, so the glue can order a commit ahead of its own bookkeeping without losing it.
    pub fn commit(&mut self, ctx: u32, tx: Transaction) {
        self.contexts.entry(ctx).or_default().pending.push(tx);
    }

    /// Applies every pending commit at tree time `now`; returns the contexts that failed (the glue posts a protocol
    /// error) and leaves the others applied.
    ///
    /// Per context the batch is atomic: the transactions are applied to a copy of the tree, and the pending bindings
    /// and damage are taken with them. A failing transaction discards the whole batch of that context (its bindings
    /// and damage too), and the context keeps its previous tree. Contexts are processed in id order.
    pub fn apply_pending(&mut self, now: f64) -> Vec<(u32, ApplyError)> {
        let mut ids: Vec<u32> = self
            .contexts
            .iter()
            .filter(|(_, c)| !c.pending.is_empty())
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();

        let mut failed = Vec::new();
        for id in ids {
            let ctx = match self.contexts.get_mut(&id) {
                Some(ctx) => ctx,
                None => continue,
            };
            if let Err(e) = apply_context(ctx, now) {
                ctx.pending.clear();
                ctx.pending_bindings.clear();
                ctx.pending_damage.clear();
                failed.push((id, e));
            }
        }
        failed
    }

    /// The layers to draw for a context at the predicted presentation time (empty for an unknown context).
    pub fn presentation(&self, ctx: u32, t: f64) -> Vec<PresentedLayer> {
        match self.contexts.get(&ctx) {
            Some(c) => c.tree.presentation(t),
            None => Vec::new(),
        }
    }

    /// Animations of a context that ended by `t`, removed from its tree (for `animation_finished`).
    pub fn take_finished(&mut self, ctx: u32, t: f64) -> Vec<AnimationId> {
        match self.contexts.get_mut(&ctx) {
            Some(c) => c.tree.take_finished(t),
            None => Vec::new(),
        }
    }

    /// Whether any context animates at `t` (the clock keeps scheduling frames).
    pub fn animating(&self, t: f64) -> bool {
        self.contexts.values().any(|c| c.tree.animating(t))
    }
}

/// Applies one context's pending batch. On error the context is left exactly as it was.
fn apply_context(ctx: &mut Context, now: f64) -> Result<(), ApplyError> {
    let mut tree = ctx.tree.clone();
    let mut seqs = Vec::with_capacity(ctx.pending.len());
    for tx in &ctx.pending {
        tree.apply(tx, now)?;
        seqs.push(tx.seq);
    }
    // Bindings and damage cannot fail: they are only committed after every transaction applied.
    for (contents, key) in ctx.pending_bindings.drain(..) {
        match key {
            Some(k) => {
                ctx.bindings.insert(contents, k);
            }
            None => {
                ctx.bindings.remove(&contents);
            }
        }
    }
    ctx.damage.extend(ctx.pending_damage.drain(..));
    ctx.tree = tree;
    ctx.pending.clear();
    ctx.applied.extend(seqs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_layers::{Change, LayerId};

    fn tx(seq: u64) -> Transaction {
        Transaction::new(seq)
    }

    #[test]
    fn create_refuses_a_second_context_for_the_same_id() {
        let mut s = ContextStore::default();
        assert!(s.create(7));
        assert!(!s.create(7));
        assert!(s.contains(7));
        assert_eq!(s.contexts.len(), 1);
    }

    #[test]
    fn remove_returns_the_context_and_forgets_it() {
        let mut s = ContextStore::default();
        s.create(3);
        s.commit(3, tx(1));
        let removed = s.remove(3).expect("context 3 exists");
        assert_eq!(removed.pending.len(), 1);
        assert!(!s.contains(3));
        assert!(s.remove(3).is_none());
    }

    #[test]
    fn commits_queue_in_order_per_context() {
        let mut s = ContextStore::default();
        s.create(1);
        s.create(2);
        s.commit(1, tx(1));
        s.commit(1, tx(2));
        s.commit(2, tx(10));
        let seqs: Vec<u64> = s.contexts[&1].pending.iter().map(|t| t.seq).collect();
        assert_eq!(seqs, vec![1, 2]);
        assert_eq!(s.contexts[&2].pending.len(), 1);
        assert!(s.contexts[&2].applied.is_empty());
    }

    #[test]
    fn a_commit_to_an_unknown_context_creates_it() {
        let mut s = ContextStore::default();
        s.commit(9, tx(4));
        assert!(s.contains(9));
        assert_eq!(s.contexts[&9].pending[0].seq, 4);
    }

    #[test]
    fn bind_and_unbind_queue_until_the_next_apply() {
        let mut s = ContextStore::default();
        s.create(1);
        assert!(s.bind(1, ContentsId(5), Some(100)));
        assert!(s.bind(1, ContentsId(5), None));
        assert!(s.bind(1, ContentsId(6), Some(101)));
        assert_eq!(
            s.contexts[&1].pending_bindings,
            vec![(ContentsId(5), Some(100)), (ContentsId(5), None), (ContentsId(6), Some(101))]
        );
        // Nothing is bound yet: bindings only take effect with a commit.
        assert_eq!(s.binding(1, ContentsId(6)), None);
    }

    #[test]
    fn bind_and_damage_refuse_an_unknown_context() {
        let mut s = ContextStore::default();
        assert!(!s.bind(4, ContentsId(1), Some(2)));
        assert!(!s.damage(4, ContentsId(1), (0, 0, 1, 1)));
        assert!(s.contexts.is_empty());
    }

    #[test]
    fn damage_queues_rectangles_in_order() {
        let mut s = ContextStore::default();
        s.create(1);
        assert!(s.damage(1, ContentsId(2), (0, 0, 10, 4)));
        assert!(s.damage(1, ContentsId(2), (5, 6, 7, 8)));
        assert_eq!(
            s.contexts[&1].pending_damage,
            vec![(ContentsId(2), (0, 0, 10, 4)), (ContentsId(2), (5, 6, 7, 8))]
        );
        assert!(s.contexts[&1].damage.is_empty());
    }

    #[test]
    fn unknown_context_reads_are_empty_and_idle() {
        let mut s = ContextStore::default();
        assert!(s.presentation(42, 1.0).is_empty());
        assert!(s.take_finished(42, 1.0).is_empty());
        assert_eq!(s.binding(42, ContentsId(1)), None);
        assert!(!s.animating(1.0), "an empty store has nothing animating");
    }

    #[test]
    fn apply_on_an_empty_store_reports_nothing() {
        let mut s = ContextStore::default();
        s.create(1);
        s.bind(1, ContentsId(1), Some(9));
        // No commit: nothing is pending, so nothing is applied and the binding waits.
        assert!(s.apply_pending(0.0).is_empty());
        assert_eq!(s.binding(1, ContentsId(1)), None);
        assert_eq!(s.contexts[&1].pending_bindings.len(), 1);
    }

    // The tests below need the tree model (`Tree::apply`, layers-model); they are ignored until that lands.

    #[test]
    fn apply_pending_applies_batches_in_order_and_records_seqs() {
        let mut s = ContextStore::default();
        s.create(1);
        let mut first = tx(1);
        first.push(Change::Create(LayerId(1))).push(Change::SetRoot(LayerId(1)));
        s.commit(1, first);
        s.commit(1, tx(2));
        assert!(s.apply_pending(0.0).is_empty());
        let c = &s.contexts[&1];
        assert!(c.pending.is_empty());
        assert_eq!(c.applied, vec![1, 2]);
        assert_eq!(c.tree.root, Some(LayerId(1)));
    }

    #[test]
    fn bindings_apply_atomically_with_the_transaction() {
        let mut s = ContextStore::default();
        s.create(1);
        s.bind(1, ContentsId(3), Some(77));
        s.damage(1, ContentsId(3), (0, 0, 2, 2));
        // Bindings wait for a commit: nothing bound before apply_pending.
        s.commit(1, tx(1));
        assert_eq!(s.binding(1, ContentsId(3)), None);
        assert!(s.apply_pending(0.0).is_empty());
        assert_eq!(s.binding(1, ContentsId(3)), Some(77));
        assert_eq!(s.contexts[&1].damage, vec![(ContentsId(3), (0, 0, 2, 2))]);
        assert!(s.contexts[&1].pending_bindings.is_empty());
        assert!(s.contexts[&1].pending_damage.is_empty());
    }

    #[test]
    fn a_failing_context_keeps_its_old_state_and_others_still_apply() {
        let mut s = ContextStore::default();
        s.create(1);
        s.create(2);
        // Context 1: a good commit, then a bad one (Set on a layer that does not exist). The whole batch fails.
        s.bind(1, ContentsId(1), Some(5));
        s.commit(1, tx(1));
        let mut bad = tx(2);
        bad.push(Change::Set {
            layer: LayerId(99),
            key: bifrost_layers::Property::Opacity,
            value: bifrost_layers::Value::Scalar(0.5),
        });
        s.commit(1, bad);
        // Context 2 applies normally.
        s.commit(2, tx(7));

        let failed = s.apply_pending(0.0);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, 1);
        assert_eq!(failed[0].1, ApplyError::UnknownLayer(LayerId(99)));
        assert_eq!(s.binding(1, ContentsId(1)), None, "a failed batch binds nothing");
        assert!(s.contexts[&1].applied.is_empty());
        assert!(s.contexts[&1].pending.is_empty());
        assert_eq!(s.contexts[&2].applied, vec![7]);
    }
}
