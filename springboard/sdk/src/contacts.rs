//! Contacts for apps (the person's address book): ask for access, fetch the fields an app needs for the contacts it may
//! see, optionally filtered by a query, and save changes (add, update and delete) to the contacts it may write.
//!
//! ```ignore
//! let status = contacts::access(ctx);            // reply: contacts::decode_status(&reply)
//! let id = contacts::fetch(ctx, &[Key::GivenName, Key::Emails], Some("ada"));
//! // in App::service_reply, once the reply for `id` arrives:
//! //   let people = contacts::decode_contacts(&reply).unwrap_or_default();   // Vec<Record>
//! //   people[0].text(...) / people[0].get(Key::Emails)
//! let mut ada = Record::new(0);
//! ada.set(Key::GivenName, Value::Text("Ada".into()));
//! ada.set(Key::Emails, Value::List(vec!["ada@example.com".into()]));
//! let saved = contacts::save(ctx, &SaveRequest { add: vec![ada], ..Default::default() });
//! // reply: contacts::decode_saved(&reply) gives the ids of the added contacts
//! ```
//! Saving needs full access: a limited grant reads only, and an app with no answer is refused.
//! The person decides access: an app that has not been answered reads `NotDetermined` and gets an error from `fetch`.
//! `request` reports the status and never grants it; the answer comes from the shell's access sheet or Settings. The
//! service is `springboard/src/shell/services/contacts.rs`, and the model is `bifrost_ui::contact_access` (every
//! contact, a picked set read-only, or none) with the records in `bifrost_ui::contact_fetch`.
pub use crate::controls::contact_access::{Choice, Error, Status};
pub use crate::controls::contact_fetch::{
    DateComponents, Key, LabeledValue, Record, Value, LABEL_HOME, LABEL_OTHER, LABEL_PHONE_MAIN, LABEL_PHONE_MOBILE,
    LABEL_WORK,
};
use crate::controls::contact_access::Access;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use std::collections::BTreeSet;

/// The service name the shell registers.
pub const SERVICE: &str = "contacts";

/// Asks for the access status and the contacts a limited grant holds. The reply decodes with [`decode_status`] and
/// [`decode_picked`].
pub fn access(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "access", &Json::Null)
}

/// Asks for the current status. It changes nothing: the person answers the access sheet or Settings.
pub fn request(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "request", &Json::Null)
}

/// Fetches the named fields of every contact the app may read, sorted by name, optionally filtered by `query`. An
/// empty `keys` is an error in the shell. The reply decodes with [`decode_contacts`].
pub fn fetch(ctx: &Ctx, keys: &[Key], query: Option<&str>) -> u32 {
    let mut params = vec![("keys".to_string(), Json::Arr(keys.iter().map(|k| Json::Str(k.name().into())).collect()))];
    if let Some(query) = query {
        params.push(("query".to_string(), Json::Str(query.into())));
    }
    service::call(ctx, SERVICE, "fetch", &Json::Obj(params))
}

/// The status in an `access` or `request` reply, or `None` for a reply of another shape.
pub fn decode_status(reply: &Json) -> Option<Status> {
    reply.get("status").and_then(Json::as_str).and_then(Status::from_name)
}

/// The contact ids a limited grant holds, from an `access` reply. Empty for any other status.
pub fn decode_picked(reply: &Json) -> BTreeSet<u64> {
    reply.get("picked").and_then(Json::as_array).map(|ids| ids.iter().filter_map(Json::as_u64).collect()).unwrap_or_default()
}

/// The changes a `save` call makes, applied as one: every add, update and delete happens, or none does. Only full access
/// writes. Each record uses the fields `fetch` reads (`given_name`, `family_name`, `organization`, `phones`, `emails`,
/// `address`, `note`); a list takes one value, because the book keeps one phone, one email and one address per contact.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRequest {
    /// New contacts. The shell gives each one its id, and the reply lists them in this order.
    pub add: Vec<Record>,
    /// Contacts to replace. Each record names the `id` of a contact the book holds, and its fields replace the
    /// contact's fields.
    pub update: Vec<Record>,
    /// Contacts to delete. Only each record's `id` is sent.
    pub delete: Vec<Record>,
}

impl SaveRequest {
    fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("add".into(), Json::Arr(self.add.iter().map(|r| record_json(r, false)).collect())),
            ("update".into(), Json::Arr(self.update.iter().map(|r| record_json(r, true)).collect())),
            ("delete".into(), Json::Arr(self.delete.iter().map(|r| Json::Obj(vec![("id".into(), Json::Num(r.id as f64))])).collect())),
        ])
    }
}

/// Saves the changes in `request`. The reply decodes with [`decode_saved`]. A request that fails in any item changes
/// nothing, and the reply is an error.
pub fn save(ctx: &Ctx, request: &SaveRequest) -> u32 {
    service::call(ctx, SERVICE, "save", &request.to_json())
}

/// The fields of a record as the wire names them, with its `id` when `with_id` is set.
fn record_json(record: &Record, with_id: bool) -> Json {
    let mut fields = Vec::new();
    if with_id {
        fields.push(("id".to_string(), Json::Num(record.id as f64)));
    }
    for key in Key::ALL {
        let value = match record.get(key) {
            Some(Value::Text(text)) => Json::Str(text.clone()),
            Some(Value::List(items)) => Json::Arr(items.iter().map(|item| Json::Str(item.clone())).collect()),
            Some(Value::Labeled(items)) => Json::Arr(items.iter().map(|item| Json::Str(item.value.clone())).collect()),
            // birthday and dates are not sent by `save` (the shell does not store them from a save yet)
            Some(Value::Date(_) | Value::Dates(_)) | None => continue,
        };
        fields.push((key.name().to_string(), value));
    }
    Json::Obj(fields)
}

/// The ids the shell gave the contacts a `save` added, in the order the request added them, or `None` for a reply of
/// another shape.
pub fn decode_saved(reply: &Json) -> Option<Vec<u64>> {
    reply.get("added")?.as_array()?.iter().map(Json::as_u64).collect()
}

/// The records in a `fetch` reply. A contact without a value for a key has no field for it, so `get` is `None`. A
/// reply of another shape, or one with a malformed record, is `None`.
pub fn decode_contacts(reply: &Json) -> Option<Vec<Record>> {
    reply.get("contacts")?.as_array()?.iter().map(decode_record).collect()
}

fn decode_record(v: &Json) -> Option<Record> {
    let mut record = Record::new(v.get("id")?.as_u64()?);
    for key in Key::ALL {
        let value = match (key, v.get(key.name())) {
            (Key::Phones | Key::Emails | Key::Urls, Some(Json::Arr(items))) => Some(Value::Labeled(decode_labeled(items))),
            (Key::Birthday, Some(date)) => decode_date(date).map(Value::Date),
            (Key::Dates, Some(Json::Arr(items))) => Some(Value::Dates(
                items.iter().filter_map(|d| Some((d.get("label")?.as_str()?.to_string(), decode_date(d)?))).collect(),
            )),
            (_, Some(Json::Str(text))) => Some(Value::Text(text.clone())),
            (_, Some(Json::Arr(items))) => {
                Some(Value::List(items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()))
            }
            _ => None,
        };
        if let Some(value) = value {
            record.set(key, value);
        }
    }
    Some(record)
}

/// Labeled entries from `{label, value, identifier?}` objects. An older reply holds plain strings: each becomes an
/// entry with no label. Identifiers the reply lacks are numbered by position.
fn decode_labeled(items: &[Json]) -> Vec<LabeledValue> {
    let entries = items
        .iter()
        .filter_map(|item| match item {
            Json::Str(value) => Some(LabeledValue::new("", value)),
            Json::Obj(_) => {
                let mut entry = LabeledValue::new(item.get("label").and_then(Json::as_str).unwrap_or(""), item.get("value")?.as_str()?);
                entry.identifier = item.get("identifier").and_then(Json::as_str).unwrap_or("").to_string();
                Some(entry)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let missing = entries.iter().any(|e| e.identifier.is_empty());
    if missing { LabeledValue::numbered(entries) } else { entries }
}

fn decode_date(v: &Json) -> Option<DateComponents> {
    let part = |name: &str| v.get(name).and_then(Json::as_u64);
    DateComponents::new(part("year").map(|y| y as i32), u8::try_from(part("month")?).ok()?, u8::try_from(part("day")?).ok()?)
}

/// Why a contacts call failed. The shell sends the reason as the error message; this maps the reasons it has a name for
/// and keeps any other message in `Other`. The variants are not `CNErrorCode` raw values: the raw numbers are not
/// verified yet, so apps match on the variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContactsError {
    /// The app holds no grant, or no write access (`CNError` authorization denied).
    NotGranted,
    /// A limited grant reads only; a save needs full access.
    ReadOnly,
    /// Access is restricted on this device and cannot be granted.
    Restricted,
    /// A limited grant was chosen with no contact picked.
    NothingPicked,
    /// The person already answered the access prompt.
    AlreadyDecided,
    /// The save names no change: it needs at least one add, update or delete.
    NothingToSave,
    /// An update names no contact id.
    MissingId,
    /// A contact is not valid: it has no name, company, address, phone or email, or a field is out of its limits.
    /// The message says which field.
    InvalidContact(String),
    /// The contacts file on disk is larger than the store reads.
    FileTooLarge,
    /// The contacts file is in a format this version does not read.
    UnknownFormat,
    /// Any other failure, with the shell's message.
    Other(String),
}

impl ContactsError {
    /// The variant for a shell error message. Messages the shell does not name keep their text in `Other`.
    pub fn from_message(message: &str) -> Self {
        match message {
            "contacts access is not granted" | "contacts write access is not granted" => ContactsError::NotGranted,
            "a limited contacts grant only reads; saving needs full access" => ContactsError::ReadOnly,
            "save needs add, update or delete" => ContactsError::NothingToSave,
            "update needs the id of a contact" => ContactsError::MissingId,
            "Invalid contact" => ContactsError::InvalidContact(String::new()),
            "Contacts file too large" => ContactsError::FileTooLarge,
            "Unknown contacts format" => ContactsError::UnknownFormat,
            other if other.starts_with("a contact needs a name") => ContactsError::InvalidContact(other.to_string()),
            other if other.ends_with("is a list") || other.ends_with("cannot be saved yet") || other.contains("takes one value per contact") || other.ends_with("is text or a list of text") || other.starts_with("a contact is an object") => {
                ContactsError::InvalidContact(other.to_string())
            }
            other => ContactsError::Other(other.to_string()),
        }
    }
}

impl From<Error> for ContactsError {
    fn from(error: Error) -> Self {
        match error {
            Error::Restricted => ContactsError::Restricted,
            Error::AlreadyDecided => ContactsError::AlreadyDecided,
            Error::NothingPicked => ContactsError::NothingPicked,
            Error::ReadOnly => ContactsError::ReadOnly,
            Error::NotAuthorized => ContactsError::NotGranted,
        }
    }
}

/// The typed error in a failed reply, or `None` for a reply that succeeded.
pub fn decode_error(reply: &Result<Json, String>) -> Option<ContactsError> {
    reply.as_ref().err().map(|message| ContactsError::from_message(message))
}

/// The access an app would hold after a choice, for a test or a preview of the sheet. The shell applies the same rules.
pub fn preview(choice: Choice) -> Result<Status, Error> {
    Access::new().request(choice)
}

/// Groups of contacts (CNGroup): nested groups and membership, with a recursive member listing. The model lives in the
/// app's own process; the shell's contact store keeps no groups yet.
pub mod groups {
    pub use bifrost_ui::contact_groups::{Error, Group, Groups};
}

/// Batched saves and change history for contacts (CNChangeHistoryFetchRequest). `Store::save` applies a batch of adds,
/// updates and deletes all or nothing and numbers each change; `changes_since` returns the events after a token. Apps
/// keep the store in process; the shell posts `system.contacts.change` when the contact file changes.
pub mod history {
    pub use bifrost_ui::contact_changes::{Change, Error, Event, Observers, Op, Store};
}

/// Contact display preferences (CNContactsUserDefaults and CNContactFormatter sort order): the sort order a list uses,
/// the country code a phone number is written with, and the sort helpers. `ContactsDefaults` keeps the user's choice;
/// `effective_sort_order` resolves "system default" against the system's order.
pub mod display {
    pub use bifrost_ui::contact_display::{
        compare_names, normalize_country_code, sort_contacts, sort_key, ContactsDefaults, SortOrder,
    };
}

/// The contact picker (ContactsUI, CNContactPicker and the people picker): the user chooses one whole contact, or one
/// value of a contact such as a phone number. The app gets only the final [`picker::Choice`], so it needs no read
/// access to the store. `Picker::new` takes the entries the shell shows, `select` picks or opens a contact, and
/// `choose` picks one value of the open contact. The shell draws the list from `visible` and the detail from `detail`.
pub mod picker {
    pub use bifrost_ui::contact_card::{Entry, Field, Key};
    pub use bifrost_ui::contact_picker::{Choice, Error, Mode, Picker};
}

/// vCard 3.0 and 4.0 for the full contact model (`bifrost_ui::contact_vcard`, the same code the Contacts app uses for
/// its import and export). `vcard::serialize` writes contacts as CRLF-terminated cards, and `vcard::contacts` reads
/// every card back, skipping properties the model has no field for. Text over `vcard::MAX_BYTES` is refused, and
/// `vcard::Error` names the failure. An app builds its own [`Contact`] values, so it never needs the store file.
pub mod vcard {
    pub use bifrost_ui::contact_book::{label, Contact, LabeledValue};
    pub use bifrost_ui::contact_vcard::{contacts, serialize, Version};
    pub use bifrost_ui::vcard::{Error, MAX_BYTES, MAX_CARDS};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_sdk_vcard_round_trips_a_contact_in_both_versions() {
        let mut ada = vcard::Contact {
            given_name: "Ada".into(),
            family_name: "Lovelace".into(),
            organization_name: "Analytical Engines".into(),
            ..vcard::Contact::default()
        };
        ada.phone_numbers.push(vcard::LabeledValue::new(Some(vcard::label::MOBILE), "+354 555 1234".into()));
        ada.email_addresses.push(vcard::LabeledValue::new(Some(vcard::label::WORK), "ada@example.org".into()));
        for version in [vcard::Version::V3, vcard::Version::V4] {
            let text = vcard::serialize(std::slice::from_ref(&ada), version);
            assert!(text.starts_with("BEGIN:VCARD\r\n"), "{version:?}");
            assert_eq!(vcard::contacts(&text).unwrap(), vec![ada.clone()], "{version:?}");
        }
    }

    #[test]
    fn groups_nest_and_list_members_recursively() {
        let mut groups = groups::Groups::new();
        let family = groups.add("Family", None).unwrap();
        let kin = groups.add("Kin", Some(family)).unwrap();
        assert!(groups.add_member(kin, 7).unwrap());
        assert!(groups.members(family, true).unwrap().contains(&7));
        assert!(!groups.members(family, false).unwrap().contains(&7));
        assert_eq!(groups.move_to(family, Some(kin)), Err(groups::Error::WouldCycle));
    }

    #[test]
    fn a_batch_of_contact_changes_is_recorded_and_fetched_by_token() {
        let mut store: history::Store<String> = history::Store::new(16);
        let ids = store.save(vec![history::Op::Add("ada".into())], |name| !name.is_empty()).unwrap();
        assert_eq!(ids.len(), 1);
        let token = store.token();
        assert_eq!(store.changes_since(0).unwrap().len(), 1);
        store.save(vec![history::Op::Delete(ids[0])], |_| true).unwrap();
        assert_eq!(store.changes_since(token).unwrap().len(), 1);
        assert_eq!(store.save(vec![history::Op::Add(String::new())], |name| !name.is_empty()), Err(history::Error::Invalid(0)));
    }

    #[test]
    fn display_preferences_sort_and_resolve_the_user_default() {
        let mut people = vec![("Zoe", "Adams"), ("ada", "Lovelace")];
        display::sort_contacts(&mut people, display::SortOrder::FamilyName, |p| (p.0, p.1));
        assert_eq!(people[0], ("Zoe", "Adams"));
        let defaults = display::ContactsDefaults::new();
        assert_eq!(defaults.effective_sort_order(display::SortOrder::UserDefault), display::SortOrder::GivenName);
        assert_eq!(display::normalize_country_code(" is "), Some("IS".to_string()));
        assert_eq!(display::normalize_country_code("Iceland"), None);
    }

    #[test]
    fn shell_messages_map_to_typed_errors() {
        assert_eq!(ContactsError::from_message("contacts write access is not granted"), ContactsError::NotGranted);
        assert_eq!(ContactsError::from_message("save needs add, update or delete"), ContactsError::NothingToSave);
        assert_eq!(ContactsError::from_message("update needs the id of a contact"), ContactsError::MissingId);
        assert_eq!(ContactsError::from_message("Contacts file too large"), ContactsError::FileTooLarge);
        assert_eq!(ContactsError::from_message("Unknown contacts format"), ContactsError::UnknownFormat);
        assert_eq!(ContactsError::from_message("Invalid contact"), ContactsError::InvalidContact(String::new()));
        assert!(matches!(ContactsError::from_message("phone is a list"), ContactsError::InvalidContact(_)));
        assert_eq!(ContactsError::from_message("disk on fire"), ContactsError::Other("disk on fire".into()));
    }

    #[test]
    fn a_picker_returns_only_the_chosen_contact_or_value() {
        let ada = picker::Entry {
            id: 3,
            name: "Ada".into(),
            fields: vec![picker::Field { key: picker::Key::Phone, label: "mobile".into(), value: "555 1234".into() }],
        };
        let mut by_contact = picker::Picker::new(vec![ada.clone()], picker::Mode::Contact { need: None });
        assert_eq!(by_contact.select(3), Ok(Some(picker::Choice::Contact(3))));
        assert_eq!(by_contact.select(9), Err(picker::Error::UnknownContact));

        let mut by_value = picker::Picker::new(vec![ada], picker::Mode::Property { keys: vec![picker::Key::Phone] });
        assert_eq!(by_value.select(3), Ok(None));
        assert_eq!(
            by_value.choose(picker::Key::Phone, 0),
            Ok(picker::Choice::Property { contact: 3, key: picker::Key::Phone, value: "555 1234".into() })
        );
        assert_eq!(by_value.choose(picker::Key::Phone, 0), Err(picker::Error::NoneOpen));
    }

    #[test]
    fn oversized_vcard_text_is_refused() {
        let big = "x".repeat(vcard::MAX_BYTES + 1);
        assert_eq!(vcard::contacts(&big), Err(vcard::Error::TooLarge));
    }

    fn reply(rows: Vec<Json>) -> Json {
        Json::Obj(vec![("contacts".into(), Json::Arr(rows))])
    }

    fn row(id: u64, given: Json, emails: Json) -> Json {
        Json::Obj(vec![("id".into(), Json::Num(id as f64)), ("given_name".into(), given), ("emails".into(), emails)])
    }

    #[test]
    fn a_status_reply_decodes_to_its_status() {
        let r = Json::Obj(vec![("status".into(), Json::Str("limited".into()))]);
        assert_eq!(decode_status(&r), Some(Status::Limited));
        assert_eq!(decode_status(&Json::Obj(vec![("status".into(), Json::Str("all".into()))])), None);
        assert_eq!(decode_status(&Json::Null), None);
    }

    #[test]
    fn the_picked_ids_decode_and_default_to_none() {
        let r = Json::Obj(vec![("picked".into(), Json::Arr(vec![Json::Num(4.0), Json::Num(9.0)]))]);
        assert_eq!(decode_picked(&r), [4u64, 9].into_iter().collect());
        assert!(decode_picked(&Json::Null).is_empty());
    }

    #[test]
    fn a_fetch_reply_decodes_to_records_with_the_fields_it_held() {
        let r = reply(vec![
            row(1, Json::Str("Ada".into()), Json::Arr(vec![Json::Str("ada@example.com".into())])),
            row(2, Json::Null, Json::Null),
        ]);
        let people = decode_contacts(&r).unwrap();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].id, 1);
        assert_eq!(people[0].get(Key::GivenName), Some(&Value::Text("Ada".into())));
        let emails = Value::Labeled(vec![LabeledValue { label: "".into(), value: "ada@example.com".into(), identifier: "1".into() }]);
        assert_eq!(people[0].get(Key::Emails), Some(&emails));
        assert_eq!(people[1].get(Key::GivenName), None);
    }

    #[test]
    fn a_malformed_record_fails_the_whole_reply() {
        let r = reply(vec![Json::Obj(vec![("given_name".into(), Json::Str("No id".into()))])]);
        assert_eq!(decode_contacts(&r), None);
        assert_eq!(decode_contacts(&Json::Null), None);
    }

    #[test]
    fn an_empty_book_decodes_to_no_records() {
        assert_eq!(decode_contacts(&reply(vec![])), Some(vec![]));
    }

    #[test]
    fn a_save_request_sends_add_and_delete_without_ids_and_update_with_them() {
        let mut ada = Record::new(0);
        ada.set(Key::GivenName, Value::Text("Ada".into()));
        ada.set(Key::Emails, Value::List(vec!["ada@example.com".into()]));
        let mut grace = Record::new(2);
        grace.set(Key::FamilyName, Value::Text("Hopper".into()));
        let mut gone = Record::new(3);
        gone.set(Key::GivenName, Value::Text("ignored".into()));
        let request = SaveRequest { add: vec![ada], update: vec![grace], delete: vec![gone] };
        let json = request.to_json();
        let add = json.get("add").and_then(Json::as_array).unwrap();
        assert_eq!(add[0].get("id"), None);
        assert_eq!(add[0].get("given_name").and_then(Json::as_str), Some("Ada"));
        let update = json.get("update").and_then(Json::as_array).unwrap();
        assert_eq!(update[0].get("id").and_then(Json::as_u64), Some(2));
        assert_eq!(update[0].get("family_name").and_then(Json::as_str), Some("Hopper"));
        let delete = json.get("delete").and_then(Json::as_array).unwrap();
        assert_eq!(delete, &[Json::Obj(vec![("id".into(), Json::Num(3.0))])]);
    }

    #[test]
    fn a_save_reply_decodes_to_the_added_ids() {
        let r = Json::Obj(vec![("added".into(), Json::Arr(vec![Json::Num(4.0), Json::Num(5.0)]))]);
        assert_eq!(decode_saved(&r), Some(vec![4, 5]));
        assert_eq!(decode_saved(&Json::Null), None);
    }

    #[test]
    fn a_preview_applies_the_same_rules_as_the_shell() {
        assert_eq!(preview(Choice::All), Ok(Status::Authorized));
        assert_eq!(preview(Choice::Picked(BTreeSet::new())), Err(Error::NothingPicked));
        assert_eq!(preview(Choice::Deny), Ok(Status::Denied));
    }

    fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn entry(label: &str, value: &str, id: &str) -> Json {
        obj(vec![("label", Json::Str(label.into())), ("value", Json::Str(value.into())), ("identifier", Json::Str(id.into()))])
    }

    #[test]
    fn labeled_arrays_birthday_and_dates_decode() {
        let row = obj(vec![
            ("id", Json::Num(3.0)),
            ("emails", Json::Arr(vec![entry("home", "a@x.is", "1"), entry("work", "b@x.is", "2")])),
            ("birthday", obj(vec![("month", Json::Num(12.0)), ("day", Json::Num(10.0))])),
            ("dates", Json::Arr(vec![obj(vec![("label", Json::Str("anniversary".into())), ("year", Json::Num(2001.0)), ("month", Json::Num(5.0)), ("day", Json::Num(6.0))])])),
        ]);
        let people = decode_contacts(&reply(vec![row])).unwrap();
        let Some(Value::Labeled(emails)) = people[0].get(Key::Emails) else { panic!("emails") };
        assert_eq!((emails.len(), emails[1].label.as_str(), emails[1].identifier.as_str()), (2, LABEL_WORK, "2"));
        assert_eq!(people[0].get(Key::Birthday), Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap())));
        assert_eq!(
            people[0].get(Key::Dates),
            Some(&Value::Dates(vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())]))
        );
    }

    #[test]
    fn an_old_single_string_reply_decodes_to_a_one_element_list() {
        let row = row(1, Json::Null, Json::Arr(vec![Json::Str("old@x.is".into())]));
        let people = decode_contacts(&reply(vec![row])).unwrap();
        let Some(Value::Labeled(emails)) = people[0].get(Key::Emails) else { panic!("emails") };
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].value, "old@x.is");
    }

    #[test]
    fn a_service_error_message_maps_to_a_typed_error() {
        assert_eq!(ContactsError::from_message("contacts access is not granted"), ContactsError::NotGranted);
        assert_eq!(
            ContactsError::from_message("a limited contacts grant only reads; saving needs full access"),
            ContactsError::ReadOnly
        );
        assert_eq!(ContactsError::from_message("fetch needs keys"), ContactsError::Other("fetch needs keys".into()));
    }

    #[test]
    fn a_failed_reply_decodes_to_its_typed_error() {
        assert_eq!(decode_error(&Err("contacts access is not granted".into())), Some(ContactsError::NotGranted));
        assert_eq!(decode_error(&Ok(Json::Null)), None);
    }

    #[test]
    fn a_bad_birthday_is_left_out() {
        let row = obj(vec![("id", Json::Num(1.0)), ("birthday", obj(vec![("month", Json::Num(13.0)), ("day", Json::Num(1.0))]))]);
        assert_eq!(decode_contacts(&reply(vec![row])).unwrap()[0].get(Key::Birthday), None);
    }
}
