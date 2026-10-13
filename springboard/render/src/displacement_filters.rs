//! Displacement distortion: moves each pixel by an offset read from a displacement map. This is the
//! model of the reference displacement filter, and it works on the packed pixels directly, so no
//! colour arithmetic is involved.
//!
//! The map's red channel moves the pixel horizontally and its green channel vertically. A channel
//! value of 0 gives an offset of `-scale / 2`, a value of 255 gives `+scale / 2`, and the values in
//! between are linear. Mid grey (128) therefore moves by about `scale / 510`, close to none. Sampling
//! is nearest-neighbour, clamped to the image edge.
use crate::gfx::Image;

/// Displaces `img` by `map`. The map must have the image's size; otherwise the image is returned
/// unchanged, as is every image when `scale` is not finite or is zero.
pub fn displacement(img: &Image, map: &Image, scale: f32) -> Image {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.px.len() != w * h || (map.w, map.h) != (w, h) || map.px.len() != w * h {
        return img.clone();
    }
    if !scale.is_finite() || scale == 0.0 {
        return img.clone();
    }
    let mut out = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let m = map.px[y * w + x];
            let dx = scale * ((m & 0xff) as f32 / 255.0 - 0.5);
            let dy = scale * (((m >> 8) & 0xff) as f32 / 255.0 - 0.5);
            let sx = (x as f32 + dx).round().clamp(0.0, (w - 1) as f32) as usize;
            let sy = (y as f32 + dy).round().clamp(0.0, (h - 1) as f32) as usize;
            out.px[y * w + x] = img.px[sy * w + sx];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(values: &[u32]) -> Image {
        Image { w: values.len(), h: 1, px: values.to_vec() }
    }

    #[test]
    fn zero_scale_is_identity() {
        let img = strip(&[1, 2, 3, 4]);
        let map = strip(&[0, 0, 0, 0]);
        assert_eq!(displacement(&img, &map, 0.0).px, img.px);
        assert_eq!(displacement(&img, &map, f32::NAN).px, img.px);
    }

    #[test]
    fn minimum_map_value_shifts_left_by_half_scale() {
        // Red 0 gives an offset of -scale / 2 = -1 with scale 2, so each pixel takes its left neighbour.
        let img = strip(&[0x10, 0x20, 0x30, 0x40]);
        let map = strip(&[0, 0, 0, 0]);
        let out = displacement(&img, &map, 2.0);
        assert_eq!(out.px, vec![0x10, 0x10, 0x20, 0x30]);
    }

    #[test]
    fn maximum_map_value_shifts_right_and_clamps_at_the_edge() {
        let img = strip(&[0x10, 0x20, 0x30, 0x40]);
        let map = strip(&[0xff, 0xff, 0xff, 0xff]);
        let out = displacement(&img, &map, 2.0);
        assert_eq!(out.px, vec![0x20, 0x30, 0x40, 0x40]);
    }

    #[test]
    fn green_channel_moves_vertically() {
        // A 1 by 3 column; green 0 moves each pixel up one row, the top row clamps to itself.
        let img = Image { w: 1, h: 3, px: vec![1, 2, 3] };
        let map = Image { w: 1, h: 3, px: vec![0, 0, 0] };
        let out = displacement(&img, &map, 2.0);
        assert_eq!(out.px, vec![1, 1, 2]);
    }

    #[test]
    fn mismatched_map_leaves_the_image_unchanged() {
        let img = strip(&[1, 2, 3]);
        let map = strip(&[0, 0]);
        assert_eq!(displacement(&img, &map, 4.0).px, img.px);
    }
}
