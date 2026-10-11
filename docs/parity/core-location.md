# Core Location: parity work (wave 2)

Official reference: https://developer.apple.com/documentation/corelocation (DocC topics read with
`tools/parity/docc.py corelocation`).

REA evidence of the rendered landing page: `ev_2d7d48823af430310c1a21764d8552d35ef935cacdd6d0b17523dc11dedb4405`.

## What was built

Three pure Rust models in `springboard/ui/src` (bifrost-ui). They read no sensor, start no service and hold no
thread. There is no device position source yet, so nothing in the desktop produces a fix; the models are the value
types an app and a future position source would share.

- `springboard/ui/src/location.rs` (new). `LocationFix` holds a coordinate, altitude, horizontal and vertical accuracy,
  course, speed and a timestamp. Unknown values are negative, as the framework reports them. `motion_between` derives
  course and average speed from two fixes. `Heading` holds the magnetic reading, the true heading once a declination
  is applied, and a sixteen-point `cardinal()` name. `heading_difference_deg` gives the shortest signed turn.
- `springboard/ui/src/geofence.rs` (new). `CircularRegion` (identifier, centre, radius, entry and exit flags) and
  `RegionMonitor`, which turns a stream of fixes into `RegionEvent` values (`Entered`, `Exited`). Each fix is judged
  with its own accuracy, so a fix inside the uncertainty band keeps the previous state. The first decision for a
  region only sets its state and reports nothing.
- `springboard/ui/src/geo.rs` (extended). `Coordinate::bearing_to` (initial great-circle bearing) and
  `Coordinate::destination` (point at a distance and bearing). These are the geometry the course and region code use.

## Official API names mirrored

- `CLLocation` (fields and accuracies), `CLLocationCoordinate2D` (the existing `geo::Coordinate`)
  -> `location::LocationFix`, `geo::Coordinate`.
- `CLHeading` -> `location::Heading`.
- `CLRegion` -> `geofence::CircularRegion`.
- `CLMonitor` and the condition model (outline only, without `CLCondition` types) -> `geofence::RegionMonitor`,
  `geofence::RegionState`, `geofence::RegionEvent`.

Not mirrored in this wave: `CLLocationManager`, `CLLocationUpdate`, `CLBackgroundActivitySession`,
`CLAuthorizationStatus`, `CLGeocoder`, `CLPlacemark`, `CLVisit`, `CLFloor`, `CLBeacon`, `CLServiceSession`, the
location push extension and the error domain. Each needs a position source, a permission model or Apple-only
infrastructure first.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/core-location.json`) | New status | BifrOSt path |
|---|---|---|---|
| Core Location | Coordinates and location values | partial | springboard/ui/src/location.rs (LocationFix, motion_between) |
| Core Location | Region monitoring and conditions | partial | springboard/ui/src/geofence.rs (CircularRegion, RegionMonitor) |
| Core Location | Compass headings and body identification | partial | springboard/ui/src/location.rs (Heading, heading_difference_deg) |

Rows still open: Location manager and live updates (gap, needs a position source), Authorization and usage
descriptions (gap), Reverse geocoding (gap, needs a network lookup), Errors and diagnostics (gap). The rows for
Forward geocoding and Location metadata in photos are unchanged.

## Tests

Each new or changed module has a `#[cfg(test)] mod tests`: `location.rs` (9 tests), `geofence.rs` (7 tests),
and `geo.rs` (3 new tests in its existing test module).

## Not verified

The code was written without a build, per the wave rules, so it has not been compiled or run.
