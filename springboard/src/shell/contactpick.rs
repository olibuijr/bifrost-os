//! The pending contact pick of each app. An app starts a pick (the contacts service `pick`), the shell's picker sheet
//! shows the list and lets the person select a contact or one value of it, and the app reads the result with
//! `pick_result`. Only the final choice is kept for the app; the sheet calls `select` and `choose`, and `visible`
//! gives it the list to draw. A pick is per app and replaces an earlier one that has not been read yet.
use crate::shell::contactstore::Contact;
use bifrost_ui::contact_card::{Entry, Field, Key};
use bifrost_ui::contact_picker::{Choice, Error, Mode, Picker};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Where a pick stands for one app.
#[derive(Debug)]
pub enum Outcome {
    /// The app has no pick.
    None,
    /// The sheet is still open.
    Pending,
    /// The person chose; reading the outcome clears it.
    Chosen(Choice),
}

struct Pending {
    picker: Option<Picker>,
    chosen: Option<Choice>,
}

thread_local! {
    static PICKS: RefCell<BTreeMap<String, Pending>> = RefCell::new(BTreeMap::new());
}

/// The card key for a field name an app asks for.
pub fn card_key(name: &str) -> Option<Key> {
    match name {
        "company" => Some(Key::Company),
        "phone" => Some(Key::Phone),
        "email" => Some(Key::Email),
        "address" => Some(Key::Address),
        "note" => Some(Key::Note),
        _ => None,
    }
}

/// Starts a pick for an app, replacing any pick it has not read.
pub fn start(app_id: &str, picker: Picker) {
    PICKS.with(|p| p.borrow_mut().insert(app_id.to_string(), Pending { picker: Some(picker), chosen: None }));
}

/// What the app's pick stands at. Reading a chosen result clears it.
pub fn outcome(app_id: &str) -> Outcome {
    PICKS.with(|p| {
        let mut picks = p.borrow_mut();
        let (chosen, open) = match picks.get_mut(app_id) {
            None => return Outcome::None,
            Some(pending) => (pending.chosen.take(), pending.picker.is_some()),
        };
        match (chosen, open) {
            (Some(choice), _) => {
                picks.remove(app_id);
                Outcome::Chosen(choice)
            }
            (None, true) => Outcome::Pending,
            (None, false) => Outcome::None,
        }
    })
}

/// The contacts the open pick lists, for the sheet to draw. `None` when no pick is open.
pub fn visible(app_id: &str) -> Option<Vec<u64>> {
    PICKS.with(|p| p.borrow().get(app_id).and_then(|pending| pending.picker.as_ref().map(|picker| picker.visible())))
}

/// Selects a contact in the open pick. In contact mode the choice is kept for the app; in property mode the contact
/// opens and the sheet calls `choose` next. Returns whether a choice was kept.
pub fn select(app_id: &str, id: u64) -> Result<bool, Error> {
    step(app_id, |picker| picker.select(id))
}

/// Chooses the `nth` value under `key` of the open contact, which keeps the choice for the app.
pub fn choose(app_id: &str, key: Key, nth: usize) -> Result<bool, Error> {
    step(app_id, |picker| picker.choose(key, nth).map(Some))
}

/// Runs one sheet step on the open pick. A step that makes the final choice keeps it for the app and closes the
/// picker, so the app reads it with `outcome`.
fn step(app_id: &str, run: impl FnOnce(&mut Picker) -> Result<Option<Choice>, Error>) -> Result<bool, Error> {
    PICKS.with(|p| {
        let mut picks = p.borrow_mut();
        let pending = picks.get_mut(app_id).ok_or(Error::NoneOpen)?;
        let picker = pending.picker.as_mut().ok_or(Error::NoneOpen)?;
        match run(picker)? {
            Some(choice) => {
                pending.chosen = Some(choice);
                pending.picker = None;
                Ok(true)
            }
            None => Ok(false),
        }
    })
}

/// Builds the picker entries of the store's contacts: the name, and the company, phone, email, address and note.
pub fn entries_of(contacts: &[Contact]) -> Vec<Entry> {
    contacts
        .iter()
        .map(|c| {
            let mut fields = Vec::new();
            let mut push = |key: Key, label: &str, value: &str| {
                if !value.trim().is_empty() {
                    fields.push(Field { key, label: label.into(), value: value.trim().into() });
                }
            };
            push(Key::Company, "company", &c.company);
            push(Key::Phone, "phone", &c.phone);
            push(Key::Email, "email", &c.email);
            push(Key::Address, "address", &c.address);
            push(Key::Note, "note", &c.notes);
            Entry { id: c.id, name: c.display_name().into(), fields }
        })
        .collect()
}

/// The picker mode an app asked for: a whole contact (optionally one with a value under `need`), or one value under
/// the listed keys.
pub fn mode_of(mode: &str, need: Option<Key>, keys: Vec<Key>) -> Result<Mode, String> {
    match mode {
        "contact" => Ok(Mode::Contact { need }),
        "property" if !keys.is_empty() => Ok(Mode::Property { keys }),
        "property" => Err("a property pick needs keys".into()),
        _ => Err("a pick is a contact or a property".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ada() -> Contact {
        Contact { id: 4, name: "Ada Lovelace".into(), phone: "555 0100".into(), email: "ada@example.com".into(), ..Contact::default() }
    }

    #[test]
    fn a_contact_pick_keeps_only_the_chosen_contact_for_the_app() {
        let app = "is.example.pick-contact";
        assert!(matches!(outcome(app), Outcome::None));
        start(app, Picker::new(entries_of(&[ada()]), Mode::Contact { need: None }));
        assert!(matches!(outcome(app), Outcome::Pending));
        assert_eq!(visible(app), Some(vec![4]));
        assert_eq!(select(app, 4), Ok(true));
        assert!(matches!(outcome(app), Outcome::Chosen(Choice::Contact(4))));
        assert!(matches!(outcome(app), Outcome::None));
    }

    #[test]
    fn a_property_pick_chooses_one_value_of_the_opened_contact() {
        let app = "is.example.pick-property";
        let mode = mode_of("property", None, vec![Key::Email]).unwrap();
        start(app, Picker::new(entries_of(&[ada()]), mode));
        assert_eq!(select(app, 4), Ok(false));
        assert_eq!(choose(app, Key::Email, 0), Ok(true));
        match outcome(app) {
            Outcome::Chosen(Choice::Property { contact, key, value }) => {
                assert_eq!((contact, key, value.as_str()), (4, Key::Email, "ada@example.com"));
            }
            other => panic!("expected a property choice, got {other:?}"),
        }
    }

    #[test]
    fn a_step_with_no_open_pick_is_refused() {
        assert_eq!(select("is.example.pick-none", 4), Err(Error::NoneOpen));
    }

    #[test]
    fn a_pick_names_its_mode_and_keys() {
        assert!(mode_of("property", None, vec![]).is_err());
        assert!(mode_of("sheet", None, vec![]).is_err());
        assert_eq!(card_key("phone"), Some(Key::Phone));
        assert_eq!(card_key("birthday"), None);
    }
}
