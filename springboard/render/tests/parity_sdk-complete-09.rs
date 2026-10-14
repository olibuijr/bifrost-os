//! Parity sdk-complete-09: headless tests for clip stacks, affine transforms, I420 pixel transfer and rotation angles.
//! The clip tests draw into an in-memory surface (gfx::Surface) and check the pixels; nothing opens a display.

use bifrost_render::affine::{Affine, TransformStack};
use bifrost_render::clip::{fill_clipped, ClipStack, Rect};
use bifrost_render::gfx::rotate::Rotation;
use bifrost_render::gfx::{Image, Surface};
use bifrost_render::video_transfer::{transfer_i420, transfer_nv12, turn_for_degrees, Nv12, I420, BT601};

fn painted_pixels(buf: &[u32]) -> Vec<usize> {
    (0..buf.len()).filter(|&i| buf[i] != 0).collect()
}

#[test]
fn a_pixel_fill_inside_a_clip_paints_only_the_clipped_pixels() {
    let (w, h) = (6usize, 4usize);
    let mut buf = vec![0u32; w * h];
    let mut surface = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
    fill_clipped(&mut surface, Rect::new(1, 1, 3, 2), Rect::new(0, 0, 10, 10), 0xffff_ffff);
    // Rows 1 and 2, columns 1 to 3: six pixels, and nothing else.
    assert_eq!(painted_pixels(&buf), vec![7, 8, 9, 13, 14, 15]);
}

#[test]
fn a_nested_clip_paints_the_intersection_of_both_clips_in_pixels() {
    let (w, h) = (6usize, 4usize);
    let mut buf = vec![0u32; w * h];
    let mut clips = ClipStack::new(w as i32, h as i32);
    clips.push(Rect::new(1, 1, 3, 2));
    assert_eq!(clips.push(Rect::new(2, 0, 10, 10)), Rect::new(2, 1, 2, 2));
    let mut surface = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
    fill_clipped(&mut surface, clips.current(), Rect::new(0, 0, 6, 4), 0xff00_ff00);
    assert_eq!(painted_pixels(&buf), vec![8, 9, 14, 15]);
}

#[test]
fn a_fill_that_misses_the_clip_paints_no_pixel() {
    let (w, h) = (4usize, 4usize);
    let mut buf = vec![0u32; w * h];
    let mut surface = Surface { w, h, stride: w, px: &mut buf, y0: 0 };
    fill_clipped(&mut surface, Rect::new(0, 0, 2, 2), Rect::new(3, 3, 5, 5), 0xff00_00ff);
    assert!(painted_pixels(&buf).is_empty());
}

#[test]
fn popping_a_clip_restores_the_clip_below_it_and_the_base_stays() {
    let mut clips = ClipStack::new(100, 80);
    clips.push(Rect::new(10, 10, 5, 5));
    assert!(clips.pop());
    assert_eq!(clips.current(), Rect::new(0, 0, 100, 80));
    assert!(!clips.pop(), "the base clip is never popped");
    assert_eq!(clips.depth(), 0);
}

fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
    assert!((actual.0 - expected.0).abs() < 1e-4 && (actual.1 - expected.1).abs() < 1e-4, "{actual:?} != {expected:?}");
}

#[test]
fn a_transform_applies_the_first_transform_first() {
    let move_then_scale = Affine::translate(10.0, 0.0).then(Affine::scale(2.0, 2.0));
    assert_close(move_then_scale.apply(1.0, 1.0), (22.0, 2.0));
    let scale_then_move = Affine::scale(2.0, 2.0).then(Affine::translate(10.0, 0.0));
    assert_close(scale_then_move.apply(1.0, 1.0), (12.0, 2.0));
}

#[test]
fn a_quarter_turn_maps_the_x_axis_onto_the_y_axis() {
    let turned = Affine::rotate(std::f32::consts::FRAC_PI_2);
    assert_close(turned.apply(1.0, 0.0), (0.0, 1.0));
}

#[test]
fn an_inverted_transform_maps_points_back_to_where_they_came_from() {
    let t = Affine::scale(2.0, 4.0).then(Affine::translate(3.0, -5.0));
    let inverse = t.invert().expect("a scale and a move can be undone");
    let (x, y) = t.apply(7.0, 9.0);
    assert_close(inverse.apply(x, y), (7.0, 9.0));
}

#[test]
fn a_collapsing_transform_has_no_inverse() {
    assert!(Affine::scale(0.0, 1.0).invert().is_none());
    assert!(Affine::IDENTITY.invert().is_some());
}

#[test]
fn a_saved_transform_state_is_restored_after_a_concat() {
    let mut stack = TransformStack::new();
    stack.concat(Affine::translate(5.0, 0.0));
    stack.save();
    stack.concat(Affine::scale(2.0, 2.0));
    // The new transform is applied to the point before the saved move: (1, 0) scales to (2, 0), then moves to (7, 0).
    assert_close(stack.current().apply(1.0, 0.0), (7.0, 0.0));
    assert!(stack.restore());
    assert_close(stack.current().apply(1.0, 0.0), (6.0, 0.0));
    assert!(!stack.restore(), "nothing is saved below the base state");
}

#[test]
fn an_i420_picture_converts_to_the_same_pixels_as_the_same_nv12_picture() {
    let y = [16u8, 235, 126, 16];
    let u = [100u8];
    let v = [200u8];
    let planar = I420 { y: &y, u: &u, v: &v, stride: 2, chroma_stride: 1, w: 2, h: 2 };
    let interleaved = [100u8, 200];
    let semi_planar = Nv12 { y: &y, uv: &interleaved, stride: 2, w: 2, h: 2 };
    let from_i420 = transfer_i420(planar, 2, 2, BT601).expect("a valid I420 picture converts");
    let from_nv12 = transfer_nv12(semi_planar, 2, 2, BT601).expect("a valid NV12 picture converts");
    assert_eq!(from_i420.px, from_nv12.px);
}

#[test]
fn an_i420_picture_with_short_chroma_planes_is_refused() {
    let y = [16u8; 4];
    let u: [u8; 0] = [];
    let v = [128u8];
    let picture = I420 { y: &y, u: &u, v: &v, stride: 2, chroma_stride: 1, w: 2, h: 2 };
    assert!(transfer_i420(picture, 2, 2, BT601).is_none());
}

#[test]
fn an_i420_picture_scales_by_nearest_neighbour_like_nv12() {
    let y = [16u8, 235, 16, 235];
    let u = [128u8];
    let v = [128u8];
    let picture = I420 { y: &y, u: &u, v: &v, stride: 2, chroma_stride: 1, w: 2, h: 2 };
    let scaled = transfer_i420(picture, 4, 4, BT601).unwrap();
    assert_eq!(scaled.px.len(), 16);
    // Columns 0 and 1 of every output row are the dark column and columns 2 and 3 the bright one.
    assert_eq!(scaled.px[0], scaled.px[1]);
    assert_ne!(scaled.px[0], scaled.px[2]);
}

#[test]
fn a_clockwise_angle_is_a_quarter_turn_and_angles_wrap_a_full_turn() {
    assert_eq!(turn_for_degrees(0), Some(Rotation::None));
    assert_eq!(turn_for_degrees(90), Some(Rotation::Cw));
    assert_eq!(turn_for_degrees(180), Some(Rotation::Half));
    assert_eq!(turn_for_degrees(270), Some(Rotation::Ccw));
    assert_eq!(turn_for_degrees(-90), Some(Rotation::Ccw), "minus ninety is a turn counter-clockwise");
    assert_eq!(turn_for_degrees(450), Some(Rotation::Cw));
    assert_eq!(turn_for_degrees(45), None, "only multiples of ninety are turns");
}

#[test]
fn a_picture_rotated_by_its_angle_keeps_its_pixels_in_a_new_order() {
    let picture = Image { w: 2, h: 1, px: vec![7, 8] };
    let turn = turn_for_degrees(90).unwrap();
    let rotated = bifrost_render::video_transfer::rotate_picture(&picture, turn, false, false);
    assert_eq!((rotated.w, rotated.h, rotated.px), (1, 2, vec![7, 8]));
}
