//! Image input and output: format detection, incremental image sources with properties and thumbnails, and
//! destinations that write PNG, JPEG, GIF, BMP, TIFF and ICO with metadata.
//!
//! Decoders live in one module per format (`png`, `jpeg`, `gif`, `bmp` (also ICO), `tiff`, `webp`); each offers a
//! cheap `probe` that reads only headers and a `decode` that returns straight RGBA frames. `ImageSource` wraps
//! them behind one API and `ImageDestination` does the same for the writers.

pub mod bmp;
pub mod encode;
pub mod exif;
pub mod gif;
pub mod jpeg;
pub mod png;
pub mod tiff;
pub mod webp;

use super::image::{CgImage, DataConsumer, DataProvider, ImageColorSpace};
use crate::gfx::Image;
pub use exif::{ColorModel, Dict, PropValue, Properties};
use std::cell::OnceCell;

/// Pictures wider or taller than this are refused rather than decoded.
pub const MAX_SIDE: usize = 8192;

/// One decoded image: straight (non-premultiplied) pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    /// RGBA, 8 bits per component.
    pub rgba8: Vec<u8>,
    /// The same pixels at 16 bits per component when the file has them.
    pub rgba16: Option<Vec<u16>>,
    pub delay_ms: u32,
}

/// All the images of a file with their properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub frames: Vec<Frame>,
    pub props: Vec<Properties>,
    pub loop_count: Option<u32>,
}

/// The container formats this module reads and (except WebP) writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Ico,
    Tiff,
    Webp,
}

impl Format {
    /// Recognises a format from the first bytes.
    pub fn detect(d: &[u8]) -> Option<Format> {
        if d.starts_with(&png::SIGNATURE) {
            Some(Format::Png)
        } else if d.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Format::Jpeg)
        } else if d.starts_with(b"GIF8") {
            Some(Format::Gif)
        } else if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
            Some(Format::Tiff)
        } else if d.len() >= 12 && &d[..4] == b"RIFF" && &d[8..12] == b"WEBP" {
            Some(Format::Webp)
        } else if d.len() >= 6 && d[0] == 0 && d[1] == 0 && matches!(d[2], 1 | 2) && d[3] == 0 {
            Some(Format::Ico)
        } else if d.len() >= 18 && d.starts_with(b"BM") && matches!(u32::from_le_bytes([d[14], d[15], d[16], d[17]]), 12 | 40 | 52 | 56 | 64 | 108 | 124) {
            Some(Format::Bmp)
        } else {
            None
        }
    }

    /// The uniform type identifier of the format.
    pub fn type_identifier(self) -> &'static str {
        match self {
            Format::Png => "public.png",
            Format::Jpeg => "public.jpeg",
            Format::Gif => "com.compuserve.gif",
            Format::Bmp => "com.microsoft.bmp",
            Format::Ico => "com.microsoft.ico",
            Format::Tiff => "public.tiff",
            Format::Webp => "org.webmproject.webp",
        }
    }

    pub fn from_type_identifier(id: &str) -> Option<Format> {
        Self::ALL.iter().copied().find(|f| f.type_identifier() == id)
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Gif => "image/gif",
            Format::Bmp => "image/bmp",
            Format::Ico => "image/vnd.microsoft.icon",
            Format::Tiff => "image/tiff",
            Format::Webp => "image/webp",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Png => &["png"],
            Format::Jpeg => &["jpg", "jpeg", "jpe"],
            Format::Gif => &["gif"],
            Format::Bmp => &["bmp"],
            Format::Ico => &["ico", "cur"],
            Format::Tiff => &["tif", "tiff"],
            Format::Webp => &["webp"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<Format> {
        let e = ext.to_ascii_lowercase();
        Self::ALL.iter().copied().find(|f| f.extensions().contains(&e.as_str()))
    }

    pub const ALL: [Format; 7] = [Format::Png, Format::Jpeg, Format::Gif, Format::Bmp, Format::Ico, Format::Tiff, Format::Webp];

    /// True when `ImageDestination` can write the format.
    pub fn can_write(self) -> bool {
        self != Format::Webp
    }
}

/// Identifiers of every format that can be read.
pub fn source_type_identifiers() -> Vec<&'static str> {
    Format::ALL.iter().map(|f| f.type_identifier()).collect()
}

/// Identifiers of every format that can be written.
pub fn destination_type_identifiers() -> Vec<&'static str> {
    Format::ALL.iter().filter(|f| f.can_write()).map(|f| f.type_identifier()).collect()
}

/// True when `bytes` start like a format this module reads.
pub fn is_image(bytes: &[u8]) -> bool {
    Format::detect(bytes).is_some()
}

/// True for file names whose extension is a readable image format.
pub fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| Format::from_extension(e).is_some())
}

/// The pixel size of the first image, from the header alone.
pub fn pixel_size(bytes: &[u8]) -> Option<(usize, usize)> {
    probe_first(Format::detect(bytes)?, bytes).map(|p| (p.pixel_width, p.pixel_height))
}

/// Reads the first image of `bytes` as premultiplied pixels (animations: the first frame).
pub fn decode(bytes: &[u8]) -> Option<Image> {
    ImageSource::new(bytes.to_vec()).image_premultiplied(0)
}

/// How far a source got (what a progressive loader asks while data arrives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Everything needed was decoded.
    Complete,
    /// The header is known; more data is expected.
    Incomplete,
    UnknownType,
    /// Not enough data yet to tell the type or the size.
    ReadingHeader,
    InvalidData,
    /// The data ended in the middle of the image and no more is coming.
    UnexpectedEof,
}

fn probe_first(format: Format, d: &[u8]) -> Option<Properties> {
    match format {
        Format::Png => png::probe(d),
        Format::Jpeg => jpeg::probe(d),
        Format::Gif => gif::probe(d),
        Format::Bmp => bmp::probe(d),
        Format::Ico => bmp::ico_probe(d),
        Format::Tiff => tiff::probe(d),
        Format::Webp => webp::probe(d),
    }
}

fn decode_all(format: Format, d: &[u8]) -> Option<Decoded> {
    match format {
        Format::Png => png::decode(d),
        Format::Jpeg => jpeg::decode(d),
        Format::Gif => gif::decode(d),
        Format::Bmp => bmp::decode(d),
        Format::Ico => bmp::ico_decode(d),
        Format::Tiff => tiff::decode(d),
        Format::Webp => webp::decode(d),
    }
}

/// Options for `ImageSource::thumbnail_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThumbnailOptions {
    /// The longest side of the result; None keeps the full size.
    pub max_pixel_size: Option<usize>,
    pub create_from_image_always: bool,
    pub create_from_image_if_absent: bool,
    /// Rotate and flip the result by the image orientation.
    pub transform: bool,
    /// Decode at 1/2, 1/4 or 1/8 size first (a speed hint).
    pub subsample_factor: Option<usize>,
}

impl Default for ThumbnailOptions {
    fn default() -> Self {
        ThumbnailOptions { max_pixel_size: None, create_from_image_always: true, create_from_image_if_absent: true, transform: true, subsample_factor: None }
    }
}

/// A reader for one picture file, possibly still arriving.
pub struct ImageSource {
    data: Vec<u8>,
    is_final: bool,
    decoded: OnceCell<Option<Decoded>>,
}

impl ImageSource {
    /// A source over complete data.
    pub fn new(data: Vec<u8>) -> Self {
        ImageSource { data, is_final: true, decoded: OnceCell::new() }
    }

    /// A source that expects data to arrive through `append` / `update_data`.
    pub fn incremental() -> Self {
        ImageSource { data: Vec::new(), is_final: false, decoded: OnceCell::new() }
    }

    pub fn from_provider(p: &DataProvider) -> Self {
        Self::new(p.data().to_vec())
    }

    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self::new(std::fs::read(path)?))
    }

    /// Replaces the accumulated data with `all` (everything received so far).
    pub fn update_data(&mut self, all: Vec<u8>, is_final: bool) {
        self.data = all;
        self.is_final = is_final;
        self.decoded = OnceCell::new();
    }

    /// Appends newly received bytes.
    pub fn append(&mut self, more: &[u8], is_final: bool) {
        self.data.extend_from_slice(more);
        self.is_final = is_final;
        self.decoded = OnceCell::new();
    }

    pub fn format(&self) -> Option<Format> {
        Format::detect(&self.data)
    }

    pub fn type_identifier(&self) -> Option<&'static str> {
        self.format().map(Format::type_identifier)
    }

    fn all(&self) -> Option<&Decoded> {
        self.decoded
            .get_or_init(|| {
                decode_all(self.format()?, &self.data).map(|mut d| {
                    // Pictures with an embedded profile are brought to the display profile here, once per decode.
                    crate::icc_transform::to_display(&mut d);
                    d
                })
            })
            .as_ref()
    }

    /// The number of images in the file (0 until the header is readable).
    pub fn count(&self) -> usize {
        let Some(f) = self.format() else { return 0 };
        if probe_first(f, &self.data).is_none() {
            return 0;
        }
        match f {
            Format::Gif => gif::count(&self.data).unwrap_or(1),
            Format::Ico => bmp::ico_count(&self.data).unwrap_or(1),
            Format::Tiff => tiff::count(&self.data).unwrap_or(1),
            _ => 1,
        }
    }

    pub fn status(&self) -> Status {
        let Some(f) = self.format() else {
            return if !self.is_final && self.data.len() < 12 { Status::ReadingHeader } else { Status::UnknownType };
        };
        if probe_first(f, &self.data).is_none() {
            return if self.is_final { Status::InvalidData } else { Status::ReadingHeader };
        }
        if self.all().is_some() {
            return Status::Complete;
        }
        if !self.is_final {
            return Status::Incomplete;
        }
        let tail = &self.data[self.data.len().saturating_sub(12)..];
        let ended = match f {
            Format::Png => tail.windows(4).any(|w| w == b"IEND") || self.data.windows(4).rev().take(64).any(|w| w == b"IEND"),
            Format::Jpeg => self.data.ends_with(&[0xFF, 0xD9]),
            Format::Gif => self.data.last() == Some(&0x3B),
            _ => false,
        };
        if ended {
            Status::InvalidData
        } else {
            Status::UnexpectedEof
        }
    }

    /// The status of one image (images are decoded together, so this matches `status` for valid indices).
    pub fn status_at(&self, index: usize) -> Status {
        match self.status() {
            Status::Complete if index >= self.all().map_or(0, |d| d.frames.len()) => Status::InvalidData,
            s => s,
        }
    }

    /// The index of the image to show by default (the largest one in an icon file, otherwise the first).
    pub fn primary_image_index(&self) -> usize {
        if self.format() == Some(Format::Ico) {
            if let Some(d) = self.all() {
                return d.frames.iter().enumerate().max_by_key(|(_, f)| f.w * f.h).map_or(0, |(i, _)| i);
            }
        }
        0
    }

    /// File-level properties: size in bytes, image count, loop count.
    pub fn properties(&self) -> Dict {
        let mut d = Dict::new();
        d.insert("FileSize".into(), PropValue::Int(self.data.len() as i64));
        d.insert("ImageCount".into(), PropValue::Int(self.count() as i64));
        if let Some(t) = self.type_identifier() {
            d.insert("UTType".into(), PropValue::Text(t.into()));
        }
        if let Some(l) = self.all().and_then(|a| a.loop_count) {
            d.insert("LoopCount".into(), PropValue::Int(l as i64));
        }
        d
    }

    /// Properties of image `index`; the first image costs no pixel decode.
    pub fn properties_at(&self, index: usize) -> Option<Properties> {
        let f = self.format()?;
        if index == 0 {
            return probe_first(f, &self.data);
        }
        self.all()?.props.get(index).cloned()
    }

    /// Pixel size of the first image, from the header.
    pub fn pixel_size(&self) -> Option<(usize, usize)> {
        self.properties_at(0).map(|p| (p.pixel_width, p.pixel_height))
    }

    /// Display time of an animation frame in milliseconds.
    pub fn delay_ms_at(&self, index: usize) -> Option<u32> {
        self.all()?.frames.get(index).map(|f| f.delay_ms)
    }

    pub fn loop_count(&self) -> Option<u32> {
        self.all()?.loop_count
    }

    /// Image `index` as a bitmap object (straight alpha, 8 or 16 bits per component).
    pub fn image_at(&self, index: usize) -> Option<CgImage> {
        let d = self.all()?;
        let f = d.frames.get(index)?;
        let p = d.props.get(index);
        let mut img = match &f.rgba16 {
            Some(w) => CgImage::from_rgba16(f.w, f.h, w)?,
            None => CgImage::from_rgba8(f.w, f.h, f.rgba8.clone())?,
        };
        if let Some(profile) = p.and_then(|p| p.icc_profile.as_ref()) {
            // Pixels already converted to the display profile carry no source profile.
            if profile.get(16..20) == Some(b"RGB ") && !crate::icc_transform::converts(profile) {
                img = img.copy_with_color_space(ImageColorSpace::Icc { components: 3, profile: std::sync::Arc::new(profile.clone()) })?;
            }
        }
        Some(img.with_type_identifier(self.type_identifier()?))
    }

    /// Image `index` as premultiplied renderer pixels.
    pub fn image_premultiplied(&self, index: usize) -> Option<Image> {
        let f = self.all()?.frames.get(index)?;
        let mut img = Image::new(f.w, f.h);
        for (o, p) in img.px.iter_mut().zip(f.rgba8.as_chunks::<4>().0.iter()) {
            *o = crate::gfx::rgba(p[0], p[1], p[2], p[3]);
        }
        Some(img)
    }

    /// A reduced copy of image `index`. `None` when the options allow no thumbnail or the image does not decode.
    pub fn thumbnail_at(&self, index: usize, o: &ThumbnailOptions) -> Option<CgImage> {
        if !o.create_from_image_always && !o.create_from_image_if_absent {
            return None;
        }
        let mut img = self.image_premultiplied(index)?;
        if let Some(f) = o.subsample_factor.filter(|f| matches!(f, 2 | 4 | 8)) {
            img = crate::imgdec::box_down(&img, f);
        }
        if let Some(m) = o.max_pixel_size.filter(|&m| m > 0) {
            img = crate::imgdec::contain(&img, m, m);
        }
        if o.transform {
            let orient = self.properties_at(index).map_or(1, |p| p.orientation);
            img = orient_image(&img, orient);
        }
        CgImage::from_premultiplied(&img)
    }

    /// The size a thumbnail of the first image would have, from the header alone (before any rotation).
    pub fn thumbnail_size(&self, max: usize) -> Option<(usize, usize)> {
        let (w, h) = self.pixel_size()?;
        Some(crate::imgdec::fit_dims(w, h, max, max))
    }
}

/// Applies an orientation value (1..=8) so that the picture is upright.
pub fn orient_image(img: &Image, orientation: u8) -> Image {
    let (sw, sh) = (img.w, img.h);
    let (dw, dh) = if (5..=8).contains(&orientation) { (sh, sw) } else { (sw, sh) };
    if orientation <= 1 || orientation > 8 {
        return img.clone();
    }
    let mut out = Image::new(dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            let (sx, sy) = match orientation {
                2 => (sw - 1 - x, y),
                3 => (sw - 1 - x, sh - 1 - y),
                4 => (x, sh - 1 - y),
                5 => (y, x),
                6 => (y, sh - 1 - x),
                7 => (sw - 1 - y, sh - 1 - x),
                _ => (sw - 1 - y, x),
            };
            out.px[y * dw + x] = img.px[sy * sw + sx];
        }
    }
    out
}

/// Per-image settings for a destination.
#[derive(Clone, Debug, Default)]
pub struct WriteOptions {
    /// Lossy quality 0.0..=1.0 (JPEG); default 0.8.
    pub quality: Option<f32>,
    pub dpi: Option<(f64, f64)>,
    pub orientation: Option<u8>,
    pub exif: Dict,
    pub tiff: Dict,
    pub gps: Dict,
    /// PNG text chunks.
    pub text: Vec<(String, String)>,
    pub delay_ms: Option<u32>,
    pub comment: Option<String>,
    pub icc: Option<Vec<u8>>,
    pub gamma: Option<f64>,
}

/// Collects images and metadata, then writes one file.
pub struct ImageDestination {
    format: Format,
    expected: usize,
    images: Vec<(CgImage, WriteOptions)>,
    defaults: WriteOptions,
    pub loop_count: u32,
    pub tiff_compression: tiff::Compression,
    pub subsample: bool,
}

impl ImageDestination {
    /// `None` when the format cannot be written. `count` is the number of images that will be added.
    pub fn new(format: Format, count: usize) -> Option<Self> {
        (format.can_write() && count > 0).then(|| ImageDestination {
            format,
            expected: count,
            images: Vec::new(),
            defaults: WriteOptions::default(),
            loop_count: 0,
            tiff_compression: tiff::Compression::Deflate,
            subsample: true,
        })
    }

    pub fn with_type_identifier(id: &str, count: usize) -> Option<Self> {
        Self::new(Format::from_type_identifier(id)?, count)
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// Settings that apply to every image unless overridden by that image's own options.
    pub fn set_properties(&mut self, o: WriteOptions) {
        self.defaults = o;
    }

    /// Adds an image; ignored beyond the declared count.
    pub fn add_image(&mut self, img: &CgImage, o: Option<WriteOptions>) {
        if self.images.len() < self.expected {
            self.images.push((img.clone(), o.unwrap_or_default()));
        }
    }

    pub fn add_image_from_source(&mut self, src: &ImageSource, index: usize, o: Option<WriteOptions>) -> bool {
        match src.image_at(index) {
            Some(img) => {
                let mut o = o.unwrap_or_default();
                if let Some(p) = src.properties_at(index) {
                    if o.orientation.is_none() && p.orientation > 1 {
                        o.orientation = Some(p.orientation);
                    }
                    if o.dpi.is_none() {
                        o.dpi = p.dpi_width.zip(p.dpi_height);
                    }
                }
                self.add_image(&img, Some(o));
                true
            }
            None => false,
        }
    }

    fn merged(&self, o: &WriteOptions) -> WriteOptions {
        let d = &self.defaults;
        let mut m = o.clone();
        m.quality = o.quality.or(d.quality);
        m.dpi = o.dpi.or(d.dpi);
        m.orientation = o.orientation.or(d.orientation);
        m.delay_ms = o.delay_ms.or(d.delay_ms);
        m.comment = o.comment.clone().or_else(|| d.comment.clone());
        m.icc = o.icc.clone().or_else(|| d.icc.clone());
        m.gamma = o.gamma.or(d.gamma);
        for (dst, src) in [(&mut m.exif, &d.exif), (&mut m.tiff, &d.tiff), (&mut m.gps, &d.gps)] {
            for (k, v) in src {
                dst.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if m.text.is_empty() {
            m.text = d.text.clone();
        }
        m
    }

    /// The TIFF-structured metadata block for one image's options.
    fn exif_block(o: &WriteOptions) -> Option<Vec<u8>> {
        let mut tiff = o.tiff.clone();
        if let Some(or) = o.orientation.filter(|&v| (2..=8).contains(&v)) {
            tiff.insert("Orientation".into(), PropValue::Int(or as i64));
        }
        if let Some((x, y)) = o.dpi {
            tiff.insert("XResolution".into(), PropValue::Float(x));
            tiff.insert("YResolution".into(), PropValue::Float(y));
            tiff.insert("ResolutionUnit".into(), PropValue::Int(2));
        }
        exif::build_block(&tiff, &o.exif, &o.gps)
    }

    /// Writes the file. `None` when no image was added or an image cannot be encoded.
    pub fn finalize(self) -> Option<Vec<u8>> {
        if self.images.is_empty() {
            return None;
        }
        let items: Vec<(Vec<u8>, usize, usize, WriteOptions)> = self.images.iter().map(|(i, o)| (i.to_rgba8(), i.width(), i.height(), self.merged(o))).collect();
        let (first_px, w, h, fo) = &items[0];
        match self.format {
            Format::Png => {
                let o = encode::PngOptions {
                    text: fo.text.clone(),
                    dpi: fo.dpi,
                    exif: Self::exif_block(&WriteOptions { dpi: None, ..fo.clone() }),
                    icc: fo.icc.clone().map(|p| ("ICC".to_string(), p)),
                    gamma: fo.gamma,
                    level: None,
                };
                encode::encode_png(*w, *h, first_px, &o)
            }
            Format::Jpeg => {
                let o = encode::JpegOptions {
                    quality: (fo.quality.unwrap_or(0.8).clamp(0.0, 1.0) * 100.0).round().max(1.0) as u8,
                    subsample: self.subsample,
                    dpi: fo.dpi,
                    exif: Self::exif_block(fo),
                    icc: fo.icc.clone(),
                    comment: fo.comment.clone(),
                };
                encode::encode_jpeg(*w, *h, first_px, &o)
            }
            Format::Gif => {
                let frames: Vec<gif::GifFrame> = items.iter().filter(|i| i.1 == *w && i.2 == *h).map(|i| gif::GifFrame { rgba: &i.0, delay_ms: i.3.delay_ms.unwrap_or(100) }).collect();
                gif::encode(*w, *h, &frames, Some(self.loop_count))
            }
            Format::Bmp => Some(bmp::encode(*w, *h, first_px, fo.dpi)),
            Format::Tiff => {
                let tiffs: Vec<Dict> = items
                    .iter()
                    .map(|i| {
                        let mut t = i.3.tiff.clone();
                        if let Some(or) = i.3.orientation.filter(|&v| (1..=8).contains(&v)) {
                            t.insert("Orientation".into(), PropValue::Int(or as i64));
                        }
                        t
                    })
                    .collect();
                let pages: Vec<tiff::Page> = items
                    .iter()
                    .zip(&tiffs)
                    .map(|(i, t)| tiff::Page { w: i.1, h: i.2, rgba: &i.0, dpi: i.3.dpi, tiff: t, exif: &i.3.exif, gps: &i.3.gps, icc: i.3.icc.as_deref() })
                    .collect();
                tiff::encode(&pages, self.tiff_compression)
            }
            Format::Ico => {
                let mut v = Vec::new();
                for (px, w, h, _) in &items {
                    if *w > 256 || *h > 256 {
                        continue;
                    }
                    v.push((*w, *h, encode::encode_png(*w, *h, px, &encode::PngOptions::default())?));
                }
                (!v.is_empty()).then(|| bmp::ico_encode(&v))
            }
            Format::Webp => None,
        }
    }

    /// Writes the file into `consumer` (and its file, when it has one).
    pub fn finalize_to(self, mut consumer: DataConsumer) -> std::io::Result<bool> {
        match self.finalize() {
            Some(b) => {
                consumer.put_bytes(&b);
                consumer.finish()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[cfg(any(test, feature = "testkit"))]
pub mod tests {
    use super::*;

    /// A solid `w` x `h` RGBA PNG for tests in other modules.
    pub fn png_bytes(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let px: Vec<u8> = (0..w * h).flat_map(|_| color).collect();
        encode::encode_png(w as usize, h as usize, &px, &encode::PngOptions::default()).unwrap()
    }

    #[cfg(test)]
    fn sample_image(w: usize, h: usize) -> CgImage {
        let px: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 40) as u8, (y * 60) as u8, 90, 255])).collect();
        CgImage::from_rgba8(w, h, px).unwrap()
    }

    #[test]
    fn detection_and_identifiers() {
        assert_eq!(Format::detect(&png_bytes(1, 1, [0; 4])), Some(Format::Png));
        assert_eq!(Format::detect(b"GIF89a.."), Some(Format::Gif));
        assert_eq!(Format::detect(b"RIFF\0\0\0\0WEBPVP8L"), Some(Format::Webp));
        assert_eq!(Format::detect(b"hello world!"), None);
        assert_eq!(Format::from_type_identifier("public.tiff"), Some(Format::Tiff));
        assert_eq!(Format::from_extension("JPG"), Some(Format::Jpeg));
        assert!(is_image_name("a.WebP") && is_image_name("b.ico") && !is_image_name("c.txt") && !is_image_name("png"));
        assert!(destination_type_identifiers().len() == 6 && source_type_identifiers().len() == 7);
    }

    #[test]
    fn png_source_reports_header_before_decoding() {
        let s = ImageSource::new(png_bytes(5, 3, [1, 2, 3, 255]));
        assert_eq!((s.count(), s.pixel_size()), (1, Some((5, 3))));
        assert_eq!(s.status(), Status::Complete);
        assert_eq!(s.type_identifier(), Some("public.png"));
        let img = s.image_at(0).unwrap();
        assert_eq!((img.width(), img.height()), (5, 3));
        assert_eq!(img.type_identifier(), Some("public.png"));
        assert_eq!(decode(&png_bytes(2, 1, [200, 100, 0, 128])).unwrap().px[0], crate::gfx::rgba(200, 100, 0, 128));
    }

    #[test]
    fn incremental_source_progresses_to_complete() {
        let bytes = png_bytes(4, 4, [9, 9, 9, 255]);
        let mut s = ImageSource::incremental();
        assert_eq!(s.status(), Status::ReadingHeader);
        s.append(&bytes[..4], false);
        assert_eq!(s.status(), Status::ReadingHeader);
        s.append(&bytes[4..30], false);
        assert_eq!(s.status(), Status::Incomplete);
        assert_eq!(s.pixel_size(), Some((4, 4)));
        s.append(&bytes[30..], true);
        assert_eq!(s.status(), Status::Complete);
        let mut cut = ImageSource::incremental();
        cut.update_data(bytes[..bytes.len() - 20].to_vec(), true);
        assert_eq!(cut.status(), Status::UnexpectedEof);
        assert_eq!(ImageSource::new(b"not an image at all".to_vec()).status(), Status::UnknownType);
    }

    #[test]
    fn thumbnails_scale_and_orient() {
        let mut o = WriteOptions::default();
        o.orientation = Some(6);
        let mut d = ImageDestination::new(Format::Tiff, 1).unwrap();
        d.add_image(&sample_image(8, 4), Some(o));
        let tiff = d.finalize().unwrap();
        let s = ImageSource::new(tiff);
        assert_eq!(s.properties_at(0).unwrap().orientation, 6);
        let t = s.thumbnail_at(0, &ThumbnailOptions { max_pixel_size: Some(4), ..Default::default() }).unwrap();
        assert_eq!((t.width(), t.height()), (2, 4), "4 x 2 scaled, then rotated upright");
        let plain = s.thumbnail_at(0, &ThumbnailOptions { transform: false, max_pixel_size: Some(4), ..Default::default() }).unwrap();
        assert_eq!((plain.width(), plain.height()), (4, 2));
        assert!(s.thumbnail_at(0, &ThumbnailOptions { create_from_image_always: false, create_from_image_if_absent: false, ..Default::default() }).is_none());
    }

    #[test]
    fn orientation_moves_the_corner_pixel() {
        let mut img = Image::new(3, 2);
        img.px[0] = 7; // top-left
        assert_eq!(orient_image(&img, 6).px[1], 7, "rotated clockwise: top-left becomes top-right");
        assert_eq!(orient_image(&img, 3).px[5], 7);
        assert_eq!(orient_image(&img, 2).px[2], 7);
        assert_eq!(orient_image(&img, 8).px[4], 7, "rotated counter-clockwise: top-left becomes bottom-left");
        assert_eq!(orient_image(&img, 1), img);
    }

    #[test]
    fn destination_round_trips_every_writable_format() {
        let img = sample_image(6, 5);
        for f in [Format::Png, Format::Bmp, Format::Tiff, Format::Gif, Format::Ico] {
            let mut d = ImageDestination::new(f, 1).unwrap();
            d.add_image(&img, None);
            let bytes = d.finalize().unwrap();
            let s = ImageSource::new(bytes);
            assert_eq!(s.format(), Some(f), "{f:?}");
            assert_eq!(s.pixel_size(), Some((6, 5)), "{f:?}");
            let back = s.image_at(0).unwrap();
            if f != Format::Gif {
                assert_eq!(back.to_rgba8(), img.to_rgba8(), "{f:?}");
            }
        }
        let mut d = ImageDestination::new(Format::Jpeg, 1).unwrap();
        d.add_image(&img, Some(WriteOptions { quality: Some(0.9), ..Default::default() }));
        let s = ImageSource::new(d.finalize().unwrap());
        assert_eq!(s.status(), Status::Complete);
        assert!(ImageDestination::new(Format::Webp, 1).is_none());
        assert!(ImageDestination::new(Format::Png, 1).unwrap().finalize().is_none());
    }

    #[test]
    fn metadata_survives_a_jpeg_and_a_png_round_trip() {
        let mut exif = Dict::new();
        exif.insert("DateTimeOriginal".into(), PropValue::Text("2026:10:13 12:00:00".into()));
        let mut gps = Dict::new();
        gps.insert("LatitudeRef".into(), PropValue::Text("N".into()));
        let o = WriteOptions { dpi: Some((150.0, 150.0)), orientation: Some(3), exif, gps, text: vec![("Author".into(), "me".into())], ..Default::default() };
        for f in [Format::Jpeg, Format::Png] {
            let mut d = ImageDestination::new(f, 1).unwrap();
            d.add_image(&sample_image(4, 4), Some(o.clone()));
            let s = ImageSource::new(d.finalize().unwrap());
            let p = s.properties_at(0).unwrap();
            assert_eq!(p.orientation, 3, "{f:?}");
            assert_eq!(p.exif["DateTimeOriginal"].as_str(), Some("2026:10:13 12:00:00"), "{f:?}");
            assert_eq!(p.gps["LatitudeRef"].as_str(), Some("N"), "{f:?}");
            assert!((p.dpi_width.unwrap() - 150.0).abs() < 0.5, "{f:?}");
        }
    }

    #[test]
    fn animated_gif_through_the_source_api() {
        let a = sample_image(3, 3);
        let b = sample_image(3, 3).cropped(0, 0, 3, 3).unwrap();
        let mut d = ImageDestination::new(Format::Gif, 2).unwrap();
        d.loop_count = 2;
        d.add_image(&a, Some(WriteOptions { delay_ms: Some(200), ..Default::default() }));
        d.add_image(&b, Some(WriteOptions { delay_ms: Some(300), ..Default::default() }));
        let s = ImageSource::new(d.finalize().unwrap());
        assert_eq!(s.count(), 2);
        assert_eq!(s.delay_ms_at(0), Some(200));
        assert_eq!(s.delay_ms_at(1), Some(300));
        assert_eq!(s.loop_count(), Some(2));
        assert_eq!(s.status_at(5), Status::InvalidData);
    }

    #[test]
    fn ico_primary_index_is_the_largest() {
        let big = encode::encode_png(32, 32, &vec![255; 32 * 32 * 4], &Default::default()).unwrap();
        let small = encode::encode_png(16, 16, &vec![255; 16 * 16 * 4], &Default::default()).unwrap();
        let ico = bmp::ico_encode(&[(16, 16, small), (32, 32, big)]);
        let s = ImageSource::new(ico);
        assert_eq!(s.count(), 2);
        assert_eq!(s.primary_image_index(), 1);
    }
}
