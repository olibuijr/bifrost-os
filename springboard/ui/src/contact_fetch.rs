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
    Urls,
    Birthday,
    Dates,
}

impl Key {
    /// Every key, in declaration order.
    pub const ALL: [Key; 10] = [
        Key::GivenName,
        Key::FamilyName,
        Key::Organization,
        Key::Phones,
        Key::Emails,
        Key::Address,
        Key::Note,
        Key::Urls,
        Key::Birthday,
        Key::Dates,
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
            Key::Urls => "urls",
            Key::Birthday => "birthday",
            Key::Dates => "dates",
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
    /// Labeled multi-values (`CNLabeledValue`): phone numbers, email addresses, URLs.
    Labeled(Vec<LabeledValue>),
    /// A date given as components (`birthday`).
    Date(DateComponents),
    /// Labeled dates (`dates`).
    Dates(Vec<(String, DateComponents)>),
}

/// `CNLabelHome`, `CNLabelWork`, `CNLabelOther` and the phone labels. The labels are plain names, so a label the
/// person typed is stored the same way.
pub const LABEL_HOME: &str = "home";
pub const LABEL_WORK: &str = "work";
pub const LABEL_OTHER: &str = "other";
/// `CNLabelPhoneNumberMobile`.
pub const LABEL_PHONE_MOBILE: &str = "mobile";
/// `CNLabelPhoneNumberMain`.
pub const LABEL_PHONE_MAIN: &str = "main";

/// One entry of a multi-value property (`CNLabeledValue`): a label, a value and an identifier that is stable within
/// the contact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabeledValue {
    pub label: String,
    pub value: String,
    pub identifier: String,
}

impl LabeledValue {
    /// An entry whose identifier is derived from its position by [`LabeledValue::numbered`].
    pub fn new(label: &str, value: &str) -> Self {
        LabeledValue { label: label.into(), value: value.into(), identifier: String::new() }
    }

    /// The entries with identifiers `1`, `2`, ... in order, so the same list always has the same identifiers.
    pub fn numbered(mut items: Vec<LabeledValue>) -> Vec<LabeledValue> {
        for (i, item) in items.iter_mut().enumerate() {
            item.identifier = (i + 1).to_string();
        }
        items
    }
}

/// A date as components (`DateComponents`). A birthday may lack the year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateComponents {
    pub year: Option<i32>,
    pub month: u8,
    pub day: u8,
}

impl DateComponents {
    /// The components, or `None` for a month outside 1..=12 or a day outside 1..=31.
    pub fn new(year: Option<i32>, month: u8, day: u8) -> Option<Self> {
        ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some(DateComponents { year, month, day })
    }

    /// `YYYY-MM-DD`, or `--MM-DD` without a year.
    pub fn to_text(self) -> String {
        match self.year {
            Some(y) => format!("{y:04}-{:02}-{:02}", self.month, self.day),
            None => format!("--{:02}-{:02}", self.month, self.day),
        }
    }

    /// Reads what [`DateComponents::to_text`] wrote.
    pub fn from_text(text: &str) -> Option<Self> {
        let (year, rest) = match text.strip_prefix("--") {
            Some(rest) => (None, rest.to_string()),
            None => {
                let (y, rest) = text.split_once('-')?;
                (Some(y.parse().ok()?), rest.to_string())
            }
        };
        let (m, d) = rest.split_once('-')?;
        DateComponents::new(year, m.parse().ok()?, d.parse().ok()?)
    }
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
            Value::Labeled(items) => items.iter().all(|item| item.value.trim().is_empty()),
            Value::Date(_) => false,
            Value::Dates(items) => items.is_empty(),
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

    #[test]
    fn labeled_values_get_stable_identifiers_and_blank_lists_are_dropped() {
        let items = LabeledValue::numbered(vec![
            LabeledValue::new(LABEL_HOME, "a@example.com"),
            LabeledValue::new(LABEL_WORK, "b@example.com"),
        ]);
        assert_eq!(items[1].identifier, "2");
        let mut r = Record::new(1);
        r.set(Key::Emails, Value::Labeled(items));
        assert!(r.get(Key::Emails).is_some());
        r.set(Key::Emails, Value::Labeled(vec![LabeledValue::new(LABEL_HOME, " ")]));
        assert_eq!(r.get(Key::Emails), None);
    }

    #[test]
    fn date_components_validate_and_round_trip_with_and_without_a_year() {
        assert_eq!(DateComponents::new(Some(1815), 12, 10).unwrap().to_text(), "1815-12-10");
        assert_eq!(DateComponents::new(None, 3, 7).unwrap().to_text(), "--03-07");
        assert_eq!(DateComponents::from_text("--03-07"), DateComponents::new(None, 3, 7));
        assert_eq!(DateComponents::from_text("1815-12-10"), DateComponents::new(Some(1815), 12, 10));
        assert_eq!(DateComponents::new(None, 13, 1), None);
        assert_eq!(DateComponents::from_text("nonsense"), None);
    }
}
