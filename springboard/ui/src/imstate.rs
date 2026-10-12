//! The state of an input method as the shell keeps it: whether the method is active, the events it recognizes, its named
//! input modes with one current mode, and the values a client stores under an integer tag. A value belongs to one
//! client, so when a client's session closes its values are dropped with it.

use std::collections::HashMap;

/// Key down events (bit mask for `recognizes`).
pub const EVENT_KEY_DOWN: u32 = 1 << 0;
/// Mouse down events.
pub const EVENT_MOUSE_DOWN: u32 = 1 << 1;
/// Mouse up events.
pub const EVENT_MOUSE_UP: u32 = 1 << 2;
/// Mouse moved events.
pub const EVENT_MOUSE_MOVED: u32 = 1 << 3;

/// A value a client stores under a tag.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Text(String),
    Flag(bool),
}

/// The state of one input method.
#[derive(Debug, Default)]
pub struct MethodState {
    active: bool,
    events: u32,
    modes: Vec<String>,
    current: Option<usize>,
    values: HashMap<(u32, u32), Value>,
}

impl MethodState {
    /// A new method, inactive, with no modes. `events` is a union of the `EVENT_*` masks.
    pub fn new(events: u32) -> Self {
        MethodState { events, ..Default::default() }
    }

    /// The union of the recognized event masks.
    pub fn recognized_events(&self) -> u32 {
        self.events
    }

    /// True when every bit of `event` is recognized.
    pub fn recognizes(&self, event: u32) -> bool {
        event != 0 && self.events & event == event
    }

    /// `activateServer`.
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// `deactivateServer`.
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Adds an input mode. A name already present is not added twice. The first mode added becomes current.
    pub fn add_mode(&mut self, name: &str) {
        if !self.modes.iter().any(|m| m == name) {
            self.modes.push(name.to_string());
        }
        if self.current.is_none() && !self.modes.is_empty() {
            self.current = Some(0);
        }
    }

    /// The modes, in the order they were added.
    pub fn modes(&self) -> &[String] {
        &self.modes
    }

    pub fn current_mode(&self) -> Option<&str> {
        self.current.and_then(|i| self.modes.get(i)).map(|s| s.as_str())
    }

    /// Makes `name` the current mode. Returns false, and keeps the current mode, when there is no such mode.
    pub fn select_mode(&mut self, name: &str) -> bool {
        match self.modes.iter().position(|m| m == name) {
            Some(i) => {
                self.current = Some(i);
                true
            }
            None => false,
        }
    }

    /// `setValue:forTag:client:`. A value already stored under the same client and tag is replaced.
    pub fn set_value(&mut self, client: u32, tag: u32, value: Value) {
        self.values.insert((client, tag), value);
    }

    /// `valueForTag:client:`.
    pub fn value(&self, client: u32, tag: u32) -> Option<&Value> {
        self.values.get(&(client, tag))
    }

    /// Drops every value that a client stored. Returns how many were removed.
    pub fn forget_client(&mut self, client: u32) -> usize {
        let before = self.values.len();
        self.values.retain(|(c, _), _| *c != client);
        before - self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognized_events_are_a_union_of_masks() {
        let m = MethodState::new(EVENT_KEY_DOWN | EVENT_MOUSE_DOWN);
        assert_eq!(m.recognized_events(), 3);
        assert!(m.recognizes(EVENT_KEY_DOWN));
        assert!(m.recognizes(EVENT_MOUSE_DOWN));
        assert!(!m.recognizes(EVENT_MOUSE_UP));
        assert!(!m.recognizes(0));
    }

    #[test]
    fn activation_toggles_the_server() {
        let mut m = MethodState::new(EVENT_KEY_DOWN);
        assert!(!m.is_active());
        m.activate();
        assert!(m.is_active());
        m.deactivate();
        assert!(!m.is_active());
    }

    #[test]
    fn modes_are_unique_and_the_first_one_is_current() {
        let mut m = MethodState::new(0);
        assert_eq!(m.current_mode(), None);
        m.add_mode("Hiragana");
        m.add_mode("Latin");
        m.add_mode("Hiragana");
        assert_eq!(m.modes().len(), 2);
        assert_eq!(m.current_mode(), Some("Hiragana"));
    }

    #[test]
    fn select_mode_switches_only_to_a_known_mode() {
        let mut m = MethodState::new(0);
        m.add_mode("Hiragana");
        m.add_mode("Latin");
        assert!(m.select_mode("Latin"));
        assert_eq!(m.current_mode(), Some("Latin"));
        assert!(!m.select_mode("Kana"));
        assert_eq!(m.current_mode(), Some("Latin"));
    }

    #[test]
    fn values_are_scoped_to_their_client() {
        let mut m = MethodState::new(0);
        m.set_value(1, 4, Value::Int(9));
        m.set_value(2, 4, Value::Text("x".to_string()));
        assert_eq!(m.value(1, 4), Some(&Value::Int(9)));
        assert_eq!(m.value(2, 4), Some(&Value::Text("x".to_string())));
        assert_eq!(m.value(3, 4), None);
        m.set_value(1, 4, Value::Flag(true));
        assert_eq!(m.value(1, 4), Some(&Value::Flag(true)));
    }

    #[test]
    fn forgetting_a_client_drops_only_its_values() {
        let mut m = MethodState::new(0);
        m.set_value(1, 4, Value::Int(9));
        m.set_value(1, 5, Value::Flag(false));
        m.set_value(2, 4, Value::Int(3));
        assert_eq!(m.forget_client(1), 2);
        assert_eq!(m.value(1, 4), None);
        assert_eq!(m.value(2, 4), Some(&Value::Int(3)));
        assert_eq!(m.forget_client(1), 0);
    }
}
