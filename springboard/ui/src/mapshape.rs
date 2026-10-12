//! Map overlays: polylines, polygons and circles, with length, area, containment and hit testing.
//!
//! The geometry is pure. Shapes hold coordinates in degrees and report sizes in metres. Lengths of polylines and the
//! radius of circles use great-circle distance. Polygon areas are measured on the Web Mercator projection, so they
//! are close to true for small shapes and stretch away from the equator. Nothing here draws a shape: a renderer turns
//! the vertices into pixels with a camera from the `mapview` module.
//!
//! Shapes do not wrap the antimeridian. A circle's area is the exact area of its spherical cap.

use std::f64::consts::PI;

use crate::geo::{metres_per_point_at_latitude, Coordinate, MapPoint, EARTH_RADIUS_M};

/// Total great-circle length of the segments through `points`, in metres. Fewer than two points give zero.
pub fn polyline_length_m(points: &[Coordinate]) -> f64 {
    points.windows(2).map(|w| w[0].distance_m(w[1])).sum::<f64>()
}

/// Distance in metres from `c` to the nearest point of the segment from `a` to `b`. The distance is measured on the
/// projection, scaled by the metres per point at `c`'s latitude.
pub fn distance_to_segment_m(a: Coordinate, b: Coordinate, c: Coordinate) -> f64 {
    let pa = MapPoint::from_coordinate(a);
    let pb = MapPoint::from_coordinate(b);
    let pc = MapPoint::from_coordinate(c);
    let (dx, dy) = (pb.x - pa.x, pb.y - pa.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((pc.x - pa.x) * dx + (pc.y - pa.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (nx, ny) = (pa.x + t * dx, pa.y + t * dy);
    let (ex, ey) = (nx - pc.x, ny - pc.y);
    (ex * ex + ey * ey).sqrt() * metres_per_point_at_latitude(c.latitude)
}

/// Distance in metres from `c` to the nearest part of a polyline. A single point is measured directly. `None` when
/// there are no points.
pub fn distance_to_polyline_m(points: &[Coordinate], c: Coordinate) -> Option<f64> {
    match points {
        [] => None,
        [only] => Some(only.distance_m(c)),
        _ => Some(
            points
                .windows(2)
                .map(|w| distance_to_segment_m(w[0], w[1], c))
                .fold(f64::INFINITY, f64::min),
        ),
    }
}

/// True when `c` is within `tolerance_m` metres of the polyline, for tapping a line on the map.
pub fn polyline_hit(points: &[Coordinate], c: Coordinate, tolerance_m: f64) -> bool {
    distance_to_polyline_m(points, c).map_or(false, |d| d <= tolerance_m)
}

/// A closed ring of coordinates. The last vertex joins back to the first.
#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    pub ring: Vec<Coordinate>,
}

impl Polygon {
    /// Builds a polygon from a ring. A closing vertex that repeats the first one is dropped. `None` with fewer than
    /// three vertices left.
    pub fn new(ring: Vec<Coordinate>) -> Option<Self> {
        let mut pts = ring;
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        (pts.len() >= 3).then_some(Polygon { ring: pts })
    }

    /// True when `c` is inside the ring, by the even-odd rule on latitude and longitude. A point exactly on an edge
    /// may come out either way.
    pub fn contains(&self, c: Coordinate) -> bool {
        let n = self.ring.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (self.ring[i], self.ring[j]);
            if (a.latitude > c.latitude) != (b.latitude > c.latitude)
                && c.longitude < (b.longitude - a.longitude) * (c.latitude - a.latitude) / (b.latitude - a.latitude)
                    + a.longitude
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Area in square metres: the shoelace area of the projected ring, scaled by the metres per point at the mean
    /// latitude of the vertices, squared. Zero for a ring with fewer than three vertices.
    pub fn area_m2(&self) -> f64 {
        let n = self.ring.len();
        if n < 3 {
            return 0.0;
        }
        let pts: Vec<MapPoint> = self.ring.iter().map(|c| MapPoint::from_coordinate(*c)).collect();
        let twice_area: f64 = (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                a.x * b.y - b.x * a.y
            })
            .sum::<f64>();
        let mean_lat = self.ring.iter().map(|c| c.latitude).sum::<f64>() / n as f64;
        let mpp = metres_per_point_at_latitude(mean_lat);
        twice_area.abs() / 2.0 * mpp * mpp
    }
}

/// A circle of `radius_m` metres around `center`, measured along the sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center: Coordinate,
    pub radius_m: f64,
}

impl Circle {
    /// `None` when the radius is not a positive finite number.
    pub fn new(center: Coordinate, radius_m: f64) -> Option<Self> {
        (radius_m.is_finite() && radius_m > 0.0).then_some(Circle { center, radius_m })
    }

    /// True when `c` is within the radius, the boundary included.
    pub fn contains(&self, c: Coordinate) -> bool {
        self.center.distance_m(c) <= self.radius_m
    }

    /// Exact area of the spherical cap the circle covers, in square metres: 2 pi R^2 (1 - cos(r / R)).
    pub fn area_m2(&self) -> f64 {
        let angle = self.radius_m / EARTH_RADIUS_M;
        2.0 * PI * EARTH_RADIUS_M * EARTH_RADIUS_M * (1.0 - angle.cos())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    #[test]
    fn polyline_length_adds_the_great_circle_segments() {
        // One degree of arc on the mean-radius sphere is R * pi / 180, about 111 195 m. Two such legs make about 222 390 m.
        let legs = [c(0.0, 0.0), c(0.0, 1.0), c(1.0, 1.0)];
        let len = polyline_length_m(&legs);
        assert!((222_300.0..222_500.0).contains(&len), "length was {len}");
        assert_eq!(polyline_length_m(&legs[..1]), 0.0);
        assert_eq!(polyline_length_m(&[]), 0.0);
    }

    #[test]
    fn polyline_hit_test_uses_the_distance_to_the_nearest_segment() {
        let line = [c(0.0, 0.0), c(0.0, 1.0)];
        // Half a degree of latitude off the equator, which is about 111 m.
        let off = c(0.001, 0.5);
        let d = distance_to_polyline_m(&line, off).expect("line has points");
        assert!((111.0..111.4).contains(&d), "distance was {d}");
        assert!(polyline_hit(&line, off, 120.0));
        assert!(!polyline_hit(&line, off, 100.0));
        // Past the end of the segment the nearest point is the end vertex.
        let beyond = c(0.0, 2.0);
        let end = distance_to_polyline_m(&line, beyond).expect("line has points");
        assert!((end - c(0.0, 1.0).distance_m(beyond)).abs() < 0.5, "end distance was {end}");
        assert!(distance_to_polyline_m(&[], off).is_none());
    }

    #[test]
    fn polygon_needs_three_vertices_and_drops_a_repeated_closing_point() {
        assert!(Polygon::new(vec![c(0.0, 0.0), c(1.0, 1.0)]).is_none());
        let closed = Polygon::new(vec![c(0.0, 0.0), c(0.0, 1.0), c(1.0, 0.0), c(0.0, 0.0)]).expect("triangle");
        assert_eq!(closed.ring.len(), 3);
    }

    #[test]
    fn polygon_containment_follows_the_ring() {
        // Triangle with vertices (lat, lon) (0,0), (0,4), (4,0): the hypotenuse is lat + lon = 4.
        let tri = Polygon::new(vec![c(0.0, 0.0), c(0.0, 4.0), c(4.0, 0.0)]).expect("triangle");
        assert!(tri.contains(c(1.0, 1.0)));
        assert!(!tri.contains(c(3.0, 3.0)));
        assert!(!tri.contains(c(-1.0, 1.0)));
    }

    #[test]
    fn polygon_area_of_a_one_degree_square_on_the_equator() {
        // Vertices from -0.5 to 0.5 in latitude and longitude. The mean latitude is 0, where the projection is exact,
        // so the area is (2 pi R / 360) squared.
        let square = Polygon::new(vec![c(-0.5, -0.5), c(-0.5, 0.5), c(0.5, 0.5), c(0.5, -0.5)]).expect("square");
        let side = 2.0 * PI * EARTH_RADIUS_M / 360.0;
        let expected = side * side;
        let got = square.area_m2();
        assert!((got - expected).abs() / expected < 1e-3, "area {got} vs {expected}");
        let degenerate = Polygon { ring: vec![c(0.0, 0.0), c(0.0, 1.0)] };
        assert_eq!(degenerate.area_m2(), 0.0);
    }

    #[test]
    fn circle_contains_points_inside_the_radius() {
        let circle = Circle::new(c(0.0, 0.0), 111_000.0).expect("valid radius");
        assert!(circle.contains(c(0.0, 0.5)), "about 55 km away");
        assert!(!circle.contains(c(0.0, 1.0)), "about 111.2 km away");
        assert!(Circle::new(c(0.0, 0.0), 0.0).is_none());
        assert!(Circle::new(c(0.0, 0.0), f64::INFINITY).is_none());
    }

    #[test]
    fn circle_area_is_close_to_pi_r_squared_for_small_radii() {
        // A 1 km circle is pi * 10^6 square metres, less a correction of about (r / R)^2 / 12, which is negligible.
        let circle = Circle::new(c(45.0, 10.0), 1000.0).expect("valid radius");
        let expected = PI * 1_000_000.0;
        assert!((circle.area_m2() - expected).abs() / expected < 1e-4, "area was {}", circle.area_m2());
    }
}
