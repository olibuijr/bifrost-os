//! Progress tracking for apps (the reference SDK's `Progress` tree). The model is `bifrost_ui::progress`; this module
//! exposes it to apps and adds the English `localizedDescription` text.
//!
//! A [`ProgressTree`] is an arena: ids come from `root`, `child` and `implicit_child`, and parents report the share of
//! each child by its pending units. Cancel, pause and resume reach all descendants.
//!
//! Not matched: publishing progress to other processes (the tree lives in the app that created it), the shared
//! progress manager and value observation.

pub use bifrost_ui::progress::{ProgressId, ProgressTree};

/// `localizedDescription`: "N% completed" with N rounded down, English only. Empty while the work is indeterminate.
pub fn localized_description(tree: &ProgressTree, id: ProgressId) -> String {
    if tree.is_indeterminate(id) {
        return String::new();
    }
    // The epsilon keeps fractions such as 0.29 (stored as 0.28999...) from rounding down a whole percent.
    let percent = (tree.fraction_completed(id) * 100.0 + 1e-9).floor() as u32;
    format!("{percent}% completed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_with_two_children_reports_half_after_one_finishes() {
        let mut t = ProgressTree::new();
        let parent = t.root(2);
        let a = t.child(parent, 1, 1);
        let _b = t.child(parent, 1, 1);
        assert_eq!(t.fraction_completed(parent), 0.0);
        t.set_completed_units(a, 1);
        assert_eq!(t.fraction_completed(parent), 0.5);
        assert!(!t.is_finished(parent));
    }

    #[test]
    fn the_description_rounds_down_and_is_empty_while_indeterminate() {
        let mut t = ProgressTree::new();
        let p = t.root(200);
        assert_eq!(localized_description(&t, p), "0% completed");
        t.set_completed_units(p, 100);
        assert_eq!(localized_description(&t, p), "50% completed");
        t.set_completed_units(p, 199);
        assert_eq!(localized_description(&t, p), "99% completed");
        t.set_completed_units(p, 200);
        assert_eq!(localized_description(&t, p), "100% completed");
        let q = t.root(100);
        t.set_completed_units(q, 29);
        assert_eq!(localized_description(&t, q), "29% completed");
        let unknown = t.root(0);
        assert_eq!(localized_description(&t, unknown), "");
    }

    #[test]
    fn cancelling_a_cancellable_parent_cancels_its_children() {
        let mut t = ProgressTree::new();
        let parent = t.root(2);
        let child = t.child(parent, 1, 1);
        assert!(!t.cancel(parent), "not cancellable by default");
        t.set_cancellable(parent, true);
        assert!(t.cancel(parent));
        assert!(t.is_cancelled(parent));
        assert!(t.is_cancelled(child), "the cancel reaches children even when they are not cancellable");
    }
}
