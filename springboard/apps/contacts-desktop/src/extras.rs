//! Extra labelled values for contacts: the second and later phone numbers, emails and addresses of a contact.
//!
//! The contact store (`springboard::shell::contactstore`) keeps one phone, one email and one address per contact, and
//! that format is not changed. The extra values live beside it in `values.txt` in the Contacts folder, keyed by the
//! contact's identifier. A missing file means no extra values, so a contacts file written before this file existed
//! opens with its single values only, and the first value of each kind stays in the store.
//!
//! ```text
//! springboard-contact-values-v1
//! <id><TAB>phone|email|address<TAB><label><TAB><value>
//! ```
//!
//! Labels are the store's predefined labels for the kind (`contactstore::LABELS`). Text is escaped with a backslash
//! (`\t`, `\n`, `\r` and `\\`). Pure std.

use springboard::shell::contactstore::{LABELS, MAX_VALUE};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const HEADER: &str = "springboard-contact-values-v1";
/// Most extra values kept for one contact. The card then shows at most three store values and six extra ones.
pub const MAX_EXTRAS: usize = 6;
const MAX_BYTES: usize = 1024 * 1024;

/// The kind of an extra value. The index is its row in `contactstore::LABELS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Phone,
    Email,
    Address,
}

impl ValueKind {
    pub fn index(self) -> usize {
        match self {
            ValueKind::Phone => 0,
            ValueKind::Email => 1,
            ValueKind::Address => 2,
        }
    }

    fn code(self) -> &'static str {
        match self {
            ValueKind::Phone => "phone",
            ValueKind::Email => "email",
            ValueKind::Address => "address",
        }
    }

    fn from_code(code: &str) -> Option<ValueKind> {
        match code {
            "phone" => Some(ValueKind::Phone),
            "email" => Some(ValueKind::Email),
            "address" => Some(ValueKind::Address),
            _ => None,
        }
    }
}

/// One extra value: its kind, its predefined label (such as `work`) and the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extra {
    pub kind: ValueKind,
    pub label: String,
    pub value: String,
}

impl Extra {
    /// True when the label is one the kind offers, and the value is non-empty, has no surrounding spaces, has no
    /// control characters and fits the store's value length.
    pub fn well_formed(&self) -> bool {
        let value = &self.value;
        LABELS[self.kind.index()].contains(&self.label.as_str())
            && !value.trim().is_empty()
            && value.trim() == value.as_str()
            && value.chars().count() <= MAX_VALUE
            && !value.chars().any(char::is_control)
    }
}

/// The extra values of every contact, by contact identifier. Contacts without extra values have no entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras(BTreeMap<u64, Vec<Extra>>);

impl Extras {
    /// The extra values of a contact, in the order they were added (empty when it has none).
    pub fn get(&self, id: u64) -> &[Extra] {
        match self.0.get(&id) {
            Some(values) => values.as_slice(),
            None => &[],
        }
    }

    /// Adds an extra value to a contact. Returns false, and adds nothing, when the value is not well formed or the
    /// contact already holds `MAX_EXTRAS` values.
    pub fn add(&mut self, id: u64, extra: Extra) -> bool {
        if !extra.well_formed() {
            return false;
        }
        let values = self.0.entry(id).or_default();
        if values.len() >= MAX_EXTRAS {
            return false;
        }
        values.push(extra);
        true
    }

    /// Removes a contact's extra values. Returns true when there were any.
    pub fn remove(&mut self, id: u64) -> bool {
        self.0.remove(&id).is_some()
    }

    /// Keeps the extra values of the contacts `keep` accepts, so values of deleted contacts are dropped on load.
    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.0.retain(|&id, _| keep(id));
    }

    /// True when one of the contact's extra values contains `query`, ignoring case. A blank query matches any contact
    /// with an extra value.
    pub fn matches(&self, id: u64, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        self.get(id).iter().any(|extra| extra.value.to_lowercase().contains(&query))
    }

    /// The file text: the header, then one line per extra value, contacts in identifier order.
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER}\n");
        for (id, values) in &self.0 {
            for extra in values {
                out.push_str(&format!("{id}\t{}\t{}\t{}\n", extra.kind.code(), escape(&extra.label), escape(&extra.value)));
            }
        }
        out
    }
}

/// Reads the file text. Returns `None` when the header is not this format's. Lines that do not read, or that the
/// rules refuse, are skipped.
pub fn parse(text: &str) -> Option<Extras> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return None;
    }
    let mut out = Extras::default();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let &[id, kind, label, value] = fields.as_slice() else {
            continue;
        };
        let (Ok(id), Some(kind)) = (id.parse::<u64>(), ValueKind::from_code(kind)) else {
            continue;
        };
        out.add(id, Extra { kind, label: unescape(label), value: unescape(value) });
    }
    Some(out)
}

/// Reads `values.txt`. A missing file is no extra values. A file that cannot be read, is too large or has another
/// header is an error, so the app does not overwrite it.
pub fn load_checked(path: &Path) -> io::Result<Extras> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Extras::default()),
        Err(e) => return Err(e),
        Ok(meta) if meta.len() > MAX_BYTES as u64 => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Contact values file too large"));
        }
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(path)?;
    parse(&text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unknown contact values format"))
}

/// Writes the file atomically: a temporary file is written beside it and then renamed over it.
pub fn save(path: &Path, values: &Extras) -> io::Result<()> {
    let text = values.to_text();
    if text.len() > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Too many contact values"));
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(kind: ValueKind, label: &str, value: &str) -> Extra {
        Extra { kind, label: label.into(), value: value.into() }
    }

    fn anna_values() -> Extras {
        let mut values = Extras::default();
        assert!(values.add(7, extra(ValueKind::Phone, "work", "555 0100")));
        assert!(values.add(7, extra(ValueKind::Email, "home", "anna@x.is")));
        values
    }

    #[test]
    fn the_file_text_is_the_header_then_one_line_per_value() {
        assert_eq!(
            anna_values().to_text(),
            "springboard-contact-values-v1\n7\tphone\twork\t555 0100\n7\temail\thome\tanna@x.is\n"
        );
    }

    #[test]
    fn values_round_trip_through_the_file_text_with_escapes() {
        let mut values = anna_values();
        assert!(values.add(9, extra(ValueKind::Address, "other", "Gata 1 \\ 2")));
        let text = values.to_text();
        assert!(text.contains("9\taddress\tother\tGata 1 \\\\ 2\n"), "{text}");
        assert_eq!(parse(&text), Some(values));
    }

    #[test]
    fn a_store_file_from_before_this_format_is_not_this_format() {
        assert_eq!(parse("springboard-contacts-v1\n1\tAnna\n"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn a_missing_file_is_no_extra_values() {
        let path = std::env::temp_dir().join(format!("extras-missing-{}", std::process::id())).join("values.txt");
        assert_eq!(load_checked(&path).unwrap(), Extras::default());
    }

    #[test]
    fn labels_outside_the_predefined_set_and_bad_lines_are_refused() {
        let mut values = Extras::default();
        assert!(!values.add(1, extra(ValueKind::Phone, "fax", "555")), "fax is not a phone label");
        assert!(!values.add(1, extra(ValueKind::Email, "mobile", "a@x.is")), "mobile is not an email label");
        assert!(!values.add(1, extra(ValueKind::Phone, "work", "  555  ")), "spaces around a value are refused");
        let text = "springboard-contact-values-v1\n1\tphone\tfax\t555\nx\tphone\twork\t1\n1\tfax\twork\t1\n2\temail\twork\tb@x.is\n";
        let read = parse(text).unwrap();
        assert_eq!(read.get(1), &[] as &[Extra]);
        assert_eq!(read.get(2), &[extra(ValueKind::Email, "work", "b@x.is")]);
    }

    #[test]
    fn a_contact_keeps_at_most_six_extra_values() {
        let mut values = Extras::default();
        for i in 0..MAX_EXTRAS {
            assert!(values.add(3, extra(ValueKind::Phone, "home", &format!("10{i}"))));
        }
        assert!(!values.add(3, extra(ValueKind::Phone, "home", "999")), "the seventh is refused");
        assert_eq!(values.get(3).len(), 6);
    }

    #[test]
    fn search_matches_extra_values_ignoring_case_and_skips_other_contacts() {
        let values = anna_values();
        assert!(values.matches(7, "ANNA@x"));
        assert!(values.matches(7, "0100"));
        assert!(!values.matches(7, "zzz"));
        assert!(!values.matches(8, "0100"), "no values for contact 8");
    }

    #[test]
    fn removing_and_pruning_drop_the_values_of_contacts() {
        let mut values = anna_values();
        assert!(values.add(8, extra(ValueKind::Phone, "work", "555 0800")));
        values.retain(|id| id != 8);
        assert!(values.get(8).is_empty());
        assert!(values.remove(7));
        assert!(!values.remove(7));
        assert_eq!(values, Extras::default());
    }
}
