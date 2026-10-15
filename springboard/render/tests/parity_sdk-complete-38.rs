//! Parity sdk-complete-38: headless tests for the temporal denoised upscaler and the plane pixel conversions.
//! Every test runs on CPU rasters in memory. No test composes a frame on a display.

use bifrost_render::filter::Plane;
use bifrost_render::plane_arith::{convert_transfer, invert, Transfer};
use bifrost_render::temporal_upscale::{box_denoise, Accumulation, TemporalScaler};
use bifrost_render::upscale::{upscale_f32, ColorProcessing};

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn box_denoise_spreads_a_bright_sample_over_its_row() {
    // A one-row raster: each output is the mean of its 3x3 window, and the single row repeats three times.
    let row: Vec<[f32; 3]> = [0.0, 0.0, 9.0, 0.0, 0.0].iter().map(|&v| [v; 3]).collect();
    let out = box_denoise(&row, 5, 1);
    let values: Vec<f32> = out.iter().map(|p| p[0]).collect();
    assert_eq!(values, vec![0.0, 3.0, 3.0, 3.0, 0.0]);
}

#[test]
fn box_denoise_leaves_a_flat_raster_unchanged() {
    let flat = vec![[0.25f32; 3]; 6];
    let out = box_denoise(&flat, 3, 2);
    assert_eq!(out.len(), 6);
    assert!(out.iter().all(|p| p.iter().all(|&c| near(c, 0.25))));
}

#[test]
fn denoised_mode_upscales_the_denoised_input_on_its_first_frame() {
    let color = vec![[1.0f32; 3], [0.0; 3], [0.0; 3], [0.0; 3]];
    let depth = vec![0.3f32; 4];
    let motion = vec![[0.0f32, 0.0]; 4];
    let mut scaler = TemporalScaler::new(2, 2, 4, 4, Accumulation::Denoised).unwrap();
    let out = scaler.resolve(&color, &depth, &motion).unwrap();
    let expected = upscale_f32(&box_denoise(&color, 2, 2), 2, 2, 4, 4, ColorProcessing::Linear).unwrap();
    assert_eq!(out, expected);
}

#[test]
fn standard_mode_upscales_the_raw_input_on_its_first_frame() {
    let color = vec![[1.0f32; 3], [0.0; 3], [0.0; 3], [0.0; 3]];
    let depth = vec![0.3f32; 4];
    let motion = vec![[0.0f32, 0.0]; 4];
    let mut scaler = TemporalScaler::new(2, 2, 4, 4, Accumulation::Standard).unwrap();
    let out = scaler.resolve(&color, &depth, &motion).unwrap();
    let raw = upscale_f32(&color, 2, 2, 4, 4, ColorProcessing::Linear).unwrap();
    let denoised = upscale_f32(&box_denoise(&color, 2, 2), 2, 2, 4, 4, ColorProcessing::Linear).unwrap();
    assert_eq!(out, raw);
    assert_ne!(out, denoised, "only denoised accumulation filters the input colour");
}

#[test]
fn only_denoised_accumulation_denoises_the_input() {
    assert!(Accumulation::Denoised.denoises());
    assert!(!Accumulation::Standard.denoises());
}

#[test]
fn inverting_a_plane_gives_its_negative() {
    let src = Plane::new(4, 1, vec![0, 1, 128, 255]).unwrap();
    let out = invert(&src);
    assert_eq!((out.width, out.height), (4, 1));
    assert_eq!(out.pixels, vec![255, 254, 127, 0]);
}

#[test]
fn linear_codes_encode_to_srgb_codes_at_the_ends_and_the_middle() {
    let src = Plane::new(3, 1, vec![0, 128, 255]).unwrap();
    let out = convert_transfer(&src, Transfer::Linear, Transfer::Srgb);
    assert_eq!(out.pixels[0], 0);
    assert_eq!(out.pixels[2], 255);
    // Linear 0.5 is about 0.735 in sRGB, which is code 188 (187.8 rounded).
    assert!((187..=189).contains(&out.pixels[1]), "got {}", out.pixels[1]);
}

#[test]
fn srgb_codes_decode_to_linear_light_at_the_ends_and_the_middle() {
    let src = Plane::new(3, 1, vec![0, 128, 255]).unwrap();
    let out = convert_transfer(&src, Transfer::Srgb, Transfer::Linear);
    assert_eq!(out.pixels[0], 0);
    assert_eq!(out.pixels[2], 255);
    // sRGB 128 is about 0.216 in linear light, which is code 55.
    assert!((54..=56).contains(&out.pixels[1]), "got {}", out.pixels[1]);
}

#[test]
fn a_linear_to_srgb_to_linear_round_trip_stays_within_two_codes() {
    let codes: Vec<u8> = (0..=255).collect();
    let src = Plane::new(256, 1, codes.clone()).unwrap();
    let encoded = convert_transfer(&src, Transfer::Linear, Transfer::Srgb);
    let back = convert_transfer(&encoded, Transfer::Srgb, Transfer::Linear);
    for (original, round) in codes.iter().zip(&back.pixels) {
        let diff = (*original as i32 - *round as i32).abs();
        assert!(diff <= 2, "code {original} came back as {round}");
    }
}

#[test]
fn conversion_keeps_the_plane_size() {
    let src = Plane::filled(3, 2, 77);
    let out = convert_transfer(&src, Transfer::Srgb, Transfer::Linear);
    assert_eq!((out.width, out.height), (3, 2));
    assert_eq!(out.pixels.len(), 6);
}
