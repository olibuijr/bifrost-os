//! Colour conversion for apps: named RGB working spaces, ICC matrix-shaper profiles and display gamma tables.
//!
//! This is the app-facing face of `bifrost_render::colorspace` (sRGB, Display P3, Adobe RGB, ROMM RGB) and
//! `bifrost_render::icc`. An app opens the bytes of an RGB profile, converts 8-bit colours from it into a working
//! space, and reads the gamma table of a display profile. Profiles that use lookup tables, or that are in a colour
//! space other than RGB, are refused. Rendering intents and black point compensation are in
//! `bifrost_render::rendering_intent`.

pub use bifrost_render::colorspace::{Space, Transform};
pub use bifrost_render::cg::color::RenderingIntent;
pub use bifrost_render::icc::{IccError, MatrixShaper, Profile, Vcgt};
pub use bifrost_render::icc_builtin::Builtin;
use bifrost_render::icc_transform::{Conversion, Model};

/// Why a profile could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorError {
    /// The bytes are not a valid ICC profile.
    Profile(IccError),
    /// The profile is valid but is not an RGB matrix-shaper profile.
    NotMatrixShaper,
    /// The profile has no video card gamma table.
    NoGammaTable,
    /// The profile's conversion model could not be read (a colour space other than RGB or grey, or no tables).
    Model(String),
}

/// Converts colours from one ICC profile into another under a rendering intent: the profile-to-profile transform
/// (`ColorSyncTransform`). Matrix-shaper and lookup-table RGB profiles, and grey profiles, are accepted on both sides.
#[derive(Clone, Debug)]
pub struct ProfileToProfile {
    conversion: Conversion,
}

impl ProfileToProfile {
    /// Prepares the conversion from the `src` profile bytes to the `dst` profile bytes.
    pub fn open(
        src: &[u8],
        dst: &[u8],
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) -> Result<Self, ColorError> {
        let src = Model::from_icc(src).map_err(ColorError::Model)?;
        let dst = Model::from_icc(dst).map_err(ColorError::Model)?;
        Ok(ProfileToProfile { conversion: Conversion::new(src, dst, intent, black_point_compensation) })
    }

    /// Converts one encoded colour (nominally 0..=1). `None` when a profile cannot take the colour.
    pub fn apply(&self, rgb: [f64; 3]) -> Option<[f64; 3]> {
        self.conversion.apply(rgb)
    }

    /// Converts one 8-bit colour. `None` when a profile cannot take the colour.
    pub fn apply_u8(&self, rgb: [u8; 3]) -> Option<[u8; 3]> {
        let out = self.apply(rgb.map(|c| c as f64 / 255.0))?;
        Some(out.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
    }
}

/// Converts colours from an RGB profile into a working space.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileConverter {
    shaper: MatrixShaper,
    to: Space,
}

impl ProfileConverter {
    /// Opens an RGB matrix-shaper profile and prepares its conversion into `to`.
    pub fn open(data: &[u8], to: Space) -> Result<Self, ColorError> {
        let profile = Profile::parse(data).map_err(ColorError::Profile)?;
        let shaper = profile.matrix_shaper(data).ok_or(ColorError::NotMatrixShaper)?;
        Ok(ProfileConverter { shaper, to })
    }

    /// The working space the converter writes.
    pub fn destination(&self) -> Space {
        self.to
    }

    /// Converts one 8-bit profile colour. Colours outside the destination gamut are clamped.
    pub fn apply_u8(&self, rgb: [u8; 3]) -> [u8; 3] {
        let out = self.apply(rgb.map(|c| c as f64 / 255.0));
        out.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// Converts one encoded colour (nominally 0..=1). The result may leave 0..=1.
    pub fn apply(&self, rgb: [f64; 3]) -> [f64; 3] {
        self.shaper.convert(rgb, self.to)
    }
}

/// The gamma table of a display profile as three 16-bit ramps of `entries` values each (red, green, blue).
pub fn gamma_ramps(data: &[u8], entries: usize) -> Result<[Vec<u16>; 3], ColorError> {
    let profile = Profile::parse(data).map_err(ColorError::Profile)?;
    let vcgt = profile.vcgt(data).ok_or(ColorError::NoGammaTable)?;
    Ok([vcgt.ramp(0, entries), vcgt.ramp(1, entries), vcgt.ramp(2, entries)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_that_is_not_a_profile_is_refused() {
        let mut data = vec![0u8; 200];
        data[0..4].copy_from_slice(&200u32.to_be_bytes());
        assert_eq!(ProfileConverter::open(&data, Space::Srgb), Err(ColorError::Profile(IccError::NotAProfile)));
    }

    #[test]
    fn a_profile_with_no_gamma_table_says_so() {
        let mut data = vec![0u8; 200];
        data[0..4].copy_from_slice(&200u32.to_be_bytes());
        data[36..40].copy_from_slice(b"acsp");
        // Header and an empty tag table: the table count at 128 is zero, so there is no vcgt tag.
        assert_eq!(gamma_ramps(&data, 4), Err(ColorError::NoGammaTable));
    }

    #[test]
    fn srgb_to_srgb_is_the_identity_within_rounding() {
        let srgb = Builtin::Srgb.bytes();
        let t = ProfileToProfile::open(&srgb, &srgb, RenderingIntent::RelativeColorimetric, false).unwrap();
        let out = t.apply([0.2, 0.5, 0.8]).unwrap();
        for (a, b) in out.iter().zip([0.2, 0.5, 0.8]) {
            assert!((a - b).abs() < 0.005, "{out:?}");
        }
        assert_eq!(t.apply_u8([128, 64, 200]), Some([128, 64, 200]));
    }

    #[test]
    fn a_grey_source_converts_to_neutral_rgb() {
        let grey = Builtin::Gray22.bytes();
        let srgb = Builtin::Srgb.bytes();
        let t = ProfileToProfile::open(&grey, &srgb, RenderingIntent::Perceptual, true).unwrap();
        let out = t.apply([0.5, 0.5, 0.5]).unwrap();
        assert!((out[0] - out[1]).abs() < 0.01 && (out[1] - out[2]).abs() < 0.01, "{out:?}");
    }

    #[test]
    fn a_profile_that_cannot_be_read_is_refused_on_either_side() {
        let srgb = Builtin::Srgb.bytes();
        let mut bad = vec![0u8; 200];
        bad[0..4].copy_from_slice(&200u32.to_be_bytes());
        assert!(matches!(
            ProfileToProfile::open(&bad, &srgb, RenderingIntent::Default, false),
            Err(ColorError::Model(_))
        ));
        assert!(matches!(
            ProfileToProfile::open(&srgb, &bad, RenderingIntent::Default, false),
            Err(ColorError::Model(_))
        ));
    }
}
