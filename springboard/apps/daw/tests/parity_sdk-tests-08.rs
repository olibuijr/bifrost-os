//! Parity sdk-tests-08: headless tests of the listening lab's loudness measures (integrated, maximum momentary and maximum
//! short-term loudness, true peak). Each test renders a short signal in memory and checks the measured levels. No
//! device, no clock and no randomness is used.

// The library target is named `daw`; the package is `bifrost-daw`, and the alias imports it under the package name.
extern crate daw as bifrost_daw;

use bifrost_daw::lab::{integrated_lufs, max_momentary_lufs, max_short_term_lufs, measure};
use std::f32::consts::TAU;

const SR: u32 = 48_000;

/// A sine of `amplitude` at `freq` Hz for `seconds`, on both channels.
fn tone(amplitude: f32, freq: f32, seconds: f32) -> Vec<f32> {
    let n = (seconds * SR as f32) as usize;
    (0..n).map(|i| amplitude * (TAU * freq * i as f32 / SR as f32).sin()).collect()
}

/// `seconds` of silence.
fn silence(seconds: f32) -> Vec<f32> {
    vec![0.0; (seconds * SR as f32) as usize]
}

/// A tone for `on` seconds followed by silence for `off` seconds.
fn burst(on: f32, off: f32) -> Vec<f32> {
    let mut out = tone(0.5, 1000.0, on);
    out.extend(silence(off));
    out
}

#[test]
fn silence_has_no_level_in_any_window() {
    let z = silence(2.0);
    assert!(integrated_lufs(&z, &z, 48_000.0).is_infinite());
    assert!(max_momentary_lufs(&z, &z, 48_000.0).is_infinite());
    assert!(max_short_term_lufs(&z, &z, 48_000.0).is_infinite());
}

#[test]
fn a_signal_shorter_than_a_momentary_window_has_no_momentary_level() {
    let short = tone(0.5, 1000.0, 0.3);
    assert!(max_momentary_lufs(&short, &short, 48_000.0).is_infinite());
    assert!(max_short_term_lufs(&short, &short, 48_000.0).is_infinite());
    assert!(integrated_lufs(&short, &short, 48_000.0).is_infinite());
}

#[test]
fn a_steady_tone_reads_the_same_as_its_integrated_level_in_every_window() {
    let t = tone(0.5, 1000.0, 4.0);
    let integrated = integrated_lufs(&t, &t, 48_000.0);
    let momentary = max_momentary_lufs(&t, &t, 48_000.0);
    let short_term = max_short_term_lufs(&t, &t, 48_000.0);
    assert!(momentary > -20.0 && momentary < -5.0, "{momentary}");
    assert!((momentary - integrated).abs() < 0.2, "{momentary} vs {integrated}");
    assert!((short_term - momentary).abs() < 0.2, "{short_term} vs {momentary}");
}

#[test]
fn doubling_the_amplitude_adds_six_decibels_to_every_level() {
    let quiet = tone(0.25, 1000.0, 4.0);
    let loud = tone(0.5, 1000.0, 4.0);
    let six_db = 10.0 * 4.0f64.log10();
    let diff = |f: fn(&[f32], &[f32], f32) -> f64| f(&loud, &loud, 48_000.0) - f(&quiet, &quiet, 48_000.0);
    assert!((diff(integrated_lufs) - six_db).abs() < 0.01, "integrated {}", diff(integrated_lufs));
    assert!((diff(max_momentary_lufs) - six_db).abs() < 0.01, "momentary {}", diff(max_momentary_lufs));
    assert!((diff(max_short_term_lufs) - six_db).abs() < 0.01, "short term {}", diff(max_short_term_lufs));
}

#[test]
fn a_tone_on_one_channel_only_reads_three_decibels_below_a_tone_on_both() {
    let t = tone(0.5, 1000.0, 4.0);
    let silent_right = silence(4.0);
    let three_db = 10.0 * 2.0f64.log10();
    let both = integrated_lufs(&t, &t, 48_000.0);
    let left_only = integrated_lufs(&t, &silent_right, 48_000.0);
    assert!((both - left_only - three_db).abs() < 0.01, "{both} vs {left_only}");
}

#[test]
fn a_short_burst_is_louder_in_the_momentary_window_than_in_the_short_term_window() {
    // One second of tone in six seconds: a 3 s window holds a third of the energy of a 400 ms window full of tone.
    let b = burst(1.0, 5.0);
    let momentary = max_momentary_lufs(&b, &b, 48_000.0);
    let short_term = max_short_term_lufs(&b, &b, 48_000.0);
    let third = 10.0 * 3.0f64.log10();
    let gap = momentary - short_term;
    assert!(gap > third - 0.6 && gap < third + 0.6, "gap {gap}");
}

#[test]
fn the_listening_lab_measure_fills_the_momentary_and_short_term_levels() {
    let t = tone(0.5, 1000.0, 4.0);
    let m = measure(&t, &t, SR);
    assert_eq!(m.momentary_max_lufs, max_momentary_lufs(&t, &t, 48_000.0));
    assert_eq!(m.short_term_max_lufs, max_short_term_lufs(&t, &t, 48_000.0));
    assert_eq!(m.lufs, integrated_lufs(&t, &t, 48_000.0));
    assert!((m.seconds - 4.0).abs() < 1e-9, "{}", m.seconds);
}

#[test]
fn the_true_peak_of_a_quarter_rate_sine_is_found_between_its_samples() {
    // A sine at a quarter of the sample rate and 45 degrees: the samples are 0.707 but the wave peaks at 1.0 between them.
    let x: Vec<f32> = (0..4800).map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin()).collect();
    let m = measure(&x, &x, SR);
    assert!(m.true_peak_db.abs() < 0.2, "{}", m.true_peak_db);
    assert!(m.true_peak_db > 20.0 * 0.7071f64.log10() + 1.0, "{}", m.true_peak_db);
}
