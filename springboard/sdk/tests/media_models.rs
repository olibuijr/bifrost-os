//! The media models re-exported by the app SDK (`bifrost_app::{media_feedback, ...}`) answer as pure state.

use bifrost_app::media_feedback::{FeedbackKind, FeedbackState, RatingCommand};

#[test]
fn feedback_state_starts_off_and_toggles_each_command_alone() {
    let mut state = FeedbackState::new();
    assert!(state.active().is_empty(), "every feedback command starts off");

    let first = FeedbackKind::ALL[0];
    let second = FeedbackKind::ALL[1];
    assert!(state.toggle(first), "toggle turns a command on");
    assert!(state.is_active(first));
    assert!(!state.is_active(second), "toggling one command leaves the others alone");

    assert!(!state.toggle(first), "a second toggle turns it off");
    assert!(state.active().is_empty());
}

#[test]
fn rating_command_refuses_an_empty_or_non_finite_scale() {
    assert!(RatingCommand::new(2.0, 2.0).is_none());
    assert!(RatingCommand::new(f32::NAN, 1.0).is_none());
    let rating = RatingCommand::new(0.0, 5.0).expect("a valid scale");
    assert_eq!(rating.minimum(), 0.0);
    assert_eq!(rating.maximum(), 5.0);
}
