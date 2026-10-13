//! The `contacts` service: the person's contacts, for the apps the person let read them. The access model is
//! `bifrost_ui::contact_access` (every contact, a picked set read-only, or none); the record shape is
//! `bifrost_ui::contact_fetch`, so a fetch returns only the fields it asked for.
//!
//! Methods (params are a JSON object):
//! - `access {}` returns `{status, picked}`: the status (`not_determined`, `restricted`, `denied`, `authorized` or
//!   `limited`) and the contact ids a limited grant holds (empty otherwise).
//! - `request {}` returns `{status}`, the current status. It changes nothing: an app cannot grant itself access. The
//!   person's answer is set by [`answer_prompt`] (the access sheet) or [`change_by_user`] (Settings).
//! - `fetch {keys, query?}` returns `{contacts: [{id, <key>: value | null, ...}]}`. `keys` names the fields
//!   (`given_name`, `family_name`, `organization`, `phones`, `emails`, `address`, `note`, `urls`, `birthday`, `dates`)
//!   and must not be empty. `phones`, `emails` and `urls` are lists of `{label, value, identifier}`, `birthday` is
//!   `{year?, month, day}` and `dates` is a list of `{label, year?, month, day}`. `query`
//!   filters by any text of the contact. Answers only the contacts the app may read, sorted by name. Without a
//!   read grant it is an error.
//!
//! The first phone and email come from the contacts file (labeled by its label row); more phones and emails, URLs, the
//! birthday and dates come from the sidecar (`contactextras`). A contact without a sidecar entry reads as before, with
//! one-element lists.
//!
//! The store holds one name per contact: the last word of it is the family name and the rest is the given name. The
//! grants live per shell thread (thread-local, like the `privacy` service), so they reset when the shell restarts.
use super::{Shell, SystemService};
use crate::shell::contactextras::{self, Book, Extras};
use crate::shell::contactstore::{self, Contact, LABELS};
use bifrost_core::json::Json;
use bifrost_ui::contact_access::{Access, Choice, Error, Status};
use bifrost_ui::contact_fetch::{project, DateComponents, Key, Keys, LabeledValue, Partial, Record, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static ACCESS: RefCell<BTreeMap<String, Access>> = RefCell::new(BTreeMap::new());
}

/// The grant an app holds (a new app has none answered yet).
pub fn access_of(app_id: &str) -> Access {
    ACCESS.with(|a| a.borrow().get(app_id).cloned().unwrap_or_default())
}

/// The answer to the access prompt the app is waiting on. The prompt is shown once.
pub fn answer_prompt(app_id: &str, choice: Choice) -> Result<Status, Error> {
    ACCESS.with(|a| a.borrow_mut().entry(app_id.to_string()).or_default().request(choice))
}

/// A change the person makes in Settings for one app. Only a system restriction blocks it.
pub fn change_by_user(app_id: &str, choice: Choice) -> Result<Status, Error> {
    ACCESS.with(|a| a.borrow_mut().entry(app_id.to_string()).or_default().change_in_settings(choice))
}

/// Whether the app holds any read grant, so the shell may load the book for it.
fn reads(app_id: &str) -> bool {
    matches!(access_of(app_id).status(), Status::Authorized | Status::Limited)
}

/// The record a store contact is seen as: the last word of its name is the family name.
pub fn record_of(c: &Contact) -> Record {
    record_with(c, &Extras::default())
}

/// The record of a store contact together with its extra multi-value fields. The file's phone and email come first.
pub fn record_with(c: &Contact, extras: &Extras) -> Record {
    let mut r = Record::new(c.id);
    let (given, family) = split_name(&c.name);
    r.set(Key::GivenName, Value::Text(given));
    r.set(Key::FamilyName, Value::Text(family));
    r.set(Key::Organization, Value::Text(c.company.clone()));
    let first = |value: &str, set: usize, more: &[LabeledValue]| {
        let label = LABELS[set].get(c.labels[set] as usize).copied().unwrap_or("other");
        let mut items = vec![LabeledValue::new(label, value)];
        items.extend(more.iter().cloned());
        LabeledValue::numbered(items.into_iter().filter(|i| !i.value.trim().is_empty()).collect())
    };
    r.set(Key::Phones, Value::Labeled(first(&c.phone, 0, &extras.phones)));
    r.set(Key::Emails, Value::Labeled(first(&c.email, 1, &extras.emails)));
    r.set(Key::Urls, Value::Labeled(LabeledValue::numbered(extras.urls.clone())));
    if let Some(b) = extras.birthday {
        r.set(Key::Birthday, Value::Date(b));
    }
    r.set(Key::Dates, Value::Dates(extras.dates.clone()));
    r.set(Key::Address, Value::Text(c.address.clone()));
    r.set(Key::Note, Value::Text(c.notes.clone()));
    r
}

fn split_name(name: &str) -> (String, String) {
    let name = name.trim();
    match name.rsplit_once(' ') {
        Some((given, family)) => (given.trim().to_string(), family.to_string()),
        None => (name.to_string(), String::new()),
    }
}

/// The JSON of one partial record: its id and each fetched key, `null` when the contact has no value for it.
pub fn partial_json(p: &Partial) -> Json {
    let mut fields = vec![("id".to_string(), Json::Num(p.id as f64))];
    for key in p.fetched() {
        let value = match p.get(key) {
            Ok(Some(Value::Text(text))) => Json::Str(text.clone()),
            Ok(Some(Value::List(items))) => Json::Arr(items.iter().map(|item| Json::Str(item.clone())).collect()),
            Ok(Some(Value::Labeled(items))) => Json::Arr(items.iter().map(labeled_json).collect()),
            Ok(Some(Value::Date(d))) => date_fields(*d, Vec::new()),
            Ok(Some(Value::Dates(items))) => {
                Json::Arr(items.iter().map(|(label, d)| date_fields(*d, vec![("label".to_string(), Json::Str(label.clone()))])).collect())
            }
            _ => Json::Null,
        };
        fields.push((key.name().to_string(), value));
    }
    Json::Obj(fields)
}

fn labeled_json(item: &LabeledValue) -> Json {
    Json::Obj(vec![
        ("label".to_string(), Json::Str(item.label.clone())),
        ("value".to_string(), Json::Str(item.value.clone())),
        ("identifier".to_string(), Json::Str(item.identifier.clone())),
    ])
}

fn date_fields(d: DateComponents, mut fields: Vec<(String, Json)>) -> Json {
    if let Some(y) = d.year {
        fields.push(("year".to_string(), Json::Num(y as f64)));
    }
    fields.push(("month".to_string(), Json::Num(d.month as f64)));
    fields.push(("day".to_string(), Json::Num(d.day as f64)));
    Json::Obj(fields)
}

/// The keys named in a `fetch` call's params.
fn keys_of(params: &Json) -> Result<Keys, String> {
    let names = params.get("keys").and_then(Json::as_array).ok_or("keys is required (a list of field names)")?;
    let mut keys = Vec::new();
    for name in names {
        let name = name.as_str().ok_or("a key is a text name")?;
        keys.push(Key::from_name(name).ok_or_else(|| format!("no contact field {name}"))?);
    }
    Keys::new(&keys).map_err(|e| format!("{e:?}"))
}

/// The answer to one method call by `app_id`. `contacts` is the store's book, loaded by the caller only for a
/// `fetch` by an app that holds a read grant. Pure: no file access here.
pub fn answer(app_id: &str, method: &str, params: &Json, contacts: &[Contact]) -> Result<Json, String> {
    answer_with(app_id, method, params, contacts, &Book::new())
}

/// [`answer`] with the sidecar of extra multi-value fields, by contact id.
pub fn answer_with(app_id: &str, method: &str, params: &Json, contacts: &[Contact], extras: &Book) -> Result<Json, String> {
    let access = access_of(app_id);
    match method {
        "access" => {
            let picked: Vec<Json> = access.picked().iter().map(|id| Json::Num(*id as f64)).collect();
            Ok(Json::Obj(vec![
                ("status".into(), Json::Str(access.status().name().into())),
                ("picked".into(), Json::Arr(picked)),
            ]))
        }
        "request" => Ok(Json::Obj(vec![("status".into(), Json::Str(access.status().name().into()))])),
        "fetch" => {
            if !reads(app_id) {
                return Err("contacts access is not granted".into());
            }
            let keys = keys_of(params)?;
            let query = params.get("query").and_then(Json::as_str).unwrap_or("");
            let mut pool: Vec<Contact> = contacts.iter().filter(|c| access.can_read(c.id)).cloned().collect();
            contactstore::sort(&mut pool);
            let hits = contactstore::search(&pool, query);
            let rows: Vec<Json> = hits
                .into_iter()
                .map(|i| partial_json(&project(&record_with(&pool[i], &extras.get(&pool[i].id).cloned().unwrap_or_default()), &keys)))
                .collect();
            Ok(Json::Obj(vec![("contacts".into(), Json::Arr(rows))]))
        }
        _ => Err(format!("contacts has no method {method}")),
    }
}

pub struct Contacts;

impl SystemService for Contacts {
    fn name(&self) -> &'static str {
        "contacts"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let path = contactstore::store_path(&shell.data_dir);
        let (book, extras) = if method == "fetch" && reads(app_id) {
            (contactstore::load_checked(&path).map_err(|e| e.to_string())?, contactextras::load(&contactextras::path_for(&path)))
        } else {
            (Vec::new(), Book::new())
        };
        answer_with(app_id, method, params, &book, &extras)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: u64, name: &str, phone: &str, email: &str, notes: &str) -> Contact {
        Contact { id, name: name.into(), phone: phone.into(), email: email.into(), notes: notes.into(), ..Default::default() }
    }

    fn book() -> Vec<Contact> {
        vec![
            contact(1, "Ada Lovelace", "+354 555 0100", "ada@example.com", "Analyst"),
            contact(2, "Grace Hopper", "", "grace@example.com", ""),
            contact(3, "Alan Turing", "+354 555 0300", "", ""),
        ]
    }

    fn keys(names: &[&str]) -> Json {
        Json::Obj(vec![("keys".into(), Json::Arr(names.iter().map(|n| Json::Str((*n).into())).collect()))])
    }

    fn with_query(names: &[&str], query: &str) -> Json {
        let mut params = keys(names);
        if let Json::Obj(fields) = &mut params {
            fields.push(("query".into(), Json::Str(query.into())));
        }
        params
    }

    #[test]
    fn a_new_app_has_no_access_and_reads_nothing() {
        let id = "is.example.contacts-new";
        let access = answer(id, "access", &Json::Null, &book()).unwrap();
        assert_eq!(access.get("status").and_then(Json::as_str), Some("not_determined"));
        assert!(answer(id, "fetch", &keys(&["given_name"]), &book()).is_err());
    }

    #[test]
    fn a_request_reports_the_status_and_cannot_grant_itself_access() {
        let id = "is.example.contacts-request";
        let reply = answer(id, "request", &Json::Null, &book()).unwrap();
        assert_eq!(reply.get("status").and_then(Json::as_str), Some("not_determined"));
        assert_eq!(access_of(id).status(), Status::NotDetermined);
    }

    #[test]
    fn full_access_fetches_only_the_named_fields_sorted_by_name() {
        let id = "is.example.contacts-all";
        assert_eq!(answer_prompt(id, Choice::All), Ok(Status::Authorized));
        let reply = answer(id, "fetch", &keys(&["family_name", "emails"]), &book()).unwrap();
        let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
        let families: Vec<_> = rows.iter().filter_map(|r| r.get("family_name").and_then(Json::as_str)).collect();
        assert_eq!(families, vec!["Lovelace", "Turing", "Hopper"]);
        assert!(rows[0].get("given_name").is_none());
        assert_eq!(rows[1].get("emails"), Some(&Json::Null));
        assert_eq!(rows[2].get("emails").and_then(Json::as_array).map(|a| a.len()), Some(1));
    }

    #[test]
    fn a_blank_value_comes_back_as_null() {
        let id = "is.example.contacts-null";
        answer_prompt(id, Choice::All).unwrap();
        let reply = answer(id, "fetch", &keys(&["note"]), &book()).unwrap();
        let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
        assert_eq!(rows[1].get("note"), Some(&Json::Null));
        assert_eq!(rows[2].get("note"), Some(&Json::Null));
        assert_eq!(rows[0].get("note").and_then(Json::as_str), Some("Analyst"));
    }

    #[test]
    fn a_picked_grant_reads_only_the_picked_contacts() {
        let id = "is.example.contacts-picked";
        answer_prompt(id, Choice::Picked([1u64, 3].into_iter().collect())).unwrap();
        let reply = answer(id, "fetch", &keys(&["family_name"]), &book()).unwrap();
        let ids: Vec<_> = reply.get("contacts").and_then(Json::as_array).unwrap().iter().filter_map(|r| r.get("id").and_then(Json::as_u64)).collect();
        assert_eq!(ids, vec![1, 3]);
        let picked = answer(id, "access", &Json::Null, &book()).unwrap();
        assert_eq!(picked.get("picked"), Some(&Json::Arr(vec![Json::Num(1.0), Json::Num(3.0)])));
    }

    #[test]
    fn a_query_filters_the_readable_contacts_by_any_text() {
        let id = "is.example.contacts-query";
        answer_prompt(id, Choice::Picked([1u64, 2].into_iter().collect())).unwrap();
        let reply = answer(id, "fetch", &with_query(&["given_name"], "example.com"), &book()).unwrap();
        assert_eq!(reply.get("contacts").and_then(Json::as_array).map(|a| a.len()), Some(2));
        let none = answer(id, "fetch", &with_query(&["given_name"], "turing"), &book()).unwrap();
        assert_eq!(none.get("contacts").and_then(Json::as_array).map(|a| a.len()), Some(0));
    }

    #[test]
    fn a_refused_app_cannot_read_and_a_settings_change_can_grant_it_later() {
        let id = "is.example.contacts-refused";
        answer_prompt(id, Choice::Deny).unwrap();
        assert!(answer(id, "fetch", &keys(&["given_name"]), &book()).is_err());
        assert_eq!(change_by_user(id, Choice::All), Ok(Status::Authorized));
        assert!(answer(id, "fetch", &keys(&["given_name"]), &book()).is_ok());
    }

    #[test]
    fn keys_are_required_and_must_be_known() {
        let id = "is.example.contacts-keys";
        answer_prompt(id, Choice::All).unwrap();
        assert!(answer(id, "fetch", &Json::Null, &book()).is_err());
        assert!(answer(id, "fetch", &keys(&[]), &book()).is_err());
        assert!(answer(id, "fetch", &keys(&["nickname"]), &book()).is_err());
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("x", "erase", &Json::Null, &book()).is_err());
    }

    #[test]
    fn a_name_splits_into_given_and_family_on_the_last_space() {
        let r = record_of(&contact(9, "Mary Ann Smith", "", "", ""));
        assert_eq!(r.get(Key::GivenName), Some(&Value::Text("Mary Ann".into())));
        assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("Smith".into())));
        let one = record_of(&contact(10, "Cher", "", "", ""));
        assert_eq!(one.get(Key::GivenName), Some(&Value::Text("Cher".into())));
        assert_eq!(one.get(Key::FamilyName), None);
    }

    fn extras_for_ada() -> Book {
        let mut book = Book::new();
        book.insert(
            1,
            Extras {
                phones: vec![LabeledValue::new("work", "+354 555 0199")],
                emails: vec![LabeledValue::new("work", "ada@work.example")],
                urls: vec![LabeledValue::new("home", "https://ada.example")],
                birthday: DateComponents::new(Some(1815), 12, 10),
                dates: vec![("anniversary".into(), DateComponents::new(None, 7, 8).unwrap())],
            },
        );
        book
    }

    #[test]
    fn two_emails_and_a_birthday_are_returned_with_labels_and_identifiers() {
        let id = "is.example.contacts-multi";
        answer_prompt(id, Choice::All).unwrap();
        let reply = answer_with(id, "fetch", &keys(&["emails", "birthday", "phones", "urls", "dates"]), &book(), &extras_for_ada()).unwrap();
        let ada = &reply.get("contacts").and_then(Json::as_array).unwrap()[0];
        let emails = ada.get("emails").and_then(Json::as_array).unwrap();
        assert_eq!(emails.len(), 2);
        assert_eq!(emails[0].get("label").and_then(Json::as_str), Some("home"));
        assert_eq!(emails[0].get("value").and_then(Json::as_str), Some("ada@example.com"));
        assert_eq!(emails[1].get("label").and_then(Json::as_str), Some("work"));
        assert_eq!(emails[1].get("identifier").and_then(Json::as_str), Some("2"));
        assert_eq!(ada.get("phones").and_then(Json::as_array).map(|a| a.len()), Some(2));
        assert_eq!(ada.get("urls").and_then(Json::as_array).map(|a| a.len()), Some(1));
        let b = ada.get("birthday").unwrap();
        assert_eq!((b.get("year").and_then(Json::as_u64), b.get("month").and_then(Json::as_u64), b.get("day").and_then(Json::as_u64)), (Some(1815), Some(12), Some(10)));
        let dates = ada.get("dates").and_then(Json::as_array).unwrap();
        assert_eq!(dates[0].get("label").and_then(Json::as_str), Some("anniversary"));
        assert!(dates[0].get("year").is_none());
    }

    #[test]
    fn keys_left_out_are_not_returned_and_a_contact_without_extras_reads_as_before() {
        let id = "is.example.contacts-multi-keys";
        answer_prompt(id, Choice::All).unwrap();
        let reply = answer_with(id, "fetch", &keys(&["emails", "birthday"]), &book(), &extras_for_ada()).unwrap();
        let rows = reply.get("contacts").and_then(Json::as_array).unwrap();
        assert!(rows[0].get("phones").is_none());
        let grace = rows.iter().find(|r| r.get("id").and_then(Json::as_u64) == Some(2)).unwrap();
        assert_eq!(grace.get("emails").and_then(Json::as_array).map(|a| a.len()), Some(1));
        assert_eq!(grace.get("birthday"), Some(&Json::Null));
    }
}
