//! The pending contact pick of each app. An app starts a pick (the contacts service `pick`), the shell's picker sheet
//! shows the list (`contact_pick_sheet`) and the person selects a contact or one value of it, and the app reads the
//! result with `pick_result`. Only the final choice is kept for the app. A pick is per app and replaces an earlier one
//! that has not been read yet.
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
    app_name: String,
    names: BTreeMap<u64, String>,
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

/// Starts a pick for an app, replacing any pick it has not read. `app_name` is the name the sheet shows.
pub fn start(app_id: &str, app_name: &str, picker: Picker, entries: &[Entry]) {
    let names = entries.iter().map(|e| (e.id, e.name.clone())).collect();
    PICKS.with(|p| {
        p.borrow_mut().insert(app_id.to_string(), Pending { app_name: app_name.to_string(), names, picker: Some(picker), chosen: None })
    });
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

/// Ends an open pick without a choice, as the person pressing Cancel does.
pub fn cancel(app_id: &str) {
    PICKS.with(|p| {
        p.borrow_mut().remove(app_id);
    });
}

/// The app with an open pick (still waiting for the person), with the name the sheet shows. Apps are listed in order,
/// so the first open one is shown first.
pub fn open_app() -> Option<(String, String)> {
    PICKS.with(|p| {
        p.borrow()
            .iter()
            .find(|(_, pending)| pending.picker.is_some())
            .map(|(id, pending)| (id.clone(), pending.app_name.clone()))
    })
}

/// The contacts the open pick lists, with their names, for the sheet to draw. `None` when no pick is open.
pub fn choices(app_id: &str) -> Option<Vec<(u64, String)>> {
    PICKS.with(|p| {
        let picks = p.borrow();
        let pending = picks.get(app_id)?;
        let picker = pending.picker.as_ref()?;
        Some(picker.visible().into_iter().map(|id| (id, pending.names.get(&id).cloned().unwrap_or_default())).collect())
    })
}

/// The values the sheet offers for the open contact in a property pick: each field's key, and its position among the
/// contact's values under that key (the `nth` that `choose` takes). `None` when no contact is open.
pub fn values(app_id: &str) -> Option<Vec<(Key, usize, String)>> {
    PICKS.with(|p| {
        let picks = p.borrow();
        let picker = picks.get(app_id)?.picker.as_ref()?;
        picker.open_contact()?;
        let fields: Vec<&Field> = picker.detail();
        let mut seen: BTreeMap<Key, usize> = BTreeMap::new();
        Some(
            fields
                .into_iter()
                .map(|f| {
                    let nth = *seen.entry(f.key).and_modify(|n| *n += 1).or_insert(0);
                    (f.key, nth, f.value.clone())
                })
                .collect(),
        )
    })
}

/// Selects a contact in the open pick. In contact mode the choice is kept for the app; in property mode the contact
/// opens and the sheet offers its values next. Returns whether a choice was kept.
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
        let entries = entries_of(&[ada()]);
        start(app, "Notes", Picker::new(entries.clone(), Mode::Contact { need: None }), &entries);
        assert!(matches!(outcome(app), Outcome::Pending));
        assert_eq!(open_app(), Some((app.to_string(), "Notes".to_string())));
        assert_eq!(choices(app), Some(vec![(4, "Ada Lovelace".to_string())]));
        assert_eq!(select(app, 4), Ok(true));
        assert!(matches!(outcome(app), Outcome::Chosen(Choice::Contact(4))));
        assert!(matches!(outcome(app), Outcome::None));
    }

    #[test]
    fn a_property_pick_offers_the_values_then_chooses_one() {
        let app = "is.example.pick-property";
        let mode = mode_of("property", None, vec![Key::Email, Key::Phone]).unwrap();
        let entries = entries_of(&[ada()]);
        start(app, "Mail", Picker::new(entries.clone(), mode), &entries);
        assert_eq!(values(app), None, "no contact is open yet");
        assert_eq!(select(app, 4), Ok(false));
        assert_eq!(values(app), Some(vec![(Key::Phone, 0, "555 0100".to_string()), (Key::Email, 0, "ada@example.com".to_string())]));
        assert_eq!(choose(app, Key::Email, 0), Ok(true));
        match outcome(app) {
            Outcome::Chosen(Choice::Property { contact, key, value }) => {
                assert_eq!((contact, key, value.as_str()), (4, Key::Email, "ada@example.com"));
            }
            other => panic!("expected a property choice, got {other:?}"),
        }
    }

    #[test]
    fn cancel_ends_an_open_pick_without_a_choice() {
        let app = "is.example.pick-cancel";
        let entries = entries_of(&[ada()]);
        start(app, "Notes", Picker::new(entries.clone(), Mode::Contact { need: None }), &entries);
        cancel(app);
        assert!(matches!(outcome(app), Outcome::None));
        assert_eq!(select(app, 4), Err(Error::NoneOpen));
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
