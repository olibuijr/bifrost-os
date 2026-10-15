//! Parity sdk-complete-32: headless tests for the per-pixel kernels (`bifrost_render::kernel`). The kernels run on
//! packed pixels in memory: no surface, no GPU and no display. Each test drives the public kernel API.

use bifrost_render::kernel::{blend_kernel, color_kernel, run_chain, warp_kernel, Blend, ColorMap, Filter, Sampler, Wrap};

const BLACK: u32 = 0xff00_0000;
const WHITE: u32 = 0xffff_ffff;

fn invert(c: [f32; 4]) -> [f32; 4] {
    [1.0 - c[0], 1.0 - c[1], 1.0 - c[2], c[3]]
}

#[test]
fn a_sampler_refuses_a_pixel_count_that_does_not_match_its_size() {
    assert!(Sampler::new(2, 2, vec![BLACK; 3], Wrap::Edge).is_none());
    assert!(Sampler::new(0, 2, Vec::new(), Wrap::Edge).is_none());
    assert!(Sampler::new(2, 2, vec![BLACK; 4], Wrap::Edge).is_some());
}

#[test]
fn a_colour_kernel_inverts_every_pixel_of_a_custom_closure() {
    let src = Sampler::new(2, 1, vec![BLACK, WHITE], Wrap::Edge).unwrap();
    assert_eq!(color_kernel(&src, invert), vec![WHITE, BLACK]);
}

#[test]
fn a_warp_kernel_with_the_identity_map_reproduces_the_source() {
    let src = Sampler::new(2, 2, vec![BLACK, WHITE, WHITE, BLACK], Wrap::Edge).unwrap();
    let out = warp_kernel(&src, 2, 2, Filter::Nearest, |x, y| (x, y)).unwrap();
    assert_eq!(out, vec![BLACK, WHITE, WHITE, BLACK]);
}

#[test]
fn a_warp_kernel_refuses_an_empty_output_size() {
    let src = Sampler::new(1, 1, vec![BLACK], Wrap::Edge).unwrap();
    assert!(warp_kernel(&src, 0, 4, Filter::Nearest, |x, y| (x, y)).is_none());
}

#[test]
fn a_clear_wrap_sampler_reads_transparent_outside_its_pixels() {
    let src = Sampler::new(1, 1, vec![WHITE], Wrap::Clear).unwrap();
    let out = warp_kernel(&src, 2, 1, Filter::Nearest, |x, y| (x, y)).unwrap();
    assert_eq!(out[0], WHITE, "the centre of the first destination pixel is inside");
    assert_eq!(out[1] >> 24, 0, "the second centre maps outside and is transparent");
}

#[test]
fn a_blend_kernel_multiplies_white_by_black_into_black() {
    assert_eq!(blend_kernel(&[WHITE], &[BLACK], 1, 1, Blend::Multiply), Some(vec![BLACK]));
}

#[test]
fn a_blend_kernel_refuses_buffers_that_do_not_match_the_size() {
    assert!(blend_kernel(&[WHITE, WHITE], &[BLACK], 1, 1, Blend::Normal).is_none());
}

#[test]
fn a_chain_of_kernels_runs_in_order_and_an_empty_chain_copies_the_source() {
    let src = Sampler::new(1, 1, vec![BLACK], Wrap::Edge).unwrap();
    let copy = run_chain(&src, &[]).unwrap();
    assert_eq!(copy, src);

    let once = ColorMap { name: "invert", map: invert };
    let twice_inverted = run_chain(&src, &[&once, &once]).unwrap();
    assert_eq!(twice_inverted.pixels, vec![BLACK]);
    let inverted = run_chain(&src, &[&once]).unwrap();
    assert_eq!(inverted.pixels, vec![WHITE]);
}

#[test]
fn a_colour_map_kernel_reports_the_name_it_was_given() {
    use bifrost_render::kernel::UnaryImageKernel;
    let map = ColorMap { name: "invert", map: invert };
    assert_eq!(map.label(), "invert");
}
