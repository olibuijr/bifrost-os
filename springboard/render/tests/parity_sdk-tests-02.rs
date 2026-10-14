//! Parity sdk-tests-02: headless tests of the HDR light model (`bifrost_render::hdr_metadata`). The checks are
//! pure arithmetic on code values and frames; no buffer is attached and no frame is composed.

use bifrost_render::buffer_attach::Transfer;
use bifrost_render::hdr_metadata::{
    code_to_nits, hlg_eotf, hlg_oetf, nits_to_pq, pq_to_nits, FrameLightSession, HdrError, CODE_MAX, HLG_NOMINAL_NITS,
    PQ_PEAK_NITS,
};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn headless_the_published_peaks_and_code_maximum_are_the_standard_values() {
    assert_eq!(PQ_PEAK_NITS, 10000.0);
    assert_eq!(HLG_NOMINAL_NITS, 1000.0);
    assert_eq!(CODE_MAX, 1023);
}

#[test]
fn headless_pq_maps_the_coded_range_onto_zero_to_ten_thousand_nits() {
    assert_eq!(pq_to_nits(0.0), 0.0);
    assert!(close(pq_to_nits(1.0), 10000.0, 1e-6));
    assert!(close(pq_to_nits(-3.0), 0.0, 1e-12), "codes below zero clamp to black");
    assert!(close(pq_to_nits(7.0), 10000.0, 1e-6), "codes above one clamp to the peak");
}

#[test]
fn headless_pq_inverse_places_a_hundred_nit_white_near_half_code() {
    let code = nits_to_pq(100.0);
    assert!(close(code, 0.508, 0.002), "100 nits is about 0.508 PQ, got {code}");
    assert!(close(nits_to_pq(10000.0), 1.0, 1e-9));
    assert!(close(nits_to_pq(50000.0), 1.0, 1e-9), "luminance above the peak clamps");
}

#[test]
fn headless_pq_round_trips_across_the_hdr_luminance_range() {
    for nits in [0.5_f64, 10.0, 100.0, 1000.0, 4000.0, 9000.0] {
        let back = pq_to_nits(nits_to_pq(nits));
        assert!(close(back, nits, nits * 1e-6), "{nits} nits came back as {back}");
    }
}

#[test]
fn headless_hlg_joins_its_branches_and_spans_zero_to_one() {
    assert_eq!(hlg_oetf(0.0), 0.0);
    assert!(close(hlg_oetf(1.0), 1.0, 1e-6), "scene white codes to full scale");
    assert!(close(hlg_eotf(1.0), 1.0, 1e-6));
    assert!(close(hlg_eotf(0.0), 0.0, 1e-12));
    for linear in [0.02_f64, 0.25, 0.6, 0.95] {
        assert!(close(hlg_eotf(hlg_oetf(linear)), linear, 1e-9), "linear {linear} did not round trip");
    }
}

#[test]
fn headless_a_code_value_converts_under_pq_and_hlg() {
    assert_eq!(code_to_nits(0, Transfer::Pq), Ok(0.0));
    assert!(close(code_to_nits(CODE_MAX, Transfer::Pq).unwrap(), 10000.0, 1e-6));
    let mid = code_to_nits(512, Transfer::Hlg).unwrap();
    let expected = hlg_eotf(512.0 / 1023.0) * HLG_NOMINAL_NITS;
    assert!(close(mid, expected, 1e-9), "HLG code 512 is {mid} nits, expected {expected}");
    assert!(close(code_to_nits(CODE_MAX, Transfer::Hlg).unwrap(), 1000.0, 1e-6));
}

#[test]
fn headless_a_code_above_the_ten_bit_maximum_is_refused() {
    assert_eq!(code_to_nits(1024, Transfer::Pq), Err(HdrError::SampleOutOfRange(1024)));
    assert_eq!(code_to_nits(1024, Transfer::Hlg), Err(HdrError::SampleOutOfRange(1024)));
}

#[test]
fn headless_an_sdr_transfer_has_no_hdr_light_model() {
    assert_eq!(code_to_nits(100, Transfer::Srgb), Err(HdrError::UnsupportedTransfer));
    assert_eq!(code_to_nits(100, Transfer::Bt709), Err(HdrError::UnsupportedTransfer));
    assert_eq!(code_to_nits(100, Transfer::Linear), Err(HdrError::UnsupportedTransfer));
}

#[test]
fn headless_a_session_refuses_a_frame_rate_that_is_not_positive_and_finite() {
    for fps in [0.0_f64, -24.0, f64::NAN, f64::INFINITY] {
        assert_eq!(FrameLightSession::new(fps, Transfer::Pq).unwrap_err(), HdrError::InvalidFrameRate, "fps {fps}");
    }
    assert_eq!(FrameLightSession::new(23.976, Transfer::Hlg).unwrap().frames_per_second(), 23.976);
}

#[test]
fn headless_a_session_refuses_a_non_hdr_transfer() {
    assert_eq!(FrameLightSession::new(25.0, Transfer::Srgb).unwrap_err(), HdrError::UnsupportedTransfer);
}

#[test]
fn headless_a_frame_reports_its_peak_and_its_plain_average() {
    let mut session = FrameLightSession::new(24.0, Transfer::Pq).unwrap();
    let light = session.attach(&[1023, 0], false).unwrap();
    assert_eq!(light.frame_peak_nits, 10000.0);
    assert!(close(light.frame_average_nits, 5000.0, 1e-6), "a plain mean of 10000 and 0");
    assert_eq!(light.scene_peak_nits, 10000.0);
}

#[test]
fn headless_frames_are_numbered_from_zero_in_session_order() {
    let mut session = FrameLightSession::new(30.0, Transfer::Hlg).unwrap();
    let indices: Vec<u64> = (0..3).map(|_| session.attach(&[256], false).unwrap().index).collect();
    assert_eq!(indices, vec![0, 1, 2]);
}

#[test]
fn headless_an_empty_or_out_of_range_frame_is_refused_and_does_not_advance_the_session() {
    let mut session = FrameLightSession::new(25.0, Transfer::Hlg).unwrap();
    assert_eq!(session.attach(&[], false), Err(HdrError::EmptyFrame));
    assert_eq!(session.attach(&[1024], false), Err(HdrError::SampleOutOfRange(1024)));
    let first = session.attach(&[100], false).unwrap();
    assert_eq!(first.index, 0, "refused frames take no index");
}

#[test]
fn headless_the_scene_peak_holds_until_a_cut_and_then_restarts() {
    let mut session = FrameLightSession::new(24.0, Transfer::Hlg).unwrap();
    let bright = session.attach(&[CODE_MAX], false).unwrap();
    assert!(close(bright.scene_peak_nits, 1000.0, 1e-6));
    let dim = session.attach(&[0], false).unwrap();
    assert_eq!(dim.frame_peak_nits, 0.0);
    assert!(close(dim.scene_peak_nits, 1000.0, 1e-6), "no cut, so the scene keeps its peak");
    let cut = session.attach(&[0], true).unwrap();
    assert_eq!(cut.scene_peak_nits, 0.0, "a cut restarts the scene peak before the frame counts");
}

#[test]
fn headless_the_scene_peak_climbs_across_frames_in_one_scene() {
    let mut session = FrameLightSession::new(24.0, Transfer::Pq).unwrap();
    let low = session.attach(&[100], false).unwrap();
    let high = session.attach(&[900], false).unwrap();
    let back = session.attach(&[200], false).unwrap();
    assert!(low.scene_peak_nits < high.scene_peak_nits);
    assert!(close(back.scene_peak_nits, high.scene_peak_nits, 1e-9));
}
