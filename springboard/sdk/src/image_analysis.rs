//! Image analysis for apps: subject lifting and Visual Look Up (the subjects and highlighted subjects of an image
//! analysis interaction, the `analysisTypes` option set, and the Look Up request for a subject).
//!
//! The models live in `bifrost_ui` so the shell and apps share one API. A segmenter gives one alpha [`Mask`] per
//! subject; [`Interaction`] holds the subjects, hit tests them, keeps the highlight, and lifts a [`Cutout`]. [`look_up`]
//! builds the [`LookUpRequest`] for the system lookup service. [`segment_background`] finds the subject of a picture
//! whose backdrop is close to one colour. Limits: no segmentation model and no lookup service here, so the apps get
//! the shape of the API and the data, not the recognition itself.

pub use bifrost_ui::subject_lift::{segment_background, Cutout, Interaction, Mask, PixelRect, Rgba, Subject, SUBJECT_THRESHOLD};
pub use bifrost_ui::visual_lookup::{look_up, AnalysisTypes, LookUpError, LookUpRequest};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sdk_exposes_the_analysis_types_and_the_threshold() {
        let set = AnalysisTypes::VISUAL_LOOK_UP | AnalysisTypes::SUBJECT;
        assert!(set.contains(AnalysisTypes::SUBJECT));
        assert_eq!(SUBJECT_THRESHOLD, 128);
        assert!(Subject::new(1, Mask::new(1, 1, vec![0]).expect("mask")).is_none());
    }

    #[test]
    fn the_colour_key_finds_the_subject_on_a_plain_backdrop() {
        // A 4x4 white image with one opaque black pixel at (2, 1): the mask covers that pixel only.
        let mut px = vec![255u8; 4 * 4 * 4];
        px[(4 + 2) * 4..(4 + 2) * 4 + 3].copy_from_slice(&[0, 0, 0]);
        let image = Rgba::new(4, 4, px).expect("image");
        let mask = segment_background(&image, 24).expect("subject");
        assert_eq!(mask.alpha_at(2, 1), Some(255));
        assert_eq!(mask.alpha_at(0, 0), Some(0));
    }
}
