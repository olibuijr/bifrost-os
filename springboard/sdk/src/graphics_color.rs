//! Colour spaces and colour values for apps. The model is `bifrost_render::cg::color`: a [`Color`] is components in a
//! [`ColorSpace`] plus an alpha. Spaces are device RGB, grey and CMYK, sRGB and its linear and extended forms,
//! Display P3, Rec. 709 and Rec. 2020, Lab, indexed, pattern and ICC profile spaces.
//!
//! ```ignore
//! let red = Color::srgb(1.0, 0.0, 0.0, 1.0);
//! let wide = red.converted(&ColorSpace::DisplayP3, RenderingIntent::Default).unwrap();
//! let profile_space = graphics_color::space_from_profile(&icc_bytes)?;
//! ```
//!
//! Conversions go through XYZ with the rendering intent. Pattern spaces and indexed targets have no conversion.

pub use bifrost_render::cg::color::{Color, ColorConversionInfo, ColorModel, ColorSpace, RenderingIntent};

/// Why an ICC profile was refused.
pub type ProfileError = bifrost_render::cg::color::ProfileError;

/// Reads an ICC profile into a colour space that keeps the profile's own transfer curves and primaries.
pub fn space_from_profile(data: &[u8]) -> Result<ColorSpace, ProfileError> {
    ColorSpace::from_icc(data)
}

/// Converts `color` into `to`. `None` for pattern colours and for indexed targets.
pub fn convert(color: &Color, to: &ColorSpace, intent: RenderingIntent) -> Option<Color> {
    color.converted(to, intent)
}

/// Prepares a conversion once, for many colours in the same pair of spaces.
pub fn conversion(from: &ColorSpace, to: &ColorSpace, intent: RenderingIntent) -> Option<ColorConversionInfo> {
    ColorConversionInfo::new(from, to, intent)
}

/// Whether the colour space covers more than sRGB (Display P3, Rec. 2020 and wide ICC profiles).
pub fn is_wide_gamut(space: &ColorSpace) -> bool {
    space.is_wide_gamut()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn srgb_from_bytes_round_trips_through_bytes() {
        let c = Color::from_rgba8(12, 200, 255, 128);
        assert_eq!(c.to_rgba8(), [12, 200, 255, 128]);
        assert_eq!(c.space(), &ColorSpace::Srgb);
        assert_eq!(c.num_components(), 3);
    }

    #[test]
    fn new_refuses_a_wrong_component_count() {
        assert!(Color::new(ColorSpace::Srgb, &[1.0, 0.0], 1.0).is_none());
        assert!(Color::new(ColorSpace::DeviceCmyk, &[0.0, 0.0, 0.0, 1.0], 1.0).is_some());
    }

    #[test]
    fn alpha_is_clamped_and_nan_is_opaque() {
        assert_eq!(Color::srgb(0.0, 0.0, 0.0, 3.0).alpha(), 1.0);
        assert_eq!(Color::srgb(0.0, 0.0, 0.0, -1.0).alpha(), 0.0);
        assert_eq!(Color::srgb(0.0, 0.0, 0.0, f32::NAN).alpha(), 1.0);
    }

    #[test]
    fn grey_converts_to_equal_rgb_channels() {
        let g = Color::generic_gray(0.5, 1.0);
        let rgb = convert(&g, &ColorSpace::Srgb, RenderingIntent::Default).expect("grey to sRGB");
        let c = rgb.components();
        assert!(close(c[0], c[1]) && close(c[1], c[2]), "{c:?}");
    }

    #[test]
    fn srgb_to_display_p3_and_back_is_stable() {
        let red = Color::srgb(1.0, 0.0, 0.0, 1.0);
        let wide = convert(&red, &ColorSpace::DisplayP3, RenderingIntent::Default).expect("to P3");
        assert!(is_wide_gamut(wide.space()));
        let back = convert(&wide, &ColorSpace::Srgb, RenderingIntent::Default).expect("back to sRGB");
        let c = back.components();
        assert!(close(c[0], 1.0) && close(c[1], 0.0) && close(c[2], 0.0), "{c:?}");
    }

    #[test]
    fn pattern_colours_have_no_rgb_value() {
        let p = Color::new(ColorSpace::pattern(None), &[], 1.0);
        if let Some(p) = p {
            assert_eq!(p.to_rgba8(), [0, 0, 0, 0]);
        }
    }

    #[test]
    fn profile_data_that_is_not_a_profile_is_refused() {
        assert!(space_from_profile(b"not an icc profile").is_err());
    }
}
