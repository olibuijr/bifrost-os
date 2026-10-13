//! System thumbnails for apps: the `thumbnails` service draws a picture of a file (or its type icon) at a size and a
//! display scale, and keeps the pictures in a disk cache that every app shares.
//!
//! [`request`] asks for one and returns the request id. The answer arrives as `App::service_reply`: decode the Ok body
//! with [`reply`]. A picture that is still being drawn arrives again later as a `ready` or `failed` service event:
//! decode it with [`event`]. A [`State::Pixels`] answer names the cached image; [`load_pixels`] reads it. A
//! [`State::Icon`] answer means the file has no picture, and the app draws the icon of its type itself.
//!
//! The models are `bifrost_ui::thumb_service`; the shell side is `springboard/src/shell/services/thumbnails.rs` and the
//! providers are `springboard/src/thumbs`.
//!
//! ```ignore
//! let id = thumbnails::request(ctx, "/home/me/photo.jpg", 64, 2.0, ThumbKind::Thumbnail);
//! // App::service_reply(ctx, id, Ok(body)) => thumbnails::reply(&body)
//! // App::service_event(ctx, service, event, body) => thumbnails::event(service, event, body)
//! ```

use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use bifrost_render::gfx::Image;

pub use bifrost_ui::thumb_service::{cache_key, icon_for, FileStamp, RequestError, ThumbArt, ThumbKind, ThumbRequest, MAX_PX, MIN_PX};

/// The shell service name.
pub const SERVICE: &str = "thumbnails";
/// The service events for a picture that was pending.
pub const READY_EVENT: &str = "ready";
pub const FAILED_EVENT: &str = "failed";

/// What the service has for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Being drawn. The same file and pixel edge arrive later as a `ready` or `failed` event.
    Pending,
    /// The picture: the cached raw image file (see [`load_pixels`]) and its size in pixels.
    Pixels { image: String, w: usize, h: usize },
    /// No picture: the type icon of the file, with the badge of its extension when it has one.
    Icon { type_id: String, badge: Option<String> },
    /// The file could not be read; the message says why.
    Failed(String),
}

/// One answer: the file and the pixel edge it is for, and its state.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub file: String,
    pub px: u32,
    pub state: State,
}

/// Asks for a thumbnail of `path` (absolute, no `..` part) that fits a square of `size` points at `scale` pixels per
/// point. Returns the request id the reply will carry.
pub fn request(ctx: &Ctx, path: &str, size: u32, scale: f32, kind: ThumbKind) -> u32 {
    let body = Json::Obj(vec![
        ("path".into(), Json::Str(path.into())),
        ("size".into(), Json::Num(size as f64)),
        ("scale".into(), Json::Num(scale as f64)),
        ("kind".into(), Json::Str(kind.as_str().into())),
    ]);
    service::call(ctx, SERVICE, "request", &body)
}

/// The answer in a reply body (the Ok value of `service_reply`). None for a body that is not a thumbnail answer.
pub fn reply(body: &Json) -> Option<Reply> {
    let file = body.str_field("file")?;
    let px = body.get("px").and_then(Json::as_u64)? as u32;
    let state = match body.str_field("state")?.as_str() {
        "pending" => State::Pending,
        "ready" => match body.str_field("art")?.as_str() {
            "pixels" => State::Pixels {
                image: body.str_field("image")?,
                w: body.get("w").and_then(Json::as_u64)? as usize,
                h: body.get("h").and_then(Json::as_u64)? as usize,
            },
            "icon" => State::Icon { type_id: body.str_field("type")?, badge: body.str_field("badge") },
            _ => return None,
        },
        "failed" => State::Failed(body.str_field("error").unwrap_or_default()),
        _ => return None,
    };
    Some(Reply { file, px, state })
}

/// A thumbnail event (the arguments of `service_event`) as a reply. None for other services and other events.
pub fn event(service: &str, event: &str, body: &Json) -> Option<Reply> {
    if service != SERVICE || (event != READY_EVENT && event != FAILED_EVENT) {
        return None;
    }
    reply(body)
}

/// The picture in the cached raw image file at `path` (the `image` of a [`State::Pixels`] answer). None when the file
/// is missing or is not a raw image.
pub fn load_pixels(path: &str) -> Option<Image> {
    Image::from_raw(&std::fs::read(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: &[(&str, Json)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    fn s(v: &str) -> Json {
        Json::Str(v.into())
    }

    #[test]
    fn a_picture_answer_names_the_cached_image_and_its_size() {
        let body = obj(&[("state", s("ready")), ("key", s("abc")), ("file", s("/a/b.png")), ("px", Json::Num(128.0)), ("art", s("pixels")), ("image", s("/c/abc.raw")), ("w", Json::Num(64.0)), ("h", Json::Num(32.0))]);
        assert_eq!(reply(&body), Some(Reply { file: "/a/b.png".into(), px: 128, state: State::Pixels { image: "/c/abc.raw".into(), w: 64, h: 32 } }));
    }

    #[test]
    fn an_icon_answer_keeps_its_type_and_badge() {
        let body = obj(&[("state", s("ready")), ("file", s("/a/b.docx")), ("px", Json::Num(64.0)), ("art", s("icon")), ("type", s("org.openxmlformats.wordprocessingml.document")), ("badge", s("DOCX"))]);
        assert_eq!(reply(&body).unwrap().state, State::Icon { type_id: "org.openxmlformats.wordprocessingml.document".into(), badge: Some("DOCX".into()) });
        let bare = obj(&[("state", s("ready")), ("file", s("/a")), ("px", Json::Num(64.0)), ("art", s("icon")), ("type", s("public.folder")), ("badge", Json::Null)]);
        assert_eq!(reply(&bare).unwrap().state, State::Icon { type_id: "public.folder".into(), badge: None });
    }

    #[test]
    fn pending_and_failed_answers_are_read() {
        let pending = obj(&[("state", s("pending")), ("file", s("/a/b.pdf")), ("px", Json::Num(96.0))]);
        assert_eq!(reply(&pending).unwrap().state, State::Pending);
        let failed = obj(&[("state", s("failed")), ("file", s("/a/b.pdf")), ("px", Json::Num(96.0)), ("error", s("gone"))]);
        assert_eq!(reply(&failed).unwrap().state, State::Failed("gone".into()));
    }

    #[test]
    fn other_bodies_are_not_thumbnail_answers() {
        assert_eq!(reply(&obj(&[("state", s("unknown")), ("file", s("/a")), ("px", Json::Num(1.0))])), None);
        assert_eq!(reply(&Json::Str("ok".into())), None);
        assert_eq!(reply(&obj(&[("state", s("pending")), ("px", Json::Num(1.0))])), None, "a file is required");
    }

    #[test]
    fn only_the_thumbnail_events_of_its_service_are_read() {
        let body = obj(&[("state", s("failed")), ("file", s("/a")), ("px", Json::Num(8.0)), ("error", s("x"))]);
        assert!(event(SERVICE, FAILED_EVENT, &body).is_some());
        assert!(event(SERVICE, READY_EVENT, &body).is_some());
        assert!(event(SERVICE, "closed", &body).is_none());
        assert!(event("quick-look", READY_EVENT, &body).is_none());
    }

    #[test]
    fn a_missing_or_foreign_cache_file_loads_no_picture() {
        assert!(load_pixels("/nonexistent/bifrost/none.raw").is_none());
        let dir = std::env::temp_dir().join(format!("sdk-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.raw");
        std::fs::write(&p, b"not an image").unwrap();
        assert!(load_pixels(p.to_str().unwrap()).is_none());
        let mut raw = b"RIMG".to_vec();
        raw.extend(1u32.to_le_bytes());
        raw.extend(1u32.to_le_bytes());
        raw.extend(0xff00_00ffu32.to_le_bytes());
        std::fs::write(&p, raw).unwrap();
        let im = load_pixels(p.to_str().unwrap()).unwrap();
        assert_eq!((im.w, im.h, im.px.len()), (1, 1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
