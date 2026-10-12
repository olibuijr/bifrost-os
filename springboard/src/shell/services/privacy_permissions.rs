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
//!
//! `media` is `camera`, `microphone` or `screen`. An unknown media name or method is an error, never a panic.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::capture_access::{CaptureAccess, Error, Media, Status};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static STORE: RefCell<BTreeMap<String, CaptureAccess>> = RefCell::new(BTreeMap::new());
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
}

/// The installed apps, in the order the shell lists them.
pub fn rows() -> Vec<Row> {
    super::super::apphost::installed()
        .iter()
        .map(|a| Row { app_id: a.manifest.id.clone(), name: a.manifest.name.clone(), access: access(&a.manifest.id) })
        .collect()
}

pub struct PrivacyPermissions;

impl SystemService for PrivacyPermissions {
    fn name(&self) -> &'static str {
        "privacy"
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(app_id, method, params)
    }
}

/// The answer to one method call by `app_id`. Pure: reads the store only.
pub fn answer(app_id: &str, method: &str, params: &Json) -> Result<Json, String> {
    {
        match method {
            "status" => {
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
}
