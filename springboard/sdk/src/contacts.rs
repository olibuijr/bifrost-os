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
pub use crate::controls::contact_fetch::{Key, Record, Value};
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
            None => continue,
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
        match v.get(key.name()) {
            Some(Json::Str(text)) => record.set(key, Value::Text(text.clone())),
            Some(Json::Arr(items)) => {
                record.set(key, Value::List(items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()))
            }
            _ => {}
        }
    }
    Some(record)
}

/// The access an app would hold after a choice, for a test or a preview of the sheet. The shell applies the same rules.
pub fn preview(choice: Choice) -> Result<Status, Error> {
    Access::new().request(choice)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(people[0].get(Key::Emails), Some(&Value::List(vec!["ada@example.com".into()])));
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
}
