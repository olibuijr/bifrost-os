//! Parity sdk-complete-32: headless tests for loudness normalization (`springboard::media::volume`). Pure arithmetic:
//! no audio device, no clock and no file is read.

use springboard::media::volume::{
    normalization_gain_db, normalized_linear, Volume, NORMALIZATION_MAX_BOOST_DB, NORMALIZATION_TARGET_DB,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_track_at_the_target_needs_no_gain() {
    assert!(close(normalization_gain_db(-16.0, -16.0, 6.0), 0.0));
}

#[test]
fn a_loud_track_is_turned_down_by_the_full_difference() {
    assert!(close(normalization_gain_db(-10.0, -16.0, 6.0), -6.0));
    assert!(close(normalization_gain_db(-4.0, -16.0, 6.0), -12.0), "the cut is not limited");
}

#[test]
fn a_quiet_track_is_raised_but_never_by_more_than_the_boost_limit() {
    assert!(close(normalization_gain_db(-19.0, -16.0, 6.0), 3.0));
    assert!(close(normalization_gain_db(-30.0, -16.0, 6.0), 6.0), "14 dB needed, 6 dB allowed");
}

#[test]
fn a_silent_or_unmeasured_track_is_left_alone() {
    assert!(close(normalization_gain_db(f32::NEG_INFINITY, -16.0, 6.0), 0.0));
    assert!(close(normalization_gain_db(f32::NAN, -16.0, 6.0), 0.0));
}

#[test]
fn a_negative_boost_limit_is_read_as_no_boost_and_still_allows_cuts() {
    assert!(close(normalization_gain_db(-30.0, -16.0, -3.0), 0.0), "a quiet track gets no boost");
    assert!(close(normalization_gain_db(-10.0, -16.0, -3.0), -6.0));
}

#[test]
fn the_published_defaults_aim_at_minus_sixteen_with_six_db_of_boost() {
    assert!(close(NORMALIZATION_TARGET_DB, -16.0));
    assert!(close(NORMALIZATION_MAX_BOOST_DB, 6.0));
}

#[test]
fn a_gain_of_minus_six_db_halves_the_full_volume_multiplier() {
    let full = Volume::new(1.0);
    assert!(close(normalized_linear(full, 0.0), 1.0));
    assert!(close(normalized_linear(full, -6.0), 0.501_187), "about half amplitude");
}

#[test]
fn muted_stays_silent_whatever_the_normalization_gain() {
    assert_eq!(normalized_linear(Volume::new(0.0), 6.0), 0.0);
}

#[test]
fn the_volume_and_the_normalization_multiply() {
    let half = Volume::at_step(8);
    let combined = normalized_linear(half, -6.0);
    assert!(close(combined, half.linear() * 0.501_187));
}
