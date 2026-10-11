//! Linear and radial gradients as sampled colour ramps. A `Ramp` maps an offset in 0..=1 to a colour
//! by interpolating between stops; `linear_t` and `radial_t` turn a point into an offset. Offsets
//! outside the ramp take the colour of the nearest end stop. Pure: the renderer or an app evaluates
//! it per pixel.

use crate::colorfx::Rgb;

/// A colour ramp: stops sorted by offset, each offset in 0..=1.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    stops: Vec<(f32, Rgb)>,
}

impl Ramp {
    /// A ramp from at least one `(offset, colour)` stop. Offsets must lie in 0..=1 and must not
    /// decrease. Two stops at the same offset make a hard edge. `None` otherwise.
    pub fn new(stops: &[(f32, Rgb)]) -> Option<Self> {
        if stops.is_empty() {
            return None;
        }
        let mut prev = 0.0f32;
        for &(t, _) in stops {
            if !(0.0..=1.0).contains(&t) || t < prev {
                return None;
            }
            prev = t;
        }
        Some(Ramp { stops: stops.to_vec() })
    }

    /// The colour at offset `t`. Offsets before the first stop or after the last take those colours;
    /// NaN is treated as 0.
    pub fn color_at(&self, t: f32) -> Rgb {
        let t = if t.is_nan() { 0.0 } else { t };
        let first = self.stops[0];
        let last = self.stops[self.stops.len() - 1];
        if t <= first.0 {
            return first.1;
        }
        if t >= last.0 {
            return last.1;
        }
        for pair in self.stops.windows(2) {
            let (t0, c0) = pair[0];
            let (t1, c1) = pair[1];
            if t <= t1 {
                let span = t1 - t0;
                return if span <= 0.0 { c1 } else { c0.mix(c1, (t - t0) / span) };
            }
        }
        last.1
    }
}

/// The offset of the point (x, y) along the line from `from` to `to`: 0 at `from`, 1 at `to`, and
/// the projection onto the line beyond them. A zero-length line gives 0.
pub fn linear_t(from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= 0.0 {
        return 0.0;
    }
    ((x - from.0) * dx + (y - from.1) * dy) / len2
}

/// The offset of the point (x, y) from `center` in units of `radius`: 0 at the centre, 1 on the
/// circle. A radius of zero or less gives 1.
pub fn radial_t(center: (f32, f32), radius: f32, x: f32, y: f32) -> f32 {
    if radius <= 0.0 {
        return 1.0;
    }
    let (dx, dy) = (x - center.0, y - center.1);
    (dx * dx + dy * dy).sqrt() / radius
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);
    const WHITE: Rgb = Rgb::new(1.0, 1.0, 1.0);
    const RED: Rgb = Rgb::new(1.0, 0.0, 0.0);

    #[test]
    fn new_rejects_empty_unsorted_and_out_of_range_stops() {
        assert!(Ramp::new(&[]).is_none());
        assert!(Ramp::new(&[(0.6, BLACK), (0.4, WHITE)]).is_none());
        assert!(Ramp::new(&[(0.0, BLACK), (1.5, WHITE)]).is_none());
        assert!(Ramp::new(&[(f32::NAN, BLACK)]).is_none());
        assert!(Ramp::new(&[(0.5, BLACK)]).is_some());
    }

    #[test]
    fn color_at_clamps_to_the_end_stops() {
        let ramp = Ramp::new(&[(0.2, BLACK), (0.8, WHITE)]).unwrap();
        assert_eq!(ramp.color_at(-1.0), BLACK);
        assert_eq!(ramp.color_at(0.0), BLACK);
        assert_eq!(ramp.color_at(2.0), WHITE);
        assert_eq!(ramp.color_at(f32::NAN), BLACK);
    }

    #[test]
    fn color_at_interpolates_between_two_stops() {
        let ramp = Ramp::new(&[(0.0, BLACK), (1.0, WHITE)]).unwrap();
        let mid = ramp.color_at(0.5);
        assert!(close(mid.r, 0.5) && close(mid.g, 0.5) && close(mid.b, 0.5));
    }

    #[test]
    fn color_at_walks_through_three_stops() {
        let ramp = Ramp::new(&[(0.0, BLACK), (0.5, RED), (1.0, WHITE)]).unwrap();
        let q = ramp.color_at(0.25);
        assert!(close(q.r, 0.5) && close(q.g, 0.0) && close(q.b, 0.0));
        let h = ramp.color_at(0.75);
        assert!(close(h.r, 1.0) && close(h.g, 0.5) && close(h.b, 0.5));
    }

    #[test]
    fn equal_offsets_make_a_hard_edge() {
        let ramp = Ramp::new(&[(0.0, BLACK), (0.5, BLACK), (0.5, WHITE), (1.0, WHITE)]).unwrap();
        assert_eq!(ramp.color_at(0.4), BLACK);
        assert_eq!(ramp.color_at(0.6), WHITE);
    }

    #[test]
    fn linear_offset_projects_onto_the_line() {
        assert!(close(linear_t((0.0, 0.0), (10.0, 0.0), 5.0, 7.0), 0.5));
        assert!(close(linear_t((0.0, 0.0), (10.0, 0.0), -10.0, 0.0), -1.0));
        assert!(close(linear_t((0.0, 0.0), (10.0, 10.0), 10.0, 0.0), 0.5));
        assert_eq!(linear_t((3.0, 3.0), (3.0, 3.0), 9.0, 9.0), 0.0);
    }

    #[test]
    fn radial_offset_is_distance_over_radius() {
        assert!(close(radial_t((0.0, 0.0), 10.0, 3.0, 4.0), 0.5));
        assert!(close(radial_t((0.0, 0.0), 10.0, 0.0, 0.0), 0.0));
        assert!(close(radial_t((0.0, 0.0), 5.0, 0.0, 10.0), 2.0));
        assert_eq!(radial_t((0.0, 0.0), 0.0, 1.0, 1.0), 1.0);
    }
}
