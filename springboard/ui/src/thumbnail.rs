//! Thumbnails of custom file types (`QLThumbnailProvider`, `QLFileThumbnailRequest`, `QLThumbnailReply`), the
//! generator's error codes (`QLThumbnailError.Code`) and progressive updates (`generateRepresentations(for:update:)`),
//! as pure models.
//!
//! An app that owns a file type answers a [`FileThumbnailRequest`] with a [`ThumbnailReply`]: a drawing in a context of
//! a given size, or an image file. [`ThumbnailReply::accepted_by`] checks the drawing against the size range the request
//! allowed. The generator reports failures as a [`ThumbnailError`]. A [`Progressive`] collects the representations of
//! one request as they are produced, so the caller shows the best one so far and ignores anything worse.
//! The representation types themselves are [`crate::preview::Representation`].

use crate::preview::{ModelError, Representation, ThumbnailRequest, MAX_SCALE};

/// `QLFileThumbnailRequest`: the file to draw, the size range the provider may answer in, and the display scale.
#[derive(Clone, Debug, PartialEq)]
pub struct FileThumbnailRequest {
    pub file_url: String,
    /// Smallest accepted size in points, width and height.
    pub minimum_size: (f32, f32),
    /// Largest accepted size in points, width and height.
    pub maximum_size: (f32, f32),
    /// Pixels per point of the target display.
    pub scale: f32,
}

impl FileThumbnailRequest {
    /// Refuses an empty file URL, a size that is zero, negative or not finite, a minimum above the maximum on either
    /// axis, and a scale outside (0, [`MAX_SCALE`]].
    pub fn new(file_url: &str, minimum_size: (f32, f32), maximum_size: (f32, f32), scale: f32) -> Result<Self, ModelError> {
        if file_url.trim().is_empty() {
            return Err(ModelError::Empty);
        }
        let ok = |v: f32| v.is_finite() && v > 0.0;
        let sizes_ok = [minimum_size.0, minimum_size.1, maximum_size.0, maximum_size.1].into_iter().all(ok);
        if !sizes_ok || !ok(scale) || scale > MAX_SCALE {
            return Err(ModelError::BadSize);
        }
        if minimum_size.0 > maximum_size.0 || minimum_size.1 > maximum_size.1 {
            return Err(ModelError::BadSize);
        }
        Ok(Self { file_url: file_url.to_string(), minimum_size, maximum_size, scale })
    }

    /// The largest size that keeps `aspect` (width over height) and fits the maximum, or `None` when that size is below
    /// the minimum on either axis, or when `aspect` is not a positive finite number. Use it to pick the drawing size.
    pub fn fit(&self, aspect: f32) -> Option<(f32, f32)> {
        if !(aspect.is_finite() && aspect > 0.0) {
            return None;
        }
        let (max_w, max_h) = self.maximum_size;
        let (w, h) = if max_w / max_h > aspect { (max_h * aspect, max_h) } else { (max_w, max_w / aspect) };
        if w < self.minimum_size.0 || h < self.minimum_size.1 {
            return None;
        }
        Some((w, h))
    }
}

/// What a reply draws: `init(contextSize:drawing:)` or `init(imageFileURL:)`.
#[derive(Clone, Debug, PartialEq)]
pub enum ThumbnailContent {
    /// A drawing made in a context of `size` points, width and height.
    Drawing { size: (f32, f32) },
    /// An image file at `path` that the system scales.
    ImageFile { path: String },
}

/// `QLThumbnailReply`: the content and an optional short `extensionBadge` the system draws on an icon thumbnail.
#[derive(Clone, Debug, PartialEq)]
pub struct ThumbnailReply {
    content: ThumbnailContent,
    extension_badge: Option<String>,
}

impl ThumbnailReply {
    /// A drawing of `size` points. Refuses a width or height that is zero, negative or not finite.
    pub fn drawing(size: (f32, f32)) -> Result<Self, ModelError> {
        let ok = |v: f32| v.is_finite() && v > 0.0;
        if !(ok(size.0) && ok(size.1)) {
            return Err(ModelError::BadSize);
        }
        Ok(Self { content: ThumbnailContent::Drawing { size }, extension_badge: None })
    }

    /// An image file reply. Refuses an empty path.
    pub fn image_file(path: &str) -> Result<Self, ModelError> {
        if path.trim().is_empty() {
            return Err(ModelError::Empty);
        }
        Ok(Self { content: ThumbnailContent::ImageFile { path: path.to_string() }, extension_badge: None })
    }

    /// Sets the badge. Surrounding spaces are removed, and a badge that is empty after that is refused.
    pub fn with_extension_badge(mut self, badge: &str) -> Result<Self, ModelError> {
        let badge = badge.trim();
        if badge.is_empty() {
            return Err(ModelError::Empty);
        }
        self.extension_badge = Some(badge.to_string());
        Ok(self)
    }

    pub fn content(&self) -> &ThumbnailContent {
        &self.content
    }

    pub fn extension_badge(&self) -> Option<&str> {
        self.extension_badge.as_deref()
    }

    /// Whether the reply is allowed for `request`. An image file always is. A drawing must lie within the request's
    /// minimum and maximum size on both axes.
    pub fn accepted_by(&self, request: &FileThumbnailRequest) -> bool {
        match &self.content {
            ThumbnailContent::ImageFile { .. } => true,
            ThumbnailContent::Drawing { size } => {
                let within = |v: f32, lo: f32, hi: f32| v >= lo && v <= hi;
                within(size.0, request.minimum_size.0, request.maximum_size.0)
                    && within(size.1, request.minimum_size.1, request.maximum_size.1)
            }
        }
    }
}

/// `QLThumbnailError.Code`: the failures a thumbnail generation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbnailError {
    GenerationFailed,
    NoCachedThumbnail,
    NoCloudThumbnail,
    RequestCancelled,
    RequestInvalid,
    SavingToUrlFailed,
}

impl ThumbnailError {
    /// Every code, in the order the official reference lists them.
    pub const ALL: [Self; 6] = [
        Self::GenerationFailed,
        Self::NoCachedThumbnail,
        Self::NoCloudThumbnail,
        Self::RequestCancelled,
        Self::RequestInvalid,
        Self::SavingToUrlFailed,
    ];

    /// A short English description for logs and the caller's own messages.
    pub fn message(self) -> &'static str {
        match self {
            Self::GenerationFailed => "The thumbnail could not be made for this file.",
            Self::NoCachedThumbnail => "No low-quality thumbnail could be made.",
            Self::NoCloudThumbnail => "The thumbnail for a remote file could not be made.",
            Self::RequestCancelled => "The thumbnail request was cancelled.",
            Self::RequestInvalid => "The thumbnail request is invalid, for example the file does not exist.",
            Self::SavingToUrlFailed => "The thumbnail could not be saved to the given location.",
        }
    }

    pub fn is_cancellation(self) -> bool {
        self == Self::RequestCancelled
    }
}

/// Progressive generation for one [`ThumbnailRequest`]: the representations arrive one at a time, best kept.
#[derive(Clone, Debug)]
pub struct Progressive {
    request: ThumbnailRequest,
    best: Option<Representation>,
    cancelled: bool,
}

impl Progressive {
    pub fn new(request: ThumbnailRequest) -> Self {
        Self { request, best: None, cancelled: false }
    }

    /// Offers a produced representation. Returns `Ok(Some(r))` when `r` is better than anything shown so far, so the
    /// caller should update its view, and `Ok(None)` when it is not requested or is no better. A cancelled session
    /// refuses every offer with [`ThumbnailError::RequestCancelled`].
    pub fn offer(&mut self, produced: Representation) -> Result<Option<Representation>, ThumbnailError> {
        if self.cancelled {
            return Err(ThumbnailError::RequestCancelled);
        }
        if !self.request.wants(produced) {
            return Ok(None);
        }
        let rank = |r: Representation| Representation::ALL.iter().position(|x| *x == r);
        let better = match self.best {
            None => true,
            Some(current) => rank(produced) < rank(current),
        };
        if better {
            self.best = Some(produced);
            Ok(Some(produced))
        } else {
            Ok(None)
        }
    }

    /// The best representation shown so far.
    pub fn best(&self) -> Option<Representation> {
        self.best
    }

    /// Stops the request. Later offers and [`Progressive::finish`] report a cancellation.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// The final answer: the best representation, or [`ThumbnailError::RequestCancelled`] after a cancel, or
    /// [`ThumbnailError::GenerationFailed`] when nothing that was requested was produced.
    pub fn finish(&self) -> Result<Representation, ThumbnailError> {
        if self.cancelled {
            return Err(ThumbnailError::RequestCancelled);
        }
        self.best.ok_or(ThumbnailError::GenerationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_request(min: (f32, f32), max: (f32, f32)) -> FileThumbnailRequest {
        FileThumbnailRequest::new("file:///a.custom", min, max, 2.0).unwrap()
    }

    #[test]
    fn file_request_refuses_bad_input() {
        assert_eq!(FileThumbnailRequest::new(" ", (8.0, 8.0), (64.0, 64.0), 1.0), Err(ModelError::Empty));
        assert_eq!(FileThumbnailRequest::new("u", (0.0, 8.0), (64.0, 64.0), 1.0), Err(ModelError::BadSize));
        assert_eq!(FileThumbnailRequest::new("u", (128.0, 8.0), (64.0, 64.0), 1.0), Err(ModelError::BadSize));
        assert_eq!(FileThumbnailRequest::new("u", (8.0, 8.0), (64.0, 64.0), MAX_SCALE + 1.0), Err(ModelError::BadSize));
        assert!(FileThumbnailRequest::new("u", (8.0, 8.0), (64.0, 64.0), MAX_SCALE).is_ok());
    }

    #[test]
    fn fit_keeps_the_aspect_inside_the_maximum() {
        let r = file_request((8.0, 8.0), (512.0, 512.0));
        assert_eq!(r.fit(2.0), Some((512.0, 256.0)));
        assert_eq!(r.fit(0.5), Some((256.0, 512.0)));
        assert_eq!(r.fit(1.0), Some((512.0, 512.0)));
        assert_eq!(r.fit(0.0), None);
        assert_eq!(r.fit(f32::NAN), None);
    }

    #[test]
    fn fit_refuses_when_the_minimum_cannot_be_met() {
        let r = file_request((64.0, 64.0), (512.0, 512.0));
        assert_eq!(r.fit(1000.0), None);
        assert_eq!(r.fit(1.0), Some((512.0, 512.0)));
    }

    #[test]
    fn reply_content_is_validated() {
        assert_eq!(ThumbnailReply::drawing((0.0, 10.0)), Err(ModelError::BadSize));
        assert_eq!(ThumbnailReply::drawing((f32::INFINITY, 10.0)), Err(ModelError::BadSize));
        assert_eq!(ThumbnailReply::image_file("  "), Err(ModelError::Empty));
        let r = ThumbnailReply::drawing((64.0, 64.0)).unwrap();
        assert_eq!(r.with_extension_badge("   "), Err(ModelError::Empty));
        let r = ThumbnailReply::drawing((64.0, 64.0)).unwrap().with_extension_badge(" ab ").unwrap();
        assert_eq!(r.extension_badge(), Some("ab"));
    }

    #[test]
    fn a_drawing_must_fit_the_requested_size_range() {
        let req = file_request((32.0, 32.0), (128.0, 128.0));
        assert!(ThumbnailReply::drawing((64.0, 96.0)).unwrap().accepted_by(&req));
        assert!(!ThumbnailReply::drawing((16.0, 96.0)).unwrap().accepted_by(&req));
        assert!(!ThumbnailReply::drawing((64.0, 256.0)).unwrap().accepted_by(&req));
        assert!(ThumbnailReply::image_file("/tmp/x.png").unwrap().accepted_by(&req));
    }

    #[test]
    fn error_codes_have_distinct_messages_and_one_cancellation() {
        assert_eq!(ThumbnailError::ALL.len(), 6);
        for (i, a) in ThumbnailError::ALL.iter().enumerate() {
            for b in &ThumbnailError::ALL[i + 1..] {
                assert_ne!(a.message(), b.message());
            }
        }
        let cancels: Vec<_> = ThumbnailError::ALL.iter().filter(|e| e.is_cancellation()).collect();
        assert_eq!(cancels, vec![&ThumbnailError::RequestCancelled]);
    }

    #[test]
    fn progressive_updates_only_for_better_representations() {
        let req = ThumbnailRequest::new("u", (64.0, 64.0), 1.0, &Representation::ALL).unwrap();
        let mut p = Progressive::new(req);
        assert_eq!(p.offer(Representation::Icon), Ok(Some(Representation::Icon)));
        assert_eq!(p.offer(Representation::Thumbnail), Ok(Some(Representation::Thumbnail)));
        assert_eq!(p.offer(Representation::LowQuality), Ok(None));
        assert_eq!(p.best(), Some(Representation::Thumbnail));
        assert_eq!(p.finish(), Ok(Representation::Thumbnail));
    }

    #[test]
    fn progressive_ignores_representations_that_were_not_requested() {
        let req = ThumbnailRequest::new("u", (64.0, 64.0), 1.0, &[Representation::Icon]).unwrap();
        let mut p = Progressive::new(req);
        assert_eq!(p.offer(Representation::Thumbnail), Ok(None));
        assert_eq!(p.finish(), Err(ThumbnailError::GenerationFailed));
        assert_eq!(p.offer(Representation::Icon), Ok(Some(Representation::Icon)));
        assert_eq!(p.finish(), Ok(Representation::Icon));
    }

    #[test]
    fn cancel_refuses_later_offers_and_reports_the_cancellation() {
        let req = ThumbnailRequest::new("u", (64.0, 64.0), 1.0, &Representation::ALL).unwrap();
        let mut p = Progressive::new(req);
        p.offer(Representation::Icon).unwrap();
        p.cancel();
        assert_eq!(p.offer(Representation::Thumbnail), Err(ThumbnailError::RequestCancelled));
        assert_eq!(p.finish(), Err(ThumbnailError::RequestCancelled));
        assert!(ThumbnailError::RequestCancelled.is_cancellation());
    }
}
