//! Directory records and queries: a record has a type, a name and multi-valued attributes, and a query selects records
//! by type, by a tree of comparisons (from `recordsearch`), sorts them by one attribute and keeps the first few.
//!
//! The attribute `name` is always present and reads the record's name; it cannot be set as an attribute. Every other
//! attribute holds a list of text values, and a comparison matches when any one value matches. Records without a
//! value for the sort attribute come after the records that have one. Comparisons and sort keys ignore case. The
//! query runs on a slice of records the caller already holds (for example the records of one node), so it never
//! touches a server.

use std::collections::BTreeMap;

use crate::recordsearch::{self, Element};

/// The attribute that reads the record's name.
pub const NAME_ATTRIBUTE: &str = "name";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The name `name` is read from the record and cannot be set.
    ReservedAttribute,
    EmptyAttribute,
}

/// One record: a type, a name and attributes with their values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirRecord {
    record_type: String,
    name: String,
    attributes: BTreeMap<String, Vec<String>>,
}

impl DirRecord {
    pub fn new(record_type: &str, name: &str) -> DirRecord {
        DirRecord { record_type: record_type.to_string(), name: name.to_string(), attributes: BTreeMap::new() }
    }

    pub fn record_type(&self) -> &str {
        &self.record_type
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Replaces the values of an attribute. An empty list removes the attribute.
    pub fn set_values(&mut self, attribute: &str, values: Vec<String>) -> Result<(), Error> {
        if attribute.is_empty() {
            return Err(Error::EmptyAttribute);
        }
        if attribute == NAME_ATTRIBUTE {
            return Err(Error::ReservedAttribute);
        }
        if values.is_empty() {
            self.attributes.remove(attribute);
        } else {
            self.attributes.insert(attribute.to_string(), values);
        }
        Ok(())
    }

    /// The values of an attribute, in the order they were set. An attribute the record lacks has no values.
    pub fn values(&self, attribute: &str) -> Vec<String> {
        if attribute == NAME_ATTRIBUTE {
            return vec![self.name.clone()];
        }
        self.attributes.get(attribute).cloned().unwrap_or_default()
    }

    /// The attribute names that hold values, in name order.
    pub fn attribute_names(&self) -> Vec<&str> {
        self.attributes.keys().map(|k| k.as_str()).collect()
    }
}

/// A search over records. Every field narrows the result; the default query returns every record in input order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Only records of this type, compared exactly.
    pub record_type: Option<String>,
    /// Only records that satisfy this tree. `None` accepts every record.
    pub predicate: Option<Element>,
    /// The attributes an any-attribute comparison looks at. Records are never searched beyond this list.
    pub search_attributes: Vec<String>,
    /// Sorts by the first value of this attribute, ignoring case, then by name and type. `None` keeps input order.
    pub sort_attribute: Option<String>,
    /// Keeps at most this many records after sorting. `None` keeps all of them.
    pub limit: Option<usize>,
}

impl Query {
    pub fn new() -> Query {
        Query::default()
    }

    /// Runs the query over `records`, returning references to the matching ones.
    pub fn run<'a>(&self, records: &[&'a DirRecord]) -> Vec<&'a DirRecord> {
        let props: Vec<&str> = self.search_attributes.iter().map(|s| s.as_str()).collect();
        let mut hits: Vec<&'a DirRecord> = records
            .iter()
            .copied()
            .filter(|r| self.type_matches(r) && self.predicate_matches(r, &props))
            .collect();
        if let Some(attribute) = &self.sort_attribute {
            hits.sort_by(|a, b| {
                sort_key(a, attribute)
                    .cmp(&sort_key(b, attribute))
                    .then_with(|| a.name().cmp(b.name()))
                    .then_with(|| a.record_type().cmp(b.record_type()))
            });
        }
        if let Some(limit) = self.limit {
            hits.truncate(limit);
        }
        hits
    }

    fn type_matches(&self, record: &DirRecord) -> bool {
        match &self.record_type {
            None => true,
            Some(t) => t == record.record_type(),
        }
    }

    fn predicate_matches(&self, record: &DirRecord, props: &[&str]) -> bool {
        match &self.predicate {
            None => true,
            Some(element) => recordsearch::matches(element, props, &|p: &str| record.values(p)),
        }
    }
}

/// A record's sort position: records with no value sort after the others, then by the lowercased first value.
fn sort_key(record: &DirRecord, attribute: &str) -> (bool, String) {
    match record.values(attribute).first() {
        None => (true, String::new()),
        Some(v) => (false, v.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recordsearch::Comparison;

    fn person(name: &str, realname: &str, emails: &[&str]) -> DirRecord {
        let mut r = DirRecord::new("users", name);
        r.set_values("realname", vec![realname.to_string()]).unwrap();
        r.set_values("email", emails.iter().map(|e| e.to_string()).collect()).unwrap();
        r
    }

    fn sample() -> Vec<DirRecord> {
        let mut staff = DirRecord::new("groups", "staff");
        staff.set_values("realname", vec!["Staff".to_string()]).unwrap();
        vec![
            person("anna", "Anna Lind", &["anna@example.is"]),
            person("bjarni", "Bjarni Olafsson", &["bjarni@example.is"]),
            person("carla", "Carla Lind", &["carla@example.com", "carla.lind@example.com"]),
            staff,
        ]
    }

    fn names(hits: &[&DirRecord]) -> Vec<String> {
        hits.iter().map(|r| r.name().to_string()).collect()
    }

    #[test]
    fn the_default_query_returns_every_record_in_input_order() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        assert_eq!(names(&Query::new().run(&refs)), vec!["anna", "bjarni", "carla", "staff"]);
    }

    #[test]
    fn a_type_filter_keeps_only_that_type() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query { record_type: Some("groups".to_string()), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["staff"]);
        let q = Query { record_type: Some("users".to_string()), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["anna", "bjarni", "carla"]);
    }

    #[test]
    fn a_comparison_on_one_attribute_matches_ignoring_case() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query { predicate: Some(Element::on("realname", Comparison::Contains, "lind")), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["anna", "carla"]);
    }

    #[test]
    fn a_multi_valued_attribute_matches_when_any_value_matches() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query { predicate: Some(Element::on("email", Comparison::Contains, "lind")), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["carla"], "only the second address contains lind");
    }

    #[test]
    fn an_any_attribute_match_looks_only_at_the_listed_attributes() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query {
            predicate: Some(Element::any_property(Comparison::Contains, "lind")),
            search_attributes: vec!["name".to_string(), "realname".to_string()],
            ..Query::default()
        };
        assert_eq!(names(&q.run(&refs)), vec!["anna", "carla"]);
        let q = Query {
            predicate: Some(Element::any_property(Comparison::Contains, "lind")),
            search_attributes: vec!["name".to_string()],
            ..Query::default()
        };
        assert_eq!(names(&q.run(&refs)), Vec::<String>::new(), "realname is not in the list");
    }

    #[test]
    fn sorting_puts_records_without_the_attribute_last() {
        let mut db = sample();
        db.push(DirRecord::new("users", "aaron"));
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query { sort_attribute: Some("email".to_string()), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["anna", "bjarni", "carla", "aaron", "staff"], "records with no email come last, by name");
    }

    #[test]
    fn a_limit_keeps_the_first_records_after_sorting() {
        let db = sample();
        let refs: Vec<&DirRecord> = db.iter().collect();
        let q = Query { sort_attribute: Some("name".to_string()), limit: Some(2), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), vec!["anna", "bjarni"]);
        let q = Query { sort_attribute: Some("name".to_string()), limit: Some(0), ..Query::default() };
        assert_eq!(names(&q.run(&refs)), Vec::<String>::new());
    }

    #[test]
    fn the_name_attribute_reads_the_record_name_and_cannot_be_set() {
        let mut r = DirRecord::new("users", "anna");
        assert_eq!(r.values("name"), vec!["anna".to_string()]);
        assert_eq!(r.set_values("name", vec!["x".to_string()]), Err(Error::ReservedAttribute));
        assert_eq!(r.set_values("", vec![]), Err(Error::EmptyAttribute));
        r.set_values("email", vec!["a@b.is".to_string()]).unwrap();
        r.set_values("email", vec![]).unwrap();
        assert!(r.attribute_names().is_empty(), "an empty list removes the attribute");
    }
}
