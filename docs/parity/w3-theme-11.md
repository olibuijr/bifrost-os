# Wave 3, theme 11: map view models (MKMapView, Map)

## What was built

Three pure modules in `springboard/ui/src/` (bifrost-ui, std only, no network, no drawing). They extend the wave 1
coordinate model in `springboard/ui/src/geo.rs` (`Coordinate`, `MapPoint`, `MapRect`, `CoordinateRegion`), which is
reused and not changed. Each module has its own `#[cfg(test)] mod tests`.

- `springboard/ui/src/mapview.rs`: `MapCamera` (centre, zoom 0..20, heading, viewport size in pixels). It converts
  between screen pixels and coordinates, computes the visible map rectangle, fits a camera to a region, pans with
  longitude wrap, zooms with clamping, and works out a scale bar (1, 2 or 5 times a power of ten metres). It also has
  `UserTracking` (off, follow, follow with heading) with `follow()`, and `compass_visible()`.
- `springboard/ui/src/mapshape.rs`: overlay geometry. `polyline_length_m`, `distance_to_segment_m`,
  `distance_to_polyline_m` and `polyline_hit` for tap hit tests, `Polygon` (ring, even-odd `contains`, projected
  `area_m2`), and `Circle` (great-circle `contains`, exact spherical-cap `area_m2`).
- `springboard/ui/src/reverse_lookup.rs`: `PlaceIndex` over caller-supplied `Place` values, with `nearest` and
  `within` by great-circle distance, and `address_line`. It is an offline nearest-place lookup. It is not a geocoder.

`lib.rs` gains three `pub mod` lines (`mapview`, `mapshape`, `reverse_lookup`) after `geofence`.

## Official APIs mirrored

- MKMapView (`https://developer.apple.com/documentation/mapkit/mkmapview`): the camera and region concepts
  (`MapCamera`, `CoordinateRegion` span, visible rectangle). DocC summary: "An embeddable map interface".
- MKCompassButton (`https://developer.apple.com/documentation/mapkit/mkcompassbutton`): `compass_visible`.
- MKUserTrackingButton (`https://developer.apple.com/documentation/mapkit/mkusertrackingbutton`): `UserTracking`.
  The official button toggles whether the map tracks the user's heading. The three-state cycle (off, follow, follow
  with heading) and the half-degree compass threshold are this module's own rules.
- MKReverseGeocodingRequest (`https://developer.apple.com/documentation/mapkit/mkreversegeocodingrequest`):
  `PlaceIndex::nearest` and `within`. The official request calls a network service and returns map items. Our
  version searches only a list the caller holds.
- CLPlacemark and the reverse geocoder in Core Location: `Place` and `address_line` mirror the placemark's name and
  region fields only. No street address, postal code or country fields exist.
- MKPolyline, MKPolygon and MKCircle: `Polyline` helpers, `Polygon` and `Circle`. These match the overlay shapes
  but draw nothing.
- Scale bar and zoom controls (MKScaleView, MKZoomControl): `MapCamera::scale_bar` and `zoom_by`. The zoom scale
  uses the 256-pixel web tile convention. This is our own convention, not Apple's.

## REA evidence

- `ev_f20568b341e380347fcd564362389c1bafab257ff71f8ba64812dcfdb2ea308a`: MKMapView page (`tools/parity/rea-page.sh`).
- `ev_d090bf7c6c00333372f8f111b68f478cf146a6285bf79594b4298b807013daaa`: MKReverseGeocodingRequest page.
- `ev_5038d4166e073fda1d3c9a45097a5e4a6248d56d4e7d145505ef5ac919db932f`: MapKit landing page, from the wave 2 matrix.

Official summaries from `python3 -I tools/parity/docc.py`: MKMapView (macOS 10.9), MKCompassButton (macOS 11.0),
MKUserTrackingButton (Mac Catalyst 13.1; the page lists no macOS version).

Not verified: the Core Location reverse-geocoder page was not captured with REA, so the placemark row rests on the
matrix row text and the DocC index, not on a captured page.

## Matrix rows closed

| Framework slug | Feature (exactly as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| mapkit | Map camera and region | partial | springboard/ui/src/mapview.rs (MapCamera) |
| mapkit | Map controls (compass, zoom, scale, user tracking) | partial | springboard/ui/src/mapview.rs (scale_bar, zoom_by, compass_visible, UserTracking) |
| mapkit | User location on a map | partial | springboard/ui/src/mapview.rs (UserTracking, MapCamera::follow); no position source |
| mapkit | Annotations, markers and overlays | partial | springboard/ui/src/mapshape.rs (Polygon, Circle, polyline_hit); nothing is drawn |
| mapkit | Reverse geocoding (coordinate to address) | partial | springboard/ui/src/reverse_lookup.rs (PlaceIndex::nearest, within, address_line); no address fields |
| core-location | Reverse geocoding (coordinates to placemark) | partial | springboard/ui/src/reverse_lookup.rs (Place, PlaceIndex::nearest); no street or country fields |

Not closed: "Map view (MKMapView, Map)" (no renderer or view), "Points of interest and map features", "Directions and
routes", "Map snapshots (static images)", "Indoor maps and GeoJSON" (no GeoJSON parser was built), and the
InputMethodKit, EventKit, HIDDriverKit, VisionKit and PDFKit rows of this theme.

## Tests and checks

- Each module has at least five behaviour tests with values worked by hand: `mapview` has 8, `mapshape` has 7 and
  `reverse_lookup` has 6. The expected values are derived from the rules in each module.
- `python3 tools/check-names.py` reports no hits. None of the three new files contains an Apple platform or product
  name.
- Per the wave instructions, nothing was built or run (no cargo, rustc or make). The tests are written but have not
  been executed.
