//! Parity sdk-tests-01: headless tests of the RAW demosaic model (`bifrost_render::bayer`) and of the depth-of-field
//! blur (`bifrost_render::depth_blur`, over `bifrost_render::blur_filters`). Every frame is processed in memory: no
//! sensor, no display and no device is opened.

use bifrost_render::bayer::{demosaic, BayerError, CfaPattern};
use bifrost_render::blur_filters::{masked_variable_blur, Edge};
use bifrost_render::depth_blur::{blur_weights, depth_blur, depth_to_disparity, disparity_to_depth};
use bifrost_render::gfx::Image;

fn row(values: &[u32]) -> Image {
    Image { w: values.len(), h: 1, px: values.to_vec() }
}

#[test]
fn headless_bggr_tile_puts_blue_on_the_top_left_photosite() {
    let p = CfaPattern::Bggr;
    assert_eq!(p.channel_at(0, 0), 2);
    assert_eq!(p.channel_at(1, 0), 1);
    assert_eq!(p.channel_at(0, 1), 1);
    assert_eq!(p.channel_at(1, 1), 0);
}

#[test]
fn headless_gbrg_tile_places_green_on_the_top_left_photosite() {
    let p = CfaPattern::Gbrg;
    assert_eq!(p.channel_at(0, 0), 1);
    assert_eq!(p.channel_at(1, 0), 2);
    assert_eq!(p.channel_at(0, 1), 0);
    assert_eq!(p.channel_at(1, 1), 1);
}

#[test]
fn headless_bggr_demosaic_keeps_each_own_sample_and_fills_the_rest() {
    // B=100 at (0,0), G=40 at (1,0), G=60 at (0,1), R=20 at (1,1). Both missing colours of each site
    // come from the whole 2x2 frame: red is 20 everywhere, green is 50, blue is 100.
    let mosaic = [100u16, 40, 60, 20];
    let rgb = demosaic(&mosaic, 2, 2, CfaPattern::Bggr, [1.0, 1.0, 1.0]).unwrap();
    assert_eq!(&rgb[0..3], &[20, 50, 100], "blue site");
    assert_eq!(&rgb[9..12], &[20, 50, 100], "red site keeps its own red and borrows blue");
}

#[test]
fn headless_demosaic_cuts_the_neighbourhood_at_the_frame_edge() {
    // One row of three red-green photosites: R=10, G=20, R=30. The first red site has one green neighbour
    // and no blue, so blue stays zero.
    let rgb = demosaic(&[10u16, 20, 30], 3, 1, CfaPattern::Rggb, [1.0, 1.0, 1.0]).unwrap();
    assert_eq!(&rgb[0..3], &[10, 20, 0]);
    assert_eq!(&rgb[3..6], &[20, 20, 0], "the green site averages the two red neighbours");
}

#[test]
fn headless_zero_gains_render_a_black_frame() {
    let rgb = demosaic(&[100u16, 40, 60, 20], 2, 2, CfaPattern::Rggb, [0.0, 0.0, 0.0]).unwrap();
    assert_eq!(rgb.len(), 12);
    assert!(rgb.iter().all(|&v| v == 0));
}

#[test]
fn headless_a_zero_sized_or_mismatched_frame_is_refused_with_its_reason() {
    assert_eq!(demosaic(&[1u16, 2], 2, 0, CfaPattern::Gbrg, [1.0; 3]).unwrap_err(), BayerError::EmptyFrame);
    assert_eq!(
        demosaic(&[1u16, 2, 3], 1, 2, CfaPattern::Gbrg, [1.0; 3]).unwrap_err(),
        BayerError::SizeMismatch { expected: 2, actual: 3 }
    );
}

#[test]
fn headless_disparity_is_the_inverse_of_depth_in_metres() {
    assert!((depth_to_disparity(4.0) - 0.25).abs() < 1e-6);
    assert!((disparity_to_depth(0.25) - 4.0).abs() < 1e-6);
    assert_eq!(depth_to_disparity(-2.0), 0.0, "a negative depth is no depth");
    assert!(disparity_to_depth(f32::NAN).is_infinite());
}

#[test]
fn headless_depth_weights_grow_with_distance_from_the_focus_depth() {
    // focus 1 m, falloff 4 m: 2 m is a quarter of the way to full blur, 0 m is as far as 2 m.
    assert_eq!(blur_weights(&[1.0, 2.0, 3.0, 5.0], 1.0, 4.0).unwrap(), vec![0, 64, 128, 255]);
    let near = blur_weights(&[0.0], 1.0, 2.0).unwrap();
    let far = blur_weights(&[2.0], 1.0, 2.0).unwrap();
    assert_eq!(near, far, "the blur is symmetric about the focus depth");
}

#[test]
fn headless_an_infinite_depth_takes_the_full_blur() {
    assert_eq!(blur_weights(&[f32::INFINITY, 1.0], 1.0, 2.0).unwrap(), vec![255, 0]);
}

#[test]
fn headless_in_focus_pixels_keep_their_own_value_while_the_far_ones_spread() {
    // Pixels 0..=2 sit at the focus depth, pixels 3 and 4 are 5 m away. The impulse at pixel 2 is sharp,
    // so it keeps its value; pixel 3 picks up light from it.
    let img = row(&[0, 0, 0xffff_ffff, 0, 0]);
    let depth = [1.0, 1.0, 1.0, 5.0, 5.0];
    let out = depth_blur(&img, &depth, 1.0, 1.0, 4.0).unwrap();
    assert_eq!(out.px[2], 0xffff_ffff);
    assert_eq!(out.px[0], 0);
    assert!(out.px[3] & 0xff > 0, "the far neighbour is blurred into the impulse");
    assert!(out.px[3] & 0xff > out.px[4] & 0xff, "the light falls off with distance from the impulse");
}

#[test]
fn headless_depth_blur_refuses_an_invalid_falloff() {
    let img = row(&[0, 0xffff_ffff]);
    assert!(depth_blur(&img, &[1.0, 1.0], 1.0, 0.0, 2.0).is_none());
    assert!(depth_blur(&img, &[1.0, 1.0], 1.0, f32::NAN, 2.0).is_none());
}

#[test]
fn headless_a_zero_mask_leaves_the_image_unchanged() {
    // opaque pixels: packing clamps each colour to the alpha, so only alpha 255 round-trips exactly
    let img = row(&[0xff10_2030, 0xff40_5060, 0xff00_00ff]);
    let mask = row(&[0, 0, 0]);
    assert_eq!(masked_variable_blur(&img, &mask, 6.0, Edge::Clamp).px, img.px);
}

#[test]
fn headless_a_mask_of_another_size_returns_the_image() {
    let img = row(&[0, 0xffff_ffff, 0]);
    let mask = row(&[0xff00_0000, 0xff00_0000]);
    assert_eq!(masked_variable_blur(&img, &mask, 3.0, Edge::Transparent).px, img.px);
}
