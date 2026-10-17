//! Suggested contacts: the people other apps saw and donated (the sender of a message in Mail) that are not in the
//! address book. The list comes from the shell's `contacts` service (`bifrost_app::contacts::suggested`, over the
//! `contact_donation` model); this module keeps what the Suggestions screen shows and the answers waiting to be sent
//! back. Accepting turns a suggestion into a contact; dismissing blocks its address for good. Nothing is sent anywhere
//! but between this app and the shell on this machine.

use bifrost_app::contacts::suggested::{Feedback, Handle, Suggested};
use bifrost_app::controls::contact_donation::normalise;
use std::collections::BTreeSet;

/// Most suggestions asked for at once.
pub const MAX_ASKED: u32 = 50;

/// The suggestions on show and the answers not yet sent to the shell.
#[derive(Debug, Default)]
pub struct Suggestions {
    items: Vec<Suggested>,
    answers: Vec<(String, Feedback)>,
}

impl Suggestions {
    /// Replaces the list with what the shell sent, leaving out everyone whose handle is in `known` (the normalised
    /// emails and phone numbers the address book holds) and everyone answered since.
    pub fn set(&mut self, items: Vec<Suggested>, known: &BTreeSet<String>) {
        let answered: BTreeSet<&str> = self.answers.iter().map(|(key, _)| key.as_str()).collect();
        self.items = items.into_iter().filter(|s| !known.contains(&s.key) && !answered.contains(s.key.as_str())).collect();
    }

    pub fn items(&self) -> &[Suggested] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Answers the suggestion at `index`: it leaves the list and the answer waits for [`Suggestions::take_answers`].
    /// Returns the suggestion that was answered.
    pub fn answer(&mut self, index: usize, answer: Feedback) -> Option<Suggested> {
        if index >= self.items.len() {
            return None;
        }
        let suggestion = self.items.remove(index);
        self.answers.push((suggestion.key.clone(), answer));
        Some(suggestion)
    }

    /// The answers to send to the shell, once.
    pub fn take_answers(&mut self) -> Vec<(String, Feedback)> {
        std::mem::take(&mut self.answers)
    }
}

/// What a row shows first: the name donated with the person, else the email address or phone number.
pub fn title_of(s: &Suggested) -> String {
    if s.name.is_empty() {
        s.value.clone()
    } else {
        s.name.clone()
    }
}

/// The second line of a row: the handle, and how many apps saw the person when more than one did.
pub fn detail_of(s: &Suggested) -> String {
    let seen = if s.sources > 1 { format!("Seen in {} apps", s.sources) } else { "Seen in an app".to_string() };
    if s.name.is_empty() {
        seen
    } else {
        format!("{}  {}", s.value, seen)
    }
}

/// The name, phone number and email address of the contact an accepted suggestion becomes. A suggestion with no name
/// gives the contact no name; the card then shows its email address or phone number.
pub fn contact_fields(s: &Suggested) -> (String, String, String) {
    match s.handle {
        Handle::Email => (s.name.clone(), String::new(), s.value.clone()),
        Handle::Phone => (s.name.clone(), s.value.clone(), String::new()),
    }
}

/// The normalised form of an email address or phone number, for matching a suggestion's key against the address book.
pub fn handle_key(handle: Handle, value: &str) -> Option<String> {
    normalise(handle, value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(key: &str, handle: Handle, name: &str, sources: u32) -> Suggested {
        Suggested { key: key.into(), handle, value: key.into(), name: name.into(), sources }
    }

    #[test]
    fn the_list_leaves_out_people_already_in_the_book_and_people_already_answered() {
        let mut list = Suggestions::default();
        let known: BTreeSet<String> = ["ada@example.com".to_string()].into();
        let sent = vec![
            suggestion("ada@example.com", Handle::Email, "Ada", 2),
            suggestion("bob@example.com", Handle::Email, "Bob", 1),
            suggestion("cy@example.com", Handle::Email, "", 1),
        ];
        list.set(sent.clone(), &known);
        assert_eq!(list.items().iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["bob@example.com", "cy@example.com"]);
        let bob = list.answer(0, Feedback::Dismissed).unwrap();
        assert_eq!(bob.key, "bob@example.com");
        // the shell has not seen the answer yet and still lists Bob: he does not come back
        list.set(sent, &known);
        assert_eq!(list.len(), 1);
        assert_eq!(list.take_answers(), vec![("bob@example.com".to_string(), Feedback::Dismissed)]);
        assert!(list.take_answers().is_empty(), "answers are sent once");
        assert!(list.answer(5, Feedback::Accepted).is_none());
    }

    #[test]
    fn a_row_shows_the_name_or_the_handle_and_how_many_apps_saw_the_person() {
        let named = suggestion("ada@example.com", Handle::Email, "Ada Lovelace", 2);
        assert_eq!((title_of(&named).as_str(), detail_of(&named).as_str()), ("Ada Lovelace", "ada@example.com  Seen in 2 apps"));
        let bare = suggestion("+3545550100", Handle::Phone, "", 1);
        assert_eq!((title_of(&bare).as_str(), detail_of(&bare).as_str()), ("+3545550100", "Seen in an app"));
    }

    #[test]
    fn an_accepted_suggestion_becomes_a_contact_with_its_email_or_phone() {
        assert_eq!(contact_fields(&suggestion("ada@example.com", Handle::Email, "Ada Lovelace", 1)), ("Ada Lovelace".into(), String::new(), "ada@example.com".into()));
        assert_eq!(contact_fields(&suggestion("+3545550100", Handle::Phone, "", 1)), (String::new(), "+3545550100".into(), String::new()));
        assert_eq!(handle_key(Handle::Email, " Ada@Example.com "), Some("ada@example.com".to_string()));
        assert_eq!(handle_key(Handle::Phone, "+354 555 0100"), Some("+3545550100".to_string()));
        assert_eq!(handle_key(Handle::Email, "nope"), None);
    }
}
