//! Map coordinates on a Web Mercator projection: `NativeCoordinate`, spans, regions, projected points, sizes and
//! rectangles, great-circle distance and distance strings. The types mirror the map-coordinate structures of the
//! official mapping framework (`NativeCoordinateRegion`, `NativeMapPoint`, `NativeMapRect`, `NativeDistanceFormatter`).
//! Units are degrees for coordinates, map points for the projection and metres for distance.

use std::f64::consts::PI;

/// Mean Earth radius in metres, used for great-circle distance and metres per degree.
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;
/// Width and height of the world in map points: 2^28, the side of the square projection.
pub const WORLD_POINTS: f64 = 268_435_456.0;
/// Latitude where the Web Mercator projection is cut off; beyond it the projection is clamped.
pub const MAX_LATITUDE: f64 = 85.051_128_779_806_59;

/// A latitude and longitude in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl Coordinate {
    /// `None` when the latitude is outside -90..=90, the longitude outside -180..=180, or either value is not finite.
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        let valid = latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude);
        valid.then_some(Coordinate { latitude, longitude })
    }

    /// Great-circle distance to `other` in metres (haversine formula on a sphere of `EARTH_RADIUS_M`).
    pub fn distance_m(self, other: Coordinate) -> f64 {
        let (lat1, lat2) = (self.latitude.to_radians(), other.latitude.to_radians());
        let dlat = lat2 - lat1;
        let dlon = (other.longitude - self.longitude).to_radians();
        let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
    }
}

/// A point on the projected world, with x and y running from 0 to `WORLD_POINTS` (`NativeMapPoint`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapPoint {
    pub x: f64,
    pub y: f64,
}

/// Projects a latitude and longitude in degrees to map points. The longitude is not wrapped.
fn project(latitude: f64, longitude: f64) -> MapPoint {
    let lat = latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    let x = (longitude + 180.0) / 360.0 * WORLD_POINTS;
    let y = (1.0 - lat.tan().asinh() / PI) / 2.0 * WORLD_POINTS;
    MapPoint { x, y }
}

impl MapPoint {
    pub fn new(x: f64, y: f64) -> Self {
        MapPoint { x, y }
    }

    /// Projects a coordinate. Latitudes beyond `MAX_LATITUDE` are clamped to the edge of the square world.
    pub fn from_coordinate(c: Coordinate) -> Self {
        project(c.latitude, c.longitude)
    }

    /// The coordinate under this point. Points outside the world give longitudes outside -180..=180.
    pub fn coordinate(self) -> Coordinate {
        let longitude = self.x / WORLD_POINTS * 360.0 - 180.0;
        let latitude = (PI * (1.0 - 2.0 * self.y / WORLD_POINTS)).sinh().atan().to_degrees();
        Coordinate { latitude, longitude }
    }

    /// Great-circle distance between the coordinates under the two points, in metres.
    pub fn distance_m(self, other: MapPoint) -> f64 {
        self.coordinate().distance_m(other.coordinate())
    }
}

/// Metres spanned by one map point at `latitude` degrees. Shrinks with the cosine of the latitude.
pub fn metres_per_point_at_latitude(latitude: f64) -> f64 {
    2.0 * PI * EARTH_RADIUS_M * latitude.clamp(-90.0, 90.0).to_radians().cos() / WORLD_POINTS
}

/// Width and height on the projection, in map points (`NativeMapSize`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapSize {
    pub width: f64,
    pub height: f64,
}

impl MapSize {
    /// The whole projected world.
    pub const WORLD: MapSize = MapSize { width: WORLD_POINTS, height: WORLD_POINTS };

    pub fn new(width: f64, height: f64) -> Self {
        MapSize { width, height }
    }
}

/// A rectangle on the projection (`NativeMapRect`). Edges are inclusive at the minimum and exclusive at the maximum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl MapRect {
    /// The null rectangle: it has no area, intersects nothing, and is the identity for `union`.
    pub const NULL: MapRect = MapRect { x: f64::INFINITY, y: f64::INFINITY, width: 0.0, height: 0.0 };
    /// The whole projected world.
    pub const WORLD: MapRect = MapRect { x: 0.0, y: 0.0, width: WORLD_POINTS, height: WORLD_POINTS };

    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        MapRect { x, y, width, height }
    }

    /// Builds a rectangle from its outer edges.
    fn from_bounds(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        MapRect { x: min_x, y: min_y, width: max_x - min_x, height: max_y - min_y }
    }

    pub fn is_null(self) -> bool {
        self.x.is_infinite() || self.y.is_infinite()
    }

    pub fn is_empty(self) -> bool {
        self.is_null() || self.width <= 0.0 || self.height <= 0.0
    }

    pub fn min_x(self) -> f64 {
        self.x
    }
    pub fn min_y(self) -> f64 {
        self.y
    }
    pub fn max_x(self) -> f64 {
        self.x + self.width
    }
    pub fn max_y(self) -> f64 {
        self.y + self.height
    }
    pub fn mid_x(self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn mid_y(self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn contains_point(self, p: MapPoint) -> bool {
        !self.is_null() && p.x >= self.min_x() && p.x < self.max_x() && p.y >= self.min_y() && p.y < self.max_y()
    }

    /// True when `other` lies entirely inside this rectangle. A null `other` is never contained.
    pub fn contains_rect(self, other: MapRect) -> bool {
        !other.is_null()
            && other.min_x() >= self.min_x()
            && other.max_x() <= self.max_x()
            && other.min_y() >= self.min_y()
            && other.max_y() <= self.max_y()
    }

    /// True when the rectangles share some area. Rectangles that only touch do not intersect.
    pub fn intersects(self, other: MapRect) -> bool {
        !self.is_null()
            && !other.is_null()
            && self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }

    /// The shared area, or `NULL` when the rectangles do not intersect.
    pub fn intersection(self, other: MapRect) -> MapRect {
        if !self.intersects(other) {
            return MapRect::NULL;
        }
        MapRect::from_bounds(
            self.min_x().max(other.min_x()),
            self.min_y().max(other.min_y()),
            self.max_x().min(other.max_x()),
            self.max_y().min(other.max_y()),
        )
    }

    /// The smallest rectangle covering both. A null operand returns the other one unchanged.
    pub fn union(self, other: MapRect) -> MapRect {
        if self.is_null() {
            return other;
        }
        if other.is_null() {
            return self;
        }
        MapRect::from_bounds(
            self.min_x().min(other.min_x()),
            self.min_y().min(other.min_y()),
            self.max_x().max(other.max_x()),
            self.max_y().max(other.max_y()),
        )
    }

    /// Shrinks each side by `dx` and `dy` (grows them when negative). The size never goes below zero.
    pub fn inset(self, dx: f64, dy: f64) -> MapRect {
        let (w, h) = ((self.width - 2.0 * dx).max(0.0), (self.height - 2.0 * dy).max(0.0));
        MapRect { x: self.x + dx, y: self.y + dy, width: w, height: h }
    }

    /// Moves the origin by `dx` and `dy`, keeping the size.
    pub fn offset(self, dx: f64, dy: f64) -> MapRect {
        MapRect { x: self.x + dx, y: self.y + dy, ..self }
    }
}

/// Height and width of a region in degrees (`NativeCoordinateSpan`). Clamped to the globe on construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateSpan {
    pub latitude_delta: f64,
    pub longitude_delta: f64,
}

/// Clamps a span to `0..=max`. NaN becomes zero; infinity becomes `max`.
fn clamp_span(value: f64, max: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, max)
    }
}

impl CoordinateSpan {
    /// Latitude is limited to 0..=180 degrees and longitude to 0..=360 degrees.
    pub fn new(latitude_delta: f64, longitude_delta: f64) -> Self {
        CoordinateSpan {
            latitude_delta: clamp_span(latitude_delta, 180.0),
            longitude_delta: clamp_span(longitude_delta, 360.0),
        }
    }
}

/// A rectangular geographic region around a centre (`NativeCoordinateRegion`). Regions do not wrap the antimeridian.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateRegion {
    pub center: Coordinate,
    pub span: CoordinateSpan,
}

impl CoordinateRegion {
    pub fn new(center: Coordinate, span: CoordinateSpan) -> Self {
        CoordinateRegion { center, span }
    }

    /// A region of `latitudinal_m` by `longitudinal_m` metres around `center`. One degree of latitude is a fixed arc;
    /// one degree of longitude shrinks with the cosine of the latitude, so the same metres give a wider longitude span
    /// away from the equator.
    pub fn from_meters(center: Coordinate, latitudinal_m: f64, longitudinal_m: f64) -> Self {
        let metres_per_degree_lat = 2.0 * PI * EARTH_RADIUS_M / 360.0;
        let metres_per_degree_lon = metres_per_degree_lat * center.latitude.to_radians().cos();
        CoordinateRegion::new(
            center,
            CoordinateSpan::new(latitudinal_m / metres_per_degree_lat, longitudinal_m / metres_per_degree_lon),
        )
    }

    /// The region that covers a map rectangle. A null rectangle gives an empty region at the origin.
    pub fn from_map_rect(rect: MapRect) -> Self {
        if rect.is_null() {
            return CoordinateRegion::new(Coordinate { latitude: 0.0, longitude: 0.0 }, CoordinateSpan::new(0.0, 0.0));
        }
        let north_west = MapPoint::new(rect.min_x(), rect.min_y()).coordinate();
        let south_east = MapPoint::new(rect.max_x(), rect.max_y()).coordinate();
        let center = MapPoint::new(rect.mid_x(), rect.mid_y()).coordinate();
        CoordinateRegion::new(
            center,
            CoordinateSpan::new(north_west.latitude - south_east.latitude, south_east.longitude - north_west.longitude),
        )
    }

    /// The map rectangle that the region covers. Its edges follow the projection, so the rectangle is not
    /// centred on `center` in map points unless the latitude is zero.
    pub fn map_rect(self) -> MapRect {
        let half_lat = self.span.latitude_delta / 2.0;
        let half_lon = self.span.longitude_delta / 2.0;
        let north = self.center.latitude + half_lat;
        let south = self.center.latitude - half_lat;
        let west = self.center.longitude - half_lon;
        let east = self.center.longitude + half_lon;
        let top_left = project(north, west);
        let bottom_right = project(south, east);
        MapRect::from_bounds(top_left.x, top_left.y, bottom_right.x, bottom_right.y)
    }

    /// True when `c` lies inside the region (edges included). The region does not wrap the antimeridian.
    pub fn contains(self, c: Coordinate) -> bool {
        (c.latitude - self.center.latitude).abs() <= self.span.latitude_delta / 2.0
            && (c.longitude - self.center.longitude).abs() <= self.span.longitude_delta / 2.0
    }

    /// The smallest region around `points`, with each span grown by `padding` (0.1 adds 10 per cent on each axis).
    /// `None` when there are no points. Coordinates are not wrapped, so a set that crosses the antimeridian is
    /// covered the long way round.
    pub fn fitting(points: &[Coordinate], padding: f64) -> Option<Self> {
        if points.is_empty() {
            return None;
        }
        let (mut min_lat, mut max_lat) = (f64::MAX, f64::MIN);
        let (mut min_lon, mut max_lon) = (f64::MAX, f64::MIN);
        for p in points {
            min_lat = min_lat.min(p.latitude);
            max_lat = max_lat.max(p.latitude);
            min_lon = min_lon.min(p.longitude);
            max_lon = max_lon.max(p.longitude);
        }
        let grow = 1.0 + padding.max(0.0);
        let center = Coordinate { latitude: (min_lat + max_lat) / 2.0, longitude: (min_lon + max_lon) / 2.0 };
        let span = CoordinateSpan::new((max_lat - min_lat) * grow, (max_lon - min_lon) * grow);
        Some(CoordinateRegion::new(center, span))
    }
}

/// Unit system for distance strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceUnits {
    Metric,
    Imperial,
}

/// A distance in short form (`NativeDistanceFormatter`, default units). Metres below a kilometre, then kilometres;
/// feet below a thousand, then miles. Negative input is shown as zero. Only the English unit symbols are produced.
pub fn format_distance(meters: f64, units: DistanceUnits) -> String {
    let meters = meters.max(0.0);
    match units {
        DistanceUnits::Metric if meters < 1000.0 => format!("{:.0} m", meters),
        DistanceUnits::Metric => format!("{:.1} km", meters / 1000.0),
        DistanceUnits::Imperial => {
            let feet = meters / 0.3048;
            if feet < 1000.0 {
                format!("{:.0} ft", feet)
            } else {
                format!("{:.1} mi", meters / 1609.344)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(latitude: f64, longitude: f64) -> Coordinate {
        Coordinate::new(latitude, longitude).expect("valid test coordinate")
    }

    #[test]
    fn coordinates_reject_out_of_range_and_non_finite_values() {
        assert!(Coordinate::new(91.0, 0.0).is_none());
        assert!(Coordinate::new(0.0, -180.5).is_none());
        assert!(Coordinate::new(f64::NAN, 0.0).is_none());
        assert_eq!(Coordinate::new(64.1466, -21.9426).map(|p| p.latitude), Some(64.1466));
    }

    #[test]
    fn great_circle_distance_matches_a_known_city_pair() {
        let london = c(51.5074, -0.1278);
        let paris = c(48.8566, 2.3522);
        let d = london.distance_m(paris);
        assert!((340_000.0..348_000.0).contains(&d), "london to paris was {d} m");
        assert_eq!(london.distance_m(london), 0.0);
    }

    #[test]
    fn mercator_projection_centres_the_origin_and_round_trips() {
        let origin = MapPoint::from_coordinate(c(0.0, 0.0));
        assert!((origin.x - WORLD_POINTS / 2.0).abs() < 1e-6);
        assert!((origin.y - WORLD_POINTS / 2.0).abs() < 1e-6);
        assert!(MapPoint::from_coordinate(c(0.0, -180.0)).x.abs() < 1e-6);
        let p = c(64.1466, -21.9426);
        let back = MapPoint::from_coordinate(p).coordinate();
        assert!((back.latitude - p.latitude).abs() < 1e-9);
        assert!((back.longitude - p.longitude).abs() < 1e-9);
    }

    #[test]
    fn projection_clamps_latitude_to_the_square_world() {
        let pole = MapPoint::from_coordinate(c(90.0, 0.0));
        assert!(pole.y.abs() < 1e-3, "north pole should sit on the top edge, got y={}", pole.y);
        let south = MapPoint::from_coordinate(c(-90.0, 0.0));
        assert!((south.y - WORLD_POINTS).abs() < 1e-3);
    }

    #[test]
    fn metres_per_point_shrinks_with_the_cosine_of_latitude() {
        let equator = metres_per_point_at_latitude(0.0);
        let circumference = 2.0 * PI * EARTH_RADIUS_M;
        assert!((equator - circumference / WORLD_POINTS).abs() < 1e-9);
        assert!((metres_per_point_at_latitude(60.0) - equator / 2.0).abs() < 1e-9);
    }

    #[test]
    fn map_rect_union_intersection_and_containment() {
        let a = MapRect::new(0.0, 0.0, 10.0, 10.0);
        let b = MapRect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.intersects(b));
        assert_eq!(a.intersection(b), MapRect::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(a.union(b), MapRect::new(0.0, 0.0, 15.0, 15.0));
        assert!(!a.contains_rect(b));
        assert!(a.contains_rect(MapRect::new(1.0, 1.0, 2.0, 2.0)));
        assert!(a.contains_point(MapPoint::new(9.9, 0.0)));
        assert!(!a.contains_point(MapPoint::new(10.0, 0.0)), "the maximum edge is exclusive");
        assert!(a.intersection(MapRect::new(20.0, 20.0, 1.0, 1.0)).is_null());
        assert_eq!(MapRect::NULL.union(a), a);
        assert!(!MapRect::NULL.intersects(a));
    }

    #[test]
    fn map_rect_inset_never_goes_negative_and_offset_keeps_size() {
        let r = MapRect::new(0.0, 0.0, 10.0, 4.0);
        assert_eq!(r.inset(1.0, 1.0), MapRect::new(1.0, 1.0, 8.0, 2.0));
        assert_eq!(r.inset(9.0, 9.0).width, 0.0);
        assert_eq!(r.offset(3.0, -1.0), MapRect::new(3.0, -1.0, 10.0, 4.0));
    }

    #[test]
    fn region_from_meters_sizes_latitude_and_widens_longitude_away_from_equator() {
        let equator = CoordinateRegion::from_meters(c(0.0, 0.0), 1000.0, 1000.0);
        let north = CoordinateRegion::from_meters(c(60.0, 0.0), 1000.0, 1000.0);
        assert!((equator.span.latitude_delta - equator.span.longitude_delta).abs() < 1e-12);
        let metres_per_degree = 2.0 * PI * EARTH_RADIUS_M / 360.0;
        assert!((equator.span.latitude_delta * metres_per_degree - 1000.0).abs() < 1e-6);
        // cos(60 degrees) is one half, so the same metres need twice the longitude span.
        assert!((north.span.longitude_delta - equator.span.longitude_delta * 2.0).abs() < 1e-9);
    }

    #[test]
    fn span_is_clamped_to_the_globe() {
        assert_eq!(CoordinateSpan::new(400.0, -3.0), CoordinateSpan { latitude_delta: 180.0, longitude_delta: 0.0 });
        assert_eq!(CoordinateSpan::new(f64::NAN, 10.0).latitude_delta, 0.0);
        assert_eq!(CoordinateSpan::new(1.0, f64::INFINITY).longitude_delta, 360.0);
    }

    #[test]
    fn region_and_map_rect_convert_both_ways() {
        let region = CoordinateRegion::new(c(64.0, -22.0), CoordinateSpan::new(0.5, 1.0));
        let back = CoordinateRegion::from_map_rect(region.map_rect());
        assert!((back.span.latitude_delta - 0.5).abs() < 1e-6);
        assert!((back.span.longitude_delta - 1.0).abs() < 1e-9);
        assert!((back.center.longitude + 22.0).abs() < 1e-9);
        assert!(back.contains(region.center));
        let empty = CoordinateRegion::from_map_rect(MapRect::NULL);
        assert_eq!(empty.span, CoordinateSpan::new(0.0, 0.0));
    }

    #[test]
    fn fitting_covers_every_point_with_padding() {
        let points = [c(64.0, -22.0), c(65.0, -20.0), c(63.5, -23.0)];
        let region = CoordinateRegion::fitting(&points, 0.1).expect("three points");
        assert!(points.iter().all(|p| region.contains(*p)));
        assert!((region.span.latitude_delta - 1.5 * 1.1).abs() < 1e-9);
        assert!((region.span.longitude_delta - 3.0 * 1.1).abs() < 1e-9);
        assert!(CoordinateRegion::fitting(&[], 0.0).is_none());
    }

    #[test]
    fn distance_strings_switch_units_at_the_threshold() {
        assert_eq!(format_distance(800.0, DistanceUnits::Metric), "800 m");
        assert_eq!(format_distance(1500.0, DistanceUnits::Metric), "1.5 km");
        assert_eq!(format_distance(-5.0, DistanceUnits::Metric), "0 m");
        assert_eq!(format_distance(30.48, DistanceUnits::Imperial), "100 ft");
        assert_eq!(format_distance(1609.344, DistanceUnits::Imperial), "1.0 mi");
    }
}
