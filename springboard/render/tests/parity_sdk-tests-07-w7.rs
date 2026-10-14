//! Parity sdk-tests-07 (wave 7): headless tests for image decoding and screen fitting. The PNG and JPEG encoders write
//! known pixels into memory, the decoder reads them back, and each test asserts the decoded pixels. The fitting helpers
//! are checked on in-memory pictures. Nothing is drawn to a display and no file is read.

use bifrost_render::cg::imageio::encode::{encode_jpeg, encode_png, JpegOptions, PngOptions};
use bifrost_render::cg::imageio::{decode, is_image, pixel_size, Format, MAX_SIDE};
use bifrost_render::gfx::{pack, Image};
use bifrost_render::imgdec::{box_down, contain, cover, fit, fit_dims, fit_size};

fn solid(w: usize, h: usize, c: u32) -> Image {
    Image { w, h, px: vec![c; w * h] }
}

fn channel(p: u32, shift: u32) -> i32 {
    ((p >> shift) & 0xff) as i32
}

#[test]
fn headless_the_picture_size_limit_is_the_eight_thousand_pixel_side() {
    assert_eq!(MAX_SIDE, 8192);
    assert_eq!(bifrost_render::imgdec::MAX_SIDE, MAX_SIDE);
}

#[test]
fn headless_a_png_decodes_to_the_same_eight_bit_pixels_it_was_written_from() {
    let rgba: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let png = encode_png(2, 2, &rgba, &PngOptions::default()).expect("a 2 by 2 picture encodes");
    let img = decode(&png).expect("the encoded picture decodes");
    assert_eq!((img.w, img.h), (2, 2));
    assert_eq!(
        img.px,
        vec![pack(255, 0, 0, 255), pack(0, 255, 0, 255), pack(0, 0, 255, 255), pack(255, 255, 255, 255)]
    );
}

#[test]
fn headless_decoded_pixels_are_premultiplied_by_their_alpha() {
    let rgba: [u8; 8] = [255, 0, 0, 128, 255, 255, 255, 0];
    let png = encode_png(2, 1, &rgba, &PngOptions::default()).expect("encodes");
    let img = decode(&png).expect("decodes");
    assert_eq!(img.px[0], pack(128, 0, 0, 128), "half-transparent red keeps half of its red");
    assert_eq!(img.px[1], pack(0, 0, 0, 0), "fully transparent white has no colour left");
}

#[test]
fn headless_a_jpeg_decodes_to_a_picture_of_the_same_size_and_colour() {
    let rgba: Vec<u8> = (0..16 * 16).flat_map(|_| [128u8, 128, 128, 255]).collect();
    let options = JpegOptions { quality: 100, subsample: false, ..JpegOptions::default() };
    let jpeg = encode_jpeg(16, 16, &rgba, &options).expect("a 16 by 16 picture encodes");
    let img = decode(&jpeg).expect("the encoded picture decodes");
    assert_eq!((img.w, img.h), (16, 16));
    for p in &img.px {
        for shift in [0, 8, 16] {
            assert!((channel(*p, shift) - 128).abs() <= 4, "grey stays near 128, got {:#010x}", p);
        }
        assert_eq!(channel(*p, 24), 255, "an opaque jpeg stays opaque");
    }
}

#[test]
fn headless_the_format_is_detected_from_the_first_bytes() {
    let png = encode_png(1, 1, &[1u8, 2, 3, 255], &PngOptions::default()).expect("encodes");
    let jpeg = encode_jpeg(8, 8, &[9u8; 8 * 8 * 4], &JpegOptions::default()).expect("encodes");
    assert_eq!(Format::detect(&png), Some(Format::Png));
    assert_eq!(Format::detect(&jpeg), Some(Format::Jpeg));
    assert!(is_image(&png));
    assert!(!is_image(b"plain text is not a picture"));
}

#[test]
fn headless_the_pixel_size_is_read_from_the_header_alone() {
    let png = encode_png(5, 3, &[0u8; 5 * 3 * 4], &PngOptions::default()).expect("encodes");
    assert_eq!(pixel_size(&png), Some((5, 3)));
    assert_eq!(pixel_size(b"not a picture"), None);
}

#[test]
fn headless_a_picture_wider_than_the_limit_is_refused_not_decoded() {
    let width = MAX_SIDE + 1;
    let png = encode_png(width, 1, &vec![0u8; width * 4], &PngOptions::default()).expect("encodes");
    assert!(decode(&png).is_none(), "a picture past 8192 points is refused");
}

#[test]
fn headless_bytes_that_are_not_a_picture_do_not_decode() {
    assert!(decode(b"not an image at all").is_none());
}

#[test]
fn headless_box_down_averages_each_block_of_pixels() {
    let img = Image {
        w: 2,
        h: 2,
        px: vec![pack(0, 0, 0, 255), pack(255, 255, 255, 255), pack(0, 0, 0, 255), pack(255, 255, 255, 255)],
    };
    let small = box_down(&img, 2);
    assert_eq!((small.w, small.h), (1, 1));
    assert_eq!(small.px, vec![pack(128, 128, 128, 255)], "half black and half white is mid grey");
    assert_eq!(box_down(&img, 1).px, img.px, "a factor of one keeps every pixel");
}

#[test]
fn headless_cover_fills_the_frame_with_the_same_colour_when_the_picture_is_solid() {
    let picture = solid(40, 10, pack(10, 20, 30, 255));
    let c = cover(&picture, 20, 20);
    assert_eq!((c.w, c.h), (20, 20));
    assert!(c.px.iter().all(|&p| p == pack(10, 20, 30, 255)));
}

#[test]
fn headless_fit_dims_shrinks_inside_the_box_and_never_enlarges() {
    assert_eq!(fit_dims(4000, 3000, 1200, 1920), (1200, 900));
    assert_eq!(fit_dims(300, 3000, 1200, 1920), (192, 1920), "a tall picture is limited by its height");
    assert_eq!(fit_dims(10, 20, 100, 100), (10, 20), "a small picture keeps its size");
    assert_eq!(fit_dims(0, 5, 10, 10), (0, 0), "an empty picture has no size");
}

#[test]
fn headless_fit_size_follows_fit_dims_for_a_picture() {
    let picture = solid(4000, 3000, pack(0, 0, 0, 255));
    assert_eq!(fit_size(&picture, 1200, 1920), (1200, 900));
    assert_eq!(fit_size(&Image::new(0, 0), 10, 10), (0, 0));
}

#[test]
fn headless_contain_scales_a_large_picture_to_fit_and_keeps_its_colour() {
    let picture = solid(4000, 3000, pack(5, 6, 7, 255));
    let c = contain(&picture, 1200, 1920);
    assert_eq!((c.w, c.h), (1200, 900));
    assert!(c.px.iter().all(|&p| p == pack(5, 6, 7, 255)));
}

#[test]
fn headless_fit_only_shrinks_a_picture_that_is_too_large() {
    let big = solid(400, 200, pack(1, 1, 1, 255));
    let shrunk = fit(&big, 100, 100);
    assert_eq!((shrunk.w, shrunk.h), (100, 50));
    let small = solid(10, 20, pack(1, 1, 1, 255));
    let same = fit(&small, 100, 100);
    assert_eq!((same.w, same.h), (10, 20));
    assert_eq!(same.px, small.px);
}
