//! Contact picker model: the list a picker shows, its search, and the choice it returns. A picker lets the user choose one
//! whole contact, or one value of a contact (a phone number, an email). Only the final choice leaves the picker, so the
//! app that asked needs no read access to the store. The model draws nothing: the shell draws the list from
//! [`Picker::visible`] and the open contact from [`Picker::detail`]. Contacts that cannot be chosen stay listed, greyed.

use std::collections::BTreeSet;

use crate::contact_card::{Entry, Field, Key};
use crate::fuzzy;

/// What the picker lets the user choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The user chooses a whole contact. With `need`, only contacts that have a value under that key can be chosen.
    Contact { need: Option<Key> },
    /// The user opens a contact and chooses one value. Only values under `keys` can be chosen.
    Property { keys: Vec<Key> },
}

/// The result handed back to the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Contact(u64),
    Property { contact: u64, key: Key, value: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownContact,
    /// The contact is listed but cannot be chosen: it is disabled, or it lacks the value the mode needs.
    NotEnabled,
    /// The step does not fit the mode: a contact pick in property mode, or a value choice in contact mode.
    WrongMode,
    /// No contact is open, so there is no value to choose.
    NoneOpen,
    /// The key is not allowed by the mode, or the open contact has no value under it at that index.
    NoValue,
}

#[derive(Clone, Debug)]
pub struct Picker {
    entries: Vec<Entry>,
    mode: Mode,
    /// The keys the detail shows. Empty means every key.
    displayed: Vec<Key>,
    disabled: BTreeSet<u64>,
    query: String,
    open: Option<u64>,
}

impl Picker {
    pub fn new(entries: Vec<Entry>, mode: Mode) -> Picker {
        Picker { entries, mode, displayed: Vec::new(), disabled: BTreeSet::new(), query: String::new(), open: None }
    }

    /// Sets the keys the detail shows (the displayed property keys).
    pub fn with_displayed(mut self, keys: Vec<Key>) -> Picker {
        self.displayed = keys;
        self
    }

    /// Marks a contact as not choosable. It stays in the list.
    pub fn disable(&mut self, id: u64) {
        self.disabled.insert(id);
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_string();
    }

    fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// True when `id` can be chosen in the current mode.
    pub fn enabled(&self, id: u64) -> bool {
        let Some(entry) = self.entry(id) else { return false };
        if self.disabled.contains(&id) {
            return false;
        }
        match &self.mode {
            Mode::Contact { need: Some(key) } => entry.has(*key),
            _ => true,
        }
    }

    /// The ids the list shows: every contact sorted by name, or the matches for the query, best match first.
    pub fn visible(&self) -> Vec<u64> {
        if self.query.is_empty() {
            let mut all: Vec<&Entry> = self.entries.iter().collect();
            all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
            return all.iter().map(|e| e.id).collect();
        }
        let texts: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                let mut text = e.name.clone();
                for field in &e.fields {
                    text.push(' ');
                    text.push_str(&field.value);
                }
                text
            })
            .collect();
        fuzzy::rank(&self.query, texts.iter().map(|s| s.as_str()))
            .into_iter()
            .map(|(i, _)| self.entries[i].id)
            .collect()
    }

    /// The fields the detail shows for the open contact, in card order. Empty when no contact is open.
    pub fn detail(&self) -> Vec<&Field> {
        let Some(entry) = self.open.and_then(|id| self.entry(id)) else { return Vec::new() };
        let mut fields: Vec<&Field> = entry
            .fields
            .iter()
            .filter(|f| !f.value.trim().is_empty() && (self.displayed.is_empty() || self.displayed.contains(&f.key)))
            .collect();
        fields.sort_by_key(|f| f.key);
        fields
    }

    /// The contact open in the detail, if any.
    pub fn open_contact(&self) -> Option<u64> {
        self.open
    }

    /// Picks a contact in contact mode (returns the choice), or opens it in property mode (returns `None`).
    pub fn select(&mut self, id: u64) -> Result<Option<Choice>, Error> {
        if self.entry(id).is_none() {
            return Err(Error::UnknownContact);
        }
        if !self.enabled(id) {
            return Err(Error::NotEnabled);
        }
        if matches!(self.mode, Mode::Contact { .. }) {
            return Ok(Some(Choice::Contact(id)));
        }
        self.open = Some(id);
        Ok(None)
    }

    /// Chooses the `nth` value (0 is the first) under `key` of the open contact, in property mode.
    pub fn choose(&mut self, key: Key, nth: usize) -> Result<Choice, Error> {
        let Mode::Property { keys } = &self.mode else { return Err(Error::WrongMode) };
        let Some(open) = self.open else { return Err(Error::NoneOpen) };
        if !keys.contains(&key) {
            return Err(Error::NoValue);
        }
        let entry = self.entry(open).ok_or(Error::UnknownContact)?;
        let value = entry.values(key).get(nth).map(|f| f.value.trim().to_string()).ok_or(Error::NoValue)?;
        self.open = None;
        Ok(Choice::Property { contact: open, key, value })
    }

    /// Closes the open contact and returns to the list.
    pub fn back(&mut self) {
        self.open = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_card::{Entry, Field, Key};

    fn field(key: Key, label: &str, value: &str) -> Field {
        Field { key, label: label.to_string(), value: value.to_string() }
    }

    fn people() -> Vec<Entry> {
        vec![
            Entry {
                id: 1,
                name: "Bo Ketils".to_string(),
                fields: vec![field(Key::Phone, "mobile", "666-0000")],
            },
            Entry {
                id: 2,
                name: "Anna Lind".to_string(),
                fields: vec![
                    field(Key::Phone, "mobile", "555-1111"),
                    field(Key::Phone, "work", "555-2222"),
                    field(Key::Email, "work", "anna@example.com"),
                ],
            },
        ]
    }

    #[test]
    fn contact_mode_returns_the_chosen_contact() {
        let mut picker = Picker::new(people(), Mode::Contact { need: None });
        assert_eq!(picker.select(2), Ok(Some(Choice::Contact(2))));
        assert_eq!(picker.select(99), Err(Error::UnknownContact));
    }

    #[test]
    fn need_limits_which_contacts_can_be_chosen() {
        let mut picker = Picker::new(people(), Mode::Contact { need: Some(Key::Email) });
        assert!(!picker.enabled(1));
        assert!(picker.enabled(2));
        assert_eq!(picker.select(1), Err(Error::NotEnabled));
        assert_eq!(picker.select(2), Ok(Some(Choice::Contact(2))));
    }

    #[test]
    fn disabled_contacts_stay_listed_but_cannot_be_chosen() {
        let mut picker = Picker::new(people(), Mode::Contact { need: None });
        picker.disable(1);
        assert_eq!(picker.visible(), vec![2, 1]);
        assert_eq!(picker.select(1), Err(Error::NotEnabled));
    }

    #[test]
    fn list_is_sorted_by_name_and_search_ranks_matches() {
        let mut picker = Picker::new(people(), Mode::Contact { need: None });
        assert_eq!(picker.visible(), vec![2, 1]);
        picker.set_query("666");
        assert_eq!(picker.visible(), vec![1]);
        picker.set_query("zzz");
        assert!(picker.visible().is_empty());
    }

    #[test]
    fn property_mode_opens_the_contact_then_returns_the_nth_value() {
        let mut picker = Picker::new(people(), Mode::Property { keys: vec![Key::Phone] });
        assert_eq!(picker.select(2), Ok(None));
        assert_eq!(picker.open_contact(), Some(2));
        assert_eq!(
            picker.choose(Key::Phone, 1),
            Ok(Choice::Property { contact: 2, key: Key::Phone, value: "555-2222".to_string() })
        );
        assert_eq!(picker.open_contact(), None);
    }

    #[test]
    fn property_choices_are_checked_against_mode_and_open_contact() {
        let mut picker = Picker::new(people(), Mode::Property { keys: vec![Key::Phone] });
        assert_eq!(picker.choose(Key::Phone, 0), Err(Error::NoneOpen));
        picker.select(2).unwrap();
        assert_eq!(picker.choose(Key::Email, 0), Err(Error::NoValue));
        assert_eq!(picker.choose(Key::Phone, 5), Err(Error::NoValue));
        let mut contacts = Picker::new(people(), Mode::Contact { need: None });
        assert_eq!(contacts.choose(Key::Phone, 0), Err(Error::WrongMode));
    }

    #[test]
    fn detail_shows_only_displayed_keys_and_back_closes_it() {
        let mut picker = Picker::new(people(), Mode::Property { keys: vec![Key::Phone, Key::Email] })
            .with_displayed(vec![Key::Email]);
        picker.select(2).unwrap();
        let shown: Vec<&str> = picker.detail().iter().map(|f| f.value.as_str()).collect();
        assert_eq!(shown, vec!["anna@example.com"]);
        picker.back();
        assert!(picker.detail().is_empty());
        assert_eq!(picker.choose(Key::Email, 0), Err(Error::NoneOpen));
    }
}
