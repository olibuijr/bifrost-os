//! Depth-of-field blur driven by a depth map, and the disparity (inverse depth) conversions that go
//! with it. A pixel is blurred in proportion to how far its depth is from the focus distance: at the
//! focus depth it stays sharp, and at `falloff` metres away (or further) it takes the full blur
//! radius. The blur itself is `blur_filters::masked_variable_blur`, so the weights are the mask.
//!
//! Depth is in metres, one value per pixel in row-major order. A depth that is not finite is taken as
//! far away, so it gets the full blur. This is a model of the reference portrait effect: it does not
//! estimate depth from a stereo pair or a single image, and it has no lens shape.
use crate::blur_filters::{masked_variable_blur, Edge};
use crate::gfx::Image;

/// Disparity (inverse depth, in 1/m) of a depth in metres. A depth that is not finite or not positive
/// gives 0, which is the disparity of a point at infinity.
pub fn depth_to_disparity(depth: f32) -> f32 {
    if depth.is_finite() && depth > 0.0 {
        1.0 / depth
    } else {
        0.0
    }
}

/// Depth in metres of a disparity. A disparity that is not finite or not positive gives infinity.
pub fn disparity_to_depth(disparity: f32) -> f32 {
    if disparity.is_finite() && disparity > 0.0 {
        1.0 / disparity
    } else {
        f32::INFINITY
    }
}

/// The blur weight of each pixel as a mask alpha, 0 (sharp) to 255 (full blur). The weight is
/// `|depth - focus| / falloff`, clamped to 1. Returns `None` when `falloff` is not a finite positive
/// number.
pub fn blur_weights(depth: &[f32], focus: f32, falloff: f32) -> Option<Vec<u8>> {
    if !(falloff.is_finite() && falloff > 0.0) {
        return None;
    }
    let weight = |d: f32| -> u8 {
        if !d.is_finite() {
            return 255;
        }
        let t = ((d - focus).abs() / falloff).clamp(0.0, 1.0);
        (t * 255.0 + 0.5) as u8
    };
    Some(depth.iter().map(|&d| weight(d)).collect())
}

/// Depth-of-field blur of `img`. `depth` holds one depth in metres per pixel and must have the length
/// of the image's buffer. Pixels at `focus` metres stay sharp, and pixels `falloff` metres or further
/// from it are blurred by `max_radius` pixels (standard deviation). Returns `None` on a length mismatch
/// or an invalid `falloff`.
pub fn depth_blur(img: &Image, depth: &[f32], focus: f32, falloff: f32, max_radius: f32) -> Option<Image> {
    if depth.len() != img.px.len() {
        return None;
    }
    let alpha = blur_weights(depth, focus, falloff)?;
    let mut mask = Image::new(img.w, img.h);
    for (m, a) in mask.px.iter_mut().zip(alpha) {
        *m = (a as u32) << 24;
    }
    Some(masked_variable_blur(img, &mask, max_radius, Edge::Clamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[u32]) -> Image {
        Image { w: values.len(), h: 1, px: values.to_vec() }
    }

    #[test]
    fn disparity_and_depth_are_inverses() {
        assert!((depth_to_disparity(2.0) - 0.5).abs() < 1e-6);
        assert!((disparity_to_depth(0.5) - 2.0).abs() < 1e-6);
        assert_eq!(depth_to_disparity(f32::INFINITY), 0.0);
        assert_eq!(depth_to_disparity(0.0), 0.0);
        assert!(disparity_to_depth(0.0).is_infinite());
    }

    #[test]
    fn weights_are_zero_at_focus_and_full_at_falloff() {
        let w = blur_weights(&[1.0, 2.0, 3.0, 10.0, f32::NAN], 1.0, 2.0).unwrap();
        assert_eq!(w[0], 0);
        assert_eq!(w[1], 128);
        assert_eq!(w[2], 255);
        assert_eq!(w[3], 255);
        assert_eq!(w[4], 255);
    }

    #[test]
    fn invalid_falloff_is_rejected() {
        assert!(blur_weights(&[1.0], 1.0, 0.0).is_none());
        assert!(blur_weights(&[1.0], 1.0, -1.0).is_none());
        assert!(blur_weights(&[1.0], 1.0, f32::NAN).is_none());
    }

    #[test]
    fn in_focus_image_is_unchanged() {
        let img = row(&[0, 0xffff_ffff, 0, 0xff80_4020, 0]);
        let depth = vec![1.0; 5];
        let out = depth_blur(&img, &depth, 1.0, 1.0, 4.0).unwrap();
        assert_eq!(out.px, img.px);
    }

    #[test]
    fn out_of_focus_impulse_spreads() {
        let mut values = vec![0u32; 9];
        values[4] = 0xffff_ffff;
        let img = row(&values);
        let depth = vec![5.0; 9];
        let out = depth_blur(&img, &depth, 1.0, 1.0, 4.0).unwrap();
        assert!(out.px[3] & 0xff > 0, "neighbour picks up light");
        assert!(out.px[4] & 0xff < 0xff, "centre loses energy");
    }

    #[test]
    fn mismatched_depth_length_is_rejected() {
        let img = row(&[0, 0, 0]);
        assert!(depth_blur(&img, &[1.0, 1.0], 1.0, 1.0, 2.0).is_none());
    }
}
