//! Texture loading for GPU views: the app-facing counterpart of a texture loader. A [`TextureLoader`] makes a [`Texture`]
//! (a descriptor and its pixel levels, ready for upload) from encoded image bytes, a file, a named entry of a
//! `TextureCatalog` with scale variants, or a decoded `Image`, with the options of `bifrost_render::texture` (sRGB,
//! mipmap allocation and generation, origin, usage). `load_async` decodes a batch on a background thread and hands the
//! results back through a [`PendingTextures`] handle the caller polls.
//!
//! Pictures are checked from their headers before any pixel is decoded, so an oversized picture is refused without
//! allocating it. The pixels stay on the CPU, premultiplied RGBA as in `bifrost_render::gfx::Image`; the shell's GLES
//! presenter uploads them when a view is attached to it. See docs/bifrost-ui.md "GPU kits" for the gaps.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use bifrost_render::gfx::Image;
use bifrost_render::texture::{self as rt, Prepared};
use bifrost_ui::texture_catalog::TextureCatalog;

use crate::image_source::{read_file, ImageSource, SourceError};

pub use bifrost_render::texture::{CacheMode, LoadOptions, Origin, Storage, TextureDescriptor, TextureError, Usage};

/// The largest picture file `from_file` reads.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// The most pixels a loaded picture may have.
pub const MAX_PIXELS: usize = 1 << 26;

/// Why a texture could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The picture could not be read or decoded (not an image, truncated, too large, file missing).
    Source(SourceError),
    /// The decoded picture does not fit the texture limits.
    Texture(TextureError),
    /// The catalog has no entry with that name.
    NotFound,
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::Source(e) => write!(f, "texture: the picture cannot be read: {e:?}"),
            LoadError::Texture(e) => write!(f, "texture: {e:?}"),
            LoadError::NotFound => write!(f, "texture: no entry with that name"),
        }
    }
}

impl std::error::Error for LoadError {}

/// A loaded texture: its descriptor and the pixel levels that carry data. With `generate_mipmaps` every level is
/// present; with only `allocate_mipmaps` the chain is reserved by the descriptor but just level 0 holds pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    descriptor: TextureDescriptor,
    levels: Vec<Image>,
    label: String,
}

impl Texture {
    /// Wraps a prepared image.
    pub fn new(prepared: Prepared) -> Texture {
        Texture { descriptor: prepared.descriptor, levels: prepared.levels, label: String::new() }
    }

    pub fn descriptor(&self) -> &TextureDescriptor {
        &self.descriptor
    }

    pub fn width(&self) -> usize {
        self.descriptor.width
    }

    pub fn height(&self) -> usize {
        self.descriptor.height
    }

    /// The levels the descriptor reserves (`mipmapLevelCount`).
    pub fn mip_level_count(&self) -> usize {
        self.descriptor.mip_levels
    }

    /// The levels that hold pixels. Equal to `mip_level_count` when the mipmaps were generated.
    pub fn loaded_levels(&self) -> usize {
        self.levels.len()
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn set_label(&mut self, label: &str) {
        self.label = label.to_string();
    }

    /// The pixels of one level.
    pub fn level(&self, level: usize) -> Option<&Image> {
        self.levels.get(level)
    }

    /// One pixel (premultiplied, `r | g << 8 | b << 16 | a << 24`) of one level.
    pub fn pixel(&self, level: usize, x: usize, y: usize) -> Option<u32> {
        let img = self.levels.get(level)?;
        (x < img.w && y < img.h).then(|| img.px[y * img.w + x])
    }

    /// The pixel under the coordinates (`u`, `v`) in 0..1, clamped to the level's edge. NaN counts as 0.
    pub fn sample_nearest(&self, u: f32, v: f32, level: usize) -> Option<u32> {
        let img = self.levels.get(level)?;
        let pick = |t: f32, n: usize| -> usize { ((t.clamp(0.0, 1.0) * n as f32) as usize).min(n - 1) };
        Some(img.px[pick(v, img.h) * img.w + pick(u, img.w)])
    }

    /// True when any pixel of level 0 is not fully opaque.
    pub fn has_alpha(&self) -> bool {
        self.levels.first().is_some_and(|l| l.px.iter().any(|&p| p >> 24 != 255))
    }

    /// Bytes the loaded levels occupy, four per pixel.
    pub fn byte_len(&self) -> usize {
        self.levels.iter().map(|l| l.px.len() * 4).sum()
    }

    /// Replaces a rectangle of one loaded level (`replace(region:...)`). Returns false, and changes nothing, when the
    /// level holds no pixels or the rectangle does not lie wholly inside it.
    pub fn replace_region(&mut self, level: usize, x: usize, y: usize, src: &Image) -> bool {
        let Some(img) = self.levels.get_mut(level) else { return false };
        if src.w.checked_mul(src.h) != Some(src.px.len()) || x.checked_add(src.w).is_none_or(|e| e > img.w) || y.checked_add(src.h).is_none_or(|e| e > img.h) {
            return false;
        }
        for row in 0..src.h {
            let to = (y + row) * img.w + x;
            img.px[to..to + src.w].copy_from_slice(&src.px[row * src.w..(row + 1) * src.w]);
        }
        true
    }
}

/// Where one texture of a batch comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextureSource {
    /// Encoded picture bytes.
    Bytes(Vec<u8>),
    /// A picture file.
    File(PathBuf),
}

/// Makes textures. The options given to `new` apply to every texture the loader makes.
#[derive(Clone, Debug, Default)]
pub struct TextureLoader {
    options: LoadOptions,
}

impl TextureLoader {
    pub fn new(options: LoadOptions) -> Self {
        TextureLoader { options }
    }

    pub fn options(&self) -> &LoadOptions {
        &self.options
    }

    /// A texture from a decoded image.
    pub fn from_image(&self, img: Image) -> Result<Texture, LoadError> {
        rt::prepare(img, &self.options).map(Texture::new).map_err(LoadError::Texture)
    }

    /// A texture from encoded picture bytes. The header is read first, so a picture above `MAX_PIXELS` or the side limit
    /// is refused before any pixel is decoded.
    pub fn from_bytes(&self, bytes: &[u8]) -> Result<Texture, LoadError> {
        let source = ImageSource::with_max_pixels(bytes, MAX_PIXELS).map_err(LoadError::Source)?;
        let img = source.decode().map_err(LoadError::Source)?;
        self.from_image(img)
    }

    /// A texture from a picture file of at most `MAX_FILE_BYTES`.
    pub fn from_file(&self, path: &Path) -> Result<Texture, LoadError> {
        let bytes = read_file(path, MAX_FILE_BYTES).map_err(LoadError::Source)?;
        self.from_bytes(&bytes)
    }

    /// A texture from a catalog entry: the variant that fits `scale` (see `TextureCatalog::resolve`), read from
    /// `base_dir` joined with the path the catalog holds.
    pub fn from_catalog(&self, catalog: &TextureCatalog, name: &str, scale: f64, base_dir: &Path) -> Result<Texture, LoadError> {
        let path = catalog.resolve(name, scale).ok_or(LoadError::NotFound)?;
        self.from_file(&base_dir.join(path))
    }

    /// Loads a batch on one background thread, in order. The caller polls the returned handle.
    pub fn load_async(&self, sources: Vec<TextureSource>) -> PendingTextures {
        let total = sources.len();
        let (tx, rx) = channel();
        let loader = self.clone();
        std::thread::spawn(move || {
            for (i, source) in sources.into_iter().enumerate() {
                let result = match source {
                    TextureSource::Bytes(b) => loader.from_bytes(&b),
                    TextureSource::File(p) => loader.from_file(&p),
                };
                if tx.send((i, result)).is_err() {
                    return;
                }
            }
        });
        PendingTextures { rx, total, received: 0 }
    }
}

/// The result of one texture of a batch, with its position in the batch.
pub type Loaded = (usize, Result<Texture, LoadError>);

/// A batch being loaded on a background thread.
pub struct PendingTextures {
    rx: Receiver<Loaded>,
    total: usize,
    received: usize,
}

impl PendingTextures {
    /// How many textures the batch holds.
    pub fn total(&self) -> usize {
        self.total
    }

    /// True once every result has been handed out.
    pub fn is_done(&self) -> bool {
        self.received >= self.total
    }

    /// The results that are ready now, without waiting.
    pub fn poll(&mut self) -> Vec<Loaded> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(item) => {
                    self.received += 1;
                    out.push(item);
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Waits for the rest of the batch. If the loading thread stopped early the results it did send are returned.
    pub fn wait(&mut self) -> Vec<Loaded> {
        let mut out = Vec::new();
        while self.received < self.total {
            match self.rx.recv() {
                Ok(item) => {
                    self.received += 1;
                    out.push(item);
                }
                Err(_) => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_source::{representation, FileType, RepProperties};
    use bifrost_render::gfx::pack;

    fn picture() -> Image {
        let px = (0..8u8).map(|i| pack(i * 30, 255 - i * 30, 7, 255)).collect();
        Image { w: 4, h: 2, px }
    }

    fn png(img: &Image) -> Vec<u8> {
        representation(img, FileType::Png, &RepProperties::default()).expect("a PNG of a small picture")
    }

    #[test]
    fn bytes_decode_into_a_single_level_texture() {
        let img = picture();
        let t = TextureLoader::default().from_bytes(&png(&img)).unwrap();
        assert_eq!((t.width(), t.height(), t.mip_level_count(), t.loaded_levels()), (4, 2, 1, 1));
        assert_eq!(t.level(0).unwrap(), &img);
        assert!(!t.has_alpha());
        assert_eq!(t.byte_len(), 32);
        assert_eq!(t.pixel(0, 3, 1), Some(img.px[7]));
        assert_eq!(t.pixel(0, 4, 0), None);
        assert_eq!(t.pixel(1, 0, 0), None);
    }

    #[test]
    fn a_bottom_left_origin_flips_the_rows() {
        let img = picture();
        let loader = TextureLoader::new(LoadOptions { origin: Origin::BottomLeft, ..LoadOptions::default() });
        let t = loader.from_image(img.clone()).unwrap();
        assert_eq!(t.pixel(0, 0, 0), Some(img.px[4]));
        assert_eq!(t.pixel(0, 0, 1), Some(img.px[0]));
    }

    #[test]
    fn generated_mipmaps_fill_the_chain_and_allocated_ones_only_reserve_it() {
        let img = picture();
        let generated = TextureLoader::new(LoadOptions { generate_mipmaps: true, ..LoadOptions::default() }).from_image(img.clone()).unwrap();
        assert_eq!(generated.mip_level_count(), 3);
        assert_eq!(generated.loaded_levels(), 3);
        assert_eq!((generated.level(1).unwrap().w, generated.level(1).unwrap().h), (2, 1));
        assert_eq!((generated.level(2).unwrap().w, generated.level(2).unwrap().h), (1, 1));
        let reserved = TextureLoader::new(LoadOptions { allocate_mipmaps: true, ..LoadOptions::default() }).from_image(img).unwrap();
        assert_eq!((reserved.mip_level_count(), reserved.loaded_levels()), (3, 1));
    }

    #[test]
    fn bad_input_is_refused_with_the_reason() {
        let l = TextureLoader::default();
        assert_eq!(l.from_bytes(b"hello world--").unwrap_err(), LoadError::Source(SourceError::NotImage));
        assert_eq!(l.from_image(Image { w: 0, h: 0, px: vec![] }).unwrap_err(), LoadError::Texture(TextureError::Empty));
        assert_eq!(l.from_image(Image { w: 2, h: 2, px: vec![0; 3] }).unwrap_err(), LoadError::Texture(TextureError::Undecodable));
        assert_eq!(l.from_file(Path::new("/nonexistent/picture.png")).unwrap_err(), LoadError::Source(SourceError::Unreadable));
    }

    #[test]
    fn a_named_texture_picks_the_variant_for_the_scale() {
        let dir = tempfile::tempdir().unwrap();
        let img = picture();
        std::fs::write(dir.path().join("icon@2x.png"), png(&img)).unwrap();
        let mut catalog = TextureCatalog::new();
        assert!(catalog.add("icon", 1.0, "icon.png"));
        assert!(catalog.add("icon", 2.0, "icon@2x.png"));
        let l = TextureLoader::default();
        assert_eq!(l.from_catalog(&catalog, "icon", 2.0, dir.path()).unwrap().level(0).unwrap(), &img);
        // The 1x file was never written, so asking for 1x reports the missing file.
        assert_eq!(l.from_catalog(&catalog, "icon", 1.0, dir.path()).unwrap_err(), LoadError::Source(SourceError::Unreadable));
        assert_eq!(l.from_catalog(&catalog, "other", 1.0, dir.path()).unwrap_err(), LoadError::NotFound);
    }

    #[test]
    fn replacing_a_region_checks_the_bounds() {
        let mut t = TextureLoader::default().from_image(picture()).unwrap();
        let patch = Image { w: 2, h: 1, px: vec![0xff11_2233, 0xff44_5566] };
        assert!(t.replace_region(0, 1, 1, &patch));
        assert_eq!(t.pixel(0, 1, 1), Some(0xff11_2233));
        assert_eq!(t.pixel(0, 2, 1), Some(0xff44_5566));
        let before = t.clone();
        assert!(!t.replace_region(0, 3, 1, &patch), "runs past the right edge");
        assert!(!t.replace_region(0, 0, 2, &patch), "below the bottom edge");
        assert!(!t.replace_region(0, usize::MAX, 0, &patch), "an offset that overflows");
        assert!(!t.replace_region(1, 0, 0, &patch), "a level with no pixels");
        assert!(!t.replace_region(0, 0, 0, &Image { w: 2, h: 2, px: vec![0; 3] }), "a malformed patch");
        assert_eq!(t, before);
    }

    #[test]
    fn sampling_clamps_to_the_edges() {
        let img = picture();
        let t = TextureLoader::default().from_image(img.clone()).unwrap();
        assert_eq!(t.sample_nearest(0.0, 0.0, 0), Some(img.px[0]));
        assert_eq!(t.sample_nearest(1.0, 1.0, 0), Some(img.px[7]));
        assert_eq!(t.sample_nearest(9.0, -3.0, 0), Some(img.px[3]));
        assert_eq!(t.sample_nearest(f32::NAN, f32::NAN, 0), Some(img.px[0]));
        assert_eq!(t.sample_nearest(0.5, 0.5, 4), None);
    }

    #[test]
    fn a_label_is_kept() {
        let mut t = TextureLoader::default().from_image(picture()).unwrap();
        assert_eq!(t.label(), "");
        t.set_label("albedo");
        assert_eq!(t.label(), "albedo");
    }

    #[test]
    fn a_batch_loads_in_the_background_and_reports_each_result() {
        let dir = tempfile::tempdir().unwrap();
        let img = picture();
        let on_disk = dir.path().join("ok.png");
        std::fs::write(&on_disk, png(&img)).unwrap();
        let sources = vec![
            TextureSource::Bytes(png(&img)),
            TextureSource::Bytes(b"hello world--".to_vec()),
            TextureSource::File(dir.path().join("missing.png")),
            TextureSource::File(on_disk),
        ];
        let mut pending = TextureLoader::default().load_async(sources);
        assert_eq!(pending.total(), 4);
        let mut results = pending.wait();
        assert!(pending.is_done());
        assert!(pending.poll().is_empty());
        results.sort_by_key(|(i, _)| *i);
        let kinds: Vec<Option<LoadError>> = results.iter().map(|(_, r)| r.as_ref().err().copied()).collect();
        assert_eq!(kinds, vec![None, Some(LoadError::Source(SourceError::NotImage)), Some(LoadError::Source(SourceError::Unreadable)), None]);
        assert_eq!(results[3].1.as_ref().unwrap().level(0).unwrap(), &img);
    }

    #[test]
    fn an_empty_batch_is_done_at_once() {
        let mut pending = TextureLoader::default().load_async(Vec::new());
        assert!(pending.is_done());
        assert!(pending.wait().is_empty());
    }
}
