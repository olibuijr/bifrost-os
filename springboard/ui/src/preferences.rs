//! Preferences: values that child nodes produce and an ancestor reads (`PreferenceKey`, `onPreferenceChange`). Each
//! child sets its contribution under its node id; the parent folds all contributions, in the order the children were
//! first set, into one value. The app then reacts to the combined value only when it changes.

/// A named preference: the value a child produces, how it starts, and how contributions combine.
pub trait PreferenceKey {
    type Value: Clone + PartialEq;
    /// The value when no child contributes one.
    fn default_value() -> Self::Value;
    /// Folds `next`, a later child's contribution, into the value accumulated so far.
    fn reduce(acc: &mut Self::Value, next: Self::Value);
}

/// The contributions of the children of one node, for one preference key.
pub struct Preferences<K: PreferenceKey> {
    contributions: Vec<(u64, K::Value)>,
    reported: K::Value,
}

impl<K: PreferenceKey> Default for Preferences<K> {
    fn default() -> Self {
        Preferences { contributions: Vec::new(), reported: K::default_value() }
    }
}

impl<K: PreferenceKey> Preferences<K> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets the contribution of `node`. A node that sets again keeps its place in the order.
    pub fn set(&mut self, node: u64, value: K::Value) {
        match self.contributions.iter_mut().find(|(n, _)| *n == node) {
            Some(slot) => slot.1 = value,
            None => self.contributions.push((node, value)),
        }
    }
    /// Withdraws the contribution of `node` (the child left the tree); returns whether it had one.
    pub fn remove(&mut self, node: u64) -> bool {
        let before = self.contributions.len();
        self.contributions.retain(|(n, _)| *n != node);
        self.contributions.len() != before
    }
    /// The combined value of every contribution, starting from the default.
    pub fn value(&self) -> K::Value {
        let mut acc = K::default_value();
        for (_, next) in &self.contributions {
            K::reduce(&mut acc, next.clone());
        }
        acc
    }
    /// The change handler: returns the new combined value if it differs from the last one reported, starting from the
    /// default value, so an unchanged tree reports nothing.
    pub fn changed(&mut self) -> Option<K::Value> {
        let now = self.value();
        if now == self.reported {
            return None;
        }
        self.reported = now.clone();
        Some(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widest child wins (a column width shared by rows).
    struct Widest;
    impl PreferenceKey for Widest {
        type Value = f32;
        fn default_value() -> f32 {
            0.0
        }
        fn reduce(acc: &mut f32, next: f32) {
            if next > *acc {
                *acc = next;
            }
        }
    }

    /// Titles gathered in child order (a breadcrumb).
    struct Titles;
    impl PreferenceKey for Titles {
        type Value = Vec<String>;
        fn default_value() -> Vec<String> {
            Vec::new()
        }
        fn reduce(acc: &mut Vec<String>, next: Vec<String>) {
            acc.extend(next);
        }
    }

    #[test]
    fn an_empty_node_has_the_default_value() {
        let p: Preferences<Widest> = Preferences::new();
        assert_eq!(p.value(), 0.0);
    }

    #[test]
    fn the_children_combine_in_the_order_they_were_first_set() {
        let mut p: Preferences<Titles> = Preferences::new();
        p.set(10, vec!["Lists".into()]);
        p.set(20, vec!["Groceries".into()]);
        p.set(30, vec!["Milk".into()]);
        assert_eq!(p.value(), vec!["Lists", "Groceries", "Milk"]);
    }

    #[test]
    fn a_child_that_sets_again_keeps_its_place_and_replaces_its_value() {
        let mut p: Preferences<Titles> = Preferences::new();
        p.set(1, vec!["A".into()]);
        p.set(2, vec!["B".into()]);
        p.set(1, vec!["A2".into()]);
        assert_eq!(p.value(), vec!["A2", "B"]);
    }

    #[test]
    fn removing_a_child_withdraws_its_contribution() {
        let mut p: Preferences<Widest> = Preferences::new();
        p.set(1, 120.0);
        p.set(2, 300.0);
        assert_eq!(p.value(), 300.0);
        assert!(p.remove(2));
        assert_eq!(p.value(), 120.0);
        assert!(!p.remove(2), "a removed child is not removed twice");
    }

    #[test]
    fn the_change_handler_reports_only_a_different_combined_value() {
        let mut p: Preferences<Widest> = Preferences::new();
        assert_eq!(p.changed(), None, "the default is what the handler starts from");
        p.set(1, 80.0);
        assert_eq!(p.changed(), Some(80.0));
        p.set(2, 40.0);
        assert_eq!(p.changed(), None, "a narrower child does not change the widest value");
        p.set(3, 200.0);
        assert_eq!(p.changed(), Some(200.0));
        assert_eq!(p.changed(), None, "an unchanged tree reports nothing");
    }

    #[test]
    fn a_child_left_with_the_widest_value_reports_the_next_widest() {
        let mut p: Preferences<Widest> = Preferences::new();
        p.set(1, 50.0);
        p.set(2, 90.0);
        assert_eq!(p.changed(), Some(90.0));
        p.remove(2);
        assert_eq!(p.changed(), Some(50.0));
    }
}
