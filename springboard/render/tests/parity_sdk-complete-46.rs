//! Parity sdk-complete-46: headless tests of the Gaussian and Laplacian pyramids over 8-bit planes. No test composes a
//! frame or touches a GPU.

use bifrost_render::filter::Plane;
use bifrost_render::laplacian_pyramid::{collapse, expand, gaussian_pyramid, laplacian_pyramid, reduce};

fn ramp(width: usize, height: usize) -> Plane {
    let pixels = (0..width * height).map(|i| ((i * 29) % 256) as u8).collect();
    Plane::new(width, height, pixels).unwrap()
}

#[test]
fn headless_a_gaussian_pyramid_halves_each_level_rounding_up() {
    let levels = gaussian_pyramid(&ramp(17, 9), 4);
    let sizes: Vec<(usize, usize)> = levels.iter().map(|p| (p.width, p.height)).collect();
    assert_eq!(sizes, vec![(17, 9), (9, 5), (5, 3), (3, 2)]);
}

#[test]
fn headless_the_laplacian_pyramid_collapses_back_to_the_source_pixels() {
    let src = ramp(13, 11);
    assert_eq!(collapse(&laplacian_pyramid(&src, 3)), Some(src));
}

#[test]
fn headless_a_flat_plane_leaves_only_its_coarsest_level_non_zero() {
    let lap = laplacian_pyramid(&Plane::filled(8, 8, 90), 3);
    assert!(lap[0].values.iter().all(|&v| v == 0));
    assert!(lap[1].values.iter().all(|&v| v == 0));
    assert_eq!(lap[2].values, vec![90i16; 4]);
}

#[test]
fn headless_expanding_a_two_pixel_row_interpolates_between_its_ends() {
    let src = Plane::new(2, 1, vec![0, 255]).unwrap();
    assert_eq!(expand(&src, 4, 1).pixels, vec![0, 64, 191, 255]);
}

#[test]
fn headless_reducing_a_flat_plane_keeps_its_value() {
    let out = reduce(&Plane::filled(6, 4, 33));
    assert_eq!((out.width, out.height), (3, 2));
    assert_eq!(out.pixels, vec![33u8; 6]);
}
