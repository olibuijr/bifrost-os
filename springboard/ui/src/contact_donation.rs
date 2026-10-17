//! Contact donations and the feedback on them: an app that sees a person who is not in the address book (the sender of a
//! message, a name in a calendar invitation) donates the name and one email address or phone number. The store keeps
//! the donations, and the address book offers them back as suggested contacts. The person answers each suggestion, and
//! the answer is the feedback: accepting turns it into a contact and ends the suggestion, dismissing blocks that address
//! so it is never offered again, even if an app donates it again.
//!
//! Everything stays on the machine: the donations are the apps' own, the feedback is a local list, and nothing is sent
//! anywhere. A person already in the address book is never suggested. Pure model: the caller keeps the clock and stores
//! the state ([`DonationStore::to_text`] and [`DonationStore::from_text`] are its file form).
//!
//! Ranking of suggestions: the address donated by the most apps first, then the most donations, then the most recent.

use std::collections::{BTreeMap, BTreeSet};

use crate::contact_book::{Contact, LabeledValue};

/// Most donations kept. When full, the donation seen longest ago is dropped.
pub const MAX_DONATIONS: usize = 500;
/// Longest name or value accepted, in bytes.
pub const MAX_FIELD: usize = 200;

/// What kind of handle a donation carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Handle {
    Email,
    Phone,
}

/// The person's answer to a suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback {
    /// Made into a contact.
    Accepted,
    /// Never offer this address again.
    Dismissed,
}

/// Why a donation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The name or value is empty or too long.
    BadField,
    /// An email needs one `@` with text on both sides; a phone number needs at least 3 digits and only digits and `+ - ( ) .` and spaces.
    BadHandle,
    /// The person dismissed this address; donations of it are ignored.
    Blocked,
}

/// One suggested person, as the store holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Donated {
    /// The normalised handle: an email in lower case, or a phone number reduced to its digits and a leading `+`.
    pub key: String,
    pub handle: Handle,
    /// The value as first donated, for display.
    pub display: String,
    /// The most recent non-empty name donated.
    pub name: String,
    /// The apps that donated it.
    pub sources: BTreeSet<String>,
    pub count: u32,
    /// The time of the latest donation, in the caller's clock (seconds).
    pub last: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DonationStore {
    entries: BTreeMap<String, Donated>,
    blocked: BTreeSet<String>,
    accepted: BTreeSet<String>,
}

/// Normalises a handle, or says why it is not one.
pub fn normalise(handle: Handle, value: &str) -> Result<String, Refusal> {
    let v = value.trim();
    match handle {
        Handle::Email => match v.split_once('@') {
            Some((l, d)) if !l.is_empty() && !d.is_empty() && !v.contains(char::is_whitespace) && !d.contains('@') => Ok(v.to_lowercase()),
            _ => Err(Refusal::BadHandle),
        },
        Handle::Phone => {
            if !v.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c)) {
                return Err(Refusal::BadHandle);
            }
            let digits: String = v.chars().enumerate().filter(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '+')).map(|(_, c)| c).collect();
            if digits.chars().filter(char::is_ascii_digit).count() < 3 {
                return Err(Refusal::BadHandle);
            }
            Ok(digits)
        }
    }
}

impl DonationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// An app donates a person it saw. `now` is the caller's clock in seconds.
    pub fn donate(&mut self, source: &str, name: &str, handle: Handle, value: &str, now: u64) -> Result<(), Refusal> {
        let (name, value) = (name.trim(), value.trim());
        if source.is_empty() || value.is_empty() || value.len() > MAX_FIELD || name.len() > MAX_FIELD {
            return Err(Refusal::BadField);
        }
        let key = normalise(handle, value)?;
        if self.blocked.contains(&key) {
            return Err(Refusal::Blocked);
        }
        let entry = self.entries.entry(key.clone()).or_insert_with(|| Donated { key, handle, display: value.to_string(), name: String::new(), sources: BTreeSet::new(), count: 0, last: now });
        if !name.is_empty() {
            entry.name = name.to_string();
        }
        entry.sources.insert(source.to_string());
        entry.count += 1;
        entry.last = entry.last.max(now);
        if self.entries.len() > MAX_DONATIONS {
            if let Some(oldest) = self.entries.values().min_by_key(|d| d.last).map(|d| d.key.clone()) {
                self.entries.remove(&oldest);
            }
        }
        Ok(())
    }

    /// Removes everything an app donated (the app was uninstalled or its donations were switched off). A person only
    /// that app donated disappears; one other apps also donated stays.
    pub fn forget_source(&mut self, source: &str) {
        for d in self.entries.values_mut() {
            d.sources.remove(source);
        }
        self.entries.retain(|_, d| !d.sources.is_empty());
    }

    /// The people to suggest, best first, at most `limit`: not accepted, not dismissed, and not already in `book`.
    pub fn suggestions(&self, book: &[Contact], limit: usize) -> Vec<&Donated> {
        let known = known_handles(book);
        let mut rows: Vec<&Donated> = self.entries.values().filter(|d| !self.accepted.contains(&d.key) && !self.blocked.contains(&d.key) && !known.contains(&d.key)).collect();
        rows.sort_by(|a, b| b.sources.len().cmp(&a.sources.len()).then(b.count.cmp(&a.count)).then(b.last.cmp(&a.last)).then(a.key.cmp(&b.key)));
        rows.truncate(limit);
        rows
    }

    /// Records the person's answer for the suggestion with `key`. Accepting returns the contact to save; dismissing
    /// returns none and blocks the address. A key that is not donated is ignored.
    pub fn feedback(&mut self, key: &str, answer: Feedback) -> Option<Contact> {
        let d = self.entries.get(key)?.clone();
        match answer {
            Feedback::Accepted => {
                self.accepted.insert(d.key.clone());
                Some(contact_of(&d))
            }
            Feedback::Dismissed => {
                self.entries.remove(key);
                self.blocked.insert(d.key);
                None
            }
        }
    }

    /// Whether `key` was dismissed.
    pub fn is_blocked(&self, key: &str) -> bool {
        self.blocked.contains(key)
    }

    /// Lifts a dismissal (the person changed their mind in Settings).
    pub fn unblock(&mut self, key: &str) -> bool {
        self.blocked.remove(key)
    }

    /// The file form: one line per donation, `D<TAB>handle<TAB>count<TAB>last<TAB>key<TAB>display<TAB>name<TAB>sources`
    /// (sources joined by commas), then `B<TAB>key` for each dismissal and `A<TAB>key` for each accepted person.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for d in self.entries.values() {
            let h = if d.handle == Handle::Email { "email" } else { "phone" };
            let sources: Vec<&str> = d.sources.iter().map(String::as_str).collect();
            out.push_str(&format!("D\t{h}\t{}\t{}\t{}\t{}\t{}\t{}\n", d.count, d.last, clean(&d.key), clean(&d.display), clean(&d.name), clean(&sources.join(","))));
        }
        for k in &self.blocked {
            out.push_str(&format!("B\t{}\n", clean(k)));
        }
        for k in &self.accepted {
            out.push_str(&format!("A\t{}\n", clean(k)));
        }
        out
    }

    /// Reads the file form; a malformed line is skipped.
    pub fn from_text(text: &str) -> Self {
        let mut s = Self::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["D", h, count, last, key, display, name, sources] => {
                    let handle = if *h == "email" { Handle::Email } else if *h == "phone" { Handle::Phone } else { continue };
                    let (Ok(count), Ok(last)) = (count.parse(), last.parse()) else { continue };
                    let sources: BTreeSet<String> = sources.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect();
                    if key.is_empty() || sources.is_empty() {
                        continue;
                    }
                    s.entries.insert(key.to_string(), Donated { key: key.to_string(), handle, display: display.to_string(), name: name.to_string(), sources, count, last });
                }
                ["B", key] if !key.is_empty() => {
                    s.blocked.insert(key.to_string());
                }
                ["A", key] if !key.is_empty() => {
                    s.accepted.insert(key.to_string());
                }
                _ => {}
            }
        }
        s
    }
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Every email (lower case) and phone number (normalised) the address book already holds.
fn known_handles(book: &[Contact]) -> BTreeSet<String> {
    let mut known = BTreeSet::new();
    for c in book {
        for e in &c.email_addresses {
            if let Ok(k) = normalise(Handle::Email, &e.value) {
                known.insert(k);
            }
        }
        for p in &c.phone_numbers {
            if let Ok(k) = normalise(Handle::Phone, &p.value) {
                known.insert(k);
            }
        }
    }
    known
}

/// The contact an accepted suggestion becomes. A name of two or more words splits at its last space into given and
/// family name; one word is a given name; no name leaves the handle alone.
fn contact_of(d: &Donated) -> Contact {
    let mut c = Contact::default();
    match d.name.rsplit_once(' ') {
        Some((given, family)) => {
            c.given_name = given.trim().to_string();
            c.family_name = family.trim().to_string();
        }
        None => c.given_name = d.name.clone(),
    }
    match d.handle {
        Handle::Email => c.email_addresses.push(LabeledValue::new(None, d.display.clone())),
        Handle::Phone => c.phone_numbers.push(LabeledValue::new(None, d.display.clone())),
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(addr: &str) -> Contact {
        Contact { email_addresses: vec![LabeledValue::new(None, addr.to_string())], ..Contact::default() }
    }

    #[test]
    fn a_donated_person_is_suggested_with_the_latest_name() {
        let mut s = DonationStore::new();
        s.donate("mail", "Ada", Handle::Email, "Ada@Example.com", 10).unwrap();
        s.donate("mail", "Ada Lovelace", Handle::Email, "ada@example.com ", 20).unwrap();
        let got = s.suggestions(&[], 10);
        assert_eq!(got.len(), 1, "one address, one suggestion");
        assert_eq!((got[0].key.as_str(), got[0].name.as_str(), got[0].count, got[0].last), ("ada@example.com", "Ada Lovelace", 2, 20));
    }

    #[test]
    fn bad_donations_are_refused() {
        let mut s = DonationStore::new();
        assert_eq!(s.donate("", "A", Handle::Email, "a@b.c", 1), Err(Refusal::BadField));
        assert_eq!(s.donate("mail", "A", Handle::Email, "", 1), Err(Refusal::BadField));
        assert_eq!(s.donate("mail", "A", Handle::Email, "no-at-sign", 1), Err(Refusal::BadHandle));
        assert_eq!(s.donate("mail", "A", Handle::Email, "a b@c.d", 1), Err(Refusal::BadHandle));
        assert_eq!(s.donate("mail", "A", Handle::Phone, "12", 1), Err(Refusal::BadHandle));
        assert_eq!(s.donate("mail", "A", Handle::Phone, "555-abc", 1), Err(Refusal::BadHandle));
        assert_eq!(s.donate("mail", &"x".repeat(MAX_FIELD + 1), Handle::Email, "a@b.c", 1), Err(Refusal::BadField));
        assert!(s.is_empty());
        assert_eq!(normalise(Handle::Phone, "+354 (555) 12-34"), Ok("+3545551234".to_string()));
    }

    #[test]
    fn a_person_already_in_the_address_book_is_not_suggested() {
        let mut s = DonationStore::new();
        s.donate("mail", "Ada", Handle::Email, "ada@example.com", 1).unwrap();
        s.donate("mail", "Bob", Handle::Phone, "555 0100", 2).unwrap();
        let mut bob = Contact::default();
        bob.phone_numbers.push(LabeledValue::new(None, "(555) 0100".to_string()));
        let got: Vec<&str> = s.suggestions(&[email("ADA@example.com"), bob], 10).into_iter().map(|d| d.key.as_str()).collect();
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn suggestions_rank_by_apps_then_donations_then_recency() {
        let mut s = DonationStore::new();
        s.donate("mail", "One", Handle::Email, "one@x.io", 100).unwrap();
        s.donate("mail", "Two", Handle::Email, "two@x.io", 50).unwrap();
        s.donate("mail", "Two", Handle::Email, "two@x.io", 60).unwrap();
        s.donate("calendar", "Three", Handle::Email, "three@x.io", 10).unwrap();
        s.donate("mail", "Three", Handle::Email, "three@x.io", 11).unwrap();
        let order: Vec<&str> = s.suggestions(&[], 10).into_iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, vec!["Three", "Two", "One"], "two apps, then two donations, then one");
        assert_eq!(s.suggestions(&[], 2).len(), 2);
    }

    #[test]
    fn accepting_makes_a_contact_and_dismissing_blocks_the_address() {
        let mut s = DonationStore::new();
        s.donate("mail", "Ada Byron Lovelace", Handle::Email, "ada@example.com", 1).unwrap();
        s.donate("mail", "Bob", Handle::Phone, "555 0100", 2).unwrap();
        let ada = s.feedback("ada@example.com", Feedback::Accepted).unwrap();
        assert_eq!((ada.given_name.as_str(), ada.family_name.as_str()), ("Ada Byron", "Lovelace"));
        assert_eq!(ada.email_addresses[0].value, "ada@example.com");
        assert!(s.suggestions(&[], 10).iter().all(|d| d.key != "ada@example.com"), "an accepted person is no longer suggested");
        assert!(s.feedback("5550100", Feedback::Dismissed).is_none());
        assert!(s.is_blocked("5550100"));
        assert_eq!(s.donate("mail", "Bob", Handle::Phone, "555-0100", 3), Err(Refusal::Blocked), "a dismissed address stays dismissed");
        assert!(s.suggestions(&[], 10).is_empty());
        assert!(s.feedback("nobody@x.io", Feedback::Accepted).is_none(), "an unknown key is ignored");
        assert!(s.unblock("5550100"));
        s.donate("mail", "Bob", Handle::Phone, "555-0100", 4).unwrap();
        assert_eq!(s.suggestions(&[], 10).len(), 1);
    }

    #[test]
    fn forgetting_an_app_keeps_people_other_apps_donated() {
        let mut s = DonationStore::new();
        s.donate("mail", "Ada", Handle::Email, "ada@x.io", 1).unwrap();
        s.donate("calendar", "Ada", Handle::Email, "ada@x.io", 2).unwrap();
        s.donate("mail", "Bob", Handle::Email, "bob@x.io", 3).unwrap();
        s.forget_source("mail");
        let keys: Vec<&str> = s.suggestions(&[], 10).into_iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, vec!["ada@x.io"]);
    }

    #[test]
    fn the_store_keeps_at_most_the_cap_dropping_the_stalest() {
        let mut s = DonationStore::new();
        for i in 0..=MAX_DONATIONS as u64 {
            s.donate("mail", "P", Handle::Email, &format!("p{i}@x.io"), i + 1).unwrap();
        }
        assert_eq!(s.len(), MAX_DONATIONS);
        assert!(s.suggestions(&[], 1000).iter().all(|d| d.key != "p0@x.io"), "the oldest donation went");
    }

    #[test]
    fn the_file_form_round_trips_and_skips_bad_lines() {
        let mut s = DonationStore::new();
        s.donate("mail", "Ada\tL", Handle::Email, "ada@x.io", 5).unwrap();
        s.donate("calendar", "", Handle::Phone, "+354 555 1234", 6).unwrap();
        s.donate("mail", "Bob", Handle::Email, "bob@x.io", 7).unwrap();
        s.feedback("bob@x.io", Feedback::Dismissed);
        s.feedback("ada@x.io", Feedback::Accepted);
        let text = s.to_text();
        let back = DonationStore::from_text(&format!("junk\nD\temail\tx\n{text}D\tfax\t1\t1\tk\tk\tk\tmail\n"));
        assert_eq!(back.len(), 2);
        assert!(back.is_blocked("bob@x.io"));
        assert_eq!(back.suggestions(&[], 10).len(), 1, "the accepted one is still hidden after a reload");
        assert_eq!(back.to_text().lines().count(), text.lines().count());
    }
}
