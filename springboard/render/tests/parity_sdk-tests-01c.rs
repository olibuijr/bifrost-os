//! Gradient ramps and offsets of `bifrost_render::gradient`, rasterised headless into an in-memory buffer: each pixel takes
//! the ramp colour at its centre's offset, and the buffer is checked pixel by pixel. No window and no display.

use bifrost_render::colorfx::Rgb;
use bifrost_render::gradient::{linear_t, radial_t, Ramp};

const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);
const WHITE: Rgb = Rgb::new(1.0, 1.0, 1.0);

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// The red channel of each pixel of a `w`-pixel row of an axial gradient from x = 0 to x = `w`.
fn axial_row(ramp: &Ramp, w: usize) -> Vec<f32> {
    (0..w).map(|x| ramp.color_at(linear_t((0.0, 0.0), (w as f32, 0.0), x as f32 + 0.5, 0.0)).r).collect()
}

#[test]
fn a_headless_axial_row_runs_from_the_first_colour_to_the_last() {
    let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
    let row = axial_row(&ramp, 4);
    assert!(close(row[0], 0.125) && close(row[1], 0.375) && close(row[2], 0.625) && close(row[3], 0.875), "{row:?}");
}

#[test]
fn a_headless_hard_edge_puts_the_colour_change_at_the_stop() {
    let ramp = Ramp::new(&[(0.0, BLACK), (0.5, BLACK), (0.5, WHITE), (1.0, WHITE)]).unwrap();
    let row = axial_row(&ramp, 4);
    assert_eq!(row[0], 0.0);
    assert_eq!(row[1], 0.0);
    assert!(close(row[2], 1.0) && close(row[3], 1.0), "{row:?}");
}

#[test]
fn a_headless_radial_buffer_grows_from_the_centre_and_holds_past_the_radius() {
    let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
    let mut buf = [0.0f32; 16];
    for (i, px) in buf.iter_mut().enumerate() {
        let (x, y) = ((i % 4) as f32 + 0.5, (i / 4) as f32 + 0.5);
        *px = ramp.color_at(radial_t((2.0, 2.0), 2.0, x, y)).r;
    }
    assert_eq!(buf[0], 1.0, "the corner is past the radius, so the last colour holds");
    assert!(close(buf[5], 0.353_553), "the pixel diagonal to the centre is 0.707 / 2 of the way: {}", buf[5]);
    assert!(close(buf[1], 0.790_569), "the pixel at (1.5, 0.5) is hypot(0.5, 1.5) / 2 from the centre: {}", buf[1]);
    assert!(buf[5] < buf[1] && buf[1] < buf[0], "the colour grows with the distance");
}

#[test]
fn a_zero_length_axis_takes_the_first_colour_everywhere() {
    let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
    assert_eq!(linear_t((1.0, 1.0), (1.0, 1.0), 5.0, 5.0), 0.0);
    assert_eq!(ramp.color_at(linear_t((1.0, 1.0), (1.0, 1.0), 5.0, 5.0)), BLACK);
    assert_eq!(radial_t((0.0, 0.0), 0.0, 1.0, 1.0), 1.0, "a radius of zero is the edge");
}

#[test]
fn a_ramp_refuses_unsorted_stops_and_nan_reads_the_first_colour() {
    assert!(Ramp::new(&[(0.6, BLACK), (0.4, WHITE)]).is_none());
    assert!(Ramp::new(&[]).is_none());
    assert!(Ramp::new(&[(f32::NAN, BLACK)]).is_none());
    let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
    assert_eq!(ramp.color_at(f32::NAN), BLACK);
}
