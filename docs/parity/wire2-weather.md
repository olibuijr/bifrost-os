# Wire 2: Weather (1.3.0) wires parity models

Branch `wire2/weather`, app `springboard/apps/weather-desktop/` (crate `bifrost-weather`, id `is.olibuijr.weather.desktop`).
Version 1.2.3 to 1.3.0 in `Cargo.toml` and `manifest.toml`, with a plain-language `CHANGELOG.md` entry at the top.
Not built in this round: the lead runs the one build and the host tests. Models are reused from `bifrost_app::controls`
(`springboard/ui/src`); no new model and no new crate was added.

## What is now wired

| Model (ui/src) | Used for | Where |
|---|---|---|
| `chart` (`axis_ticks`, scales) | Temperature axis: round tick values, linear scale from the lowest to highest hour | `apps/weather-desktop/src/insights.rs` `temperature_scale` |
| `chart_marks` (`Plot`, `Frame`, `line_path`, `Interpolation::Linear`) | The hourly temperature line and its points, mapped into the card | `insights.rs` `temperature_plot`, `temperature_line`; drawn in `weather.rs` `draw_main_with` |
| `mapview` (`MapView`, `MapCamera`, `MapType`) | Map card: a camera centred on the selected place at zoom 8, the standard map type, scale bar | `insights.rs` `map_view`; `weather.rs` MAP block |
| `geo` (`Coordinate`, `format_distance`, `DistanceUnits`) | Coordinate labels on the grid, scale-bar text in km or miles | `insights.rs` `grid`, `coordinate_label`, `scale_text` |
| `mapview::MapCamera::scale_bar` | The scale bar length | `weather.rs` MAP block |
| `location_auth` (`AuthorizationStatus`, `LocationError`) | Locations footer: plain words on location access, and the error text for a failed location request | `insights.rs` `location_note`; `weather.rs` `place_rows` |

How a user reaches each one:

- **Temperature chart**: open Weather, pick a place, and scroll the forecast below the detail tiles. The card is titled
  TEMPERATURE. The temperatures are on the left in the chosen unit, and the first hour ("Now") and last hour are labelled
  along the bottom.
- **Map**: below the chart, a card titled MAP. It shows latitude and longitude lines (labelled N/S and E/W) around the
  selected place, a white centre mark for the place, and a scale bar at the lower right. The note under it says the map
  is the standard type with no tiles loaded.
- **Location access**: Locations (the Places screen), at the end of the list. The footer says
  "Location: not asked. Weather uses the places you add, not this PC's position." The shell has no location service yet,
  so the status is fixed at not asked (`LOCATION` in `weather.rs`).

## Behaviour decisions

- The map draws no tiles: no network map source exists, and the note says so instead of drawing a fake street map.
  The map type is shown as `Standard`; `MapInteraction` and `MapView` gestures are not wired (no pinch, drag or
  rotate yet).
- The chart is a line only. The chart kit's `chart_content` builder is not used; the card is drawn with the
  `chart_marks` geometry and the `chart` scale.
- Location errors: `location_note(status, Some(error))` maps the error cases the app would show (denied, prompt
  declined, unknown location, network, and the generic user-action and transient groups). Nothing in this app raises a
  location error yet, so the error branch is covered by the unit tests only.
- The app does not request location. Adding a location permission needs the shell's permission list
  (`springboard/src`, outside this app), which this round does not touch.

## How to verify on Midget (the test PC)

1. After the lead's release: `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.weather.desktop`.
2. Take `bifrost pc midget screen /tmp/weather-main.png`. Scroll the forecast down with `bifrost pc midget swipe` (up);
   the TEMPERATURE card and the MAP card appear under the detail tiles. Check the line runs left to right in hour order and
   the tick labels match the forecast's high and low.
3. Change the unit with `bifrost pc midget tap-window` on the Celsius or Fahrenheit button (Locations screen), then check the
   scale bar text changes between km and miles and the chart labels change to °F.
4. Open Locations (`tap-window` on the Locations bar item) and scroll to the end of the list: the location note is the last line.
5. Light and Dark: `bifrost pc midget appearance light` and `dark`; the card fills and white text stay readable on the sky.
6. Host preview: `bifrost pc smoke weather-desktop --sizes 1200x780 --scale 1.5625` (and `--light`).

Host unit tests (for the lead, not run here): `cargo test -p bifrost-weather --offline` in `springboard/`. The new tests are
in `insights.rs` (scale, points, line mapping, map camera, grid order, labels, scale text, location note, map note) and
`weather.rs` `places_end_with_the_location_note`.

## REA evidence (official Apple documentation, rendered)

- `ev_14a22f4ec39c7efbbb6f1329c8b1cb25cbc87ee56766d1454fc8ec238ceebddd`: MKMapView page (standard, hybrid and imagery
  map configurations; centring on coordinates and the region size).
- `ev_2602ca34fc700c3ff8adab564c67dcbcb024ae39c0763797663bf5e6955b5a6c`: CLAuthorizationStatus page (notDetermined,
  restricted, denied, authorized always and when in use; the usage-description keys).
- `ev_8aa44299c418dbd6fe535ac212b249592985e45c79b68d49464c3e48827548ec`: Swift Charts page (marks, scales, axes, and
  the localization and accessibility features).

DocC (`python3 -I tools/parity/docc.py`): `mapkit`, `mapkit/mkmapview`, `charts`, `corelocation`, `corelocation/clauthorizationstatus`.

## Remaining gaps

- No map tiles, no map gestures (pinch, drag, rotate, tilt), no user-location annotation and no pins or overlays.
- No location service and no location permission: the status is always "not asked", so the granted and denied states are
  reachable only through the model's unit tests. The shell's permission list has no location entry.
- No chart accessibility: no `chart_ax` descriptor, so the chart has no spoken summary or per-point description.
- No Bar, area or scatter marks in the forecast chart; no tap-to-read values on the chart (`chart_marks::Plot::value`
  exists but is not hooked up).
- Snapshot images (`tests/snapshots/`) do not include the new cards below the fold; the lead should check whether the
  compliance test measures the full scroll height.

## Matrix rows touched

Rows are not edited here (the matrix is shared). The lead updates them in the same round as the release.

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| mapkit | Map view (MKMapView, Map) | `springboard/apps/weather-desktop/src/insights.rs`, `weather.rs` (MAP card) |
| mapkit | Map controls (compass, zoom, scale, user tracking) | `insights.rs` `map_view`; scale bar in `weather.rs` |
| mapkit | Map camera and region | `insights.rs` `map_view`, `grid` |
| core-location | Authorization and usage descriptions | `insights.rs` `location_note`; `weather.rs` `LOCATION`, `place_rows` |
| core-location | Errors and diagnostics | `insights.rs` `location_note` (error branch, unit tests only) |
| swift-charts | Chart view and content builder | `insights.rs` `temperature_plot`, `temperature_line`; `weather.rs` TEMPERATURE card |
| swift-charts | Scales, domains and ranges | `insights.rs` `temperature_scale` |

Rows left at their current status: `mapkit` "User location on a map", `swift-charts` "Bar marks" and "Accessibility and
localization of charts" (not wired by this round).
