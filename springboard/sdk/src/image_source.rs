//! Image sources for apps: read a picture's format and pixel size from its header, decode it,
//! or make a thumbnail that fits a box. The header is read before any pixel is decoded, so an
//! oversized picture is refused without allocating its pixels, and a layout can ask for the
//! thumbnail size before the decode. Every format of `cg::imageio` is read; other data is `NotImage`.
//!
//! The stored size is the size of the pixels in the file. A picture whose EXIF orientation turns it a quarter turn is
//! shown with its sides swapped, so `upright_size`, `thumbnail_size`, `decode_upright` and `thumbnail` follow the
//! orientation, while `size` and `decode` keep the stored pixels.
use bifrost_render::gfx::Image;
use bifrost_render::cg::imageio::{self, orient_image, ImageSource as CodecSource, MAX_SIDE};
use bifrost_render::imgdec;
use bifrost_render::pixel_layout::unpremultiply;
use std::path::Path;
pub use bifrost_render::cg::imageio::Format;

/// Why a picture could not be read, decoded or made into a thumbnail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The bytes do not start like a supported picture format.
    NotImage,
    /// The data ends before the size fields of the header.
    Truncated,
    /// The picture is wider or taller than the decode limit (`MAX_SIDE`), or has more pixels than the caller allows.
    TooLarge { width: usize, height: usize },
    /// The header is present but the pixel data does not decode, or the size is zero.
    Undecodable,
    /// A thumbnail box with a zero side.
    EmptyBox,
    /// The file is missing, is not a regular file, or cannot be read.
    Unreadable,
    /// The file is larger than the byte limit given to `read_file`. Its size is reported without reading it.
    FileTooLarge { bytes: u64 },
}

/// A picture held as bytes by the caller. Its format, size and orientation are known before any decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSource<'a> {
    bytes: &'a [u8],
    format: Format,
    width: usize,
    height: usize,
    orientation: u8,
}

impl<'a> ImageSource<'a> {
    /// Reads the format and size from the header. Pictures wider or taller than `MAX_SIDE` are refused here, before
    /// decoding.
    pub fn new(bytes: &'a [u8]) -> Result<Self, SourceError> {
        Self::with_max_pixels(bytes, usize::MAX)
    }

    /// Like [`ImageSource::new`], and also refuses a picture with more than `max_pixels` pixels. An app that makes
    /// thumbnails on a small budget passes its own limit; the decoded picture takes four bytes per pixel.
    pub fn with_max_pixels(bytes: &'a [u8], max_pixels: usize) -> Result<Self, SourceError> {
        let format = Format::detect(bytes).ok_or(SourceError::NotImage)?;
        let (width, height) = match imageio::pixel_size(bytes) {
            Some(s) if s.0 > 0 && s.1 > 0 => s,
            _ if bytes.len() < 64 => return Err(SourceError::Truncated),
            _ => return Err(SourceError::Undecodable),
        };
        if width > MAX_SIDE || height > MAX_SIDE || width.saturating_mul(height) > max_pixels {
            return Err(SourceError::TooLarge { width, height });
        }
        let orientation = CodecSource::new(bytes.to_vec()).properties_at(0).map_or(1, |p| p.orientation);
        Ok(ImageSource { bytes, format, width, height, orientation })
    }

    /// The container format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The pixel size of the stored pixels, from the header.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The pixel size the picture is shown at: the stored size, with the sides swapped for a quarter turn.
    pub fn upright_size(&self) -> (usize, usize) {
        if (5..=8).contains(&self.orientation) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }

    /// The size of a thumbnail that fits `max_w` x `max_h`, computed from the header alone.
    /// The picture is never enlarged.
    pub fn thumbnail_size(&self, max_w: usize, max_h: usize) -> Result<(usize, usize), SourceError> {
        if max_w == 0 || max_h == 0 {
            return Err(SourceError::EmptyBox);
        }
        let (w, h) = self.upright_size();
        Ok(imgdec::fit_dims(w, h, max_w, max_h))
    }

    /// Decodes the whole picture into premultiplied RGBA, as stored.
    pub fn decode(&self) -> Result<Image, SourceError> {
        imageio::decode(self.bytes).ok_or(SourceError::Undecodable)
    }

    /// Decodes the whole picture and turns it upright by its EXIF orientation.
    pub fn decode_upright(&self) -> Result<Image, SourceError> {
        Ok(orient_image(&self.decode()?, self.orientation))
    }

    /// Decodes the picture upright and scaled to fit `max_w` x `max_h`, keeping the aspect. Never enlarged.
    pub fn thumbnail(&self, max_w: usize, max_h: usize) -> Result<Image, SourceError> {
        self.thumbnail_size(max_w, max_h)?;
        Ok(imgdec::contain(&self.decode_upright()?, max_w, max_h))
    }

    /// The small preview a camera or writer embedded in the file (the JPEG of the EXIF IFD1), upright. Decoding it
    /// costs a fraction of the full picture. `None` when the file embeds none.
    pub fn embedded_thumbnail(&self) -> Option<Image> {
        let src = CodecSource::new(self.bytes.to_vec());
        Some(orient_image(&src.embedded_thumbnail_image(0)?, self.orientation))
    }

    /// Like [`ImageSource::thumbnail`], but uses the embedded preview when it is at least as large as the thumbnail
    /// would be, so a grid of camera photos fills without decoding each one in full. Falls back to the full decode.
    pub fn fast_thumbnail(&self, max_w: usize, max_h: usize) -> Result<Image, SourceError> {
        let (tw, th) = self.thumbnail_size(max_w, max_h)?;
        if let Some(preview) = self.embedded_thumbnail().filter(|p| p.w >= tw && p.h >= th) {
            return Ok(imgdec::contain(&preview, max_w, max_h));
        }
        self.thumbnail(max_w, max_h)
    }
}

/// Reads a picture file so that it can be given to [`ImageSource::new`]. A file larger than `max_bytes` is refused from
/// its size, before any of it is read. Directories and missing files are `Unreadable`.
pub fn read_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, SourceError> {
    let meta = std::fs::metadata(path).map_err(|_| SourceError::Unreadable)?;
    if !meta.is_file() {
        return Err(SourceError::Unreadable);
    }
    if meta.len() > max_bytes {
        return Err(SourceError::FileTooLarge { bytes: meta.len() });
    }
    std::fs::read(path).map_err(|_| SourceError::Unreadable)
}

/// A file type a bitmap can be written as (`representation(using:properties:)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Png,
    Jpeg,
}

/// Encoder properties. `compression_factor` is 0.0 (smallest file) to 1.0 (best quality) and applies to JPEG only;
/// it defaults to 0.8.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RepProperties {
    pub compression_factor: Option<f32>,
}

/// Encodes a decoded picture as `file_type`. Transparent pixels are un-premultiplied for PNG and composited over
/// white for JPEG. `None` when the picture has no pixels or the encoder refuses it.
pub fn representation(image: &Image, file_type: FileType, props: &RepProperties) -> Option<Vec<u8>> {
    use bifrost_render::cg::imageio::encode::{encode_jpeg, encode_png, JpegOptions, PngOptions};
    if image.w == 0 || image.h == 0 || image.px.len() < image.w * image.h {
        return None;
    }
    let rgba: Vec<u8> = image.px.iter().take(image.w * image.h).flat_map(|&p| unpremultiply(p)).collect();
    match file_type {
        FileType::Png => encode_png(image.w, image.h, &rgba, &PngOptions::default()),
        FileType::Jpeg => {
            let quality = (props.compression_factor.unwrap_or(0.8).clamp(0.0, 1.0) * 99.0).round() as u8 + 1;
            encode_jpeg(image.w, image.h, &rgba, &JpegOptions { quality, ..JpegOptions::default() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG signature and an IHDR chunk with a zero CRC; enough for the header reader.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    /// SOI, one APP0 segment of 4 bytes (length 4 counts itself), then a baseline frame header.
    fn jpeg_header(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0xAA, 0xBB, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        v
    }

    #[test]
    fn png_header_gives_format_and_size() {
        let bytes = png_header(300, 200);
        let src = ImageSource::new(&bytes).unwrap();
        assert_eq!(src.format(), Format::Png);
        assert_eq!(src.size(), (300, 200));
    }

    #[test]
    fn jpeg_size_skips_the_app_segment_before_the_frame() {
        // The APP0 segment is 2 + 4 bytes; the frame header starts at offset 8 and carries
        // height 0x00C8 = 200 and width 0x012C = 300.
        let bytes = jpeg_header(300, 200);
        let src = ImageSource::new(&bytes).unwrap();
        assert_eq!(src.format(), Format::Jpeg);
        assert_eq!(src.size(), (300, 200));
    }

    #[test]
    fn data_that_is_not_png_or_jpeg_is_refused() {
        assert_eq!(ImageSource::new(b"hello world--").unwrap_err(), SourceError::NotImage);
    }

    #[test]
    fn png_header_cut_before_the_size_is_truncated() {
        let full = png_header(1, 1);
        assert_eq!(ImageSource::new(&full[..20]).unwrap_err(), SourceError::Truncated);
    }

    #[test]
    fn oversized_picture_is_refused_before_decoding() {
        // 9000 is above MAX_SIDE (8192), so the header alone decides.
        let bytes = png_header(9000, 100);
        assert_eq!(ImageSource::new(&bytes).unwrap_err(), SourceError::TooLarge { width: 9000, height: 100 });
    }

    #[test]
    fn thumbnail_size_fits_the_box_without_enlarging() {
        let bytes = png_header(300, 200);
        let src = ImageSource::new(&bytes).unwrap();
        // 150 / 300 = 0.5 is the smaller factor, so the picture becomes 150 x 100.
        assert_eq!(src.thumbnail_size(150, 150), Ok((150, 100)));
        // A box larger than the picture keeps its size: the factor is capped at 1.
        assert_eq!(src.thumbnail_size(600, 600), Ok((300, 200)));
        assert_eq!(src.thumbnail_size(0, 10), Err(SourceError::EmptyBox));
    }

    #[test]
    fn header_that_does_not_decode_is_undecodable() {
        // A valid header with no image data: the decoder returns nothing.
        let bytes = png_header(1, 1);
        let src = ImageSource::new(&bytes).unwrap();
        assert_eq!(src.decode().err(), Some(SourceError::Undecodable));
    }

    fn checker() -> Image {
        // 4 x 4, opaque red and blue squares (R in the low byte, alpha in the high byte)
        let px = (0..16).map(|i| if (i % 4 + i / 4) % 2 == 0 { 0xFF00_00FF } else { 0xFFFF_0000 }).collect();
        Image { w: 4, h: 4, px }
    }

    #[test]
    fn png_representation_round_trips_the_pixels() {
        let img = checker();
        let bytes = representation(&img, FileType::Png, &RepProperties::default()).unwrap();
        let back = ImageSource::new(&bytes).unwrap();
        assert_eq!((back.format(), back.size()), (Format::Png, (4, 4)));
        assert_eq!(back.decode().unwrap().px, img.px);
    }

    #[test]
    fn jpeg_representation_decodes_and_quality_changes_the_size() {
        let img = checker();
        let low = representation(&img, FileType::Jpeg, &RepProperties { compression_factor: Some(0.0) }).unwrap();
        let high = representation(&img, FileType::Jpeg, &RepProperties { compression_factor: Some(1.0) }).unwrap();
        assert_eq!(ImageSource::new(&high).unwrap().size(), (4, 4));
        assert!(high.len() > low.len());
    }

    fn jpeg_with_preview(w: usize, h: usize) -> Vec<u8> {
        use bifrost_render::cg::image::CgImage;
        use bifrost_render::cg::imageio::{ImageDestination, WriteOptions};
        let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 90, 160, 255]).collect();
        let mut d = ImageDestination::new(Format::Jpeg, 1).unwrap();
        d.embed_thumbnail = true;
        d.add_image(&CgImage::from_rgba8(w, h, px).unwrap(), Some(WriteOptions { orientation: Some(6), ..Default::default() }));
        d.finalize().unwrap()
    }

    #[test]
    fn the_embedded_preview_is_upright_and_serves_small_thumbnails() {
        let bytes = jpeg_with_preview(320, 240);
        let src = ImageSource::new(&bytes).unwrap();
        let preview = src.embedded_thumbnail().expect("the writer embedded a preview");
        assert_eq!((preview.w, preview.h), (120, 160), "160 x 120 stored, turned a quarter by orientation 6");
        let small = src.fast_thumbnail(64, 64).unwrap();
        assert_eq!((small.w, small.h), (48, 64));
        let big = src.fast_thumbnail(300, 300).unwrap();
        assert_eq!((big.w, big.h), (225, 300), "larger than the preview: decoded in full");
        let plain = representation(&checker(), FileType::Jpeg, &RepProperties::default()).unwrap();
        assert!(ImageSource::new(&plain).unwrap().embedded_thumbnail().is_none());
    }

    #[test]
    fn an_empty_picture_has_no_representation() {
        let empty = Image { w: 0, h: 0, px: Vec::new() };
        assert_eq!(representation(&empty, FileType::Png, &RepProperties::default()), None);
        let short = Image { w: 2, h: 2, px: vec![0; 3] };
        assert_eq!(representation(&short, FileType::Jpeg, &RepProperties::default()), None);
    }
}
