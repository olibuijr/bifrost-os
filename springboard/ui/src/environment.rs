//! Environment values propagated through a view hierarchy (`EnvironmentValues`): a value set by a node is seen by its
//! descendants, and a nested scope overrides it until the scope is left. A node enters a scope before its children
//! and leaves it after them, so a setting never leaks into a sibling.

/// One environment value. The variants cover what views read: switches, counts, scales and names.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    Text(String),
}

/// The environment as seen from the node being laid out. The root scope always exists.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    scopes: Vec<Vec<(String, EnvValue)>>,
}

impl Default for Environment {
    fn default() -> Self {
        Environment { scopes: vec![Vec::new()] }
    }
}

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }
    /// Opens a scope for a child: values set in it override the ones above until `leave`.
    pub fn enter(&mut self) {
        self.scopes.push(Vec::new());
    }
    /// Closes the innermost scope and drops its settings; returns false at the root, which stays.
    pub fn leave(&mut self) -> bool {
        if self.scopes.len() > 1 {
            self.scopes.pop();
            true
        } else {
            false
        }
    }
    /// How many scopes are open, the root included.
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }
    /// Sets `key` in the current scope. A second set in the same scope replaces the first.
    pub fn set(&mut self, key: &str, value: EnvValue) {
        let Some(scope) = self.scopes.last_mut() else { return };
        match scope.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => scope.push((key.to_string(), value)),
        }
    }
    /// Removes the setting of `key` in the current scope only; a value from an outer scope shows again. Returns
    /// whether the current scope had one.
    pub fn unset(&mut self, key: &str) -> bool {
        let Some(scope) = self.scopes.last_mut() else { return false };
        let before = scope.len();
        scope.retain(|(k, _)| k != key);
        scope.len() != before
    }
    /// The value of `key` as the current node sees it: the innermost setting wins.
    pub fn get(&self, key: &str) -> Option<&EnvValue> {
        self.scopes.iter().rev().find_map(|scope| scope.iter().find(|(k, _)| k == key).map(|(_, v)| v))
    }
    /// A switch read: `false` when the key is unset or holds something other than a bool.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(EnvValue::Bool(true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_set_at_the_root_is_seen_by_every_scope() {
        let mut env = Environment::new();
        env.set("line-limit", EnvValue::Int(2));
        env.enter();
        env.enter();
        assert_eq!(env.get("line-limit"), Some(&EnvValue::Int(2)));
        assert_eq!(env.depth(), 3);
    }

    #[test]
    fn a_nested_scope_overrides_until_it_is_left() {
        let mut env = Environment::new();
        env.set("tint", EnvValue::Text("blue".into()));
        env.enter();
        env.set("tint", EnvValue::Text("red".into()));
        assert_eq!(env.get("tint"), Some(&EnvValue::Text("red".into())));
        assert!(env.leave());
        assert_eq!(env.get("tint"), Some(&EnvValue::Text("blue".into())), "the sibling sees the outer value again");
    }

    #[test]
    fn leaving_the_root_is_refused_and_the_root_keeps_its_values() {
        let mut env = Environment::new();
        env.set("scale", EnvValue::Float(1.5));
        assert!(!env.leave());
        assert_eq!(env.depth(), 1);
        assert_eq!(env.get("scale"), Some(&EnvValue::Float(1.5)));
    }

    #[test]
    fn unset_reveals_the_outer_value_and_a_second_set_replaces_the_first() {
        let mut env = Environment::new();
        env.set("tint", EnvValue::Text("blue".into()));
        env.enter();
        env.set("tint", EnvValue::Text("red".into()));
        env.set("tint", EnvValue::Text("green".into()));
        assert_eq!(env.scopes.last().map(|s| s.len()), Some(1), "one slot per key in a scope");
        assert!(env.unset("tint"));
        assert_eq!(env.get("tint"), Some(&EnvValue::Text("blue".into())));
        assert!(!env.unset("tint"), "nothing left to remove in this scope");
    }

    #[test]
    fn flags_read_only_true_bools_and_missing_keys_are_false() {
        let mut env = Environment::new();
        assert!(!env.flag("reduce-motion"));
        env.set("reduce-motion", EnvValue::Bool(true));
        assert!(env.flag("reduce-motion"));
        env.set("reduce-motion", EnvValue::Text("yes".into()));
        assert!(!env.flag("reduce-motion"), "a text value is not a switch");
    }

    #[test]
    fn leaving_a_scope_drops_the_settings_made_in_it() {
        let mut env = Environment::new();
        env.enter();
        env.set("count", EnvValue::Int(3));
        assert_eq!(env.get("count"), Some(&EnvValue::Int(3)));
        env.leave();
        assert_eq!(env.get("count"), None);
    }
}
