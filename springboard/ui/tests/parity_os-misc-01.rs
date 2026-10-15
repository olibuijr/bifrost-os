//! Parity os-misc-01: the vertical i-beam pointer shape in the cursor model and its layered art.
//! Pure model tests: no display, no device, no compositor.

use bifrost_ui::cursor::Kind;
use bifrost_ui::cursor_art::{art, SIZE};

#[test]
fn the_vertical_i_beam_is_a_named_shape_with_its_own_service_name() {
    assert!(Kind::ALL.contains(&Kind::VerticalIBeam));
    assert_eq!(Kind::VerticalIBeam.name(), "vertical_i_beam");
    assert_eq!(Kind::from_name("vertical_i_beam"), Some(Kind::VerticalIBeam));
    assert_ne!(Kind::VerticalIBeam, Kind::IBeam, "the vertical variant is a separate kind");
}

#[test]
fn the_vertical_i_beam_uses_the_vertical_text_protocol_shape() {
    assert_eq!(Kind::VerticalIBeam.shape(), "vertical-text");
    assert_eq!(Kind::IBeam.shape(), "text", "the horizontal i-beam keeps its shape");
}

#[test]
fn the_vertical_i_beam_art_has_a_horizontal_bar_and_no_vertical_stem() {
    let n = SIZE as usize;
    let vertical = art(Kind::VerticalIBeam, 1);
    let ibeam = art(Kind::IBeam, 1);
    let centre_column = n / 2;
    // The horizontal i-beam has a vertical stem through the centre column; the vertical variant does not.
    assert!(ibeam.fill[8 * n + centre_column] > 0, "the i-beam stem covers the centre column above the middle");
    assert_eq!(vertical.fill[8 * n + centre_column], 0, "no vertical stem above the middle");
    // The horizontal bar runs through the middle row.
    assert!(vertical.fill[12 * n + centre_column] > 0, "the bar covers the centre pixel");
    assert!(vertical.fill[12 * n + 4] > 0, "the bar reaches the left side of the bar");
}

#[test]
fn the_vertical_i_beam_art_is_distinct_from_every_other_shape() {
    let fill = art(Kind::VerticalIBeam, 1).fill;
    for &other in Kind::ALL.iter().filter(|&&k| k != Kind::VerticalIBeam) {
        assert_ne!(art(other, 1).fill, fill, "{other:?} would look the same as the vertical i-beam");
    }
}
