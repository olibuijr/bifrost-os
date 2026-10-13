//! Image analysis types and Visual Look Up requests: the model behind an image analysis interaction's
//! `analysisTypes` (text, Visual Look Up, machine-readable codes and subjects) and the request that a Look Up
//! action sends for one subject.
//!
//! The lookup itself is a system service and is not in this crate. This module decides which analyses an image
//! view runs and builds the request the service needs: the subject's id, its bounds in the image, and its cutout.

use crate::subject_lift::{Cutout, Interaction, PixelRect, Rgba};

/// A set of image analyses, as the image analysis interaction's `analysisTypes` option set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnalysisTypes(u8);

impl AnalysisTypes {
    pub const NONE: Self = Self(0);
    pub const TEXT: Self = Self(1);
    pub const VISUAL_LOOK_UP: Self = Self(2);
    pub const MACHINE_READABLE_CODE: Self = Self(4);
    pub const SUBJECT: Self = Self(8);
    pub const ALL: Self = Self(1 | 2 | 4 | 8);

    pub fn bits(self) -> u8 {
        self.0
    }

    /// The set with bits that name no analysis dropped.
    pub fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every analysis in `other` is in this set. Every set contains the empty set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The set without the analyses in `other`.
    pub fn remove(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl std::ops::BitOr for AnalysisTypes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

/// A Visual Look Up request for one subject of an image, ready for the lookup service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookUpRequest {
    /// The id of the subject, as the interaction names it.
    pub subject: u32,
    /// Where the subject sits in the image, in image pixels.
    pub bounds: PixelRect,
    /// The subject's pixels with the background transparent.
    pub cutout: Cutout,
}

/// Why a Look Up request was not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookUpError {
    /// The analysis types of the view do not include Visual Look Up.
    NotEnabled,
    /// The interaction has no subject with that id.
    UnknownSubject,
    /// The image is not the size of the subject's mask.
    ImageSizeMismatch,
}

/// Builds the Look Up request for `subject` in `image`, when the view runs Visual Look Up.
pub fn look_up(
    types: AnalysisTypes,
    interaction: &Interaction,
    subject: u32,
    image: &Rgba,
) -> Result<LookUpRequest, LookUpError> {
    if !types.contains(AnalysisTypes::VISUAL_LOOK_UP) {
        return Err(LookUpError::NotEnabled);
    }
    let s = interaction.subject(subject).ok_or(LookUpError::UnknownSubject)?;
    let bounds = s.bounds();
    let cutout = interaction.lift(subject, image).ok_or(LookUpError::ImageSizeMismatch)?;
    Ok(LookUpRequest { subject, bounds, cutout })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subject_lift::{Mask, Subject};

    fn interaction() -> Interaction {
        let mask = Mask::new(2, 2, vec![255, 0, 0, 0]).expect("mask");
        Interaction::new(vec![Subject::new(3, mask).expect("subject")])
    }

    fn image(w: usize, h: usize) -> Rgba {
        Rgba::new(w, h, vec![1, 2, 3, 255].repeat(w * h)).expect("image")
    }

    #[test]
    fn analysis_types_combine_and_drop_unknown_bits() {
        let set = AnalysisTypes::TEXT | AnalysisTypes::SUBJECT;
        assert!(set.contains(AnalysisTypes::SUBJECT));
        assert!(!set.contains(AnalysisTypes::VISUAL_LOOK_UP));
        assert!(set.contains(AnalysisTypes::NONE));
        assert_eq!(AnalysisTypes::from_bits(0xff), AnalysisTypes::ALL);
        assert_eq!(set.remove(AnalysisTypes::TEXT), AnalysisTypes::SUBJECT);
        assert_eq!(set.intersection(AnalysisTypes::TEXT), AnalysisTypes::TEXT);
        assert!(AnalysisTypes::default().is_empty());
        assert_eq!(AnalysisTypes::ALL.bits(), 15);
    }

    #[test]
    fn look_up_needs_visual_look_up_in_the_analysis_types() {
        let it = interaction();
        let img = image(2, 2);
        let err = look_up(AnalysisTypes::TEXT | AnalysisTypes::SUBJECT, &it, 3, &img);
        assert_eq!(err, Err(LookUpError::NotEnabled));
    }

    #[test]
    fn look_up_builds_a_request_with_bounds_and_cutout() {
        let it = interaction();
        let img = image(2, 2);
        let req = look_up(AnalysisTypes::VISUAL_LOOK_UP, &it, 3, &img).expect("request");
        assert_eq!(req.subject, 3);
        assert_eq!(req.bounds, PixelRect { x: 0, y: 0, w: 1, h: 1 });
        assert_eq!((req.cutout.width(), req.cutout.height()), (1, 1));
        assert_eq!(req.cutout.pixels(), &[1, 2, 3, 255]);
    }

    #[test]
    fn look_up_refuses_an_unknown_subject_and_a_wrong_size_image() {
        let it = interaction();
        assert_eq!(look_up(AnalysisTypes::ALL, &it, 4, &image(2, 2)), Err(LookUpError::UnknownSubject));
        assert_eq!(look_up(AnalysisTypes::ALL, &it, 3, &image(3, 2)), Err(LookUpError::ImageSizeMismatch));
    }
}
