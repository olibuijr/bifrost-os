//! Attachments: a string-keyed dictionary of values carried with a shared buffer (size hints, labels, flags).
//! Pure model: keys are kept in sorted order, so listing them is stable.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Attachment {
    Int(i64),
    Text(String),
    Flag(bool),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Attachments {
    map: BTreeMap<String, Attachment>,
}

impl Attachments {
    pub fn new() -> Self {
        Attachments { map: BTreeMap::new() }
    }

    /// Stores `value` under `key` and returns the value it replaced, if any.
    pub fn set(&mut self, key: &str, value: Attachment) -> Option<Attachment> {
        self.map.insert(key.to_string(), value)
    }

    pub fn get(&self, key: &str) -> Option<&Attachment> {
        self.map.get(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Attachment> {
        self.map.remove(key)
    }

    /// All keys in sorted order.
    pub fn keys(&self) -> Vec<&str> {
        self.map.keys().map(|k| k.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_returns_the_value() {
        let mut a = Attachments::new();
        assert_eq!(a.set("width", Attachment::Int(640)), None);
        assert_eq!(a.get("width"), Some(&Attachment::Int(640)));
        assert_eq!(a.get("height"), None);
    }

    #[test]
    fn setting_an_existing_key_returns_the_replaced_value() {
        let mut a = Attachments::new();
        a.set("label", Attachment::Text("a".to_string()));
        let old = a.set("label", Attachment::Text("b".to_string()));
        assert_eq!(old, Some(Attachment::Text("a".to_string())));
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn keys_are_listed_in_sorted_order() {
        let mut a = Attachments::new();
        a.set("c", Attachment::Flag(true));
        a.set("a", Attachment::Flag(false));
        a.set("b", Attachment::Int(1));
        assert_eq!(a.keys(), vec!["a", "b", "c"]);
    }

    #[test]
    fn remove_returns_the_value_and_drops_the_key() {
        let mut a = Attachments::new();
        a.set("dirty", Attachment::Flag(true));
        assert_eq!(a.remove("dirty"), Some(Attachment::Flag(true)));
        assert_eq!(a.remove("dirty"), None);
        assert!(a.is_empty());
    }

    #[test]
    fn clear_removes_every_key() {
        let mut a = Attachments::new();
        a.set("x", Attachment::Int(1));
        a.set("y", Attachment::Int(2));
        a.clear();
        assert_eq!(a.len(), 0);
        assert!(a.keys().is_empty());
    }
}
