//! Colour tagging for a shared buffer: the colour space its pixels are in, and an optional ICC profile that describes it.
//! Pure model: a profile is checked for its header fields only (length, signature, data colour space). Nothing is converted.
use crate::surface_attach::{Attachment, Attachments};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    LinearSrgb,
    DisplayP3,
    Gray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorError {
    /// The profile is shorter than its 128-byte header.
    TooShort,
    /// The profile signature field is not the ICC signature.
    BadSignature,
    /// The profile describes a different colour model than the tagged space.
    SpaceMismatch,
}

/// Attachment key under which the colour space name is stored.
pub const ATTACHMENT_KEY: &str = "colour-space";

const HEADER_LEN: usize = 128;
/// Data colour space field of an ICC header.
const COLOR_SPACE_AT: usize = 16;
/// Profile file signature field of an ICC header.
const SIGNATURE_AT: usize = 36;

const ALL_SPACES: [ColorSpace; 4] = [
    ColorSpace::Srgb,
    ColorSpace::LinearSrgb,
    ColorSpace::DisplayP3,
    ColorSpace::Gray,
];

impl ColorSpace {
    pub fn name(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "srgb",
            ColorSpace::LinearSrgb => "linear-srgb",
            ColorSpace::DisplayP3 => "display-p3",
            ColorSpace::Gray => "gray",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        ALL_SPACES.iter().copied().find(|s| s.name() == name)
    }

    /// The data colour space an ICC header must carry for this space.
    fn icc_data_space(self) -> &'static [u8; 4] {
        match self {
            ColorSpace::Gray => b"GRAY",
            _ => b"RGB ",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceColor {
    space: ColorSpace,
    icc: Option<Vec<u8>>,
}

impl SurfaceColor {
    /// A tag with a colour space and no profile.
    pub fn new(space: ColorSpace) -> Self {
        SurfaceColor { space, icc: None }
    }

    /// A tag with a colour space and a profile, after checking the profile header against the space.
    pub fn with_icc(space: ColorSpace, profile: &[u8]) -> Result<Self, ColorError> {
        if profile.len() < HEADER_LEN {
            return Err(ColorError::TooShort);
        }
        if profile[SIGNATURE_AT..SIGNATURE_AT + 4] != b"acsp"[..] {
            return Err(ColorError::BadSignature);
        }
        let want = space.icc_data_space();
        if profile[COLOR_SPACE_AT..COLOR_SPACE_AT + 4] != want[..] {
            return Err(ColorError::SpaceMismatch);
        }
        Ok(SurfaceColor {
            space,
            icc: Some(profile.to_vec()),
        })
    }

    pub fn space(&self) -> ColorSpace {
        self.space
    }

    pub fn icc(&self) -> Option<&[u8]> {
        self.icc.as_deref()
    }

    /// Writes the colour space name into the buffer's attachments.
    pub fn store(&self, attachments: &mut Attachments) {
        attachments.set(
            ATTACHMENT_KEY,
            Attachment::Text(self.space.name().to_string()),
        );
    }

    /// The colour space recorded in the attachments, if one is recorded and known.
    pub fn read(attachments: &Attachments) -> Option<ColorSpace> {
        match attachments.get(ATTACHMENT_KEY) {
            Some(Attachment::Text(name)) => ColorSpace::from_name(name),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128-byte header with the given data colour space and a valid signature.
    fn profile(space: &[u8; 4]) -> Vec<u8> {
        let mut p = vec![0u8; HEADER_LEN];
        p[COLOR_SPACE_AT..COLOR_SPACE_AT + 4].copy_from_slice(space);
        p[SIGNATURE_AT..SIGNATURE_AT + 4].copy_from_slice(b"acsp");
        p
    }

    #[test]
    fn every_space_name_round_trips_and_unknown_names_are_none() {
        for s in ALL_SPACES {
            assert_eq!(ColorSpace::from_name(s.name()), Some(s));
        }
        assert_eq!(ColorSpace::from_name("cmyk"), None);
    }

    #[test]
    fn a_tag_without_a_profile_has_no_icc_data() {
        let c = SurfaceColor::new(ColorSpace::Srgb);
        assert_eq!(c.space(), ColorSpace::Srgb);
        assert!(c.icc().is_none());
    }

    #[test]
    fn a_profile_shorter_than_its_header_is_refused() {
        let p = profile(b"RGB ");
        assert_eq!(
            SurfaceColor::with_icc(ColorSpace::Srgb, &p[..127]),
            Err(ColorError::TooShort)
        );
    }

    #[test]
    fn a_profile_without_the_icc_signature_is_refused() {
        let mut p = profile(b"RGB ");
        p[SIGNATURE_AT] = 0;
        assert_eq!(
            SurfaceColor::with_icc(ColorSpace::Srgb, &p),
            Err(ColorError::BadSignature)
        );
    }

    #[test]
    fn a_grey_space_needs_a_grey_profile() {
        let rgb = profile(b"RGB ");
        assert_eq!(
            SurfaceColor::with_icc(ColorSpace::Gray, &rgb),
            Err(ColorError::SpaceMismatch)
        );
        let grey = profile(b"GRAY");
        let c = SurfaceColor::with_icc(ColorSpace::Gray, &grey).unwrap();
        assert_eq!(c.icc().unwrap().len(), HEADER_LEN);
    }

    #[test]
    fn a_wide_gamut_tag_is_stored_by_name_and_read_back() {
        let p = profile(b"RGB ");
        let c = SurfaceColor::with_icc(ColorSpace::DisplayP3, &p).unwrap();
        let mut attachments = Attachments::new();
        c.store(&mut attachments);
        assert_eq!(
            attachments.get(ATTACHMENT_KEY),
            Some(&Attachment::Text("display-p3".to_string()))
        );
        assert_eq!(SurfaceColor::read(&attachments), Some(ColorSpace::DisplayP3));
    }

    #[test]
    fn reading_an_empty_attachment_set_gives_no_space() {
        assert_eq!(SurfaceColor::read(&Attachments::new()), None);
    }
}
