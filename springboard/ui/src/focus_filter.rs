//! Focus filters: an app declares a filter intent, the user sets its values for each Focus, and the app reads the
//! values that apply while that Focus is on. The system keeps one app context per Focus and reports the context
//! of the active Focus through `current`. An app whose context went stale calls `invalidate`, so the system asks
//! for `current` again.
//!
//! The shell's Focus is Do Not Disturb (`shell::focus`). The `focus_filter` shell service keeps one `FocusFilters` per
//! app and activates Do Not Disturb as the Focus that is on; the SDK client is `bifrost_app::focus_filter`.

/// The values an app stores for one Focus: key and value text, as the filter intent's parameters carry them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppContext {
    pairs: Vec<(String, String)>,
}

impl AppContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key`, replacing the value it had before (one value per key).
    pub fn set(&mut self, key: &str, value: &str) {
        match self.pairs.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value.into(),
            None => self.pairs.push((key.into(), value.into())),
        }
    }

    /// The value stored under `key`, if any.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Every key and its value, in the order the keys were first set.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// The number of keys stored.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Per-Focus app contexts, the Focus that is on now, and a generation that counts changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusFilters {
    /// (Focus name, the context the user set for it): one entry per Focus
    contexts: Vec<(String, AppContext)>,
    /// the Focus that is on now, if any
    active: Option<String>,
    /// counts changes to the stored contexts, so a reader can tell that its copy is stale
    generation: u64,
}

impl FocusFilters {
    pub fn new() -> Self {
        Self::default()
    }

    /// The context stored for `focus`.
    pub fn context(&self, focus: &str) -> Option<&AppContext> {
        self.contexts.iter().find(|(f, _)| f == focus).map(|(_, c)| c)
    }

    /// Stores `ctx` for `focus`, replacing the context it had. Storing the same context again changes nothing.
    pub fn set_context(&mut self, focus: &str, ctx: AppContext) {
        if self.context(focus) == Some(&ctx) {
            return;
        }
        self.contexts.retain(|(f, _)| f != focus);
        self.contexts.push((focus.into(), ctx));
        self.generation += 1;
    }

    /// Removes the context stored for `focus`. Returns whether there was one.
    pub fn remove(&mut self, focus: &str) -> bool {
        let before = self.contexts.len();
        self.contexts.retain(|(f, _)| f != focus);
        let removed = self.contexts.len() != before;
        if removed {
            self.generation += 1;
        }
        removed
    }

    /// Makes `focus` the Focus that is on, or none when `None` is given.
    pub fn activate(&mut self, focus: Option<&str>) {
        self.active = focus.map(String::from);
    }

    /// The Focus that is on now.
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// The context that applies now: the active Focus's context. `None` when no Focus is on or it has no context.
    pub fn current(&self) -> Option<&AppContext> {
        self.context(self.active.as_deref()?)
    }

    /// Tells the system that the app's context changed, so it asks for `current` again. Returns the new generation.
    pub fn invalidate(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// The number of changes made so far, counting stored contexts and invalidations.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> AppContext {
        let mut c = AppContext::new();
        for (k, v) in pairs {
            c.set(k, v);
        }
        c
    }

    #[test]
    fn no_context_applies_until_one_is_set_for_the_active_focus() {
        let mut f = FocusFilters::new();
        assert!(f.current().is_none());
        f.activate(Some("Work"));
        assert!(f.current().is_none(), "the active Focus has no context yet");
        f.set_context("Work", ctx(&[("mode", "Quiet")]));
        assert_eq!(f.current().unwrap().get("mode"), Some("Quiet"));
    }

    #[test]
    fn switching_focus_switches_the_context_and_no_focus_gives_none() {
        let mut f = FocusFilters::new();
        f.set_context("Work", ctx(&[("mode", "Quiet")]));
        f.set_context("Personal", ctx(&[("mode", "Open")]));
        f.activate(Some("Work"));
        assert_eq!(f.current().unwrap().get("mode"), Some("Quiet"));
        f.activate(Some("Personal"));
        assert_eq!(f.current().unwrap().get("mode"), Some("Open"));
        f.activate(None);
        assert!(f.current().is_none());
        assert_eq!(f.active(), None);
    }

    #[test]
    fn setting_a_key_twice_replaces_the_value() {
        let mut c = AppContext::new();
        c.set("a", "1");
        c.set("a", "2");
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("a"), Some("2"));
        assert_eq!(c.get("b"), None);
        c.set("z", "9");
        assert_eq!(c.pairs(), &[("a".to_string(), "2".to_string()), ("z".to_string(), "9".to_string())]);
        assert!(!c.is_empty());
        assert!(AppContext::new().is_empty());
    }

    #[test]
    fn generation_counts_real_changes_only() {
        let mut f = FocusFilters::new();
        assert_eq!(f.generation(), 0);
        f.set_context("Work", ctx(&[("mode", "Quiet")]));
        assert_eq!(f.generation(), 1);
        f.set_context("Work", ctx(&[("mode", "Quiet")]));
        assert_eq!(f.generation(), 1, "the same context again changes nothing");
        f.set_context("Work", ctx(&[("mode", "Busy")]));
        assert_eq!(f.generation(), 2);
        assert!(!f.remove("Nope"));
        assert_eq!(f.generation(), 2, "removing a Focus with no context changes nothing");
        assert!(f.remove("Work"));
        assert_eq!(f.generation(), 3);
        assert_eq!(f.invalidate(), 4);
        assert_eq!(f.generation(), 4);
    }

    #[test]
    fn removing_the_active_focus_context_leaves_nothing_current() {
        let mut f = FocusFilters::new();
        f.set_context("Work", ctx(&[("mode", "Quiet")]));
        f.activate(Some("Work"));
        assert!(f.current().is_some());
        assert!(f.remove("Work"));
        assert!(f.current().is_none());
        assert_eq!(f.active(), Some("Work"), "the Focus is still on, it just has no context");
    }

    #[test]
    fn a_new_context_for_a_focus_replaces_the_whole_old_one() {
        let mut f = FocusFilters::new();
        f.set_context("Work", ctx(&[("room", "3")]));
        f.set_context("Work", ctx(&[("mode", "Busy")]));
        let c = f.context("Work").unwrap();
        assert_eq!(c.get("room"), None, "keys from the old context are gone");
        assert_eq!(c.get("mode"), Some("Busy"));
        assert_eq!(c.len(), 1);
    }
}
