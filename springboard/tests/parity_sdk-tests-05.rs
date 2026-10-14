//! Parity sdk-tests-05: headless tests for the markup editor in `springboard::shell::markup`. The editor draws into
//! in-memory images and the tests feed it touch events and then assert the pixels. No display, shell or device is
//! involved.
//!
//! The picture of a 300 by 500 point editor sits between the bar and the tray at about x 65 to 235 and y 92 to 376.
//! The tool buttons, the colour swatches and the rest of the layout are read from the editor's own geometry.

use springboard::font::Font;
use springboard::gfx::{pack, Image, Surface};
use springboard::shell::markup::{Markup, Outcome, Tool, COLORS};
use springboard::shell::ui::{Ui, BLACK, CELL};
use springboard::shell::Event;
use std::path::PathBuf;

const W: usize = 300;
const H: usize = 500;
const PAPER: u32 = pack(200, 200, 200, 255);

/// Centres of the four tool buttons in the tray, in editor points.
const PEN_AT: (f32, f32) = (46.0, 418.0);
const MARKER_AT: (f32, f32) = (115.0, 418.0);
const PENCIL_AT: (f32, f32) = (184.0, 418.0);
const ERASER_AT: (f32, f32) = (253.0, 418.0);

/// Centre of colour swatch `i` in the colour row.
fn swatch_at(i: usize) -> (f32, f32) {
    (36.0 + 38.0 * i as f32, 470.0)
}

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

/// An editor over a blank picture, with the untouched pixels kept for comparison.
fn editor() -> (Markup, Vec<u32>) {
    let base = Image { w: W, h: H, px: vec![PAPER; W * H] };
    (Markup::new(base, PathBuf::from("/markup-test.png"), 1.0), vec![PAPER; W * H])
}

fn tap(m: &mut Markup, ui: &Ui, at: (f32, f32)) -> Option<Outcome> {
    m.touch(ui, Event::Down(at.0, at.1), 0.0);
    m.touch(ui, Event::Up(at.0, at.1), 0.0)
}

/// Presses at `from`, drags in twenty steps to `to`, and releases there.
fn stroke(m: &mut Markup, ui: &Ui, from: (f32, f32), to: (f32, f32)) {
    m.touch(ui, Event::Down(from.0, from.1), 0.0);
    for k in 1..=20 {
        let f = k as f32 / 20.0;
        m.touch(ui, Event::Move(from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f), 0.0);
    }
    m.touch(ui, Event::Up(to.0, to.1), 0.0);
}

/// Pixels of the marked-up image that differ from the untouched picture, as (index, colour).
fn changed(m: &Markup, base: &[u32]) -> Vec<(usize, u32)> {
    m.image().px.iter().enumerate().filter(|(i, p)| **p != base[*i]).map(|(i, p)| (i, *p)).collect()
}

/// Whether two packed colours match within two steps per channel (premultiplied arithmetic may round).
fn near(a: u32, b: u32) -> bool {
    (0..4).all(|c| ((a >> (8 * c)) & 255).abs_diff((b >> (8 * c)) & 255) <= 2)
}

const STROKE_AT_200: ((f32, f32), (f32, f32)) = ((100.0, 200.0), (200.0, 200.0));

#[test]
fn a_new_editor_starts_with_the_pen_and_nothing_to_undo() {
    let (m, base) = editor();
    assert_eq!(m.tool(), Tool::Pen);
    assert!(!m.can_undo());
    assert_eq!(m.image().px, base, "the marked-up image starts as the picture");
}

#[test]
fn headless_a_pen_stroke_inks_the_picture_in_the_selected_colour() {
    let ui = ui();
    let (mut m, base) = editor();
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let ink = changed(&m, &base);
    assert!(!ink.is_empty(), "the stroke changes pixels");
    assert!(m.can_undo());
    assert!(ink.iter().all(|(_, p)| near(*p, COLORS[2])), "the default colour is the third swatch");
}

#[test]
fn headless_a_stroke_outside_the_picture_draws_nothing() {
    let ui = ui();
    let (mut m, base) = editor();
    stroke(&mut m, &ui, (5.0, 300.0), (30.0, 300.0));
    assert!(changed(&m, &base).is_empty(), "the margin beside the picture takes no ink");
    assert!(!m.can_undo());
}

#[test]
fn headless_a_tap_on_the_picture_leaves_a_dot_of_ink() {
    let ui = ui();
    let (mut m, base) = editor();
    tap(&mut m, &ui, (150.0, 200.0));
    assert!(!changed(&m, &base).is_empty(), "a press without a drag stamps one dot");
}

#[test]
fn headless_a_touch_on_the_picture_finishes_nothing() {
    let ui = ui();
    let (mut m, _) = editor();
    assert_eq!(tap(&mut m, &ui, (150.0, 200.0)), None, "only Cancel and Done finish the editor");
}

#[test]
fn headless_the_marker_draws_a_wider_band_than_the_pen() {
    let ui = ui();
    let (mut pen, base) = editor();
    stroke(&mut pen, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let (mut marker, base_m) = editor();
    tap(&mut marker, &ui, MARKER_AT);
    assert_eq!(marker.tool(), Tool::Marker);
    stroke(&mut marker, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let (p, k) = (changed(&pen, &base).len(), changed(&marker, &base_m).len());
    assert!(k > 3 * p, "a 14 point marker covers several times the rows of a 3 point pen: pen {p}, marker {k}");
}

#[test]
fn headless_the_marker_is_translucent_so_its_ink_is_not_the_full_colour() {
    let ui = ui();
    let (mut m, base) = editor();
    tap(&mut m, &ui, MARKER_AT);
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let ink = changed(&m, &base);
    assert!(!ink.is_empty());
    assert!(ink.iter().all(|(_, p)| !near(*p, COLORS[2])), "the marker lets the picture show through");
}

#[test]
fn headless_the_pencil_is_thinner_than_the_marker() {
    let ui = ui();
    let (mut pencil, base) = editor();
    tap(&mut pencil, &ui, PENCIL_AT);
    assert_eq!(pencil.tool(), Tool::Pencil);
    stroke(&mut pencil, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let (mut marker, base_m) = editor();
    tap(&mut marker, &ui, MARKER_AT);
    stroke(&mut marker, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    let (p, k) = (changed(&pencil, &base).len(), changed(&marker, &base_m).len());
    assert!(!changed(&pencil, &base).is_empty());
    assert!(p < k, "a 2 point pencil covers fewer rows than a 14 point marker: pencil {p}, marker {k}");
}

#[test]
fn headless_the_eraser_restores_the_picture_under_a_stroke() {
    let ui = ui();
    let (mut m, base) = editor();
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    assert!(!changed(&m, &base).is_empty());
    tap(&mut m, &ui, ERASER_AT);
    assert_eq!(m.tool(), Tool::Eraser);
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    assert!(changed(&m, &base).is_empty(), "the eraser copies the picture back over the ink");
}

#[test]
fn headless_choosing_a_swatch_after_the_eraser_returns_to_the_pen() {
    let ui = ui();
    let (mut m, base) = editor();
    tap(&mut m, &ui, ERASER_AT);
    assert_eq!(m.tool(), Tool::Eraser);
    tap(&mut m, &ui, swatch_at(6));
    assert_eq!(m.tool(), Tool::Pen, "a colour ends the eraser");
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    assert!(changed(&m, &base).iter().all(|(_, p)| near(*p, COLORS[6])), "the next stroke uses the chosen colour");
}

#[test]
fn headless_a_tapped_swatch_sets_the_ink_for_later_strokes() {
    let ui = ui();
    let (mut m, base) = editor();
    tap(&mut m, &ui, swatch_at(3));
    stroke(&mut m, &ui, STROKE_AT_200.0, STROKE_AT_200.1);
    assert!(changed(&m, &base).iter().all(|(_, p)| near(*p, COLORS[3])));
}

#[test]
fn headless_undo_removes_only_the_last_stroke() {
    let ui = ui();
    let (mut m, base) = editor();
    stroke(&mut m, &ui, (100.0, 150.0), (200.0, 150.0));
    stroke(&mut m, &ui, (100.0, 260.0), (200.0, 260.0));
    let rows = |v: &[(usize, u32)]| -> (bool, bool) { (v.iter().any(|(i, _)| i / W < 150), v.iter().any(|(i, _)| i / W > 250)) };
    assert_eq!(rows(&changed(&m, &base)), (true, true), "both strokes are inked");
    m.undo();
    assert_eq!(rows(&changed(&m, &base)), (true, false), "the first stroke stays and the second is gone");
    assert!(m.can_undo());
    m.undo();
    assert!(changed(&m, &base).is_empty(), "the second undo restores the picture");
    assert!(!m.can_undo());
}

#[test]
fn headless_undo_on_an_empty_editor_keeps_the_picture() {
    let (mut m, base) = editor();
    m.undo();
    assert_eq!(m.image().px, base);
    assert!(!m.can_undo());
}

#[test]
fn headless_the_frame_draws_a_black_window_with_the_tray_below_the_picture() {
    let ui = ui();
    let (m, _) = editor();
    let mut px = vec![0u32; W * H];
    m.draw(&mut Surface { w: W, h: H, stride: W, px: &mut px, y0: 0 }, &ui);
    assert_eq!(px[0], BLACK, "the frame starts black");
    assert_eq!(px[(H - 1) * W + 1], CELL, "the tray fills the bottom edge");
}
