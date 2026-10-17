//! People in the headers of a message: splitting a `From` value into the parts a contact needs, and checking whether the
//! address book already holds an address (Message > Add Sender to Contacts).

use crate::message::address_of;
use bifrost_app::contacts::{Key, Record, Value};
use bifrost_app::controls::contact_book::{label, Contact, LabeledValue as BookValue};

/// A sender split into what a contact card holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub given: String,
    pub family: String,
    pub address: String,
}

/// Reads a `Name <address>` value. A display name of the form `Last, First` is turned around; a longer name puts its
/// last word in the family name; with no display name the part of the address before `@` is the given name.
pub fn person_from(from: &str) -> Option<Person> {
    let address = address_of(from).to_string();
    if !address.contains('@') {
        return None;
    }
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
    let name = if name.contains('@') { "" } else { name };
    let (given, family) = if let Some((last, first)) = name.split_once(',') {
        (first.trim().to_string(), last.trim().to_string())
    } else if let Some((first, last)) = name.rsplit_once(' ') {
        (first.trim().to_string(), last.trim().to_string())
    } else if !name.is_empty() {
        (name.to_string(), String::new())
    } else {
        (address.split('@').next().unwrap_or("").to_string(), String::new())
    };
    Some(Person { given, family, address })
}

impl Person {
    /// The contact record to save: the names and the address.
    pub fn record(&self) -> Record {
        let mut record = Record::new(0);
        record.set(Key::GivenName, Value::Text(self.given.clone()));
        record.set(Key::FamilyName, Value::Text(self.family.clone()));
        record.set(Key::Emails, Value::List(vec![self.address.clone()]));
        record
    }
}

/// The address-book entry a fetched record is for the recipient suggestions: its names, organization and the email
/// addresses with their labels. Nothing else of the record is read.
pub fn book_entry(record: &Record) -> Contact {
    let text = |key: Key| match record.get(key) {
        Some(Value::Text(t)) => t.trim().to_string(),
        _ => String::new(),
    };
    let mut contact = Contact {
        identifier: record.id.to_string(),
        given_name: text(Key::GivenName),
        family_name: text(Key::FamilyName),
        organization_name: text(Key::Organization),
        ..Contact::default()
    };
    match record.get(Key::Emails) {
        Some(Value::Labeled(items)) => {
            for item in items {
                let tag = (!item.label.is_empty()).then(|| label::from_standard_name(&item.label).unwrap_or_else(|| item.label.clone()));
                contact.email_addresses.push(BookValue::new(tag.as_deref(), item.value.clone()));
            }
        }
        Some(Value::List(items)) => contact.email_addresses.extend(items.iter().map(|a| BookValue::new(None, a.clone()))),
        _ => {}
    }
    contact
}

/// True when a fetched contact already has this email address.
pub fn has_address(record: &Record, address: &str) -> bool {
    match record.get(Key::Emails) {
        Some(Value::Labeled(items)) => items.iter().any(|item| item.value.trim().eq_ignore_ascii_case(address)),
        Some(Value::List(items)) => items.iter().any(|item| item.trim().eq_ignore_ascii_case(address)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::contacts::LabeledValue;

    fn person(from: &str) -> (String, String, String) {
        let p = person_from(from).unwrap();
        (p.given, p.family, p.address)
    }

    #[test]
    fn names_are_split_into_given_and_family() {
        assert_eq!(person("Ana Smith <ana@example.com>"), ("Ana".into(), "Smith".into(), "ana@example.com".into()));
        assert_eq!(person("\"Smith, Ana\" <ana@example.com>"), ("Ana".into(), "Smith".into(), "ana@example.com".into()));
        assert_eq!(person("Anna Maria Jonsdottir <am@example.com>"), ("Anna Maria".into(), "Jonsdottir".into(), "am@example.com".into()));
        assert_eq!(person("Ana <ana@example.com>"), ("Ana".into(), String::new(), "ana@example.com".into()));
    }

    #[test]
    fn a_bare_address_gives_its_local_part_as_the_name() {
        assert_eq!(person("bo@example.com"), ("bo".into(), String::new(), "bo@example.com".into()));
        assert_eq!(person("<bo@example.com>"), ("bo".into(), String::new(), "bo@example.com".into()));
        assert_eq!(person("bo@example.com <bo@example.com>"), ("bo".into(), String::new(), "bo@example.com".into()));
    }

    #[test]
    fn a_value_without_an_address_is_no_person() {
        assert!(person_from("Undisclosed recipients").is_none());
        assert!(person_from("").is_none());
    }

    #[test]
    fn the_record_holds_the_names_and_the_address() {
        let record = person_from("Ana Smith <ana@example.com>").unwrap().record();
        assert_eq!(record.get(Key::GivenName), Some(&Value::Text("Ana".into())));
        assert_eq!(record.get(Key::FamilyName), Some(&Value::Text("Smith".into())));
        assert_eq!(record.get(Key::Emails), Some(&Value::List(vec!["ana@example.com".into()])));
        let nameless = person_from("<x@example.com>").unwrap().record();
        assert!(nameless.get(Key::FamilyName).is_none(), "a blank field is not kept");
    }

    #[test]
    fn a_record_becomes_an_address_book_entry_with_labeled_emails() {
        let mut record = Record::new(3);
        record.set(Key::GivenName, Value::Text("Ana".into()));
        record.set(Key::FamilyName, Value::Text("Smith".into()));
        record.set(Key::Emails, Value::Labeled(vec![LabeledValue::new("work", "ana@example.com"), LabeledValue::new("", "ana@home.example")]));
        let entry = book_entry(&record);
        assert_eq!((entry.given_name.as_str(), entry.family_name.as_str()), ("Ana", "Smith"));
        assert_eq!(entry.email_addresses.len(), 2);
        assert_eq!(entry.email_addresses[0].label.as_deref(), Some("_$!<Work>!$_"), "a standard label keeps its book form");
        assert_eq!(entry.email_addresses[1].label, None);
        let listed = Record::new(4);
        assert!(book_entry(&listed).email_addresses.is_empty());
    }

    #[test]
    fn a_fetched_contact_is_found_by_its_address_without_case() {
        let mut record = Record::new(7);
        record.set(Key::Emails, Value::Labeled(vec![LabeledValue::new("work", "Ana@Example.com")]));
        assert!(has_address(&record, "ana@example.com"));
        assert!(!has_address(&record, "bo@example.com"));
        assert!(!has_address(&Record::new(8), "ana@example.com"));
    }
}
