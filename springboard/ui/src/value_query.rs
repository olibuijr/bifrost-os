//! The result of an intent value query: the items a query returned, each with an identifier, a title and named
//! properties (the reference SDK's resolved value query result). Pure model: an intent fills it in, and a test reads
//! it back by position, by identifier or by a property value. Nothing here runs a query.

use std::collections::{BTreeMap, BTreeSet};

/// One item a query returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueItem {
    id: String,
    title: String,
    properties: BTreeMap<String, String>,
}

/// Why a result is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// An item has an empty identifier.
    EmptyId,
    /// Two items share an identifier. The field is the identifier.
    DuplicateId(String),
}

impl ValueItem {
    /// An item with no properties. The identifier must not be empty.
    pub fn new(id: &str, title: &str) -> Result<ValueItem, QueryError> {
        if id.trim().is_empty() {
            return Err(QueryError::EmptyId);
        }
        Ok(ValueItem { id: id.to_string(), title: title.to_string(), properties: BTreeMap::new() })
    }

    /// Adds or replaces a named property, and returns the item so calls can be chained.
    pub fn with_property(mut self, key: &str, value: &str) -> ValueItem {
        self.properties.insert(key.to_string(), value.to_string());
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The value of a property, if the item has it.
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }
}

/// The items a query returned, in the order the query gave them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueQueryResult {
    items: Vec<ValueItem>,
}

impl ValueQueryResult {
    /// Wraps the items. Identifiers must be unique, so that a lookup by identifier has one answer.
    pub fn new(items: Vec<ValueItem>) -> Result<ValueQueryResult, QueryError> {
        let mut seen = BTreeSet::new();
        for item in &items {
            if !seen.insert(item.id.clone()) {
                return Err(QueryError::DuplicateId(item.id.clone()));
            }
        }
        Ok(ValueQueryResult { items })
    }

    /// The items in query order.
    pub fn items(&self) -> &[ValueItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item with identifier `id`, if the result has it.
    pub fn get(&self, id: &str) -> Option<&ValueItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The items whose property `key` equals `value`, in query order.
    pub fn with_property_value(&self, key: &str, value: &str) -> Vec<&ValueItem> {
        self.items.iter().filter(|i| i.property(key) == Some(value)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: &str, title: &str, city: &str) -> ValueItem {
        ValueItem::new(id, title).unwrap().with_property("city", city)
    }

    #[test]
    fn items_keep_query_order_and_look_up_by_id() {
        let r = ValueQueryResult::new(vec![place("p1", "Botanical Garden", "Phoenix"), place("p2", "Museum", "Tucson")]).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.items()[0].title(), "Botanical Garden");
        assert_eq!(r.get("p2").map(|i| i.title()), Some("Museum"));
        assert_eq!(r.get("p9"), None);
    }

    #[test]
    fn duplicate_identifiers_are_refused() {
        let err = ValueQueryResult::new(vec![place("a", "One", "X"), place("a", "Two", "Y")]);
        assert_eq!(err, Err(QueryError::DuplicateId("a".to_string())));
    }

    #[test]
    fn an_empty_identifier_is_refused() {
        assert_eq!(ValueItem::new("  ", "x"), Err(QueryError::EmptyId));
    }

    #[test]
    fn properties_are_read_and_replaced() {
        let item = ValueItem::new("n", "Note").unwrap().with_property("city", "Phoenix").with_property("city", "Mesa");
        assert_eq!(item.property("city"), Some("Mesa"));
        assert_eq!(item.property("zip"), None);
    }

    #[test]
    fn filter_by_property_keeps_matching_items_in_order() {
        let r = ValueQueryResult::new(vec![
            place("a", "Garden", "Phoenix"),
            place("b", "Museum", "Tucson"),
            place("c", "Zoo", "Phoenix"),
        ])
        .unwrap();
        let hits: Vec<&str> = r.with_property_value("city", "Phoenix").iter().map(|i| i.id()).collect();
        assert_eq!(hits, vec!["a", "c"]);
        assert!(r.with_property_value("city", "Flagstaff").is_empty());
    }

    #[test]
    fn an_empty_result_is_valid() {
        let r = ValueQueryResult::new(Vec::new()).unwrap();
        assert!(r.is_empty());
        assert_eq!(r.get("x"), None);
    }
}
