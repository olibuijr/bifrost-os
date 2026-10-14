//! Reading and writing the picture formats the Images actions use. Reading goes through the SDK's `ImageSource`, which
//! decodes what the shell can show. Writing supports PNG (through the `png` crate) and BMP (32-bit, written here).
//! Pixels in an `Image` are premultiplied; they are straightened back to plain RGBA before they are written.

use bifrost_app::image_source::ImageSource;
use bifrost_app::kit::Image;
use std::path::Path;

/// The formats an image can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Bmp,
}

impl Format {
    pub fn from_name(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "png" => Some(Format::Png),
            "bmp" => Some(Format::Bmp),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Bmp => "bmp",
        }
    }
}

/// Decodes the picture in `path`.
pub fn decode_file(path: &Path) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let source = ImageSource::new(&bytes)
        .map_err(|e| format!("{} is not a readable picture ({e:?})", path.display()))?;
    source
        .decode()
        .map_err(|e| format!("{} could not be decoded ({e:?})", path.display()))
}

/// The picture as the bytes of a file in `format`.
pub fn encode(image: &Image, format: Format) -> Result<Vec<u8>, String> {
    match format {
        Format::Png => encode_png(image),
        Format::Bmp => Ok(encode_bmp(image)),
    }
}

/// One premultiplied pixel as straight RGBA bytes.
fn unpremultiply(p: u32) -> [u8; 4] {
    let (r, g, b, a) = (p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff, p >> 24);
    let straight = |c: u32| {
        if a == 0 {
            0
        } else {
            ((c * 255 + a / 2) / a).min(255) as u8
        }
    };
    [straight(r), straight(g), straight(b), a as u8]
}

fn encode_png(image: &Image) -> Result<Vec<u8>, String> {
    let rgba: Vec<u8> = image.px.iter().flat_map(|&p| unpremultiply(p)).collect();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.w as u32, image.h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A 32-bit uncompressed BMP with the rows bottom-up, as Windows bitmaps are stored (54-byte header, BGRA pixels).
fn encode_bmp(image: &Image) -> Vec<u8> {
    let (w, h) = (image.w, image.h);
    let data_len = w * h * 4;
    let mut out = Vec::with_capacity(54 + data_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = unpremultiply(image.px[y * w + x]);
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 by 2 picture: opaque red, green, blue and a half-transparent white.
    fn sample() -> Image {
        Image {
            w: 2,
            h: 2,
            px: vec![
                0xff00_00ff, // opaque red (r in the low byte)
                0xff00_ff00, // opaque green
                0xffff_0000, // opaque blue
                0x8080_8080, // half-transparent grey (premultiplied)
            ],
        }
    }

    #[test]
    fn format_names_parse() {
        assert_eq!(Format::from_name(" PNG "), Some(Format::Png));
        assert_eq!(Format::from_name("bmp"), Some(Format::Bmp));
        assert_eq!(Format::from_name("jpeg"), None);
        assert_eq!(Format::Bmp.extension(), "bmp");
    }

    #[test]
    fn unpremultiply_straightens_colour() {
        assert_eq!(unpremultiply(0x0000_0000), [0, 0, 0, 0]);
        assert_eq!(unpremultiply(0xff10_2030), [0x30, 0x20, 0x10, 0xff]);
        // premultiplied 128 at alpha 128 is about 255 straight
        assert_eq!(unpremultiply(0x8080_8080), [255, 255, 255, 128]);
    }

    #[test]
    fn bmp_has_header_and_pixel_data() {
        let bytes = encode(&sample(), Format::Bmp).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(bytes.len(), 54 + 2 * 2 * 4);
        assert_eq!(
            u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
            54
        );
        assert_eq!(
            i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]),
            2
        );
        assert_eq!(
            i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]),
            2
        );
        // bottom-up: the first stored pixel is the bottom-left one, blue (BGRA: b=255, g=0, r=0)
        assert_eq!(&bytes[54..58], &[0xff, 0x00, 0x00, 0xff]);
    }

    #[test]
    fn png_starts_with_its_signature() {
        let bytes = encode(&sample(), Format::Png).unwrap();
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }
}
