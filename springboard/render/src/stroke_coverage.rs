//! Anti-aliased stroke coverage for polylines, as used for vector map roads and outlines. A pixel
//! centre at distance `d` from the polyline gets coverage `clamp(half_width + 0.5 - d, 0, 1)`: full
//! inside the stroke, a one-pixel linear ramp at its edge, none outside. Pure std code.

/// Distance from `p` to the segment `a`-`b`. A degenerate segment gives the distance to `a`.
pub fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Distance from `p` to the nearest segment of the polyline. `None` for an empty point list; a
/// single point counts as a degenerate segment.
pub fn polyline_distance(p: (f32, f32), points: &[(f32, f32)]) -> Option<f32> {
    points
        .windows(2)
        .map(|w| distance_to_segment(p, w[0], w[1]))
        .reduce(f32::min)
        .or_else(|| points.first().map(|&a| distance_to_segment(p, a, a)))
}

/// Anti-aliased coverage of the stroke of `half_width` around `points`, in 0..=1. A negative or
/// NaN half-width, or an empty list, gives 0.
pub fn stroke_coverage(p: (f32, f32), points: &[(f32, f32)], half_width: f32) -> f32 {
    if !(half_width >= 0.0) {
        return 0.0;
    }
    match polyline_distance(p, points) {
        Some(d) => (half_width + 0.5 - d).clamp(0.0, 1.0),
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: [(f32, f32); 2] = [(0.0, 0.0), (10.0, 0.0)];

    #[test]
    fn centre_of_the_stroke_is_fully_covered() {
        assert_eq!(stroke_coverage((5.0, 0.0), &LINE, 1.0), 1.0);
    }

    #[test]
    fn the_edge_is_half_covered() {
        assert!((stroke_coverage((5.0, 1.0), &LINE, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn outside_the_stroke_is_empty() {
        assert_eq!(stroke_coverage((5.0, 2.0), &LINE, 1.0), 0.0);
    }

    #[test]
    fn past_the_end_uses_the_endpoint() {
        // (-1, 0) is 1 pixel from the start point, which is the stroke edge for half_width 0.5.
        assert!((stroke_coverage((-1.0, 0.0), &LINE, 0.5) - 0.0).abs() < 1e-6);
        assert!((stroke_coverage((-0.5, 0.0), &LINE, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_single_point_is_a_round_dot() {
        assert_eq!(stroke_coverage((3.0, 3.0), &[(3.0, 3.0)], 0.5), 1.0);
        assert_eq!(stroke_coverage((3.0, 3.0), &[], 0.5), 0.0);
    }
}
