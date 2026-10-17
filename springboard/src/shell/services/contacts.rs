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
//! - `save {add?, update?, delete?}` changes the book and returns `{added: [id, ...]}`, the ids the new contacts got.
//!   `add` and `update` hold records of the fields above (`update` also needs the contact `id` and changes only the
//!   fields it names; `null` clears one). `delete` holds `{id}` objects. Only full access writes: a limited grant is read-only. The whole
//!   call applies or none of it does, and the shell stores the book only when it succeeds.
//!
//! The first phone and email come from the contacts file (labeled by its label row); more phones and emails, URLs, the
//! birthday and dates come from the sidecar (`contactextras`). A contact without a sidecar entry reads as before, with
//! one-element lists.
//!
//! The store holds one name per contact: the last word of it is the family name and the rest is the given name. The
//! grants live per shell thread (thread-local, like the `privacy` service), so they reset when the shell restarts.
use super::{Shell, SystemService};
use crate::shell::contactextras::{self, Book, Extras};
use crate::shell::contactchanges;
use crate::shell::contactgroups;
use crate::shell::contactpick;
use crate::shell::contactphotos;
use crate::shell::contactstore::{self, Contact, LABELS};
use bifrost_core::json::Json;
use bifrost_ui::contact_groups::Groups;
use bifrost_ui::contact_vcard;
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
pub fn reads(app_id: &str) -> bool {
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

/// A record from a `save` call: each name must be a known field (or `id`), holding text or a list of text. `null`
/// clears the field. The `id` is 0 when the call names none.
fn record_of_json(v: &Json) -> Result<(Record, Vec<Key>), String> {
    let Json::Obj(fields) = v else {
        return Err("a contact is an object of fields".into());
    };
    let (mut record, mut cleared) = (Record::new(0), Vec::new());
    for (name, value) in fields {
        if name.as_str() == "id" {
            record.id = value.as_u64().ok_or_else(|| "id is a number".to_string())?;
            continue;
        }
        let key = Key::from_name(name).ok_or_else(|| format!("no contact field {name}"))?;
        let value = match value {
            Json::Null => {
                // a record drops blank values, so a cleared field is kept aside
                cleared.push(key);
                continue;
            }
            Json::Str(text) => Value::Text(text.clone()),
            Json::Arr(items) => Value::List(
                items
                    .iter()
                    .map(|item| item.as_str().map(str::to_string).ok_or_else(|| format!("{name} is a list of text")))
                    .collect::<Result<Vec<String>, String>>()?,
            ),
            _ => return Err(format!("{name} is text or a list of text")),
        };
        record.set(key, value);
    }
    Ok((record, cleared))
}

/// The one value a record holds for a key, or blank. The book keeps one phone, one email and one address per contact,
/// so a list of more than one is refused rather than cut down.
fn value_of(record: &Record, key: Key) -> Result<String, String> {
    match record.get(key) {
        None => Ok(String::new()),
        Some(Value::Text(text)) => Ok(text.trim().to_string()),
        Some(Value::List(items)) => match items.as_slice() {
            [] => Ok(String::new()),
            [one] => Ok(one.trim().to_string()),
            _ => Err(format!("{} takes one value per contact", key.name())),
        },
        Some(Value::Labeled(items)) => match items.as_slice() {
            [] => Ok(String::new()),
            [one] => Ok(one.value.trim().to_string()),
            _ => Err(format!("{} takes one value per contact", key.name())),
        },
        // birthday and dates live in the contacts sidecar, which `save` does not write yet
        Some(Value::Date(_) | Value::Dates(_)) => Err(format!("{} cannot be saved yet", key.name())),
    }
}

/// The contact a record describes. An update (`base`) changes only the fields the record names, as an update in a
/// save request writes only the keys it sets, and keeps the flags and label rows. The book stores one name: its last
/// word is the family name and the rest the given name.
fn contact_of(id: u64, (record, cleared): &(Record, Vec<Key>), base: Option<&Contact>) -> Result<Contact, String> {
    let field = |key: Key, kept: &str| -> Result<String, String> {
        if cleared.contains(&key) {
            Ok(String::new())
        } else if record.get(key).is_some() {
            value_of(record, key)
        } else {
            Ok(kept.to_string())
        }
    };
    let (kept_given, kept_family) = base.map_or(("", ""), |c| c.name.rsplit_once(' ').unwrap_or(("", c.name.as_str())));
    let given = field(Key::GivenName, kept_given)?;
    let family = field(Key::FamilyName, kept_family)?;
    let contact = Contact {
        id,
        name: [given, family].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" "),
        company: field(Key::Organization, base.map_or("", |c| &c.company))?,
        phone: field(Key::Phones, base.map_or("", |c| &c.phone))?,
        email: field(Key::Emails, base.map_or("", |c| &c.email))?,
        address: field(Key::Address, base.map_or("", |c| &c.address))?,
        notes: field(Key::Note, base.map_or("", |c| &c.notes))?,
        favorite: base.is_some_and(|c| c.favorite),
        labels: base.map_or([0; 3], |c| c.labels),
    };
    if contact.valid() {
        Ok(contact)
    } else {
        Err("a contact needs a name, company, address, phone or email, within the field limits".into())
    }
}

/// The list under `key` in a `save` call, empty when the call omits it.
fn list_of<'a>(params: &'a Json, key: &str) -> Result<&'a [Json], String> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::Arr(items)) => Ok(items.as_slice()),
        Some(_) => Err(format!("{key} is a list")),
    }
}

/// Applies a `save` call to `book` for `app_id` and returns the reply and the new book. Pure: the caller stores the new
/// book only when this is `Ok`, so a call that fails in any item changes nothing. Only full access writes. An add gets
/// the next free id; an update and a delete must name a contact the book holds.
pub fn apply_save(app_id: &str, params: &Json, book: &[Contact]) -> Result<(Json, Vec<Contact>), String> {
    access_of(app_id).check_write().map_err(|e| match e {
        Error::ReadOnly => "a limited contacts grant only reads; saving needs full access".to_string(),
        _ => "contacts write access is not granted".to_string(),
    })?;
    let adds = list_of(params, "add")?;
    let updates = list_of(params, "update")?;
    let deletes = list_of(params, "delete")?;
    if adds.is_empty() && updates.is_empty() && deletes.is_empty() {
        return Err("save needs add, update or delete".into());
    }
    let mut next = book.to_vec();
    let mut added = Vec::new();
    for item in adds {
        let id = contactstore::new_id(&next).ok_or_else(|| "the book is full".to_string())?;
        next.push(contact_of(id, &record_of_json(item)?, None)?);
        added.push(Json::Num(id as f64));
    }
    for item in updates {
        let parsed = record_of_json(item)?;
        let record = &parsed.0;
        if record.id == 0 {
            return Err("update needs the id of a contact".into());
        }
        let at = next.iter().position(|c| c.id == record.id).ok_or_else(|| format!("no contact {}", record.id))?;
        let updated = contact_of(record.id, &parsed, Some(&next[at]))?;
        next[at] = updated;
    }
    for item in deletes {
        let id = item.get("id").and_then(Json::as_u64).ok_or_else(|| "delete needs the id of a contact".to_string())?;
        let at = next.iter().position(|c| c.id == id).ok_or_else(|| format!("no contact {id}"))?;
        next.remove(at);
    }
    Ok((Json::Obj(vec![("added".into(), Json::Arr(added))]), next))
}

/// The answer to one method call by `app_id`. `contacts` is the store's book, loaded by the caller only for a
/// `fetch` by an app that holds a read grant. Pure: no file access here. A `save` is answered by [`apply_save`]
/// through the service, which stores the book.
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
        "save" => Err("save changes the book, so the service stores it: call it through the shell".into()),
        _ => Err(format!("contacts has no method {method}")),
    }
}

/// The group methods. Reads need a read grant, writes need full access, and a group holds only contacts the app can
/// see. Returns the reply and whether the groups changed, so the caller saves the sidecar only then.
pub fn groups_answer(
    app_id: &str,
    method: &str,
    params: &Json,
    contacts: &[Contact],
    groups: &mut Groups,
) -> Result<(Json, bool), String> {
    let access = access_of(app_id);
    let number = |key: &str| {
        params.get(key).and_then(Json::as_u64).ok_or_else(|| format!("{method} needs {key}"))
    };
    let writable = || access.check_write().map_err(|_| "contacts write access is not granted".to_string());
    let no_grant = || "contacts access is not granted".to_string();
    match method {
        "groups" => {
            if !reads(app_id) {
                return Err(no_grant());
            }
            let rows: Vec<Json> = groups
                .all()
                .map(|g| {
                    Json::Obj(vec![
                        ("id".into(), Json::Num(g.id as f64)),
                        ("name".into(), Json::Str(g.name.clone())),
                        ("parent".into(), g.parent.map(|p| Json::Num(p as f64)).unwrap_or(Json::Null)),
                    ])
                })
                .collect();
            Ok((Json::Obj(vec![("groups".into(), Json::Arr(rows))]), false))
        }
        "group_members" => {
            if !reads(app_id) {
                return Err(no_grant());
            }
            let recursive = matches!(params.get("recursive"), Some(Json::Bool(true)));
            let members = groups.members(number("group")?, recursive).map_err(|e| format!("{e:?}"))?;
            let visible: Vec<Json> =
                members.into_iter().filter(|m| access.can_read(*m)).map(|m| Json::Num(m as f64)).collect();
            Ok((Json::Obj(vec![("contacts".into(), Json::Arr(visible))]), false))
        }
        "group_add" => {
            writable()?;
            let name = params.get("name").and_then(Json::as_str).ok_or_else(|| "group_add needs name".to_string())?;
            let parent = match params.get("parent") {
                None | Some(Json::Null) => None,
                Some(_) => Some(number("parent")?),
            };
            let id = groups.add(name, parent).map_err(|e| format!("{e:?}"))?;
            Ok((Json::Obj(vec![("id".into(), Json::Num(id as f64))]), true))
        }
        "group_remove" => {
            writable()?;
            let removed = groups.remove(number("group")?).map_err(|e| format!("{e:?}"))?;
            Ok((Json::Obj(vec![("removed".into(), Json::Num(removed.len() as f64))]), true))
        }
        "group_add_member" | "group_remove_member" => {
            writable()?;
            let (group, contact) = (number("group")?, number("contact")?);
            if !contacts.iter().any(|c| c.id == contact) || !access.can_read(contact) {
                return Err("no such contact".into());
            }
            let changed = if method == "group_add_member" {
                groups.add_member(group, contact)
            } else {
                groups.remove_member(group, contact)
            }
            .map_err(|e| format!("{e:?}"))?;
            Ok((Json::Obj(vec![("changed".into(), Json::Bool(changed))]), changed))
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
        let groups_path = contactgroups::path_for(&path);
        let load = || contactstore::load_checked(&path).map_err(|e| e.to_string());
        match method {
            "save" => {
                let book = if access_of(app_id).check_write().is_ok() { load()? } else { Vec::new() };
                let (reply, next) = apply_save(app_id, params, &book)?;
                contactstore::save(&path, &next).map_err(|e| e.to_string())?;
                let changes = contactchanges::diff(&book, &next);
                if !changes.is_empty() {
                    let log_path = contactchanges::path_for(&path);
                    let mut log = contactchanges::load(&log_path);
                    log.append(&changes);
                    contactchanges::save(&log_path, &log).map_err(|e| e.to_string())?;
                    // A deleted contact takes its picture with it.
                    let photos = contactphotos::dir_for(&path);
                    for (kind, id) in &changes {
                        if *kind == contactchanges::Kind::Deleted {
                            let _ = contactphotos::remove(&photos, *id);
                        }
                    }
                }
                // A deleted contact leaves every group it was in.
                let mut groups = contactgroups::load(&groups_path);
                let stale: Vec<u64> = groups
                    .all()
                    .flat_map(|g| groups.members(g.id, false).unwrap_or_default())
                    .filter(|m| !next.iter().any(|c| c.id == *m))
                    .collect();
                if !stale.is_empty() {
                    for m in stale {
                        groups.forget_contact(m);
                    }
                    contactgroups::save(&groups_path, &groups).map_err(|e| e.to_string())?;
                }
                Ok(reply)
            }
            "fetch" if reads(app_id) => {
                let extras = contactextras::load(&contactextras::path_for(&path));
                answer_with(app_id, method, params, &load()?, &extras)
            }
            "pick" => {
                // A pick shows the whole book to the person, who chooses; the app itself reads nothing from it.
                let need = params.get("need").and_then(Json::as_str).and_then(contactpick::card_key);
                let keys = params
                    .get("keys")
                    .and_then(Json::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|k| k.as_str().and_then(contactpick::card_key))
                    .collect();
                let mode_name = params.get("mode").and_then(Json::as_str).unwrap_or("");
                let mode = contactpick::mode_of(mode_name, need, keys)?;
                let entries = contactpick::entries_of(&load()?);
                contactpick::start(app_id, bifrost_ui::contact_picker::Picker::new(entries, mode));
                Ok(Json::Obj(vec![("status".into(), Json::Str("pending".into()))]))
            }
            "pick_result" => match contactpick::outcome(app_id) {
                contactpick::Outcome::None => Err("no contact pick is open for this app".into()),
                contactpick::Outcome::Pending => Ok(Json::Obj(vec![("status".into(), Json::Str("pending".into()))])),
                contactpick::Outcome::Chosen(bifrost_ui::contact_picker::Choice::Contact(id)) => Ok(Json::Obj(vec![
                    ("status".into(), Json::Str("chosen".into())),
                    ("contact".into(), Json::Num(id as f64)),
                ])),
                contactpick::Outcome::Chosen(bifrost_ui::contact_picker::Choice::Property { contact, key: _, value }) => {
                    Ok(Json::Obj(vec![
                        ("status".into(), Json::Str("chosen".into())),
                        ("contact".into(), Json::Num(contact as f64)),
                        ("value".into(), Json::Str(value)),
                    ]))
                }
            },
            "photo" => {
                if !reads(app_id) {
                    return Err("contacts access is not granted".into());
                }
                let id = params.get("id").and_then(Json::as_u64).ok_or_else(|| "photo needs id".to_string())?;
                if !access_of(app_id).can_read(id) {
                    return Err("no such contact".into());
                }
                match contactphotos::read(&contactphotos::dir_for(&path), id) {
                    Some((format, bytes)) => Ok(Json::Obj(vec![
                        ("format".into(), Json::Str(contactphotos::format_name(format).into())),
                        ("data".into(), Json::Str(contact_vcard::base64_encode(&bytes))),
                    ])),
                    None => Ok(Json::Obj(vec![("format".into(), Json::Null)])),
                }
            }
            "photo_set" | "photo_remove" => {
                if access_of(app_id).check_write().is_err() {
                    return Err("contacts write access is not granted".into());
                }
                let id = params.get("id").and_then(Json::as_u64).ok_or_else(|| "photo needs id".to_string())?;
                if !load()?.iter().any(|c| c.id == id) {
                    return Err("no such contact".into());
                }
                let dir = contactphotos::dir_for(&path);
                if method == "photo_remove" {
                    let removed = contactphotos::remove(&dir, id).map_err(|e| e.to_string())?;
                    return Ok(Json::Obj(vec![("removed".into(), Json::Bool(removed))]));
                }
                let encoded = params.get("data").and_then(Json::as_str).ok_or_else(|| "photo_set needs data".to_string())?;
                let bytes = contact_vcard::base64_decode(encoded).ok_or_else(|| "photo data is not base64".to_string())?;
                let format = contactphotos::write(&dir, id, &bytes)?;
                Ok(Json::Obj(vec![("format".into(), Json::Str(contactphotos::format_name(format).into()))]))
            }
            "changes" => {
                if !reads(app_id) {
                    return Err("contacts access is not granted".into());
                }
                let token = params.get("token").and_then(Json::as_u64).unwrap_or(0);
                let log = contactchanges::load(&contactchanges::path_for(&path));
                let access = access_of(app_id);
                let rows: Vec<Json> = log
                    .since(token)?
                    .into_iter()
                    .filter(|e| access.can_read(e.id))
                    .map(|e| {
                        Json::Obj(vec![
                            ("seq".into(), Json::Num(e.seq as f64)),
                            ("kind".into(), Json::Str(e.kind.name().into())),
                            ("id".into(), Json::Num(e.id as f64)),
                        ])
                    })
                    .collect();
                Ok(Json::Obj(vec![
                    ("token".into(), Json::Num(log.last_seq as f64)),
                    ("changes".into(), Json::Arr(rows)),
                ]))
            }
            "groups" | "group_members" | "group_add" | "group_remove" | "group_add_member" | "group_remove_member" => {
                let mut groups: Groups = contactgroups::load(&groups_path);
                let contacts = if access_of(app_id).check_write().is_ok() || reads(app_id) { load()? } else { Vec::new() };
                let (reply, changed) = groups_answer(app_id, method, params, &contacts, &mut groups)?;
                if changed {
                    contactgroups::save(&groups_path, &groups).map_err(|e| e.to_string())?;
                }
                Ok(reply)
            }
            _ => answer_with(app_id, method, params, &[], &Book::new()),
        }
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

    fn obj(parts: &[(&str, Json)]) -> Json {
        Json::Obj(parts.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
    }

    fn text(value: &str) -> Json {
        Json::Str(value.into())
    }

    fn texts(values: &[&str]) -> Json {
        Json::Arr(values.iter().map(|v| Json::Str((*v).into())).collect())
    }

    fn ids_of(book: &[Contact]) -> Vec<u64> {
        book.iter().map(|c| c.id).collect()
    }

    #[test]
    fn groups_hold_only_known_contacts_and_report_their_members() {
        let id = "is.example.contacts-groups-full";
        answer_prompt(id, Choice::All).unwrap();
        let mut groups = Groups::new();
        let book = book();
        let (reply, changed) = groups_answer(id, "group_add", &obj(&[("name", text("Family"))]), &book, &mut groups).unwrap();
        assert!(changed);
        let family = Json::Num(reply.get("id").and_then(Json::as_u64).unwrap() as f64);
        let add = obj(&[("group", family.clone()), ("contact", Json::Num(1.0))]);
        assert!(groups_answer(id, "group_add_member", &add, &book, &mut groups).unwrap().1);
        let (listed, _) = groups_answer(id, "group_members", &obj(&[("group", family.clone())]), &book, &mut groups).unwrap();
        assert_eq!(listed.get("contacts").and_then(Json::as_array).map(|a| a.len()), Some(1));
        let unknown = obj(&[("group", family), ("contact", Json::Num(99.0))]);
        assert!(groups_answer(id, "group_add_member", &unknown, &book, &mut groups).is_err());
    }

    #[test]
    fn group_writes_are_refused_without_full_access() {
        let id = "is.example.contacts-groups-undecided";
        let mut groups = Groups::new();
        assert!(groups_answer(id, "group_add", &obj(&[("name", text("Family"))]), &book(), &mut groups).is_err());
        assert!(groups.is_empty());
    }

    #[test]
    fn saving_needs_full_access() {
        let add = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("emails", texts(&["a@x"]))])]))]);
        let undecided = "is.example.contacts-save-undecided";
        assert!(apply_save(undecided, &add, &book()).is_err());
        let refused = "is.example.contacts-save-denied";
        answer_prompt(refused, Choice::Deny).unwrap();
        assert!(apply_save(refused, &add, &book()).is_err());
    }

    #[test]
    fn a_limited_grant_reads_but_cannot_save() {
        let id = "is.example.contacts-save-limited";
        answer_prompt(id, Choice::Picked([1u64].into_iter().collect())).unwrap();
        let edit = obj(&[("update", Json::Arr(vec![obj(&[("id", Json::Num(1.0)), ("given_name", text("Ada"))])]))]);
        assert!(apply_save(id, &edit, &book()).is_err());
    }

    #[test]
    fn full_access_adds_a_contact_with_the_next_free_id() {
        let id = "is.example.contacts-save-add";
        answer_prompt(id, Choice::All).unwrap();
        let add = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("emails", texts(&["a@x"]))])]))]);
        let (reply, next) = apply_save(id, &add, &book()).unwrap();
        assert_eq!(reply.get("added"), Some(&Json::Arr(vec![Json::Num(4.0)])));
        assert_eq!(ids_of(&next), vec![1, 2, 3, 4]);
        assert_eq!(next[3].name, "Ada");
        assert_eq!(next[3].email, "a@x");
    }

    #[test]
    fn several_adds_get_ids_in_order_and_the_reply_lists_them() {
        let id = "is.example.contacts-save-adds";
        answer_prompt(id, Choice::All).unwrap();
        let add = obj(&[(
            "add",
            Json::Arr(vec![obj(&[("given_name", text("Ada"))]), obj(&[("organization", text("Analytical Co"))])]),
        )]);
        let (reply, next) = apply_save(id, &add, &book()).unwrap();
        assert_eq!(reply.get("added"), Some(&Json::Arr(vec![Json::Num(4.0), Json::Num(5.0)])));
        assert_eq!(ids_of(&next), vec![1, 2, 3, 4, 5]);
        assert_eq!(next[4].company, "Analytical Co");
    }

    #[test]
    fn an_update_replaces_the_fields_of_a_known_contact_and_keeps_its_flags() {
        let id = "is.example.contacts-save-update";
        answer_prompt(id, Choice::All).unwrap();
        let mut stored = book();
        stored[1].favorite = true;
        let edit = obj(&[(
            "update",
            Json::Arr(vec![obj(&[
                ("id", Json::Num(2.0)),
                ("given_name", text("Grace")),
                ("family_name", text("Hopper")),
                ("emails", texts(&["g@x"])),
            ])]),
        )]);
        let (_, next) = apply_save(id, &edit, &stored).unwrap();
        assert_eq!(next[1].name, "Grace Hopper");
        assert_eq!(next[1].email, "g@x");
        assert_eq!(next[1].phone, "");
        assert!(next[1].favorite);
    }

    #[test]
    fn an_update_keeps_the_fields_it_does_not_name_and_null_clears_one() {
        let id = "is.example.contacts-save-partial";
        answer_prompt(id, Choice::All).unwrap();
        let edit = obj(&[("update", Json::Arr(vec![obj(&[("id", Json::Num(1.0)), ("emails", texts(&["ada@new"])), ("note", Json::Null)])]))]);
        let (_, next) = apply_save(id, &edit, &book()).unwrap();
        assert_eq!(next[0].name, "Ada Lovelace");
        assert_eq!(next[0].phone, "+354 555 0100");
        assert_eq!(next[0].email, "ada@new");
        assert_eq!(next[0].notes, "");
        let rename = obj(&[("update", Json::Arr(vec![obj(&[("id", Json::Num(1.0)), ("given_name", text("Augusta Ada"))])]))]);
        let (_, next) = apply_save(id, &rename, &book()).unwrap();
        assert_eq!(next[0].name, "Augusta Ada Lovelace");
    }

    #[test]
    fn an_update_of_an_unknown_or_unnamed_contact_errors() {
        let id = "is.example.contacts-save-update-bad";
        answer_prompt(id, Choice::All).unwrap();
        let unknown = obj(&[("update", Json::Arr(vec![obj(&[("id", Json::Num(99.0)), ("given_name", text("No"))])]))]);
        assert!(apply_save(id, &unknown, &book()).is_err());
        let unnamed = obj(&[("update", Json::Arr(vec![obj(&[("given_name", text("No id"))])]))]);
        assert!(apply_save(id, &unnamed, &book()).is_err());
    }

    #[test]
    fn delete_removes_the_contact_and_an_unknown_id_errors() {
        let id = "is.example.contacts-save-delete";
        answer_prompt(id, Choice::All).unwrap();
        let remove = obj(&[("delete", Json::Arr(vec![obj(&[("id", Json::Num(1.0))])]))]);
        let (_, next) = apply_save(id, &remove, &book()).unwrap();
        assert_eq!(ids_of(&next), vec![2, 3]);
        let missing = obj(&[("delete", Json::Arr(vec![obj(&[("id", Json::Num(99.0))])]))]);
        assert!(apply_save(id, &missing, &book()).is_err());
    }

    #[test]
    fn a_save_applies_all_of_its_changes_or_none() {
        let id = "is.example.contacts-save-atomic";
        answer_prompt(id, Choice::All).unwrap();
        let mixed = obj(&[
            ("add", Json::Arr(vec![obj(&[("given_name", text("Ada"))])])),
            ("delete", Json::Arr(vec![obj(&[("id", Json::Num(99.0))])])),
        ]);
        assert!(apply_save(id, &mixed, &book()).is_err());
    }

    #[test]
    fn an_invalid_or_unknown_field_is_refused() {
        let id = "is.example.contacts-save-invalid";
        answer_prompt(id, Choice::All).unwrap();
        let note_only = obj(&[("add", Json::Arr(vec![obj(&[("note", text("only a note"))])]))]);
        assert!(apply_save(id, &note_only, &book()).is_err());
        let unknown = obj(&[("add", Json::Arr(vec![obj(&[("nickname", text("Ada"))])]))]);
        assert!(apply_save(id, &unknown, &book()).is_err());
        let two_phones = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("phones", texts(&["1", "2"]))])]))]);
        assert!(apply_save(id, &two_phones, &book()).is_err());
    }

    #[test]
    fn an_empty_save_is_refused() {
        let id = "is.example.contacts-save-empty";
        answer_prompt(id, Choice::All).unwrap();
        assert!(apply_save(id, &obj(&[]), &book()).is_err());
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
