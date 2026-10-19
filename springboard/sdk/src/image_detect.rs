//! Feature detection in app images: QR codes (`CIDetector` of type QR code, `CIQRCodeFeature`). The decoder
//! is `bifrost_render::qr_decode`, the same one the Camera's scan overlay uses; `image_generate::qr_code`
//! makes codes it reads back. Bright axis-aligned rectangles are found by `image_effects::rectangles`.
//! Limits: model 2 codes of versions 1 to 10 only, rotated by any angle but without perspective correction;
//! one code per image (the first that decodes); no faces, text or barcodes.
use bifrost_render::gfx::Image;

/// The text of the QR code in `img`, or `None` when no readable code is found (or the image is under 21 by 21
/// pixels). Light-on-dark codes are tried as well as dark-on-light ones.
pub fn qr_message(img: &Image) -> Option<String> {
    if img.px.len() != img.w.checked_mul(img.h)? {
        return None;
    }
    let luma = bifrost_render::qr_decode::luma_of(&img.px);
    bifrost_render::qr_decode::decode_luma(&luma, img.w, img.h)
}

/// The text of the QR code in an 8-bit luminance image (`w` by `h`, row-major, `luma.len() >= w * h`).
pub fn qr_message_luma(luma: &[u8], w: usize, h: usize) -> Option<String> {
    bifrost_render::qr_decode::decode_luma(luma, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_generate::{qr_code, qr_code_styled, Correction};

    #[test]
    fn a_generated_code_reads_back_in_every_segment_mode() {
        for text in ["https://bifrost.example/pay?id=42", "HELLO WORLD", "0123456789012345"] {
            for level in [Correction::L, Correction::M, Correction::Q, Correction::H] {
                let img = qr_code(text, level, 5).unwrap();
                assert_eq!(qr_message(&img).as_deref(), Some(text), "{text} at {level:?}");
            }
        }
    }

    #[test]
    fn a_light_on_dark_code_reads_back() {
        let img = qr_code_styled("inverted", Correction::M, 5, 4, 0xffff_ffff, 0xff00_0000, false).unwrap();
        assert_eq!(qr_message(&img).as_deref(), Some("inverted"));
    }

    #[test]
    fn images_without_a_code_give_none() {
        assert_eq!(qr_message(&Image { w: 60, h: 60, px: vec![0xff80_8080; 3600] }), None);
        assert_eq!(qr_message(&Image { w: 10, h: 10, px: vec![0; 100] }), None);
        assert_eq!(qr_message(&Image { w: 40, h: 40, px: vec![0; 5] }), None);
    }
}
