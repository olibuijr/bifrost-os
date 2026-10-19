//! The system thumbnail providers and generator. The model (request, providers by content type, cache key) is
//! `bifrost_ui::thumb_service`; the shell serves it as the `thumbnails` service (`shell/services/thumbnails.rs`) and
//! apps ask through `bifrost_app::thumbnails`.
//!
//! Providers (registered by content type; the nearest registered type to the file's type wins):
//!
//! | provider | content types | what it draws |
//! |---|---|---|
//! | [`images::ImageProvider`] | `public.image` | the picture, scaled to fit |
//! | [`text::TextProvider`] | `public.text` | the first page, laid out with the typesetter |
//! | [`text::RichTextProvider`] | `public.rtf` | the first page of the text |
//! | [`text::WebProvider`] | `public.html` | the title and text of the page (no rendering of markup) |
//! | [`images::FontProvider`] | `public.font` | sample glyphs drawn with the font |
//! | [`images::OfficeProvider`] | word processing, spreadsheet and presentation documents | the picture the document embeds |
//! | [`cards::PackageProvider`] | `public.archive` | the type icon with a format badge |
//! | [`cards::ContactProvider`] | `public.vcard` | a contact card |
//! | [`cards::CalendarProvider`] | `public.calendar-event` | a calendar page |
//! | [`media::AudiovisualProvider`] | `public.audiovisual-content` | the cover picture the file embeds, else a video's first key frame or an audio waveform card; each with the running time badge ([`duration`]) |
//! | [`pdf::PdfProvider`] | `com.adobe.pdf` | the first page, drawn by the PDF renderer |
//! | [`text::ClippingsProvider`] | text clippings and web locations | the clipped text on a tabbed page |
//!
//! Thumbnails are kept as raw images in a disk cache named by [`bifrost_ui::thumb_service::cache_key`].

pub mod canvas;
pub mod cards;
pub mod duration;
pub mod images;
pub mod media;
pub mod pdf;
pub mod text;

use crate::shell::photos::thumbs::DiskCache;
use bifrost_ui::thumb_service::{cache_key, FileStamp, Registry, SourceFile, ThumbArt, ThumbKind, ThumbRequest};
use std::path::{Path, PathBuf};

/// Thumbnail files the disk cache keeps.
pub const MAX_FILES: usize = 4000;
/// Bytes the disk cache may take.
pub const MAX_BYTES: u64 = 256 << 20;

/// Every provider of the system.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    r.register(Box::new(images::ImageProvider));
    r.register(Box::new(text::TextProvider));
    r.register(Box::new(text::RichTextProvider));
    r.register(Box::new(text::WebProvider));
    r.register(Box::new(images::FontProvider));
    r.register(Box::new(images::OfficeProvider));
    r.register(Box::new(cards::PackageProvider));
    r.register(Box::new(cards::ContactProvider));
    r.register(Box::new(cards::CalendarProvider));
    r.register(Box::new(media::AudiovisualProvider));
    r.register(Box::new(pdf::PdfProvider));
    r.register(Box::new(text::ClippingsProvider));
    r
}

/// Where the thumbnail cache lives: `$XDG_CACHE_HOME/bifrost/thumbnails`, else `~/.cache/bifrost/thumbnails`.
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("bifrost").join("thumbnails")
}

/// What a request resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A cached picture: the key, the file holding it (raw image format) and its size.
    Pixels { key: String, path: PathBuf, w: usize, h: usize },
    /// The type icon, with a badge.
    Icon { type_id: String, badge: Option<String> },
}

impl Answer {
    fn of_art(art: ThumbArt) -> Option<Answer> {
        match art {
            ThumbArt::Icon { type_id, badge } => Some(Answer::Icon { type_id, badge }),
            ThumbArt::Pixels(_) => None,
        }
    }
}

/// Resolves requests: cache lookup, then the providers.
pub struct Generator {
    registry: Registry,
    cache: DiskCache,
}

/// The size recorded in a cached raw image's header.
fn raw_size(path: &Path) -> Option<(usize, usize)> {
    use std::io::Read;
    let mut h = [0u8; 12];
    std::fs::File::open(path).ok()?.read_exact(&mut h).ok()?;
    if &h[..4] != b"RIMG" {
        return None;
    }
    Some((u32::from_le_bytes(h[4..8].try_into().ok()?) as usize, u32::from_le_bytes(h[8..12].try_into().ok()?) as usize))
}

/// The file's stamp. A directory or a missing file is an error: the caller asks for files.
pub fn stamp_of(path: &Path) -> Result<(FileStamp, bool), String> {
    let md = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    Ok((FileStamp { mtime, size: md.len() }, md.is_dir()))
}

impl Generator {
    pub fn new(cache: DiskCache) -> Self {
        Generator { registry: registry(), cache }
    }

    /// A generator over the default cache directory.
    pub fn standard() -> Self {
        Generator::new(DiskCache::new(cache_dir(), MAX_FILES).with_max_bytes(MAX_BYTES))
    }

    pub fn cache(&self) -> &DiskCache {
        &self.cache
    }

    /// How far the built-in provider for the file named `file_name` is from the file's own type (0 when a provider
    /// names that very type). `None` when no built-in provider draws it: the type icon is all it gets.
    pub fn provider_distance(&self, file_name: &str) -> Option<usize> {
        let type_id = bifrost_ui::uttype::type_for_extension(file_name).unwrap_or("public.data");
        self.registry.distance_for(type_id)
    }

    /// The cache key and the answer when it needs no drawing: the icon kind, a folder, or a cache hit.
    pub fn quick(&self, req: &ThumbRequest) -> Result<(String, Option<Answer>), String> {
        let path = Path::new(&req.path);
        if req.kind == ThumbKind::Icon {
            let file = SourceFile::new(path.to_path_buf(), FileStamp::default());
            let key = cache_key(&req.path, FileStamp::default(), req.px(), req.kind);
            return Ok((key, Answer::of_art(bifrost_ui::thumb_service::icon_for(&file))));
        }
        let (stamp, is_dir) = stamp_of(path)?;
        let key = cache_key(&req.path, stamp, req.px(), req.kind);
        if is_dir {
            return Ok((key, Some(Answer::Icon { type_id: "public.folder".into(), badge: None })));
        }
        // a type no provider draws is answered with its icon at once, without a worker
        let file = SourceFile::new(path.to_path_buf(), stamp);
        if self.registry.provider_for(&file.type_id).is_none() {
            return Ok((key, Answer::of_art(bifrost_ui::thumb_service::icon_for(&file))));
        }
        let cached = self.cache.path(&key);
        if let Some((w, h)) = raw_size(&cached) {
            return Ok((key.clone(), Some(Answer::Pixels { key, path: cached, w, h })));
        }
        Ok((key, None))
    }

    /// Draws the thumbnail (blocking: decoding and layout) and caches the pixels.
    pub fn generate(&self, req: &ThumbRequest) -> Result<Answer, String> {
        let (key, quick) = self.quick(req)?;
        if let Some(a) = quick {
            return Ok(a);
        }
        let (stamp, _) = stamp_of(Path::new(&req.path))?;
        let file = SourceFile::new(PathBuf::from(&req.path), stamp);
        match self.registry.render(&file, req.px(), req.kind) {
            ThumbArt::Pixels(p) => {
                let img = bifrost_render::gfx::Image { w: p.w, h: p.h, px: p.px };
                self.cache.write(&key, &img);
                Ok(Answer::Pixels { key: key.clone(), path: self.cache.path(&key), w: img.w, h: img.h })
            }
            ThumbArt::Icon { type_id, badge } => Ok(Answer::Icon { type_id, badge }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::thumb_service::ThumbProvider;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-thumbs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 40, 20);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&vec![90u8; 40 * 20 * 4]).unwrap();
        }
        out
    }

    fn generator(dir: &Path) -> Generator {
        Generator::new(DiskCache::new(dir.join("cache"), 100))
    }

    #[test]
    fn every_provider_is_registered_once_and_picked_by_type() {
        let r = registry();
        assert_eq!(r.len(), 12);
        let name = |t: &str| r.provider_for(t).map(ThumbProvider::name);
        assert_eq!(name("public.png"), Some("image"));
        assert_eq!(name("public.rust-source"), Some("text"));
        assert_eq!(name("public.rtf"), Some("rich-text"));
        assert_eq!(name("public.html"), Some("web"));
        assert_eq!(name("public.truetype-ttf-font"), Some("font"));
        assert_eq!(name("org.openxmlformats.wordprocessingml.document"), Some("office"));
        assert_eq!(name("org.oasis-open.opendocument.presentation"), Some("office"));
        assert_eq!(name("public.zip-archive"), Some("package"));
        assert_eq!(name("org.debian.deb-archive"), Some("package"));
        assert_eq!(name("public.vcard"), Some("contact"));
        assert_eq!(name("public.calendar-event"), Some("calendar"));
        assert_eq!(name("public.mp3"), Some("audiovisual"));
        assert_eq!(name("public.mpeg-4"), Some("audiovisual"));
        assert_eq!(name("com.adobe.pdf"), Some("pdf"));
        assert_eq!(name("bifrost.text-clipping"), Some("clippings"));
        assert_eq!(name("bifrost.web-location"), Some("clippings"));
        // a vector picture is both an image and XML text: the image provider is nearer
        assert_eq!(name("public.svg-image"), Some("image"));
        assert_eq!(name("public.folder"), None);
    }

    #[test]
    fn a_picture_is_drawn_once_and_then_served_from_the_cache() {
        let dir = scratch("cache");
        let file = dir.join("a.png");
        std::fs::write(&file, png()).unwrap();
        let g = generator(&dir);
        let req = ThumbRequest::new(file.to_str().unwrap(), 10, 1.0, ThumbKind::Thumbnail).unwrap();
        assert!(g.quick(&req).unwrap().1.is_none(), "nothing cached yet");
        let first = g.generate(&req).unwrap();
        let Answer::Pixels { w, h, path, .. } = &first else { panic!("pixels, got {first:?}") };
        assert_eq!((*w, *h), (16, 8), "40x20 fitted to the minimum edge of 16 px");
        assert!(path.exists());
        assert_eq!(g.quick(&req).unwrap().1, Some(first.clone()));
        // a changed file is a new key
        std::fs::write(&file, [png(), vec![0]].concat()).unwrap();
        assert!(g.quick(&req).unwrap().1.is_none());
    }

    #[test]
    fn the_icon_kind_needs_no_file() {
        let g = generator(&scratch("icon"));
        let req = ThumbRequest::new("/nowhere/x.docx", 32, 2.0, ThumbKind::Icon).unwrap();
        let (_, a) = g.quick(&req).unwrap();
        assert_eq!(a, Some(Answer::Icon { type_id: "org.openxmlformats.wordprocessingml.document".into(), badge: Some("DOCX".into()) }));
    }

    #[test]
    fn a_missing_file_is_an_error_and_a_folder_is_a_folder_icon() {
        let dir = scratch("missing");
        let g = generator(&dir);
        let missing = ThumbRequest::new(dir.join("nope.png").to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert!(g.generate(&missing).is_err());
        let folder = ThumbRequest::new(dir.to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert_eq!(g.generate(&folder).unwrap(), Answer::Icon { type_id: "public.folder".into(), badge: None });
    }

    #[test]
    fn an_undrawable_file_falls_back_to_its_icon() {
        let dir = scratch("fallback");
        let file = dir.join("bad.png");
        std::fs::write(&file, b"not a png").unwrap();
        let req = ThumbRequest::new(file.to_str().unwrap(), 32, 1.0, ThumbKind::Thumbnail).unwrap();
        assert_eq!(generator(&dir).generate(&req).unwrap(), Answer::Icon { type_id: "public.png".into(), badge: Some("PNG".into()) });
    }

    #[test]
    fn the_cache_directory_follows_the_environment() {
        assert!(cache_dir().ends_with("bifrost/thumbnails"));
    }
}
