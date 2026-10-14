//! Parity sdk-complete-30: headless tests for block motion estimation between two 8-bit planes. Each test builds its
//! planes in memory; no video frame is decoded and no device is opened.

use bifrost_render::block_motion::{MotionEstimation, MotionVector};
use bifrost_render::filter::Plane;

/// A 16 by 16 ramp whose value at (x, y) is `x + 16 * y`. Every displacement changes the sum of absolute differences
/// by a different amount, so the best match of a block is unique.
fn ramp_at(x: usize, y: usize) -> u8 {
    (x + 16 * y) as u8
}

fn ramp() -> Plane {
    let pixels = (0..16).flat_map(|y| (0..16).map(move |x| ramp_at(x, y))).collect();
    Plane::new(16, 16, pixels).expect("16 by 16 pixels")
}

/// The ramp moved one pixel to the right: column x of the current plane holds column x - 1 of the reference.
fn ramp_moved_right() -> Plane {
    let pixels = (0..16).flat_map(|y| (0..16).map(move |x| ramp_at(x.saturating_sub(1), y))).collect();
    Plane::new(16, 16, pixels).expect("16 by 16 pixels")
}

#[test]
fn a_session_refuses_an_empty_plane_or_an_empty_block() {
    assert!(MotionEstimation::new(0, 16, 8, 2).is_none());
    assert!(MotionEstimation::new(16, 0, 8, 2).is_none());
    assert!(MotionEstimation::new(16, 16, 0, 2).is_none());
    assert!(MotionEstimation::new(16, 16, 8, 2).is_some());
}

#[test]
fn the_block_grid_clips_the_last_row_and_column_of_blocks() {
    let session = MotionEstimation::new(10, 10, 4, 1).expect("a valid session");
    assert_eq!(session.grid(), (3, 3), "10 pixels in blocks of 4 need three blocks across and down");
    let field = session.estimate(&Plane::filled(10, 10, 9), &Plane::filled(10, 10, 9)).expect("matching sizes");
    assert_eq!((field.cols, field.rows, field.vectors.len()), (3, 3, 9));
}

#[test]
fn a_static_scene_keeps_every_block_at_zero_offset_with_no_difference() {
    let session = MotionEstimation::new(16, 16, 8, 2).expect("a valid session");
    let field = session.estimate(&ramp(), &ramp()).expect("matching sizes");
    assert!(field.vectors.iter().all(|v| *v == MotionVector { dx: 0, dy: 0, sad: 0 }));
}

#[test]
fn a_moved_ramp_is_matched_at_offset_minus_one_in_the_right_hand_blocks() {
    let session = MotionEstimation::new(16, 16, 8, 2).expect("a valid session");
    let field = session.estimate(&ramp_moved_right(), &ramp()).expect("matching sizes");
    // Block 1 is the top right block and block 3 the bottom right one. Both lie fully inside the reference when
    // displaced by (-1, 0), where they match exactly.
    assert_eq!(field.vectors[1], MotionVector { dx: -1, dy: 0, sad: 0 });
    assert_eq!(field.vectors[3], MotionVector { dx: -1, dy: 0, sad: 0 });
}

#[test]
fn every_chosen_block_lies_inside_the_reference_plane() {
    let session = MotionEstimation::new(16, 16, 8, 2).expect("a valid session");
    let field = session.estimate(&ramp_moved_right(), &ramp()).expect("matching sizes");
    for (i, v) in field.vectors.iter().enumerate() {
        let (x0, y0) = ((i % 2) as i32 * 8, (i / 2) as i32 * 8);
        assert!((0..=8).contains(&(x0 + v.dx)), "block {i} moves outside the plane horizontally");
        assert!((0..=8).contains(&(y0 + v.dy)), "block {i} moves outside the plane vertically");
    }
}

#[test]
fn a_zero_range_reports_the_plain_difference_of_each_block_at_zero_offset() {
    let session = MotionEstimation::new(16, 16, 8, 0).expect("a valid session");
    let field = session.estimate(&Plane::filled(16, 16, 10), &Plane::filled(16, 16, 13)).expect("matching sizes");
    // Each 8 by 8 block differs by 3 at every pixel: 64 * 3 = 192.
    assert_eq!(field.vectors.len(), 4);
    assert!(field.vectors.iter().all(|v| *v == MotionVector { dx: 0, dy: 0, sad: 192 }));
}

#[test]
fn planes_of_another_size_are_refused() {
    let session = MotionEstimation::new(16, 16, 8, 2).expect("a valid session");
    assert!(session.estimate(&Plane::filled(8, 16, 0), &ramp()).is_none());
    assert!(session.estimate(&ramp(), &Plane::filled(16, 8, 0)).is_none());
}
