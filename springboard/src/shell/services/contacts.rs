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
//!   (`given_name`, `middle_name`, `family_name`, `name_suffix`, `name_prefix`, `nickname`, `organization`,
//!   `job_title`, `department`, `phones`, `emails`, `address`, `note`, `urls`, `relations`, `birthday`, `dates`,
//!   `social_profiles`, `instant_messages`) and must not be empty. `phones`, `emails`, `urls` and `relations` are lists
//!   of `{label, value, identifier}`, `birthday` is `{year?, month, day}`, `dates` is a list of
//!   `{label, year?, month, day}`, and `social_profiles` and `instant_messages` are lists of
//!   `{label, service, username, url, user_identifier, identifier}`. `query`
//!   filters by any text of the contact. Answers only the contacts the app may read, sorted by name. Without a
//!   read grant it is an error.
//! - `view {contact}` shows the person one contact's card in Contacts (`contact` is a name or an identifier) and
//!   returns `{status: "shown"}`. It reads nothing from the book, so it needs no grant.
//! - `save {add?, update?, delete?}` changes the book and returns `{added: [id, ...]}`, the ids the new contacts got.
//!   `add` and `update` hold records of the fields above in the shapes `fetch` returns (`update` also needs the contact
//!   `id` and changes only the fields it names; `null` clears one). A list of labeled values may also be plain strings.
//!   The first phone and email go to the contacts file, the rest and every other multi-value or extra field to the
//!   sidecar; the address takes one value. `delete` holds `{id}` objects. Only full access writes: a limited grant is read-only. The whole
//!   call applies or none of it does, and the shell stores the book only when it succeeds.
//!
//! - `export_vcard {ids?, version?}` returns `{vcard, count}`: the contacts the app may read as vCard text (`version` is
//!   `"3.0"` by default or `"4.0"`; `ids` narrows it). Needs a read grant; a contact the app cannot read is left out.
//! - `import_vcard {vcard}` reads the cards in the text, adds the people the book does not hold yet (same name and the
//!   same email or phone counts as held) and returns `{added: [id, ...], skipped, duplicates}`. Needs full access; the
//!   whole text applies or none of it does. Extra phones, emails, URLs, the birthday and dates go to the sidecar and a
//!   card's picture to the picture store; each added contact is an added event in the change log.
//! - `containers {}` returns `{containers: [{id, name, kind}]}`; `kind` is `local`, `account` or `other`. The local
//!   container (id 1) always exists. `container_members {container}` returns `{contacts: [id, ...]}` of the contacts the
//!   app can read; `container_of {contact}` returns `{container}`. Every contact is in exactly one container, the local
//!   one unless it was assigned elsewhere. These need a read grant.
//! - `container_add {name, kind?}` returns `{id}`; `container_rename {container, name}`; `container_remove {container}`
//!   returns `{moved}` (its contacts go back to the local container, none is deleted; the local one cannot be
//!   removed); `container_assign {container, contact}` returns `{changed}` and records an updated event for the contact.
//!   These need full access.
//! - `donate {name?, handle, value}` (any app) offers a person the app saw: `handle` is `email` or `phone`. The shell keeps
//!   it in a sidecar (`contactdonations`) with the app as its source, and returns `{donated}`: false for an address the
//!   person dismissed before. A value that is no email address or phone number is an error.
//! - `suggestions {limit?}` returns `{suggestions: [{key, handle, value, name, sources, count}]}`, the donated people not
//!   yet in the book, best first. `feedback {key, answer}` (`accepted` or `dismissed`) records the person's answer; an
//!   accepted one returns `{contact: {given_name, family_name, email, phone}}` for the book to save, a dismissed one is
//!   never offered again. Only the Contacts app (the owner of the book) may call these two.
//!
//! The first phone and email come from the contacts file (labeled by its label row); more phones and emails, URLs, the
//! birthday and dates, the name prefix, nickname, job title, department, relations, social profiles and instant
//! message addresses come from the sidecar (`contactextras`). A contact without a sidecar entry reads as before, with
//! one-element lists.
//!
//! The store holds one name per contact: the last word of it is the family name and the rest is the given name. The
//! grants live per shell thread (thread-local, like the `privacy` service), so they reset when the shell restarts.
use super::{Shell, SystemService};
use crate::shell::contactextras::{self, Book, Extras};
use crate::shell::contactcard;
use crate::shell::contactchanges;
use crate::shell::contactcontainers::{self, Containers};
use crate::shell::contactdonations;
use crate::shell::contactgroups;
use crate::shell::contactpick;
use crate::shell::contactphotos;
use crate::shell::contactstore::{self, Contact, LABELS};
use bifrost_core::json::Json;
use bifrost_ui::contact_groups::Groups;
use bifrost_ui::contact_vcard;
use bifrost_ui::contact_access::{Access, Choice, Error, Status};
use bifrost_ui::contact_book::{Contact as BookContact, LabeledValue as BookValue};
use bifrost_ui::contact_donation::{DonationStore, Feedback, Handle, Refusal};
use bifrost_ui::contact_fetch::{project, DateComponents, Key, Keys, LabeledValue, Partial, Profile, Record, Value};
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
    super::privacy_permissions::mark_grants_dirty();
    ACCESS.with(|a| a.borrow_mut().entry(app_id.to_string()).or_default().request(choice))
}

/// Every app with a contacts answer, as `(app, status, picked contacts)`, for the privacy store to save.
pub fn grants_snapshot() -> Vec<(String, Status, Vec<u64>)> {
    ACCESS.with(|a| {
        a.borrow()
            .iter()
            .map(|(app, access)| (app.clone(), access.status(), access.picked().iter().copied().collect()))
            .collect()
    })
}

/// A change the person makes in Settings for one app. Only a system restriction blocks it.
pub fn change_by_user(app_id: &str, choice: Choice) -> Result<Status, Error> {
    super::privacy_permissions::mark_grants_dirty();
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
    let parts = name_parts(&c.name);
    r.set(Key::GivenName, Value::Text(parts.given));
    r.set(Key::MiddleName, Value::Text(parts.middle));
    r.set(Key::FamilyName, Value::Text(parts.family));
    r.set(Key::NameSuffix, Value::Text(parts.suffix));
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
    r.set(Key::NamePrefix, Value::Text(extras.name_prefix.clone()));
    r.set(Key::Nickname, Value::Text(extras.nickname.clone()));
    r.set(Key::JobTitle, Value::Text(extras.job_title.clone()));
    r.set(Key::Department, Value::Text(extras.department.clone()));
    r.set(Key::Relations, Value::Labeled(LabeledValue::numbered(extras.relations.clone())));
    r.set(Key::SocialProfiles, Value::Profiles(Profile::numbered(extras.social_profiles.clone())));
    r.set(Key::InstantMessages, Value::Profiles(Profile::numbered(extras.instant_messages.clone())));
    r
}

/// The parts of a store name: given, middle, family and suffix. The store keeps one name string, so these are derived
/// from it on read and joined back into it on save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NameParts {
    pub(crate) given: String,
    pub(crate) middle: String,
    pub(crate) family: String,
    pub(crate) suffix: String,
}

/// Generational and professional suffixes that end a name ("Jr", "III", "Esq."). Matched without case.
const NAME_SUFFIXES: [&str; 9] = ["Jr", "Jr.", "Sr", "Sr.", "II", "III", "IV", "Esq", "Esq."];

/// Splits a name the way a contact card reads it: a trailing suffix is taken off first, then the first word is the
/// given name, the last word the family name and any words between them the middle name. A one-word name is a given
/// name; a two-word name has no middle.
pub(crate) fn name_parts(name: &str) -> NameParts {
    let mut words: Vec<&str> = name.split_whitespace().collect();
    let mut parts = NameParts::default();
    if words.len() > 1 {
        if let Some(last) = words.last().copied() {
            if NAME_SUFFIXES.iter().any(|s| s.eq_ignore_ascii_case(last)) {
                parts.suffix = last.to_string();
                words.pop();
            }
        }
    }
    match words.len() {
        0 => {}
        1 => parts.given = words[0].to_string(),
        n => {
            parts.given = words[0].to_string();
            parts.middle = words[1..n - 1].join(" ");
            parts.family = words[n - 1].to_string();
        }
    }
    parts
}

/// Joins name parts back into the one name string the store keeps, skipping empty parts.
pub(crate) fn join_name(parts: &NameParts) -> String {
    [&parts.given, &parts.middle, &parts.family, &parts.suffix]
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
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
            Ok(Some(Value::Profiles(items))) => Json::Arr(items.iter().map(profile_json).collect()),
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

fn profile_json(p: &Profile) -> Json {
    let field = |name: &str, value: &str| (name.to_string(), Json::Str(value.to_string()));
    Json::Obj(vec![
        field("label", &p.label),
        field("service", &p.service),
        field("username", &p.username),
        field("url", &p.url),
        field("user_identifier", &p.user_identifier),
        field("identifier", &p.identifier),
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
        if matches!(value, Json::Null) {
            // a record drops blank values, so a cleared field is kept aside
            cleared.push(key);
            continue;
        }
        record.set(key, value_of_json(key, value)?);
    }
    Ok((record, cleared))
}

/// The date in a `{year?, month, day}` object.
fn date_of_json(v: &Json) -> Option<DateComponents> {
    let year = match v.get("year").and_then(Json::number) {
        Some(y) if y.fract() == 0.0 && y.abs() < 1.0e6 => Some(y as i32),
        Some(_) => return None,
        None => None,
    };
    DateComponents::new(year, u8::try_from(v.get("month")?.as_u64()?).ok()?, u8::try_from(v.get("day")?.as_u64()?).ok()?)
}

/// The text under `field` of a JSON object, empty when it is missing.
fn text_field(item: &Json, field: &str) -> String {
    item.get(field).and_then(Json::as_str).unwrap_or("").to_string()
}

/// A `save` field in the shape `fetch` returns it. Labeled lists may be plain strings (no label) or `{label, value}`
/// objects; a list of only strings stays a plain list.
fn value_of_json(key: Key, value: &Json) -> Result<Value, String> {
    let name = key.name();
    match (key, value) {
        (Key::Birthday, Json::Obj(_)) => date_of_json(value).map(Value::Date).ok_or_else(|| format!("{name} is {{year?, month, day}}")),
        (Key::Dates, Json::Arr(items)) => items
            .iter()
            .map(|d| date_of_json(d).map(|date| (text_field(d, "label"), date)).ok_or_else(|| format!("{name} is a list of {{label, year?, month, day}}")))
            .collect::<Result<Vec<_>, String>>()
            .map(Value::Dates),
        (Key::SocialProfiles | Key::InstantMessages, Json::Arr(items)) => {
            let social = key == Key::SocialProfiles;
            items
                .iter()
                .map(|p| match p {
                    Json::Obj(_) => Ok(Profile {
                        label: text_field(p, "label"),
                        service: text_field(p, "service"),
                        username: text_field(p, "username"),
                        url: if social { text_field(p, "url") } else { String::new() },
                        user_identifier: if social { text_field(p, "user_identifier") } else { String::new() },
                        identifier: String::new(),
                    }),
                    _ => Err(format!("{name} is a list of {{label, service, username}} objects")),
                })
                .collect::<Result<Vec<_>, String>>()
                .map(Value::Profiles)
        }
        (Key::Birthday | Key::Dates | Key::SocialProfiles | Key::InstantMessages, _) => {
            Err(format!("{name} is not in the shape fetch returns"))
        }
        (_, Json::Str(text)) => Ok(Value::Text(text.clone())),
        (_, Json::Arr(items)) if items.iter().all(|i| matches!(i, Json::Str(_))) => {
            Ok(Value::List(items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()))
        }
        (_, Json::Arr(items)) => items
            .iter()
            .map(|item| match item {
                Json::Str(v) => Ok(LabeledValue::new("", v)),
                Json::Obj(_) => item
                    .get("value")
                    .and_then(Json::as_str)
                    .map(|v| LabeledValue::new(&text_field(item, "label"), v))
                    .ok_or_else(|| format!("{name} entries are text or {{label, value}}")),
                _ => Err(format!("{name} is a list of text")),
            })
            .collect::<Result<Vec<_>, String>>()
            .map(Value::Labeled),
        _ => Err(format!("{name} is text or a list of text")),
    }
}

/// The entries of a text, list or labeled field, in order, trimmed, with blank ones left out.
fn entries_of(record: &Record, key: Key) -> Vec<LabeledValue> {
    let items: Vec<(&str, &str)> = match record.get(key) {
        Some(Value::Text(text)) => vec![("", text.as_str())],
        Some(Value::List(items)) => items.iter().map(|v| ("", v.as_str())).collect(),
        Some(Value::Labeled(items)) => items.iter().map(|i| (i.label.as_str(), i.value.as_str())).collect(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(label, value)| LabeledValue::new(label.trim(), value.trim()))
        .collect()
}

/// The one value a record holds for a key, or blank. The book keeps one address per contact, so a list of more than
/// one is refused rather than cut down. Phones and emails give their first entry: the rest go to the sidecar
/// ([`extras_of`]).
fn value_of(record: &Record, key: Key) -> Result<String, String> {
    if matches!(key, Key::Phones | Key::Emails) {
        return Ok(entries_of(record, key).first().map(|i| i.value.clone()).unwrap_or_default());
    }
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
        // these live in the sidecar, written by `extras_of`
        Some(Value::Date(_) | Value::Dates(_) | Value::Profiles(_)) => Err(format!("{} is not a contacts file field", key.name())),
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
    let kept = base.map_or_else(NameParts::default, |c| name_parts(&c.name));
    let name = NameParts {
        given: field(Key::GivenName, &kept.given)?,
        middle: field(Key::MiddleName, &kept.middle)?,
        family: field(Key::FamilyName, &kept.family)?,
        suffix: field(Key::NameSuffix, &kept.suffix)?,
    };
    let contact = Contact {
        id,
        name: join_name(&name),
        company: field(Key::Organization, base.map_or("", |c| &c.company))?,
        phone: field(Key::Phones, base.map_or("", |c| &c.phone))?,
        email: field(Key::Emails, base.map_or("", |c| &c.email))?,
        address: field(Key::Address, base.map_or("", |c| &c.address))?,
        notes: field(Key::Note, base.map_or("", |c| &c.notes))?,
        favorite: base.is_some_and(|c| c.favorite),
        uid: base.map_or_else(|| crate::shell::contactstore::new_uid(id), |c| c.uid.clone()),
        labels: first_labels(record, cleared, base.map_or([0; 3], |c| c.labels)),
    };
    if contact.valid() {
        Ok(contact)
    } else {
        Err("a contact needs a name, company, address, phone or email, within the field limits".into())
    }
}

/// The label rows of a saved contact: a labeled first phone or email the save names sets its row (a label the row
/// does not hold reads as `other`); anything else keeps the row.
fn first_labels(record: &Record, cleared: &[Key], mut labels: [u8; 3]) -> [u8; 3] {
    for (set, key) in [(0usize, Key::Phones), (1, Key::Emails)] {
        if cleared.contains(&key) {
            continue;
        }
        if let Some(first) = entries_of(record, key).into_iter().next().filter(|i| !i.label.is_empty()) {
            let label = first.label.to_lowercase();
            let names = LABELS[set];
            let at = names.iter().position(|n| *n == label).or_else(|| names.iter().position(|n| *n == "other")).unwrap_or(0);
            labels[set] = at as u8;
        }
    }
    labels
}

/// Refuses text longer than the sidecar keeps, rather than cutting it.
fn fits(key: Key, text: &str) -> Result<(), String> {
    if text.chars().count() > contactextras::MAX_FIELD {
        Err(format!("{} is longer than {} characters", key.name(), contactextras::MAX_FIELD))
    } else {
        Ok(())
    }
}

/// The sidecar extras of a saved contact. An update (`base`) changes only the fields the record names, and `null`
/// clears one. Phones and emails after the first, URLs, relations, the birthday, dates, the name prefix, nickname, job
/// title, department, social profiles and instant message addresses live here.
fn extras_of(record: &Record, cleared: &[Key], base: Option<&Extras>) -> Result<Extras, String> {
    let mut e = base.cloned().unwrap_or_default();
    let named = |key: Key| record.get(key).is_some() || cleared.contains(&key);
    for (key, skip) in [(Key::Phones, 1), (Key::Emails, 1), (Key::Urls, 0), (Key::Relations, 0)] {
        if !named(key) {
            continue;
        }
        let items: Vec<LabeledValue> = entries_of(record, key).into_iter().skip(skip).collect();
        for i in &items {
            fits(key, &i.label)?;
            fits(key, &i.value)?;
        }
        match key {
            Key::Phones => e.phones = items,
            Key::Emails => e.emails = items,
            Key::Urls => e.urls = items,
            _ => e.relations = items,
        }
    }
    if named(Key::Birthday) {
        e.birthday = match record.get(Key::Birthday) {
            Some(Value::Date(d)) => Some(*d),
            _ => None,
        };
    }
    if named(Key::Dates) {
        e.dates = match record.get(Key::Dates) {
            Some(Value::Dates(items)) => items.iter().map(|(l, d)| (l.trim().to_string(), *d)).collect(),
            _ => Vec::new(),
        };
        for (label, _) in &e.dates {
            fits(Key::Dates, label)?;
        }
    }
    for key in [Key::NamePrefix, Key::Nickname, Key::JobTitle, Key::Department] {
        if !named(key) {
            continue;
        }
        let text = match record.get(key) {
            None => String::new(),
            Some(Value::Text(t)) => t.trim().to_string(),
            Some(Value::List(items)) if items.len() <= 1 => items.first().map(|t| t.trim().to_string()).unwrap_or_default(),
            Some(_) => return Err(format!("{} is text", key.name())),
        };
        fits(key, &text)?;
        match key {
            Key::NamePrefix => e.name_prefix = text,
            Key::Nickname => e.nickname = text,
            Key::JobTitle => e.job_title = text,
            _ => e.department = text,
        }
    }
    for key in [Key::SocialProfiles, Key::InstantMessages] {
        if !named(key) {
            continue;
        }
        let items: Vec<Profile> = match record.get(key) {
            Some(Value::Profiles(items)) => {
                items.iter().filter(|p| !p.is_blank()).map(|p| Profile { identifier: String::new(), ..p.clone() }).collect()
            }
            _ => Vec::new(),
        };
        for p in &items {
            for text in [&p.label, &p.service, &p.username, &p.url, &p.user_identifier] {
                fits(key, text)?;
            }
        }
        if key == Key::SocialProfiles {
            e.social_profiles = items;
        } else {
            e.instant_messages = items;
        }
    }
    Ok(e)
}

/// The list under `key` in a `save` call, empty when the call omits it.
fn list_of<'a>(params: &'a Json, key: &str) -> Result<&'a [Json], String> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::Arr(items)) => Ok(items.as_slice()),
        Some(_) => Err(format!("{key} is a list")),
    }
}

/// Applies a `save` call to `book` for `app_id` and returns the reply and the new book, without the sidecar fields
/// (more phones and emails, URLs, dates and the other extras are dropped). For a caller that keeps no sidecar; the
/// service uses [`apply_save_with`].
pub fn apply_save(app_id: &str, params: &Json, book: &[Contact]) -> Result<(Json, Vec<Contact>), String> {
    apply_save_with(app_id, params, book, &Book::new()).map(|(reply, next, _)| (reply, next))
}

/// Keeps a contact's extras in `book`, or drops its entry when it has none.
fn keep_extras(book: &mut Book, id: u64, extras: Extras) {
    if extras.is_empty() {
        book.remove(&id);
    } else {
        book.insert(id, extras);
    }
}

/// Applies a `save` call to `book` and its sidecar `extras` for `app_id`, and returns the reply, the new book and the
/// new sidecar. Pure: the caller stores both only when this is `Ok`, so a call that fails in any item changes nothing.
/// Only full access writes. An add gets the next free id; an update and a delete must name a contact the book holds. A
/// deleted contact leaves the sidecar too.
pub fn apply_save_with(app_id: &str, params: &Json, book: &[Contact], extras: &Book) -> Result<(Json, Vec<Contact>, Book), String> {
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
    let mut more = extras.clone();
    let mut added = Vec::new();
    for item in adds {
        let id = contactstore::new_id(&next).ok_or_else(|| "the book is full".to_string())?;
        let parsed = record_of_json(item)?;
        next.push(contact_of(id, &parsed, None)?);
        keep_extras(&mut more, id, extras_of(&parsed.0, &parsed.1, None)?);
        added.push(Json::Num(id as f64));
    }
    for item in updates {
        let parsed = record_of_json(item)?;
        let id = parsed.0.id;
        if id == 0 {
            return Err("update needs the id of a contact".into());
        }
        let at = next.iter().position(|c| c.id == id).ok_or_else(|| format!("no contact {id}"))?;
        let updated = contact_of(id, &parsed, Some(&next[at]))?;
        let e = extras_of(&parsed.0, &parsed.1, more.get(&id))?;
        keep_extras(&mut more, id, e);
        next[at] = updated;
    }
    for item in deletes {
        let id = item.get("id").and_then(Json::as_u64).ok_or_else(|| "delete needs the id of a contact".to_string())?;
        let at = next.iter().position(|c| c.id == id).ok_or_else(|| format!("no contact {id}"))?;
        next.remove(at);
        more.remove(&id);
    }
    Ok((Json::Obj(vec![("added".into(), Json::Arr(added))]), next, more))
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

/// Answers `export_vcard {ids?, version?}`: the cards of the contacts the app may read, as `{vcard, count}`. `version` is
/// `"3.0"` (the default) or `"4.0"`; `ids` narrows the export to those contacts (an id the app cannot read is left out,
/// never an error, so the reply does not tell the app which unreadable contacts exist). Needs a read grant.
pub fn vcard_export_answer(
    app_id: &str,
    params: &Json,
    book: &[Contact],
    extras: &Book,
    photo: &dyn Fn(u64) -> Option<Vec<u8>>,
) -> Result<Json, String> {
    if !reads(app_id) {
        return Err("contacts access is not granted".into());
    }
    let access = access_of(app_id);
    let version = match params.get("version").and_then(Json::as_str) {
        None | Some("3.0") => contact_vcard::Version::V3,
        Some("4.0") => contact_vcard::Version::V4,
        Some(other) => return Err(format!("export_vcard version is 3.0 or 4.0, not {other}")),
    };
    let wanted: Option<Vec<u64>> = match params.get("ids") {
        None | Some(Json::Null) => None,
        Some(Json::Arr(items)) => Some(items.iter().filter_map(Json::as_u64).collect()),
        Some(_) => return Err("ids is a list".into()),
    };
    let readable: Vec<Contact> = book
        .iter()
        .filter(|c| access.can_read(c.id) && wanted.as_ref().map_or(true, |w| w.contains(&c.id)))
        .cloned()
        .collect();
    let text = contactcard::export(&readable, extras, None, photo, version);
    Ok(Json::Obj(vec![("vcard".into(), Json::Str(text)), ("count".into(), Json::Num(readable.len() as f64))]))
}

/// Answers `import_vcard {vcard}`: reads the cards in the text and plans their addition to the book. Needs full access
/// (a limited grant is read-only). Returns the reply `{added: [id, ...], skipped, duplicates}` and the plan the caller
/// stores; nothing is stored when this is an error. A person already in the book is a duplicate and is not added.
pub fn vcard_import_plan(app_id: &str, params: &Json, book: &[Contact]) -> Result<(Json, contactcard::Plan), String> {
    access_of(app_id).check_write().map_err(|e| match e {
        Error::ReadOnly => "a limited contacts grant only reads; importing needs full access".to_string(),
        _ => "contacts write access is not granted".to_string(),
    })?;
    let text = params.get("vcard").and_then(Json::as_str).ok_or_else(|| "import_vcard needs vcard".to_string())?;
    let plan = contactcard::plan_import(text, book)?;
    let reply = Json::Obj(vec![
        ("added".into(), Json::Arr(plan.added.iter().map(|id| Json::Num(*id as f64)).collect())),
        ("skipped".into(), Json::Num(plan.skipped as f64)),
        ("duplicates".into(), Json::Num(plan.duplicates as f64)),
    ]);
    Ok((reply, plan))
}

/// The container methods. Reads need a read grant, writes need full access, and a container lists only the contacts
/// the app can see. Returns the reply, whether the containers changed (so the caller saves the sidecar) and the
/// contacts whose container changed (an updated event each in the change log).
pub fn containers_answer(
    app_id: &str,
    method: &str,
    params: &Json,
    contacts: &[Contact],
    containers: &mut Containers,
) -> Result<(Json, bool, Vec<u64>), String> {
    let access = access_of(app_id);
    let number = |key: &str| params.get(key).and_then(Json::as_u64).ok_or_else(|| format!("{method} needs {key}"));
    let writable = || access.check_write().map_err(|_| "contacts write access is not granted".to_string());
    let no_grant = || "contacts access is not granted".to_string();
    let fail = |e: contactcontainers::Error| e.message().to_string();
    match method {
        "containers" => {
            if !reads(app_id) {
                return Err(no_grant());
            }
            let rows: Vec<Json> = containers
                .all()
                .map(|c| {
                    Json::Obj(vec![
                        ("id".into(), Json::Num(c.id as f64)),
                        ("name".into(), Json::Str(c.name.clone())),
                        ("kind".into(), Json::Str(c.kind.name().into())),
                    ])
                })
                .collect();
            Ok((Json::Obj(vec![("containers".into(), Json::Arr(rows))]), false, Vec::new()))
        }
        "container_members" => {
            if !reads(app_id) {
                return Err(no_grant());
            }
            let ids: Vec<u64> = contacts.iter().map(|c| c.id).filter(|id| access.can_read(*id)).collect();
            let members = containers.members(number("container")?, &ids).map_err(fail)?;
            let list = members.into_iter().map(|m| Json::Num(m as f64)).collect();
            Ok((Json::Obj(vec![("contacts".into(), Json::Arr(list))]), false, Vec::new()))
        }
        "container_of" => {
            if !reads(app_id) {
                return Err(no_grant());
            }
            let contact = number("contact")?;
            if !contacts.iter().any(|c| c.id == contact) || !access.can_read(contact) {
                return Err("no such contact".into());
            }
            Ok((Json::Obj(vec![("container".into(), Json::Num(containers.container_of(contact) as f64))]), false, Vec::new()))
        }
        "container_add" => {
            writable()?;
            let name = params.get("name").and_then(Json::as_str).ok_or_else(|| "container_add needs name".to_string())?;
            let kind = match params.get("kind").and_then(Json::as_str) {
                None => contactcontainers::Kind::Other,
                Some(k) => contactcontainers::Kind::from_name(k).ok_or_else(|| format!("container kind {k} is unknown"))?,
            };
            let id = containers.add(name, kind).map_err(fail)?;
            Ok((Json::Obj(vec![("id".into(), Json::Num(id as f64))]), true, Vec::new()))
        }
        "container_rename" => {
            writable()?;
            let name = params.get("name").and_then(Json::as_str).ok_or_else(|| "container_rename needs name".to_string())?;
            containers.rename(number("container")?, name).map_err(fail)?;
            Ok((Json::Obj(vec![("renamed".into(), Json::Bool(true))]), true, Vec::new()))
        }
        "container_remove" => {
            writable()?;
            let moved = containers.remove(number("container")?).map_err(fail)?;
            let reply = Json::Obj(vec![("moved".into(), Json::Num(moved.len() as f64))]);
            Ok((reply, true, moved))
        }
        "container_assign" => {
            writable()?;
            let (container, contact) = (number("container")?, number("contact")?);
            if !contacts.iter().any(|c| c.id == contact) || !access.can_read(contact) {
                return Err("no such contact".into());
            }
            let changed = containers.assign(contact, container).map_err(fail)?;
            let reply = Json::Obj(vec![("changed".into(), Json::Bool(changed))]);
            Ok((reply, changed, if changed { vec![contact] } else { Vec::new() }))
        }
        _ => Err(format!("contacts has no method {method}")),
    }
}

pub struct Contacts;

/// The apps that own the book: only they see the donated people and answer for them.
pub fn owns_book(app_id: &str) -> bool {
    matches!(app_id, "is.olibuijr.contacts" | "is.olibuijr.contacts.desktop")
}

/// The book as the donation model needs it: the email addresses and phone numbers the store and its sidecar hold, so a
/// person already in the book is never suggested.
pub fn handles_of(contacts: &[Contact], extras: &Book) -> Vec<BookContact> {
    contacts
        .iter()
        .map(|c| {
            let more = extras.get(&c.id).cloned().unwrap_or_default();
            let mut book = BookContact { identifier: c.id.to_string(), ..BookContact::default() };
            for email in std::iter::once(c.email.as_str()).chain(more.emails.iter().map(|e| e.value.as_str())) {
                if !email.trim().is_empty() {
                    book.email_addresses.push(BookValue::new(None, email.to_string()));
                }
            }
            for phone in std::iter::once(c.phone.as_str()).chain(more.phones.iter().map(|p| p.value.as_str())) {
                if !phone.trim().is_empty() {
                    book.phone_numbers.push(BookValue::new(None, phone.to_string()));
                }
            }
            book
        })
        .collect()
}

/// Answers `donate`, `suggestions` and `feedback` over `store`. `book` is the people already in the address book and
/// `now` the clock in seconds. The flag says whether the store changed and must be saved.
pub fn donations_answer(app_id: &str, method: &str, params: &Json, store: &mut DonationStore, book: &[BookContact], now: u64) -> Result<(Json, bool), String> {
    let text = |name: &str| params.get(name).and_then(Json::as_str).unwrap_or("");
    match method {
        "donate" => {
            let handle = match text("handle") {
                "email" => Handle::Email,
                "phone" => Handle::Phone,
                other => return Err(format!("donate needs handle email or phone, not {other:?}")),
            };
            match store.donate(app_id, text("name"), handle, text("value"), now) {
                Ok(()) => Ok((Json::Obj(vec![("donated".into(), Json::Bool(true))]), true)),
                Err(Refusal::Blocked) => Ok((Json::Obj(vec![("donated".into(), Json::Bool(false))]), false)),
                Err(Refusal::BadField) => Err("donate needs a value and a name of at most 200 bytes each".into()),
                Err(Refusal::BadHandle) => Err("the value is not an email address or phone number".into()),
            }
        }
        "suggestions" | "feedback" if !owns_book(app_id) => Err("only the Contacts app sees suggested contacts".into()),
        "suggestions" => {
            let limit = params.get("limit").and_then(Json::as_u64).map_or(20, |n| n.min(100) as usize);
            let rows = store
                .suggestions(book, limit)
                .into_iter()
                .map(|d| {
                    Json::Obj(vec![
                        ("key".into(), Json::Str(d.key.clone())),
                        ("handle".into(), Json::Str(if d.handle == Handle::Email { "email" } else { "phone" }.into())),
                        ("value".into(), Json::Str(d.display.clone())),
                        ("name".into(), Json::Str(d.name.clone())),
                        ("sources".into(), Json::Num(d.sources.len() as f64)),
                        ("count".into(), Json::Num(f64::from(d.count))),
                    ])
                })
                .collect();
            Ok((Json::Obj(vec![("suggestions".into(), Json::Arr(rows))]), false))
        }
        "feedback" => {
            let key = text("key");
            let answer = match text("answer") {
                "accepted" => Feedback::Accepted,
                "dismissed" => Feedback::Dismissed,
                other => return Err(format!("feedback answer is accepted or dismissed, not {other:?}")),
            };
            let contact = store.feedback(key, answer);
            match (answer, contact) {
                (Feedback::Accepted, Some(c)) => {
                    let first = |list: &[BookValue<String>]| list.first().map_or(Json::Null, |v| Json::Str(v.value.clone()));
                    let reply = Json::Obj(vec![
                        ("given_name".into(), Json::Str(c.given_name.clone())),
                        ("family_name".into(), Json::Str(c.family_name.clone())),
                        ("email".into(), first(&c.email_addresses)),
                        ("phone".into(), first(&c.phone_numbers)),
                    ]);
                    Ok((Json::Obj(vec![("contact".into(), reply)]), true))
                }
                (Feedback::Accepted, None) => Err("no such suggestion".into()),
                (Feedback::Dismissed, _) => Ok((Json::Obj(Vec::new()), true)),
            }
        }
        _ => Err(format!("contacts has no method {method}")),
    }
}

/// The Contacts app, which draws a contact's card for the `view` method.
pub const CONTACTS_APP: &str = "is.olibuijr.contacts.desktop";

/// The longest contact name or identifier a `view` call may carry, in characters.
const VIEW_QUERY_MAX: usize = 200;

/// The text the Contacts app is asked to open: the `contact` param, a name or an identifier (a number is an identifier).
/// A blank value, a control character or a longer value is refused.
pub fn view_query(params: &Json) -> Result<String, String> {
    let text = match params.get("contact") {
        Some(Json::Num(n)) if n.fract() == 0.0 && *n >= 0.0 => format!("{}", *n as u64),
        Some(Json::Str(s)) => s.trim().to_string(),
        _ => return Err("view needs a contact: a name or an identifier".into()),
    };
    if text.is_empty() || text.chars().count() > VIEW_QUERY_MAX || text.chars().any(char::is_control) {
        return Err("view needs a contact: a name or an identifier".into());
    }
    Ok(text)
}

impl SystemService for Contacts {
    fn name(&self) -> &'static str {
        "contacts"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let path = contactstore::store_path(&shell.data_dir);
        let groups_path = contactgroups::path_for(&path);
        let load = || contactstore::load_checked(&path).map_err(|e| e.to_string());
        match method {
            "save" => {
                let book = if access_of(app_id).check_write().is_ok() { load()? } else { Vec::new() };
                let extras_path = contactextras::path_for(&path);
                let extras = if book.is_empty() { Book::new() } else { contactextras::load(&extras_path) };
                let (reply, next, next_extras) = apply_save_with(app_id, params, &book, &extras)?;
                contactstore::save(&path, &next).map_err(|e| e.to_string())?;
                if next_extras != extras {
                    contactextras::save(&extras_path, &next_extras).map_err(|e| e.to_string())?;
                }
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
                // A deleted contact leaves its container too.
                let containers_path = contactcontainers::path_for(&path);
                let mut containers = contactcontainers::load(&containers_path);
                let mut left = false;
                for (kind, id) in &changes {
                    if *kind == contactchanges::Kind::Deleted {
                        left |= containers.forget_contact(*id);
                    }
                }
                if left {
                    contactcontainers::save(&containers_path, &containers).map_err(|e| e.to_string())?;
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
            "export_vcard" => {
                if !reads(app_id) {
                    return Err("contacts access is not granted".into());
                }
                let extras = contactextras::load(&contactextras::path_for(&path));
                let photos = contactphotos::dir_for(&path);
                let photo = |id: u64| contactphotos::read(&photos, id).map(|(_, bytes)| bytes);
                vcard_export_answer(app_id, params, &load()?, &extras, &photo)
            }
            "import_vcard" => {
                let book = if access_of(app_id).check_write().is_ok() { load()? } else { Vec::new() };
                let (reply, plan) = vcard_import_plan(app_id, params, &book)?;
                if plan.added.is_empty() {
                    return Ok(reply);
                }
                contactstore::save(&path, &plan.book).map_err(|e| e.to_string())?;
                if !plan.extras.is_empty() {
                    let extras_path = contactextras::path_for(&path);
                    let mut extras = contactextras::load(&extras_path);
                    for (id, e) in &plan.extras {
                        extras.insert(*id, e.clone());
                    }
                    contactextras::save(&extras_path, &extras).map_err(|e| e.to_string())?;
                }
                // a picture the store refuses (not PNG or JPEG, or too big) leaves the contact without one
                let photos = contactphotos::dir_for(&path);
                for (id, bytes) in &plan.photos {
                    let _ = contactphotos::write(&photos, *id, bytes);
                }
                let created: Vec<(contactchanges::Kind, u64)> = contactchanges::diff(&book, &plan.book);
                contactchanges::record_for(&path, &created).map_err(|e| e.to_string())?;
                Ok(reply)
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
                let app_name = super::super::apphost::installed()
                    .iter()
                    .find(|a| a.manifest.id == app_id)
                    .map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
                contactpick::start(app_id, &app_name, bifrost_ui::contact_picker::Picker::new(entries.clone(), mode), &entries);
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
                    if removed {
                        contactchanges::record_for(&path, &[(contactchanges::Kind::Updated, id)]).map_err(|e| e.to_string())?;
                    }
                    return Ok(Json::Obj(vec![("removed".into(), Json::Bool(removed))]));
                }
                let encoded = params.get("data").and_then(Json::as_str).ok_or_else(|| "photo_set needs data".to_string())?;
                let bytes = contact_vcard::base64_decode(encoded).ok_or_else(|| "photo data is not base64".to_string())?;
                let format = contactphotos::write(&dir, id, &bytes)?;
                contactchanges::record_for(&path, &[(contactchanges::Kind::Updated, id)]).map_err(|e| e.to_string())?;
                Ok(Json::Obj(vec![("format".into(), Json::Str(contactphotos::format_name(format).into()))]))
            }
            "view" => {
                // Shows the person one contact's card in Contacts. The card is drawn by that app, so the caller reads
                // nothing from the book and needs no grant; it learns only that the request was made.
                let query = view_query(params)?;
                shell.run_intent(CONTACTS_APP, "view-card", "contact", &query, t);
                Ok(Json::Obj(vec![("status".into(), Json::Str("shown".into()))]))
            }
            "request" => {
                // An undecided app gets the access sheet; the status it reads is the one before the answer.
                let status = access_of(app_id).status();
                if status == Status::NotDetermined {
                    let name = super::super::apphost::installed()
                        .iter()
                        .find(|a| a.manifest.id == app_id)
                        .map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
                    super::privacy_permissions::queue_prompt(app_id, &name, super::privacy_permissions::Kind::Contacts, "");
                }
                Ok(Json::Obj(vec![("status".into(), Json::Str(status.name().into()))]))
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
            "donate" | "suggestions" | "feedback" => {
                let donations_path = contactdonations::path_for(&path);
                let mut store = contactdonations::load(&donations_path);
                // the owner's suggestions leave out everyone already in the book
                let book = if method == "donate" || !owns_book(app_id) { Vec::new() } else { handles_of(&load()?, &contactextras::load(&contactextras::path_for(&path))) };
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let (reply, changed) = donations_answer(app_id, method, params, &mut store, &book, now)?;
                if changed {
                    contactdonations::save(&donations_path, &store).map_err(|e| e.to_string())?;
                }
                Ok(reply)
            }
            "containers" | "container_members" | "container_of" | "container_add" | "container_rename" | "container_remove" | "container_assign" => {
                let containers_path = contactcontainers::path_for(&path);
                let mut containers = contactcontainers::load(&containers_path);
                let contacts = if access_of(app_id).check_write().is_ok() || reads(app_id) { load()? } else { Vec::new() };
                let (reply, changed, moved) = containers_answer(app_id, method, params, &contacts, &mut containers)?;
                if changed {
                    contactcontainers::save(&containers_path, &containers).map_err(|e| e.to_string())?;
                }
                if !moved.is_empty() {
                    // a contact that changed container is an updated contact in the change log
                    let events: Vec<_> = moved.iter().map(|id| (contactchanges::Kind::Updated, *id)).collect();
                    contactchanges::record_for(&path, &events).map_err(|e| e.to_string())?;
                }
                Ok(reply)
            }
            "groups" | "group_members" | "group_add" | "group_remove" | "group_add_member" | "group_remove_member" => {
                let mut groups: Groups = contactgroups::load(&groups_path);
                let before = groups.clone();
                let contacts = if access_of(app_id).check_write().is_ok() || reads(app_id) { load()? } else { Vec::new() };
                let (reply, changed) = groups_answer(app_id, method, params, &contacts, &mut groups)?;
                if changed {
                    contactgroups::save(&groups_path, &groups).map_err(|e| e.to_string())?;
                    // a contact whose groups changed is an updated contact in the change log
                    let changes = contactchanges::group_changes(&before, &groups, &contacts);
                    contactchanges::record_for(&path, &changes).map_err(|e| e.to_string())?;
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

    #[test]
    fn view_takes_a_name_or_an_identifier_and_nothing_else() {
        let obj = |v: Json| Json::Obj(vec![("contact".into(), v)]);
        assert_eq!(view_query(&obj(Json::Str("  Ada Lovelace ".into()))), Ok("Ada Lovelace".into()));
        assert_eq!(view_query(&obj(Json::Num(42.0))), Ok("42".into()));
        assert!(view_query(&obj(Json::Str("   ".into()))).is_err());
        assert!(view_query(&obj(Json::Str("a\nb".into()))).is_err());
        assert!(view_query(&obj(Json::Str("x".repeat(201)))).is_err());
        assert!(view_query(&obj(Json::Num(-1.0))).is_err());
        assert!(view_query(&obj(Json::Num(1.5))).is_err());
        assert!(view_query(&Json::Obj(vec![])).is_err());
    }

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

    const OWNER: &str = "is.olibuijr.contacts.desktop";

    fn donate(store: &mut DonationStore, app: &str, name: &str, handle: &str, value: &str) -> Result<(Json, bool), String> {
        let params = Json::Obj(vec![
            ("name".into(), Json::Str(name.into())),
            ("handle".into(), Json::Str(handle.into())),
            ("value".into(), Json::Str(value.into())),
        ]);
        donations_answer(app, "donate", &params, store, &[], 100)
    }

    fn suggested(store: &mut DonationStore, book: &[BookContact]) -> Vec<String> {
        let (reply, _) = donations_answer(OWNER, "suggestions", &Json::Null, store, book, 100).unwrap();
        reply.get("suggestions").and_then(Json::as_array).unwrap().iter().filter_map(|r| r.get("key").and_then(Json::as_str)).map(str::to_string).collect()
    }

    fn feedback(store: &mut DonationStore, key: &str, answer: &str) -> Result<(Json, bool), String> {
        let params = Json::Obj(vec![("key".into(), Json::Str(key.into())), ("answer".into(), Json::Str(answer.into()))]);
        donations_answer(OWNER, "feedback", &params, store, &[], 100)
    }

    #[test]
    fn any_app_donates_but_only_the_contacts_app_sees_the_suggestions() {
        let mut store = DonationStore::new();
        let (reply, changed) = donate(&mut store, "is.olibuijr.mail.desktop", "Ada Lovelace", "email", "Ada@Example.com").unwrap();
        assert_eq!(reply.get("donated"), Some(&Json::Bool(true)));
        assert!(changed);
        assert!(donate(&mut store, "is.olibuijr.mail.desktop", "X", "email", "not-an-address").is_err());
        assert!(donate(&mut store, "is.olibuijr.mail.desktop", "X", "fax", "1").is_err());
        for method in ["suggestions", "feedback"] {
            assert!(donations_answer("is.olibuijr.mail.desktop", method, &Json::Null, &mut store, &[], 100).is_err(), "{method} is the owner's");
        }
        assert_eq!(suggested(&mut store, &[]), vec!["ada@example.com"]);
    }

    #[test]
    fn a_person_in_the_book_is_not_suggested() {
        let mut store = DonationStore::new();
        donate(&mut store, "mail", "Ada Lovelace", "email", "ADA@example.com").unwrap();
        donate(&mut store, "mail", "Bob Stone", "email", "bob@example.com").unwrap();
        donate(&mut store, "mail", "Alan", "phone", "+354 555 0300").unwrap();
        donate(&mut store, "mail", "Cy", "phone", "+354 555 0999").unwrap();
        let known = handles_of(&book(), &Book::new());
        assert_eq!(suggested(&mut store, &known), vec!["+3545550999", "bob@example.com"]);
    }

    #[test]
    fn accepting_returns_the_contact_to_save_and_dismissing_blocks_the_address() {
        let mut store = DonationStore::new();
        donate(&mut store, "mail", "Ada Lovelace", "email", "ada@example.com").unwrap();
        donate(&mut store, "mail", "Bob", "email", "bob@example.com").unwrap();
        let (reply, changed) = feedback(&mut store, "ada@example.com", "accepted").unwrap();
        assert!(changed);
        let contact = reply.get("contact").unwrap();
        assert_eq!(contact.get("given_name").and_then(Json::as_str), Some("Ada"));
        assert_eq!(contact.get("family_name").and_then(Json::as_str), Some("Lovelace"));
        assert_eq!(contact.get("email").and_then(Json::as_str), Some("ada@example.com"));
        assert_eq!(contact.get("phone"), Some(&Json::Null));
        assert!(feedback(&mut store, "nobody@example.com", "accepted").is_err());
        feedback(&mut store, "bob@example.com", "dismissed").unwrap();
        assert!(suggested(&mut store, &[]).is_empty(), "an answered suggestion is gone");
        let (again, saved) = donate(&mut store, "mail", "Bob", "email", "bob@example.com").unwrap();
        assert_eq!(again.get("donated"), Some(&Json::Bool(false)), "a dismissed address stays dismissed");
        assert!(!saved);
        assert!(feedback(&mut store, "bob@example.com", "maybe").is_err());
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
        let unknown = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("shoe_size", text("38"))])]))]);
        assert_eq!(apply_save(id, &unknown, &book()).err(), Some("no contact field shoe_size".to_string()));
        // the book keeps one address per contact, so a second one is refused rather than cut
        let two_addresses = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("address", texts(&["1 Main St", "2 High St"]))])]))]);
        assert!(apply_save(id, &two_addresses, &book()).is_err());
        // phones after the first are kept in the sidecar, not refused
        let two_phones = obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), ("phones", texts(&["1", "2"]))])]))]);
        let (_, next, more) = apply_save_with(id, &two_phones, &book(), &Book::new()).unwrap();
        let added = next.last().unwrap();
        assert_eq!(added.phone, "1");
        assert_eq!(more[&added.id].phones.iter().map(|p| p.value.as_str()).collect::<Vec<_>>(), vec!["2"]);
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
        assert!(answer(id, "fetch", &keys(&["nickname"]), &book()).is_ok(), "nickname is a known field");
        assert_eq!(answer(id, "fetch", &keys(&["shoe_size"]), &book()).err(), Some("no contact field shoe_size".to_string()));
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        assert!(answer("x", "erase", &Json::Null, &book()).is_err());
    }

    #[test]
    fn a_name_splits_into_given_middle_and_family() {
        let r = record_of(&contact(9, "Mary Ann Smith", "", "", ""));
        assert_eq!(r.get(Key::GivenName), Some(&Value::Text("Mary".into())));
        assert_eq!(r.get(Key::MiddleName), Some(&Value::Text("Ann".into())));
        assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("Smith".into())));
        let two = record_of(&contact(11, "Jan van der Berg", "", "", ""));
        assert_eq!(two.get(Key::MiddleName), Some(&Value::Text("van der".into())));
        let one = record_of(&contact(10, "Cher", "", "", ""));
        assert_eq!(one.get(Key::GivenName), Some(&Value::Text("Cher".into())));
        assert_eq!(one.get(Key::FamilyName), None);
    }

    #[test]
    fn a_trailing_suffix_is_split_off_the_name() {
        let r = record_of(&contact(12, "Martin Luther King Jr.", "", "", ""));
        assert_eq!(r.get(Key::GivenName), Some(&Value::Text("Martin".into())));
        assert_eq!(r.get(Key::MiddleName), Some(&Value::Text("Luther".into())));
        assert_eq!(r.get(Key::FamilyName), Some(&Value::Text("King".into())));
        assert_eq!(r.get(Key::NameSuffix), Some(&Value::Text("Jr.".into())));
    }

    #[test]
    fn a_name_joins_back_to_the_same_string_when_nothing_changes() {
        for name in ["Mary Ann Smith", "Cher", "Henry Ford III", "Jan van der Berg"] {
            assert_eq!(join_name(&name_parts(name)), name);
        }
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
                ..Default::default()
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

    fn no_photo(_: u64) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn export_needs_a_read_grant_and_a_limited_grant_exports_only_its_contacts() {
        let id = "is.example.contacts-vcard-export";
        assert!(vcard_export_answer(id, &Json::Null, &book(), &Book::new(), &no_photo).is_err());
        answer_prompt(id, Choice::Picked([1u64, 3].into_iter().collect())).unwrap();
        let reply = vcard_export_answer(id, &Json::Null, &book(), &Book::new(), &no_photo).unwrap();
        assert_eq!(reply.get("count"), Some(&Json::Num(2.0)));
        let text = reply.get("vcard").and_then(Json::as_str).unwrap();
        assert_eq!(text.matches("BEGIN:VCARD").count(), 2);
        // asking by id for a contact the grant does not hold leaves it out, without an error
        let ask = obj(&[("ids", Json::Arr(vec![Json::Num(2.0), Json::Num(3.0)]))]);
        let reply = vcard_export_answer(id, &ask, &book(), &Book::new(), &no_photo).unwrap();
        assert_eq!(reply.get("count"), Some(&Json::Num(1.0)));
    }

    #[test]
    fn export_takes_the_version_and_refuses_an_unknown_one() {
        let id = "is.example.contacts-vcard-version";
        answer_prompt(id, Choice::All).unwrap();
        let four = obj(&[("version", text("4.0"))]);
        let reply = vcard_export_answer(id, &four, &book(), &Book::new(), &no_photo).unwrap();
        assert!(reply.get("vcard").and_then(Json::as_str).unwrap().contains("VERSION:4.0"));
        let odd = obj(&[("version", text("2.1"))]);
        assert!(vcard_export_answer(id, &odd, &book(), &Book::new(), &no_photo).is_err());
    }

    #[test]
    fn import_needs_full_access_and_adds_new_people_with_the_next_ids() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada King\r\nN:King;Ada;;;\r\nEMAIL:ada@king.is\r\nEND:VCARD\r\n";
        let ask = obj(&[("vcard", text(card))]);
        let none = "is.example.contacts-vcard-import-none";
        assert!(vcard_import_plan(none, &ask, &book()).is_err());
        let limited = "is.example.contacts-vcard-import-limited";
        answer_prompt(limited, Choice::Picked([1u64].into_iter().collect())).unwrap();
        assert!(vcard_import_plan(limited, &ask, &book()).is_err());
        let full = "is.example.contacts-vcard-import-full";
        answer_prompt(full, Choice::All).unwrap();
        let (reply, plan) = vcard_import_plan(full, &ask, &book()).unwrap();
        assert_eq!(reply.get("added"), Some(&Json::Arr(vec![Json::Num(4.0)])));
        assert_eq!(plan.book.len(), 4);
        assert_eq!(plan.book[3].email, "ada@king.is");
    }

    #[test]
    fn importing_the_same_cards_twice_adds_nothing_the_second_time() {
        let id = "is.example.contacts-vcard-twice";
        answer_prompt(id, Choice::All).unwrap();
        let exported = vcard_export_answer(id, &Json::Null, &book(), &Book::new(), &no_photo).unwrap();
        let ask = obj(&[("vcard", text(exported.get("vcard").and_then(Json::as_str).unwrap()))]);
        let (reply, plan) = vcard_import_plan(id, &ask, &book()).unwrap();
        assert_eq!(reply.get("added"), Some(&Json::Arr(vec![])));
        assert_eq!(reply.get("duplicates"), Some(&Json::Num(3.0)));
        assert_eq!(plan.book, book());
    }

    #[test]
    fn import_refuses_text_without_cards_and_a_call_without_text() {
        let id = "is.example.contacts-vcard-bad";
        answer_prompt(id, Choice::All).unwrap();
        assert!(vcard_import_plan(id, &obj(&[("vcard", text("not a card"))]), &book()).is_err());
        assert!(vcard_import_plan(id, &Json::Null, &book()).is_err());
    }

    #[test]
    fn containers_list_the_local_one_and_a_read_grant_is_needed() {
        let id = "is.example.contacts-containers-list";
        let mut containers = Containers::default();
        assert!(containers_answer(id, "containers", &Json::Null, &book(), &mut containers).is_err());
        answer_prompt(id, Choice::All).unwrap();
        let (reply, changed, _) = containers_answer(id, "containers", &Json::Null, &book(), &mut containers).unwrap();
        assert!(!changed);
        let rows = reply.get("containers").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("kind").and_then(Json::as_str), Some("local"));
    }

    #[test]
    fn a_contact_assigned_to_a_container_leaves_the_local_one_and_the_move_is_logged() {
        let id = "is.example.contacts-containers-assign";
        answer_prompt(id, Choice::All).unwrap();
        let mut containers = Containers::default();
        let (added, changed, _) = containers_answer(id, "container_add", &obj(&[("name", text("Work")), ("kind", text("account"))]), &book(), &mut containers).unwrap();
        assert!(changed);
        let work = added.get("id").and_then(Json::as_u64).unwrap();
        let assign = obj(&[("container", Json::Num(work as f64)), ("contact", Json::Num(2.0))]);
        let (reply, changed, moved) = containers_answer(id, "container_assign", &assign, &book(), &mut containers).unwrap();
        assert_eq!((reply.get("changed"), changed, moved), (Some(&Json::Bool(true)), true, vec![2]));
        let ask = obj(&[("container", Json::Num(work as f64))]);
        let (members, _, _) = containers_answer(id, "container_members", &ask, &book(), &mut containers).unwrap();
        assert_eq!(members.get("contacts"), Some(&Json::Arr(vec![Json::Num(2.0)])));
        let local = obj(&[("container", Json::Num(1.0))]);
        let (members, _, _) = containers_answer(id, "container_members", &local, &book(), &mut containers).unwrap();
        assert_eq!(members.get("contacts"), Some(&Json::Arr(vec![Json::Num(1.0), Json::Num(3.0)])));
        // assigning again changes nothing and logs nothing
        let (_, changed, moved) = containers_answer(id, "container_assign", &assign, &book(), &mut containers).unwrap();
        assert!(!changed && moved.is_empty());
    }

    #[test]
    fn removing_a_container_moves_its_contacts_home_and_the_local_one_stays() {
        let id = "is.example.contacts-containers-remove";
        answer_prompt(id, Choice::All).unwrap();
        let mut containers = Containers::default();
        let work = containers.add("Work", contactcontainers::Kind::Account).unwrap();
        containers.assign(1, work).unwrap();
        let ask = obj(&[("container", Json::Num(work as f64))]);
        let (reply, changed, moved) = containers_answer(id, "container_remove", &ask, &book(), &mut containers).unwrap();
        assert_eq!((reply.get("moved"), changed, moved), (Some(&Json::Num(1.0)), true, vec![1]));
        assert_eq!(containers.container_of(1), contactcontainers::LOCAL);
        let local = obj(&[("container", Json::Num(1.0))]);
        assert!(containers_answer(id, "container_remove", &local, &book(), &mut containers).is_err());
    }

    #[test]
    fn container_writes_need_full_access_and_a_contact_the_app_can_see() {
        let limited = "is.example.contacts-containers-limited";
        answer_prompt(limited, Choice::Picked([1u64].into_iter().collect())).unwrap();
        let mut containers = Containers::default();
        let add = obj(&[("name", text("Nope"))]);
        assert!(containers_answer(limited, "container_add", &add, &book(), &mut containers).is_err());
        let full = "is.example.contacts-containers-full";
        answer_prompt(full, Choice::All).unwrap();
        let work = containers.add("Work", contactcontainers::Kind::Account).unwrap();
        let ghost = obj(&[("container", Json::Num(work as f64)), ("contact", Json::Num(99.0))]);
        assert!(containers_answer(full, "container_assign", &ghost, &book(), &mut containers).is_err());
        let unknown = obj(&[("container", Json::Num(99.0)), ("contact", Json::Num(1.0))]);
        assert!(containers_answer(full, "container_assign", &unknown, &book(), &mut containers).is_err());
    }

    #[test]
    fn a_limited_grant_sees_only_its_contacts_in_a_container() {
        let id = "is.example.contacts-containers-picked";
        answer_prompt(id, Choice::Picked([1u64, 3].into_iter().collect())).unwrap();
        let mut containers = Containers::default();
        let ask = obj(&[("container", Json::Num(1.0))]);
        let (members, _, _) = containers_answer(id, "container_members", &ask, &book(), &mut containers).unwrap();
        assert_eq!(members.get("contacts"), Some(&Json::Arr(vec![Json::Num(1.0), Json::Num(3.0)])));
        assert!(containers_answer(id, "container_of", &obj(&[("contact", Json::Num(2.0))]), &book(), &mut containers).is_err());
    }

    fn labeled(items: &[(&str, &str)]) -> Json {
        Json::Arr(items.iter().map(|(l, v)| obj(&[("label", text(l)), ("value", text(v))])).collect())
    }

    #[test]
    fn a_save_stores_every_phone_and_the_extra_fields_and_a_fetch_returns_them() {
        let id = "is.example.contacts-save-extras";
        answer_prompt(id, Choice::All).unwrap();
        let add = obj(&[(
            "add",
            Json::Arr(vec![obj(&[
                ("given_name", text("Ada")),
                ("name_prefix", text("Dr.")),
                ("job_title", text("Analyst")),
                ("phones", labeled(&[("work", "+354 1"), ("home", "+354 2")])),
                ("relations", labeled(&[("spouse", "William")])),
                ("birthday", obj(&[("month", Json::Num(12.0)), ("day", Json::Num(10.0))])),
                ("dates", Json::Arr(vec![obj(&[("label", text("anniversary")), ("year", Json::Num(1835.0)), ("month", Json::Num(7.0)), ("day", Json::Num(8.0))])])),
                ("social_profiles", Json::Arr(vec![obj(&[("service", text("mastodon")), ("username", text("@ada")), ("url", text("https://example.social/@ada"))])])),
                ("instant_messages", Json::Arr(vec![obj(&[("label", text("home")), ("service", text("xmpp")), ("username", text("ada@chat.example"))])])),
            ])]),
        )]);
        let (_, next, extras) = apply_save_with(id, &add, &book(), &Book::new()).unwrap();
        let ada = next.iter().find(|c| c.id == 4).unwrap();
        assert_eq!(ada.phone, "+354 1");
        assert_eq!(LABELS[0][ada.labels[0] as usize], "work", "the first phone's label sets the file's label row");
        let e = &extras[&4];
        assert_eq!(e.phones, vec![LabeledValue::new("home", "+354 2")]);
        assert_eq!((e.name_prefix.as_str(), e.job_title.as_str()), ("Dr.", "Analyst"));
        assert_eq!(e.birthday, DateComponents::new(None, 12, 10));
        assert_eq!(e.instant_messages[0].url, "", "an instant message address has no URL");
        let reply = answer_with(id, "fetch", &keys(&["phones", "relations", "social_profiles", "job_title"]), &next, &extras).unwrap();
        let people = reply.get("contacts").and_then(Json::as_array).unwrap();
        let fetched = people.iter().find(|p| p.get("id").and_then(Json::as_u64) == Some(4)).unwrap();
        assert_eq!(fetched.get("phones").and_then(Json::as_array).map(|p| p.len()), Some(2));
        assert_eq!(fetched.get("relations").and_then(Json::as_array).and_then(|r| r[0].get("label")).and_then(Json::as_str), Some("spouse"));
        let social = &fetched.get("social_profiles").and_then(Json::as_array).unwrap()[0];
        assert_eq!(social.get("service").and_then(Json::as_str), Some("mastodon"));
        assert_eq!(social.get("identifier").and_then(Json::as_str), Some("1"));
        assert_eq!(fetched.get("job_title").and_then(Json::as_str), Some("Analyst"));
    }

    #[test]
    fn an_update_changes_only_the_extras_it_names_null_clears_one_and_a_delete_drops_them() {
        let id = "is.example.contacts-save-extras-update";
        answer_prompt(id, Choice::All).unwrap();
        let mut extras = Book::new();
        extras.insert(1, Extras { nickname: "Countess".into(), urls: vec![LabeledValue::new("home", "https://ada.example")], ..Default::default() });
        let update = obj(&[("update", Json::Arr(vec![obj(&[("id", Json::Num(1.0)), ("department", text("Engines")), ("nickname", Json::Null)])]))]);
        let (_, _, after) = apply_save_with(id, &update, &book(), &extras).unwrap();
        assert_eq!(after[&1].department, "Engines");
        assert_eq!(after[&1].nickname, "");
        assert_eq!(after[&1].urls.len(), 1, "a field the update does not name is kept");
        let delete = obj(&[("delete", Json::Arr(vec![obj(&[("id", Json::Num(1.0))])]))]);
        let (_, _, gone) = apply_save_with(id, &delete, &book(), &after).unwrap();
        assert!(!gone.contains_key(&1));
    }

    #[test]
    fn a_save_refuses_a_bad_date_a_bare_profile_and_an_overlong_extra() {
        let id = "is.example.contacts-save-extras-bad";
        answer_prompt(id, Choice::All).unwrap();
        let add = |field: &str, value: Json| obj(&[("add", Json::Arr(vec![obj(&[("given_name", text("Ada")), (field, value)])]))]);
        let bad_date = add("birthday", obj(&[("month", Json::Num(13.0)), ("day", Json::Num(1.0))]));
        assert!(apply_save_with(id, &bad_date, &[], &Book::new()).is_err());
        assert!(apply_save_with(id, &add("social_profiles", texts(&["@ada"])), &[], &Book::new()).is_err());
        let long = "x".repeat(contactextras::MAX_FIELD + 1);
        assert_eq!(
            apply_save_with(id, &add("nickname", text(&long)), &[], &Book::new()).err(),
            Some(format!("nickname is longer than {} characters", contactextras::MAX_FIELD))
        );
    }
}
