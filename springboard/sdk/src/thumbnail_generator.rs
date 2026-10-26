//! Thumbnail generation for apps, both sides of `QuickLookThumbnailing`.
//!
//! - [`ThumbnailGenerator`] is the client (`QLThumbnailGenerator`): it runs several generations at once, each a
//!   [`GeneratorRequest`] with its representation tiers, and turns the `thumbnails` service's replies and events into
//!   [`Update`]s (the update handler): the icon first, then the low quality picture, then the full one.
//! - [`FileThumbnailRequest`], [`ThumbnailReply`] and [`answer_provider_call`] are the provider side
//!   (`QLThumbnailProvider`, `QLFileThumbnailRequest`, `QLThumbnailReply`): an app that declares a `[[preview]]`
//!   block for its file types in its manifest answers the shell's `preview.render` control call with the PNG it drew.
//!   The shell asks it for the file's thumbnail (Files, Search) as well as for the preview panel, and falls back to
//!   the built-in provider when the app gives no picture.
//!
//! ```ignore
//! // provider side, in the app's control handler
//! if let Some(reply) = thumbnail_generator::answer_provider_call(method, params, |req| {
//!     let png = draw_model(&req.file, req.maximum_width, req.maximum_height)?;
//!     ThumbnailReply::picture(&png).ok_or_else(|| ThumbnailError::new(ThumbnailErrorCode::Failed, "bad path"))
//! }) { return reply; }
//! ```
use crate::runtime::Ctx;
use crate::thumbnails::{request, Generation, GeneratorRequest, Reply, Representation, State, ThumbRequest, MAX_PX};
use bifrost_core::json::Json;

/// One update for a generation started on a [`ThumbnailGenerator`] (the update handler of
/// `QLThumbnailGenerator.generateRepresentations(for:update:)`).
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// A representation at least as good as any before it.
    Representation { id: u32, representation: Representation },
    /// The generation failed and is over.
    Failed { id: u32, error: crate::thumbnail_error::ThumbnailError },
}

/// Runs several generations at once and routes the replies and events to them. A generation is one
/// [`GeneratorRequest`]; the tracker sends its planned service requests, matches each answer to the generation that
/// asked for that file and pixel edge, and reports updates cheapest tier first, never a worse one after a better one.
#[derive(Debug, Default)]
pub struct ThumbnailGenerator {
    next: u32,
    live: Vec<(u32, GeneratorRequest, Generation)>,
}

/// The most generations kept at once; a new one beyond it is refused.
pub const MAX_GENERATIONS: usize = 256;

impl ThumbnailGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `req` and returns its id with the service requests to send (the pure half of [`Self::generate`]).
    /// None when [`MAX_GENERATIONS`] are already running.
    pub fn begin(&mut self, req: GeneratorRequest) -> Option<(u32, Vec<ThumbRequest>)> {
        if self.live.len() >= MAX_GENERATIONS {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        let asks = req.plan().into_iter().map(|(_, r, _)| r).collect();
        let generation = Generation::new(&req);
        self.live.push((id, req, generation));
        Some((id, asks))
    }

    /// Starts `req`: sends every planned request to the shell. Returns the generation id, or None when too many are
    /// running. Feed each `service_reply` body and `ready` or `failed` event to [`Self::accept`].
    pub fn generate(&mut self, ctx: &Ctx, req: GeneratorRequest) -> Option<u32> {
        let (id, asks) = self.begin(req)?;
        for r in &asks {
            request(ctx, &r.path, r.size, r.scale, r.kind);
        }
        Some(id)
    }

    /// Hands an answer to the generations waiting for that file. A finished or failed generation is dropped.
    pub fn accept(&mut self, reply: &Reply) -> Vec<Update> {
        let mut out = Vec::new();
        let mut done = Vec::new();
        for (id, req, generation) in &mut self.live {
            if req.request.path != reply.file || !generation.wants(reply.px) {
                continue;
            }
            if let State::Failed(message) = &reply.state {
                let error = crate::thumbnail_error::ThumbnailError::new(crate::thumbnail_error::classify(message), message.clone());
                generation.cancel();
                out.push(Update::Failed { id: *id, error });
            } else if let Some(representation) = generation.accept(reply.clone()) {
                out.push(Update::Representation { id: *id, representation });
            }
            if generation.is_finished() {
                done.push(*id);
            }
        }
        self.live.retain(|(id, ..)| !done.contains(id));
        out
    }

    /// Cancels a generation (`cancel` on the reference request). True when it was running.
    pub fn cancel(&mut self, id: u32) -> bool {
        let before = self.live.len();
        self.live.retain(|(i, ..)| *i != id);
        self.live.len() != before
    }

    /// How many generations are running.
    pub fn running(&self) -> usize {
        self.live.len()
    }
}

/// What the shell sends an app that declared a thumbnail provider (`[[preview]]` in its manifest, generator
/// [`crate::quick_look::PREVIEW_METHOD`]): the file and the size to draw it at (`QLFileThumbnailRequest`).
#[derive(Clone, Debug, PartialEq)]
pub struct FileThumbnailRequest {
    /// Absolute path of the file to draw.
    pub file: String,
    /// The largest picture wanted, in pixels.
    pub maximum_width: u32,
    pub maximum_height: u32,
}

impl FileThumbnailRequest {
    /// Reads the call parameters `{"file", "width", "height"}`. The file must be an absolute path with no `..` part and
    /// the size must be between 1 and [`MAX_PX`] pixels.
    pub fn parse(params: &Json) -> Result<Self, crate::thumbnail_error::ThumbnailError> {
        use crate::thumbnail_error::{ThumbnailError, ThumbnailErrorCode::BadRequest};
        let bad = |why: &str| ThumbnailError::new(BadRequest, why);
        let file = params.str_field("file").ok_or_else(|| bad("the request has no file"))?;
        let path = std::path::Path::new(&file);
        if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(bad("the file must be an absolute path with no .. part"));
        }
        let side = |key: &str| {
            params
                .get(key)
                .and_then(Json::as_u64)
                .filter(|v| (1..=MAX_PX as u64).contains(v))
                .map(|v| v as u32)
                .ok_or_else(|| bad("the size must be between 1 and the pixel limit"))
        };
        Ok(FileThumbnailRequest { file, maximum_width: side("width")?, maximum_height: side("height")? })
    }

    /// The size of a picture of `w` by `h` fitted inside the wanted size, aspect kept, never past it.
    pub fn fit(&self, w: u32, h: u32) -> (u32, u32) {
        if w == 0 || h == 0 {
            return (0, 0);
        }
        let k = (self.maximum_width as f32 / w as f32).min(self.maximum_height as f32 / h as f32);
        (((w as f32 * k).round() as u32).clamp(1, self.maximum_width), ((h as f32 * k).round() as u32).clamp(1, self.maximum_height))
    }
}

/// An app's answer to a [`FileThumbnailRequest`] (`QLThumbnailReply`): the PNG it drew.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailReply {
    picture: String,
}

impl ThumbnailReply {
    /// A reply naming the PNG at `path`: absolute, no `..` part, ending in `.png`. None for anything else, so an app
    /// cannot send the shell a path it would refuse.
    pub fn picture(path: &str) -> Option<Self> {
        let p = std::path::Path::new(path);
        let ok = p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir)) && path.to_ascii_lowercase().ends_with(".png");
        ok.then(|| ThumbnailReply { picture: path.to_string() })
    }

    pub fn path(&self) -> &str {
        &self.picture
    }

    /// The control reply body: `{"picture": "<path>"}`.
    pub fn to_json(&self) -> Json {
        crate::quick_look::picture_reply(&self.picture)
    }
}

/// Answers the shell's provider call. `method` and `params` are the control call; `provide` draws the file. None when
/// `method` is not the provider method, so the app's other control methods run as before. A refused request or a failed
/// draw is an error reply, which the shell treats as "no picture" and shows the type icon.
pub fn answer_provider_call(
    method: &str,
    params: &Json,
    provide: impl FnOnce(&FileThumbnailRequest) -> Result<ThumbnailReply, crate::thumbnail_error::ThumbnailError>,
) -> Option<Result<Json, String>> {
    if method != crate::quick_look::PREVIEW_METHOD {
        return None;
    }
    Some(FileThumbnailRequest::parse(params).and_then(|r| provide(&r)).map(|r| r.to_json()).map_err(|e| e.to_string()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbnail_error::ThumbnailErrorCode;
    use crate::thumbnails::RepresentationType;

    fn reply(file: &str, px: u32, state: State) -> Reply {
        Reply { file: file.into(), px, state }
    }

    fn pixels() -> State {
        State::Pixels { image: "/c/x.raw".into(), w: 1, h: 1 }
    }

    fn icon() -> State {
        State::Icon { type_id: "public.png".into(), badge: None }
    }

    fn all(path: &str, size: u32, scale: f32) -> GeneratorRequest {
        GeneratorRequest::new(path, size, scale, &[]).unwrap()
    }

    #[test]
    fn a_generation_sends_one_request_per_tier_and_gets_an_id() {
        let mut g = ThumbnailGenerator::new();
        let (id, asks) = g.begin(all("/a/b.png", 100, 2.0)).unwrap();
        assert_eq!(asks.iter().map(ThumbRequest::px).collect::<Vec<_>>(), vec![200, 50, 200]);
        let (id2, _) = g.begin(all("/a/c.png", 100, 2.0)).unwrap();
        assert_ne!(id, id2);
        assert_eq!(g.running(), 2);
    }

    #[test]
    fn answers_go_only_to_the_generation_that_asked_for_the_file() {
        let mut g = ThumbnailGenerator::new();
        let (a, _) = g.begin(all("/a/b.png", 100, 2.0)).unwrap();
        let (b, _) = g.begin(all("/a/c.png", 100, 2.0)).unwrap();
        let ups = g.accept(&reply("/a/c.png", 200, icon()));
        assert_eq!(ups.len(), 1);
        assert!(matches!(&ups[0], Update::Representation { id, representation } if *id == b && representation.kind == RepresentationType::Icon));
        assert!(g.accept(&reply("/a/b.png", 77, pixels())).is_empty(), "an edge no tier asked for is not ours");
        let _ = a;
    }

    #[test]
    fn updates_run_icon_low_full_and_the_finished_generation_is_dropped() {
        let mut g = ThumbnailGenerator::new();
        g.begin(all("/a/b.png", 100, 2.0)).unwrap();
        let kinds: Vec<_> = [(200, icon()), (50, pixels()), (200, pixels())]
            .into_iter()
            .flat_map(|(px, st)| g.accept(&reply("/a/b.png", px, st)))
            .map(|u| match u {
                Update::Representation { representation, .. } => representation.kind,
                Update::Failed { .. } => panic!("no failure expected"),
            })
            .collect();
        assert_eq!(kinds, vec![RepresentationType::Icon, RepresentationType::LowQualityThumbnail, RepresentationType::Thumbnail]);
        assert_eq!(g.running(), 0);
    }

    #[test]
    fn a_failure_reports_its_code_and_ends_that_generation_only() {
        let mut g = ThumbnailGenerator::new();
        let (a, _) = g.begin(all("/a/gone.png", 64, 1.0)).unwrap();
        g.begin(all("/a/other.png", 64, 1.0)).unwrap();
        let ups = g.accept(&reply("/a/gone.png", 64, State::Failed("no such file".into())));
        assert!(matches!(&ups[..], [Update::Failed { id, error }] if *id == a && error.code == ThumbnailErrorCode::NoSuchFile));
        assert_eq!(g.running(), 1);
        assert!(g.accept(&reply("/a/gone.png", 64, icon())).is_empty());
    }

    #[test]
    fn a_cancelled_generation_hears_nothing_more() {
        let mut g = ThumbnailGenerator::new();
        let (id, _) = g.begin(all("/a/b.png", 64, 1.0)).unwrap();
        assert!(g.cancel(id));
        assert!(!g.cancel(id), "already gone");
        assert!(g.accept(&reply("/a/b.png", 64, pixels())).is_empty());
    }

    #[test]
    fn too_many_generations_are_refused() {
        let mut g = ThumbnailGenerator::new();
        for i in 0..MAX_GENERATIONS {
            assert!(g.begin(all(&format!("/a/{i}.png"), 64, 1.0)).is_some());
        }
        assert!(g.begin(all("/a/extra.png", 64, 1.0)).is_none());
    }

    fn json(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn a_provider_request_needs_an_absolute_clean_file_and_a_sane_size() {
        let ok = FileThumbnailRequest::parse(&json(r#"{"file":"/d/model.obj","width":128,"height":64}"#)).unwrap();
        assert_eq!((ok.file.as_str(), ok.maximum_width, ok.maximum_height), ("/d/model.obj", 128, 64));
        for bad in [
            r#"{"width":8,"height":8}"#,
            r#"{"file":"rel.obj","width":8,"height":8}"#,
            r#"{"file":"/d/../etc/passwd","width":8,"height":8}"#,
            r#"{"file":"/d/a","width":0,"height":8}"#,
            r#"{"file":"/d/a","width":8,"height":99999}"#,
            r#"{"file":"/d/a","width":"8","height":8}"#,
        ] {
            let e = FileThumbnailRequest::parse(&json(bad)).unwrap_err();
            assert_eq!(e.code, ThumbnailErrorCode::BadRequest, "{bad}");
        }
    }

    #[test]
    fn a_picture_fits_inside_the_wanted_size_with_its_aspect_kept() {
        let r = FileThumbnailRequest { file: "/d/a".into(), maximum_width: 100, maximum_height: 50 };
        assert_eq!(r.fit(200, 200), (50, 50));
        assert_eq!(r.fit(400, 100), (100, 25));
        assert_eq!(r.fit(10, 10), (50, 50), "a small picture is scaled up to the box, as the shell fits it");
        assert_eq!(r.fit(0, 10), (0, 0));
    }

    #[test]
    fn a_reply_names_only_a_png_the_shell_would_accept() {
        assert!(ThumbnailReply::picture("/d/t.png").is_some());
        assert!(ThumbnailReply::picture("/d/T.PNG").is_some());
        for bad in ["t.png", "/d/t.jpg", "/d/../t.png", ""] {
            assert!(ThumbnailReply::picture(bad).is_none(), "{bad}");
        }
        let body = ThumbnailReply::picture("/d/t.png").unwrap().to_json();
        assert_eq!(body.str_field("picture").as_deref(), Some("/d/t.png"));
    }

    #[test]
    fn the_provider_helper_answers_only_its_method_and_turns_failures_into_error_replies() {
        let params = json(r#"{"file":"/d/model.obj","width":64,"height":64}"#);
        assert!(answer_provider_call("other.method", &params, |_| unreachable!()).is_none());
        let ok = answer_provider_call("preview.render", &params, |r| Ok(ThumbnailReply::picture(&format!("{}.png", r.file)).unwrap())).unwrap().unwrap();
        assert_eq!(ok.str_field("picture").as_deref(), Some("/d/model.obj.png"));
        let refused = answer_provider_call("preview.render", &json(r#"{"file":"x"}"#), |_| unreachable!()).unwrap();
        assert!(refused.is_err(), "a bad request never reaches the app's drawing code");
        let failed = answer_provider_call("preview.render", &params, |_| Err(crate::thumbnail_error::ThumbnailError::new(ThumbnailErrorCode::Unsupported, "no mesh"))).unwrap();
        assert!(failed.unwrap_err().contains("unsupported"));
    }
}
