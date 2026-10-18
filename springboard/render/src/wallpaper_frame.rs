//! One frame of an animated wallpaper: a diagonal gradient (top-left to bottom-right) whose two
//! colour ramps cross-fade back and forth with a ping-pong phase. Built on `gradient`; pure, so the
//! compositor or a wallpaper app calls it once per frame.

use crate::colorfx::Rgb;
use crate::gradient::{cross_fade, linear_t, ping_pong_phase, Ramp};

/// The `width` x `height` pixels of one frame, row-major, at `time` seconds. The phase loops every
/// `period` seconds (see `ping_pong_phase`). Empty when either size is zero.
pub fn render_frame(
    width: usize,
    height: usize,
    from: &Ramp,
    to: &Ramp,
    time: f32,
    period: f32,
) -> Vec<Rgb> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let phase = ping_pong_phase(time, period);
    // The gradient runs from the top-left pixel centre to the bottom-right one.
    let end = ((width.max(2) - 1) as f32, (height.max(2) - 1) as f32);
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let t = linear_t((0.0, 0.0), end, x as f32, y as f32);
            out.push(cross_fade(from, to, phase, t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Rgb, b: Rgb) -> bool {
        (a.r - b.r).abs() < 1e-5 && (a.g - b.g).abs() < 1e-5 && (a.b - b.b).abs() < 1e-5
    }

    fn ramps() -> (Ramp, Ramp) {
        let from = Ramp::new(&[
            (0.0, Rgb::new(0.0, 0.0, 0.0)),
            (1.0, Rgb::new(1.0, 1.0, 1.0)),
        ])
        .unwrap();
        let to = Ramp::new(&[
            (0.0, Rgb::new(1.0, 0.0, 0.0)),
            (1.0, Rgb::new(0.0, 0.0, 1.0)),
        ])
        .unwrap();
        (from, to)
    }

    #[test]
    fn empty_size_gives_no_pixels() {
        let (from, to) = ramps();
        assert!(render_frame(0, 4, &from, &to, 0.0, 10.0).is_empty());
        assert!(render_frame(4, 0, &from, &to, 0.0, 10.0).is_empty());
    }

    #[test]
    fn start_of_the_loop_shows_the_first_ramp() {
        let (from, to) = ramps();
        let frame = render_frame(3, 2, &from, &to, 0.0, 10.0);
        assert_eq!(frame.len(), 6);
        assert!(near(frame[0], Rgb::new(0.0, 0.0, 0.0)));
    }

    #[test]
    fn half_period_shows_the_second_ramp() {
        let (from, to) = ramps();
        let frame = render_frame(3, 2, &from, &to, 5.0, 10.0);
        assert!(near(frame[0], Rgb::new(1.0, 0.0, 0.0)));
    }
}
