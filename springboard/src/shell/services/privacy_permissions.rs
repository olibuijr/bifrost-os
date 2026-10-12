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
use bifrost_ui::capture_access::{CaptureAccess, Error, Media, Status};
use bifrost_ui::location_auth::{AuthorizationStatus, RequestOutcome};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

/// Most sheets waiting behind the one on screen. A request past this is dropped; the app asks again later.
pub const MAX_QUEUED_PROMPTS: usize = 8;

thread_local! {
    static STORE: RefCell<BTreeMap<String, CaptureAccess>> = RefCell::new(BTreeMap::new());
    static LOCATION: RefCell<BTreeMap<String, AuthorizationStatus>> = RefCell::new(BTreeMap::new());
    static PROMPTS: RefCell<VecDeque<Prompt>> = RefCell::new(VecDeque::new());
}

/// Which permission an app asked for: location, or one capture media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Location,
    Media(Media),
}

impl Kind {
    /// The name the sheet's topic is known by.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Location => "location",
            Kind::Media(m) => media_name(m),
        }
    }

    /// The sheet's title: the app and the thing it wants to use.
    pub fn title(self, app_name: &str) -> String {
        let want = match self {
            Kind::Location => "use your location",
            Kind::Media(Media::Video) => "use your camera",
            Kind::Media(Media::Audio) => "use your microphone",
            Kind::Media(Media::Screen) => "record your screen",
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

/// Queues a sheet for an app's request. A repeat of a request already waiting is not queued twice, and the queue
/// holds at most [`MAX_QUEUED_PROMPTS`].
pub fn queue_prompt(app_id: &str, app_name: &str, kind: Kind) {
    PROMPTS.with(|q| {
        let mut q = q.borrow_mut();
        if q.iter().any(|p| p.app_id == app_id && p.kind == kind) || q.len() >= MAX_QUEUED_PROMPTS {
            return;
        }
        q.push_back(Prompt { app_id: app_id.to_string(), app_name: app_name.to_string(), kind });
    });
}

/// Whether the person still has to answer this request. A request answered since it was queued is dropped.
pub fn still_asked(prompt: &Prompt) -> bool {
    match prompt.kind {
        Kind::Location => location_status(&prompt.app_id) == AuthorizationStatus::NotDetermined,
        Kind::Media(m) => access(&prompt.app_id).status(m) == Status::NotDetermined,
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
    match prompt.kind {
        Kind::Location => {
            set_location_by_user(&prompt.app_id, allowed);
        }
        Kind::Media(m) => {
            let _ = set_by_user(&prompt.app_id, m, allowed);
        }
    }
}

/// The location status of one app (a new app has not been asked yet).
pub fn location_status(app_id: &str) -> AuthorizationStatus {
    LOCATION.with(|s| s.borrow().get(app_id).copied().unwrap_or(AuthorizationStatus::NotDetermined))
}

/// The person's location switch for one app: on allows location while the app is in use, off refuses it. A restricted
/// app keeps the status it has, so the switch shows the answer that stands.
pub fn set_location_by_user(app_id: &str, allowed: bool) -> AuthorizationStatus {
    let current = location_status(app_id);
    if current == AuthorizationStatus::Restricted {
        return current;
    }
    let next = if allowed { AuthorizationStatus::AuthorizedWhenInUse } else { AuthorizationStatus::Denied };
    LOCATION.with(|s| {
        s.borrow_mut().insert(app_id.to_string(), next);
    });
    next
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

/// The capture permissions of one app (a new app has none answered yet).
pub fn access(app_id: &str) -> CaptureAccess {
    STORE.with(|s| s.borrow().get(app_id).copied().unwrap_or_default())
}

/// A change the person makes in Settings for one app and media type.
pub fn set_by_user(app_id: &str, media: Media, allowed: bool) -> Result<Status, Error> {
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
        })
        .collect()
}

pub struct PrivacyPermissions;

impl SystemService for PrivacyPermissions {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let reply = answer(app_id, method, params)?;
        if let Some(kind) = wants_prompt(method, params, &reply) {
            let name = super::super::apphost::installed()
                .iter()
                .find(|a| a.manifest.id == app_id)
                .map_or_else(|| app_id.to_string(), |a| a.manifest.name.clone());
            queue_prompt(app_id, &name, kind);
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
        assert!(answer("x", "request", &obj("camera")).is_err());
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
        queue_prompt(id, "Maps", Kind::Location);
        queue_prompt(id, "Maps", Kind::Location);
        let p = take_prompt().expect("one prompt");
        assert_eq!(p.app_name, "Maps");
        assert!(take_prompt().is_none(), "the repeat was not queued twice");
        decide(&p, true);
        assert_eq!(location_status(id), AuthorizationStatus::AuthorizedWhenInUse);
    }

    #[test]
    fn a_media_sheet_that_is_denied_writes_denied() {
        let id = "is.example.mic";
        queue_prompt(id, "Voice", Kind::Media(Media::Audio));
        let p = take_prompt().unwrap();
        decide(&p, false);
        assert!(!access(id).can_capture(Media::Audio));
        assert_eq!(access(id).status(Media::Audio), Status::Denied);
    }

    #[test]
    fn a_request_answered_in_settings_before_its_turn_is_dropped() {
        let id = "is.example.late";
        queue_prompt(id, "Late", Kind::Location);
        set_location_by_user(id, false);
        assert!(take_prompt().is_none());
    }
}
