//! An in-memory record store with a model-context style API: insert, fetch, change, delete, save and roll back.
//! Unique constraints are checked on every write. Relations and their delete rules come from `relations`, and each
//! save appends one transaction to `history`. Nothing here touches disk. The app persists what it chooses to.

use crate::model_history::{Change, History, Token};
use crate::model_schema::{Attribute, Schema, SchemaError};
use crate::relations::{Relation, RelationError, Relations};
use std::collections::BTreeMap;

/// One stored record: its id, its entity name and its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    pub entity: String,
    pub fields: BTreeMap<String, String>,
}

impl Record {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|value| value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// No record has this id, or it was deleted.
    NotFound(u64),
    /// Another record of the same entity already has this value for a unique field.
    Duplicate { entity: String, field: String, value: String },
    /// A relation refused the change.
    Relation(RelationError),
    /// The entity has a schema and the write uses a field the schema does not declare.
    Schema(SchemaError),
}

impl From<RelationError> for StoreError {
    fn from(error: RelationError) -> Self {
        StoreError::Relation(error)
    }
}

impl From<SchemaError> for StoreError {
    fn from(error: SchemaError) -> Self {
        StoreError::Schema(error)
    }
}

/// The working records, the last saved snapshot of them, and the history of saves. Ids start at 1 and are never
/// reused, not even after a rollback.
#[derive(Debug, Default)]
pub struct Store {
    records: BTreeMap<u64, Record>,
    relations: Relations,
    saved: BTreeMap<u64, Record>,
    saved_relations: Relations,
    uniques: Vec<(String, String)>,
    schema: Schema,
    next_id: u64,
    history: History,
}

impl Store {
    pub fn new() -> Self {
        Store { next_id: 1, ..Store::default() }
    }

    /// Requires one value of `field` to be unique among the records of `entity`.
    pub fn declare_unique(&mut self, entity: &str, field: &str) {
        let key = (entity.to_string(), field.to_string());
        if !self.uniques.contains(&key) {
            self.uniques.push(key);
        }
    }

    /// Declares a field of `entity` with its options. A unique field also gets the uniqueness check on every write.
    /// Once an entity has a declared field, a write that uses any other field of it is refused.
    pub fn declare_attribute(&mut self, entity: &str, field: &str, attribute: Attribute) {
        self.schema.declare(entity, field, attribute);
        if attribute.unique {
            self.declare_unique(entity, field);
        }
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The fields of a record that a backend persists: every field except the transient ones.
    pub fn persistent_fields(&self, id: u64) -> Option<BTreeMap<String, String>> {
        let record = self.records.get(&id)?;
        Some(self.schema.persistent_fields(&record.entity, &record.fields))
    }

    pub fn declare_relation(&mut self, relation: Relation) {
        self.relations.declare(relation);
    }

    pub fn relations(&self) -> &Relations {
        &self.relations
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn get(&self, id: u64) -> Option<&Record> {
        self.records.get(&id)
    }

    /// The records of `entity` that `keep` accepts, in id order.
    pub fn fetch<F: Fn(&Record) -> bool>(&self, entity: &str, keep: F) -> Vec<&Record> {
        self.records.values().filter(|record| record.entity == entity && keep(*record)).collect()
    }

    /// Whether the working records or links differ from the last save.
    pub fn has_changes(&self) -> bool {
        self.records != self.saved || self.relations != self.saved_relations
    }

    /// Inserts a record and returns its id.
    pub fn insert(&mut self, entity: &str, fields: &[(&str, &str)]) -> Result<u64, StoreError> {
        let fields: BTreeMap<String, String> =
            fields.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
        self.schema.validate(entity, &fields)?;
        self.check_unique(None, entity, &fields)?;
        let id = self.next_id;
        self.next_id += 1;
        self.records.insert(id, Record { id, entity: entity.to_string(), fields });
        Ok(id)
    }

    /// Sets one field of a record. A value that another record of the entity already holds for a unique field is
    /// refused, and the record is left as it was.
    pub fn set(&mut self, id: u64, field: &str, value: &str) -> Result<(), StoreError> {
        let record = self.records.get(&id).ok_or(StoreError::NotFound(id))?;
        let mut fields = record.fields.clone();
        fields.insert(field.to_string(), value.to_string());
        let entity = record.entity.clone();
        self.schema.validate(&entity, &fields)?;
        self.check_unique(Some(id), &entity, &fields)?;
        if let Some(record) = self.records.get_mut(&id) {
            record.fields = fields;
        }
        Ok(())
    }

    /// Links `target` to `owner` through a declared relation. Both records must exist.
    pub fn link(&mut self, owner: u64, relation: &str, target: u64) -> Result<bool, StoreError> {
        for id in [owner, target] {
            if !self.records.contains_key(&id) {
                return Err(StoreError::NotFound(id));
            }
        }
        Ok(self.relations.link(owner, relation, target)?)
    }

    pub fn unlink(&mut self, owner: u64, relation: &str, target: u64) -> bool {
        self.relations.unlink(owner, relation, target)
    }

    /// Deletes a record and whatever its delete rules take with it. Returns the ids removed, root first.
    /// A refused delete changes nothing.
    pub fn delete(&mut self, id: u64) -> Result<Vec<u64>, StoreError> {
        if !self.records.contains_key(&id) {
            return Err(StoreError::NotFound(id));
        }
        let doomed = self.relations.plan_delete(id)?;
        self.relations.apply_delete(&doomed);
        for gone in &doomed {
            self.records.remove(gone);
        }
        Ok(doomed)
    }

    /// Throws away every change since the last save.
    pub fn rollback(&mut self) {
        self.records = self.saved.clone();
        self.relations = self.saved_relations.clone();
    }

    /// Checks the relation minimums, then saves: the changes become one transaction in the history. Returns its
    /// token, or `None` when nothing had changed.
    pub fn save(&mut self, author: &str, at: u64) -> Result<Option<Token>, StoreError> {
        for (id, record) in &self.records {
            self.relations.check_counts(*id, &record.entity)?;
        }
        let changes = self.pending_changes();
        let token = self.history.commit(author, at, changes);
        self.saved = self.records.clone();
        self.saved_relations = self.relations.clone();
        Ok(token)
    }

    fn pending_changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        for (id, record) in &self.records {
            match self.saved.get(id) {
                None => changes.push(Change::Insert { record: *id, entity: record.entity.clone() }),
                Some(old) if old != record => {
                    changes.push(Change::Update { record: *id, entity: record.entity.clone() })
                }
                Some(_) => {}
            }
        }
        for (id, old) in &self.saved {
            if !self.records.contains_key(id) {
                changes.push(Change::Delete { record: *id, entity: old.entity.clone() });
            }
        }
        for owner in self.relations.changed_owners(&self.saved_relations) {
            if changes.iter().any(|change| change.record() == owner) {
                continue;
            }
            if let Some(record) = self.records.get(&owner) {
                changes.push(Change::Update { record: owner, entity: record.entity.clone() });
            }
        }
        changes
    }

    fn check_unique(&self, id: Option<u64>, entity: &str, fields: &BTreeMap<String, String>) -> Result<(), StoreError> {
        for (constrained, field) in &self.uniques {
            if constrained.as_str() != entity {
                continue;
            }
            let Some(value) = fields.get(field) else { continue };
            let clash = self.records.values().any(|record| {
                record.entity == entity && Some(record.id) != id && record.field(field) == Some(value.as_str())
            });
            if clash {
                return Err(StoreError::Duplicate {
                    entity: entity.to_string(),
                    field: field.clone(),
                    value: value.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_history::Descriptor;
    use crate::relations::DeleteRule;

    #[test]
    fn unique_field_refuses_a_second_record_on_insert_and_on_set() {
        let mut s = Store::new();
        s.declare_unique("Contact", "email");
        let a = s.insert("Contact", &[("email", "a@example.test")]).unwrap();
        assert!(matches!(s.insert("Contact", &[("email", "a@example.test")]), Err(StoreError::Duplicate { .. })));
        let b = s.insert("Contact", &[("email", "b@example.test")]).unwrap();
        assert!(matches!(s.set(b, "email", "a@example.test"), Err(StoreError::Duplicate { .. })));
        assert_eq!(s.get(b).unwrap().field("email"), Some("b@example.test"), "a refused set changes nothing");
        s.set(b, "email", "b@example.test").unwrap();
        assert_eq!(s.get(a).unwrap().field("email"), Some("a@example.test"));
    }

    #[test]
    fn save_writes_one_transaction_of_insert_update_and_delete_changes() {
        let mut s = Store::new();
        let a = s.insert("Note", &[("title", "one")]).unwrap();
        assert_eq!(s.save("ui", 10).unwrap(), Some(Token(1)));
        assert!(!s.has_changes());
        s.set(a, "title", "two").unwrap();
        let b = s.insert("Note", &[("title", "three")]).unwrap();
        assert!(s.has_changes());
        s.save("ui", 20).unwrap();
        s.delete(b).unwrap();
        s.save("ui", 30).unwrap();

        let log = s.history().fetch(&Descriptor::default());
        assert_eq!(log.len(), 3);
        assert_eq!(
            log[1].changes,
            vec![
                Change::Update { record: a, entity: "Note".into() },
                Change::Insert { record: b, entity: "Note".into() },
            ]
        );
        assert_eq!(log[2].changes, vec![Change::Delete { record: b, entity: "Note".into() }]);
        assert_eq!(log[2].author, "ui");
        assert_eq!(log[2].at, 30);
    }

    #[test]
    fn rollback_restores_the_last_saved_records_and_links() {
        let mut s = Store::new();
        s.declare_relation(Relation::new("Folder", "notes", DeleteRule::NoAction));
        let f = s.insert("Folder", &[("name", "work")]).unwrap();
        let n = s.insert("Note", &[("title", "one")]).unwrap();
        s.link(f, "notes", n).unwrap();
        s.save("ui", 1).unwrap();

        s.set(n, "title", "changed").unwrap();
        let extra = s.insert("Note", &[("title", "extra")]).unwrap();
        s.unlink(f, "notes", n);
        s.rollback();

        assert_eq!(s.get(n).unwrap().field("title"), Some("one"));
        assert!(s.get(extra).is_none(), "a record inserted after the save is gone");
        assert_eq!(s.relations().targets(f, "notes"), vec![n]);
        assert!(!s.has_changes());
    }

    #[test]
    fn delete_removes_cascaded_records_and_reports_them_root_first() {
        let mut s = Store::new();
        s.declare_relation(Relation::new("Folder", "notes", DeleteRule::Cascade));
        let f = s.insert("Folder", &[("name", "work")]).unwrap();
        let n1 = s.insert("Note", &[("title", "one")]).unwrap();
        let n2 = s.insert("Note", &[("title", "two")]).unwrap();
        s.link(f, "notes", n1).unwrap();
        s.link(f, "notes", n2).unwrap();
        assert_eq!(s.delete(f), Ok(vec![f, n1, n2]));
        assert!(s.fetch("Note", |_| true).is_empty());
        assert!(s.get(f).is_none());
    }

    #[test]
    fn deny_keeps_every_record_and_link_in_place() {
        let mut s = Store::new();
        s.declare_relation(Relation::new("Owner", "pets", DeleteRule::Deny));
        let owner = s.insert("Owner", &[("name", "ada")]).unwrap();
        let pet = s.insert("Pet", &[("name", "rex")]).unwrap();
        s.link(owner, "pets", pet).unwrap();
        assert_eq!(
            s.delete(owner),
            Err(StoreError::Relation(RelationError::Denied { relation: "pets".into(), owner }))
        );
        assert!(s.get(owner).is_some());
        assert_eq!(s.relations().targets(owner, "pets"), vec![pet]);
    }

    #[test]
    fn save_refuses_a_relation_below_its_minimum_and_accepts_it_once_met() {
        let mut s = Store::new();
        s.declare_relation(Relation::new("Playlist", "tracks", DeleteRule::NoAction).min(1));
        let list = s.insert("Playlist", &[("name", "mix")]).unwrap();
        assert_eq!(
            s.save("ui", 1),
            Err(StoreError::Relation(RelationError::TooFew { relation: "tracks".into(), min: 1 }))
        );
        let track = s.insert("Track", &[("name", "song")]).unwrap();
        s.link(list, "tracks", track).unwrap();
        assert_eq!(s.save("ui", 2), Ok(Some(Token(1))));
    }

    #[test]
    fn a_link_alone_is_saved_as_an_update_of_its_owner() {
        let mut s = Store::new();
        s.declare_relation(Relation::new("Folder", "notes", DeleteRule::NoAction));
        let f = s.insert("Folder", &[("name", "work")]).unwrap();
        let n = s.insert("Note", &[("title", "one")]).unwrap();
        s.save("ui", 1).unwrap();
        s.link(f, "notes", n).unwrap();
        s.save("ui", 2).unwrap();
        let log = s.history().fetch(&Descriptor { after: Token(1), ..Descriptor::default() });
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].changes, vec![Change::Update { record: f, entity: "Folder".into() }]);
    }

    #[test]
    fn a_declared_entity_refuses_a_field_its_schema_does_not_declare() {
        let mut s = Store::new();
        s.declare_attribute("Contact", "email", Attribute { unique: true, ..Attribute::default() });
        let a = s.insert("Contact", &[("email", "a@example.test")]).unwrap();
        assert_eq!(
            s.insert("Contact", &[("email", "b@example.test"), ("nick", "ada")]),
            Err(StoreError::Schema(SchemaError::UnknownField { entity: "Contact".into(), field: "nick".into() }))
        );
        assert_eq!(
            s.set(a, "nick", "ada"),
            Err(StoreError::Schema(SchemaError::UnknownField { entity: "Contact".into(), field: "nick".into() }))
        );
        assert_eq!(s.get(a).unwrap().field("nick"), None, "a refused set changes nothing");
        assert_eq!(s.fetch("Contact", |_| true).len(), 1);
        assert!(s.schema().unique_fields("Contact").contains(&"email"));
    }

    #[test]
    fn transient_fields_stay_out_of_the_persisted_fields() {
        let mut s = Store::new();
        s.declare_attribute("Note", "title", Attribute::default());
        s.declare_attribute("Note", "cursor", Attribute { transient: true, ..Attribute::default() });
        let n = s.insert("Note", &[("title", "one"), ("cursor", "14")]).unwrap();
        assert_eq!(s.get(n).unwrap().field("cursor"), Some("14"), "the record still holds the transient field");
        let persisted = s.persistent_fields(n).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted.get("title").map(String::as_str), Some("one"));
        assert_eq!(s.persistent_fields(99), None);
    }
}
