//! Data-based previews (`QLPreviewProvider`, `QLFilePreviewRequest`, `QLPreviewReply`, `QLPreviewReplyAttachment`)
//! and thumbnail requests (`QLThumbnailGenerator.Request`), as pure models.
//!
//! A [`Registry`] maps a file type to the provider that previews it, using the type tree in [`crate::uttype`]: the
//! most specific registered type wins. A provider answers with a [`Reply`]: HTML with inline [`Attachment`]s, a PDF
//! page size, a bitmap size, or a file the viewer already shows. A [`ThumbnailRequest`] asks for one or more
//! [`Representation`]s of a file at a size in points and a scale, and [`ThumbnailRequest::best_of`] picks the best
//! one that was produced.

use crate::uttype;

/// Largest bitmap edge a reply may declare, in pixels. Larger sizes are refused, not clamped.
pub const MAX_BITMAP_EDGE: u32 = 16384;
/// Largest display scale a thumbnail request may ask for.
pub const MAX_SCALE: f32 = 4.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// A size, scale or bitmap edge is zero, negative, not finite, or above its limit.
    BadSize,
    /// An attachment id is empty or would break a `cid:` reference (whitespace, quotes, angle brackets).
    BadAttachmentId,
    /// Two attachments share an id.
    DuplicateAttachment,
    /// A file reply or registration has no name or type.
    Empty,
    /// A thumbnail request asks for no representation.
    NoRepresentation,
}

/// `QLPreviewReplyAttachment(data:contentType:)`: an inline resource an HTML preview refers to as `cid:<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub data: Vec<u8>,
    pub content_type: String,
}

/// What a reply draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// `init(dataOfContentType:...)` for HTML: the body and the attachments it names.
    Html { body: String, attachments: Vec<Attachment> },
    /// `init(forPDFWithPageSize:...)`: a PDF whose pages are `width` by `height` points.
    Pdf { width: f32, height: f32 },
    /// `init(contextSize:isBitmap:...)`: a bitmap of `width` by `height` pixels.
    Bitmap { width: u32, height: u32 },
    /// `init(fileURL:)`: the viewer shows the file itself.
    File { path: String },
}

/// `QLPreviewReply`: the content and the title shown above it. An empty title means the file name is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    title: String,
    content: Content,
}

impl Reply {
    /// Validates the content: sizes must be finite and positive within [`MAX_BITMAP_EDGE`], attachment ids must be
    /// usable in a `cid:` URL and unique, and a file reply needs a name.
    pub fn new(content: Content) -> Result<Self, ModelError> {
        match &content {
            Content::Pdf { width, height } => {
                if !(width.is_finite() && height.is_finite() && *width > 0.0 && *height > 0.0) {
                    return Err(ModelError::BadSize);
                }
            }
            Content::Bitmap { width, height } => {
                if *width == 0 || *height == 0 || *width > MAX_BITMAP_EDGE || *height > MAX_BITMAP_EDGE {
                    return Err(ModelError::BadSize);
                }
            }
            Content::Html { attachments, .. } => {
                for (i, a) in attachments.iter().enumerate() {
                    if !valid_id(&a.id) {
                        return Err(ModelError::BadAttachmentId);
                    }
                    if attachments[..i].iter().any(|b| b.id == a.id) {
                        return Err(ModelError::DuplicateAttachment);
                    }
                }
            }
            Content::File { path } => {
                if path.trim().is_empty() {
                    return Err(ModelError::Empty);
                }
            }
        }
        Ok(Self { title: String::new(), content })
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// The custom title, empty when none was set.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The title shown: the custom one, or `file_name` when the reply left the title empty.
    pub fn display_title<'a>(&'a self, file_name: &'a str) -> &'a str {
        if self.title.is_empty() { file_name } else { &self.title }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    /// Resolves a `cid:<id>` URL from an HTML body to its attachment. A bare id does not match.
    pub fn resolve_cid(&self, url: &str) -> Option<&Attachment> {
        let Content::Html { attachments, .. } = &self.content else { return None };
        let id = url.strip_prefix("cid:")?;
        attachments.iter().find(|a| a.id == id)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
}

/// `QLPreviewProvider` registrations: which content types each provider previews (the supported-types list of a
/// preview extension). Registering a provider again replaces its types.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    entries: Vec<(String, Vec<String>)>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `provider` previews `types`. Type identifiers may be system types or the app's own; both must be non-empty.
    pub fn register(&mut self, provider: &str, types: &[&str]) -> Result<(), ModelError> {
        if provider.trim().is_empty() || types.iter().any(|t| t.trim().is_empty()) {
            return Err(ModelError::Empty);
        }
        let types: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        match self.entries.iter_mut().find(|(p, _)| p == provider) {
            Some(entry) => entry.1 = types,
            None => self.entries.push((provider.to_string(), types)),
        }
        Ok(())
    }

    /// The provider for a file name, looked up through its type. `None` for an unknown extension or no match.
    pub fn provider_for(&self, file_name: &str) -> Option<&str> {
        self.provider_for_type(uttype::type_for_extension(file_name)?)
    }

    /// The provider for a type. A registration for the type itself (distance 0) beats one for an ancestor, and a
    /// nearer ancestor beats a farther one. Ties go to the provider registered first.
    pub fn provider_for_type(&self, type_id: &str) -> Option<&str> {
        let mut best: Option<(usize, &str)> = None;
        for (provider, types) in &self.entries {
            for t in types {
                let Some(d) = uttype::distance(type_id, t) else { continue };
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, provider.as_str()));
                }
            }
        }
        best.map(|(_, p)| p)
    }
}

/// `QLThumbnailGenerator.Request.RepresentationTypes`: a size of thumbnail a request can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    Thumbnail,
    LowQuality,
    Icon,
}

impl Representation {
    /// Best first: a full thumbnail beats a low-quality one, which beats an icon.
    pub const ALL: [Self; 3] = [Self::Thumbnail, Self::LowQuality, Self::Icon];
}

/// `QLThumbnailGenerator.Request(fileAt:size:scale:representationTypes:)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThumbnailRequest {
    pub url: String,
    /// Width and height in points.
    pub size: (f32, f32),
    /// Pixels per point of the target display.
    pub scale: f32,
    representations: Vec<Representation>,
}

impl ThumbnailRequest {
    /// Refuses a size or scale that is zero, negative, not finite, or above [`MAX_SCALE`], and a request with no
    /// representation. Repeated representations are kept once.
    pub fn new(url: &str, size: (f32, f32), scale: f32, representations: &[Representation]) -> Result<Self, ModelError> {
        let ok = |v: f32| v.is_finite() && v > 0.0;
        if !(ok(size.0) && ok(size.1) && ok(scale) && scale <= MAX_SCALE) {
            return Err(ModelError::BadSize);
        }
        let mut kept: Vec<Representation> = Vec::new();
        for r in Representation::ALL {
            if representations.contains(&r) {
                kept.push(r);
            }
        }
        if kept.is_empty() {
            return Err(ModelError::NoRepresentation);
        }
        Ok(Self { url: url.to_string(), size, scale, representations: kept })
    }

    /// The requested pixel size, rounded to whole pixels and at least one pixel per edge.
    pub fn pixel_size(&self) -> (u32, u32) {
        let px = |v: f32| (v * self.scale).round().max(1.0) as u32;
        (px(self.size.0), px(self.size.1))
    }

    pub fn wants(&self, r: Representation) -> bool {
        self.representations.contains(&r)
    }

    /// `generateBestRepresentation`: the best requested representation that was produced, if any.
    pub fn best_of(&self, produced: &[Representation]) -> Option<Representation> {
        Representation::ALL.into_iter().find(|r| self.wants(*r) && produced.contains(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(ids: &[&str]) -> Content {
        Content::Html {
            body: "<img src=\"cid:a\">".into(),
            attachments: ids.iter().map(|id| Attachment { id: id.to_string(), data: vec![1], content_type: "image/png".into() }).collect(),
        }
    }

    #[test]
    fn content_sizes_are_validated() {
        assert_eq!(Reply::new(Content::Pdf { width: 0.0, height: 10.0 }), Err(ModelError::BadSize));
        assert_eq!(Reply::new(Content::Pdf { width: f32::NAN, height: 10.0 }), Err(ModelError::BadSize));
        assert_eq!(Reply::new(Content::Bitmap { width: 0, height: 4 }), Err(ModelError::BadSize));
        assert_eq!(Reply::new(Content::Bitmap { width: MAX_BITMAP_EDGE + 1, height: 4 }), Err(ModelError::BadSize));
        assert!(Reply::new(Content::Bitmap { width: 1200, height: 800 }).is_ok());
        assert_eq!(Reply::new(Content::File { path: "  ".into() }), Err(ModelError::Empty));
    }

    #[test]
    fn attachment_ids_are_unique_and_usable_in_urls() {
        assert_eq!(Reply::new(html(&["a", "a"])), Err(ModelError::DuplicateAttachment));
        assert_eq!(Reply::new(html(&["a b"])), Err(ModelError::BadAttachmentId));
        assert_eq!(Reply::new(html(&[""])), Err(ModelError::BadAttachmentId));
        assert!(Reply::new(html(&["a", "b"])).is_ok());
    }

    #[test]
    fn title_falls_back_to_the_file_name() {
        let r = Reply::new(Content::File { path: "/x/notes.md".into() }).unwrap();
        assert_eq!(r.display_title("notes.md"), "notes.md");
        let r = r.with_title("Meeting notes");
        assert_eq!(r.title(), "Meeting notes");
        assert_eq!(r.display_title("notes.md"), "Meeting notes");
    }

    #[test]
    fn cid_urls_resolve_to_attachments_only() {
        let r = Reply::new(html(&["logo"])).unwrap();
        assert_eq!(r.resolve_cid("cid:logo").map(|a| a.content_type.as_str()), Some("image/png"));
        assert!(r.resolve_cid("logo").is_none());
        assert!(r.resolve_cid("cid:other").is_none());
        let pdf = Reply::new(Content::Pdf { width: 612.0, height: 792.0 }).unwrap();
        assert!(pdf.resolve_cid("cid:logo").is_none());
    }

    #[test]
    fn provider_is_chosen_by_the_most_specific_registered_type() {
        let mut reg = Registry::new();
        reg.register("text", &["public.plain-text"]).unwrap();
        reg.register("md", &["net.daringfireball.markdown"]).unwrap();
        assert_eq!(reg.provider_for("notes.md"), Some("md"));
        assert_eq!(reg.provider_for("notes.txt"), Some("text"));
        assert_eq!(reg.provider_for("photo.png"), None);
        assert_eq!(reg.provider_for("no-extension"), None);
    }

    #[test]
    fn exact_types_beat_ancestors_and_registering_again_replaces() {
        let mut reg = Registry::new();
        reg.register("img", &["public.image"]).unwrap();
        assert_eq!(reg.provider_for("photo.png"), Some("img"));
        reg.register("png", &["public.png"]).unwrap();
        assert_eq!(reg.provider_for("photo.png"), Some("png"));
        reg.register("png", &["public.jpeg"]).unwrap();
        assert_eq!(reg.provider_for("photo.png"), Some("img"));
        assert_eq!(reg.provider_for("photo.jpg"), Some("png"));
        assert_eq!(reg.register("", &["public.png"]), Err(ModelError::Empty));
    }

    #[test]
    fn app_defined_types_match_exactly() {
        let mut reg = Registry::new();
        reg.register("custom", &["com.example.thing"]).unwrap();
        assert_eq!(reg.provider_for_type("com.example.thing"), Some("custom"));
        assert_eq!(reg.provider_for_type("com.example.other"), None);
    }

    #[test]
    fn thumbnail_request_pixels_and_scale_limits() {
        let r = ThumbnailRequest::new("file:///a.png", (64.0, 40.5), 2.0, &[Representation::Thumbnail]).unwrap();
        assert_eq!(r.pixel_size(), (128, 81));
        assert_eq!(ThumbnailRequest::new("u", (0.0, 4.0), 1.0, &[Representation::Icon]), Err(ModelError::BadSize));
        assert_eq!(ThumbnailRequest::new("u", (4.0, 4.0), MAX_SCALE + 0.5, &[Representation::Icon]), Err(ModelError::BadSize));
        assert_eq!(ThumbnailRequest::new("u", (4.0, 4.0), 1.0, &[]), Err(ModelError::NoRepresentation));
    }

    #[test]
    fn best_representation_is_the_best_requested_one_produced() {
        let r = ThumbnailRequest::new("u", (64.0, 64.0), 1.0, &[Representation::Icon, Representation::LowQuality]).unwrap();
        assert!(!r.wants(Representation::Thumbnail));
        assert_eq!(r.best_of(&[Representation::Icon, Representation::LowQuality]), Some(Representation::LowQuality));
        assert_eq!(r.best_of(&[Representation::Thumbnail]), None);
        let all = ThumbnailRequest::new("u", (64.0, 64.0), 1.0, &Representation::ALL).unwrap();
        assert_eq!(all.best_of(&[Representation::Icon, Representation::Thumbnail]), Some(Representation::Thumbnail));
    }
}
