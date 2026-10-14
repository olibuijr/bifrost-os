//! Parity sdk-rest-10: headless tests for the typed media analysis errors. Every check runs on plain values in memory.

use bifrost_ui::face_group::FaceGroups;
use bifrost_ui::media_intelligence_error::{check_embedding, check_threshold, MediaIntelligenceError};
use bifrost_ui::ml_runtime::RuntimeError;

/// Face 1 [1,0] and face 2 [0,1] on asset 1. Face 3 [1,0] and face 4 [1,1] on asset 2.
fn sample() -> FaceGroups {
    let mut g = FaceGroups::new();
    g.try_insert_face(1, vec![1.0, 0.0]).unwrap();
    g.try_insert_face(1, vec![0.0, 1.0]).unwrap();
    g.try_insert_face(2, vec![1.0, 0.0]).unwrap();
    g.try_insert_face(2, vec![1.0, 1.0]).unwrap();
    g
}

#[test]
fn an_empty_face_embedding_is_refused_as_empty() {
    let mut g = FaceGroups::new();
    assert_eq!(g.try_insert_face(1, Vec::new()), Err(MediaIntelligenceError::EmptyEmbedding));
    assert!(g.faces_in_asset(1).is_empty(), "a refused face is not stored");
}

#[test]
fn a_face_embedding_with_a_non_finite_value_is_refused_at_that_index() {
    let mut g = FaceGroups::new();
    assert_eq!(g.try_insert_face(1, vec![1.0, f64::NAN]), Err(MediaIntelligenceError::NonFiniteValue { index: 1 }));
    assert_eq!(g.try_insert_face(1, vec![f64::INFINITY, 0.0]), Err(MediaIntelligenceError::NonFiniteValue { index: 0 }));
}

#[test]
fn a_face_embedding_of_another_length_than_the_stored_faces_is_refused_with_both_lengths() {
    let mut g = FaceGroups::new();
    g.try_insert_face(1, vec![1.0, 0.0]).unwrap();
    assert_eq!(
        g.try_insert_face(2, vec![1.0, 0.0, 0.0]),
        Err(MediaIntelligenceError::DimensionMismatch { expected: 2, found: 3 })
    );
}

#[test]
fn a_valid_face_is_stored_and_takes_the_next_id() {
    let mut g = FaceGroups::new();
    assert_eq!(g.try_insert_face(1, vec![1.0, 0.0]), Ok(1));
    assert_eq!(g.try_insert_face(1, vec![0.0, 1.0]), Ok(2));
    assert_eq!(g.faces_in_asset(1), vec![1, 2]);
}

#[test]
fn a_threshold_outside_the_cosine_range_is_refused_with_its_value() {
    assert_eq!(sample().try_entities(1.5), Err(MediaIntelligenceError::InvalidThreshold(1.5)));
    assert_eq!(sample().try_entities(-1.5), Err(MediaIntelligenceError::InvalidThreshold(-1.5)));
}

#[test]
fn a_nan_threshold_is_refused() {
    assert!(matches!(check_threshold(f64::NAN), Err(MediaIntelligenceError::InvalidThreshold(t)) if t.is_nan()));
}

#[test]
fn the_cosine_range_bounds_are_valid_thresholds() {
    assert_eq!(check_threshold(-1.0), Ok(()));
    assert_eq!(check_threshold(1.0), Ok(()));
}

#[test]
fn a_valid_threshold_groups_the_faces_as_entities_does() {
    assert_eq!(sample().try_entities(0.9), Ok(vec![vec![1, 3], vec![2], vec![4]]));
    assert_eq!(sample().try_entities(0.9), Ok(sample().entities(0.9)));
}

#[test]
fn an_empty_embedding_check_with_no_stored_length_only_needs_values() {
    assert_eq!(check_embedding(&[], None), Err(MediaIntelligenceError::EmptyEmbedding));
    assert_eq!(check_embedding(&[0.5], None), Ok(()));
}

#[test]
fn a_runtime_failure_is_carried_into_the_media_error() {
    let err = MediaIntelligenceError::from(RuntimeError::UnknownModel("face-embedder".into()));
    assert_eq!(err, MediaIntelligenceError::Runtime(RuntimeError::UnknownModel("face-embedder".into())));
}

#[test]
fn each_error_says_what_was_wrong_in_plain_words() {
    assert_eq!(MediaIntelligenceError::EmptyEmbedding.to_string(), "the face embedding has no values");
    assert_eq!(
        MediaIntelligenceError::DimensionMismatch { expected: 2, found: 3 }.to_string(),
        "the face embedding has 3 values but the stored faces have 2"
    );
    assert_eq!(
        MediaIntelligenceError::NonFiniteValue { index: 4 }.to_string(),
        "the face embedding value 4 is not a finite number"
    );
}
