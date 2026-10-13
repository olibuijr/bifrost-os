//! Geometry of the star rating drawn on a song row: the outline of one star and where a run of stars sits.
//! Pure functions; `Music` fills the polygon with `icons::fill_poly`.

use std::f32::consts::PI;

/// The ten corners of a five-point star with outer radius `r` centred on `c`, point up, in drawing order.
pub fn star_points(c: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let inner = r * 0.4;
    (0..10)
        .map(|k| {
            let radius = if k % 2 == 0 { r } else { inner };
            let angle = -PI / 2.0 + k as f32 * PI / 5.0;
            (c.0 + radius * angle.cos(), c.1 + radius * angle.sin())
        })
        .collect()
}

/// The x of the centre of each of `count` stars of radius `r`, laid left to right with `pitch` between centres so the
/// last star ends at `right`.
pub fn star_centres(right: f32, count: u8, r: f32, pitch: f32) -> Vec<f32> {
    (0..count).map(|k| right - r - f32::from(count - 1 - k) * pitch).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_has_ten_corners_alternating_between_two_radii() {
        let pts = star_points((100.0, 50.0), 10.0);
        assert_eq!(pts.len(), 10);
        let dist = |p: (f32, f32)| ((p.0 - 100.0).powi(2) + (p.1 - 50.0).powi(2)).sqrt();
        for (k, p) in pts.iter().enumerate() {
            let want = if k % 2 == 0 { 10.0 } else { 4.0 };
            assert!((dist(*p) - want).abs() < 1e-3, "corner {k}");
        }
        assert!((pts[0].0 - 100.0).abs() < 1e-3 && (pts[0].1 - 40.0).abs() < 1e-3, "the first point is straight up");
    }

    #[test]
    fn stars_end_at_the_right_edge_and_sit_a_pitch_apart() {
        let xs = star_centres(100.0, 3, 5.0, 11.0);
        assert_eq!(xs, [73.0, 84.0, 95.0]);
        assert_eq!(star_centres(100.0, 1, 5.0, 11.0), [95.0]);
        assert!(star_centres(100.0, 0, 5.0, 11.0).is_empty());
    }
}
