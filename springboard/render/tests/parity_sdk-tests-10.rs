//! HDR transfer curves, extended range headroom and per-frame HDR metadata.
//!
//! Headless: the curves and the frame session are plain numeric functions. No display, presenter or device is read,
//! and every frame is an in-memory slice of nits.

use bifrost_render::buffer_attach::Transfer;
use bifrost_render::hdr_frame::{code12, frame_luminance, pq_encode, signal_for_nits, FrameSession};
use bifrost_render::hdr_transfer::{hlg_decode, hlg_encode, pq_decode, pq_encode as pq_encode_curve, EdrHeadroom};

fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    assert!((actual - expected).abs() <= tolerance, "{actual} is not {expected} within {tolerance}");
}

#[test]
fn the_perceptual_quantizer_maps_black_and_the_peak_to_the_ends_of_the_code_range() {
    assert_close(pq_encode_curve(0.0), 0.0, 1e-6);
    assert_close(pq_encode_curve(10000.0), 1.0, 1e-12);
    assert_eq!(pq_decode(0.0), 0.0);
    assert_close(pq_decode(1.0), 10000.0, 1e-6);
}

#[test]
fn the_perceptual_quantizer_is_increasing_and_clamps_outside_its_range() {
    let codes: Vec<f64> = [0.1, 1.0, 100.0, 1000.0, 10000.0].iter().map(|&n| pq_encode_curve(n)).collect();
    assert!(codes.windows(2).all(|w| w[0] < w[1]), "{codes:?}");
    assert_eq!(pq_encode_curve(20000.0), pq_encode_curve(10000.0), "above the peak");
    assert_eq!(pq_encode_curve(-5.0), pq_encode_curve(0.0), "below black");
    assert_close(pq_decode(2.0), 10000.0, 1e-6);
    assert_eq!(pq_decode(-1.0), 0.0);
}

#[test]
fn a_half_code_value_is_about_ninety_two_nits_under_the_perceptual_quantizer() {
    assert_close(pq_decode(0.5), 92.2, 0.5);
}

#[test]
fn the_hybrid_log_gamma_curve_is_zero_at_black_and_one_at_reference_white() {
    assert_eq!(hlg_encode(0.0), 0.0);
    assert_eq!(hlg_encode(-1.0), 0.0, "negative light is black");
    assert_close(hlg_encode(1.0), 1.0, 1e-4);
    assert_eq!(hlg_decode(0.0), 0.0);
    assert_close(hlg_decode(1.0), 1.0, 1e-4);
}

#[test]
fn the_hybrid_log_gamma_square_root_segment_decodes_to_a_square_over_three() {
    assert_close(hlg_decode(0.25), 0.0625 / 3.0, 1e-12);
    assert_close(hlg_decode(-3.0), 0.0, 1e-12);
    assert_close(hlg_decode(3.0), hlg_decode(1.0), 1e-12);
}

#[test]
fn headroom_is_the_peak_over_the_reference_and_values_are_clamped_to_it() {
    let headroom = EdrHeadroom::new(200.0, 1000.0).unwrap();
    assert_eq!(headroom.reference_nits(), 200.0);
    assert_eq!(headroom.headroom(), 5.0);
    assert_eq!(headroom.extended_value(200.0), 1.0, "reference white");
    assert_eq!(headroom.extended_value(1000.0), 5.0, "the peak");
    assert_eq!(headroom.extended_value(5000.0), 5.0, "above the peak");
    assert_eq!(headroom.extended_value(0.0), 0.0, "black");
}

#[test]
fn a_peak_below_the_reference_leaves_no_headroom_and_bad_luminance_is_refused() {
    assert_eq!(EdrHeadroom::new(100.0, 80.0).unwrap().headroom(), 1.0);
    assert_eq!(EdrHeadroom::new(100.0, 0.0), None);
    assert_eq!(EdrHeadroom::new(f64::INFINITY, 1000.0), None);
    assert_eq!(EdrHeadroom::new(100.0, f64::INFINITY), None);
}

#[test]
fn frame_luminance_takes_the_peak_and_the_mean_with_negative_samples_as_black() {
    assert_eq!(frame_luminance(&[]), None);
    let lum = frame_luminance(&[-1.0, 4.0]).unwrap();
    assert_eq!(lum.max_nits, 4.0);
    assert_eq!(lum.average_nits, 2.0);
}

#[test]
fn the_twelve_bit_code_clamps_its_signal_and_rounds() {
    assert_eq!(code12(0.0), 0);
    assert_eq!(code12(1.0), 4095);
    assert_eq!(code12(2.0), 4095);
    assert_eq!(code12(-0.5), 0);
    assert_eq!(code12(0.5), 2048);
}

#[test]
fn only_hdr_transfers_have_a_signal_for_a_luminance() {
    assert_eq!(signal_for_nits(Transfer::Srgb, 100.0), None);
    assert_eq!(signal_for_nits(Transfer::Pq, 10000.0), Some(pq_encode(10000.0)));
    assert_eq!(signal_for_nits(Transfer::Hlg, 1000.0), Some(hlg_encode(1.0)));
}

#[test]
fn a_frame_session_needs_an_hdr_transfer_and_a_positive_frame_rate() {
    assert!(FrameSession::new(Transfer::Pq, 24.0).is_some());
    assert!(FrameSession::new(Transfer::Hlg, 60.0).is_some());
    assert_eq!(FrameSession::new(Transfer::Srgb, 60.0), None);
    assert_eq!(FrameSession::new(Transfer::Pq, 0.0), None);
    assert_eq!(FrameSession::new(Transfer::Pq, f64::NAN), None);
    assert_eq!(FrameSession::new(Transfer::Pq, 24.0).unwrap().frame_rate(), 24.0);
}

#[test]
fn a_frame_reports_its_peak_and_mean_codes_and_the_running_light_levels() {
    let mut session = FrameSession::new(Transfer::Pq, 24.0).unwrap();
    let meta = session.push_frame(&[100.0, 200.0, 0.0, -50.0], false).unwrap();
    assert_eq!(meta.transfer, Transfer::Pq);
    assert_eq!(meta.max_code, code12(pq_encode(200.0)));
    assert_eq!(meta.average_code, code12(pq_encode(75.0)));
    assert!(!meta.scene_change);
    assert_eq!(meta.max_cll, 200);
    assert_eq!(meta.max_fall, 75);
    assert_eq!(session.frames(), 1);
}

#[test]
fn the_content_light_level_keeps_the_largest_values_seen() {
    let mut session = FrameSession::new(Transfer::Pq, 24.0).unwrap();
    session.push_frame(&[10000.0], true).unwrap();
    let dim = session.push_frame(&[50.0, 60.0], false).unwrap();
    assert_eq!(dim.max_cll, 10000, "the bright frame stays the maximum");
    assert_eq!(dim.max_fall, 10000);
    session.push_frame(&[5.0], false).unwrap();
    assert_eq!(session.max_cll(), 10000);
    assert_eq!(session.max_fall(), 10000);
    assert_eq!(session.frames(), 3);
}

#[test]
fn a_full_scale_frame_takes_the_top_code_and_a_black_frame_the_bottom() {
    let mut session = FrameSession::new(Transfer::Pq, 24.0).unwrap();
    let bright = session.push_frame(&[10000.0], true).unwrap();
    assert_eq!(bright.max_code, 4095);
    assert_eq!(bright.average_code, 4095);
    assert!(bright.scene_change);
    let black = session.push_frame(&[0.0], false).unwrap();
    assert_eq!(black.max_code, 0);
}

#[test]
fn hlg_metadata_uses_the_nominal_thousand_nit_reference() {
    let mut session = FrameSession::new(Transfer::Hlg, 25.0).unwrap();
    let meta = session.push_frame(&[1000.0], false).unwrap();
    assert_eq!(meta.max_code, 4095, "1000 nits is the nominal reference of this transfer");
    assert_eq!(meta.max_cll, 1000);
}

#[test]
fn an_empty_frame_is_refused_and_leaves_the_session_unchanged() {
    let mut session = FrameSession::new(Transfer::Pq, 24.0).unwrap();
    assert_eq!(session.push_frame(&[], false), None);
    assert_eq!(session.frames(), 0);
    assert_eq!(session.max_cll(), 0);
}
