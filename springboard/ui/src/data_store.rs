//! A pluggable record store: the interface a storage backend implements, a batch of writes that applies in full or
//! not at all, and an in-memory backend. Fetches return records in id order. A backend that writes files belongs
//! outside this crate, so the UI models stay free of disk access. `model_store` decides what the records hold;
//! this module only moves them in and out.

use crate::model_store::Record;
use std::collections::BTreeMap;

/// One write of a batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    /// Inserts the record, or replaces the stored record with the same id.
    Put(Record),
    /// Removes the record with this id.
    Remove(u64),
}

/// Writes applied in order. A batch applies in full, or changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    writes: Vec<Write>,
}

impl Batch {
    pub fn new() -> Self {
        Batch::default()
    }

    pub fn put(&mut self, record: Record) -> &mut Self {
        self.writes.push(Write::Put(record));
        self
    }

    pub fn remove(&mut self, id: u64) -> &mut Self {
        self.writes.push(Write::Remove(id));
        self
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataStoreError {
    /// A remove names an id the store does not hold. The whole batch is refused.
    NotFound(u64),
    /// Ids start at 1, so a record with id 0 is refused. The whole batch is refused.
    InvalidId(u64),
}

/// What a storage backend must do: fetch the records of an entity, apply a batch, and erase everything.
pub trait DataStore {
    /// The records of an entity, in id order.
    fn fetch(&self, entity: &str) -> Vec<Record>;

    fn fetch_count(&self, entity: &str) -> usize {
        self.fetch(entity).len()
    }

    /// The ids of the records of an entity, in id order.
    fn fetch_identifiers(&self, entity: &str) -> Vec<u64> {
        self.fetch(entity).iter().map(|record| record.id).collect()
    }

    /// Applies a batch in order. A refused batch changes nothing.
    fn save(&mut self, batch: &Batch) -> Result<(), DataStoreError>;

    /// Removes every record of every entity.
    fn erase(&mut self);
}

/// A backend that keeps its records in memory.
#[derive(Debug, Default)]
pub struct MemoryDataStore {
    records: BTreeMap<u64, Record>,
}

impl MemoryDataStore {
    pub fn new() -> Self {
        MemoryDataStore::default()
    }
}

impl DataStore for MemoryDataStore {
    fn fetch(&self, entity: &str) -> Vec<Record> {
        self.records.values().filter(|record| record.entity == entity).cloned().collect()
    }

    fn save(&mut self, batch: &Batch) -> Result<(), DataStoreError> {
        // Apply to a copy, so a failing write leaves the stored records as they were.
        let mut next = self.records.clone();
        for write in batch.writes() {
            match write {
                Write::Put(record) => {
                    if record.id == 0 {
                        return Err(DataStoreError::InvalidId(record.id));
                    }
                    next.insert(record.id, record.clone());
                }
                Write::Remove(id) => {
                    if next.remove(id).is_none() {
                        return Err(DataStoreError::NotFound(*id));
                    }
                }
            }
        }
        self.records = next;
        Ok(())
    }

    fn erase(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u64, entity: &str, title: &str) -> Record {
        let fields: BTreeMap<String, String> = [("title".to_string(), title.to_string())].into_iter().collect();
        Record { id, entity: entity.to_string(), fields }
    }

    #[test]
    fn fetch_returns_one_entity_in_id_order() {
        let mut store = MemoryDataStore::new();
        let mut batch = Batch::new();
        batch.put(record(2, "Note", "two")).put(record(3, "Folder", "work")).put(record(1, "Note", "one"));
        store.save(&batch).unwrap();
        assert_eq!(store.fetch_identifiers("Note"), vec![1, 2]);
        assert_eq!(store.fetch_count("Note"), 2);
        assert_eq!(store.fetch_count("Folder"), 1);
        assert_eq!(store.fetch("Note")[0].field("title"), Some("one"));
    }

    #[test]
    fn a_refused_batch_changes_nothing() {
        let mut store = MemoryDataStore::new();
        let mut first = Batch::new();
        first.put(record(1, "Note", "one"));
        store.save(&first).unwrap();

        let mut bad = Batch::new();
        bad.put(record(2, "Note", "two")).remove(99);
        assert_eq!(store.save(&bad), Err(DataStoreError::NotFound(99)));
        assert_eq!(store.fetch_identifiers("Note"), vec![1], "the put before the refused remove is not kept");
    }

    #[test]
    fn a_record_with_id_zero_is_refused() {
        let mut store = MemoryDataStore::new();
        let mut batch = Batch::new();
        batch.put(record(0, "Note", "none"));
        assert_eq!(store.save(&batch), Err(DataStoreError::InvalidId(0)));
        assert_eq!(store.fetch_count("Note"), 0);
    }

    #[test]
    fn writes_apply_in_order_so_a_remove_can_follow_a_put() {
        let mut store = MemoryDataStore::new();
        let mut batch = Batch::new();
        batch.put(record(5, "Note", "five")).remove(5);
        assert_eq!(store.save(&batch), Ok(()));
        assert_eq!(store.fetch_count("Note"), 0);

        let mut again = Batch::new();
        again.remove(5);
        assert_eq!(store.save(&again), Err(DataStoreError::NotFound(5)));
    }

    #[test]
    fn a_put_with_an_existing_id_replaces_the_record() {
        let mut store = MemoryDataStore::new();
        let mut first = Batch::new();
        first.put(record(1, "Note", "old"));
        store.save(&first).unwrap();
        let mut second = Batch::new();
        second.put(record(1, "Note", "new"));
        store.save(&second).unwrap();
        assert_eq!(store.fetch_count("Note"), 1);
        assert_eq!(store.fetch("Note")[0].field("title"), Some("new"));
    }

    #[test]
    fn erase_removes_every_record_and_a_batch_reports_its_length() {
        let mut store = MemoryDataStore::new();
        let mut batch = Batch::new();
        batch.put(record(1, "Note", "one")).put(record(2, "Folder", "work")).remove(7);
        assert_eq!(batch.len(), 3);
        assert!(!batch.is_empty());
        assert_eq!(batch.writes()[2], Write::Remove(7));
        assert_eq!(store.save(&batch), Err(DataStoreError::NotFound(7)));

        let mut ok = Batch::new();
        ok.put(record(1, "Note", "one")).put(record(2, "Folder", "work"));
        store.save(&ok).unwrap();
        store.erase();
        assert_eq!(store.fetch_count("Note"), 0);
        assert_eq!(store.fetch_count("Folder"), 0);
        assert!(Batch::new().is_empty());
    }
}
