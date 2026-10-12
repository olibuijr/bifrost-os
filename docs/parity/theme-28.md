# Theme 28: map camera and region

Cross-framework theme: MapKit, EventKit, Core Location and VisionKit gap rows about map cameras, regions, geofences and
camera regions of interest. Built in pure Rust with no build run in this round (per the theme brief).

## What was built

- `springboard/ui/src/geo.rs` (extended). Map coordinate module from an earlier wave, now also holding:
  - `Coordinate::bearing_deg` and `Coordinate::destination`: initial great-circle bearing and the point a distance
    away along a bearing, on the same sphere as `distance_m`. The longitude of a destination is wrapped to -180..=180.
  - `MapCamera`: a virtual viewpoint with `center`, `heading` (degrees clockwise from north, normalised to 0..360),
    `pitch` (degrees from straight down, clamped to 0..=90) and `center_distance_m`. `from_eye` builds it from an eye
    coordinate and an altitude (`None` for a non-positive altitude). `altitude` and `eye_coordinate` read it back.
  - `MapCameraPosition` (`Automatic`, `Region`, `Camera`) with `region` (fit to content, fixed region, or `None` for a
    camera, which has no span) and `center`.
- `springboard/ui/src/geofence.rs` (new, one `pub mod geofence;` line in `springboard/ui/src/lib.rs`). Circular
  geofences and their conditions:
  - `CircularRegion` with an identifier, centre, radius in metres and separate entry and exit notification flags.
  - `Condition` (`Inside`, `Outside`, `Unknown`) evaluated against the accuracy circle of a reported position.
  - `Monitor` with `add`, `remove`, `condition` and `update`. `update` returns `Transition`s (`Event::Entered` or
    `Event::Exited`). An `Unknown` result keeps the last determined state, so boundary noise does not repeat events.

No device position source, no SDK API and no app wiring exist. The model is pure and is fed by its caller, so the
rows below are `partial`, not `parity`.

## Official API names mirrored

- MapKit: `MKCoordinateRegion`, `MKCoordinateSpan`, `MKMapCamera` (`init(lookingAtCenter:fromEyeCoordinate:eyeAltitude:)`,
  `init(lookingAtCenter:fromDistance:pitch:heading:)`), `MKMapPoint`, `MKMapRect`, `MKMapSize`, `MapCameraPosition`
  (SwiftUI, named in the matrix row).
- Core Location: `CLCircularRegion` (deprecated in favour of the next item), `CLCircularGeographicCondition`
  (`initWithCenter:radius:`, `center`, `radius`). The region monitoring article ("Monitoring the user's proximity to
  geographic regions") describes the monitor that `Monitor` mirrors. Its `CLMonitor` page was not resolvable through
  `docc.py` (HTTP 404), so no official DocC facts are cited for `CLMonitor` itself.
- VisionKit: `CameraRegionView`. Read from the official page: it needs the enterprise `Camera Region access`
  entitlement and is a visionOS view. Not mirrored.

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

| Page | REA evidence id |
|---|---|
| MapKit framework landing | ev_5950750307b63c5ab74b970e88a4cd43805f56c00b95596023ef99a49763f61a |
| MKMapCamera | ev_ffd13a74af3f341a989a1a7018e327351b3c7f061f095cf92cb677a047918b6e |
| MKCoordinateRegion | ev_7849105506c3c5a3c57bcdbbfdcfae49cdf9ffb38e90725a87daaba780b9b55d |
| MKMapPoint | ev_25ce61f6b7d25a6042fa5dfa49f4d1226a3abac4506c7551da390a4f3fdad4d3 |
| CLCircularRegion | ev_9dabf09169a26a3a73a7c9954f4e226b76b333672c98a7d57f103e26aa81d0a5 |
| CLCircularGeographicCondition | ev_86f5b09e3eaa6060e71ead067061fd066969244c08050a45cd8cfd97713fdd46 |
| CameraRegionView | ev_f8c8f90c4e6c98e5c513e38087f3ba22f4168f06b32757fe48132cdb0f61df0e |

`MKMapRect` and `MKMapSize` have no separate REA record in this round; they are referenced from the `MKCoordinateRegion`
page's map coordinates list.

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| mapkit | Map camera and region | partial | springboard/ui/src/geo.rs (`CoordinateRegion`, `MapCamera`, `MapCameraPosition`) |
| mapkit | Geographic coordinate types (MKMapPoint, MKMapRect, MKMapSize) | partial | springboard/ui/src/geo.rs (`MapPoint`, `MapRect`, `MapSize`, `format_distance`) |
| core-location | Region monitoring and conditions | partial | springboard/ui/src/geofence.rs (`CircularRegion`, `Condition`, `Monitor`) |
| visionkit | Stabilised camera region of interest | n/a (reclassified from gap) | none. Official page: enterprise entitlement, visionOS only |

Notes for the lead:

- `Geographic coordinate types` was already implemented in `geo.rs` before this round. The matrix still said `gap`,
  so the row is stale and is now `partial`.
- `Reverse geocoding` (mapkit and core-location), `Location-based reminders` (eventkit) and the other mapkit rows stay
  `gap`. Reverse geocoding needs a lookup service, which the no-network rule excludes. `Location-based reminders` could
  reuse `geofence.rs`, but the Reminders app does not use it yet.

## Verification

- `python3 tools/check-names.py`: no hits.
- No cargo, rustc or test runs in this round, per the theme brief. The new code and its tests (7 in `geo.rs`, 7 in
  `geofence.rs`) have been reviewed by reading only and must be compiled and run in the next build round.
- The `tools/parity/parity.py search` command failed on a corrupt semantic embedding cache (`keys.npy` bad CRC). The
  search was therefore not used to find existing modules; the bifrost-os index search and a reading of `geo.rs` were used.
