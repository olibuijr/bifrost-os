//! Contact search over directory records. The person records of a directory node (type `users`) become contacts, and a
//! text query matches any of the contact attributes as a substring, ignoring case. The same query is also written as an
//! LDAP search filter (RFC 4515) for a directory server.
//!
//! Pure model: [`contact_filter`] returns the filter text and [`search_contacts`] runs the query over records the caller
//! already holds. No connection is opened here. A server client sends the filter and maps the entries it gets back into
//! [`DirRecord`]s, so the client is the part that needs a directory service.

use crate::directory_query::{DirRecord, Query};
use crate::recordsearch::{Comparison, Element};

/// The record type that holds people in a directory node.
pub const PERSON_RECORD_TYPE: &str = "users";

/// The attributes a contact search reads, in filter order. The names are the standard LDAP attribute names of a
/// person entry: `cn` is the full name, `mail` the addresses and `telephoneNumber` the phone numbers.
pub const CONTACT_ATTRIBUTES: [&str; 5] = ["cn", "givenName", "sn", "mail", "telephoneNumber"];

/// One person found by a contact search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

impl Contact {
    /// Reads a person record. The display name is the first `cn` value, or the record name when the record has none.
    pub fn from_record(record: &DirRecord) -> Contact {
        let name = record.values("cn").into_iter().next().unwrap_or_else(|| record.name().to_string());
        Contact { name, emails: record.values("mail"), phones: record.values("telephoneNumber") }
    }
}

/// Escapes a value for an LDAP search filter (RFC 4515). The characters `*`, `(`, `)`, `\` and NUL become a backslash
/// and two hex digits, so user text cannot add terms or wildcards to a filter.
pub fn escape_filter_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '*' => out.push_str("\\2a"),
            '(' => out.push_str("\\28"),
            ')' => out.push_str("\\29"),
            '\\' => out.push_str("\\5c"),
            '\0' => out.push_str("\\00"),
            other => out.push(other),
        }
    }
    out
}

/// The LDAP search filter for a contact query: person entries whose any contact attribute contains the trimmed text.
/// `None` when the query is blank, because a blank query would list the whole directory.
pub fn contact_filter(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let escaped = escape_filter_value(text);
    let terms: String = CONTACT_ATTRIBUTES.iter().map(|a| format!("({a}=*{escaped}*)")).collect();
    Some(format!("(&(objectClass=person)(|{terms}))"))
}

/// Runs a contact query over `records` and returns at most `limit` contacts, sorted by display name. A blank query
/// returns nothing. Records of other types, such as groups, are never returned.
pub fn search_contacts(records: &[&DirRecord], text: &str, limit: usize) -> Vec<Contact> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let query = Query {
        record_type: Some(PERSON_RECORD_TYPE.to_string()),
        predicate: Some(Element::Any(CONTACT_ATTRIBUTES.iter().map(|a| Element::on(a, Comparison::Contains, text)).collect())),
        search_attributes: CONTACT_ATTRIBUTES.iter().map(|a| a.to_string()).collect(),
        sort_attribute: Some("cn".to_string()),
        limit: Some(limit),
    };
    query.run(records).into_iter().map(Contact::from_record).collect()
}
