//! The Contacts side of App Intents: a person in the address book as an `AppEntity`, and the "Find Contacts" intent.
//!
//! A contact record becomes an [`Entity`] of type [`CONTACT_TYPE`] whose id is the contact id, whose title is the
//! display name (given, middle and family names and the suffix, or the organization when the contact has no name) and
//! whose subtitle is the first email address, else the first phone number, else the organization. An `entity.query`
//! over contact records ([`query_entities`]) answers the three query shapes the intents layer parses: suggested
//! (every contact, capped), ids, and a string matched against the name, organization, emails and phones.
//!
//! The caller decides which records it may see (`contacts::fetch` honours the person's access choice), so a limited
//! grant only ever yields the picked contacts here.

use crate::contacts::{Key, Record, Value};
use crate::intents::{Entity, EntityQuery, Intent, Param, ParamType};

/// The entity type name contacts carry.
pub const CONTACT_TYPE: &str = "contact";
/// Id of the intent that finds contacts by a search string.
pub const FIND_ID: &str = "contacts_find";
/// Name of the search-string parameter of [`FIND_ID`].
pub const QUERY_PARAM: &str = "query";
/// How many contacts a suggested query returns at most.
pub const MAX_SUGGESTED: usize = 20;

/// The Find Contacts intent (a shortcut: the query is optional, so Search can run it without asking for input).
pub fn contacts_intents() -> Vec<Intent> {
    vec![Intent::new(FIND_ID, "Find Contacts", "Finds people in the address book by name, organization, email or phone.")
        .param(Param::new(QUERY_PARAM, "Search", ParamType::String).optional())
        .shortcut()]
}

fn text(record: &Record, key: Key) -> Option<&str> {
    match record.get(key) {
        Some(Value::Text(t)) if !t.trim().is_empty() => Some(t.trim()),
        _ => None,
    }
}

/// The values under a multi-value key (emails, phones, urls) as plain strings.
fn values(record: &Record, key: Key) -> Vec<&str> {
    match record.get(key) {
        Some(Value::Labeled(items)) => items.iter().map(|i| i.value.as_str()).collect(),
        Some(Value::List(items)) => items.iter().map(String::as_str).collect(),
        Some(Value::Text(t)) => vec![t.as_str()],
        _ => Vec::new(),
    }
}

/// The name a contact shows: given, middle and family names and the suffix joined by spaces; the organization when
/// none of them is set; `"No Name"` when the contact has neither.
pub fn display_name(record: &Record) -> String {
    let parts: Vec<&str> = [Key::GivenName, Key::MiddleName, Key::FamilyName, Key::NameSuffix].iter().filter_map(|&k| text(record, k)).collect();
    if !parts.is_empty() {
        return parts.join(" ");
    }
    text(record, Key::Organization).unwrap_or("No Name").to_string()
}

/// The contact as an App Intents entity.
pub fn contact_entity(record: &Record) -> Entity {
    let entity = Entity::new(CONTACT_TYPE, &record.id.to_string(), &display_name(record));
    let has_name = [Key::GivenName, Key::FamilyName].iter().any(|&k| text(record, k).is_some());
    let subtitle = values(record, Key::Emails)
        .first()
        .copied()
        .or_else(|| values(record, Key::Phones).first().copied())
        .or_else(|| if has_name { text(record, Key::Organization) } else { None });
    match subtitle {
        Some(s) => entity.subtitle(s),
        None => entity,
    }
}

/// True when `needle` (already lowercased, non-empty) is in the contact's name, organization, emails or phones.
fn matches(record: &Record, needle: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(needle);
    has(&display_name(record))
        || text(record, Key::Organization).is_some_and(has)
        || values(record, Key::Emails).into_iter().any(has)
        || values(record, Key::Phones).into_iter().any(has)
}

/// The contacts an `entity.query` selects, as entities in the order of `people`. Unknown ids are skipped. A string
/// query that is blank behaves like a suggested query.
pub fn query_entities(people: &[Record], query: &EntityQuery) -> Vec<Entity> {
    match query {
        EntityQuery::Suggested => people.iter().take(MAX_SUGGESTED).map(contact_entity).collect(),
        EntityQuery::Ids(ids) => people.iter().filter(|p| ids.iter().any(|i| *i == p.id.to_string())).map(contact_entity).collect(),
        EntityQuery::String(s) => {
            let needle = s.trim().to_lowercase();
            if needle.is_empty() {
                return query_entities(people, &EntityQuery::Suggested);
            }
            people.iter().filter(|p| matches(p, &needle)).map(contact_entity).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::LabeledValue;

    fn person(id: u64, given: &str, family: &str, email: &str, org: &str) -> Record {
        let mut r = Record::new(id);
        if !given.is_empty() {
            r.set(Key::GivenName, Value::Text(given.into()));
        }
        if !family.is_empty() {
            r.set(Key::FamilyName, Value::Text(family.into()));
        }
        if !email.is_empty() {
            r.set(Key::Emails, Value::Labeled(vec![LabeledValue::new("work", email)]));
        }
        if !org.is_empty() {
            r.set(Key::Organization, Value::Text(org.into()));
        }
        r
    }

    fn book() -> Vec<Record> {
        vec![
            person(1, "Ada", "Lovelace", "ada@example.com", "Analytical Engines"),
            person(2, "Grace", "Hopper", "", "Navy"),
            person(3, "", "", "", "Acme Ltd"),
        ]
    }

    #[test]
    fn a_contact_is_an_entity_with_its_name_and_first_email() {
        let e = contact_entity(&book()[0]);
        assert_eq!((e.ty.as_str(), e.id.as_str(), e.title.as_str()), (CONTACT_TYPE, "1", "Ada Lovelace"));
        assert_eq!(e.subtitle.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn without_an_email_the_subtitle_falls_back_to_the_organization() {
        let people = book();
        assert_eq!(contact_entity(&people[1]).subtitle.as_deref(), Some("Navy"));
        let mut phone_only = person(4, "Linus", "", "", "");
        phone_only.set(Key::Phones, Value::Labeled(vec![LabeledValue::new("mobile", "555-0100")]));
        assert_eq!(contact_entity(&phone_only).subtitle.as_deref(), Some("555-0100"));
    }

    #[test]
    fn a_contact_with_only_an_organization_is_titled_by_it() {
        let people = book();
        let e = contact_entity(&people[2]);
        assert_eq!(e.title, "Acme Ltd");
        assert_eq!(e.subtitle, None);
        assert_eq!(display_name(&Record::new(9)), "No Name");
    }

    #[test]
    fn a_string_query_matches_name_email_and_organization_ignoring_case() {
        let people = book();
        let ids = |q: &str| query_entities(&people, &EntityQuery::String(q.into())).into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids("LOVE"), ["1"]);
        assert_eq!(ids("example.com"), ["1"]);
        assert_eq!(ids("navy"), ["2"]);
        assert_eq!(ids("acme"), ["3"]);
        assert!(ids("zzz").is_empty());
        assert_eq!(ids("  ").len(), 3, "a blank string is a suggested query");
    }

    #[test]
    fn id_and_suggested_queries_follow_the_book_order() {
        let people = book();
        let by_id = query_entities(&people, &EntityQuery::Ids(vec!["3".into(), "1".into(), "99".into()]));
        assert_eq!(by_id.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        let many: Vec<Record> = (0..50).map(|i| person(i, "P", "Q", "", "")).collect();
        assert_eq!(query_entities(&many, &EntityQuery::Suggested).len(), MAX_SUGGESTED);
    }

    #[test]
    fn the_find_intent_is_a_shortcut_with_an_optional_query() {
        let list = contacts_intents();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].method(), "intent.contacts_find");
        assert!(list[0].shortcut);
        assert_eq!(list[0].params[0].name, QUERY_PARAM);
        assert!(list[0].validate(&bifrost_core::json::Json::Obj(vec![])).is_ok(), "the query may be left out");
    }
}
