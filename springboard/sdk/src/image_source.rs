//! Image sources for apps: read a picture's format and pixel size from its header, decode it,
//! or make a thumbnail that fits a box. The header is read before any pixel is decoded, so an
//! oversized picture is refused without allocating its pixels, and a layout can ask for the
//! thumbnail size before the decode. PNG and JPEG are the formats the renderer decodes; other
//! data is reported as `NotImage`.
use bifrost_render::gfx::Image;
use bifrost_render::imgdec::{self, MAX_SIDE};

/// Container formats an `ImageSource` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

/// Why a picture could not be read, decoded or made into a thumbnail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The bytes do not start like a PNG or a JPEG.
    NotImage,
    /// The data ends before the size fields of the header.
    Truncated,
    /// The picture is wider or taller than the decode limit (`MAX_SIDE`).
    TooLarge { width: usize, height: usize },
    /// The header is present but the pixel data does not decode, or the size is zero.
    Undecodable,
    /// A thumbnail box with a zero side.
    EmptyBox,
}

/// A picture held as bytes by the caller. Its format and size are known before any decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSource<'a> {
    bytes: &'a [u8],
    format: Format,
    width: usize,
    height: usize,
}

impl<'a> ImageSource<'a> {
    /// Reads the format and size from the header. Pictures wider or taller than `MAX_SIDE`
    /// are refused here, before decoding.
    pub fn new(bytes: &'a [u8]) -> Result<Self, SourceError> {
        let (format, (width, height)) = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            (Format::Png, png_size(bytes)?)
        } else if bytes.starts_with(&[0xFF, 0xD8]) {
            (Format::Jpeg, jpeg_size(bytes)?)
        } else {
            return Err(SourceError::NotImage);
        };
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(SourceError::TooLarge { width, height });
        }
        Ok(ImageSource { bytes, format, width, height })
    }

    /// The container format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The pixel size from the header.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The size of a thumbnail that fits `max_w` x `max_h`, computed from the header alone.
    /// The picture is never enlarged.
    pub fn thumbnail_size(&self, max_w: usize, max_h: usize) -> Result<(usize, usize), SourceError> {
        if max_w == 0 || max_h == 0 {
            return Err(SourceError::EmptyBox);
        }
        Ok(imgdec::fit_dims(self.width, self.height, max_w, max_h))
    }

    /// Decodes the whole picture into premultiplied RGBA.
    pub fn decode(&self) -> Result<Image, SourceError> {
        imgdec::decode(self.bytes).ok_or(SourceError::Undecodable)
    }

    /// Decodes the picture scaled to fit `max_w` x `max_h`, keeping the aspect. Never enlarged.
    pub fn thumbnail(&self, max_w: usize, max_h: usize) -> Result<Image, SourceError> {
        self.thumbnail_size(max_w, max_h)?;
        Ok(imgdec::contain(&self.decode()?, max_w, max_h))
    }
}

fn nonzero(width: usize, height: usize) -> Result<(usize, usize), SourceError> {
    if width == 0 || height == 0 {
        Err(SourceError::Undecodable)
    } else {
        Ok((width, height))
    }
}

/// The IHDR chunk follows the 8-byte signature: length (4), type (4), then width and height
/// as big-endian u32 values.
fn png_size(b: &[u8]) -> Result<(usize, usize), SourceError> {
    if b.len() < 24 {
        return Err(SourceError::Truncated);
    }
    if &b[12..16] != &b"IHDR"[..] {
        return Err(SourceError::Undecodable);
    }
    let width = u32::from_be_bytes([b[16], b[17], b[18], b[19]]) as usize;
    let height = u32::from_be_bytes([b[20], b[21], b[22], b[23]]) as usize;
    nonzero(width, height)
}

/// Walks the JPEG segments after the SOI marker until a frame header (SOF0 to SOF15). Each
/// segment is FF, a marker byte and a 2-byte length that counts itself. The frame header holds
/// the precision (1 byte), the height (2 bytes) and the width (2 bytes).
fn jpeg_size(b: &[u8]) -> Result<(usize, usize), SourceError> {
    let mut i = 2;
    while i + 1 < b.len() {
        if b[i] != 0xFF {
            return Err(SourceError::Undecodable);
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0x01 || m == 0xD8 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return Err(SourceError::Undecodable);
        }
        if i + 4 > b.len() {
            return Err(SourceError::Truncated);
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        // 0xC4 (DHT), 0xC8 (JPG) and 0xCC (DAC) sit inside the SOF range but are not frames.
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            if i + 9 > b.len() {
                return Err(SourceError::Truncated);
            }
            let height = u16::from_be_bytes([b[i + 5], b[i + 6]]) as usize;
            let width = u16::from_be_bytes([b[i + 7], b[i + 8]]) as usize;
            return nonzero(width, height);
        }
        if len < 2 {
            return Err(SourceError::Undecodable);
        }
        i += 2 + len;
    }
    Err(SourceError::Truncated)
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
        v.extend_from_slice(&[0x03, 0x01, 0x22, 0x00]);
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
        assert_eq!(ImageSource::new(b"GIF89a------").unwrap_err(), SourceError::NotImage);
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
}
