//! The `privacy` service: each installed app's capture permissions (camera, microphone, screen recording), answered
//! from the pure model in `bifrost_ui::capture_access`. Apps call `status` and `can_capture`; the person changes the
//! answers in Settings > Privacy & Security > App Permissions (see `pages::permissions_page`).
//!
//! The store is per shell thread: every service call and every Settings change runs on the shell thread, so a
//! thread-local map is the one copy of the state. Nothing is written to disk yet, so the answers reset when the
//! shell restarts (see docs/parity/wire-svc-privacy.md, gaps).
//!
//! Methods (params are a JSON object):
//! - `status {media}` returns `{media, status}`, where status is `not_determined`, `restricted`, `denied` or `authorized`.
//! - `can_capture {media}` returns `{media, allowed}`, true only when the person authorized the media type.
//! - `all {}` returns `{camera, microphone, screen}` with each status.
//! - `location_status {}` returns `{status, authorized}`, where status is `not_determined`, `restricted`, `denied`,
//!   `authorized_always` or `authorized_when_in_use` (the model in `bifrost_ui::location_auth`).
//! - `location_request {}` returns `{outcome}`: `prompt` when the person has not answered, `already_granted`, or
//!   `refused`. A `prompt` queues the Allow / Don't Allow sheet (see `shell/permission_sheet.rs`).
//! - `request {media}` returns `{media, status}`, the current status. When the media type is `not_determined` it
//!   queues the same sheet; the answer shows up in later `status` calls.
//!
//! The sheet's answer is written with `decide`, and the Settings switches stay the other way to change it.
//!
//! `media` is `camera`, `microphone` or `screen`. An unknown media name or method is an error, never a panic.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::account_policy::Feature;
use bifrost_ui::capture_access::{CaptureAccess, Error, Media, Status};
use bifrost_ui::location_auth::{AuthorizationStatus, Level, RequestOutcome, UsageDescriptions};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Most sheets waiting behind the one on screen. A request past this is dropped; the app asks again later.
pub const MAX_QUEUED_PROMPTS: usize = 8;

thread_local! {
    static STORE: RefCell<BTreeMap<String, CaptureAccess>> = RefCell::new(BTreeMap::new());
    static LOCATION: RefCell<BTreeMap<String, AuthorizationStatus>> = RefCell::new(BTreeMap::new());
    static PROMPTS: RefCell<VecDeque<Prompt>> = RefCell::new(VecDeque::new());
    /// Apps whose location grant is Allow Once: they lose it when the app quits (see `expire_once`).
    static ONCE: RefCell<BTreeSet<String>> = RefCell::new(BTreeSet::new());
    /// Apps the person allowed to post input events and install event taps (the `events` service). Only Settings
    /// writes it; a manifest permission alone never grants it.
    static ACCESSIBILITY: RefCell<BTreeSet<String>> = RefCell::new(BTreeSet::new());
}

/// The answer to a location request: Allow Once (until the app quits), Allow While Using App, or Don't Allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    Once,
    WhileUsing,
    Deny,
}

/// Which permission an app asked for: location, or one capture media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Location,
    Media(Media),
    /// The contact store (the contacts service's access sheet).
    Contacts,
    /// The person's calendar (the calendar service's access sheet). `write_only` asks only to add events.
    Calendars { write_only: bool },
    /// One USB or raw HID device for one app (the `usb` service's sheet). The prompt's `purpose` holds the device key.
    Usb,
}

impl Kind {
    /// The name the sheet's topic is known by.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Location => "location",
            Kind::Media(m) => media_name(m),
            Kind::Contacts => "contacts",
            Kind::Calendars { .. } => "calendars",
            Kind::Usb => "usb",
        }
    }

    /// The sheet's title: the app and the thing it wants to use.
    pub fn title(self, app_name: &str) -> String {
        let want = match self {
            Kind::Location => "use your location",
            Kind::Media(Media::Video) => "use your camera",
            Kind::Media(Media::Audio) => "use your microphone",
            Kind::Media(Media::Screen) => "record your screen",
            Kind::Contacts => "use your contacts",
            Kind::Calendars { write_only: true } => "add events to your calendar",
            Kind::Calendars { write_only: false } => "use your calendar",
            Kind::Usb => "use a device connected to this computer",
        };
        format!("\u{201c}{app_name}\u{201d} would like to {want}.")
    }
}

/// One request from an app that is waiting for the person's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub app_id: String,
    pub app_name: String,
    pub kind: Kind,
    /// The manifest's `[privacy] location` text, shown on the sheet (empty for media requests).
    pub purpose: String,
}

/// Which of the sheet's two permission kinds an answer to `method` needs: a location prompt when the reply is
/// `prompt`, a media prompt when a `request` finds the media type `not_determined`. None for any other call.
pub fn wants_prompt(method: &str, params: &Json, reply: &Json) -> Option<Kind> {
    match method {
        "location_request" if reply.get("outcome").and_then(Json::as_str) == Some("prompt") => Some(Kind::Location),
        "request" if reply.get("status").and_then(Json::as_str) == Some("not_determined") => {
            media_of(params).ok().map(Kind::Media)
        }
        _ => None,
    }
}

/// Queues a sheet for an app's request. A repeat of a request already waiting (same app, kind and purpose: for a USB
/// device the purpose is the device) is not queued twice, and the queue holds at most [`MAX_QUEUED_PROMPTS`]. Returns
/// whether the request is now waiting for an answer, which is false only when the queue is full.
pub fn queue_prompt(app_id: &str, app_name: &str, kind: Kind, purpose: &str) -> bool {
    PROMPTS.with(|q| {
        let mut q = q.borrow_mut();
        if q.iter().any(|p| p.app_id == app_id && p.kind == kind && p.purpose == purpose) {
            return true;
        }
        if q.len() >= MAX_QUEUED_PROMPTS {
            return false;
        }
        q.push_back(Prompt { app_id: app_id.to_string(), app_name: app_name.to_string(), kind, purpose: purpose.to_string() });
        true
    })
}

/// Whether the person still has to answer this request. A request answered since it was queued is dropped.
pub fn still_asked(prompt: &Prompt) -> bool {
    match prompt.kind {
        Kind::Location => location_status(&prompt.app_id) == AuthorizationStatus::NotDetermined,
        Kind::Media(m) => access(&prompt.app_id).status(m) == Status::NotDetermined,
        Kind::Contacts => super::contacts::access_of(&prompt.app_id).status() == bifrost_ui::contact_access::Status::NotDetermined,
        Kind::Calendars { .. } => super::calendar::access_of(&prompt.app_id).status(bifrost_ui::calendar_access::EntityType::Event) == bifrost_ui::calendar_access::Status::NotDetermined,
        Kind::Usb => super::usb_access::still_asked(&prompt.app_id, &prompt.purpose),
    }
}

/// The next request that still needs an answer, for the shell to show as a sheet.
pub fn take_prompt() -> Option<Prompt> {
    PROMPTS.with(|q| {
        let mut q = q.borrow_mut();
        while let Some(p) = q.pop_front() {
            if still_asked(&p) {
                return Some(p);
            }
        }
        None
    })
}

/// Writes the person's answer to a sheet: Allow is `allowed`, Don't Allow is not. The same rules as the Settings
/// switches apply (a restricted location keeps its status).
pub fn decide(prompt: &Prompt, allowed: bool) {
    decide_grant(prompt, if allowed { Grant::WhileUsing } else { Grant::Deny });
}

/// Writes the person's sheet answer: `grant` for a location request; for a media request only `Deny` refuses and
/// anything else allows (the media store has no session-only grant).
pub fn decide_grant(prompt: &Prompt, grant: Grant) {
    match prompt.kind {
        Kind::Location => {
            set_location_grant(&prompt.app_id, grant);
        }
        Kind::Media(m) => {
            let _ = set_by_user(&prompt.app_id, m, grant != Grant::Deny);
        }
        // Allow Once lasts until the app quits or the device is unplugged; While Using stands for Always Allow here
        Kind::Usb => {
            super::usb_access::answer(&prompt.app_id, &prompt.purpose, grant);
        }
        Kind::Contacts => {
            let choice = if grant == Grant::Deny { bifrost_ui::contact_access::Choice::Deny } else { bifrost_ui::contact_access::Choice::All };
            let _ = super::contacts::answer_prompt(&prompt.app_id, choice);
        }
        Kind::Calendars { write_only } => {
            super::calendar::answer_prompt(&prompt.app_id, write_only, grant != Grant::Deny);
        }
    }
}

/// Writes a location grant. Allow Once and Allow While Using both make the app authorized while in use; Allow Once is
/// also remembered so `expire_once` can take it back. A restricted app keeps its status.
pub fn set_location_grant(app_id: &str, grant: Grant) -> AuthorizationStatus {
    set_location_grant_with(app_id, grant, crate::bare::account_policy::feature_switched_off(Feature::Location))
}

/// [`set_location_grant`] with the content limit given. While the limit is on the answer is still stored, so it is
/// there when the limit lifts, but the status reported stays restricted (see [`location_status_with`]).
pub fn set_location_grant_with(app_id: &str, grant: Grant, limited: bool) -> AuthorizationStatus {
    mark_grants_dirty();
    let current = stored_location_status(app_id);
    if current == AuthorizationStatus::Restricted {
        return location_status_with(app_id, limited);
    }
    let next = if grant == Grant::Deny { AuthorizationStatus::Denied } else { AuthorizationStatus::AuthorizedWhenInUse };
    ONCE.with(|o| {
        let mut o = o.borrow_mut();
        if grant == Grant::Once {
            o.insert(app_id.to_string());
        } else {
            o.remove(app_id);
        }
    });
    LOCATION.with(|s| {
        s.borrow_mut().insert(app_id.to_string(), next);
    });
    location_status_with(app_id, limited)
}

/// Takes back every Allow Once grant whose app is no longer running (`is_running`): the app's location goes back to
/// not asked, so its next request shows the sheet again. Returns the apps it reset.
pub fn expire_once(is_running: impl Fn(&str) -> bool) -> Vec<String> {
    let gone: Vec<String> = ONCE.with(|o| o.borrow().iter().filter(|a| !is_running(a.as_str())).cloned().collect());
    if !gone.is_empty() {
        mark_grants_dirty();
    }
    for app in &gone {
        ONCE.with(|o| o.borrow_mut().remove(app));
        LOCATION.with(|s| s.borrow_mut().remove(app));
    }
    gone
}

/// The location status of one app (a new app has not been asked yet).
pub fn location_status(app_id: &str) -> AuthorizationStatus {
    location_status_with(app_id, crate::bare::account_policy::feature_switched_off(Feature::Location))
}

/// The status the person's answer stands for, as stored (the answer is kept even while a limit hides it).
pub fn stored_location_status(app_id: &str) -> AuthorizationStatus {
    LOCATION.with(|s| s.borrow().get(app_id).copied().unwrap_or(AuthorizationStatus::NotDetermined))
}

/// The reported status: restricted while the installed account's content limits switch location off, else the stored
/// answer.
pub fn location_status_with(app_id: &str, limited: bool) -> AuthorizationStatus {
    if limited {
        AuthorizationStatus::Restricted
    } else {
        stored_location_status(app_id)
    }
}

/// The person's location switch for one app: on allows location while the app is in use, off refuses it. A restricted
/// app keeps the status it has, so the switch shows the answer that stands.
pub fn set_location_by_user(app_id: &str, allowed: bool) -> AuthorizationStatus {
    mark_grants_dirty();
    set_location_grant(app_id, if allowed { Grant::WhileUsing } else { Grant::Deny })
}

/// The three choices of an app's Location Services access popup (Settings > Privacy & Security > Location Services).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationAccess {
    Never,
    AskNextTime,
    WhileUsing,
}

impl LocationAccess {
    /// The popup's choices, in the order it lists them.
    pub const ALL: [LocationAccess; 3] = [LocationAccess::Never, LocationAccess::AskNextTime, LocationAccess::WhileUsing];

    /// The text of the choice (and of the row's value).
    pub fn label(self) -> &'static str {
        match self {
            LocationAccess::Never => "Never",
            LocationAccess::AskNextTime => "Ask Next Time",
            LocationAccess::WhileUsing => "While Using the App",
        }
    }

    /// The choice an app's grant shows as. A restricted app shows Never (its grant cannot change, see `label_of`).
    pub fn of(status: AuthorizationStatus) -> LocationAccess {
        match status {
            AuthorizationStatus::NotDetermined => LocationAccess::AskNextTime,
            AuthorizationStatus::Denied | AuthorizationStatus::Restricted => LocationAccess::Never,
            AuthorizationStatus::AuthorizedWhenInUse | AuthorizationStatus::AuthorizedAlways => LocationAccess::WhileUsing,
        }
    }
}

/// The value a row shows for an app's grant: its choice, or Restricted when the device restricts the app.
pub fn access_label_of(status: AuthorizationStatus) -> &'static str {
    if status == AuthorizationStatus::Restricted {
        "Restricted"
    } else {
        LocationAccess::of(status).label()
    }
}

/// Writes an app's location access from the popup. Never denies, While Using allows while in use, and Ask Next Time
/// forgets the answer so the app's next request shows the sheet again. A restricted app keeps its status.
pub fn set_location_access(app_id: &str, choice: LocationAccess) -> AuthorizationStatus {
    mark_grants_dirty();
    match choice {
        LocationAccess::Never => set_location_grant(app_id, Grant::Deny),
        LocationAccess::WhileUsing => set_location_grant(app_id, Grant::WhileUsing),
        LocationAccess::AskNextTime => {
            let current = location_status(app_id);
            if current != AuthorizationStatus::Restricted {
                ONCE.with(|o| {
                    o.borrow_mut().remove(app_id);
                });
                LOCATION.with(|s| {
                    s.borrow_mut().remove(app_id);
                });
            }
            location_status(app_id)
        }
    }
}

/// Every app that has an answer for location (granted, refused or Allow Once), sorted by app id. An app the person
/// has not answered for is not listed: the store keeps no entry for it.
pub fn location_asked() -> Vec<(String, AuthorizationStatus)> {
    LOCATION.with(|s| s.borrow().iter().map(|(id, st)| (id.clone(), *st)).collect())
}

/// The wire name of a location status.
pub fn location_name(status: AuthorizationStatus) -> &'static str {
    match status {
        AuthorizationStatus::NotDetermined => "not_determined",
        AuthorizationStatus::Restricted => "restricted",
        AuthorizationStatus::Denied => "denied",
        AuthorizationStatus::AuthorizedAlways => "authorized_always",
        AuthorizationStatus::AuthorizedWhenInUse => "authorized_when_in_use",
    }
}

/// The wire name of a request outcome.
pub fn outcome_name(outcome: RequestOutcome) -> &'static str {
    match outcome {
        RequestOutcome::Prompt => "prompt",
        RequestOutcome::AlreadyGranted => "already_granted",
        RequestOutcome::Refused => "refused",
    }
}

/// The capture permissions of one app (a new app has none answered yet). The installed account's content limits
/// restrict the media types it switches off: the stored answer stays, and it returns when the limit is lifted.
pub fn access(app_id: &str) -> CaptureAccess {
    let stored = STORE.with(|s| s.borrow().get(app_id).copied().unwrap_or_default());
    limited_access(stored, crate::bare::account_policy::feature_switched_off(Feature::Camera), crate::bare::account_policy::feature_switched_off(Feature::Microphone))
}

/// `stored` with the media types the content limits switch off restricted.
pub fn limited_access(mut access: CaptureAccess, camera_off: bool, microphone_off: bool) -> CaptureAccess {
    if camera_off {
        access.set_restricted(Media::Video, true);
    }
    if microphone_off {
        access.set_restricted(Media::Audio, true);
    }
    access
}

/// A change the person makes in Settings for one app and media type.
pub fn set_by_user(app_id: &str, media: Media, allowed: bool) -> Result<Status, Error> {
    mark_grants_dirty();
    STORE.with(|s| {
        let mut map = s.borrow_mut();
        let entry = map.entry(app_id.to_string()).or_default();
        entry.set_by_user(media, allowed)
    })
}

/// The wire name of a media type.
pub fn media_name(media: Media) -> &'static str {
    match media {
        Media::Video => "camera",
        Media::Audio => "microphone",
        Media::Screen => "screen",
    }
}

/// Every media type, in the order Settings lists them.
pub const MEDIA: [Media; 3] = [Media::Video, Media::Audio, Media::Screen];

/// The wire name of a status.
pub fn status_name(status: Status) -> &'static str {
    match status {
        Status::NotDetermined => "not_determined",
        Status::Restricted => "restricted",
        Status::Denied => "denied",
        Status::Authorized => "authorized",
    }
}

fn media_of(params: &Json) -> Result<Media, String> {
    let name = params.get("media").and_then(Json::as_str).ok_or("media is required (camera, microphone or screen)")?;
    match name {
        "camera" => Ok(Media::Video),
        "microphone" => Ok(Media::Audio),
        "screen" => Ok(Media::Screen),
        other => Err(format!("no media {other}")),
    }
}

/// One installed app's row in the App Permissions page: its id, its name and its answers.
pub struct Row {
    pub app_id: String,
    pub name: String,
    pub access: CaptureAccess,
    pub location: AuthorizationStatus,
    /// Whether the app may read contacts (a read grant: full or picked).
    pub contacts: bool,
    /// Whether the app may use the person's calendar (add-only or full access).
    pub calendars: bool,
    /// How many USB or raw HID devices the app holds a grant for.
    pub usb: usize,
    /// Whether the app's manifest asks to control the computer (`accessibility`); only such apps are listed.
    pub accessibility_asked: bool,
    /// Whether the person allowed the app to control the computer (post input events, watch input).
    pub accessibility: bool,
}

/// The installed apps, in the order the shell lists them.
pub fn rows() -> Vec<Row> {
    super::super::apphost::installed()
        .iter()
        .map(|a| Row {
            app_id: a.manifest.id.clone(),
            name: a.manifest.name.clone(),
            access: access(&a.manifest.id),
            location: location_status(&a.manifest.id),
            contacts: contacts_allowed(&a.manifest.id),
            calendars: super::calendar::allowed(&a.manifest.id),
            usb: super::usb_access::grant_count(&a.manifest.id),
            accessibility_asked: a.manifest.permissions.iter().any(|p| p == "accessibility"),
            accessibility: accessibility_allowed(&a.manifest.id),
        })
        .collect()
}

/// Whether the person lets the app read contacts. A Settings switch and the access sheet both set it.
pub fn contacts_allowed(app_id: &str) -> bool {
    super::contacts::reads(app_id)
}

/// The person's Settings switch for an app's contacts. A restricted app refuses it. Returns whether the app now reads
/// contacts.
pub fn set_contacts_by_user(app_id: &str, allowed: bool) -> bool {
    let choice = if allowed { bifrost_ui::contact_access::Choice::All } else { bifrost_ui::contact_access::Choice::Deny };
    let _ = super::contacts::change_by_user(app_id, choice);
    contacts_allowed(app_id)
}

/// The person's Settings switch for an app's calendar: on is full access, off is denied. A restricted app keeps its
/// status. Returns whether the app may now use the calendar.
pub fn set_calendars_by_user(app_id: &str, allowed: bool) -> bool {
    super::calendar::change_by_user(app_id, allowed);
    super::calendar::allowed(app_id)
}

/// Whether the person allowed the app to control the computer through the `events` service.
pub fn accessibility_allowed(app_id: &str) -> bool {
    ACCESSIBILITY.with(|a| a.borrow().contains(app_id))
}

/// Records the person's answer for an app (a Settings switch or a saved grant) without checking the manifest.
fn set_accessibility(app_id: &str, allowed: bool) {
    ACCESSIBILITY.with(|a| {
        let mut a = a.borrow_mut();
        if allowed {
            a.insert(app_id.to_string());
        } else {
            a.remove(app_id);
        }
    });
    mark_grants_dirty();
}

/// The person's Settings switch for an app's control of the computer. Only an app whose manifest asks for the
/// `accessibility` permission can be allowed. Returns whether the app is now allowed.
pub fn set_accessibility_by_user(app_id: &str, allowed: bool) -> bool {
    if allowed && !super::app_has_permission(app_id, "accessibility") {
        return false;
    }
    set_accessibility(app_id, allowed);
    accessibility_allowed(app_id)
}

pub struct PrivacyPermissions;

/// The purpose string an app's manifest gives for location (`[privacy] location`), empty when it has none.
pub fn manifest_location_purpose(app_id: &str) -> String {
    super::super::apphost::installed().iter().find(|a| a.manifest.id == app_id).map_or_else(String::new, |a| a.manifest.location_purpose.clone())
}

/// Refuses a location request whose manifest has no purpose string. The prompt is the only place the person learns
/// why the app wants location, so an empty text must not reach the sheet.
pub fn location_purpose_gate(purpose: &str) -> Result<(), String> {
    let texts = UsageDescriptions::new(purpose, "");
    texts
        .for_level(Level::WhenInUse)
        .map(|_| ())
        .map_err(|_| "location needs a purpose string: add [privacy] location = \"...\" to manifest.toml".to_string())
}

impl SystemService for PrivacyPermissions {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let purpose = manifest_location_purpose(app_id);
        if method == "location_request" {
            location_purpose_gate(&purpose)?;
        }
        let reply = answer(app_id, method, params)?;
        if let Some(kind) = wants_prompt(method, params, &reply) {
            let name = super::super::apphost::installed()
                .iter()
                .find(|a| a.manifest.id == app_id)
                .map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
            queue_prompt(app_id, &name, kind, if kind == Kind::Location { purpose.trim() } else { "" });
        }
        Ok(reply)
    }
}

/// The answer to one method call by `app_id`. Pure: reads the store only.
pub fn answer(app_id: &str, method: &str, params: &Json) -> Result<Json, String> {
    {
        match method {
            "status" | "request" => {
                let media = media_of(params)?;
                let status = access(app_id).status(media);
                Ok(Json::Obj(vec![
                    ("media".into(), Json::Str(media_name(media).into())),
                    ("status".into(), Json::Str(status_name(status).into())),
                ]))
            }
            "can_capture" => {
                let media = media_of(params)?;
                Ok(Json::Obj(vec![
                    ("media".into(), Json::Str(media_name(media).into())),
                    ("allowed".into(), Json::Bool(access(app_id).can_capture(media))),
                ]))
            }
            "all" => {
                let a = access(app_id);
                Ok(Json::Obj(
                    MEDIA.iter().map(|m| (media_name(*m).to_string(), Json::Str(status_name(a.status(*m)).into()))).collect(),
                ))
            }
            "location_status" => {
                let status = location_status(app_id);
                Ok(Json::Obj(vec![
                    ("status".into(), Json::Str(location_name(status).into())),
                    ("authorized".into(), Json::Bool(status.is_authorized())),
                ]))
            }
            "location_request" => {
                let outcome = location_status(app_id).request_outcome();
                Ok(Json::Obj(vec![("outcome".into(), Json::Str(outcome_name(outcome).into()))]))
            }
            _ => Err(format!("privacy has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_limits_restrict_only_the_media_they_switch_off() {
        let mut stored = CaptureAccess::default();
        stored.request(Media::Video, true);
        stored.request(Media::Audio, true);
        let limited = limited_access(stored, true, false);
        assert_eq!(limited.status(Media::Video), Status::Restricted);
        assert_eq!(limited.status(Media::Audio), Status::Authorized);
        assert!(!limited.can_capture(Media::Video));
        // the stored answer is untouched, so lifting the limit brings it back
        assert_eq!(limited_access(stored, false, false).status(Media::Video), Status::Authorized);
    }

    #[test]
    fn a_location_grant_is_stored_while_a_limit_is_on_and_reported_restricted() {
        let app = "limit.location.test";
        let answer = set_location_grant_with(app, Grant::WhileUsing, true);
        assert_eq!(answer, AuthorizationStatus::Restricted, "the limit hides the answer");
        assert_eq!(stored_location_status(app), AuthorizationStatus::AuthorizedWhenInUse, "the answer is kept");
        assert_eq!(location_status_with(app, false), AuthorizationStatus::AuthorizedWhenInUse, "it returns when the limit lifts");
        assert_eq!(set_location_grant_with(app, Grant::Deny, false), AuthorizationStatus::Denied);
    }

    #[test]
    fn a_location_request_needs_a_purpose_string() {
        assert_eq!(location_purpose_gate("  Finna næstu stöð  "), Ok(()));
        assert!(location_purpose_gate("").is_err());
        assert!(location_purpose_gate(" \t ").is_err());
        assert!(location_purpose_gate("").unwrap_err().contains("[privacy] location"), "the error names the manifest key");
    }

    fn obj(media: &str) -> Json {
        Json::Obj(vec![("media".into(), Json::Str(media.into()))])
    }

    #[test]
    fn a_new_app_has_no_answers_and_cannot_capture() {
        let a = access("is.example.new");
        assert_eq!(a.status(Media::Video), Status::NotDetermined);
        assert!(!a.can_capture(Media::Audio));
    }

    #[test]
    fn a_settings_change_is_seen_by_the_app_per_media_type() {
        let id = "is.example.cam";
        assert_eq!(set_by_user(id, Media::Video, true), Ok(Status::Authorized));
        assert!(access(id).can_capture(Media::Video));
        assert!(!access(id).can_capture(Media::Audio));
        assert_eq!(set_by_user(id, Media::Video, false), Ok(Status::Denied));
        assert!(!access(id).can_capture(Media::Video));
    }

    #[test]
    fn the_service_answers_status_and_can_capture() {
        let id = "is.example.svc";
        set_by_user(id, Media::Audio, true).unwrap();
        let s = answer(id, "status", &obj("microphone")).unwrap();
        assert_eq!(s.get("status").and_then(Json::as_str), Some("authorized"));
        let c = answer(id, "can_capture", &obj("camera")).unwrap();
        assert_eq!(c.get("allowed"), Some(&Json::Bool(false)));
        let all = answer(id, "all", &Json::Null).unwrap();
        assert_eq!(all.get("screen").and_then(Json::as_str), Some("not_determined"));
    }

    #[test]
    fn an_unknown_media_or_method_is_an_error() {
        assert!(answer("x", "status", &obj("location")).is_err());
        assert!(answer("x", "status", &Json::Null).is_err());
        assert!(answer("x", "erase", &obj("camera")).is_err());
    }

    #[test]
    fn a_new_app_has_no_location_answer_and_is_asked_for_one() {
        let id = "is.example.loc-new";
        assert_eq!(location_status(id), AuthorizationStatus::NotDetermined);
        let s = answer(id, "location_status", &Json::Null).unwrap();
        assert_eq!(s.get("status").and_then(Json::as_str), Some("not_determined"));
        assert_eq!(s.get("authorized"), Some(&Json::Bool(false)));
        let r = answer(id, "location_request", &Json::Null).unwrap();
        assert_eq!(r.get("outcome").and_then(Json::as_str), Some("prompt"));
    }

    #[test]
    fn the_location_switch_sets_when_in_use_or_denied_and_the_request_follows() {
        let id = "is.example.loc-switch";
        assert_eq!(set_location_by_user(id, true), AuthorizationStatus::AuthorizedWhenInUse);
        let s = answer(id, "location_status", &Json::Null).unwrap();
        assert_eq!(s.get("status").and_then(Json::as_str), Some("authorized_when_in_use"));
        assert_eq!(s.get("authorized"), Some(&Json::Bool(true)));
        let r = answer(id, "location_request", &Json::Null).unwrap();
        assert_eq!(r.get("outcome").and_then(Json::as_str), Some("already_granted"));
        assert_eq!(set_location_by_user(id, false), AuthorizationStatus::Denied);
        let r = answer(id, "location_request", &Json::Null).unwrap();
        assert_eq!(r.get("outcome").and_then(Json::as_str), Some("refused"));
    }

    #[test]
    fn a_restricted_app_keeps_its_location_status_when_the_person_changes_the_switch() {
        let id = "is.example.loc-restricted";
        LOCATION.with(|s| {
            s.borrow_mut().insert(id.to_string(), AuthorizationStatus::Restricted);
        });
        assert_eq!(set_location_by_user(id, true), AuthorizationStatus::Restricted);
        assert_eq!(location_status(id), AuthorizationStatus::Restricted);
    }

    #[test]
    fn a_prompt_is_wanted_only_for_an_unanswered_location_or_media_request() {
        let none = Json::Null;
        let prompt = Json::Obj(vec![("outcome".into(), Json::Str("prompt".into()))]);
        assert_eq!(wants_prompt("location_request", &none, &prompt), Some(Kind::Location));
        let granted = Json::Obj(vec![("outcome".into(), Json::Str("already_granted".into()))]);
        assert_eq!(wants_prompt("location_request", &none, &granted), None);
        let asked = Json::Obj(vec![("status".into(), Json::Str("not_determined".into()))]);
        assert_eq!(wants_prompt("request", &obj("camera"), &asked), Some(Kind::Media(Media::Video)));
        let answered = Json::Obj(vec![("status".into(), Json::Str("denied".into()))]);
        assert_eq!(wants_prompt("request", &obj("camera"), &answered), None);
        assert_eq!(wants_prompt("status", &obj("camera"), &asked), None, "a plain status call never prompts");
    }

    #[test]
    fn a_request_answers_with_the_current_status() {
        let id = "is.example.req";
        let r = answer(id, "request", &obj("screen")).unwrap();
        assert_eq!(r.get("status").and_then(Json::as_str), Some("not_determined"));
        assert!(answer(id, "request", &Json::Null).is_err());
    }

    #[test]
    fn the_sheet_titles_name_the_app_and_the_thing_asked_for() {
        assert_eq!(Kind::Location.title("Notes"), "\u{201c}Notes\u{201d} would like to use your location.");
        assert_eq!(Kind::Media(Media::Audio).title("Voice"), "\u{201c}Voice\u{201d} would like to use your microphone.");
        assert_eq!(Kind::Media(Media::Screen).name(), "screen");
    }

    #[test]
    fn a_queued_request_is_shown_once_and_answered_by_the_person() {
        let id = "is.example.sheet";
        queue_prompt(id, "Maps", Kind::Location, "");
        queue_prompt(id, "Maps", Kind::Location, "");
        let p = take_prompt().expect("one prompt");
        assert_eq!(p.app_name, "Maps");
        assert!(take_prompt().is_none(), "the repeat was not queued twice");
        decide(&p, true);
        assert_eq!(location_status(id), AuthorizationStatus::AuthorizedWhenInUse);
    }

    #[test]
    fn two_usb_devices_of_one_app_each_get_a_sheet_and_a_full_queue_says_so() {
        let id = "is.example.usbtwo";
        assert!(queue_prompt(id, "Dev", Kind::Usb, "1-1"));
        assert!(queue_prompt(id, "Dev", Kind::Usb, "1-2"), "another device is another request");
        assert!(queue_prompt(id, "Dev", Kind::Usb, "1-1"), "the same device waits once");
        let mine = || PROMPTS.with(|q| q.borrow().iter().filter(|p| p.app_id == id).count());
        assert_eq!(mine(), 2);
        let mut n = 0;
        while queue_prompt(&format!("is.example.fill{n}"), "Fill", Kind::Location, "") {
            n += 1;
            assert!(n <= MAX_QUEUED_PROMPTS, "the queue must refuse at its limit");
        }
        PROMPTS.with(|q| q.borrow_mut().clear());
    }

    #[test]
    fn a_media_sheet_that_is_denied_writes_denied() {
        let id = "is.example.mic";
        queue_prompt(id, "Voice", Kind::Media(Media::Audio), "");
        let p = take_prompt().unwrap();
        decide(&p, false);
        assert!(!access(id).can_capture(Media::Audio));
        assert_eq!(access(id).status(Media::Audio), Status::Denied);
    }

    #[test]
    fn a_request_answered_in_settings_before_its_turn_is_dropped() {
        let id = "is.example.late";
        queue_prompt(id, "Late", Kind::Location, "");
        set_location_by_user(id, false);
        assert!(take_prompt().is_none());
    }
}

// ---- persisted grants -------------------------------------------------------------------------------------------------
// The capture (camera, microphone, screen) and contacts answers are saved to `privacy-grants.tsv` beside the other shell
// data, so an Allow or Don't Allow survives a restart. Location answers are not saved yet.

const GRANTS_FILE: &str = "privacy-grants.tsv";
const GRANTS_HEADER: &str = "springboard-privacy-grants-v1";

thread_local! {
    static GRANTS_DIRTY: std::cell::Cell<bool> = std::cell::Cell::new(false);
    static GRANTS_LOADED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

/// Marks the grants as changed, so the next `persist_grants` writes them.
pub fn mark_grants_dirty() {
    GRANTS_DIRTY.with(|d| d.set(true));
}

/// Restores the saved grants the first time it runs, then writes them again whenever they change. Called from the
/// shell's tick, so a grant made in Settings or on a sheet is saved within a tick.
pub fn persist_grants(data_dir: &std::path::Path) {
    let path = data_dir.join(GRANTS_FILE);
    if !GRANTS_LOADED.with(|l| l.replace(true)) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            apply_grants(&text);
        }
        GRANTS_DIRTY.with(|d| d.set(false));
        return;
    }
    if GRANTS_DIRTY.with(|d| d.replace(false)) {
        if bifrost_core::fsutil::write_atomic(&path, render_grants()).is_err() {
            mark_grants_dirty();
        }
    }
}

/// The saved grants as text: one `media` line per capture answer and one `contacts` line per contacts answer.
pub fn render_grants() -> String {
    let mut out = format!("{GRANTS_HEADER}\n");
    STORE.with(|s| {
        for (app, access) in s.borrow().iter() {
            for media in MEDIA {
                match access.status(media) {
                    Status::Authorized => out.push_str(&format!("media\t{app}\t{}\t1\n", media_name(media))),
                    Status::Denied => out.push_str(&format!("media\t{app}\t{}\t0\n", media_name(media))),
                    Status::NotDetermined | Status::Restricted => {}
                }
            }
        }
    });
    for (app, status, picked) in super::contacts::grants_snapshot() {
        match status {
            bifrost_ui::contact_access::Status::Authorized => out.push_str(&format!("contacts\t{app}\tall\n")),
            bifrost_ui::contact_access::Status::Denied => out.push_str(&format!("contacts\t{app}\tdeny\n")),
            bifrost_ui::contact_access::Status::Limited => {
                let ids: Vec<String> = picked.iter().map(|id| id.to_string()).collect();
                out.push_str(&format!("contacts\t{app}\tpicked\t{}\n", ids.join(",")));
            }
            bifrost_ui::contact_access::Status::NotDetermined | bifrost_ui::contact_access::Status::Restricted => {}
        }
    }
    for (app, status) in super::calendar::grants_snapshot() {
        use bifrost_ui::calendar_access::Status as Cal;
        match status {
            Cal::FullAccess => out.push_str(&format!("calendars\t{app}\tfull\n")),
            Cal::WriteOnly => out.push_str(&format!("calendars\t{app}\tadd\n")),
            Cal::Denied => out.push_str(&format!("calendars\t{app}\tdeny\n")),
            Cal::NotDetermined | Cal::Restricted => {}
        }
    }
    ACCESSIBILITY.with(|a| {
        for app in a.borrow().iter() {
            out.push_str(&format!("accessibility\t{app}\tallow\n"));
        }
    });
    // Location answers. An Allow Once grant lasts only while its app runs, so it is not saved.
    let once: std::collections::BTreeSet<String> = ONCE.with(|o| o.borrow().clone());
    for (app, status) in location_asked() {
        if once.contains(&app) {
            continue;
        }
        if let Some(name) = location_saved_name(status) {
            out.push_str(&format!("location\t{app}\t{name}\n"));
        }
    }
    out
}

/// The saved name of a location answer, or `None` for an answer that is not saved (not asked, or restricted).
fn location_saved_name(status: AuthorizationStatus) -> Option<&'static str> {
    match status {
        AuthorizationStatus::Denied => Some("denied"),
        AuthorizationStatus::AuthorizedAlways => Some("always"),
        AuthorizationStatus::AuthorizedWhenInUse => Some("when_in_use"),
        AuthorizationStatus::NotDetermined | AuthorizationStatus::Restricted => None,
    }
}

/// Applies saved grants. Lines that do not parse are skipped; a restricted answer keeps its status as the model says.
pub fn apply_grants(text: &str) {
    if text.lines().next() != Some(GRANTS_HEADER) {
        return;
    }
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["media", app, name, allowed] => {
                let Some(media) = MEDIA.iter().copied().find(|m| media_name(*m) == *name) else { continue };
                let _ = set_by_user(app, media, *allowed == "1");
            }
            ["contacts", app, "all"] => {
                let _ = super::contacts::answer_prompt(app, bifrost_ui::contact_access::Choice::All);
            }
            ["contacts", app, "deny"] => {
                let _ = super::contacts::answer_prompt(app, bifrost_ui::contact_access::Choice::Deny);
            }
            ["contacts", app, "picked", ids] => {
                let picked: Option<std::collections::BTreeSet<u64>> = ids
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse::<u64>().ok())
                    .collect();
                if let Some(picked) = picked {
                    let _ = super::contacts::answer_prompt(app, bifrost_ui::contact_access::Choice::Picked(picked));
                }
            }
            ["calendars", app, "full"] => {
                super::calendar::answer_prompt(app, false, true);
            }
            ["calendars", app, "add"] => {
                super::calendar::answer_prompt(app, true, true);
            }
            ["calendars", app, "deny"] => {
                super::calendar::answer_prompt(app, false, false);
            }
            ["accessibility", app, "allow"] => set_accessibility(app, true),
            ["location", app, status] => {
                let status = match *status {
                    "denied" => AuthorizationStatus::Denied,
                    "always" => AuthorizationStatus::AuthorizedAlways,
                    "when_in_use" => AuthorizationStatus::AuthorizedWhenInUse,
                    _ => continue,
                };
                LOCATION.with(|s| s.borrow_mut().insert(app.to_string(), status));
            }
            _ => {}
        }
    }
    GRANTS_DIRTY.with(|d| d.set(false));
}

#[cfg(test)]
mod grants_tests {
    use super::*;

    #[test]
    fn a_saved_grant_round_trips_through_its_text() {
        let app = "is.example.grants.round";
        set_by_user(app, Media::Video, true).unwrap();
        let picked = "is.example.grants.picked";
        super::super::contacts::answer_prompt(picked, bifrost_ui::contact_access::Choice::Picked([3u64, 9].into_iter().collect())).unwrap();
        let text = render_grants();
        assert!(text.contains(&format!("media\t{app}\tcamera\t1\n")), "{text}");
        assert!(text.contains(&format!("contacts\t{picked}\tpicked\t3,9\n")), "{text}");

        let other = "is.example.grants.restored";
        apply_grants(&text.replace("is.example.grants.round", other).replace("is.example.grants.picked", "is.example.grants.picked2"));
        assert_eq!(access(other).status(Media::Video), Status::Authorized);
        assert_eq!(super::super::contacts::access_of("is.example.grants.picked2").picked().len(), 2);
    }

    #[test]
    fn a_calendar_answer_round_trips_through_its_text() {
        let (add, full, no) = ("is.example.grants.cal.add", "is.example.grants.cal.full", "is.example.grants.cal.no");
        super::super::calendar::answer_prompt(add, true, true);
        super::super::calendar::answer_prompt(full, false, true);
        super::super::calendar::answer_prompt(no, false, false);
        let text = render_grants();
        assert!(text.contains(&format!("calendars\t{add}\tadd\n")), "{text}");
        assert!(text.contains(&format!("calendars\t{full}\tfull\n")), "{text}");
        assert!(text.contains(&format!("calendars\t{no}\tdeny\n")), "{text}");
        apply_grants(&format!("{GRANTS_HEADER}\ncalendars\tis.example.grants.cal.back\tadd\n"));
        assert!(super::super::calendar::allowed("is.example.grants.cal.back"));
        assert!(!super::super::calendar::access_of("is.example.grants.cal.back").can_read(bifrost_ui::calendar_access::EntityType::Event));
    }

    #[test]
    fn the_calendar_sheet_asks_for_the_least_the_app_needs() {
        let add = Kind::Calendars { write_only: true };
        assert_eq!(add.name(), "calendars");
        assert_eq!(add.title("Mail"), "\u{201c}Mail\u{201d} would like to add events to your calendar.");
        assert_eq!(Kind::Calendars { write_only: false }.title("Mail"), "\u{201c}Mail\u{201d} would like to use your calendar.");
        let app = "is.example.grants.cal.sheet";
        let prompt = Prompt { app_id: app.into(), app_name: "Example".into(), kind: add, purpose: String::new() };
        assert!(still_asked(&prompt));
        decide(&prompt, true);
        assert!(!still_asked(&prompt));
        assert!(super::super::calendar::allowed(app));
    }

    #[test]
    fn accessibility_is_off_until_the_person_allows_it_and_survives_a_restart() {
        let app = "is.example.grants.ax";
        assert!(!accessibility_allowed(app), "a manifest alone grants nothing");
        assert!(!set_accessibility_by_user(app, true), "an app that does not declare the permission cannot be allowed");
        set_accessibility(app, true);
        let text = render_grants();
        assert!(text.contains(&format!("accessibility\t{app}\tallow\n")), "{text}");
        set_accessibility(app, false);
        assert!(!accessibility_allowed(app));
        apply_grants(&text);
        assert!(accessibility_allowed(app));
        set_accessibility(app, false);
    }

    #[test]
    fn a_file_without_the_header_restores_nothing() {
        apply_grants("media\tis.example.grants.none\tcamera\t1\n");
        assert_eq!(access("is.example.grants.none").status(Media::Video), Status::NotDetermined);
    }
}

#[cfg(test)]
mod location_grants_tests {
    use super::*;

    #[test]
    fn a_location_answer_is_saved_except_allow_once() {
        let kept = "is.example.grants.loc-kept";
        set_location_grant(kept, Grant::WhileUsing);
        let once = "is.example.grants.loc-once";
        set_location_grant(once, Grant::Once);
        let text = render_grants();
        assert!(text.contains(&format!("location\t{kept}\twhen_in_use\n")), "{text}");
        assert!(!text.contains(once), "an Allow Once grant ends with its app: {text}");
    }

    #[test]
    fn a_saved_location_answer_restores_its_status() {
        apply_grants("springboard-privacy-grants-v1\nlocation\tis.example.grants.loc-back\tdenied\n");
        assert_eq!(location_status("is.example.grants.loc-back"), AuthorizationStatus::Denied);
    }
}
