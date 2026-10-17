//! Recipient autocomplete: the suggestions a To, Cc or Bcc field offers while an address is typed, and the field's
//! token model. Suggestions merge two sources, the address book ([`Contact`] email addresses) and the recipients Mail
//! and Messages used last ([`RecipientRecents`]), without repeating an address.
//!
//! Ranking, best first: a favourite recipient; a match at the start of the address or of a name word before a match
//! inside one; a recent recipient before a contact that was never mailed; then name order. An address found in both
//! sources is one suggestion with the contact's name. Nothing leaves the machine: the address book and the recents are
//! the only inputs.
//!
//! [`RecipientField`] is the field itself: the committed recipients, the text being typed, the suggestion highlighted
//! with the arrow keys, and the keys that commit (return, comma, tab) or remove the last recipient (backspace on empty
//! text). Pure model: the caller draws the tokens and the list, and passes the contacts and recents in.

use crate::contact_book::Contact;
use crate::recipient_recents::RecipientRecents;

/// Where a suggestion came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Only in the recents.
    Recent,
    /// Only in the address book.
    Contact,
    /// In both.
    Both,
}

/// One row of the suggestion list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The address in lower case.
    pub address: String,
    /// The display name: the contact's when there is one, else the recents' name, else the address.
    pub name: String,
    /// The label of the address in the contact ("Work"), when the address book supplied one.
    pub label: Option<String>,
    pub favourite: bool,
    pub source: Source,
}

impl Suggestion {
    /// The token text the field shows: `Name <address>`, or the address alone when it has no separate name.
    pub fn token(&self) -> String {
        if self.name.is_empty() || self.name == self.address {
            self.address.clone()
        } else {
            format!("{} <{}>", self.name, self.address)
        }
    }
}

/// The name a contact is listed under: given and family name, else nickname, else organization.
pub fn display_name(c: &Contact) -> String {
    let full = [c.given_name.as_str(), c.family_name.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
    if !full.is_empty() {
        full
    } else if !c.nickname.is_empty() {
        c.nickname.clone()
    } else {
        c.organization_name.clone()
    }
}

/// How well `needle` (lower case) matches `hay`: 2 at the start of the text or of a word, 1 inside a word, 0 not at all.
fn match_rank(hay: &str, needle: &str) -> u8 {
    let hay = hay.to_lowercase();
    if hay.starts_with(needle) || hay.split(|c: char| !c.is_alphanumeric()).any(|w| !w.is_empty() && w.starts_with(needle)) {
        2
    } else if hay.contains(needle) {
        1
    } else {
        0
    }
}

/// Suggestions for the text typed so far, at most `limit`. Empty text suggests the recents only (the list a field
/// shows when it is first focused); contacts need at least one typed character.
pub fn complete(text: &str, contacts: &[Contact], recents: &RecipientRecents, limit: usize) -> Vec<Suggestion> {
    let needle = text.trim().to_lowercase();
    let mut rows: Vec<(Suggestion, u8)> = Vec::new();
    for r in recents.suggest(&needle, usize::MAX) {
        let rank = if needle.is_empty() { 2 } else { match_rank(&r.address, &needle).max(match_rank(&r.name, &needle)) };
        rows.push((Suggestion { address: r.address, name: r.name, label: None, favourite: r.favourite, source: Source::Recent }, rank));
    }
    if !needle.is_empty() {
        for c in contacts {
            let name = display_name(c);
            for e in &c.email_addresses {
                let address = e.value.trim().to_lowercase();
                if !address.contains('@') {
                    continue;
                }
                let rank = match_rank(&address, &needle).max(match_rank(&name, &needle)).max(match_rank(&c.nickname, &needle));
                if rank == 0 {
                    continue;
                }
                let label = e.label.as_deref().filter(|l| !l.is_empty()).map(|l| crate::contact_book::label::standard_name(l).unwrap_or(l).to_string());
                if let Some((s, r)) = rows.iter_mut().find(|(s, _)| s.address == address) {
                    s.source = Source::Both;
                    s.label = label;
                    if !name.is_empty() {
                        s.name = name.clone();
                    }
                    *r = (*r).max(rank);
                } else {
                    let shown = if name.is_empty() { address.clone() } else { name.clone() };
                    rows.push((Suggestion { address, name: shown, label, favourite: false, source: Source::Contact }, rank));
                }
            }
        }
    }
    // the sort is stable, so the recents keep their most-recent-first order among equals
    rows.sort_by(|(a, ar), (b, br)| {
        b.favourite
            .cmp(&a.favourite)
            .then(br.cmp(ar))
            .then((a.source == Source::Contact).cmp(&(b.source == Source::Contact)))
            .then_with(|| if a.source == Source::Contact && b.source == Source::Contact { a.name.to_lowercase().cmp(&b.name.to_lowercase()) } else { std::cmp::Ordering::Equal })
    });
    rows.into_iter().take(limit).map(|(s, _)| s).collect()
}

/// An address-only validity check for typed text: one `@` with text on both sides, no spaces.
pub fn is_address(text: &str) -> bool {
    let t = text.trim();
    match t.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !t.contains(char::is_whitespace) && !domain.contains('@'),
        None => false,
    }
}

/// The keys that act on a recipient field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Comma,
    Tab,
    Backspace,
    Up,
    Down,
    Escape,
}

/// A recipient field: committed recipients, typed text and the suggestion list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipientField {
    /// The committed recipients, as their addresses.
    pub recipients: Vec<String>,
    text: String,
    suggestions: Vec<Suggestion>,
    highlighted: Option<usize>,
}

impl RecipientField {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// The typed text changed: the list is rebuilt, without the recipients already in the field, and nothing is
    /// highlighted until an arrow key moves there.
    pub fn set_text(&mut self, text: &str, contacts: &[Contact], recents: &RecipientRecents, limit: usize) {
        self.text = text.to_string();
        let all = complete(text, contacts, recents, usize::MAX);
        self.suggestions = all.into_iter().filter(|s| !self.recipients.contains(&s.address)).take(limit).collect();
        self.highlighted = None;
    }

    fn commit(&mut self, address: String) {
        if !self.recipients.contains(&address) {
            self.recipients.push(address);
        }
        self.text.clear();
        self.suggestions.clear();
        self.highlighted = None;
    }

    /// Handles a key. Returns the address that was committed or removed, when the key did either.
    pub fn press(&mut self, key: Key) -> Option<String> {
        match key {
            Key::Down if !self.suggestions.is_empty() => {
                self.highlighted = Some(self.highlighted.map_or(0, |i| (i + 1).min(self.suggestions.len() - 1)));
                None
            }
            Key::Up => {
                self.highlighted = match self.highlighted {
                    Some(0) | None => None,
                    Some(i) => Some(i - 1),
                };
                None
            }
            Key::Escape => {
                self.suggestions.clear();
                self.highlighted = None;
                None
            }
            Key::Return | Key::Comma | Key::Tab => {
                let address = match self.highlighted {
                    Some(i) => self.suggestions.get(i).map(|s| s.address.clone()),
                    None if is_address(&self.text) => Some(self.text.trim().to_lowercase()),
                    None => None,
                };
                let address = address?;
                self.commit(address.clone());
                Some(address)
            }
            Key::Backspace if self.text.is_empty() => self.recipients.pop(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_book::LabeledValue;

    fn person(given: &str, family: &str, emails: &[(&str, &str)]) -> Contact {
        Contact {
            given_name: given.into(),
            family_name: family.into(),
            email_addresses: emails.iter().map(|(l, v)| LabeledValue::new(Some(l), v.to_string())).collect(),
            ..Contact::default()
        }
    }

    fn recents(list: &[(&str, &str)]) -> RecipientRecents {
        let mut r = RecipientRecents::new(10);
        for (a, n) in list {
            r.record(a, n);
        }
        r
    }

    #[test]
    fn contacts_and_recents_merge_without_repeating_an_address() {
        let book = [person("Ada", "Lovelace", &[("_$!<Work>!$_", "ada@example.com")]), person("Bob", "Stone", &[("", "bob@example.com")])];
        let recent = recents(&[("ada@example.com", "")]);
        let got = complete("ada", &book, &recent, 10);
        assert_eq!(got.len(), 1, "one address, one row");
        assert_eq!(got[0].source, Source::Both);
        assert_eq!(got[0].name, "Ada Lovelace", "the contact's name wins over the bare address");
        assert_eq!(got[0].label.as_deref(), Some("Work"));
        assert_eq!(got[0].token(), "Ada Lovelace <ada@example.com>");
    }

    #[test]
    fn a_word_start_match_beats_a_match_inside_a_word() {
        let book = [person("Carol", "Marsh", &[("", "carol@example.com")]), person("Marcus", "Hale", &[("", "mh@example.com")])];
        let recent = recents(&[("farmar@example.com", "Farm Arthur")]);
        let got: Vec<String> = complete("mar", &book, &recent, 10).into_iter().map(|s| s.address).collect();
        // Marcus and Carol Marsh match at a word start; "farmar" only inside a word, so it comes last
        assert_eq!(got, vec!["carol@example.com", "mh@example.com", "farmar@example.com"]);
    }

    #[test]
    fn a_favourite_is_first_and_empty_text_lists_recents_only() {
        let book = [person("Zed", "Zimmer", &[("", "zed@example.com")])];
        let mut recent = recents(&[("a@example.com", "A"), ("b@example.com", "B")]);
        recent.set_favourite("a@example.com", true);
        let first: Vec<String> = complete("", &book, &recent, 10).into_iter().map(|s| s.address).collect();
        assert_eq!(first, vec!["a@example.com", "b@example.com"], "contacts need a typed character");
        assert!(complete("zed", &book, &recent, 10).iter().any(|s| s.source == Source::Contact));
        assert_eq!(complete("", &book, &recent, 1).len(), 1, "the limit applies");
        assert!(complete("qqq", &book, &recent, 10).is_empty());
    }

    #[test]
    fn a_contact_with_no_valid_address_is_never_suggested() {
        let book = [person("No", "Mail", &[("", "not-an-address")])];
        assert!(complete("no", &book, &RecipientRecents::new(5), 10).is_empty());
    }

    #[test]
    fn typing_then_return_commits_a_valid_address_and_backspace_removes_it() {
        let book = [person("Ada", "Lovelace", &[("", "ada@example.com")])];
        let recent = RecipientRecents::new(5);
        let mut field = RecipientField::new();
        field.set_text("ad", &book, &recent, 5);
        assert_eq!(field.suggestions().len(), 1);
        assert_eq!(field.press(Key::Return), None, "\"ad\" is not an address and nothing is highlighted");
        assert_eq!(field.press(Key::Down), None);
        assert_eq!(field.highlighted(), Some(0));
        assert_eq!(field.press(Key::Return).as_deref(), Some("ada@example.com"));
        assert_eq!(field.recipients, vec!["ada@example.com"]);
        assert_eq!(field.text(), "");
        field.set_text("ad", &book, &recent, 5);
        assert!(field.suggestions().is_empty(), "an address already in the field is not offered again");
        field.set_text("Bo@Example.com", &book, &recent, 5);
        assert_eq!(field.press(Key::Comma).as_deref(), Some("bo@example.com"), "a typed address commits in lower case");
        assert_eq!(field.press(Key::Backspace).as_deref(), Some("bo@example.com"), "backspace on empty text removes the last recipient");
        assert_eq!(field.recipients.len(), 1);
    }

    #[test]
    fn the_arrow_keys_stay_in_the_list_and_escape_closes_it() {
        let book = [person("Ann", "A", &[("", "ann@example.com")]), person("Anna", "B", &[("", "anna@example.com")])];
        let mut field = RecipientField::new();
        field.set_text("an", &book, &RecipientRecents::new(5), 5);
        field.press(Key::Down);
        field.press(Key::Down);
        field.press(Key::Down);
        assert_eq!(field.highlighted(), Some(1), "down stops at the last row");
        field.press(Key::Up);
        field.press(Key::Up);
        assert_eq!(field.highlighted(), None, "up from the first row returns to the typed text");
        field.press(Key::Escape);
        assert!(field.suggestions().is_empty());
        assert!(is_address("a@b.c") && !is_address("a b@c") && !is_address("@c") && !is_address("a@") && !is_address("a@@c"));
    }
}
