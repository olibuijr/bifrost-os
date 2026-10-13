//! The system thumbnail model: one request shape for every app (a file, a size in points, a display scale, and whether
//! an icon or a content thumbnail is wanted), providers registered by content type, and the cache key that makes a
//! rendered thumbnail reusable (path, modification time and size of the file, plus the pixel size and kind).
//!
//! The model is pure. Decoding and drawing happen in the providers the shell registers (`springboard::thumbs`); the
//! shell service answers requests from them and keeps the pixels in a disk cache under the key made here.
//!
//! * [`ThumbRequest`] is what an app asks for. [`ThumbRequest::px`] is the edge of the square the thumbnail has to fit,
//!   in pixels, bounded by [`MIN_PX`] and [`MAX_PX`].
//! * A [`ThumbProvider`] names the content types it handles ([`crate::uttype`] identifiers; a type is also handled when
//!   it conforms to one of them) and answers with a [`ThumbArt`]: pixels, or a type icon with an optional badge.
//! * [`Registry::render`] picks the provider whose registered type is nearest to the file's type, and falls back to the
//!   type icon with a badge when there is none or the provider cannot draw the file.

use crate::uttype;
use std::path::PathBuf;

/// Smallest thumbnail edge in pixels.
pub const MIN_PX: u32 = 16;
/// Largest thumbnail edge in pixels.
pub const MAX_PX: u32 = 1024;
/// Longest badge text, in characters.
pub const MAX_BADGE: usize = 5;

/// What the caller wants drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThumbKind {
    /// The icon of the file's type (with a badge for the extension where it helps).
    Icon,
    /// A picture of the file's content, falling back to the icon when the content cannot be drawn.
    Thumbnail,
}

impl ThumbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ThumbKind::Icon => "icon",
            ThumbKind::Thumbnail => "thumbnail",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "icon" => Some(ThumbKind::Icon),
            "thumbnail" => Some(ThumbKind::Thumbnail),
            _ => None,
        }
    }
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The path is empty, relative or has a `..` part.
    BadPath,
    /// The size is zero, or the scale is not a positive finite number up to 8.
    BadSize,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::BadPath => f.write_str("the path must be absolute and have no .. part"),
            RequestError::BadSize => f.write_str("the size must be positive and the scale between 0 and 8"),
        }
    }
}

/// A thumbnail request: the file, the size in points, the display scale and the kind.
#[derive(Clone, Debug, PartialEq)]
pub struct ThumbRequest {
    pub path: String,
    /// Edge of the square the thumbnail has to fit, in points.
    pub size: u32,
    /// Pixels per point of the target display.
    pub scale: f32,
    pub kind: ThumbKind,
}

impl ThumbRequest {
    pub fn new(path: &str, size: u32, scale: f32, kind: ThumbKind) -> Result<Self, RequestError> {
        let p = std::path::Path::new(path);
        let clean = p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir));
        if !clean {
            return Err(RequestError::BadPath);
        }
        if size == 0 || !(scale.is_finite() && scale > 0.0 && scale <= 8.0) {
            return Err(RequestError::BadSize);
        }
        Ok(ThumbRequest { path: path.to_string(), size, scale, kind })
    }

    /// The edge of the thumbnail in pixels: `size` times `scale`, rounded, between [`MIN_PX`] and [`MAX_PX`].
    pub fn px(&self) -> u32 {
        ((self.size as f32 * self.scale).round() as u32).clamp(MIN_PX, MAX_PX)
    }
}

/// The part of a file's identity that makes a cached thumbnail stale when it changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileStamp {
    /// Modification time, Unix seconds.
    pub mtime: i64,
    pub size: u64,
}

/// The cache key of a thumbnail: path, modification time and size of the file, the pixel edge and the kind. 16 hex
/// digits (FNV-1a), so it is a safe file name.
pub fn cache_key(path: &str, stamp: FileStamp, px: u32, kind: ThumbKind) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(path.as_bytes());
    eat(&[0]);
    eat(&stamp.mtime.to_le_bytes());
    eat(&stamp.size.to_le_bytes());
    eat(&px.to_le_bytes());
    eat(kind.as_str().as_bytes());
    format!("{h:016x}")
}

/// Premultiplied pixels, one `u32` per pixel (the render image layout).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Pixels {
    /// Refuses an empty image, a side over [`MAX_PX`] and a pixel vector of the wrong length.
    pub fn new(w: usize, h: usize, px: Vec<u32>) -> Option<Self> {
        let ok = w > 0 && h > 0 && w <= MAX_PX as usize && h <= MAX_PX as usize && px.len() == w * h;
        ok.then_some(Pixels { w, h, px })
    }
}

/// What a provider made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbArt {
    /// A picture of the content.
    Pixels(Pixels),
    /// The icon of a content type, with a short badge such as the extension. The caller draws the icon.
    Icon { type_id: String, badge: Option<String> },
}

/// The file a provider draws.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    /// The content type, from the extension (`public.data` when unknown).
    pub type_id: String,
    pub stamp: FileStamp,
}

impl SourceFile {
    pub fn new(path: PathBuf, stamp: FileStamp) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let type_id = uttype::type_for_extension(&name).unwrap_or("public.data").to_string();
        SourceFile { path, type_id, stamp }
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// A thumbnail provider: draws files of the content types it names.
pub trait ThumbProvider {
    /// A short name for logs and the parity notes.
    fn name(&self) -> &'static str;
    /// The content types handled; a file whose type conforms to one of them is handled too.
    fn types(&self) -> &'static [&'static str];
    /// Draws `file` into a square of `px` pixels. `None` means "cannot draw this one": the type icon is used.
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt>;
}

/// The extension in capitals as a badge ("DOCX"), when it is short enough to read on an icon.
pub fn badge_for(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() || ext.chars().count() > MAX_BADGE || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext.to_ascii_uppercase())
}

/// The icon answer for a file: its type and the extension badge.
pub fn icon_for(file: &SourceFile) -> ThumbArt {
    ThumbArt::Icon { type_id: file.type_id.clone(), badge: badge_for(&file.name()) }
}

/// Providers by content type.
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn ThumbProvider>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// Adds a provider. When two providers name the same distance to a file's type the earlier one wins.
    pub fn register(&mut self, p: Box<dyn ThumbProvider>) {
        self.providers.push(p);
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// The provider whose registered type is nearest to `type_id` in the conformance tree.
    pub fn provider_for(&self, type_id: &str) -> Option<&dyn ThumbProvider> {
        let mut best: Option<(usize, &dyn ThumbProvider)> = None;
        for p in &self.providers {
            let d = p.types().iter().filter_map(|t| uttype::distance(type_id, t)).min();
            if let Some(d) = d {
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, p.as_ref()));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// The art for `file`: the icon for [`ThumbKind::Icon`]; for a thumbnail the provider's pixels, else the icon.
    pub fn render(&self, file: &SourceFile, px: u32, kind: ThumbKind) -> ThumbArt {
        if kind == ThumbKind::Thumbnail {
            if let Some(art) = self.provider_for(&file.type_id).and_then(|p| p.render(file, px)) {
                return art;
            }
        }
        icon_for(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static [&'static str], bool);
    impl ThumbProvider for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn types(&self) -> &'static [&'static str] {
            self.0
        }
        fn render(&self, _: &SourceFile, _: u32) -> Option<ThumbArt> {
            self.1.then(|| ThumbArt::Pixels(Pixels::new(1, 1, vec![7]).unwrap()))
        }
    }

    fn file(name: &str) -> SourceFile {
        SourceFile::new(PathBuf::from(format!("/d/{name}")), FileStamp { mtime: 5, size: 9 })
    }

    #[test]
    fn requests_need_a_clean_absolute_path_and_a_sane_size() {
        assert!(ThumbRequest::new("/a/b.png", 64, 2.0, ThumbKind::Thumbnail).is_ok());
        assert_eq!(ThumbRequest::new("a/b.png", 64, 2.0, ThumbKind::Icon), Err(RequestError::BadPath));
        assert_eq!(ThumbRequest::new("/a/../b", 64, 2.0, ThumbKind::Icon), Err(RequestError::BadPath));
        assert_eq!(ThumbRequest::new("", 64, 2.0, ThumbKind::Icon), Err(RequestError::BadPath));
        assert_eq!(ThumbRequest::new("/a", 0, 2.0, ThumbKind::Icon), Err(RequestError::BadSize));
        assert_eq!(ThumbRequest::new("/a", 8, f32::NAN, ThumbKind::Icon), Err(RequestError::BadSize));
        assert_eq!(ThumbRequest::new("/a", 8, 9.0, ThumbKind::Icon), Err(RequestError::BadSize));
    }

    #[test]
    fn the_pixel_edge_is_size_times_scale_within_bounds() {
        let r = |s, sc| ThumbRequest::new("/a", s, sc, ThumbKind::Thumbnail).unwrap().px();
        assert_eq!(r(64, 2.0), 128);
        assert_eq!(r(1, 1.0), MIN_PX);
        assert_eq!(r(4000, 2.0), MAX_PX);
        assert_eq!(r(10, 1.25), 13u32.max(MIN_PX));
    }

    #[test]
    fn kinds_round_trip_through_their_names() {
        for k in [ThumbKind::Icon, ThumbKind::Thumbnail] {
            assert_eq!(ThumbKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(ThumbKind::parse("poster"), None);
    }

    #[test]
    fn the_cache_key_changes_with_every_part_of_the_stamp() {
        let s = FileStamp { mtime: 1, size: 2 };
        let k = cache_key("/a", s, 64, ThumbKind::Thumbnail);
        assert_eq!(k.len(), 16);
        assert_eq!(k, cache_key("/a", s, 64, ThumbKind::Thumbnail));
        assert_ne!(k, cache_key("/b", s, 64, ThumbKind::Thumbnail));
        assert_ne!(k, cache_key("/a", FileStamp { mtime: 2, size: 2 }, 64, ThumbKind::Thumbnail));
        assert_ne!(k, cache_key("/a", FileStamp { mtime: 1, size: 3 }, 64, ThumbKind::Thumbnail));
        assert_ne!(k, cache_key("/a", s, 65, ThumbKind::Thumbnail));
        assert_ne!(k, cache_key("/a", s, 64, ThumbKind::Icon));
    }

    #[test]
    fn pixels_are_checked() {
        assert!(Pixels::new(2, 2, vec![0; 4]).is_some());
        assert!(Pixels::new(2, 2, vec![0; 3]).is_none());
        assert!(Pixels::new(0, 2, vec![]).is_none());
        assert!(Pixels::new(MAX_PX as usize + 1, 1, vec![0; MAX_PX as usize + 1]).is_none());
    }

    #[test]
    fn badges_are_short_upper_case_extensions() {
        assert_eq!(badge_for("a.docx").as_deref(), Some("DOCX"));
        assert_eq!(badge_for("a.tar.gz").as_deref(), Some("GZ"));
        assert_eq!(badge_for("a.verylongext"), None);
        assert_eq!(badge_for("noext"), None);
        assert_eq!(badge_for(".hidden"), None);
    }

    #[test]
    fn the_nearest_registered_type_wins() {
        let mut r = Registry::new();
        r.register(Box::new(Fixed(&["public.text"], false)));
        r.register(Box::new(Fixed(&["public.plain-text"], true)));
        // a .txt file is plain text: distance 0 to the second, 1 to the first
        assert!(r.provider_for("public.plain-text").unwrap().render(&file("a.txt"), 32).is_some());
        // html is text but not plain text
        assert!(r.provider_for("public.html").unwrap().render(&file("a.html"), 32).is_none());
        assert!(r.provider_for("public.png").is_none());
    }

    #[test]
    fn a_provider_that_cannot_draw_falls_back_to_the_icon() {
        let mut r = Registry::new();
        r.register(Box::new(Fixed(&["public.image"], false)));
        match r.render(&file("a.png"), 32, ThumbKind::Thumbnail) {
            ThumbArt::Icon { type_id, badge } => {
                assert_eq!(type_id, "public.png");
                assert_eq!(badge.as_deref(), Some("PNG"));
            }
            other => panic!("expected the icon, got {other:?}"),
        }
    }

    #[test]
    fn the_icon_kind_never_calls_a_provider() {
        let mut r = Registry::new();
        r.register(Box::new(Fixed(&["public.image"], true)));
        assert!(matches!(r.render(&file("a.png"), 32, ThumbKind::Icon), ThumbArt::Icon { .. }));
        assert!(matches!(r.render(&file("a.png"), 32, ThumbKind::Thumbnail), ThumbArt::Pixels(_)));
    }
}
