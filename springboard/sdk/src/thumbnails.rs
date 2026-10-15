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

pub use bifrost_ui::thumb_service::{edge_px, FileStamp, RequestError, ThumbArt, ThumbKind, ThumbRequest, MAX_PX, MIN_PX};

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

/// The kinds of picture a generator can deliver, from the cheapest to the best (`QLThumbnailGenerator.Request.RepresentationTypes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RepresentationType {
    /// The icon of the file's type.
    Icon,
    /// A small picture of the content, drawn quickly (half the requested edge).
    LowQualityThumbnail,
    /// The picture at the requested size.
    Thumbnail,
}

impl RepresentationType {
    /// All kinds, the default of a request that names none.
    pub const ALL: [RepresentationType; 3] = [RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail];
}

/// A generator request (`QLThumbnailGenerator.Request`): the file, the size in points, the scale and the wanted kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorRequest {
    pub request: ThumbRequest,
    types: Vec<RepresentationType>,
}

impl GeneratorRequest {
    /// An empty `types` list means every kind. Duplicates are dropped and the kinds are kept cheapest first.
    pub fn new(path: &str, size: u32, scale: f32, types: &[RepresentationType]) -> Result<Self, RequestError> {
        let request = ThumbRequest::new(path, size, scale, ThumbKind::Thumbnail)?;
        let mut types = if types.is_empty() { RepresentationType::ALL.to_vec() } else { types.to_vec() };
        types.sort();
        types.dedup();
        Ok(GeneratorRequest { request, types })
    }

    pub fn representation_types(&self) -> &[RepresentationType] {
        &self.types
    }

    /// The service requests to send, in delivery order: the kind, the shell request for it and the pixel edge the
    /// answer will carry.
    pub fn plan(&self) -> Vec<(RepresentationType, ThumbRequest, u32)> {
        self.types
            .iter()
            .map(|&t| {
                let (kind, size) = match t {
                    RepresentationType::Icon => (ThumbKind::Icon, self.request.size),
                    RepresentationType::LowQualityThumbnail => (ThumbKind::Thumbnail, (self.request.size / 2).max(1)),
                    RepresentationType::Thumbnail => (ThumbKind::Thumbnail, self.request.size),
                };
                let req = ThumbRequest { size, kind, ..self.request.clone() };
                let px = req.px();
                (t, req, px)
            })
            .collect()
    }
}

/// One delivered picture (`QLThumbnailRepresentation`).
#[derive(Clone, Debug, PartialEq)]
pub struct Representation {
    pub kind: RepresentationType,
    pub reply: Reply,
}

/// Tracks the answers of one `generateRepresentations(for:update:)` call. Feed it each reply or event; it hands back
/// the representation to pass to the update handler, never a worse one than already delivered.
#[derive(Debug)]
pub struct Generation {
    plan: Vec<(RepresentationType, u32)>,
    best: Option<RepresentationType>,
    done: bool,
}

impl Generation {
    pub fn new(req: &GeneratorRequest) -> Self {
        Generation { plan: req.plan().into_iter().map(|(t, _, px)| (t, px)).collect(), best: None, done: false }
    }

    /// The representation a reply is for. None for a reply that is pending, for another size, or not better than
    /// what was delivered. A failed reply ends the generation.
    pub fn accept(&mut self, reply: Reply) -> Option<Representation> {
        if self.done {
            return None;
        }
        match reply.state {
            State::Pending => return None,
            State::Failed(_) => {
                self.done = true;
                return None;
            }
            _ => {}
        }
        let is_icon = matches!(reply.state, State::Icon { .. });
        let kind = self.plan.iter().rev().find(|(t, px)| *px == reply.px && (*t == RepresentationType::Icon) == is_icon).map(|(t, _)| *t)?;
        if self.best.is_some_and(|b| b >= kind) {
            return None;
        }
        self.best = Some(kind);
        self.done = self.plan.last().is_some_and(|(t, _)| *t == kind);
        Some(Representation { kind, reply })
    }

    /// True once the best wanted kind arrived or a request failed.
    pub fn is_finished(&self) -> bool {
        self.done
    }

    /// Ends the generation early (the caller no longer wants the picture). Later replies are refused.
    pub fn cancel(&mut self) {
        self.done = true;
    }
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

    fn ready(px: u32, state: State) -> Reply {
        Reply { file: "/a/b.png".into(), px, state }
    }

    fn pixels() -> State {
        State::Pixels { image: "/c/x.raw".into(), w: 1, h: 1 }
    }

    fn icon() -> State {
        State::Icon { type_id: "public.png".into(), badge: None }
    }

    #[test]
    fn a_request_orders_and_dedups_its_kinds() {
        let r = GeneratorRequest::new("/a/b.png", 128, 2.0, &[RepresentationType::Thumbnail, RepresentationType::Icon, RepresentationType::Icon]).unwrap();
        assert_eq!(r.representation_types(), &[RepresentationType::Icon, RepresentationType::Thumbnail]);
        assert_eq!(GeneratorRequest::new("/a/b.png", 128, 1.0, &[]).unwrap().representation_types().len(), 3);
        assert_eq!(GeneratorRequest::new("a/b.png", 128, 1.0, &[]), Err(RequestError::BadPath));
    }

    #[test]
    fn the_plan_goes_icon_then_low_quality_then_full() {
        let plan = GeneratorRequest::new("/a/b.png", 100, 2.0, &[]).unwrap().plan();
        let kinds: Vec<_> = plan.iter().map(|p| (p.0, p.1.kind, p.2)).collect();
        assert_eq!(
            kinds,
            vec![
                (RepresentationType::Icon, ThumbKind::Icon, 200),
                (RepresentationType::LowQualityThumbnail, ThumbKind::Thumbnail, 100),
                (RepresentationType::Thumbnail, ThumbKind::Thumbnail, 200),
            ]
        );
    }

    #[test]
    fn updates_arrive_icon_then_low_then_final_and_never_go_back() {
        let mut g = Generation::new(&GeneratorRequest::new("/a/b.png", 100, 2.0, &[]).unwrap());
        assert_eq!(g.accept(ready(200, State::Pending)), None);
        assert_eq!(g.accept(ready(200, icon())).unwrap().kind, RepresentationType::Icon);
        assert_eq!(g.accept(ready(100, pixels())).unwrap().kind, RepresentationType::LowQualityThumbnail);
        assert!(!g.is_finished());
        assert_eq!(g.accept(ready(200, icon())), None, "an icon is no better than the low picture");
        assert_eq!(g.accept(ready(200, pixels())).unwrap().kind, RepresentationType::Thumbnail);
        assert!(g.is_finished());
        assert_eq!(g.accept(ready(100, pixels())), None);
    }

    #[test]
    fn a_failure_ends_the_generation() {
        let mut g = Generation::new(&GeneratorRequest::new("/a/b.png", 100, 1.0, &[]).unwrap());
        assert_eq!(g.accept(ready(100, State::Failed("gone".into()))), None);
        assert!(g.is_finished());
        assert_eq!(g.accept(ready(100, icon())), None);
    }
}
