//! Distortion effects for app images (`CICategoryDistortionEffect`): twirl, vortex, pinch, bump and
//! glass (`CITwirlDistortion`, `CIVortexDistortion`, `CIPinchDistortion`, `CIBumpDistortion`,
//! `CIGlassDistortion`), the hole and light tunnel (`CIHoleDistortion`, `CILightTunnel`), the torus lens
//! with its ring width (`CITorusLensDistortion`) and the circle splash (`CICircleSplashDistortion`).
//! Displacement stays in `image_effects`. The pixel work lives in the render crate modules
//! `distortion_filters` and `lens_distortions`. Every filter keeps the image's size; points that read
//! outside the image are transparent. `center` is in image pixels from the top left.
//! Limits: CPU only, and the falloff profiles are this system's own models of the reference shapes;
//! see docs/bifrost-ui.md, "Core Image filters".
use bifrost_render::distortion_filters as d;
use bifrost_render::gfx::Image;
use bifrost_render::lens_distortions as l;

pub use bifrost_render::lens_distortions::{HOLE_RADIUS, SPLASH_RADIUS, TORUS_RADIUS, TORUS_REFRACTION, TORUS_WIDTH, TUNNEL_RADIUS};

/// Turns the pixels around `center` by up to `angle` radians, fading out at `radius`.
pub fn twirl(img: &Image, center: (f32, f32), radius: f32, angle: f32) -> Image {
    d::twirl(img, center, radius, angle)
}

/// A twirl whose turn falls off with the square of the distance.
pub fn vortex(img: &Image, center: (f32, f32), radius: f32, angle: f32) -> Image {
    d::vortex(img, center, radius, angle)
}

/// Pulls pixels towards `center`; `scale` 0 to 1.
pub fn pinch(img: &Image, center: (f32, f32), radius: f32, scale: f32) -> Image {
    d::pinch(img, center, radius, scale)
}

/// A bulge (`scale` above 0) or dent (below 0) around `center`; `scale` -1 to 1.
pub fn bump(img: &Image, center: (f32, f32), radius: f32, scale: f32) -> Image {
    d::bump(img, center, radius, scale)
}

/// Displaces pixels by the luminance slope of `texture`, tiled over the image.
pub fn glass(img: &Image, texture: &Image, scale: f32) -> Image {
    d::glass(img, texture, scale)
}

/// A transparent hole of `radius` at `center` with the picture pushed outwards (default radius 150).
pub fn hole(img: &Image, center: (f32, f32), radius: f32) -> Image {
    l::hole(img, center, radius)
}

/// Keeps the disc of `radius` and streams its rim outwards, turned by `rotation` radians per radius
/// (defaults: rotation 0, radius 100).
pub fn light_tunnel(img: &Image, center: (f32, f32), rotation: f32, radius: f32) -> Image {
    l::light_tunnel(img, center, rotation, radius)
}

/// A glass ring of `width` on the circle of `radius` that magnifies by up to `1 + refraction`
/// (defaults: radius 160, width 80, refraction 1.7; refraction runs 0 to 5).
pub fn torus_lens(img: &Image, center: (f32, f32), radius: f32, width: f32, refraction: f32) -> Image {
    l::torus_lens(img, center, radius, width, refraction)
}

/// Keeps the disc of `radius` and splashes its circumference outwards (default radius 150).
pub fn circle_splash(img: &Image, center: (f32, f32), radius: f32) -> Image {
    l::circle_splash_rim(img, center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Image {
        let mut i = Image::new(n, n);
        for (k, p) in i.px.iter_mut().enumerate() {
            *p = (k as u32 % 251) | 0xff00_0000;
        }
        i
    }

    #[test]
    fn distortions_keep_the_size_and_neutral_settings_change_nothing() {
        let img = ramp(16);
        let c = (8.0, 8.0);
        assert_eq!(twirl(&img, c, 5.0, 0.0), img);
        assert_eq!(pinch(&img, c, 5.0, 0.0), img);
        assert_eq!(hole(&img, c, 0.0), img);
        assert_eq!(torus_lens(&img, c, 5.0, 4.0, 0.0), img);
        for out in [vortex(&img, c, 5.0, 1.0), bump(&img, c, 5.0, 0.5), hole(&img, c, 3.0), light_tunnel(&img, c, 1.0, 3.0), circle_splash(&img, c, 3.0)] {
            assert_eq!((out.w, out.h), (16, 16));
        }
        assert_eq!(hole(&img, c, 3.0).px[8 * 16 + 8] >> 24, 0, "the hole is clear");
    }
}
