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
//! `request` reports the status and never grants it. An app that is undecided gets the access sheet queued, and the answer
//! comes from that sheet or from Settings. The
//! service is `springboard/src/shell/services/contacts.rs`, and the model is `bifrost_ui::contact_access` (every
//! contact, a picked set read-only, or none) with the records in `bifrost_ui::contact_fetch`.
pub use crate::controls::contact_access::{Choice, Error, Status};
pub use crate::controls::contact_book::{format_postal_address, PostalAddress};
pub use crate::controls::contact_fetch::{
    is_predefined_label, localized_label, DateComponents, Key, LabeledValue, Profile, Record, Value, LABEL_DATE_ANNIVERSARY,
    LABEL_HOME, LABEL_OTHER, LABEL_PHONE_HOME_FAX, LABEL_PHONE_MAIN, LABEL_PHONE_MOBILE, LABEL_PHONE_OTHER_FAX,
    LABEL_PHONE_PAGER, LABEL_PHONE_WORK_FAX, LABEL_RELATION_MANAGER, LABEL_SCHOOL, LABEL_URL_HOME_PAGE, LABEL_WORK,
    PREDEFINED_LABELS, RELATION_LABELS,
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

/// Asks for the current status. It never grants access itself. An app that has not been answered yet gets the access sheet
/// queued, so the person can answer it; the answer shows in later `request` and `access` replies, and the person can
/// also change it in Settings > Privacy & Security > App Permissions.
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
/// writes. Each record uses the fields `fetch` reads, in the same values: labeled phones, emails, URLs and relations
/// (as many as the contact has), the birthday and dates, the name prefix, nickname, job title and department, and
/// social profiles and instant message addresses. The address takes one value. An update changes only the fields its
/// record holds.
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
            Some(Value::Labeled(items)) => Json::Arr(
                items
                    .iter()
                    .map(|item| Json::Obj(vec![("label".into(), Json::Str(item.label.clone())), ("value".into(), Json::Str(item.value.clone()))]))
                    .collect(),
            ),
            Some(Value::Date(d)) => date_json(*d, Vec::new()),
            Some(Value::Dates(items)) => {
                Json::Arr(items.iter().map(|(label, d)| date_json(*d, vec![("label".into(), Json::Str(label.clone()))])).collect())
            }
            Some(Value::Profiles(items)) => Json::Arr(
                items
                    .iter()
                    .map(|p| {
                        Json::Obj(
                            [("label", &p.label), ("service", &p.service), ("username", &p.username), ("url", &p.url), ("user_identifier", &p.user_identifier)]
                                .into_iter()
                                .map(|(k, v)| (k.to_string(), Json::Str(v.clone())))
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            None => continue,
        };
        fields.push((key.name().to_string(), value));
    }
    Json::Obj(fields)
}

/// A date as `{year?, month, day}` after the `fields` given.
fn date_json(d: DateComponents, mut fields: Vec<(String, Json)>) -> Json {
    if let Some(y) = d.year {
        fields.push(("year".into(), Json::Num(y as f64)));
    }
    fields.push(("month".into(), Json::Num(d.month as f64)));
    fields.push(("day".into(), Json::Num(d.day as f64)));
    Json::Obj(fields)
}

/// The ids the shell gave the contacts a `save` added, in the order the request added them, or `None` for a reply of
/// another shape.
pub fn decode_saved(reply: &Json) -> Option<Vec<u64>> {
    reply.get("added")?.as_array()?.iter().map(Json::as_u64).collect()
}

/// The records in a `fetch` reply. A contact without a value for a key has no field for it, so `get` is `None`. A
/// reply of another shape, or one with a malformed record, is `None`.
/// The contacts whose value under `key` contains `text`, ignoring case and surrounding spaces (ABSearchElement on one
/// property). Text, list and labeled values are all searched; an empty `text` matches every contact that has a value
/// under `key`. The records must have been fetched with `key`, or they have no value to match.
pub fn matching<'a>(people: &'a [Record], key: Key, text: &str) -> Vec<&'a Record> {
    let needle = text.trim().to_lowercase();
    people
        .iter()
        .filter(|person| {
            let contains = |value: &str| value.to_lowercase().contains(&needle);
            match person.get(key) {
                Some(Value::Text(value)) => contains(value),
                Some(Value::List(items)) => items.iter().any(|item| contains(item)),
                Some(Value::Labeled(items)) => items.iter().any(|item| contains(&item.value)),
                _ => false,
            }
        })
        .collect()
}

pub fn decode_contacts(reply: &Json) -> Option<Vec<Record>> {
    reply.get("contacts")?.as_array()?.iter().map(decode_record).collect()
}

fn decode_record(v: &Json) -> Option<Record> {
    let mut record = Record::new(v.get("id")?.as_u64()?);
    for key in Key::ALL {
        let value = match (key, v.get(key.name())) {
            (Key::Phones | Key::Emails | Key::Urls | Key::Relations, Some(Json::Arr(items))) => Some(Value::Labeled(decode_labeled(items))),
            (Key::SocialProfiles | Key::InstantMessages, Some(Json::Arr(items))) => Some(Value::Profiles(decode_profiles(items))),
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

/// Profiles from `{label, service, username, url?, user_identifier?, identifier?}` objects; other entries are skipped.
/// Identifiers the reply lacks are numbered by position.
fn decode_profiles(items: &[Json]) -> Vec<Profile> {
    let text = |item: &Json, name: &str| item.get(name).and_then(Json::as_str).unwrap_or("").to_string();
    let entries: Vec<Profile> = items
        .iter()
        .filter(|item| matches!(item, Json::Obj(_)))
        .map(|item| Profile {
            label: text(item, "label"),
            service: text(item, "service"),
            username: text(item, "username"),
            url: text(item, "url"),
            user_identifier: text(item, "user_identifier"),
            identifier: text(item, "identifier"),
        })
        .collect();
    if entries.iter().any(|p| p.identifier.is_empty()) {
        Profile::numbered(entries)
    } else {
        entries
    }
}

fn decode_date(v: &Json) -> Option<DateComponents> {
    let part = |name: &str| v.get(name).and_then(Json::as_u64);
    DateComponents::new(part("year").map(|y| y as i32), u8::try_from(part("month")?).ok()?, u8::try_from(part("day")?).ok()?)
}

/// The reference error domain's raw codes (`CNErrorCode`), measured from the reference SDK's bridge metadata. An app
/// that reports or logs a contacts failure the way the reference framework does reads them through
/// [`ContactsError::code`].
pub mod error_code {
    pub const COMMUNICATION_ERROR: i64 = 1;
    pub const DATA_ACCESS_ERROR: i64 = 2;
    pub const AUTHORIZATION_DENIED: i64 = 100;
    pub const NO_ACCESSIBLE_WRITABLE_CONTAINERS: i64 = 101;
    pub const UNAUTHORIZED_KEYS: i64 = 102;
    pub const FEATURE_DISABLED_BY_USER: i64 = 103;
    pub const FEATURE_NOT_AVAILABLE: i64 = 104;
    pub const RECORD_DOES_NOT_EXIST: i64 = 200;
    pub const INSERTED_RECORD_ALREADY_EXISTS: i64 = 201;
    pub const CONTAINMENT_CYCLE: i64 = 202;
    pub const CONTAINMENT_SCOPE: i64 = 203;
    pub const PARENT_RECORD_DOES_NOT_EXIST: i64 = 204;
    pub const RECORD_IDENTIFIER_INVALID: i64 = 205;
    pub const RECORD_NOT_WRITABLE: i64 = 206;
    pub const PARENT_CONTAINER_NOT_WRITABLE: i64 = 207;
    pub const VALIDATION_MULTIPLE_ERRORS: i64 = 300;
    pub const VALIDATION_TYPE_MISMATCH: i64 = 301;
    pub const VALIDATION_CONFIGURATION_ERROR: i64 = 302;
    pub const PREDICATE_INVALID: i64 = 400;
    pub const POLICY_VIOLATION: i64 = 500;
    pub const CLIENT_IDENTIFIER_INVALID: i64 = 600;
    pub const CLIENT_IDENTIFIER_DOES_NOT_EXIST: i64 = 601;
    pub const CLIENT_IDENTIFIER_COLLISION: i64 = 602;
    pub const CHANGE_HISTORY_EXPIRED: i64 = 603;
    pub const CHANGE_HISTORY_INVALID_ANCHOR: i64 = 604;
    pub const CHANGE_HISTORY_INVALID_FETCH_REQUEST: i64 = 605;
    pub const VCARD_MALFORMED: i64 = 700;
    pub const VCARD_SUMMARIZATION_ERROR: i64 = 701;
}

/// Why a contacts call failed. The shell sends the reason as the error message; this maps the reasons it has a name for
/// and keeps any other message in `Other`. [`ContactsError::code`] gives the reference raw code of each variant.
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
    /// The call names a contact the book does not hold (or the app cannot see), with the shell's message.
    RecordNotFound(String),
    /// The text is not a readable vCard, with the shell's message.
    VCardMalformed(String),
    /// The change token is older than the history the shell keeps; read the whole book again.
    ChangeHistoryExpired,
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
            "the change token has expired; read the whole book again" => ContactsError::ChangeHistoryExpired,
            "no such contact" => ContactsError::RecordNotFound("no such contact".into()),
            "no vCard in the text" => ContactsError::VCardMalformed("no vCard in the text".into()),
            other if other.starts_with("not a readable vCard") => ContactsError::VCardMalformed(other.to_string()),
            other if other.starts_with("no contact ") && other["no contact ".len()..].chars().all(|c| c.is_ascii_digit()) => {
                ContactsError::RecordNotFound(other.to_string())
            }
            other if other.starts_with("a contact needs a name") => ContactsError::InvalidContact(other.to_string()),
            other if other.ends_with("is a list") || other.ends_with("cannot be saved yet") || other.ends_with("is not a contacts file field") || other.contains(" is longer than ") || other.ends_with("is not in the shape fetch returns") || other.ends_with("is text") || other.contains("takes one value per contact") || other.ends_with("is text or a list of text") || other.starts_with("a contact is an object") => {
                ContactsError::InvalidContact(other.to_string())
            }
            other => ContactsError::Other(other.to_string()),
        }
    }
}

impl ContactsError {
    /// The reference raw code (`CNErrorCode`) this failure stands for. The access-model failures (no grant, restricted,
    /// nothing picked, already decided) are all authorization denied, as the reference reports them; a read-only grant
    /// has no writable container; a contact that is not valid is a validation error; a store file that cannot be read
    /// is a data access error. A failure the shell has no name for reads as a data access error.
    pub fn code(&self) -> i64 {
        use error_code::*;
        match self {
            ContactsError::NotGranted | ContactsError::Restricted | ContactsError::NothingPicked | ContactsError::AlreadyDecided => {
                AUTHORIZATION_DENIED
            }
            ContactsError::ReadOnly => NO_ACCESSIBLE_WRITABLE_CONTAINERS,
            ContactsError::NothingToSave => VALIDATION_CONFIGURATION_ERROR,
            ContactsError::MissingId => RECORD_IDENTIFIER_INVALID,
            ContactsError::InvalidContact(_) => VALIDATION_MULTIPLE_ERRORS,
            ContactsError::FileTooLarge | ContactsError::UnknownFormat | ContactsError::Other(_) => DATA_ACCESS_ERROR,
            ContactsError::RecordNotFound(_) => RECORD_DOES_NOT_EXIST,
            ContactsError::VCardMalformed(_) => VCARD_MALFORMED,
            ContactsError::ChangeHistoryExpired => CHANGE_HISTORY_EXPIRED,
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

/// Groups of contacts (CNGroup): nested groups and membership, with a recursive member listing. The shell keeps the
/// groups beside the contact store, so they persist and are shared; the calls below use it. `Groups` is the same model
/// the shell uses, for an app that wants to check a change before it sends it.
pub mod groups {
    use super::*;
    pub use bifrost_ui::contact_groups::{Error, Group, Groups};

    fn call_with(ctx: &Ctx, method: &str, pairs: &[(&str, u64)]) -> u32 {
        let params = pairs.iter().map(|(k, v)| (k.to_string(), Json::Num(*v as f64))).collect();
        service::call(ctx, SERVICE, method, &Json::Obj(params))
    }

    /// Lists the groups the shell keeps, as the app sees them. The shell keeps the groups, so they persist and are
    /// shared by every app with a read grant. Decodes with [`decode_list`].
    pub fn list(ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "groups", &Json::Null)
    }

    /// Adds a group at the top level, or under `parent`. Needs full access. The reply holds the new `id`.
    pub fn add(ctx: &Ctx, name: &str, parent: Option<u64>) -> u32 {
        let mut params = vec![("name".to_string(), Json::Str(name.into()))];
        if let Some(parent) = parent {
            params.push(("parent".to_string(), Json::Num(parent as f64)));
        }
        service::call(ctx, SERVICE, "group_add", &Json::Obj(params))
    }

    /// Removes a group and its subgroups. Needs full access.
    pub fn remove(ctx: &Ctx, group: u64) -> u32 {
        call_with(ctx, "group_remove", &[("group", group)])
    }

    /// Puts a contact in a group. Needs full access and a contact the app can see.
    pub fn add_member(ctx: &Ctx, group: u64, contact: u64) -> u32 {
        call_with(ctx, "group_add_member", &[("group", group), ("contact", contact)])
    }

    /// Takes a contact out of a group. Needs full access.
    pub fn remove_member(ctx: &Ctx, group: u64, contact: u64) -> u32 {
        call_with(ctx, "group_remove_member", &[("group", group), ("contact", contact)])
    }

    /// The contacts in a group, with subgroups' members when `recursive`. Decodes with [`decode_members`].
    pub fn members(ctx: &Ctx, group: u64, recursive: bool) -> u32 {
        let params = Json::Obj(vec![
            ("group".into(), Json::Num(group as f64)),
            ("recursive".into(), Json::Bool(recursive)),
        ]);
        service::call(ctx, SERVICE, "group_members", &params)
    }

    /// The groups in a `groups` reply as `(id, name, parent)`, or `None` for a reply of another shape.
    pub fn decode_list(reply: &Json) -> Option<Vec<(u64, String, Option<u64>)>> {
        reply
            .get("groups")?
            .as_array()?
            .iter()
            .map(|g| Some((g.get("id")?.as_u64()?, g.get("name")?.as_str()?.to_string(), g.get("parent").and_then(Json::as_u64))))
            .collect()
    }

    /// The contact ids in a `group_members` reply.
    pub fn decode_members(reply: &Json) -> Option<Vec<u64>> {
        reply.get("contacts")?.as_array()?.iter().map(Json::as_u64).collect()
    }
}

/// Containers (CNContainer): the places a contact lives, such as the local book or a synced account. Every contact is in
/// exactly one container, the local one (id 1, which always exists and cannot be removed) unless it was assigned
/// elsewhere. Removing a container moves its contacts back to the local one and deletes none. The shell keeps the
/// containers beside the contact store, so they persist and are shared by every app with a read grant; writes need
/// full access.
pub mod containers {
    use super::*;

    /// The id of the local container.
    pub const LOCAL: u64 = 1;

    fn call_with(ctx: &Ctx, method: &str, pairs: &[(&str, u64)]) -> u32 {
        let params = pairs.iter().map(|(k, v)| (k.to_string(), Json::Num(*v as f64))).collect();
        service::call(ctx, SERVICE, method, &Json::Obj(params))
    }

    /// What kind of place a container is.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Local,
        Account,
        Other,
    }

    impl Kind {
        /// The wire name.
        pub fn name(self) -> &'static str {
            match self {
                Kind::Local => "local",
                Kind::Account => "account",
                Kind::Other => "other",
            }
        }

        pub fn from_name(name: &str) -> Option<Kind> {
            match name {
                "local" => Some(Kind::Local),
                "account" => Some(Kind::Account),
                "other" => Some(Kind::Other),
                _ => None,
            }
        }
    }

    /// Lists the containers. Decodes with [`decode_list`].
    pub fn list(ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "containers", &Json::Null)
    }

    /// The contacts the app can read that live in `container`. Decodes with [`decode_members`].
    pub fn members(ctx: &Ctx, container: u64) -> u32 {
        call_with(ctx, "container_members", &[("container", container)])
    }

    /// The container one contact lives in. Decodes with [`decode_container`].
    pub fn of_contact(ctx: &Ctx, contact: u64) -> u32 {
        call_with(ctx, "container_of", &[("contact", contact)])
    }

    /// Adds a container (`Kind::Local` is refused: the book has one). Needs full access. The reply holds the new `id`.
    pub fn add(ctx: &Ctx, name: &str, kind: Kind) -> u32 {
        let params = Json::Obj(vec![("name".into(), Json::Str(name.into())), ("kind".into(), Json::Str(kind.name().into()))]);
        service::call(ctx, SERVICE, "container_add", &params)
    }

    /// Renames a container. Needs full access.
    pub fn rename(ctx: &Ctx, container: u64, name: &str) -> u32 {
        let params = Json::Obj(vec![("container".into(), Json::Num(container as f64)), ("name".into(), Json::Str(name.into()))]);
        service::call(ctx, SERVICE, "container_rename", &params)
    }

    /// Removes a container; its contacts go back to the local one. Needs full access.
    pub fn remove(ctx: &Ctx, container: u64) -> u32 {
        call_with(ctx, "container_remove", &[("container", container)])
    }

    /// Moves a contact to a container. Needs full access and a contact the app can read.
    pub fn assign(ctx: &Ctx, container: u64, contact: u64) -> u32 {
        call_with(ctx, "container_assign", &[("container", container), ("contact", contact)])
    }

    /// The containers in a `containers` reply as `(id, name, kind)`, or `None` for a reply of another shape.
    pub fn decode_list(reply: &Json) -> Option<Vec<(u64, String, Kind)>> {
        reply
            .get("containers")?
            .as_array()?
            .iter()
            .map(|c| Some((c.get("id")?.as_u64()?, c.get("name")?.as_str()?.to_string(), Kind::from_name(c.get("kind")?.as_str()?)?)))
            .collect()
    }

    /// The contact ids in a `container_members` reply.
    pub fn decode_members(reply: &Json) -> Option<Vec<u64>> {
        reply.get("contacts")?.as_array()?.iter().map(Json::as_u64).collect()
    }

    /// The container id in a `container_of` reply.
    pub fn decode_container(reply: &Json) -> Option<u64> {
        reply.get("container")?.as_u64()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_list_reply_decodes_with_kinds_and_an_unknown_kind_is_refused() {
            let row = |id: f64, name: &str, kind: &str| {
                Json::Obj(vec![("id".into(), Json::Num(id)), ("name".into(), Json::Str(name.into())), ("kind".into(), Json::Str(kind.into()))])
            };
            let reply = Json::Obj(vec![("containers".into(), Json::Arr(vec![row(1.0, "On My Computer", "local"), row(2.0, "Work", "account")]))]);
            let got = decode_list(&reply).unwrap();
            assert_eq!(got[0], (LOCAL, "On My Computer".to_string(), Kind::Local));
            assert_eq!(got[1].2, Kind::Account);
            let odd = Json::Obj(vec![("containers".into(), Json::Arr(vec![row(3.0, "X", "cloud")]))]);
            assert_eq!(decode_list(&odd), None);
        }

        #[test]
        fn member_and_container_replies_decode() {
            let members = Json::Obj(vec![("contacts".into(), Json::Arr(vec![Json::Num(2.0), Json::Num(5.0)]))]);
            assert_eq!(decode_members(&members), Some(vec![2, 5]));
            assert_eq!(decode_container(&Json::Obj(vec![("container".into(), Json::Num(2.0))])), Some(2));
            assert_eq!(decode_container(&Json::Null), None);
        }
    }
}

/// Change history for contacts (CNChangeHistoryFetchRequest). The shell logs each applied save as numbered events, and
/// [`history::changes`] returns the events after a token. `Store` is the in-process model of the same log, for an app
/// that keeps its own batches; the shell posts `system.contacts.change` when the contact file changes.
pub mod history {
    use super::*;
    pub use bifrost_ui::contact_changes::{Change, Error, Event, Observers, Op, Store};

    /// The changes to contacts after `token` (0 for all the kept history), as the shell logged them. The reply holds the
    /// token to keep for the next call and the changes the app may read. Decodes with [`decode_changes`]; an expired
    /// token is an error, and the app reads the whole book again.
    pub fn changes(ctx: &Ctx, token: u64) -> u32 {
        service::call(ctx, SERVICE, "changes", &Json::Obj(vec![("token".into(), Json::Num(token as f64))]))
    }

    /// The next token and the `(seq, kind, id)` changes in a `changes` reply, where kind is `added`, `updated` or
    /// `deleted`.
    pub fn decode_changes(reply: &Json) -> Option<(u64, Vec<(u64, String, u64)>)> {
        let token = reply.get("token")?.as_u64()?;
        let rows = reply
            .get("changes")?
            .as_array()?
            .iter()
            .map(|c| Some((c.get("seq")?.as_u64()?, c.get("kind")?.as_str()?.to_string(), c.get("id")?.as_u64()?)))
            .collect::<Option<Vec<_>>>()?;
        Some((token, rows))
    }
}

/// Contact pictures (CNContact image and thumbnail data): PNG or JPEG only, detected from the first bytes, capped at
/// `photos::MAX_BYTES`. `Photos::set` replaces a contact's picture and keeps the old one when the new data is refused;
/// `fit_size` works out a thumbnail size inside a square. Notes are plain text on the contact record.
pub mod photos {
    use super::*;
    pub use bifrost_ui::contact_photo::{fit_size, format_of, Error, Format, Photos, MAX_BYTES};

    /// Asks the shell for a contact's picture. Decodes with [`decode`].
    pub fn get(ctx: &Ctx, contact: u64) -> u32 {
        service::call(ctx, SERVICE, "photo", &Json::Obj(vec![("id".into(), Json::Num(contact as f64))]))
    }

    /// Sets a contact's picture in the shell (PNG or JPEG, within `MAX_BYTES`). Needs full access. The shell refuses
    /// other data and keeps the old picture.
    pub fn set(ctx: &Ctx, contact: u64, data: &[u8]) -> u32 {
        let params = Json::Obj(vec![
            ("id".into(), Json::Num(contact as f64)),
            ("data".into(), Json::Str(bifrost_ui::contact_vcard::base64_encode(data))),
        ]);
        service::call(ctx, SERVICE, "photo_set", &params)
    }

    /// Removes a contact's picture in the shell. Needs full access.
    pub fn remove(ctx: &Ctx, contact: u64) -> u32 {
        service::call(ctx, SERVICE, "photo_remove", &Json::Obj(vec![("id".into(), Json::Num(contact as f64))]))
    }

    /// The format and bytes in a `photo` reply, or `None` when the contact has no picture or the reply is another shape.
    pub fn decode(reply: &Json) -> Option<(Format, Vec<u8>)> {
        let format = match reply.get("format")?.as_str()? {
            "png" => Format::Png,
            "jpeg" => Format::Jpeg,
            _ => return None,
        };
        let bytes = bifrost_ui::contact_vcard::base64_decode(reply.get("data")?.as_str()?)?;
        Some((format, bytes))
    }
}

/// Contact display preferences (CNContactsUserDefaults and CNContactFormatter sort order): the sort order a list uses,
/// the country code a phone number is written with, and the sort helpers. `ContactsDefaults` keeps the user's choice;
/// `effective_sort_order` resolves "system default" against the system's order.
pub mod display {
    pub use bifrost_ui::contact_display::{
        compare_names, full_name, name_order_for_language, normalize_country_code, sort_contacts, sort_key, ContactsDefaults,
        DisplayNameOrder, SortOrder,
    };
    /// Postal addresses laid out for their country (CNPostalAddressFormatter with the address's ISO country code).
    pub use bifrost_ui::contact_address::{lines, lines_for_country, single_line, Style as AddressStyle};
}

/// The contact picker (ContactsUI, CNContactPicker and the people picker): the user chooses one whole contact, or one
/// value of a contact such as a phone number. The app gets only the final [`picker::Choice`], so it needs no read
/// access to the store. `Picker::new` takes the entries the shell shows, `select` picks or opens a contact, and
/// `choose` picks one value of the open contact. The shell draws the list from `visible` and the detail from `detail`.
pub mod picker {
    use super::*;
    pub use bifrost_ui::contact_card::{Entry, Field, Key};
    pub use bifrost_ui::contact_picker::{Choice, Error, Mode, Picker};

    /// Starts a pick in the shell. `mode` is `"contact"` (one whole contact, with `need` naming a field it must have)
    /// or `"property"` (one value under `keys`, such as `"phone"` or `"email"`). The person chooses in the shell's
    /// picker sheet; read the result with [`result`]. Nothing about the book is sent to the app.
    pub fn start(ctx: &Ctx, mode: &str, need: Option<&str>, keys: &[&str]) -> u32 {
        let mut params = vec![
            ("mode".to_string(), Json::Str(mode.into())),
            ("keys".to_string(), Json::Arr(keys.iter().map(|k| Json::Str((*k).into())).collect())),
        ];
        if let Some(need) = need {
            params.push(("need".to_string(), Json::Str(need.into())));
        }
        service::call(ctx, SERVICE, "pick", &Json::Obj(params))
    }

    /// Asks for the result of the app's pick. Decodes with [`decode_result`].
    pub fn result(ctx: &Ctx) -> u32 {
        service::call(ctx, SERVICE, "pick_result", &Json::Null)
    }

    /// What a `pick_result` reply says: still open, or the chosen contact and, for a property pick, its value.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum PickResult {
        Pending,
        Chosen { contact: u64, value: Option<String> },
    }

    /// The pick state in a `pick_result` reply, or `None` for a reply of another shape.
    pub fn decode_result(reply: &Json) -> Option<PickResult> {
        match reply.get("status")?.as_str()? {
            "pending" => Some(PickResult::Pending),
            "chosen" => Some(PickResult::Chosen {
                contact: reply.get("contact")?.as_u64()?,
                value: reply.get("value").and_then(Json::as_str).map(str::to_string),
            }),
            _ => None,
        }
    }
}

/// vCard 3.0 and 4.0 for the full contact model (`bifrost_ui::contact_vcard`, the same code the Contacts app uses for
/// its import and export). `vcard::serialize` writes contacts as CRLF-terminated cards, and `vcard::contacts` reads
/// every card back, skipping properties the model has no field for. Text over `vcard::MAX_BYTES` is refused, and
/// `vcard::Error` names the failure. An app builds its own [`Contact`] values, so it never needs the store file.
pub mod vcard {
    pub use bifrost_ui::contact_book::{label, Contact, LabeledValue};
    pub use bifrost_ui::contact_vcard::{contacts, serialize, Version};
    pub use bifrost_ui::vcard::{Error, MAX_BYTES, MAX_CARDS};

    use crate::runtime::Ctx;
    use crate::service;
    use bifrost_core::json::Json;

    /// Asks the shell for the contacts the app may read as vCard text. `ids` narrows it to those contacts (an id the app
    /// cannot read is left out), `version` is 3.0 or 4.0. Needs a read grant; a limited grant exports only its contacts.
    /// The reply decodes with [`decode_export`].
    pub fn export(ctx: &Ctx, ids: Option<&[u64]>, version: Version) -> u32 {
        service::call(ctx, super::SERVICE, "export_vcard", &export_params(ids, version))
    }

    /// The params [`export`] sends.
    pub fn export_params(ids: Option<&[u64]>, version: Version) -> Json {
        let mut params = vec![(
            "version".to_string(),
            Json::Str(match version {
                Version::V3 => "3.0".into(),
                Version::V4 => "4.0".into(),
            }),
        )];
        if let Some(ids) = ids {
            params.push(("ids".to_string(), Json::Arr(ids.iter().map(|id| Json::Num(*id as f64)).collect())));
        }
        Json::Obj(params)
    }

    /// The vCard text and the number of contacts in it, from an `export` reply.
    pub fn decode_export(reply: &Json) -> Option<(String, usize)> {
        Some((reply.get("vcard")?.as_str()?.to_string(), reply.get("count")?.as_u64()? as usize))
    }

    /// Asks the shell to add the people in `text` to the book. Needs full access; the whole text applies or none of it
    /// does. A person the book already holds is not added again. The reply decodes with [`decode_import`].
    pub fn import(ctx: &Ctx, text: &str) -> u32 {
        service::call(ctx, super::SERVICE, "import_vcard", &Json::Obj(vec![("vcard".into(), Json::Str(text.into()))]))
    }

    /// What an import did: the ids the new contacts got, the cards the store cannot hold, and the cards of people the
    /// book already held.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Imported {
        pub added: Vec<u64>,
        pub skipped: usize,
        pub duplicates: usize,
    }

    /// The outcome in an `import` reply, or `None` for a reply of another shape.
    pub fn decode_import(reply: &Json) -> Option<Imported> {
        Some(Imported {
            added: reply.get("added")?.as_array()?.iter().map(Json::as_u64).collect::<Option<Vec<_>>>()?,
            skipped: reply.get("skipped")?.as_u64()? as usize,
            duplicates: reply.get("duplicates")?.as_u64()? as usize,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_export_params_name_the_version_and_only_the_asked_ids() {
            let all = export_params(None, Version::V3);
            assert_eq!(all.get("version").and_then(Json::as_str), Some("3.0"));
            assert!(all.get("ids").is_none());
            let some = export_params(Some(&[2, 5]), Version::V4);
            assert_eq!(some.get("version").and_then(Json::as_str), Some("4.0"));
            assert_eq!(some.get("ids"), Some(&Json::Arr(vec![Json::Num(2.0), Json::Num(5.0)])));
        }

        #[test]
        fn replies_decode_and_other_shapes_do_not() {
            let reply = Json::Obj(vec![("vcard".into(), Json::Str("BEGIN:VCARD".into())), ("count".into(), Json::Num(1.0))]);
            assert_eq!(decode_export(&reply), Some(("BEGIN:VCARD".to_string(), 1)));
            assert_eq!(decode_export(&Json::Null), None);
            let done = Json::Obj(vec![
                ("added".into(), Json::Arr(vec![Json::Num(4.0)])),
                ("skipped".into(), Json::Num(1.0)),
                ("duplicates".into(), Json::Num(2.0)),
            ]);
            assert_eq!(decode_import(&done), Some(Imported { added: vec![4], skipped: 1, duplicates: 2 }));
            assert_eq!(decode_import(&Json::Null), None);
        }
    }
}

/// People apps saw and donated, and the person's answers (`bifrost_ui::contact_donation`). An app that sees someone who
/// is not in the book, such as the sender of a message, calls [`suggested::donate`] with the name and one email address
/// or phone number. The shell keeps the donations on the machine and offers them back to the Contacts app only, as
/// suggested contacts: [`suggested::list`] reads them, [`suggested::answer`] records accept or dismiss. A dismissed
/// address is never offered again, even if an app donates it again. Donating needs no contacts access: it gives the
/// book nothing and reads nothing.
pub mod suggested {
    use super::SERVICE;
    pub use crate::controls::contact_donation::{Feedback, Handle};
    use crate::runtime::Ctx;
    use crate::service;
    use bifrost_core::json::Json;

    /// Donates a person the app saw. `name` may be empty. The reply decodes with [`decode_donated`].
    pub fn donate(ctx: &Ctx, name: &str, handle: Handle, value: &str) -> u32 {
        service::call(ctx, SERVICE, "donate", &donate_params(name, handle, value))
    }

    /// The parameters of a `donate` call.
    pub fn donate_params(name: &str, handle: Handle, value: &str) -> Json {
        Json::Obj(vec![
            ("name".into(), Json::Str(name.into())),
            ("handle".into(), Json::Str(if handle == Handle::Email { "email" } else { "phone" }.into())),
            ("value".into(), Json::Str(value.into())),
        ])
    }

    /// Whether the shell kept a `donate` (false for an address the person dismissed before), or `None` for a reply of
    /// another shape.
    pub fn decode_donated(reply: &Json) -> Option<bool> {
        match reply.get("donated")? {
            Json::Bool(kept) => Some(*kept),
            _ => None,
        }
    }

    /// One suggested person, as the Contacts app lists them.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Suggested {
        /// The handle in its normalised form; the key `answer` takes.
        pub key: String,
        pub handle: Handle,
        /// The email address or phone number as first donated.
        pub value: String,
        /// The latest name donated; empty when none came with it.
        pub name: String,
        /// How many apps donated the person.
        pub sources: u32,
    }

    /// Asks for up to `limit` suggested people, best first. Only the Contacts app may. The reply decodes with
    /// [`decode_list`].
    pub fn list(ctx: &Ctx, limit: u32) -> u32 {
        service::call(ctx, SERVICE, "suggestions", &Json::Obj(vec![("limit".into(), Json::Num(f64::from(limit)))]))
    }

    /// The people in a `suggestions` reply, or `None` for a reply of another shape.
    pub fn decode_list(reply: &Json) -> Option<Vec<Suggested>> {
        reply
            .get("suggestions")?
            .as_array()?
            .iter()
            .map(|row| {
                Some(Suggested {
                    key: row.get("key")?.as_str()?.to_string(),
                    handle: if row.get("handle")?.as_str()? == "email" { Handle::Email } else { Handle::Phone },
                    value: row.get("value")?.as_str()?.to_string(),
                    name: row.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                    sources: row.get("sources").and_then(Json::as_u64).unwrap_or(1) as u32,
                })
            })
            .collect()
    }

    /// The person's answer to the suggestion with `key`. Only the Contacts app may. Accepting replies with the contact
    /// to save ([`decode_accepted`]); dismissing blocks the address.
    pub fn answer(ctx: &Ctx, key: &str, answer: Feedback) -> u32 {
        let name = if answer == Feedback::Accepted { "accepted" } else { "dismissed" };
        service::call(ctx, SERVICE, "feedback", &Json::Obj(vec![("key".into(), Json::Str(key.into())), ("answer".into(), Json::Str(name.into()))]))
    }

    /// The contact an accepted suggestion becomes: its names and the email address or phone number.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Accepted {
        pub given_name: String,
        pub family_name: String,
        pub email: Option<String>,
        pub phone: Option<String>,
    }

    /// The contact in an accepted `feedback` reply, or `None` for any other reply.
    pub fn decode_accepted(reply: &Json) -> Option<Accepted> {
        let c = reply.get("contact")?;
        Some(Accepted {
            given_name: c.get("given_name").and_then(Json::as_str).unwrap_or("").to_string(),
            family_name: c.get("family_name").and_then(Json::as_str).unwrap_or("").to_string(),
            email: c.get("email").and_then(Json::as_str).map(str::to_string),
            phone: c.get("phone").and_then(Json::as_str).map(str::to_string),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn row(key: &str, name: &str) -> Json {
            Json::Obj(vec![
                ("key".into(), Json::Str(key.into())),
                ("handle".into(), Json::Str("email".into())),
                ("value".into(), Json::Str(key.into())),
                ("name".into(), Json::Str(name.into())),
                ("sources".into(), Json::Num(2.0)),
            ])
        }

        #[test]
        fn a_donation_names_its_handle_and_a_suggestion_list_decodes() {
            let params = donate_params("Ada", Handle::Phone, "+354 555 0100");
            assert_eq!(params.get("handle").and_then(Json::as_str), Some("phone"));
            assert_eq!(decode_donated(&Json::Obj(vec![("donated".into(), Json::Bool(false))])), Some(false));
            let reply = Json::Obj(vec![("suggestions".into(), Json::Arr(vec![row("ada@example.com", "Ada"), row("x@example.com", "")]))]);
            let list = decode_list(&reply).unwrap();
            assert_eq!((list.len(), list[0].key.as_str(), list[0].name.as_str(), list[0].sources), (2, "ada@example.com", "Ada", 2));
            assert_eq!(list[1].name, "");
            assert!(decode_list(&Json::Null).is_none());
        }

        #[test]
        fn an_accepted_reply_decodes_to_the_contact_and_a_dismissal_to_none() {
            let reply = Json::Obj(vec![(
                "contact".into(),
                Json::Obj(vec![
                    ("given_name".into(), Json::Str("Ada".into())),
                    ("family_name".into(), Json::Str("Lovelace".into())),
                    ("email".into(), Json::Str("ada@example.com".into())),
                    ("phone".into(), Json::Null),
                ]),
            )]);
            let accepted = decode_accepted(&reply).unwrap();
            assert_eq!((accepted.family_name.as_str(), accepted.email.as_deref(), accepted.phone), ("Lovelace", Some("ada@example.com"), None));
            assert!(decode_accepted(&Json::Obj(Vec::new())).is_none());
        }
    }
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
    fn a_search_matches_one_property_ignoring_case() {
        let mut ada = Record::new(1);
        ada.set(Key::GivenName, Value::Text("Ada".into()));
        ada.set(Key::Emails, Value::List(vec!["ada@example.com".into()]));
        let mut bob = Record::new(2);
        bob.set(Key::GivenName, Value::Text("Bob".into()));
        let people = vec![ada, bob];
        let ids = |found: Vec<&Record>| found.iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(matching(&people, Key::GivenName, " ADA ")), vec![1]);
        assert_eq!(ids(matching(&people, Key::Emails, "example")), vec![1]);
        assert_eq!(ids(matching(&people, Key::Emails, "")), vec![1]);
    }

    #[test]
    fn a_refused_picture_keeps_the_contacts_old_picture() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        let mut photos = photos::Photos::new();
        assert_eq!(photos.set(4, &png), Ok(photos::Format::Png));
        assert_eq!(photos.set(4, b"not a picture"), Err(photos::Error::UnknownFormat));
        assert_eq!(photos.data(4), Some(&png[..]));
        assert_eq!(photos.format(4), Some(photos::Format::Png));
        assert_eq!(photos::fit_size(400, 200, 100), (100, 50));
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
    fn shell_replies_decode_into_the_sdk_types() {
        let groups = Json::Obj(vec![(
            "groups".into(),
            Json::Arr(vec![
                Json::Obj(vec![("id".into(), Json::Num(1.0)), ("name".into(), Json::Str("Family".into())), ("parent".into(), Json::Null)]),
                Json::Obj(vec![("id".into(), Json::Num(2.0)), ("name".into(), Json::Str("Kin".into())), ("parent".into(), Json::Num(1.0))]),
            ]),
        )]);
        assert_eq!(
            groups::decode_list(&groups),
            Some(vec![(1, "Family".to_string(), None), (2, "Kin".to_string(), Some(1))])
        );

        let changes = Json::Obj(vec![
            ("token".into(), Json::Num(2.0)),
            (
                "changes".into(),
                Json::Arr(vec![Json::Obj(vec![
                    ("seq".into(), Json::Num(2.0)),
                    ("kind".into(), Json::Str("deleted".into())),
                    ("id".into(), Json::Num(5.0)),
                ])]),
            ),
        ]);
        assert_eq!(history::decode_changes(&changes), Some((2, vec![(2, "deleted".to_string(), 5)])));

        let pending = Json::Obj(vec![("status".into(), Json::Str("pending".into()))]);
        assert_eq!(picker::decode_result(&pending), Some(picker::PickResult::Pending));
        let chosen = Json::Obj(vec![
            ("status".into(), Json::Str("chosen".into())),
            ("contact".into(), Json::Num(4.0)),
            ("value".into(), Json::Str("ada@example.com".into())),
        ]);
        assert_eq!(
            picker::decode_result(&chosen),
            Some(picker::PickResult::Chosen { contact: 4, value: Some("ada@example.com".into()) })
        );

        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 9];
        let photo = Json::Obj(vec![
            ("format".into(), Json::Str("png".into())),
            ("data".into(), Json::Str(bifrost_ui::contact_vcard::base64_encode(&png))),
        ]);
        assert_eq!(photos::decode(&photo), Some((photos::Format::Png, png.to_vec())));
        assert_eq!(photos::decode(&Json::Obj(vec![("format".into(), Json::Null)])), None);
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

    #[test]
    fn a_save_request_sends_labeled_values_dates_and_profiles_in_the_fetch_shapes() {
        let mut ada = Record::new(0);
        ada.set(Key::GivenName, Value::Text("Ada".into()));
        ada.set(Key::Phones, Value::Labeled(vec![LabeledValue::new("work", "1"), LabeledValue::new("home", "2")]));
        ada.set(Key::Birthday, Value::Date(DateComponents::new(None, 12, 10).unwrap()));
        ada.set(Key::SocialProfiles, Value::Profiles(vec![Profile::new("", "mastodon", "@ada")]));
        let json = SaveRequest { add: vec![ada], ..Default::default() }.to_json();
        let add = &json.get("add").and_then(Json::as_array).unwrap()[0];
        let phones = add.get("phones").and_then(Json::as_array).unwrap();
        assert_eq!(phones.len(), 2);
        assert_eq!(phones[1].get("label").and_then(Json::as_str), Some("home"));
        assert_eq!(add.get("birthday").and_then(|b| b.get("year")), None);
        assert_eq!(add.get("birthday").and_then(|b| b.get("month")).and_then(Json::as_u64), Some(12));
        let social = &add.get("social_profiles").and_then(Json::as_array).unwrap()[0];
        assert_eq!(social.get("service").and_then(Json::as_str), Some("mastodon"));
        // what the request sends decodes back to the same values
        let mut sent = add.clone();
        if let Json::Obj(fields) = &mut sent {
            fields.push(("id".into(), Json::Num(9.0)));
        }
        let back = decode_contacts(&reply(vec![sent])).unwrap();
        assert_eq!(back[0].get(Key::Birthday), Some(&Value::Date(DateComponents::new(None, 12, 10).unwrap())));
        let Some(Value::Profiles(profiles)) = back[0].get(Key::SocialProfiles) else { panic!("profiles") };
        assert_eq!((profiles[0].username.as_str(), profiles[0].identifier.as_str()), ("@ada", "1"));
    }

    #[test]
    fn relations_and_instant_messages_decode_as_labeled_values_and_profiles() {
        let row = Json::Obj(vec![
            ("id".into(), Json::Num(1.0)),
            ("relations".into(), Json::Arr(vec![Json::Obj(vec![("label".into(), Json::Str("spouse".into())), ("value".into(), Json::Str("William".into()))])])),
            ("instant_messages".into(), Json::Arr(vec![Json::Obj(vec![("service".into(), Json::Str("xmpp".into())), ("username".into(), Json::Str("ada@chat.example".into()))])])),
            ("nickname".into(), Json::Str("Countess".into())),
        ]);
        let people = decode_contacts(&reply(vec![row])).unwrap();
        let Some(Value::Labeled(relations)) = people[0].get(Key::Relations) else { panic!("relations") };
        assert_eq!((relations[0].label.as_str(), relations[0].value.as_str()), ("spouse", "William"));
        assert!(matches!(people[0].get(Key::InstantMessages), Some(Value::Profiles(p)) if p[0].service == "xmpp"));
        assert_eq!(people[0].get(Key::Nickname), Some(&Value::Text("Countess".into())));
        assert_eq!(localized_label(LABEL_PHONE_PAGER), "Pager");
    }

    #[test]
    fn the_shell_messages_for_sidecar_fields_map_to_an_invalid_contact() {
        assert!(matches!(ContactsError::from_message("nickname is longer than 320 characters"), ContactsError::InvalidContact(_)));
        assert!(matches!(ContactsError::from_message("birthday is not in the shape fetch returns"), ContactsError::InvalidContact(_)));
    }

    #[test]
    fn each_error_has_its_reference_code_and_the_new_messages_are_named() {
        assert_eq!(ContactsError::NotGranted.code(), 100);
        assert_eq!(ContactsError::Restricted.code(), error_code::AUTHORIZATION_DENIED);
        assert_eq!(ContactsError::ReadOnly.code(), 101);
        assert_eq!(ContactsError::MissingId.code(), 205);
        assert_eq!(ContactsError::from_message("no contact 9"), ContactsError::RecordNotFound("no contact 9".into()));
        assert_eq!(ContactsError::from_message("no contact 9").code(), 200);
        assert!(matches!(ContactsError::from_message("no contact field shoe_size"), ContactsError::Other(_)));
        assert_eq!(ContactsError::from_message("the change token has expired; read the whole book again").code(), 603);
        assert_eq!(ContactsError::from_message("no vCard in the text").code(), error_code::VCARD_MALFORMED);
        assert_eq!(ContactsError::from_message("disk on fire").code(), error_code::DATA_ACCESS_ERROR);
    }
}
