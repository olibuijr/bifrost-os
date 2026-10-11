//! Batched saves and change history for a set of records (contacts, or any record type). One batch of adds, updates
//! and deletes applies all or nothing. Each applied change gets the next sequence number, so a client keeps a token
//! and fetches only what came after it.

use std::collections::{BTreeMap, VecDeque};

/// A history bound sized for a contacts set (one event per change). Pass it to `Store::new`.
pub const MAX_HISTORY: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<T> {
    /// Adds a record; the store gives it the next identifier.
    Add(T),
    /// Replaces the record with this identifier.
    Update(u64, T),
    /// Removes the record with this identifier.
    Delete(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<T> {
    Added(T),
    Updated { before: T, after: T },
    Deleted(T),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event<T> {
    /// The position in the history. The token after this event is `seq`.
    pub seq: u64,
    pub id: u64,
    pub change: Change<T>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The record at this index of the batch failed validation.
    Invalid(usize),
    /// An update or a delete names no record.
    UnknownRecord(u64),
    /// The token is older than the retained history, so some changes it needs were dropped.
    Expired,
}

#[derive(Debug)]
pub struct Store<T> {
    records: BTreeMap<u64, T>,
    next_id: u64,
    seq: u64,
    history: VecDeque<Event<T>>,
    limit: usize,
}

impl<T: Clone> Store<T> {
    /// A store that keeps at most `limit` history events (at least one).
    pub fn new(limit: usize) -> Self {
        Store { records: BTreeMap::new(), next_id: 0, seq: 0, history: VecDeque::new(), limit: limit.max(1) }
    }

    pub fn get(&self, id: u64) -> Option<&T> {
        self.records.get(&id)
    }

    /// Every record with its identifier, in identifier order.
    pub fn records(&self) -> impl Iterator<Item = (u64, &T)> {
        self.records.iter().map(|(id, record)| (*id, record))
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The token for the current state. Keep it, then ask `changes_since` for what happened after it.
    pub fn token(&self) -> u64 {
        self.seq
    }

    /// Applies a batch in order. `valid` checks every record that an add or an update brings in.
    /// Returns the identifiers given to the adds, in batch order. When any operation fails, nothing changes,
    /// including the identifiers the store would have handed out. The batch works on a copy of the records,
    /// so a large store pays for a copy per save.
    pub fn save(&mut self, ops: Vec<Op<T>>, valid: impl Fn(&T) -> bool) -> Result<Vec<u64>, Error> {
        let mut records = self.records.clone();
        let mut next_id = self.next_id;
        let mut added = Vec::new();
        let mut events: Vec<(u64, Change<T>)> = Vec::new();
        for (index, op) in ops.into_iter().enumerate() {
            match op {
                Op::Add(record) => {
                    if !valid(&record) {
                        return Err(Error::Invalid(index));
                    }
                    next_id += 1;
                    records.insert(next_id, record.clone());
                    added.push(next_id);
                    events.push((next_id, Change::Added(record)));
                }
                Op::Update(id, record) => {
                    if !valid(&record) {
                        return Err(Error::Invalid(index));
                    }
                    let before = records.get(&id).cloned().ok_or(Error::UnknownRecord(id))?;
                    records.insert(id, record.clone());
                    events.push((id, Change::Updated { before, after: record }));
                }
                Op::Delete(id) => {
                    let before = records.remove(&id).ok_or(Error::UnknownRecord(id))?;
                    events.push((id, Change::Deleted(before)));
                }
            }
        }
        self.records = records;
        self.next_id = next_id;
        for (id, change) in events {
            self.seq += 1;
            self.history.push_back(Event { seq: self.seq, id, change });
            if self.history.len() > self.limit {
                self.history.pop_front();
            }
        }
        Ok(added)
    }

    /// The events after `token`, oldest first. A token from the future has nothing after it.
    /// `Expired` when the history no longer reaches back to the token.
    pub fn changes_since(&self, token: u64) -> Result<Vec<&Event<T>>, Error> {
        if token >= self.seq {
            return Ok(Vec::new());
        }
        match self.history.front() {
            Some(first) if first.seq <= token + 1 => Ok(self.history.iter().filter(|e| e.seq > token).collect()),
            _ => Err(Error::Expired),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> String {
        s.to_string()
    }

    fn any(_: &String) -> bool {
        true
    }

    #[test]
    fn adds_get_identifiers_in_batch_order_and_each_change_is_an_event() {
        let mut store: Store<String> = Store::new(100);
        let ids = store.save(vec![Op::Add(text("a")), Op::Add(text("b"))], any).unwrap();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(store.get(2).map(String::as_str), Some("b"));
        assert_eq!(store.token(), 2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn a_failing_batch_changes_nothing_and_the_next_identifier_is_not_spent() {
        let mut store: Store<String> = Store::new(100);
        store.save(vec![Op::Add(text("a"))], any).unwrap();
        let failed = store.save(vec![Op::Add(text("x")), Op::Update(1, text("changed")), Op::Delete(99)], any);
        assert_eq!(failed, Err(Error::UnknownRecord(99)));
        assert_eq!(store.get(1).map(String::as_str), Some("a"));
        assert_eq!(store.len(), 1);
        assert_eq!(store.token(), 1, "no event was recorded for the failed batch");
        assert_eq!(store.save(vec![Op::Add(text("y"))], any).unwrap(), vec![2], "the failed add did not use id 2");
    }

    #[test]
    fn validation_refuses_the_batch_at_the_offending_operation() {
        let mut store: Store<String> = Store::new(100);
        let not_empty = |s: &String| !s.is_empty();
        let result = store.save(vec![Op::Add(text("ok")), Op::Add(String::new())], not_empty);
        assert_eq!(result, Err(Error::Invalid(1)));
        assert!(store.is_empty());
    }

    #[test]
    fn add_update_and_delete_record_the_before_and_after_states_in_order() {
        let mut store: Store<String> = Store::new(100);
        store.save(vec![Op::Add(text("a"))], any).unwrap();
        store.save(vec![Op::Update(1, text("b"))], any).unwrap();
        store.save(vec![Op::Delete(1)], any).unwrap();
        let events = store.changes_since(0).unwrap();
        let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!(events[0].change, Change::Added(text("a")));
        assert_eq!(events[1].change, Change::Updated { before: text("a"), after: text("b") });
        assert_eq!(events[2].change, Change::Deleted(text("b")));
        assert!(store.is_empty());
    }

    #[test]
    fn a_token_fetches_only_the_changes_after_it() {
        let mut store: Store<String> = Store::new(100);
        store.save(vec![Op::Add(text("a"))], any).unwrap();
        let seen = store.token();
        store.save(vec![Op::Add(text("b"))], any).unwrap();
        let newer = store.changes_since(seen).unwrap();
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].id, 2);
        assert!(store.changes_since(store.token()).unwrap().is_empty());
        assert!(store.changes_since(99).unwrap().is_empty(), "a token from the future has nothing after it");
    }

    #[test]
    fn a_batch_with_repeated_updates_records_each_step() {
        let mut store: Store<String> = Store::new(100);
        store.save(vec![Op::Add(text("a"))], any).unwrap();
        store.save(vec![Op::Update(1, text("b")), Op::Update(1, text("c"))], any).unwrap();
        let events = store.changes_since(1).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].change, Change::Updated { before: text("a"), after: text("b") });
        assert_eq!(events[1].change, Change::Updated { before: text("b"), after: text("c") });
        assert_eq!(store.get(1).map(String::as_str), Some("c"));
    }

    #[test]
    fn a_token_older_than_the_retained_history_is_expired_not_silently_short() {
        let mut store: Store<String> = Store::new(2);
        for word in ["a", "b", "c"] {
            store.save(vec![Op::Add(text(word))], any).unwrap();
        }
        assert_eq!(store.changes_since(0), Err(Error::Expired), "event 1 was dropped");
        let tail = store.changes_since(1).unwrap();
        assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
    }
}
