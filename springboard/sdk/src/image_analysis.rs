//! Image analysis for apps: subject lifting and Visual Look Up (the subjects and highlighted subjects of an image
//! analysis interaction, the `analysisTypes` option set, and the Look Up request for a subject).
//!
//! The models live in `bifrost_ui` so the shell and apps share one API. A segmenter gives one alpha [`Mask`] per
//! subject; [`Interaction`] holds the subjects, hit tests them, keeps the highlight, and lifts a [`Cutout`]. [`look_up`]
//! builds the [`LookUpRequest`] for the system lookup service. Limits: no segmentation model and no lookup service
//! here, so the apps get the shape of the API and the data, not the recognition itself.

pub use bifrost_ui::subject_lift::{Cutout, Interaction, Mask, PixelRect, Rgba, Subject, SUBJECT_THRESHOLD};
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
}
