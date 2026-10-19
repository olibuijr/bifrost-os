//! Forward Servo-decoded favicons; resource discovery and decoding stay in the engine.
use super::*;

pub(super) fn pixels(view: &WebView) -> Option<String> {
    use image::{DynamicImage, GrayAlphaImage, GrayImage, RgbImage, RgbaImage};
    use servo::PixelFormat;
    let icon = view.favicon()?;
    if icon.width == 0 || icon.height == 0 || icon.width > 1024 || icon.height > 1024 {
        return None;
    }
    let data = icon.data().to_vec();
    let image = match icon.format {
        PixelFormat::K8 => DynamicImage::ImageLuma8(GrayImage::from_raw(icon.width, icon.height, data)?),
        PixelFormat::KA8 => DynamicImage::ImageLumaA8(GrayAlphaImage::from_raw(icon.width, icon.height, data)?),
        PixelFormat::RGB8 => DynamicImage::ImageRgb8(RgbImage::from_raw(icon.width, icon.height, data)?),
        PixelFormat::RGBA8 => DynamicImage::ImageRgba8(RgbaImage::from_raw(icon.width, icon.height, data)?),
        PixelFormat::BGRA8 => {
            let mut rgba = data;
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            DynamicImage::ImageRgba8(RgbaImage::from_raw(icon.width, icon.height, rgba)?)
        }
    };
    let rgba = image.resize_exact(32, 32, image::imageops::FilterType::Triangle).into_rgba8();
    use std::fmt::Write;
    let mut encoded = String::with_capacity(32 * 32 * 8);
    for byte in rgba.as_raw() {
        let _ = write!(encoded, "{byte:02x}");
    }
    Some(encoded)
}
