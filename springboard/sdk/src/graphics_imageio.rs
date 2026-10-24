//! Image files for apps: recognise a format from its bytes, read the first image as pixels, and write pixels back out
//! as PNG, JPEG, GIF, BMP, TIFF, ICO or WebP. The codecs are `bifrost_render::cg::imageio`. WebP is read in every
//! variant (lossy, lossless, animated) and written lossless.
//!
//! ```ignore
//! let format = graphics_imageio::detect(&bytes); // Some(Format::Jpeg)
//! let picture = graphics_imageio::decode(&bytes); // premultiplied pixels, first frame
//! let png = graphics_imageio::encode(Format::Png, w, h, &rgba).unwrap();
//! ```
//!
//! Pixels passed to the writers are straight (not premultiplied) RGBA, four bytes per pixel, rows top to bottom.

use bifrost_render::cg::image::CgImage;
use bifrost_render::cg::imageio::{Format, ImageDestination, WriteOptions};
use bifrost_render::gfx::Image;

/// The format of the image in `bytes`, from its first bytes.
pub fn detect(bytes: &[u8]) -> Option<Format> {
    Format::detect(bytes)
}

/// The pixel size of the first image in `bytes`, read from its header only.
pub fn pixel_size(bytes: &[u8]) -> Option<(usize, usize)> {
    bifrost_render::cg::imageio::pixel_size(bytes)
}

/// The first image in `bytes` as premultiplied pixels (the first frame of an animation). `None` for data that is not
/// an image in a supported format.
pub fn decode(bytes: &[u8]) -> Option<Image> {
    bifrost_render::cg::imageio::decode(bytes)
}

/// Whether [`encode`] can write `format`.
pub fn can_write(format: Format) -> bool {
    format.can_write()
}

/// Writes straight RGBA pixels as one image in `format` (WebP lossless). `None` for a pixel buffer whose length is
/// not `width * height * 4`, or when the codec refuses the size.
pub fn encode(format: Format, width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    encode_with(format, width, height, rgba, None)
}

/// Writes a JPEG at `quality` (0.0 to 1.0; values outside are clamped).
pub fn encode_jpeg(width: usize, height: usize, rgba: &[u8], quality: f32) -> Option<Vec<u8>> {
    let q = if quality.is_nan() { 0.8 } else { quality.clamp(0.0, 1.0) };
    encode_with(Format::Jpeg, width, height, rgba, Some(q))
}

fn encode_with(format: Format, width: usize, height: usize, rgba: &[u8], quality: Option<f32>) -> Option<Vec<u8>> {
    if !format.can_write() || width == 0 || height == 0 || rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let image = CgImage::from_rgba8(width, height, rgba.to_vec())?;
    let mut dest = ImageDestination::new(format, 1)?;
    let options = WriteOptions { quality, ..WriteOptions::default() };
    dest.add_image(&image, Some(options));
    dest.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_pixels() -> Vec<u8> {
        vec![255, 0, 0, 255, 0, 0, 255, 255]
    }

    #[test]
    fn png_round_trips_size_and_format() {
        let bytes = encode(Format::Png, 2, 1, &two_pixels()).expect("png encodes");
        assert_eq!(detect(&bytes), Some(Format::Png));
        assert_eq!(pixel_size(&bytes), Some((2, 1)));
        let img = decode(&bytes).expect("png decodes");
        assert_eq!((img.w, img.h), (2, 1));
    }

    #[test]
    fn gif_and_bmp_write_and_read_back() {
        for format in [Format::Gif, Format::Bmp] {
            let bytes = encode(format, 2, 1, &two_pixels()).expect("encodes");
            assert_eq!(detect(&bytes), Some(format), "{format:?}");
            assert_eq!(pixel_size(&bytes), Some((2, 1)), "{format:?}");
        }
    }

    #[test]
    fn jpeg_keeps_its_size_at_any_quality() {
        let bytes = encode_jpeg(2, 1, &two_pixels(), 0.5).expect("jpeg encodes");
        assert_eq!(detect(&bytes), Some(Format::Jpeg));
        assert_eq!(pixel_size(&bytes), Some((2, 1)));
        assert!(encode_jpeg(2, 1, &two_pixels(), f32::NAN).is_some());
    }

    #[test]
    fn webp_is_written_lossless() {
        assert!(can_write(Format::Webp));
        let bytes = encode(Format::Webp, 2, 1, &two_pixels()).expect("webp encodes");
        assert_eq!(detect(&bytes), Some(Format::Webp));
        let img = decode(&bytes).expect("webp decodes");
        assert_eq!(img.px, decode(&encode(Format::Png, 2, 1, &two_pixels()).unwrap()).unwrap().px, "lossless");
    }

    #[test]
    fn a_wrong_pixel_count_or_an_empty_size_is_refused() {
        assert!(encode(Format::Png, 2, 1, &[0; 7]).is_none());
        assert!(encode(Format::Png, 0, 1, &[]).is_none());
    }

    #[test]
    fn data_that_is_not_an_image_is_not_decoded() {
        assert_eq!(detect(b"hello world"), None);
        assert!(decode(b"hello world").is_none());
    }
}
