//! Parity sdk-complete-15: headless tests of the drawing model's stylus rendering. Pressure scales the stroke width,
//! and each point's opacity scales its paint. Everything is in memory: no display, no device and no clock.

use bifrost_ui::drawing::{pressure_width, Drawing, Ink, InkKind, Stroke, StrokePoint};
use bifrost_ui::Rect;

const RED: u32 = 0xffff_0000;

fn ink(width: f32) -> Ink {
    Ink { kind: InkKind::Pen, color: RED, width }
}

fn point(x: f32, y: f32, force: f32, opacity: f32) -> StrokePoint {
    StrokePoint { force, opacity, ..StrokePoint::at(x, y) }
}

/// A horizontal stroke from x = 0 to x = 10 at height `y`, with every point at `force` and `opacity`.
fn flat_stroke(y: f32, width: f32, force: f32, opacity: f32) -> Stroke {
    Stroke { ink: ink(width), points: vec![point(0.0, y, force, opacity), point(10.0, y, force, opacity)] }
}

/// Renders one stroke over a 10 by 10 point area at scale one.
fn render(stroke: Stroke) -> Vec<u32> {
    let drawing = Drawing::from_strokes(vec![stroke]);
    drawing.image(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 1.0).pixels
}

fn alpha(pixels: &[u32], x: usize, y: usize) -> u32 {
    pixels[y * 10 + x] >> 24
}

#[test]
fn headless_a_point_without_a_force_reading_keeps_the_full_width() {
    assert_eq!(pressure_width(4.0, 0.0), 4.0);
}

#[test]
fn headless_a_full_force_draws_the_full_width() {
    assert_eq!(pressure_width(4.0, 1.0), 4.0);
    assert_eq!(pressure_width(4.0, 3.0), 4.0, "forces above one are clamped to the full width");
}

#[test]
fn headless_a_half_force_draws_five_eighths_of_the_width() {
    assert!((pressure_width(4.0, 0.5) - 2.5).abs() < 1e-6);
}

#[test]
fn headless_a_light_touch_draws_a_quarter_of_the_width() {
    assert!((pressure_width(8.0, 0.0001) - 2.0).abs() < 1e-3);
}

#[test]
fn headless_a_firm_stroke_covers_pixels_a_light_stroke_leaves_empty() {
    // The line runs along y = 5.5; the row centred on y = 7.5 is two points away from it. A full-width stroke of
    // width 4 reaches that row, a half-force stroke (width 2.5) does not.
    let firm = render(flat_stroke(5.5, 4.0, 1.0, 1.0));
    let light = render(flat_stroke(5.5, 4.0, 0.5, 1.0));
    assert!(alpha(&firm, 5, 7) > 0, "the firm stroke reaches two points from its centre line");
    assert_eq!(alpha(&light, 5, 7), 0, "the light stroke stays within 1.25 points of its centre line");
    assert_eq!(alpha(&light, 5, 5), 255, "the light stroke is still solid on its centre line");
}

#[test]
fn headless_a_fully_transparent_stroke_paints_no_pixels() {
    let pixels = render(flat_stroke(5.5, 4.0, 0.0, 0.0));
    assert!(pixels.iter().all(|&p| p == 0), "opacity zero leaves the raster empty");
}

#[test]
fn headless_a_half_opaque_stroke_paints_half_the_alpha() {
    let pixels = render(flat_stroke(5.5, 4.0, 0.0, 0.5));
    let a = alpha(&pixels, 5, 5);
    assert!((125..=131).contains(&a), "alpha {a} is about half of 255");
}

#[test]
fn headless_point_opacity_fades_a_stroke_along_its_length() {
    let stroke = Stroke {
        ink: ink(4.0),
        points: vec![point(0.0, 5.5, 0.0, 1.0), point(4.0, 5.5, 0.0, 1.0), point(6.0, 5.5, 0.0, 0.0), point(10.0, 5.5, 0.0, 0.0)],
    };
    let pixels = render(stroke);
    assert_eq!(alpha(&pixels, 1, 5), 255, "the opaque start stays solid");
    assert_eq!(alpha(&pixels, 9, 5), 0, "the transparent end paints nothing");
}

#[test]
fn headless_a_single_point_stroke_paints_a_dot_at_its_force_width() {
    let stroke = Stroke { ink: ink(4.0), points: vec![point(5.5, 5.5, 1.0, 1.0)] };
    let pixels = render(stroke);
    assert_eq!(alpha(&pixels, 5, 5), 255, "the dot is solid at its centre");
    assert!(alpha(&pixels, 5, 7) > 0, "a full-force dot of width 4 reaches two points out");
}
