//! Parity sdk-complete-39: headless tests of the render models for convolution and morphology, tone tools, the
//! spatial upscaler's colour modes, and the pixel layout conversions. Every test runs on in-memory samples; no
//! display, device or compositor is touched.

use bifrost_render::histogram::{apply_lut, curve_lut, equalization_lut, equalize, histogram, identity_lut, levels_lut};
use bifrost_render::morphology::{close, convolve, dilate, erode, open, Kernel, MorphologyError};
use bifrost_render::pixel_layout::{cmyk_to_rgb_u8, from_bytes_u8, gray_to_rgb_u8, rgb_to_gray_u8, to_bytes, Alpha, Depth, Layout};
use bifrost_render::upscale::{upscale_f32, upscale_rgb8, ColorProcessing, UpscaleError};

fn box3() -> Kernel {
    Kernel::new(3, 1, vec![1.0 / 3.0; 3]).expect("a 3x1 box is a valid kernel")
}

#[test]
fn a_kernel_must_have_odd_sides_and_one_weight_per_cell() {
    assert_eq!(Kernel::new(2, 3, vec![0.0; 6]), Err(MorphologyError::BadKernel), "even width has no centre");
    assert_eq!(Kernel::new(3, 0, Vec::new()), Err(MorphologyError::BadKernel), "zero height");
    assert_eq!(Kernel::new(3, 3, vec![0.0; 8]), Err(MorphologyError::BadKernel), "eight weights for nine cells");
    let k = Kernel::new(5, 3, vec![0.0; 15]).expect("odd sides and fifteen weights");
    assert_eq!((k.width(), k.height()), (5, 3));
}

#[test]
fn a_centre_one_kernel_returns_the_plane_unchanged() {
    let mut weights = vec![0.0f32; 9];
    weights[4] = 1.0;
    let identity = Kernel::new(3, 3, weights).unwrap();
    let plane: Vec<u8> = (0..6).map(|i| i * 40).collect();
    assert_eq!(convolve(&plane, 3, 2, &identity), Ok(plane));
}

#[test]
fn a_box_kernel_blurs_a_step_edge_and_repeats_the_edge_samples() {
    let step = [0u8, 0, 255, 255];
    assert_eq!(convolve(&step, 4, 1, &box3()), Ok(vec![0, 85, 170, 255]));
}

#[test]
fn a_flat_plane_stays_flat_under_a_blur() {
    let plane = vec![77u8; 4 * 3];
    let blur = Kernel::new(3, 3, vec![1.0 / 9.0; 9]).unwrap();
    assert_eq!(convolve(&plane, 4, 3, &blur), Ok(vec![77; 12]));
}

#[test]
fn convolution_clamps_its_output_to_the_byte_range() {
    let gain = Kernel::new(1, 1, vec![2.0]).unwrap();
    assert_eq!(convolve(&[200, 10], 2, 1, &gain), Ok(vec![255, 20]));
}

#[test]
fn convolution_refuses_a_plane_whose_length_does_not_match_its_size() {
    assert_eq!(convolve(&[1, 2, 3], 2, 2, &box3()), Err(MorphologyError::LengthMismatch));
    assert_eq!(convolve(&[], 0, 4, &box3()), Err(MorphologyError::ZeroSize));
}

#[test]
fn dilation_grows_a_bright_sample_by_the_element_radius() {
    assert_eq!(dilate(&[0, 0, 9, 0, 0], 5, 1, 1, 0), Ok(vec![0, 9, 9, 9, 0]));
}

#[test]
fn erosion_shrinks_a_bright_run_and_grows_a_dark_sample() {
    assert_eq!(erode(&[9, 9, 0, 9, 9], 5, 1, 1, 0), Ok(vec![9, 0, 0, 0, 9]));
}

#[test]
fn opening_removes_a_speck_smaller_than_the_element() {
    assert_eq!(open(&[0, 0, 9, 0, 0], 5, 1, 1, 0), Ok(vec![0; 5]));
}

#[test]
fn closing_fills_a_hole_smaller_than_the_element() {
    assert_eq!(close(&[9, 9, 0, 9, 9], 5, 1, 1, 0), Ok(vec![9; 5]));
}

#[test]
fn morphology_spreads_along_each_axis_of_the_element_separately() {
    // A single bright sample in the centre of a 3x3 plane.
    let mut plane = vec![0u8; 9];
    plane[4] = 50;
    assert_eq!(dilate(&plane, 3, 3, 1, 1), Ok(vec![50; 9]), "a 3x3 element covers the plane");
    assert_eq!(dilate(&plane, 3, 3, 1, 0), Ok(vec![0, 0, 0, 50, 50, 50, 0, 0, 0]), "a 3x1 element fills the middle row");
    assert_eq!(dilate(&plane, 3, 3, 0, 1), Ok(vec![0, 50, 0, 0, 50, 0, 0, 50, 0]), "a 1x3 element fills the middle column");
}

#[test]
fn morphology_refuses_an_empty_or_mismatched_plane() {
    assert_eq!(dilate(&[], 0, 0, 1, 1), Err(MorphologyError::ZeroSize));
    assert_eq!(erode(&[1, 2], 3, 1, 1, 1), Err(MorphologyError::LengthMismatch));
}

#[test]
fn histogram_counts_each_sample_value() {
    let h = histogram(&[0, 0, 255, 7]);
    assert_eq!((h[0], h[7], h[255]), (2, 1, 1));
    assert_eq!(h.iter().sum::<u32>(), 4);
}

#[test]
fn equalization_spreads_two_levels_across_the_full_range() {
    assert_eq!(equalize(&[10, 10, 200, 200]), vec![0, 0, 255, 255]);
    let lut = equalization_lut(&histogram(&[10, 10, 200, 200]));
    assert_eq!(lut[10], 0);
    assert_eq!(lut[200], 255);
}

#[test]
fn equalization_leaves_a_flat_or_empty_image_unchanged() {
    assert_eq!(equalize(&[128, 128, 128]), vec![128, 128, 128]);
    assert_eq!(equalization_lut(&[0u32; 256]), identity_lut());
}

#[test]
fn a_levels_stretch_maps_the_black_and_white_points_to_the_ends() {
    let lut = levels_lut(50, 200).expect("white is above black");
    assert_eq!((lut[50], lut[200], lut[0], lut[255]), (0, 255, 0, 255));
    assert_eq!(lut[125], 128, "75 * 255 / 150 is 127.5, which rounds up");
    assert!(levels_lut(9, 9).is_none());
}

#[test]
fn a_tone_curve_interpolates_and_holds_its_end_values() {
    let curve = curve_lut(&[(0, 0), (128, 64), (255, 255)]).unwrap();
    assert_eq!((curve[0], curve[64], curve[128], curve[255]), (0, 32, 64, 255));
    let held = curve_lut(&[(64, 100), (192, 200)]).unwrap();
    assert_eq!((held[0], held[128], held[255]), (100, 150, 200));
    assert!(curve_lut(&[]).is_none());
    assert!(curve_lut(&[(10, 0), (10, 5)]).is_none(), "equal inputs have no slope");
}

#[test]
fn a_lookup_table_maps_each_sample_through_its_entry() {
    let mut lut = identity_lut();
    lut[1] = 9;
    assert_eq!(apply_lut(&[0, 1, 2], &lut), vec![0, 9, 2]);
}

#[test]
fn a_flat_raster_stays_flat_in_every_colour_mode() {
    let src = vec![[0.5f32; 3]; 3 * 3];
    for mode in [ColorProcessing::Linear, ColorProcessing::Perceptual, ColorProcessing::Hdr] {
        let out = upscale_f32(&src, 3, 3, 6, 6, mode).unwrap();
        assert_eq!(out.len(), 36);
        assert!(out.iter().all(|p| p.iter().all(|v| (v - 0.5).abs() < 1e-4)), "{mode:?}");
    }
}

#[test]
fn the_linear_mode_clamps_overshoot_and_the_hdr_mode_keeps_it() {
    // A two-sample ramp from 0 to 2 upscaled to four samples: the last output overshoots to 2.140625.
    let src = [[0.0f32; 3], [2.0; 3]];
    let linear = upscale_f32(&src, 2, 1, 4, 1, ColorProcessing::Linear).unwrap();
    let hdr = upscale_f32(&src, 2, 1, 4, 1, ColorProcessing::Hdr).unwrap();
    assert_eq!(linear[3][0], 1.0, "linear output is clamped to the unit range");
    assert!((hdr[3][0] - 2.140625).abs() < 1e-5, "hdr output keeps the overshoot: {}", hdr[3][0]);
    assert!(hdr[0][0] < 0.0, "hdr output keeps undershoot below zero");
    assert_eq!(linear[0][0], 0.0);
}

#[test]
fn the_byte_upscaler_refuses_hdr_because_bytes_cannot_hold_values_above_one() {
    assert_eq!(upscale_rgb8(&[0; 3], 1, 1, 2, 2, ColorProcessing::Hdr), Err(UpscaleError::HdrNeedsFloat));
}

#[test]
fn the_upscaler_refuses_zero_sizes_and_wrong_sample_counts() {
    let src = [[0.0f32; 3]; 3];
    assert_eq!(upscale_f32(&src, 0, 3, 2, 2, ColorProcessing::Linear), Err(UpscaleError::ZeroSize));
    assert_eq!(upscale_f32(&src, 2, 2, 4, 4, ColorProcessing::Linear), Err(UpscaleError::LengthMismatch));
    assert_eq!(upscale_rgb8(&[1, 2, 3, 4, 5], 1, 1, 2, 2, ColorProcessing::Linear), Err(UpscaleError::LengthMismatch));
}

#[test]
fn cmyk_ink_corners_convert_to_the_rgb_cube() {
    assert_eq!(cmyk_to_rgb_u8(&[0, 0, 0, 0]), Some(vec![255, 255, 255]));
    assert_eq!(cmyk_to_rgb_u8(&[255, 0, 0, 0]), Some(vec![0, 255, 255]));
    assert_eq!(cmyk_to_rgb_u8(&[0, 0, 0, 255]), Some(vec![0, 0, 0]));
    assert_eq!(cmyk_to_rgb_u8(&[1, 2, 3]), None, "a partial pixel is refused");
}

#[test]
fn grey_pixels_expand_to_rgb_and_rgb_collapses_to_luma() {
    assert_eq!(gray_to_rgb_u8(&[10, 200]), vec![10, 10, 10, 200, 200, 200]);
    assert_eq!(rgb_to_gray_u8(&[255, 255, 255, 0, 0, 0]), Some(vec![255, 0]));
    assert_eq!(rgb_to_gray_u8(&[1, 2]), None);
}

#[test]
fn straight_alpha_bytes_round_trip_through_premultiplied_pixels() {
    let layout = Layout { depth: Depth::Bits8, alpha: Alpha::Straight };
    let bytes = vec![255, 0, 0, 128, 0, 0, 0, 0];
    let px = from_bytes_u8(&bytes, layout).expect("two whole pixels");
    assert_eq!(to_bytes(&px, layout), bytes);
}

#[test]
fn sixteen_bit_output_drops_alpha_and_widens_each_channel() {
    let layout = Layout { depth: Depth::Bits16, alpha: Alpha::None };
    let px = from_bytes_u8(&[1, 2, 3], Layout { depth: Depth::Bits8, alpha: Alpha::None }).unwrap();
    assert_eq!(to_bytes(&px, layout), vec![1, 1, 2, 2, 3, 3]);
}
