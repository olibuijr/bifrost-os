//! Progress tracking model (Foundation `Progress` parity): a tree of units of work with a total and a completed count,
//! `fractionCompleted`, `isFinished`, `isIndeterminate`, cancellation, pausing and user info.
//!
//! A child stands for `pending` units of its parent's total. The parent's fraction is its own completed units plus
//! each child's fraction times that child's pending units, over the parent's total, clamped to 0..=1. Work with a total
//! of zero or less is indeterminate: its fraction is 0 until `finish` is called. `finish` is a BifrOSt addition for
//! work whose size is never known; Foundation completes work by setting the completed count to the total.
//!
//! Cancel, pause and resume reach all descendants and fire each one's handler once. Not modelled here: publishing progress to other processes, the shared progress manager, value observation, time
//! estimates and throughput. The shell reads a tree through `fraction_completed` and `is_finished`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

/// Identifies a progress object. Ids are only meaningful for the [`ProgressTree`] that issued them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProgressId(usize);

/// A stored handler (`cancellationHandler`, `pausingHandler`, `resumingHandler`). Handlers are never compared.
#[derive(Clone, Default)]
struct Handler(Option<Rc<RefCell<dyn FnMut()>>>);

impl fmt::Debug for Handler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() { "Handler(set)" } else { "Handler(none)" })
    }
}

impl PartialEq for Handler {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Entry {
    on_cancel: Handler,
    on_pause: Handler,
    on_resume: Handler,
    total: i64,
    completed: i64,
    parent: Option<usize>,
    /// Units of the parent's total that this object stands for (0 for a root).
    pending: i64,
    children: Vec<usize>,
    finished: bool,
    cancellable: bool,
    cancelled: bool,
    pausable: bool,
    paused: bool,
    user_info: BTreeMap<String, String>,
}

/// An arena of progress objects that share one tree of parents and children.
#[derive(Clone, Debug, Default)]
pub struct ProgressTree {
    entries: Vec<Entry>,
    /// `becomeCurrent` stack: the object, its pending units, and whether an implicit child took them.
    current: Vec<(usize, i64, bool)>,
}

impl ProgressTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// `init(totalUnitCount:)`: a progress object that is not part of any tree yet.
    pub fn root(&mut self, total: i64) -> ProgressId {
        self.push(Entry { total, ..Entry::default() })
    }

    /// `init(totalUnitCount:parent:pendingUnitCount:)`: a child that reports `pending` units of `parent`'s work.
    /// A negative `pending` counts as zero.
    pub fn child(&mut self, parent: ProgressId, total: i64, pending: i64) -> ProgressId {
        let id = self.push(Entry { total, pending: pending.max(0), parent: Some(parent.0), ..Entry::default() });
        self.entries[parent.0].children.push(id.0);
        id
    }

    fn push(&mut self, entry: Entry) -> ProgressId {
        self.entries.push(entry);
        ProgressId(self.entries.len() - 1)
    }

    /// `totalUnitCount`.
    pub fn total_units(&self, id: ProgressId) -> i64 {
        self.entries[id.0].total
    }

    pub fn set_total_units(&mut self, id: ProgressId, total: i64) {
        self.entries[id.0].total = total;
    }

    /// `completedUnitCount`: the units this object finished itself, not counting its children.
    pub fn completed_units(&self, id: ProgressId) -> i64 {
        self.entries[id.0].completed
    }

    /// Sets the completed count; a negative count is stored as zero.
    pub fn set_completed_units(&mut self, id: ProgressId, units: i64) {
        self.entries[id.0].completed = units.max(0);
    }

    /// Adds finished units; a negative amount adds nothing.
    pub fn add_completed_units(&mut self, id: ProgressId, units: i64) {
        let e = &mut self.entries[id.0];
        e.completed = e.completed.saturating_add(units.max(0));
    }

    pub fn parent(&self, id: ProgressId) -> Option<ProgressId> {
        self.entries[id.0].parent.map(ProgressId)
    }

    /// The units of the parent's total this object stands for.
    pub fn pending_units(&self, id: ProgressId) -> i64 {
        self.entries[id.0].pending
    }

    /// `isIndeterminate`: a total of zero or less.
    pub fn is_indeterminate(&self, id: ProgressId) -> bool {
        self.entries[id.0].total <= 0
    }

    /// `fractionCompleted`: 1 once finished, 0 while indeterminate, otherwise the completed units plus each child's
    /// share over the total, clamped to 0..=1.
    pub fn fraction_completed(&self, id: ProgressId) -> f64 {
        let e = &self.entries[id.0];
        if e.finished {
            return 1.0;
        }
        if e.total <= 0 {
            return 0.0;
        }
        let from_children: f64 = e
            .children
            .iter()
            .map(|&c| self.fraction_completed(ProgressId(c)) * self.entries[c].pending as f64)
            .sum();
        ((e.completed as f64 + from_children) / e.total as f64).clamp(0.0, 1.0)
    }

    /// `isFinished`: `finish` was called, or a determinate total is reached through the completed units and children.
    pub fn is_finished(&self, id: ProgressId) -> bool {
        let e = &self.entries[id.0];
        e.finished || (e.total > 0 && self.fraction_completed(id) >= 1.0)
    }

    /// Completes indeterminate work (or any work) at once.
    pub fn finish(&mut self, id: ProgressId) {
        self.entries[id.0].finished = true;
    }

    pub fn set_cancellable(&mut self, id: ProgressId, on: bool) {
        self.entries[id.0].cancellable = on;
    }

    /// `isCancellable`.
    pub fn is_cancellable(&self, id: ProgressId) -> bool {
        self.entries[id.0].cancellable
    }

    /// `isCancelled`.
    pub fn is_cancelled(&self, id: ProgressId) -> bool {
        self.entries[id.0].cancelled
    }

    /// `cancel()`. True when this call cancelled the work. Only cancellable work that is neither cancelled nor
    /// finished can be cancelled. Cancelling reaches every descendant, cancellable or not; each one that was not
    /// cancelled yet is flagged and its cancellation handler fires once, the parent first.
    pub fn cancel(&mut self, id: ProgressId) -> bool {
        let e = &self.entries[id.0];
        if !e.cancellable || e.cancelled || self.is_finished(id) {
            return false;
        }
        let mut fired = Vec::new();
        for n in self.subtree(id.0) {
            let e = &mut self.entries[n];
            if !e.cancelled {
                e.cancelled = true;
                fired.extend(e.on_cancel.0.clone());
            }
        }
        Self::fire(fired);
        true
    }

    /// `cancellationHandler`: runs once when this object is cancelled, directly or through an ancestor.
    pub fn set_cancellation_handler(&mut self, id: ProgressId, f: impl FnMut() + 'static) {
        self.entries[id.0].on_cancel = Handler(Some(Rc::new(RefCell::new(f))));
    }

    /// `pausingHandler`: runs when this object is paused, directly or through an ancestor.
    pub fn set_pausing_handler(&mut self, id: ProgressId, f: impl FnMut() + 'static) {
        self.entries[id.0].on_pause = Handler(Some(Rc::new(RefCell::new(f))));
    }

    /// `resumingHandler`: runs when this object is resumed, directly or through an ancestor.
    pub fn set_resuming_handler(&mut self, id: ProgressId, f: impl FnMut() + 'static) {
        self.entries[id.0].on_resume = Handler(Some(Rc::new(RefCell::new(f))));
    }

    /// `id` and all its descendants, parents before children.
    fn subtree(&self, id: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.entries[n].children.iter().rev());
        }
        out
    }

    fn fire(handlers: Vec<Rc<RefCell<dyn FnMut()>>>) {
        for h in handlers {
            (h.borrow_mut())();
        }
    }

    /// `addChild(_:withPendingUnitCount:)`: attaches a root object as a child standing for `pending` units. Ignored
    /// (false) when `child` already has a parent or attaching would make a cycle.
    pub fn add_child(&mut self, parent: ProgressId, child: ProgressId, pending: i64) -> bool {
        if self.entries[child.0].parent.is_some() {
            return false;
        }
        let mut up = Some(parent.0);
        while let Some(n) = up {
            if n == child.0 {
                return false;
            }
            up = self.entries[n].parent;
        }
        self.entries[child.0].parent = Some(parent.0);
        self.entries[child.0].pending = pending.max(0);
        self.entries[parent.0].children.push(child.0);
        true
    }

    /// `Progress.current()`: the object most recently made current.
    pub fn current(&self) -> Option<ProgressId> {
        self.current.last().map(|&(n, _, _)| ProgressId(n))
    }

    /// `becomeCurrent(withPendingUnitCount:)`.
    pub fn become_current(&mut self, id: ProgressId, pending: i64) {
        self.current.push((id.0, pending.max(0), false));
    }

    /// `Progress(totalUnitCount:)` created while an object is current: the first one under a `become_current`
    /// attaches to it with the pending units; any other is a free root.
    pub fn implicit_child(&mut self, total: i64) -> ProgressId {
        let id = self.root(total);
        if let Some(&(parent, pending, false)) = self.current.last() {
            self.add_child(ProgressId(parent), id, pending);
            if let Some(top) = self.current.last_mut() {
                top.2 = true;
            }
        }
        id
    }

    /// `resignCurrent()`. When no implicit child took the pending units, the object completes them itself.
    pub fn resign_current(&mut self, id: ProgressId) {
        if self.current.last().map(|c| c.0) != Some(id.0) {
            return;
        }
        if let Some((n, pending, taken)) = self.current.pop() {
            if !taken {
                self.add_completed_units(ProgressId(n), pending);
            }
        }
    }

    pub fn set_pausable(&mut self, id: ProgressId, on: bool) {
        self.entries[id.0].pausable = on;
    }

    /// `isPausable`.
    pub fn is_pausable(&self, id: ProgressId) -> bool {
        self.entries[id.0].pausable
    }

    /// `isPaused`.
    pub fn is_paused(&self, id: ProgressId) -> bool {
        self.entries[id.0].paused
    }

    /// `pause()`. True when this call paused pausable work that is running (not cancelled and not finished).
    /// Descendants are paused too, pausable or not, each firing its pausing handler once.
    pub fn pause(&mut self, id: ProgressId) -> bool {
        let e = &self.entries[id.0];
        if !e.pausable || e.paused || e.cancelled || self.is_finished(id) {
            return false;
        }
        let mut fired = Vec::new();
        for n in self.subtree(id.0) {
            let e = &mut self.entries[n];
            if !e.paused {
                e.paused = true;
                fired.extend(e.on_pause.0.clone());
            }
        }
        Self::fire(fired);
        true
    }

    /// `resume()`. True when this call resumed paused work; descendants resume with it.
    pub fn resume(&mut self, id: ProgressId) -> bool {
        if !self.entries[id.0].paused {
            return false;
        }
        let mut fired = Vec::new();
        for n in self.subtree(id.0) {
            let e = &mut self.entries[n];
            if std::mem::replace(&mut e.paused, false) {
                fired.extend(e.on_resume.0.clone());
            }
        }
        Self::fire(fired);
        true
    }

    /// `setUserInfoObject(_:forKey:)`, for text values.
    pub fn set_user_info(&mut self, id: ProgressId, key: &str, value: &str) {
        self.entries[id.0].user_info.insert(key.to_string(), value.to_string());
    }

    /// `userInfo`, one value.
    pub fn user_info(&self, id: ProgressId, key: &str) -> Option<&str> {
        self.entries[id.0].user_info.get(key).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_determinate_root_fills_with_its_own_completed_units() {
        let mut t = ProgressTree::new();
        let p = t.root(4);
        assert!(!t.is_indeterminate(p));
        t.set_completed_units(p, 1);
        assert_eq!(t.fraction_completed(p), 0.25);
        assert!(!t.is_finished(p));
        t.add_completed_units(p, 3);
        assert_eq!(t.completed_units(p), 4);
        assert_eq!(t.fraction_completed(p), 1.0);
        assert!(t.is_finished(p));
    }

    #[test]
    fn children_report_their_pending_share_of_the_parent() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        let a = t.child(root, 2, 6);
        let b = t.child(root, 4, 4);
        assert_eq!(t.parent(a), Some(root));
        assert_eq!(t.pending_units(b), 4);
        t.set_completed_units(a, 1); // half of A's 6 units
        assert!((t.fraction_completed(root) - 0.3).abs() < 1e-9);
        t.set_completed_units(b, 4); // all of B's 4 units
        assert!((t.fraction_completed(root) - 0.7).abs() < 1e-9);
        assert!(!t.is_finished(root));
        t.set_completed_units(a, 2);
        assert!(t.is_finished(root), "the children's shares add up to the whole");
    }

    #[test]
    fn indeterminate_work_stays_at_zero_until_it_is_finished() {
        let mut t = ProgressTree::new();
        let p = t.root(0);
        t.add_completed_units(p, 50);
        assert!(t.is_indeterminate(p));
        assert_eq!(t.fraction_completed(p), 0.0);
        assert!(!t.is_finished(p));
        t.finish(p);
        assert_eq!(t.fraction_completed(p), 1.0);
        assert!(t.is_finished(p));
    }

    #[test]
    fn a_child_with_no_total_contributes_only_once_it_is_finished() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        let unknown = t.child(root, 0, 5);
        assert_eq!(t.fraction_completed(root), 0.0, "an indeterminate child adds nothing yet");
        t.finish(unknown);
        assert!((t.fraction_completed(root) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn counts_are_clamped_and_negative_inputs_add_nothing() {
        let mut t = ProgressTree::new();
        let root = t.root(4);
        t.set_completed_units(root, 9);
        assert_eq!(t.fraction_completed(root), 1.0, "over-completion stops at one");
        t.set_completed_units(root, -3);
        assert_eq!(t.completed_units(root), 0);
        let child = t.child(root, 2, -5);
        assert_eq!(t.pending_units(child), 0);
        t.set_completed_units(child, 2);
        assert_eq!(t.fraction_completed(root), 0.0, "a child with no pending units counts for nothing");
    }

    #[test]
    fn cancel_and_pause_need_the_capability_and_act_once() {
        let mut t = ProgressTree::new();
        let p = t.root(3);
        assert!(!t.cancel(p), "not cancellable by default");
        t.set_cancellable(p, true);
        assert!(t.cancel(p));
        assert!(t.is_cancelled(p));
        assert!(!t.cancel(p), "already cancelled");

        assert!(!t.pause(p), "not pausable by default");
        let q = t.root(3);
        t.set_pausable(q, true);
        assert!(t.pause(q) && t.is_paused(q));
        assert!(!t.pause(q), "already paused");
        assert!(t.resume(q) && !t.is_paused(q));
        assert!(!t.resume(q), "nothing to resume");
    }

    #[test]
    fn finished_work_cannot_be_cancelled_and_user_info_is_replaced_per_key() {
        let mut t = ProgressTree::new();
        let p = t.root(1);
        t.set_cancellable(p, true);
        t.add_completed_units(p, 1);
        assert!(!t.cancel(p), "a finished job has nothing left to cancel");
        assert!(!t.is_cancelled(p));

        assert_eq!(t.user_info(p, "file"), None);
        t.set_user_info(p, "file", "a.txt");
        t.set_user_info(p, "file", "b.txt");
        assert_eq!(t.user_info(p, "file"), Some("b.txt"));
        assert_eq!(t.user_info(p, "other"), None);
    }

    fn counter() -> (Rc<RefCell<u32>>, impl FnMut() + 'static) {
        let n = Rc::new(RefCell::new(0));
        let n2 = n.clone();
        (n, move || *n2.borrow_mut() += 1)
    }

    #[test]
    fn cancel_reaches_descendants_and_fires_each_handler_once() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        let a = t.child(root, 2, 5);
        let b = t.child(a, 2, 1); // not cancellable, still flagged
        t.set_cancellable(root, true);
        let (ca, fa) = counter();
        let (cb, fb) = counter();
        t.set_cancellation_handler(a, fa);
        t.set_cancellation_handler(b, fb);
        assert!(t.cancel(root));
        assert!(t.is_cancelled(a) && t.is_cancelled(b));
        assert!(!t.cancel(root));
        assert_eq!((*ca.borrow(), *cb.borrow()), (1, 1));
    }

    #[test]
    fn pause_and_resume_propagate_with_handlers() {
        let mut t = ProgressTree::new();
        let root = t.root(4);
        let kid = t.child(root, 2, 2);
        t.set_pausable(root, true);
        let (cp, fp) = counter();
        let (cr, fr) = counter();
        t.set_pausing_handler(kid, fp);
        t.set_resuming_handler(kid, fr);
        assert!(t.pause(root) && t.is_paused(kid));
        assert!(t.resume(root) && !t.is_paused(kid));
        assert_eq!((*cp.borrow(), *cr.borrow()), (1, 1));
    }

    #[test]
    fn become_current_attaches_the_implicit_child_with_pending_units() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        assert_eq!(t.current(), None);
        t.become_current(root, 4);
        assert_eq!(t.current(), Some(root));
        let kid = t.implicit_child(2);
        let free = t.implicit_child(2);
        t.resign_current(root);
        assert_eq!(t.current(), None);
        assert_eq!((t.parent(kid), t.pending_units(kid)), (Some(root), 4));
        assert_eq!(t.parent(free), None);
        assert_eq!(t.completed_units(root), 0);
    }

    #[test]
    fn resigning_without_a_child_completes_the_pending_units() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        t.become_current(root, 3);
        t.resign_current(root);
        assert_eq!(t.completed_units(root), 3);
    }

    #[test]
    fn add_child_attaches_roots_only_and_refuses_cycles() {
        let mut t = ProgressTree::new();
        let p = t.root(10);
        let c = t.root(2);
        assert!(t.add_child(p, c, 5));
        assert!(!t.add_child(p, c, 5), "already has a parent");
        assert!(!t.add_child(c, p, 1), "would loop");
        t.set_completed_units(c, 1);
        assert!((t.fraction_completed(p) - 0.25).abs() < 1e-9);
    }
}
