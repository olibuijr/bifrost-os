# Parity wave 3: MapKit (map overlays, point of interest filter, routes)

Official reference: https://developer.apple.com/documentation/mapkit (REA evidence of the rendered landing page:
`ev_6f0f6e264627d2a27fb59670b4b3211ce35276951f9e73eff6ec65f878a859e2`).

Three pure Rust models in `springboard/ui/src` (bifrost-ui, std only, no network, no drawing). They extend the
coordinate model in `springboard/ui/src/geo.rs` (wave 1) and do not duplicate it. Nothing is wired into the shell
or an app yet.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/map_overlay.rs` | `MKCircle` (`init(center:radius:)`, `coordinate`, `radius`, `boundingMapRect`), `MKPolyline` (`init(coordinates:count:)`), `MKPolygon` (`init(coordinates:count:interiorPolygons:)`, `interiorPolygons`) | `MapCircle` containment by great-circle distance and bounding region. `MapPolyline` length in metres and bounding region. `MapPolygon` containment with holes (even-odd ray cast), perimeter and area in square metres (shoelace on projected points, scaled by mean latitude). |
| `springboard/ui/src/poi_filter.rs` | `MKPointOfInterestFilter` (`init(including:)`, `init(excluding:)`, `includingAll`, `excludingAll`, `includes(_:)`, `excludes(_:)`), `MKPointOfInterestCategory` (raw names) | `PointOfInterestFilter` with the four filter kinds, `apply` over category names, and `KNOWN_CATEGORIES` (84 official raw names, exact match). |
| `springboard/ui/src/route.rs` | `MKRoute` (`steps`, `distance`, `polyline`, `name`, `transportType`), `MKRoute.Step` (`instructions`, `notice`, `distance`, `transportType`, `polyline`), `MKDirections` transport types | `Route` and `RouteStep`. Route distance is the sum of step distances. Route geometry joins step polylines without repeating the shared point. `step_at` finds the step that owns a distance along the route. `remaining_m` gives the distance left. Expected travel time is not modelled. |

Registered in `springboard/ui/src/lib.rs` with one `pub mod` line each.

## Rules the code implements (this module's own, not the official model)

- Overlay distances are great-circle metres using `EARTH_RADIUS_M` from `geo.rs`. Polygon containment is a flat
  ray cast on latitude and longitude, so it is exact for small overlays only. Polygon area is the shoelace formula on
  projected points scaled by the mean latitude of the ring.
- A polygon ring is not closed by repeating its first point; the closing edge is implied. A ring needs three points.
- A step owns the distance interval `[start, end)`, so a boundary distance belongs to the next step, and zero-length
  steps own nothing.
- A polyline join drops the first point of a step only when it equals the last point already in the route.

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-computed values (for example 0.01 degrees of latitude is
R * pi / 180 * 0.01 = 1111.95 m; a 0.002 degree square hole is 222.39 m a side).

- `map_overlay`: 8 tests (circle containment and bounding, polyline length and bounds, polygon containment, holes,
  perimeter and area, bounds).
- `poi_filter`: 6 tests (default, including, excluding, all and empty sets, apply order, known raw names).
- `route`: 7 tests (distance sum, `step_at` boundaries, zero-length steps, remaining distance clamp, polyline join,
  instructions skipping empty text, invalid input).

Not run: per the wave rules no cargo build or test was run in this round. The wave lead runs the crate tests.

## REA evidence (official pages, rendered)

- MapKit landing: `ev_6f0f6e264627d2a27fb59670b4b3211ce35276951f9e73eff6ec65f878a859e2`
- MKPolygon: `ev_236d72fd55f58e8a8f8060621fe5cb3d6f7c6f667fbea196d57211919eb4c3ae`
- MKPointOfInterestFilter: `ev_a13dcbd2985c44c73adb751560a10890e62f0dea2e5731b231b2dcde443876a3`
- MKRoute: `ev_0d5bbbb980e7e0fb55f91d23dede5acb03f1a6e8a8d1504adece9ba3a4ed3935`

DocC facts used (`python3 -I tools/parity/docc.py mapkit/<path>`): mapkit/mkcircle, mkpolyline, mkpolygon,
mkpointofinterestfilter, mkpointofinterestcategory, mkroute, mkroute/step.

## Matrix rows closed (new status `partial`; the lead updates `docs/parity-matrix/` centrally)

| Framework slug | Feature (exactly as in data/mapkit.json) | New status | BifrOSt path |
|---|---|---|---|
| mapkit | Annotations, markers and overlays | partial | springboard/ui/src/map_overlay.rs |
| mapkit | Points of interest and map features | partial | springboard/ui/src/poi_filter.rs |
| mapkit | Directions and routes | partial | springboard/ui/src/route.rs |

Remaining gaps in those rows: no annotation or marker views, no drawing of overlays, no map features selection,
no route service and no expected travel time.
