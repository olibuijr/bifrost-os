//! Partial records: a fetch names the fields it needs (its keys) and gets back a record that holds only those fields.
//! Asking for a field the fetch did not name is an error, so a caller that left out a key fails loudly instead of
//! reading an empty value. Pure: the store keeps full records and calls `project` for each fetch.

use std::collections::{BTreeMap, BTreeSet};

/// One field of a contact that a fetch can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    GivenName,
    FamilyName,
    Organization,
    Phones,
    Emails,
    Address,
    Note,
}

impl Key {
    /// Every key, in declaration order.
    pub const ALL: [Key; 7] = [
        Key::GivenName,
        Key::FamilyName,
        Key::Organization,
        Key::Phones,
        Key::Emails,
        Key::Address,
        Key::Note,
    ];
}

impl Key {
    /// The wire name of the key, as the `contacts` service and the SDK spell it.
    pub fn name(self) -> &'static str {
        match self {
            Key::GivenName => "given_name",
            Key::FamilyName => "family_name",
            Key::Organization => "organization",
            Key::Phones => "phones",
            Key::Emails => "emails",
            Key::Address => "address",
            Key::Note => "note",
        }
    }

    /// The key a wire name stands for, or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.iter().copied().find(|key| key.name() == name)
    }
}

/// A field value: text, or a list (phone numbers, email addresses, postal addresses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A fetch with no keys would return nothing.
    NoKeys,
    /// The field was not part of the fetch.
    NotFetched(Key),
}

/// The keys one fetch asks for. Duplicates collapse into one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys(BTreeSet<Key>);

impl Keys {
    pub fn new(keys: &[Key]) -> Result<Keys, Error> {
        if keys.is_empty() {
            return Err(Error::NoKeys);
        }
        Ok(Keys(keys.iter().copied().collect()))
    }

    /// Every key.
    pub fn all() -> Keys {
        Keys(Key::ALL.iter().copied().collect())
    }

    pub fn contains(&self, key: Key) -> bool {
        self.0.contains(&key)
    }
}

/// A stored contact: an identifier and the fields it has. A blank value is not stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    fields: BTreeMap<Key, Value>,
}

impl Record {
    pub fn new(id: u64) -> Self {
        Record { id, fields: BTreeMap::new() }
    }

    /// The value of a field the record holds, or `None` when it is blank or was never set.
    pub fn get(&self, key: Key) -> Option<&Value> {
        self.fields.get(&key)
    }

    /// Sets a field, replacing an earlier value. A value that is all blank removes the field.
    pub fn set(&mut self, key: Key, value: Value) {
        let blank = match &value {
            Value::Text(text) => text.trim().is_empty(),
            Value::List(items) => items.iter().all(|item| item.trim().is_empty()),
        };
        if blank {
            self.fields.remove(&key);
        } else {
            self.fields.insert(key, value);
        }
    }
}

/// A record as a fetch returns it: its identifier and only the fields the fetch asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub id: u64,
    fetched: BTreeSet<Key>,
    values: BTreeMap<Key, Value>,
}

impl Partial {
    /// The value of a fetched field, or `None` when the record has none. Any other field is an error.
    pub fn get(&self, key: Key) -> Result<Option<&Value>, Error> {
        if !self.fetched.contains(&key) {
            return Err(Error::NotFetched(key));
        }
        Ok(self.values.get(&key))
    }

    /// The text of a fetched text field; an empty string when there is none.
    pub fn text(&self, key: Key) -> Result<&str, Error> {
        match self.get(key)? {
            Some(Value::Text(text)) => Ok(text.as_str()),
            _ => Ok(""),
        }
    }

    /// The keys this record was fetched with.
    pub fn fetched(&self) -> impl Iterator<Item = Key> + '_ {
        self.fetched.iter().copied()
    }
}

/// The partial record a fetch with `keys` returns for a stored record.
pub fn project(record: &Record, keys: &Keys) -> Partial {
    let values: BTreeMap<Key, Value> = record
        .fields
        .iter()
        .filter(|(key, _)| keys.contains(**key))
        .map(|(key, value)| (*key, value.clone()))
        .collect();
    Partial { id: record.id, fetched: keys.0.clone(), values }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        let mut r = Record::new(7);
        r.set(Key::GivenName, Value::Text("Ada".into()));
        r.set(Key::FamilyName, Value::Text("Lovelace".into()));
        r.set(Key::Emails, Value::List(vec!["ada@example.com".into()]));
        r.set(Key::Note, Value::Text("   ".into()));
        r
    }

    #[test]
    fn a_fetch_needs_at_least_one_key_and_duplicates_collapse() {
        assert_eq!(Keys::new(&[]), Err(Error::NoKeys));
        let keys = Keys::new(&[Key::Note, Key::Note]).unwrap();
        assert!(keys.contains(Key::Note));
        assert_eq!(keys.0.len(), 1);
    }

    #[test]
    fn the_partial_record_holds_only_the_fetched_fields() {
        let keys = Keys::new(&[Key::GivenName, Key::Phones]).unwrap();
        let p = project(&sample(), &keys);
        assert_eq!(p.id, 7);
        assert_eq!(p.text(Key::GivenName), Ok("Ada"));
        assert_eq!(p.get(Key::Phones), Ok(None));
        assert_eq!(p.get(Key::Emails), Err(Error::NotFetched(Key::Emails)));
        assert_eq!(p.text(Key::FamilyName), Err(Error::NotFetched(Key::FamilyName)));
    }

    #[test]
    fn a_missing_field_reads_as_absent_not_as_an_error() {
        let keys = Keys::new(&[Key::Organization]).unwrap();
        let p = project(&sample(), &keys);
        assert_eq!(p.get(Key::Organization), Ok(None));
        assert_eq!(p.text(Key::Organization), Ok(""));
    }

    #[test]
    fn blank_values_are_not_stored() {
        let p = project(&sample(), &Keys::all());
        assert_eq!(p.get(Key::Note), Ok(None));
        assert_eq!(p.fetched().count(), Key::ALL.len());
    }

    #[test]
    fn all_keys_fetch_every_stored_field() {
        let p = project(&sample(), &Keys::all());
        assert_eq!(p.text(Key::FamilyName), Ok("Lovelace"));
        assert_eq!(p.get(Key::Emails), Ok(Some(&Value::List(vec!["ada@example.com".into()]))));
        assert_eq!(p.get(Key::Address), Ok(None));
    }

    #[test]
    fn every_key_has_a_wire_name_that_reads_back() {
        for key in Key::ALL {
            assert_eq!(Key::from_name(key.name()), Some(key));
        }
        assert_eq!(Key::from_name("nickname"), None);
        assert_eq!(Key::GivenName.name(), "given_name");
    }

    #[test]
    fn a_record_reads_back_the_fields_it_holds() {
        let r = sample();
        assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("Lovelace".into())));
        assert_eq!(r.get(Key::Note), None);
    }

    #[test]
    fn setting_a_field_again_replaces_it() {
        let mut r = sample();
        r.set(Key::GivenName, Value::Text("Augusta".into()));
        let p = project(&r, &Keys::new(&[Key::GivenName]).unwrap());
        assert_eq!(p.text(Key::GivenName), Ok("Augusta"));
    }
}
