//! More image effects for app images: vibrance, a five-point tone curve, photo looks and false colour,
//! highlight and shadow, edge detection, displacement, depth-of-field blur and bright rectangle
//! detection. Apps decode an image with `ImageSource` and call these on the `Image`. The pixel work
//! lives in the render crate modules `color_curve`, `photo_effects`, `edge_tone_filters`,
//! `displacement_filters`, `depth_blur` and `feature_detect`, which the shell can use too.
//! Limits: these run on the CPU with no lazy image graph; rectangle detection finds axis-aligned
//! regions only (no perspective quadrilaterals, faces, text or barcodes); depth blur needs a depth map
//! from the app, since it does not estimate depth.
use bifrost_render::colorfx::{self, Rgb};
use bifrost_render::gfx::Image;

pub use bifrost_render::color_curve::ToneCurve;
pub use bifrost_render::feature_detect::{Feature, Rect};
pub use bifrost_render::photo_effects::PhotoLook;
pub use bifrost_render::white_balance::{WhiteBalance, DEFAULT_NEUTRAL};

/// Raises the saturation of the less saturated colours of `img` more than the saturated ones. `amount`
/// 0 is the identity; negative values reduce saturation.
pub fn vibrance(img: &Image, amount: f32) -> Image {
    map_colours(img, |c| bifrost_render::color_curve::vibrance(c, amount))
}

/// Maps every channel of `img` through `curve`, keeping alpha.
pub fn tone_curve(img: &Image, curve: &ToneCurve) -> Image {
    map_colours(img, |c| curve.apply(c))
}

/// Applies a photo look (mono, noir, tonal, chrome, fade, instant, process or transfer) to `img`.
pub fn photo_look(img: &Image, look: PhotoLook) -> Image {
    map_colours(img, |c| bifrost_render::photo_effects::apply(look, c))
}

/// False colour: black pixels become `dark` and white pixels become `light`, with the colours between
/// mixed by luma.
pub fn false_colour(img: &Image, dark: Rgb, light: Rgb) -> Image {
    map_colours(img, |c| bifrost_render::photo_effects::false_colour(c, dark, light))
}

/// White balance as a white-point conversion (`CITemperatureAndTint`): `neutral` is the (kelvin, tint)
/// of the light the picture was taken under and `target` the light it should look taken under; both
/// default to `DEFAULT_NEUTRAL` (6500 K, tint 0). A neutral above the target warms the picture; positive
/// tint is magenta. Alpha is kept.
pub fn temperature_and_tint(img: &Image, neutral: (f32, f32), target: (f32, f32)) -> Image {
    let wb = WhiteBalance::new(neutral, target);
    map_colours(img, |c| wb.apply(c))
}

/// Darkens the highlights (`highlights` 0 to 1, default 1) and lifts or darkens the shadows
/// (`shadows` -1 to 1, default 0). The defaults leave the image unchanged.
pub fn highlight_shadow(img: &Image, highlights: f32, shadows: f32) -> Image {
    bifrost_render::edge_tone_filters::highlight_shadow(img, highlights, shadows)
}

/// The edges of `img` as a grey image: the Sobel gradient magnitude of the luma, times `scale`.
pub fn edges(img: &Image, scale: f32) -> Image {
    bifrost_render::edge_tone_filters::edges(img, scale)
}

/// Moves each pixel of `img` by the red and green values of `map` (the same size as `img`), scaled by
/// `scale` pixels. A map value of 0 moves by `-scale / 2` and 255 by `+scale / 2`. Returns `img`
/// unchanged when the sizes differ.
pub fn displacement(img: &Image, map: &Image, scale: f32) -> Image {
    bifrost_render::displacement_filters::displacement(img, map, scale)
}

/// Depth-of-field blur. `depth` holds one depth in metres per pixel (row-major, the length of the
/// image). Pixels at `focus` metres stay sharp and pixels `falloff` metres or further away take
/// `max_radius` pixels of blur. Returns `None` on a length mismatch or a `falloff` that is not positive.
pub fn depth_blur(img: &Image, depth: &[f32], focus: f32, falloff: f32, max_radius: f32) -> Option<Image> {
    bifrost_render::depth_blur::depth_blur(img, depth, focus, falloff, max_radius)
}

/// The bright regions of `img` (luma at least `threshold`) that have at least `min_pixels` pixels and
/// fill at least `min_fill` (0 to 1) of their bounding box, largest first. A `min_fill` of 0.95 keeps
/// solid rectangles.
pub fn rectangles(img: &Image, threshold: u8, min_pixels: usize, min_fill: f32) -> Vec<Feature> {
    bifrost_render::feature_detect::filled_rectangles(img, threshold, min_pixels, min_fill)
}

/// Applies `f` to every pixel of `img` as straight RGB, keeping alpha.
fn map_colours(img: &Image, f: impl Fn(Rgb) -> Rgb) -> Image {
    let px = img.px.iter().map(|&p| colorfx::map_pixel(p, &f)).collect();
    Image { w: img.w, h: img.h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn temperature_and_tint_keeps_alpha_and_warms_with_a_bluer_neutral() {
        let img = solid(1, 1, 0xff80_8080);
        let warm = temperature_and_tint(&img, (9000.0, 0.0), DEFAULT_NEUTRAL);
        assert_eq!(warm.px[0] >> 24, 0xff);
        assert!((warm.px[0] & 0xff) > (warm.px[0] >> 16 & 0xff), "red above blue");
        let same = temperature_and_tint(&img, DEFAULT_NEUTRAL, DEFAULT_NEUTRAL);
        assert!(((same.px[0] & 0xff) as i32 - 0x80).abs() <= 1);
    }

    #[test]
    fn identity_settings_leave_the_image_unchanged() {
        let img = solid(2, 2, 0xff40_80c0);
        assert_eq!(vibrance(&img, 0.0), img);
        assert_eq!(tone_curve(&img, &ToneCurve::identity()), img);
        assert_eq!(highlight_shadow(&img, 1.0, 0.0), img);
    }

    #[test]
    fn photo_look_keeps_the_size_and_alpha() {
        let img = solid(3, 1, 0x8040_80c0);
        let out = photo_look(&img, PhotoLook::Mono);
        assert_eq!((out.w, out.h, out.px.len()), (3, 1, 3));
        assert!(out.px.iter().all(|&p| p >> 24 == 0x80));
    }

    #[test]
    fn false_colour_maps_white_to_the_light_colour() {
        let img = solid(1, 1, 0xffff_ffff);
        let dark = Rgb::new(0.0, 0.0, 0.0);
        let light = Rgb::new(1.0, 0.0, 0.0);
        let out = false_colour(&img, dark, light);
        assert_eq!(out.px[0] & 0xff, 0xff);
        assert_eq!((out.px[0] >> 8) & 0xff, 0);
    }

    #[test]
    fn displacement_refuses_a_size_mismatch_by_returning_the_input() {
        let img = solid(2, 2, 0xff10_2030);
        let map = solid(1, 1, 0);
        assert_eq!(displacement(&img, &map, 4.0), img);
    }

    #[test]
    fn depth_blur_refuses_a_depth_slice_of_the_wrong_length() {
        let img = solid(2, 2, 0xff10_2030);
        assert!(depth_blur(&img, &[1.0; 3], 1.0, 1.0, 2.0).is_none());
        assert!(depth_blur(&img, &[1.0; 4], 1.0, 1.0, 2.0).is_some());
    }

    #[test]
    fn rectangles_finds_a_solid_block() {
        let mut img = solid(6, 6, 0xff00_0000);
        for y in 1..4 {
            for x in 2..5 {
                img.px[y * 6 + x] = 0xffff_ffff;
            }
        }
        let found = rectangles(&img, 128, 4, 0.95);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bounds, Rect { x: 2, y: 1, w: 3, h: 3 });
    }
}
