# MapKit parity: map coordinate model

Framework: MapKit, <https://developer.apple.com/documentation/mapkit>
REA evidence of the rendered framework page: `ev_8a46c1037184a0886f39b32ddbdd908510a039dbbb824edfd2aba4cf6026a41f`
(earlier matrix row evidence, from the first audit: `ev_5038d4166e073fda1d3c9a45097a5e4a6248d56d4e7d145505ef5ac919db932f`).

## What was built

`springboard/ui/src/geo.rs` (bifrost-ui, pure Rust, std only, no shell or renderer dependencies). It adds:

- `Coordinate`: a validated latitude and longitude in degrees, with `distance_m` (haversine great-circle distance).
- `MapPoint`: Web Mercator projection of a coordinate onto a square world of `WORLD_POINTS` (2^28) map points,
  with the inverse `coordinate()`, and `distance_m` between two points.
- `metres_per_point_at_latitude`: metres spanned by one map point, scaled by the cosine of the latitude.
- `MapSize` and `MapRect`: size and rectangle on the projection, with null rectangle handling, containment,
  intersection, union, inset and offset.
- `CoordinateSpan` (clamped to the globe) and `CoordinateRegion`: centre and span, sizing from metres, fitting to a
  set of coordinates with padding, containment, and conversion to and from map rectangles.
- `DistanceUnits` and `format_distance`: short distance strings in metric (m, km) and imperial (ft, mi).

Wiring: one `pub mod geo;` line in `springboard/ui/src/lib.rs`. Unit tests live in `geo.rs` (`mod tests`, 11 tests).

## Official API names mirrored

| Official type | BifrOSt type |
|---|---|
| MKCoordinateRegion | `CoordinateRegion` |
| MKCoordinateSpan | `CoordinateSpan` |
| MKMapPoint | `MapPoint` |
| MKMapSize | `MapSize` (`MapSize::WORLD`) |
| MKMapRect | `MapRect` (`MapRect::NULL`, `MapRect::WORLD`) |
| MKDistanceFormatter (default style, partial) | `format_distance` |
| CLLocationCoordinate2D shape (coordinate only) | `Coordinate` |

Functions mirrored in behaviour: `distance(to:)` (`MapPoint::distance_m`), `MKMetersPerMapPointAtLatitude`
(`metres_per_point_at_latitude`), `init(center:latitudinalMeters:longitudinalMeters:)` (`CoordinateRegion::from_meters`),
`init(_:)` for region from rectangle and rectangle from region (`from_map_rect`, `map_rect`), `contains`/`intersects`/
`union`/`intersection`/`insetBy`/`offsetBy` on rectangles.

## Decisions and limits

- Earth radius is a mean sphere (6 371 008.8 m). The official framework's exact model is not stated on the
  framework page, so distances are approximations of its results.
- `WORLD_POINTS` is 2^28. The official `MKMapSize.world` page describes the value without giving the number; the
  constant is the standard Web Mercator world size and is not verified against a document in this session.
- Regions and rectangles do not wrap the antimeridian. `CoordinateRegion::fitting` covers a set that crosses it the
  long way round.
- Distance strings use English symbols only and fixed thresholds (metres below 1 000, feet below 1 000). The official
  distance formatter is locale-aware; the SDK already has a measurement formatter in `springboard/sdk/src/locale.rs`
  that is not reused here, because bifrost-ui cannot depend on the SDK crate.
- Nothing is built or tested in this change; the build and test stage must confirm it compiles.

## Remaining gaps

- No map view, tile rendering or annotations (the renderer has no map layer).
- No camera model (heading, pitch, altitude) such as the official map camera.
- No map-rect division, and no GeoJSON or IMDF parsing.
- No reverse geocoding, points-of-interest search, search completion, directions or snapshots (these need an
  external service or a renderer, and are outside pure Rust std-only scope).
- No location service API for apps.
- No maps URL handling or default navigation app.
