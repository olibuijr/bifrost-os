//! Map overlays as pure data: a circle, an open polyline and a closed polygon with optional holes, all on the
//! coordinates from [`crate::geo`]. Nothing is drawn here; the renderer reads the points. The types follow the
//! overlay classes of the official mapping framework (a circle with a centre and a radius, a polyline, a polygon
//! with interior polygons) in outline.
//!
//! Distances along a line and the radius of a circle are great-circle metres. Containment in a polygon and the
//! area of a polygon use a flat model: containment is a ray cast on latitude and longitude, and area is the shoelace
//! formula on the projected points scaled by the mean latitude of the ring. Both are exact for small overlays and
//! approximate for overlays that span many degrees. This module's own rule, not part of the official model.

use crate::geo::{metres_per_point_at_latitude, Coordinate, CoordinateRegion, MapPoint};

/// A circle of `radius_m` metres around `center` (the official circle overlay).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapCircle {
    pub center: Coordinate,
    /// Radius in metres, always positive.
    pub radius_m: f64,
}

impl MapCircle {
    /// `None` when the radius is not finite or not positive.
    pub fn new(center: Coordinate, radius_m: f64) -> Option<Self> {
        (radius_m.is_finite() && radius_m > 0.0).then_some(MapCircle { center, radius_m })
    }

    /// True when `c` is within the radius, edge included.
    pub fn contains(&self, c: Coordinate) -> bool {
        self.center.distance_m(c) <= self.radius_m
    }

    /// The region that covers the circle's square bounding box, with the centre in the middle.
    pub fn bounding_region(&self) -> CoordinateRegion {
        CoordinateRegion::from_meters(self.center, 2.0 * self.radius_m, 2.0 * self.radius_m)
    }
}

/// An open line through one or more coordinates (the official polyline overlay).
#[derive(Clone, Debug, PartialEq)]
pub struct MapPolyline {
    pub coordinates: Vec<Coordinate>,
}

impl MapPolyline {
    /// `None` when there are no coordinates.
    pub fn new(coordinates: Vec<Coordinate>) -> Option<Self> {
        (!coordinates.is_empty()).then_some(MapPolyline { coordinates })
    }

    /// Sum of the great-circle distances between consecutive points, in metres. A single point has length zero.
    pub fn length_m(&self) -> f64 {
        self.coordinates.windows(2).map(|w| w[0].distance_m(w[1])).sum()
    }

    /// The smallest region that covers every point, with no padding. `None` cannot happen for a valid polyline.
    pub fn bounding_region(&self) -> Option<CoordinateRegion> {
        CoordinateRegion::fitting(&self.coordinates, 0.0)
    }
}

/// A closed polygon with optional interior rings that cut holes out of it (the official polygon overlay).
/// The rings are not closed by repeating the first point; the closing edge is implied.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPolygon {
    pub outer: Vec<Coordinate>,
    /// Holes. Each ring must also have at least three points.
    pub interior: Vec<Vec<Coordinate>>,
}

/// True when `c` is inside the ring by the even-odd rule on latitude and longitude. Points on an edge may go either way.
fn ring_contains(ring: &[Coordinate], c: Coordinate) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.latitude > c.latitude) != (b.latitude > c.latitude) {
            let cross = (b.longitude - a.longitude) * (c.latitude - a.latitude) / (b.latitude - a.latitude) + a.longitude;
            if c.longitude < cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Sum of the edges of a ring, including the edge that closes it, in metres.
fn ring_perimeter_m(ring: &[Coordinate]) -> f64 {
    let open: f64 = ring.windows(2).map(|w| w[0].distance_m(w[1])).sum();
    open + ring[ring.len() - 1].distance_m(ring[0])
}

/// Area of a ring in square metres: the shoelace formula on projected points, scaled by the mean latitude.
fn ring_area_m2(ring: &[Coordinate]) -> f64 {
    let points: Vec<MapPoint> = ring.iter().map(|c| MapPoint::from_coordinate(*c)).collect();
    let mut twice: f64 = 0.0;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        twice += a.x * b.y - b.x * a.y;
    }
    let mean_lat = ring.iter().map(|c| c.latitude).sum::<f64>() / ring.len() as f64;
    let metres_per_point = metres_per_point_at_latitude(mean_lat);
    (twice.abs() / 2.0) * metres_per_point * metres_per_point
}

impl MapPolygon {
    /// `None` when the outer ring or any hole has fewer than three points.
    pub fn new(outer: Vec<Coordinate>, interior: Vec<Vec<Coordinate>>) -> Option<Self> {
        let valid = outer.len() >= 3 && interior.iter().all(|h| h.len() >= 3);
        valid.then_some(MapPolygon { outer, interior })
    }

    /// True when `c` is inside the outer ring and outside every hole.
    pub fn contains(&self, c: Coordinate) -> bool {
        ring_contains(&self.outer, c) && !self.interior.iter().any(|h| ring_contains(h, c))
    }

    /// Length of the outer ring, closing edge included, in metres. Holes are not counted.
    pub fn perimeter_m(&self) -> f64 {
        ring_perimeter_m(&self.outer)
    }

    /// Area in square metres: the outer ring minus the holes. Never negative.
    pub fn area_m2(&self) -> f64 {
        let holes: f64 = self.interior.iter().map(|h| ring_area_m2(h)).sum();
        (ring_area_m2(&self.outer) - holes).max(0.0)
    }

    /// The smallest region that covers the outer ring, with no padding.
    pub fn bounding_region(&self) -> Option<CoordinateRegion> {
        CoordinateRegion::fitting(&self.outer, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    /// A square of side 0.01 degrees on the equator, about 1111.95 m a side.
    fn square() -> Vec<Coordinate> {
        vec![c(0.0, 0.0), c(0.0, 0.01), c(0.01, 0.01), c(0.01, 0.0)]
    }

    #[test]
    fn circle_contains_points_within_its_radius() {
        let circle = MapCircle::new(c(0.0, 0.0), 500.0).expect("positive radius");
        // 0.001 degrees of latitude is about 111.2 m, inside 500 m.
        assert!(circle.contains(c(0.001, 0.0)));
        // 0.01 degrees is about 1112 m, outside 500 m.
        assert!(!circle.contains(c(0.01, 0.0)));
        assert!(MapCircle::new(c(0.0, 0.0), 0.0).is_none());
        assert!(MapCircle::new(c(0.0, 0.0), f64::NAN).is_none());
    }

    #[test]
    fn circle_bounding_region_spans_twice_the_radius() {
        let circle = MapCircle::new(c(0.0, 0.0), 1000.0).expect("positive radius");
        let r = circle.bounding_region();
        // 2000 m over 111195.08 m per degree is about 0.017986 degrees on each axis.
        assert!((r.span.latitude_delta - 2000.0 / 111_195.08).abs() < 1e-6);
        assert!((r.span.longitude_delta - 2000.0 / 111_195.08).abs() < 1e-6);
        assert_eq!(r.center, c(0.0, 0.0));
    }

    #[test]
    fn polyline_length_sums_the_legs() {
        // Two legs of 0.01 degrees each: 1111.95 m north and 1111.95 m east along the equator and then north.
        let line = MapPolyline::new(vec![c(0.0, 0.0), c(0.0, 0.01), c(0.01, 0.01)]).expect("points");
        assert!((line.length_m() - 2223.90).abs() < 0.05);
        let single = MapPolyline::new(vec![c(5.0, 5.0)]).expect("one point");
        assert_eq!(single.length_m(), 0.0);
        assert!(MapPolyline::new(Vec::new()).is_none());
    }

    #[test]
    fn polyline_bounding_region_covers_its_points() {
        let line = MapPolyline::new(vec![c(0.0, 0.0), c(0.02, 0.04)]).expect("points");
        let r = line.bounding_region().expect("non-empty");
        assert!((r.center.latitude - 0.01).abs() < 1e-12);
        assert!((r.center.longitude - 0.02).abs() < 1e-12);
        assert!((r.span.latitude_delta - 0.02).abs() < 1e-12);
        assert!((r.span.longitude_delta - 0.04).abs() < 1e-12);
    }

    #[test]
    fn polygon_contains_points_inside_the_outer_ring_only() {
        let poly = MapPolygon::new(square(), Vec::new()).expect("three or more points");
        assert!(poly.contains(c(0.005, 0.005)));
        assert!(!poly.contains(c(0.02, 0.005)));
        assert!(!poly.contains(c(0.005, -0.001)));
        assert!(MapPolygon::new(vec![c(0.0, 0.0), c(1.0, 1.0)], Vec::new()).is_none());
    }

    #[test]
    fn polygon_holes_are_excluded_from_containment() {
        let hole = vec![c(0.004, 0.004), c(0.004, 0.006), c(0.006, 0.006), c(0.006, 0.004)];
        let poly = MapPolygon::new(square(), vec![hole]).expect("valid rings");
        assert!(poly.contains(c(0.002, 0.002)));
        assert!(!poly.contains(c(0.005, 0.005)));
        // The hole is 0.002 degrees square, so its area is about 222.4 m on each side.
        let outer_only = MapPolygon::new(square(), Vec::new()).expect("valid rings");
        let hole_area = outer_only.area_m2() - poly.area_m2();
        assert!((hole_area - 222.39 * 222.39).abs() / (222.39 * 222.39) < 0.01);
    }

    #[test]
    fn polygon_perimeter_and_area_of_a_small_square() {
        let poly = MapPolygon::new(square(), Vec::new()).expect("valid rings");
        // Four sides of 1111.95 m each.
        assert!((poly.perimeter_m() - 4.0 * 1111.95).abs() < 0.5);
        // The side squared is about 1.236e6 square metres.
        let area = poly.area_m2();
        assert!(area > 1_230_000.0 && area < 1_240_000.0, "area {area}");
    }

    #[test]
    fn polygon_bounding_region_is_centred_on_the_ring() {
        let poly = MapPolygon::new(square(), Vec::new()).expect("valid rings");
        let r = poly.bounding_region().expect("non-empty");
        assert!((r.center.latitude - 0.005).abs() < 1e-12);
        assert!((r.center.longitude - 0.005).abs() < 1e-12);
        assert!((r.span.latitude_delta - 0.01).abs() < 1e-12);
    }
}
