//! Colour conversion for apps: named RGB working spaces, ICC matrix-shaper profiles and display gamma tables.
//!
//! This is the app-facing face of `bifrost_render::colorspace` (sRGB, Display P3, Adobe RGB, ROMM RGB) and
//! `bifrost_render::icc`. An app opens the bytes of an RGB profile, converts 8-bit colours from it into a working
//! space, and reads the gamma table of a display profile. Profiles that use lookup tables, or that are in a colour
//! space other than RGB, are refused. Rendering intents and black point compensation are in
//! `bifrost_render::rendering_intent`.

pub use bifrost_render::colorspace::{Space, Transform};
pub use bifrost_render::icc::{IccError, MatrixShaper, Profile, Vcgt};

/// Why a profile could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorError {
    /// The bytes are not a valid ICC profile.
    Profile(IccError),
    /// The profile is valid but is not an RGB matrix-shaper profile.
    NotMatrixShaper,
    /// The profile has no video card gamma table.
    NoGammaTable,
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
}
