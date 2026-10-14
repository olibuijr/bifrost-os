//! Parity sdk-complete-40: headless tests for the flashing light model (frame luminance, the change report) and
//! the input source model. No test reads a display or a device.

use bifrost_ui::flashing_lights::{frame_luminance, pixel_luminance, FlashingLights, Sample};

fn s(time_ms: u64, luminance: f64) -> Sample {
    Sample { time_ms, luminance }
}

fn square_wave() -> Vec<Sample> {
    (0..=10).map(|i| s(i * 100, if i % 2 == 0 { 0.0 } else { 1.0 })).collect()
}

#[test]
fn white_and_black_pixels_have_luminance_one_and_zero() {
    assert!((pixel_luminance(0xFFFF_FFFF) - 1.0).abs() < 1e-9);
    assert_eq!(pixel_luminance(0xFF00_0000), 0.0);
}

#[test]
fn the_alpha_byte_does_not_change_the_luminance() {
    assert_eq!(pixel_luminance(0x0012_3456), pixel_luminance(0xFF12_3456));
}

#[test]
fn a_mid_grey_pixel_reads_the_linear_value_of_half_the_channel() {
    // sRGB 0x80 is about 0.2159 linear, and the three channels weigh to the same value.
    assert!((pixel_luminance(0xFF80_8080) - 0.2158605).abs() < 1e-4);
}

#[test]
fn pure_green_is_the_brightest_primary_because_the_eye_weighs_it_most() {
    let red = pixel_luminance(0xFFFF_0000);
    let green = pixel_luminance(0xFF00_FF00);
    let blue = pixel_luminance(0xFF00_00FF);
    assert!((red - 0.2126).abs() < 1e-4);
    assert!((green - 0.7152).abs() < 1e-4);
    assert!((blue - 0.0722).abs() < 1e-4);
}

#[test]
fn a_frame_luminance_is_the_mean_of_its_pixels() {
    let mean = frame_luminance(&[0xFF00_0000, 0xFFFF_FFFF]).expect("a frame with pixels");
    assert!((mean - 0.5).abs() < 1e-9);
}

#[test]
fn an_empty_frame_has_no_luminance() {
    assert_eq!(frame_luminance(&[]), None);
}

#[test]
fn a_flash_sequence_is_reported_once_when_it_becomes_flagged() {
    let mut lights = FlashingLights::new();
    assert_eq!(lights.analyse_change(&square_wave()), Some(true));
    assert_eq!(lights.analyse_change(&square_wave()), None, "the same state is not reported again");
    assert!(lights.is_flagged());
}

#[test]
fn a_calm_sequence_after_a_flagged_one_reports_the_clearing() {
    let mut lights = FlashingLights::new();
    lights.analyse_change(&square_wave());
    let calm = [s(0, 0.2), s(100, 0.25)];
    assert_eq!(lights.analyse_change(&calm), Some(false));
    assert!(!lights.is_flagged());
}

#[test]
fn a_calm_picture_never_reports_a_change() {
    let mut lights = FlashingLights::new();
    assert_eq!(lights.analyse_change(&[s(0, 0.5), s(100, 0.5)]), None);
}
