//! Parity sdk-complete-47: headless tests for the aggregate device drift model and the sample clock model. Nothing here
//! opens an audio device, reads a clock or touches the network.

use bifrost_app::audio_aggregate::{
    drift_corrections, resample_ratio, AggregateDescription, ClockAnchor, ClockDomain, SampleClock, SubDevice,
    SubDeviceClock,
};

fn sub(uid: &str, drift: bool) -> SubDevice {
    SubDevice { uid: uid.into(), drift_compensation: drift }
}

fn clock(uid: &str, domain: u32, sample_rate: f64) -> SubDeviceClock {
    SubDeviceClock { uid: uid.into(), domain: ClockDomain(domain), sample_rate }
}

/// Microphone (flagged for drift compensation) and a speaker that is the clock source, plus a second microphone that is
/// not flagged.
fn call_device() -> AggregateDescription {
    AggregateDescription {
        name: "Call".into(),
        uid: "agg-call".into(),
        sub_devices: vec![sub("mic", true), sub("speaker", false), sub("mic-2", false)],
        main_sub_device: Some("speaker".into()),
        ..Default::default()
    }
}

#[test]
fn the_resample_ratio_is_the_source_rate_over_the_sub_device_rate() {
    assert_eq!(resample_ratio(48_000.0, 44_100.0).unwrap(), 48_000.0 / 44_100.0);
    assert_eq!(resample_ratio(48_000.0, 48_000.0).unwrap(), 1.0);
}

#[test]
fn a_zero_negative_or_non_finite_rate_is_refused_by_the_resample_ratio() {
    assert!(resample_ratio(0.0, 48_000.0).is_err());
    assert!(resample_ratio(48_000.0, -44_100.0).is_err());
    assert!(resample_ratio(48_000.0, f64::NAN).is_err());
    assert!(resample_ratio(f64::INFINITY, 48_000.0).is_err());
}

#[test]
fn only_flagged_sub_devices_in_another_clock_domain_get_a_correction() {
    let clocks = [clock("mic", 3, 44_100.0), clock("speaker", 7, 48_000.0), clock("mic-2", 3, 44_100.0)];
    let corrections = drift_corrections(&call_device(), &clocks).unwrap();
    assert_eq!(corrections.len(), 1, "mic-2 is in the other domain but is not flagged");
    assert_eq!(corrections[0].uid, "mic");
    assert_eq!(corrections[0].ratio, 48_000.0 / 44_100.0);
}

#[test]
fn a_flagged_sub_device_on_the_clock_source_clock_needs_no_correction() {
    let clocks = [clock("mic", 7, 48_000.0), clock("speaker", 7, 48_000.0), clock("mic-2", 7, 48_000.0)];
    assert!(drift_corrections(&call_device(), &clocks).unwrap().is_empty());
}

#[test]
fn the_clock_source_never_gets_a_correction_even_when_flagged() {
    let mut d = call_device();
    d.sub_devices[1].drift_compensation = true;
    let clocks = [clock("mic", 3, 44_100.0), clock("speaker", 7, 48_000.0), clock("mic-2", 3, 44_100.0)];
    let corrections = drift_corrections(&d, &clocks).unwrap();
    assert!(corrections.iter().all(|c| c.uid != "speaker"));
}

#[test]
fn a_flagged_sub_device_without_a_known_clock_is_refused() {
    let clocks = [clock("speaker", 7, 48_000.0), clock("mic-2", 3, 44_100.0)];
    let err = drift_corrections(&call_device(), &clocks).unwrap_err();
    assert!(err.contains("mic"), "the message names the sub-device with no clock: {err}");
}

#[test]
fn an_invalid_description_is_refused_before_any_correction_is_computed() {
    let mut d = call_device();
    d.sub_devices.clear();
    assert!(drift_corrections(&d, &[]).is_err());
}

#[test]
fn an_anchor_maps_host_time_to_samples_at_the_sample_rate() {
    let clk = SampleClock::new(ClockAnchor { host_ticks: 0, sample: 0 }, 48_000.0).unwrap();
    assert_eq!(clk.sample_at(1_000_000_000, 1_000_000_000.0), 48_000.0);
    assert_eq!(clk.sample_at(500_000_000, 1_000_000_000.0), 24_000.0);
}

#[test]
fn host_time_before_the_anchor_maps_to_earlier_samples() {
    let clk = SampleClock::new(ClockAnchor { host_ticks: 2_000_000_000, sample: 96_000 }, 48_000.0).unwrap();
    assert_eq!(clk.sample_at(1_000_000_000, 1_000_000_000.0), 48_000.0);
}

#[test]
fn a_sample_position_maps_back_to_the_host_time_it_was_reached() {
    let clk = SampleClock::new(ClockAnchor { host_ticks: 1_000, sample: 0 }, 48_000.0).unwrap();
    assert_eq!(clk.host_at(48_000, 1_000_000_000.0), 1_000_000_000 + 1_000);
    assert_eq!(clk.host_at(0, 1_000_000_000.0), 1_000);
}

#[test]
fn a_sample_clock_refuses_a_non_positive_rate() {
    assert!(SampleClock::new(ClockAnchor { host_ticks: 0, sample: 0 }, 0.0).is_err());
    assert!(SampleClock::new(ClockAnchor { host_ticks: 0, sample: 0 }, f64::NAN).is_err());
}

#[test]
fn a_clock_running_a_tenth_of_a_percent_fast_drifts_48_samples_per_second() {
    let source = SampleClock::new(ClockAnchor { host_ticks: 0, sample: 0 }, 48_000.0).unwrap();
    let fast = SampleClock::new(ClockAnchor { host_ticks: 0, sample: 0 }, 48_048.0).unwrap();
    let skew = source.skew(&fast, 1_000_000_000, 1_000_000_000.0);
    assert!((skew - 48.0).abs() < 1e-6, "skew was {skew}");
    assert_eq!(source.skew(&source, 1_000_000_000, 1_000_000_000.0), 0.0);
}
