//! Artwork for a media item as a pure model: `Artwork` (an identifier the system caches by, and the image sizes the item
//! can supply), `ArtworkSize`, and `ArtworkError`. [`Artwork::choose`] picks the size to load for a target,
//! [`aspect_fit`] gives the drawn size inside a target, and [`request_pixels`] turns points and a display scale into
//! the pixel size a provider is asked for (the size argument of an artwork provider).
//!
//! [`ArtworkImage`] is the pixels themselves, the part that crosses the `now_playing` service: straight RGBA8, at most
//! [`WIRE_SIDE_MAX`] on a side, so a publish stays under 90 KB of JSON. [`ArtworkImage::to_json`] and
//! [`ArtworkImage::from_json`] are the wire form (`{w, h, rgba: base64}`); the shell turns it into an image with
//! [`ArtworkImage::to_image`] and falls back to its glyph when there is none or it is malformed. No encoded image
//! (PNG, JPEG) is decoded here: an app decodes its own cover and passes pixels.

use bifrost_core::json::Json;
use bifrost_core::plist::{base64_decode, base64_encode};
use bifrost_render::gfx::Image;

/// The largest side a representation may have, in pixels.
pub const MAX_SIDE: u32 = 16_384;

/// The largest side of the pixels an app sends to the shell, in pixels. 128 by 128 RGBA is 64 KB, 87 KB as base64: the
/// service frame holds it with room for the rest of the publish, and it is sharp on the 44 pt tile and the detail art.
pub const WIRE_SIDE_MAX: u32 = 128;

/// The most bytes of pixel data a wire artwork may carry.
pub const WIRE_BYTES_MAX: usize = (WIRE_SIDE_MAX as usize) * (WIRE_SIDE_MAX as usize) * 4;

/// The pixel size of one image representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtworkSize {
    pub width: u32,
    pub height: u32,
}

impl ArtworkSize {
    /// Width times height, in pixels.
    pub fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// True when this size is at least the target on both sides.
    pub fn covers(self, width: u32, height: u32) -> bool {
        self.width >= width && self.height >= height
    }
}

/// Why an artwork value was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtworkError {
    /// The identifier is empty after trimming.
    EmptyId,
    /// A side is zero or larger than [`MAX_SIDE`] (or [`WIRE_SIDE_MAX`] for wire pixels).
    InvalidSize,
    /// The pixel data is missing, is not base64, or does not hold width times height RGBA pixels.
    BadPixels,
}

/// `Artwork`: an identifier and the representations the item can supply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artwork {
    id: String,
    sizes: Vec<ArtworkSize>,
}

impl Artwork {
    /// `init(id:artworkProvider:)` without the provider: the identifier is trimmed and must not be empty.
    pub fn new(id: &str) -> Result<Self, ArtworkError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(ArtworkError::EmptyId);
        }
        Ok(Artwork { id: id.to_string(), sizes: Vec::new() })
    }

    /// `id`: the identifier the system caches artwork under.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The representations added so far, in the order they were added.
    pub fn sizes(&self) -> &[ArtworkSize] {
        &self.sizes
    }

    /// Adds a representation. A size that is already present is not added twice.
    pub fn add_representation(&mut self, width: u32, height: u32) -> Result<(), ArtworkError> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(ArtworkError::InvalidSize);
        }
        let size = ArtworkSize { width, height };
        if !self.sizes.contains(&size) {
            self.sizes.push(size);
        }
        Ok(())
    }

    /// The representation to load for a target of `width` by `height` pixels: the smallest one that covers the target
    /// on both sides (the first of equal areas). When none covers it, the largest one. `None` when there are none.
    pub fn choose(&self, width: u32, height: u32) -> Option<ArtworkSize> {
        let covering = self
            .sizes
            .iter()
            .copied()
            .filter(|s| s.covers(width, height))
            .min_by_key(|s| s.area());
        covering.or_else(|| self.sizes.iter().copied().max_by_key(|s| s.area()))
    }
}

/// The size of `source` drawn inside a `width` by `height` box, keeping its aspect ratio. Uses whole pixels, rounded
/// down, and fills the box along the side that limits it. `None` when the box has a zero side.
pub fn aspect_fit(source: ArtworkSize, width: u32, height: u32) -> Option<ArtworkSize> {
    if width == 0 || height == 0 || source.width == 0 || source.height == 0 {
        return None;
    }
    let (sw, sh, w, h) = (source.width as u64, source.height as u64, width as u64, height as u64);
    if sw * h <= sh * w {
        // The source is at least as tall as the box's proportions: height fills the box.
        Some(ArtworkSize { width: (sw * h / sh) as u32, height })
    } else {
        Some(ArtworkSize { width, height: (sh * w / sw) as u32 })
    }
}

/// The pixel size to ask a provider for: `points` times `scale_percent` (200 for a 2x display), rounded up. `None` when
/// either value is zero or the result is larger than [`MAX_SIDE`].
pub fn request_pixels(points: u32, scale_percent: u32) -> Option<u32> {
    if points == 0 || scale_percent == 0 {
        return None;
    }
    let pixels = (points as u64 * scale_percent as u64 + 99) / 100;
    if pixels > MAX_SIDE as u64 {
        None
    } else {
        Some(pixels as u32)
    }
}

/// Artwork pixels: straight (not premultiplied) RGBA8 rows, at most [`WIRE_SIDE_MAX`] on a side. The wire form of
/// `MPNowPlayingInfoPropertyArtwork` for the `now_playing` service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl ArtworkImage {
    /// Pixels from straight RGBA8 bytes. Each side is 1 to [`WIRE_SIDE_MAX`] and `rgba` holds exactly width times
    /// height pixels.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ArtworkError> {
        if width == 0 || height == 0 || width > WIRE_SIDE_MAX || height > WIRE_SIDE_MAX {
            return Err(ArtworkError::InvalidSize);
        }
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(ArtworkError::BadPixels);
        }
        Ok(ArtworkImage { width, height, rgba })
    }

    /// Pixels from a premultiplied image as the shell draws them (`bifrost_render::gfx::Image`: R in the low byte of
    /// each `u32`). An image larger than [`WIRE_SIDE_MAX`] is averaged down to fit, keeping its aspect ratio.
    pub fn from_premultiplied(width: usize, height: usize, px: &[u32]) -> Result<Self, ArtworkError> {
        if width == 0 || height == 0 || width.checked_mul(height) != Some(px.len()) {
            return Err(ArtworkError::BadPixels);
        }
        let side = WIRE_SIDE_MAX as usize;
        let (dw, dh) = if width <= side && height <= side {
            (width, height)
        } else if width >= height {
            (side, (height * side / width).max(1))
        } else {
            ((width * side / height).max(1), side)
        };
        let mut rgba = Vec::with_capacity(dw * dh * 4);
        for dy in 0..dh {
            let y0 = dy * height / dh;
            let y1 = ((dy + 1) * height / dh).max(y0 + 1).min(height);
            for dx in 0..dw {
                let x0 = dx * width / dw;
                let x1 = ((dx + 1) * width / dw).max(x0 + 1).min(width);
                let mut sum = [0u64; 4];
                for y in y0..y1 {
                    for p in &px[y * width + x0..y * width + x1] {
                        for (c, total) in sum.iter_mut().enumerate() {
                            *total += ((p >> (8 * c)) & 255) as u64;
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u64;
                let a = sum[3] / n;
                let straight = |c: u64| if a == 0 { 0 } else { ((c / n * 255 + a / 2) / a).min(255) as u8 };
                rgba.extend_from_slice(&[straight(sum[0]), straight(sum[1]), straight(sum[2]), a as u8]);
            }
        }
        ArtworkImage::new(dw as u32, dh as u32, rgba)
    }

    /// A square tile of `side` pixels (clamped to 8 to [`WIRE_SIDE_MAX`]) with a row of vertical waveform bars on a
    /// flat background: the stand-in for an item that has no cover (a recording). `bars` are heights from 0 to 1; each
    /// is drawn centred, rounded to whole pixels and at least 2 px tall. `background` and `ink` are straight RGB.
    pub fn waveform(side: u32, bars: &[f32], background: [u8; 3], ink: [u8; 3]) -> ArtworkImage {
        let side = side.clamp(8, WIRE_SIDE_MAX) as usize;
        let mut rgba = Vec::with_capacity(side * side * 4);
        for _ in 0..side * side {
            rgba.extend_from_slice(&[background[0], background[1], background[2], 255]);
        }
        let n = bars.len();
        let margin = side / 6;
        let span = side - 2 * margin;
        for (i, h) in bars.iter().enumerate() {
            // each bar owns a slot of the span and fills two thirds of it (at least one pixel)
            let (s0, s1) = (i * span / n, ((i + 1) * span / n).max(i * span / n + 1));
            let bar_w = ((s1 - s0) * 2 / 3).max(1);
            let x0 = margin + s0 + (s1 - s0 - bar_w.min(s1 - s0)) / 2;
            let bar_h = ((h.clamp(0.0, 1.0) * span as f32).round() as usize).clamp(2, span);
            let y0 = (side - bar_h) / 2;
            for y in y0..y0 + bar_h {
                for x in x0..(x0 + bar_w).min(side) {
                    let at = (y * side + x) * 4;
                    rgba[at..at + 3].copy_from_slice(&ink);
                }
            }
        }
        ArtworkImage { width: side as u32, height: side as u32, rgba }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The straight RGBA8 bytes, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// A cheap fingerprint of the pixels, so an app can tell a new cover from the one it already sent.
    pub fn fingerprint(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64 ^ ((self.width as u64) << 32 | self.height as u64);
        for b in &self.rgba {
            h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// The wire object: `{"w": n, "h": n, "rgba": "<base64>"}`.
    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("w".to_string(), Json::Num(self.width as f64)),
            ("h".to_string(), Json::Num(self.height as f64)),
            ("rgba".to_string(), Json::Str(base64_encode(&self.rgba))),
        ])
    }

    /// Reads the wire object. Refuses a side that is not a whole number from 1 to [`WIRE_SIDE_MAX`], text that is not
    /// base64 and pixel data of the wrong length. The size is checked before the text is decoded, so an oversized
    /// value costs nothing.
    pub fn from_json(body: &Json) -> Result<ArtworkImage, ArtworkError> {
        let side = |key: &str| {
            let v = body.get(key).and_then(Json::number).ok_or(ArtworkError::InvalidSize)?;
            if v.is_finite() && v >= 1.0 && v <= WIRE_SIDE_MAX as f64 && v.fract() == 0.0 {
                Ok(v as u32)
            } else {
                Err(ArtworkError::InvalidSize)
            }
        };
        let (width, height) = (side("w")?, side("h")?);
        let text = body.get("rgba").and_then(Json::as_str).ok_or(ArtworkError::BadPixels)?;
        if text.len() > WIRE_BYTES_MAX / 3 * 4 + 8 {
            return Err(ArtworkError::BadPixels);
        }
        let rgba = base64_decode(text).ok_or(ArtworkError::BadPixels)?;
        ArtworkImage::new(width, height, rgba)
    }

    /// The pixels as an image the shell draws: premultiplied, R in the low byte.
    pub fn to_image(&self) -> Image {
        let px = self
            .rgba
            .chunks_exact(4)
            .map(|p| {
                let a = p[3] as u32;
                let m = |c: u8| (c as u32 * a + 127) / 255;
                m(p[0]) | m(p[1]) << 8 | m(p[2]) << 16 | a << 24
            })
            .collect();
        Image { w: self.width as usize, h: self.height as usize, px }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> ArtworkSize {
        ArtworkSize { width, height }
    }

    #[test]
    fn an_identifier_is_trimmed_and_must_not_be_empty() {
        assert_eq!(Artwork::new("   "), Err(ArtworkError::EmptyId));
        assert_eq!(Artwork::new(" cover-7 ").unwrap().id(), "cover-7");
    }

    #[test]
    fn a_representation_needs_sides_from_one_to_the_maximum_and_is_not_added_twice() {
        let mut a = Artwork::new("cover").unwrap();
        assert_eq!(a.add_representation(0, 300), Err(ArtworkError::InvalidSize));
        assert_eq!(a.add_representation(300, MAX_SIDE + 1), Err(ArtworkError::InvalidSize));
        assert_eq!(a.add_representation(MAX_SIDE, 1), Ok(()));
        a.add_representation(300, 300).unwrap();
        a.add_representation(300, 300).unwrap();
        assert_eq!(a.sizes(), &[size(MAX_SIDE, 1), size(300, 300)]);
    }

    #[test]
    fn choose_takes_the_smallest_size_that_covers_the_target() {
        let mut a = Artwork::new("cover").unwrap();
        for side in [300, 600, 1200] {
            a.add_representation(side, side).unwrap();
        }
        assert_eq!(a.choose(500, 500), Some(size(600, 600)));
        assert_eq!(a.choose(600, 600), Some(size(600, 600)), "an exact match covers");
        assert_eq!(a.choose(601, 600), Some(size(1200, 1200)), "one short side is not enough");
    }

    #[test]
    fn choose_falls_back_to_the_largest_size_when_none_covers() {
        let mut a = Artwork::new("wide").unwrap();
        a.add_representation(300, 300).unwrap();
        a.add_representation(1200, 400).unwrap();
        // 2000x100 is covered by neither: 1200x400 has the larger area (480000 against 90000).
        assert_eq!(a.choose(2000, 100), Some(size(1200, 400)));
        assert_eq!(Artwork::new("empty").unwrap().choose(10, 10), None);
    }

    #[test]
    fn aspect_fit_fills_the_box_along_the_limiting_side() {
        // Wider than the box: width fills 300, height is 800 * 300 / 1200 = 200.
        assert_eq!(aspect_fit(size(1200, 800), 300, 300), Some(size(300, 200)));
        // Taller than the box: height fills 300, width is 800 * 300 / 1200 = 200.
        assert_eq!(aspect_fit(size(800, 1200), 300, 300), Some(size(200, 300)));
        // Same proportions as the box: both sides fill it.
        assert_eq!(aspect_fit(size(4, 3), 400, 300), Some(size(400, 300)));
        assert_eq!(aspect_fit(size(4, 3), 0, 300), None);
    }

    #[test]
    fn aspect_fit_rounds_down_to_whole_pixels() {
        // 1000 by 300 into 100 by 100: width fills 100, height is 300 * 100 / 1000 = 30.
        assert_eq!(aspect_fit(size(1000, 300), 100, 100), Some(size(100, 30)));
        // 300 by 1000 into 100 by 100: height fills 100, width is 300 * 100 / 1000 = 30.
        assert_eq!(aspect_fit(size(300, 1000), 100, 100), Some(size(30, 100)));
        // 2 by 3 into 10 by 10: height 10, width 2 * 10 / 3 = 6.66, rounded down to 6.
        assert_eq!(aspect_fit(size(2, 3), 10, 10), Some(size(6, 10)));
    }

    #[test]
    fn request_pixels_scales_points_and_rounds_up() {
        assert_eq!(request_pixels(300, 200), Some(600), "300 points on a 2x display");
        assert_eq!(request_pixels(1, 150), Some(2), "1.5 rounds up to 2");
        assert_eq!(request_pixels(100, 100), Some(100), "1x is unchanged");
    }

    #[test]
    fn request_pixels_refuses_zero_and_oversized_requests() {
        assert_eq!(request_pixels(0, 200), None);
        assert_eq!(request_pixels(300, 0), None);
        assert_eq!(request_pixels(20_000, 100), None, "above the maximum side");
    }

    #[test]
    fn wire_pixels_round_trip_through_json_and_back_to_a_premultiplied_image() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 255];
        let art = ArtworkImage::new(2, 2, rgba.clone()).unwrap();
        let text = art.to_json().encode();
        let back = ArtworkImage::from_json(&bifrost_core::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back, art);
        assert_eq!(back.rgba(), &rgba[..]);
        let image = back.to_image();
        assert_eq!((image.w, image.h), (2, 2));
        assert_eq!(image.px[0], 255 | 255 << 24, "opaque red");
        assert_eq!(image.px[1], 128 << 8 | 128 << 24, "half-transparent green is premultiplied");
        assert_eq!(image.px[2], 0, "a transparent pixel has no colour");
    }

    #[test]
    fn the_wire_form_is_bounded_and_refuses_bad_data() {
        assert_eq!(ArtworkImage::new(WIRE_SIDE_MAX + 1, 1, vec![0; 4]), Err(ArtworkError::InvalidSize));
        assert_eq!(ArtworkImage::new(0, 4, vec![]), Err(ArtworkError::InvalidSize));
        assert_eq!(ArtworkImage::new(2, 2, vec![0; 15]), Err(ArtworkError::BadPixels));
        let biggest = ArtworkImage::new(WIRE_SIDE_MAX, WIRE_SIDE_MAX, vec![7; WIRE_BYTES_MAX]).unwrap();
        assert!(biggest.to_json().encode().len() < 90_000, "a publish stays far under the 256 KB service frame");
        let parse = |t: &str| bifrost_core::json::parse(t).unwrap();
        assert_eq!(ArtworkImage::from_json(&parse(r#"{"w":2,"h":2,"rgba":"AAAA"}"#)), Err(ArtworkError::BadPixels), "too few bytes");
        assert_eq!(ArtworkImage::from_json(&parse(r#"{"w":2,"h":2,"rgba":"!!!!"}"#)), Err(ArtworkError::BadPixels), "not base64");
        assert_eq!(ArtworkImage::from_json(&parse(r#"{"w":1000,"h":2,"rgba":""}"#)), Err(ArtworkError::InvalidSize));
        assert_eq!(ArtworkImage::from_json(&parse(r#"{"w":1.5,"h":2,"rgba":""}"#)), Err(ArtworkError::InvalidSize));
        assert_eq!(ArtworkImage::from_json(&parse(r#"{"w":2,"h":2}"#)), Err(ArtworkError::BadPixels), "no pixels");
        let huge = format!(r#"{{"w":2,"h":2,"rgba":"{}"}}"#, "A".repeat(200_000));
        assert_eq!(ArtworkImage::from_json(&parse(&huge)), Err(ArtworkError::BadPixels), "oversized text is refused undecoded");
    }

    #[test]
    fn a_large_premultiplied_image_is_averaged_down_to_the_wire_size() {
        // 256 by 128: the left half opaque white, the right half transparent
        let px: Vec<u32> = (0..256 * 128).map(|i| if i % 256 < 128 { 0xffff_ffff } else { 0 }).collect();
        let art = ArtworkImage::from_premultiplied(256, 128, &px).unwrap();
        assert_eq!((art.width(), art.height()), (WIRE_SIDE_MAX, 64), "the aspect ratio is kept");
        assert_eq!(&art.rgba()[0..4], &[255, 255, 255, 255]);
        let last = art.rgba().len() - 4;
        assert_eq!(&art.rgba()[last..], &[0, 0, 0, 0]);
        let small = ArtworkImage::from_premultiplied(2, 1, &[0xff00_00ff, 0x8000_0080]).unwrap();
        assert_eq!(small.rgba(), &[255, 0, 0, 255, 255, 0, 0, 128], "a small image is kept and un-premultiplied");
        assert_eq!(ArtworkImage::from_premultiplied(2, 2, &[0; 3]), Err(ArtworkError::BadPixels));
    }

    #[test]
    fn a_waveform_tile_draws_centred_bars_and_its_fingerprint_follows_the_bars() {
        let tile = ArtworkImage::waveform(96, &[0.2, 1.0, 0.5], [10, 10, 10], [250, 60, 50]);
        assert_eq!((tile.width(), tile.height()), (96, 96));
        let at = |x: usize, y: usize| tile.rgba()[(y * 96 + x) * 4..(y * 96 + x) * 4 + 4].to_vec();
        assert_eq!(at(0, 0), vec![10, 10, 10, 255], "the corner is background");
        assert_eq!(at(47, 48), vec![250, 60, 50, 255], "the full-height middle bar crosses the centre row");
        assert_eq!(at(47, 17), vec![250, 60, 50, 255], "and reaches the top of the span");
        assert_eq!(at(25, 17), vec![10, 10, 10, 255], "the short first bar does not");
        let other = ArtworkImage::waveform(96, &[0.9, 0.1, 0.5], [10, 10, 10], [250, 60, 50]);
        assert_ne!(tile.fingerprint(), other.fingerprint());
        assert_eq!(tile.fingerprint(), tile.clone().fingerprint());
        assert_eq!(ArtworkImage::waveform(1000, &[], [0; 3], [255; 3]).width(), WIRE_SIDE_MAX, "the side is clamped");
    }
}
