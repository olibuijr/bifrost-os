//! Parity sdk-complete-18: headless tests for the pointer model. The cursor stack and the layered cursor art are
//! composed into memory bitmaps; nothing here draws to a display or moves a pointer.

use bifrost_ui::cursor::{Cursors, Kind};
use bifrost_ui::cursor_art::{art, PointerColors, SIZE};

#[test]
fn the_pointer_shows_the_top_of_the_stack_and_falls_back_to_the_arrow() {
    let mut c = Cursors::new();
    assert_eq!(c.current(), Kind::Arrow);
    c.push(Kind::IBeam);
    c.push(Kind::Move);
    assert_eq!(c.current(), Kind::Move);
    assert_eq!(c.pop(), Some(Kind::Move));
    assert_eq!(c.current(), Kind::IBeam);
    c.pop();
    assert_eq!(c.current(), Kind::Arrow, "an empty stack shows the arrow");
    assert_eq!(c.pop(), None, "popping an empty stack does nothing");
}

#[test]
fn set_changes_the_shape_on_top_without_growing_the_stack() {
    let mut c = Cursors::new();
    c.push(Kind::Crosshair);
    c.set(Kind::ZoomIn);
    assert_eq!(c.depth(), 1);
    assert_eq!(c.current(), Kind::ZoomIn);
}

#[test]
fn every_hide_needs_its_own_unhide_before_the_pointer_shows() {
    let mut c = Cursors::new();
    c.hide();
    c.hide();
    c.unhide();
    assert!(!c.is_visible(), "one hide is still in force");
    c.unhide();
    assert!(c.is_visible());
    c.unhide();
    assert!(c.is_visible(), "an extra unhide does not go negative");
}

#[test]
fn a_cursor_hidden_until_the_mouse_moves_shows_on_the_next_move() {
    let mut c = Cursors::new();
    c.set_hidden_until_mouse_moves(true);
    assert!(!c.is_visible());
    c.mouse_moved();
    assert!(c.is_visible());
}

#[test]
fn the_composed_bitmap_has_one_rgba_pixel_per_mask_pixel_at_each_scale() {
    let one = art(Kind::Arrow, 1).compose_colors(PointerColors::default());
    assert_eq!(one.len(), (SIZE * SIZE * 4) as usize);
    let two = art(Kind::Arrow, 2).compose_colors(PointerColors::default());
    assert_eq!(two.len(), (SIZE * 2 * SIZE * 2 * 4) as usize);
}

#[test]
fn every_shape_composes_to_a_bitmap_of_its_own_size() {
    for &kind in Kind::ALL {
        for scale in [1, 2] {
            let a = art(kind, scale);
            let bitmap = a.compose_colors(PointerColors::default());
            assert_eq!(bitmap.len(), (a.size * a.size * 4) as usize, "{kind:?} at scale {scale}");
            assert!(bitmap.chunks_exact(4).any(|px| px[3] > 0), "{kind:?} at scale {scale} paints something");
        }
    }
}

#[test]
fn scales_outside_one_and_two_are_clamped_into_that_range() {
    assert_eq!(art(Kind::Move, 0).size, SIZE);
    assert_eq!(art(Kind::Move, 7).size, SIZE * 2);
}

#[test]
fn recolouring_the_pointer_changes_the_composed_fill_colour() {
    let a = art(Kind::Arrow, 1);
    let white = PointerColors { outline: [255, 255, 255, 255], fill: [255, 255, 255, 255] };
    let black = PointerColors::default();
    let w = a.compose_colors(white);
    let b = a.compose_colors(black);
    assert_ne!(w, b);
    let solid = a.fill.iter().position(|&v| v == 255).expect("a solid fill pixel");
    assert_eq!(w[solid * 4..solid * 4 + 4], [255, 255, 255, 255]);
    assert_eq!(b[solid * 4..solid * 4 + 4], [0, 0, 0, 255]);
}
