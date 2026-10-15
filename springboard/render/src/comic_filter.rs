//! Comic effect: flat colours with black outlines. Each colour channel is reduced to a few levels, and
//! the pixels on a strong edge (the Sobel edge magnitude from `edge_tone_filters::edges`) are drawn in
//! black. The alpha of every pixel is kept.
//! Differences from the reference: the reference effect has no parameters; here the number of levels and
//! the edge threshold are set by the caller, with the defaults of 4 levels and a threshold of 0.25.
use crate::edge_tone_filters::edges;
use crate::gfx::Image;

/// The default number of levels per colour channel.
pub const DEFAULT_LEVELS: u8 = 4;

/// The default edge threshold, as a fraction of the full edge strength.
pub const DEFAULT_THRESHOLD: f32 = 0.25;

/// Flattens `img` to `levels` tones per channel (clamped to 2..=8) and outlines the edges whose
/// strength is above `threshold` (clamped to 0..=1) in black. A threshold that is not finite uses
/// `DEFAULT_THRESHOLD`.
pub fn comic(img: &Image, levels: u8, threshold: f32) -> Image {
    let levels = levels.clamp(2, 8) as f32;
    let threshold = if threshold.is_finite() { threshold.clamp(0.0, 1.0) } else { DEFAULT_THRESHOLD };
    let edge = edges(img, 1.0);
    let mut out = Image::new(img.w, img.h);
    for (i, (&p, &e)) in img.px.iter().zip(edge.px.iter()).enumerate() {
        let alpha = p >> 24;
        if (e & 255) as f32 / 255.0 > threshold {
            out.px[i] = alpha << 24;
            continue;
        }
        let mut channels = [0u32; 3];
        for (k, channel) in channels.iter_mut().enumerate() {
            let v = ((p >> (8 * k)) & 255) as f32 / 255.0;
            let tone = (v * (levels - 1.0)).round() / (levels - 1.0);
            *channel = ((tone * 255.0 + 0.5) as u32).min(alpha);
        }
        out.px[i] = channels[0] | channels[1] << 8 | channels[2] << 16 | alpha << 24;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_area_keeps_its_tone_and_alpha() {
        let mut img = Image::new(3, 3);
        img.px.iter_mut().for_each(|p| *p = 0xffff_ffff);
        let out = comic(&img, 2, 0.25);
        assert_eq!(out.px, vec![0xffff_ffff; 9]);
    }

    #[test]
    fn a_strong_edge_is_drawn_in_black() {
        let mut img = Image::new(4, 1);
        for x in 0..4 {
            img.px[x] = if x < 2 { 0xff00_0000 } else { 0xffff_ffff };
        }
        let out = comic(&img, 4, 0.25);
        assert_eq!(out.px[1] & 0x00ff_ffff, 0, "the pixel beside the step is black");
        assert_eq!(out.px[1] >> 24, 0xff, "the outline keeps the alpha");
    }

    #[test]
    fn transparent_pixels_stay_transparent() {
        let img = Image::new(2, 2);
        assert_eq!(comic(&img, DEFAULT_LEVELS, DEFAULT_THRESHOLD).px, vec![0; 4]);
    }

    #[test]
    fn a_nan_threshold_falls_back_to_the_default() {
        let mut img = Image::new(4, 1);
        for x in 0..4 {
            img.px[x] = if x < 2 { 0xff00_0000 } else { 0xffff_ffff };
        }
        assert_eq!(comic(&img, 4, f32::NAN).px, comic(&img, 4, DEFAULT_THRESHOLD).px);
    }
}
