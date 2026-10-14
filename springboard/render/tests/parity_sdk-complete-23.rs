//! Parity sdk-complete-23: headless tests of the CPU image filters in `bifrost_render::filter`. Every test runs on
//! in-memory planes and checks concrete pixel values. No test touches a GPU, a display or a device.

use bifrost_render::filter::{
    arithmetic, area_filter, convolve_axis, convolve_rgb, dilate, equalize, erode, histogram, is_convolution_side,
    laplacian, median, sobel, specification_table, specify, threshold, transpose, Arithmetic, Axis, Extreme,
    HistogramInfo, Plane, RgbImage, Threshold, CONVOLUTION_SIDES,
};

fn row(values: &[u8]) -> Plane {
    Plane::new(values.len(), 1, values.to_vec()).expect("a one-row plane has one byte per value")
}

#[test]
fn headless_binary_threshold_gives_the_maximum_above_the_threshold() {
    let src = row(&[0, 100, 101, 255]);
    assert_eq!(threshold(&src, Threshold::Binary, 100, 200).pixels, vec![0, 0, 200, 200]);
    assert_eq!(threshold(&src, Threshold::BinaryInverse, 100, 200).pixels, vec![200, 200, 0, 0]);
}

#[test]
fn headless_to_zero_thresholds_keep_the_value_on_one_side_only() {
    let src = row(&[0, 100, 101, 255]);
    assert_eq!(threshold(&src, Threshold::ToZero, 100, 200).pixels, vec![0, 0, 101, 255]);
    assert_eq!(threshold(&src, Threshold::ToZeroInverse, 100, 200).pixels, vec![0, 100, 0, 0]);
}

#[test]
fn headless_truncate_threshold_clamps_at_the_threshold_and_ignores_the_maximum() {
    let src = row(&[0, 100, 101, 255]);
    assert_eq!(threshold(&src, Threshold::Truncate, 100, 7).pixels, vec![0, 100, 100, 100]);
}

#[test]
fn headless_histogram_counts_sum_to_the_pixel_count_in_each_bin() {
    let src = row(&[0, 1, 1, 255]);
    let bins = histogram(&src, HistogramInfo::new(256, 0, 255).unwrap());
    assert_eq!(bins.len(), 256);
    assert_eq!((bins[0], bins[1], bins[255]), (1, 2, 1));
    assert_eq!(bins.iter().sum::<u32>(), 4);
}

#[test]
fn headless_histogram_bins_clip_values_outside_the_range_into_the_end_bins() {
    let src = row(&[0, 150, 199, 255]);
    let info = HistogramInfo::new(4, 100, 199).unwrap();
    assert_eq!(histogram(&src, info), vec![1, 0, 1, 2]);
}

#[test]
fn headless_histogram_bins_refuse_zero_entries_and_an_empty_range() {
    assert!(HistogramInfo::new(0, 0, 255).is_none());
    assert!(HistogramInfo::new(257, 0, 255).is_none());
    assert!(HistogramInfo::new(4, 9, 9).is_none());
}

#[test]
fn headless_equalization_spreads_a_two_level_image_to_both_ends_of_the_range() {
    assert_eq!(equalize(&row(&[10, 10, 10, 200])).pixels, vec![0, 0, 0, 255]);
}

#[test]
fn headless_equalization_keeps_a_flat_plane_unchanged() {
    let src = Plane::filled(4, 2, 77);
    assert_eq!(equalize(&src), src);
}

#[test]
fn headless_specification_maps_two_source_levels_onto_the_reference_levels() {
    // Half the source is black and half is white. The reference is half 50 and half 200, so the source takes those.
    let src = row(&[0, 0, 255, 255]);
    let reference = row(&[50, 50, 200, 200]);
    assert_eq!(specify(&src, &reference).pixels, vec![50, 50, 200, 200]);
}

#[test]
fn headless_specification_against_the_same_histogram_is_the_identity() {
    let src = row(&[0, 1, 2, 3]);
    assert_eq!(specify(&src, &src).pixels, vec![0, 1, 2, 3]);
}

#[test]
fn headless_specification_with_an_empty_reference_maps_every_level_to_zero() {
    assert_eq!(specification_table(&[1, 1], &[0, 0]), vec![0, 0]);
}

#[test]
fn headless_specification_follows_the_cumulative_share_of_the_source() {
    // Half of the source sits at level 0, so it takes the reference level that first reaches half of the
    // reference (level 1). All of the source is reached by level 3.
    let source = [4u32, 0, 0, 4];
    let reference = [1u32, 1, 1, 1];
    assert_eq!(specification_table(&source, &reference), vec![1, 1, 1, 3]);
}

#[test]
fn headless_erode_takes_the_minimum_and_dilate_the_maximum_over_the_kernel() {
    let src = row(&[255, 255, 0, 255, 255]);
    assert_eq!(erode(&src, 3, 1).unwrap().pixels, vec![255, 0, 0, 0, 255]);
    let bright = row(&[0, 0, 255, 0, 0]);
    assert_eq!(dilate(&bright, 3, 1).unwrap().pixels, vec![0, 255, 255, 255, 0]);
}

#[test]
fn headless_dilation_of_one_pixel_fills_a_three_by_three_block() {
    let mut src = Plane::filled(5, 5, 0);
    src.pixels[2 * 5 + 2] = 255;
    let out = dilate(&src, 3, 3).unwrap();
    for y in 0..5 {
        for x in 0..5 {
            let inside = (1..=3).contains(&x) && (1..=3).contains(&y);
            assert_eq!(out.pixels[y * 5 + x], if inside { 255 } else { 0 }, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn headless_area_filters_refuse_even_or_zero_sides() {
    let src = Plane::filled(3, 3, 1);
    assert!(area_filter(&src, 2, 3, Extreme::Min).is_none());
    assert!(erode(&src, 3, 0).is_none());
    assert!(dilate(&src, 0, 3).is_none());
}

#[test]
fn headless_horizontal_convolution_reads_only_along_the_row() {
    // Taps [0, 0, 1] copy the pixel one step to the right, and the last column repeats the edge pixel.
    let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
    let out = convolve_axis(&src, Axis::Horizontal, &[0.0, 0.0, 1.0], 0.0).unwrap();
    assert_eq!(out.pixels, vec![2, 3, 3, 5, 6, 6]);
}

#[test]
fn headless_vertical_convolution_reads_only_down_the_column() {
    let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
    let out = convolve_axis(&src, Axis::Vertical, &[0.0, 0.0, 1.0], 0.0).unwrap();
    assert_eq!(out.pixels, vec![4, 5, 6, 4, 5, 6]);
}

#[test]
fn headless_one_dimensional_convolution_adds_the_bias_and_clamps() {
    let src = row(&[10, 200]);
    assert_eq!(convolve_axis(&src, Axis::Horizontal, &[0.0, 1.0, 0.0], 100.0).unwrap().pixels, vec![110, 255]);
    assert_eq!(convolve_axis(&src, Axis::Horizontal, &[0.0, 1.0, 0.0], -50.0).unwrap().pixels, vec![0, 150]);
}

#[test]
fn headless_convolution_accepts_every_odd_side_from_three_to_nine() {
    assert_eq!(CONVOLUTION_SIDES, 3..=9);
    for side in [3usize, 5, 7, 9] {
        assert!(is_convolution_side(side), "side {side}");
        let weights = vec![1.0f32 / side as f32; side];
        let flat = Plane::filled(12, 1, 90);
        let out = convolve_axis(&flat, Axis::Horizontal, &weights, 0.0).unwrap();
        assert_eq!(out.pixels, vec![90u8; 12], "side {side} keeps a flat plane");
    }
}

#[test]
fn headless_convolution_rejects_sides_outside_three_to_nine_and_even_sides() {
    for side in [1usize, 2, 4, 6, 8, 11] {
        assert!(!is_convolution_side(side), "side {side}");
    }
    let src = row(&[1, 2, 3]);
    assert!(convolve_axis(&src, Axis::Vertical, &[1.0; 11], 0.0).is_none());
    assert!(convolve_axis(&src, Axis::Horizontal, &[1.0], 0.0).is_none());
}

#[test]
fn headless_rgb_convolution_filters_each_channel_on_its_own() {
    // A centre weight of 2 doubles every byte, and values above 127 clamp to 255. The channels never mix.
    let src = RgbImage::new(2, 1, vec![10, 200, 30, 40, 50, 60]).unwrap();
    let weights = [0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0];
    let out = convolve_rgb(&src, 3, 3, &weights, 0.0).unwrap();
    assert_eq!(out.pixels, vec![20, 255, 60, 80, 100, 120]);
    assert_eq!((out.width, out.height), (2, 1));
}

#[test]
fn headless_rgb_convolution_refuses_a_side_outside_the_supported_range() {
    let src = RgbImage::new(2, 1, vec![0; 6]).unwrap();
    assert!(convolve_rgb(&src, 11, 11, &[0.0; 121], 0.0).is_none());
    assert!(convolve_rgb(&src, 2, 2, &[0.0; 4], 0.0).is_none());
}

#[test]
fn headless_rgb_image_needs_three_bytes_per_pixel() {
    assert!(RgbImage::new(2, 1, vec![0; 5]).is_none());
    assert_eq!(RgbImage::new(2, 1, vec![0; 6]).unwrap().pixels.len(), 6);
}

#[test]
fn headless_median_removes_a_single_bright_outlier() {
    let mut src = Plane::filled(3, 3, 10);
    src.pixels[4] = 255;
    assert_eq!(median(&src, 3).unwrap().pixels, vec![10u8; 9]);
    assert!(median(&src, 2).is_none());
}

#[test]
fn headless_laplacian_of_a_flat_plane_is_the_bias() {
    let src = Plane::filled(4, 4, 77);
    assert_eq!(laplacian(&src, 128.0).pixels, vec![128u8; 16]);
    assert_eq!(laplacian(&src, 0.0).pixels, vec![0u8; 16]);
}

#[test]
fn headless_sobel_magnitude_is_zero_on_a_flat_plane() {
    assert_eq!(sobel(&Plane::filled(3, 2, 200)).pixels, vec![0u8; 6]);
}

#[test]
fn headless_arithmetic_saturates_and_scales_as_fractions_of_255() {
    let a = row(&[200, 50]);
    let b = row(&[100, 80]);
    assert_eq!(arithmetic(&a, &b, Arithmetic::Add).unwrap().pixels, vec![255, 130]);
    assert_eq!(arithmetic(&a, &b, Arithmetic::Subtract).unwrap().pixels, vec![100, 0]);
    assert_eq!(arithmetic(&a, &b, Arithmetic::Multiply).unwrap().pixels, vec![78, 15]);
    assert_eq!(arithmetic(&a, &b, Arithmetic::Divide).unwrap().pixels, vec![255, 159]);
}

#[test]
fn headless_transpose_swaps_width_and_height_and_twice_is_the_identity() {
    let src = Plane::new(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap();
    let out = transpose(&src);
    assert_eq!((out.width, out.height), (2, 3));
    assert_eq!(out.pixels, vec![1, 4, 2, 5, 3, 6]);
    assert_eq!(transpose(&out), src);
}
