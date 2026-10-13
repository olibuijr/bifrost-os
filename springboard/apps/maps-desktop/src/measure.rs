//! Measure Distance: points clicked on the map, with the length of the line through them.

use crate::route::haversine_m;

/// The measured line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    points: Vec<(f64, f64)>,
}

impl Measure {
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    pub fn add(&mut self, lat: f64, lon: f64) {
        if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0 {
            self.points.push((lat, lon));
        }
    }

    /// Takes the last point away.
    pub fn undo(&mut self) -> Option<(f64, f64)> {
        self.points.pop()
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// The length of the line through the points, in metres.
    pub fn total_m(&self) -> f64 {
        self.points.windows(2).map(|w| haversine_m(w[0], w[1])).sum()
    }

    /// The length of the last segment, in metres.
    pub fn last_leg_m(&self) -> f64 {
        match self.points.as_slice() {
            [.., a, b] => haversine_m(*a, *b),
            _ => 0.0,
        }
    }
}

/// The length of any line, in metres (a route, a track).
pub fn path_length_m(points: &[(f64, f64)]) -> f64 {
    points.windows(2).map(|w| haversine_m(w[0], w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_total_is_the_sum_of_the_legs_and_undo_takes_the_last_point_away() {
        let mut m = Measure::default();
        assert_eq!(m.total_m(), 0.0);
        m.add(64.0, -21.0);
        assert_eq!(m.total_m(), 0.0);
        m.add(64.0, -20.0);
        m.add(65.0, -20.0);
        let a = haversine_m((64.0, -21.0), (64.0, -20.0));
        let b = haversine_m((64.0, -20.0), (65.0, -20.0));
        assert!((m.total_m() - (a + b)).abs() < 1e-6);
        assert!((m.last_leg_m() - b).abs() < 1e-6);
        assert_eq!(m.undo(), Some((65.0, -20.0)));
        assert!((m.total_m() - a).abs() < 1e-6);
        m.clear();
        assert!(m.points().is_empty());
        assert_eq!(m.undo(), None);
    }

    #[test]
    fn points_outside_the_world_are_refused() {
        let mut m = Measure::default();
        m.add(91.0, 0.0);
        m.add(f64::NAN, 0.0);
        assert!(m.points().is_empty());
        assert!((path_length_m(&[(0.0, 0.0), (0.0, 1.0)]) - 111_195.0).abs() < 200.0);
    }
}
