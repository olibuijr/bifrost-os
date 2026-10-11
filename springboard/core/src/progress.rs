//! Progress trees (`Progress`, `ProgressManager`, `Subprogress`): a job owns some units of work, and a parent gives a
//! child job a share of its own units. A job's fraction completed is its finished units plus each child's pending
//! share times that child's fraction, clamped to 1. Jobs live in one arena and are named by `ProgressId`, so the
//! tree needs no shared pointers. A caller that shares one across threads wraps it in its own lock.

/// Names a job in the tree that issued it. An id is only valid for the tree that created it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressId(usize);

#[derive(Debug)]
struct Node {
    total: u64,
    completed: u64,
    cancelled: bool,
    /// The parent job and how many of its units this job stands for.
    parent: Option<(ProgressId, u64)>,
    children: Vec<ProgressId>,
}

#[derive(Debug, Default)]
pub struct ProgressTree {
    nodes: Vec<Node>,
}

impl ProgressTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// A job with `total` units and no parent (`init(totalUnitCount:)`).
    pub fn root(&mut self, total: u64) -> ProgressId {
        self.push(Node { total, completed: 0, cancelled: false, parent: None, children: Vec::new() })
    }

    /// A job that stands for `pending` of `parent`'s units (`init(totalUnitCount:parent:pendingUnitCount:)`).
    /// A child of a cancelled job starts cancelled.
    pub fn child(&mut self, parent: ProgressId, total: u64, pending: u64) -> ProgressId {
        let cancelled = self.nodes[parent.0].cancelled;
        let id = self.push(Node { total, completed: 0, cancelled, parent: Some((parent, pending)), children: Vec::new() });
        self.nodes[parent.0].children.push(id);
        id
    }

    fn push(&mut self, node: Node) -> ProgressId {
        self.nodes.push(node);
        ProgressId(self.nodes.len() - 1)
    }

    pub fn total(&self, id: ProgressId) -> u64 {
        self.nodes[id.0].total
    }

    pub fn completed(&self, id: ProgressId) -> u64 {
        self.nodes[id.0].completed
    }

    /// Changes the job's own unit count (`totalUnitCount`). Finished units never exceed the new total.
    pub fn set_total(&mut self, id: ProgressId, total: u64) {
        let node = &mut self.nodes[id.0];
        node.total = total;
        node.completed = node.completed.min(total);
    }

    /// Finishes `units` more of the job's own work, never past its total.
    pub fn advance(&mut self, id: ProgressId, units: u64) {
        let node = &mut self.nodes[id.0];
        node.completed = node.completed.saturating_add(units).min(node.total);
    }

    /// Finishes all of the job's own units.
    pub fn finish(&mut self, id: ProgressId) {
        let node = &mut self.nodes[id.0];
        node.completed = node.total;
    }

    /// The fraction completed, from 0.0 to 1.0. A job with no units reports 0.0 until it has some.
    pub fn fraction(&self, id: ProgressId) -> f64 {
        let node = &self.nodes[id.0];
        if node.total == 0 {
            return 0.0;
        }
        let from_children: f64 = node
            .children
            .iter()
            .map(|&c| {
                let pending = self.nodes[c.0].parent.map_or(0, |(_, units)| units);
                pending as f64 * self.fraction(c)
            })
            .sum();
        ((node.completed as f64 + from_children) / node.total as f64).clamp(0.0, 1.0)
    }

    /// True once the fraction completed reaches 1.0 (`isFinished`).
    pub fn is_finished(&self, id: ProgressId) -> bool {
        self.fraction(id) >= 1.0
    }

    /// Cancels the job and every job under it (`cancel()` reaches the children).
    pub fn cancel(&mut self, id: ProgressId) {
        let mut stack = vec![id];
        while let Some(next) = stack.pop() {
            self.nodes[next.0].cancelled = true;
            stack.extend(self.nodes[next.0].children.iter().copied());
        }
    }

    pub fn is_cancelled(&self, id: ProgressId) -> bool {
        self.nodes[id.0].cancelled
    }

    pub fn parent(&self, id: ProgressId) -> Option<ProgressId> {
        self.nodes[id.0].parent.map(|(parent, _)| parent)
    }

    pub fn children(&self, id: ProgressId) -> &[ProgressId] {
        &self.nodes[id.0].children
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn own_units_drive_the_fraction_and_clamp_at_the_total() {
        let mut t = ProgressTree::new();
        let job = t.root(10);
        t.advance(job, 4);
        assert!(close(t.fraction(job), 0.4));
        assert!(!t.is_finished(job));
        t.advance(job, 100);
        assert_eq!(t.completed(job), 10, "finished units never pass the total");
        assert!(close(t.fraction(job), 1.0));
        assert!(t.is_finished(job));
    }

    #[test]
    fn a_child_share_scales_into_the_parent() {
        let mut t = ProgressTree::new();
        let parent = t.root(100);
        let child = t.child(parent, 10, 50);
        assert_eq!(t.parent(child), Some(parent));
        t.advance(child, 5);
        assert!(close(t.fraction(parent), 0.25), "half of the child's 50 units");
        t.finish(child);
        assert!(close(t.fraction(parent), 0.5));
        assert_eq!(t.children(parent), &[child]);
    }

    #[test]
    fn nested_children_roll_up_through_each_level() {
        let mut t = ProgressTree::new();
        let root = t.root(10);
        let mid = t.child(root, 4, 5);
        let grand = t.child(mid, 2, 2);
        t.finish(grand);
        assert!(close(t.fraction(mid), 0.5), "2 of the 4 units");
        assert!(close(t.fraction(root), 0.25), "5 of 10 units at half");
        assert!(!t.is_finished(root));
    }

    #[test]
    fn a_pending_share_larger_than_the_parent_is_clamped() {
        let mut t = ProgressTree::new();
        let parent = t.root(2);
        let child = t.child(parent, 1, 5);
        t.finish(child);
        assert!(close(t.fraction(parent), 1.0), "5 units of 2 would overshoot");
    }

    #[test]
    fn cancel_reaches_descendants_and_later_children() {
        let mut t = ProgressTree::new();
        let root = t.root(4);
        let mid = t.child(root, 2, 2);
        let grand = t.child(mid, 1, 1);
        assert!(!t.is_cancelled(grand));
        t.cancel(root);
        assert!(t.is_cancelled(root) && t.is_cancelled(mid) && t.is_cancelled(grand));
        let late = t.child(mid, 1, 1);
        assert!(t.is_cancelled(late), "a child added after cancelling starts cancelled");
    }

    #[test]
    fn zero_units_report_nothing_and_a_new_total_clamps_completed() {
        let mut t = ProgressTree::new();
        let empty = t.root(0);
        t.advance(empty, 5);
        assert_eq!(t.fraction(empty), 0.0);
        assert!(!t.is_finished(empty));
        let job = t.root(10);
        t.advance(job, 8);
        t.set_total(job, 4);
        assert_eq!(t.completed(job), 4);
        assert!(t.is_finished(job));
    }
}
