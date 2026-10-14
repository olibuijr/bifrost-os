//! Parity sdk-tests-17: headless tests of the render models for reductions and statistics, the halftone and tile
//! filters, and the pixel format descriptions. Each test works on pixel values held in memory and asserts exact
//! values. No display or device is used.

use bifrost_render::filter::{histogram, HistogramInfo, Plane};
use bifrost_render::gfx::Image;
use bifrost_render::halftone_tile_filters::{
    affine_tile, cmyk_halftone, kaleidoscope, op_tile, screen, Screen, ScreenParams,
};
use bifrost_render::pixel_format::{find, FourCc};
use bifrost_render::plane_stats::{reduce_columns, reduce_rows, statistics, Integral, Reduce};

/// Packs a colour as the render crate does: red in the low byte, alpha in the high byte.
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

fn solid(w: usize, h: usize, px: u32) -> Image {
    let mut img = Image::new(w, h);
    img.px.fill(px);
    img
}

// ---- row and column reductions -------------------------------------------------------------------------------

#[test]
fn a_row_sum_is_not_clamped_to_the_byte_range() {
    let src = Plane::filled(3, 2, 255);
    assert_eq!(reduce_rows(&src, Reduce::Sum), vec![765.0, 765.0]);
    assert_eq!(reduce_columns(&src, Reduce::Sum), vec![510.0, 510.0, 510.0]);
}

#[test]
fn row_and_column_reductions_of_a_pixel_grid_match_their_definitions() {
    // rows: [10, 40, 20] and [5, 5, 90]
    let src = Plane::new(3, 2, vec![10, 40, 20, 5, 5, 90]).unwrap();
    assert_eq!(reduce_rows(&src, Reduce::Min), vec![10.0, 5.0]);
    assert_eq!(reduce_rows(&src, Reduce::Max), vec![40.0, 90.0]);
    assert_eq!(reduce_rows(&src, Reduce::Mean), vec![70.0 / 3.0, 100.0 / 3.0]);
    assert_eq!(reduce_columns(&src, Reduce::Mean), vec![7.5, 22.5, 55.0]);
    assert_eq!(reduce_columns(&src, Reduce::Min), vec![5.0, 5.0, 20.0]);
}

#[test]
fn reductions_of_an_empty_plane_have_no_lines_and_a_zero_width_row_folds_to_zero() {
    let empty = Plane::new(0, 0, Vec::new()).unwrap();
    assert!(reduce_rows(&empty, Reduce::Max).is_empty());
    assert!(reduce_columns(&empty, Reduce::Sum).is_empty());
    let no_columns = Plane::new(0, 3, Vec::new()).unwrap();
    assert_eq!(reduce_rows(&no_columns, Reduce::Mean), vec![0.0; 3]);
}

// ---- integral images and region statistics -------------------------------------------------------------------

#[test]
fn an_integral_image_sums_a_large_plane_without_overflow() {
    let src = Plane::filled(1000, 1000, 255);
    let integral = Integral::new(&src);
    assert_eq!(integral.rect_sum(0, 0, 1000, 1000), 255_000_000);
    assert_eq!(integral.rect_sum(250, 500, 100, 10), 255 * 1000);
    assert_eq!(integral.rect_sum(0, 0, 0, 1000), 0);
}

#[test]
fn an_integral_rectangle_matches_a_direct_sum_over_a_pixel_pattern() {
    let (w, h) = (7usize, 5usize);
    let pixels: Vec<u8> = (0..w * h).map(|i| (i * 37 % 256) as u8).collect();
    let src = Plane::new(w, h, pixels.clone()).unwrap();
    let integral = Integral::new(&src);
    for (x, y, rw, rh) in [(0, 0, 7, 5), (2, 1, 3, 2), (6, 4, 1, 1), (1, 0, 4, 5)] {
        let mut direct = 0u64;
        for yy in y..y + rh {
            for xx in x..x + rw {
                direct += pixels[yy * w + xx] as u64;
            }
        }
        assert_eq!(integral.rect_sum(x, y, rw, rh), direct, "rectangle at {x},{y} of {rw}x{rh}");
    }
}

#[test]
fn a_squared_integral_gives_the_sum_of_squares_of_a_region() {
    assert_eq!(Integral::of_squares(&Plane::filled(4, 4, 3)).rect_sum(0, 0, 4, 4), 16 * 9);
    assert_eq!(Integral::of_squares(&Plane::filled(4, 4, 3)).rect_sum(1, 1, 2, 1), 2 * 9);
}

#[test]
fn an_area_average_is_the_mean_of_its_region_only() {
    // top row 10 20 30 40, bottom row 200 200 200 200
    let src = Plane::new(4, 2, vec![10, 20, 30, 40, 200, 200, 200, 200]).unwrap();
    assert_eq!(statistics(&src, 0, 0, 4, 1).unwrap().mean, 25.0);
    assert_eq!(statistics(&src, 0, 1, 4, 1).unwrap().mean, 200.0);
    let whole = statistics(&src, 0, 0, 4, 2).unwrap();
    assert_eq!(whole.mean, 112.5);
    // sum of squares 163000 over 8 pixels, mean squared 12656.25
    assert_eq!(whole.variance, 7718.75);
    assert_eq!((whole.min, whole.max), (10, 200));
}

#[test]
fn a_flat_region_has_no_variance_and_clips_to_the_plane() {
    let src = Plane::filled(5, 5, 90);
    let inside = statistics(&src, 1, 1, 3, 3).unwrap();
    assert_eq!((inside.mean, inside.variance, inside.min, inside.max), (90.0, 0.0, 90, 90));
    let clipped = statistics(&src, 3, 3, 10, 10).unwrap();
    assert_eq!(clipped.mean, 90.0);
    assert!(statistics(&src, 5, 0, 1, 1).is_none(), "a region outside the plane is empty");
}

// ---- histograms ----------------------------------------------------------------------------------------------

#[test]
fn a_histogram_counts_each_pixel_once_in_its_bin() {
    let src = Plane::new(4, 1, vec![0, 63, 64, 255]).unwrap();
    let bins = histogram(&src, HistogramInfo::new(4, 0, 255).unwrap());
    assert_eq!(bins, vec![2, 1, 0, 1]);
    assert_eq!(bins.iter().sum::<u32>(), 4);
}

#[test]
fn a_full_range_histogram_puts_every_value_in_its_own_bin() {
    let flat = Plane::filled(16, 16, 200);
    let bins = histogram(&flat, HistogramInfo::new(256, 0, 255).unwrap());
    assert_eq!(bins[200], 256);
    assert_eq!(bins.iter().filter(|&&n| n > 0).count(), 1);
}

// ---- halftone screens ----------------------------------------------------------------------------------------

#[test]
fn a_mid_grey_image_gets_both_inked_and_clear_pixels_from_every_screen() {
    let img = solid(24, 24, rgba(128, 128, 128, 255));
    for kind in [Screen::Dot, Screen::Line, Screen::Circular, Screen::Hatched] {
        let out = screen(&img, kind, &ScreenParams::default());
        let inked = out.px.iter().filter(|&&p| p & 255 < 128).count();
        assert!(inked > 576 / 10 && inked < 576 * 9 / 10, "{kind:?} inks {inked} of 576 pixels");
    }
}

#[test]
fn a_dot_screen_repeats_every_pattern_width_across_a_uniform_image() {
    let img = solid(24, 6, rgba(128, 128, 128, 255));
    let out = screen(&img, Screen::Dot, &ScreenParams::default());
    assert_eq!((out.w, out.h), (24, 6));
    for y in 0..6 {
        for x in 0..18 {
            assert_eq!(out.px[y * 24 + x], out.px[y * 24 + x + 6], "pixel {x},{y} against one period later");
        }
    }
}

#[test]
fn a_pure_red_image_keeps_its_red_under_cmyk_halftone() {
    let img = solid(12, 12, rgba(255, 0, 0, 255));
    let out = cmyk_halftone(&img, &ScreenParams::default(), 1.0, 0.5);
    assert!(out.px.iter().all(|&p| p & 255 == 255), "cyan and black are empty for red, so red is never absorbed");
    assert!(out.px.iter().any(|&p| (p >> 8) & 255 < 128), "magenta ink absorbs green where it covers");
    assert!(out.px.iter().all(|&p| p >> 24 == 255), "the alpha of each pixel is kept");
}

// ---- tile filters --------------------------------------------------------------------------------------------

#[test]
fn an_op_tile_of_a_uniform_image_is_the_same_colour_everywhere() {
    let colour = rgba(40, 90, 200, 255);
    let img = solid(9, 7, colour);
    let out = op_tile(&img, (4.0, 3.0), 2.8, 0.3, 65.0);
    assert_eq!((out.w, out.h), (9, 7));
    assert!(out.px.iter().all(|&p| p == colour));
}

#[test]
fn a_kaleidoscope_keeps_the_colour_of_a_uniform_image_inside_its_radius() {
    let colour = rgba(10, 20, 30, 255);
    let out = kaleidoscope(&solid(8, 8, colour), (4.0, 4.0), 6, 0.0);
    for y in 0..8 {
        for x in 0..8 {
            let (dx, dy) = (x as f32 + 0.5 - 4.0, y as f32 + 0.5 - 4.0);
            if (dx * dx + dy * dy).sqrt() <= 3.5 {
                assert_eq!(out.px[y * 8 + x], colour, "pixel {x},{y}");
            }
        }
    }
}

#[test]
fn an_affine_tile_shifted_by_one_tile_width_returns_the_same_pixels() {
    let mut img = Image::new(4, 2);
    img.px = (0..8).map(|k| rgba(k * 30, 0, 255 - k * 30, 255)).collect();
    assert_eq!(affine_tile(&img, [1.0, 0.0, 0.0, 1.0, 4.0, 0.0]).px, img.px);
    assert_eq!(affine_tile(&img, [1.0, 0.0, 0.0, 1.0, 0.0, 2.0]).px, img.px);
}

// ---- pixel format descriptions -------------------------------------------------------------------------------

#[test]
fn a_fourcc_stores_its_first_character_in_the_low_byte() {
    assert_eq!(FourCc::new(*b"NV12").0, 0x3231_564e);
    assert_eq!(FourCc::new(*b"AR24").bytes(), *b"AR24");
}

#[test]
fn a_code_parses_only_as_four_printable_bytes() {
    assert_eq!(FourCc::parse("NV12").map(|c| c.to_string()), Some("NV12".to_string()));
    assert_eq!(FourCc::parse("NV 2"), None, "a space is not a printable code character");
    assert_eq!(FourCc::parse(""), None);
    assert_eq!(FourCc::parse("RG16").unwrap(), FourCc::new(*b"RG16"));
}

#[test]
fn every_known_format_has_a_distinct_code_and_resolves_to_itself() {
    for (i, format) in bifrost_render::pixel_format::TABLE.iter().enumerate() {
        assert!(bifrost_render::pixel_format::TABLE[..i].iter().all(|other| other.code != format.code), "{}", format.name);
        assert_eq!(find(format.code), Some(format), "{}", format.name);
    }
}

#[test]
fn a_full_hd_nv12_frame_is_about_three_megabytes_in_two_planes() {
    let nv12 = find(FourCc::parse("NV12").unwrap()).unwrap();
    assert!(nv12.is_planar());
    assert_eq!(nv12.plane_size(0, 1920, 1080), Some((1920, 1080)));
    assert_eq!(nv12.plane_size(1, 1920, 1080), Some((1920, 540)));
    assert_eq!(nv12.buffer_len(1920, 1080), Some(3_110_400));
}

#[test]
fn a_full_hd_nv16_frame_keeps_full_height_chroma() {
    let nv16 = find(FourCc::parse("NV16").unwrap()).unwrap();
    assert_eq!(nv16.plane_size(1, 1920, 1080), Some((1920, 1080)));
    assert_eq!(nv16.buffer_len(1920, 1080), Some(4_147_200));
}

#[test]
fn a_packed_format_has_one_plane_that_scales_with_the_picture() {
    let argb = find(FourCc::parse("AR24").unwrap()).unwrap();
    assert!(!argb.is_planar());
    assert_eq!(argb.planes.len(), 1);
    assert_eq!(argb.buffer_len(1920, 1080), Some(1920 * 1080 * 4));
    assert_eq!(argb.plane_size(0, 0, 10), Some((0, 10)));
}
