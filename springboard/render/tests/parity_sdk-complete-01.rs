//! Parity sdk-complete-01: headless tests for the core image models. Every test works on packed pixels or colour
//! values in memory: no display, no device, no network and no clock. Each assertion checks a concrete value.
use bifrost_render::affine::{self, Affine};
use bifrost_render::colorcube::ColorCube;
use bifrost_render::colorfx::Rgb;
use bifrost_render::colorlab::{delta_e, Lab};
use bifrost_render::colormap::{ColorMap, Palette};
use bifrost_render::comic_filter;
use bifrost_render::distortion_filters;
use bifrost_render::generators;
use bifrost_render::gfx::Image;
use bifrost_render::gradient::Ramp;
use bifrost_render::spot_light;
use bifrost_render::stylize_filters;

const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };
const WHITE: Rgb = Rgb { r: 1.0, g: 1.0, b: 1.0 };
const RED: Rgb = Rgb { r: 1.0, g: 0.0, b: 0.0 };
const GREEN: Rgb = Rgb { r: 0.0, g: 1.0, b: 0.0 };

fn close(a: Rgb, b: Rgb, tolerance: f32) -> bool {
    (a.r - b.r).abs() <= tolerance && (a.g - b.g).abs() <= tolerance && (a.b - b.b).abs() <= tolerance
}

/// A 9 by 9 image whose pixel `k` holds the value `k`, so every source position is identifiable.
fn numbered_9x9() -> Image {
    let mut i = Image::new(9, 9);
    for (k, p) in i.px.iter_mut().enumerate() {
        *p = 0xff00_0000 | k as u32;
    }
    i
}

// Affine transform and crop (bifrost_render::affine).

#[test]
fn affine_translation_then_scale_applies_the_first_transform_first() {
    let t = Affine::translation(10.0, 0.0).then(Affine::scale(2.0, 2.0));
    assert_eq!(t.apply(1.0, 1.0), (22.0, 2.0));
}

#[test]
fn affine_inverse_of_a_singular_matrix_is_none() {
    assert_eq!(Affine::scale(0.0, 3.0).inverse(), None);
    assert!(affine::transform(&numbered_9x9(), Affine::scale(0.0, 1.0)).is_none());
}

#[test]
fn affine_translation_by_one_pixel_moves_the_content_right() {
    let img = numbered_9x9();
    let out = affine::transform(&img, Affine::translation(1.0, 0.0)).unwrap();
    assert_eq!(out.px[4 * 9 + 1], img.px[4 * 9]);
    assert_eq!(out.px[4 * 9], 0, "the uncovered left column is transparent");
}

#[test]
fn affine_crop_pads_the_pixels_outside_the_source_with_transparency() {
    let img = numbered_9x9();
    let out = affine::crop(&img, 7, 7, 3, 3);
    assert_eq!((out.w, out.h), (3, 3));
    assert_eq!(out.px[0], img.px[7 * 9 + 7]);
    assert_eq!(out.px[2], 0);
    assert_eq!(out.px[8], 0);
}

// Generators and gradients (bifrost_render::generators, bifrost_render::gradient).

#[test]
fn generator_checkerboard_has_cells_of_the_given_size() {
    let img = generators::checkerboard(6, 2, 3, 1, 2).unwrap();
    assert_eq!(img.px, vec![1, 1, 1, 2, 2, 2, 1, 1, 1, 2, 2, 2]);
    assert!(generators::checkerboard(6, 2, 0, 1, 2).is_none());
}

#[test]
fn generator_stripes_reject_a_non_positive_width() {
    assert!(generators::stripes(4, 4, -1.0, 0.0, 1, 2).is_none());
    assert_eq!(generators::stripes(4, 1, 1.0, 0.0, 5, 6).unwrap().px, vec![5, 6, 5, 6]);
}

#[test]
fn generator_linear_gradient_is_opaque_and_increases_across_the_image() {
    let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
    let img = generators::linear_gradient(8, 1, (0.0, 0.0), (8.0, 0.0), &ramp);
    assert!(img.px.iter().all(|p| p >> 24 == 0xff));
    assert!((img.px[0] & 255) < (img.px[7] & 255));
}

#[test]
fn generator_radial_gradient_is_the_start_colour_at_the_centre() {
    let ramp = Ramp::new(&[(0.0, RED), (1.0, GREEN)]).unwrap();
    let img = generators::radial_gradient(5, 5, (2.5, 2.5), 2.0, &ramp);
    assert_eq!(img.px[2 * 5 + 2], 0xff00_00ff, "red is the low byte of a premultiplied pixel");
    assert_eq!(img.px[0], 0xff00_ff00, "a corner beyond the radius takes the end colour");
}

// Distortion (bifrost_render::distortion_filters): circle splash and torus lens.

#[test]
fn circle_splash_magnifies_the_centre_and_leaves_the_rim_alone() {
    let img = numbered_9x9();
    let out = distortion_filters::circle_splash(&img, (4.5, 4.5), 4.0);
    assert_eq!(out.px[4 * 9 + 6], img.px[4 * 9 + 5], "a pixel at distance 2 samples distance 1");
    assert_eq!(out.px[4 * 9], img.px[4 * 9], "a pixel beyond the radius is untouched");
}

#[test]
fn circle_splash_with_no_radius_is_the_identity() {
    let img = numbered_9x9();
    assert_eq!(distortion_filters::circle_splash(&img, (4.5, 4.5), 0.0).px, img.px);
}

#[test]
fn torus_lens_with_no_refraction_is_the_identity() {
    let img = numbered_9x9();
    assert_eq!(distortion_filters::torus_lens(&img, (4.5, 4.5), 4.0, 0.0).px, img.px);
    assert_eq!(distortion_filters::torus_lens(&img, (4.5, 4.5), 4.0, f32::NAN).px, img.px);
}

#[test]
fn torus_lens_pulls_the_picture_inwards_by_the_refraction() {
    let img = numbered_9x9();
    let out = distortion_filters::torus_lens(&img, (4.5, 4.5), 4.0, 1.0);
    assert_eq!(out.px[4 * 9 + 6], img.px[4 * 9 + 7], "distance 2 samples distance 3, which is pixel 7");
    assert_eq!(out.px[4 * 9], img.px[4 * 9], "the rim at and beyond the radius stays fixed");
}

// Spot light (bifrost_render::spot_light).

#[test]
fn spot_light_adds_light_under_the_lamp_and_none_when_switched_off() {
    let grey = Image { w: 5, h: 5, px: vec![0xff40_4040; 25] };
    let lit = spot_light::spot_light(&grey, (2.5, 2.5, 10.0), (2.5, 2.5, 0.0), 0.5, 1.0, WHITE);
    assert_eq!(lit.px[2 * 5 + 2] & 255, 192);
    let off = spot_light::spot_light(&grey, (2.5, 2.5, 10.0), (2.5, 2.5, 0.0), 0.0, 1.0, WHITE);
    assert_eq!(off.px, grey.px);
}

// Comic effect (bifrost_render::comic_filter).

#[test]
fn comic_outlines_a_step_in_black_and_keeps_flat_areas() {
    let mut img = Image::new(4, 1);
    for x in 0..4 {
        img.px[x] = if x < 2 { 0xff00_0000 } else { 0xffff_ffff };
    }
    let out = comic_filter::comic(&img, comic_filter::DEFAULT_LEVELS, comic_filter::DEFAULT_THRESHOLD);
    assert_eq!(out.px[1] & 0x00ff_ffff, 0);
    assert_eq!(comic_filter::comic(&img, 2, 1.0).px[2] & 0x00ff_ffff, 0x00ff_ffff, "at threshold one no edge passes, so the bright side stays white");
}

// Colour cube (bifrost_render::colorcube).

#[test]
fn colour_cube_identity_returns_the_input_colour() {
    let cube = ColorCube::identity(3).unwrap();
    let c = Rgb { r: 0.5, g: 0.25, b: 1.0 };
    assert!(close(cube.lookup(c), c, 1e-5));
    assert_eq!(cube.size(), 3);
}

#[test]
fn colour_cube_rejects_a_size_or_entry_count_it_cannot_hold() {
    assert!(ColorCube::identity(1).is_none());
    assert!(ColorCube::new(2, vec![BLACK; 7]).is_none());
    assert!(ColorCube::new(2, vec![BLACK; 8]).is_some());
}

#[test]
fn colour_cube_inverts_a_table_that_negates_each_channel() {
    let data: Vec<Rgb> = (0..8)
        .map(|k| Rgb { r: (k & 1) as f32, g: (k >> 1 & 1) as f32, b: (k >> 2 & 1) as f32 })
        .map(|c| Rgb { r: 1.0 - c.r, g: 1.0 - c.g, b: 1.0 - c.b })
        .collect();
    let cube = ColorCube::new(2, data).unwrap();
    assert!(close(cube.lookup(BLACK), WHITE, 1e-5));
    assert!(close(cube.lookup(RED), Rgb { r: 0.0, g: 1.0, b: 1.0 }, 1e-5));
}

// Colour map and palettes (bifrost_render::colormap).

#[test]
fn colour_map_sends_mid_luma_to_the_middle_of_its_gradient() {
    let map = ColorMap::new(vec![BLACK, WHITE]).unwrap();
    assert!(close(map.at(0.5), Rgb { r: 0.5, g: 0.5, b: 0.5 }, 1e-5));
    assert!(ColorMap::new(Vec::new()).is_none());
}

#[test]
fn colour_map_recolours_a_pixel_and_keeps_its_alpha() {
    let map = ColorMap::new(vec![RED, GREEN]).unwrap();
    assert_eq!(map.map_pixel(0x80ff_ffff) >> 24, 0x80, "alpha is kept");
}

#[test]
fn palette_snaps_each_colour_to_its_nearest_entry_in_rgb_and_in_lab() {
    let near_white = Rgb { r: 0.9, g: 0.9, b: 0.9 };
    for perceptual in [false, true] {
        let palette = Palette::new(vec![BLACK, WHITE], perceptual).unwrap();
        assert!(close(palette.nearest(near_white), WHITE, 1e-6), "perceptual {perceptual}");
        assert_eq!(palette.palettize_pixel(0x80ff_ffff) >> 24, 0x80, "perceptual {perceptual}: alpha is kept");
    }
    assert!(Palette::new(Vec::new(), false).is_none());
}

// CIE L*a*b* colour (bifrost_render::colorlab).

#[test]
fn lab_of_white_and_black_are_the_ends_of_the_lightness_scale() {
    let white = Lab::from_rgb(WHITE);
    assert!((white.l - 100.0).abs() < 0.01);
    assert!(white.a.abs() < 0.01 && white.b.abs() < 0.01);
    assert!(Lab::from_rgb(BLACK).l.abs() < 0.01);
}

#[test]
fn lab_round_trip_returns_the_same_grey() {
    let grey = Rgb { r: 0.4, g: 0.4, b: 0.4 };
    assert!(close(Lab::from_rgb(grey).to_rgb(), grey, 1e-3));
}

#[test]
fn lab_difference_is_zero_for_equal_colours_and_positive_otherwise() {
    let red = Lab::from_rgb(RED);
    assert_eq!(delta_e(red, red), 0.0);
    assert!(delta_e(red, Lab::from_rgb(GREEN)) > 50.0);
}

// Sharpen (bifrost_render::stylize_filters).

#[test]
fn unsharp_mask_with_no_intensity_is_the_identity() {
    let img = numbered_9x9();
    assert_eq!(stylize_filters::unsharp_mask(&img, 2.0, 0.0).px, img.px);
    assert_eq!(stylize_filters::unsharp_mask(&img, 2.0, f32::NAN).px, img.px);
}
