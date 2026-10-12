# Wave 4, theme 19: map view, place details, GeoJSON, map links, location reminders

Theme 19 ("Map view (MKMapView, Map)") groups seven gap rows across MapKit, EventKit and WidgetKit. This round closes
four of them with partial models. Nothing is wired into an app or the shell, and nothing is tested on the VM or Midget.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/core/src/geojson.rs` | `MKGeoJSONDecoder`, `MKGeoJSONFeature`, `MKGeoJSONObject` (decoding only) | Parses GeoJSON text (Feature, FeatureCollection, GeometryCollection, the six geometry types) into a flat `Vec<Feature>`. Positions are longitude then latitude with an optional altitude. Uses the existing `core::json` reader. Rejects bad coordinates, short position lists and unclosed rings. |
| `springboard/ui/src/map_item.rs` | `MKMapItem` (name, phone number, URL, time zone, location, address), the detail rows of `MKMapItemDetailViewController` | `MapItem` holds one place. `detail_rows()` returns address, phone, website, time zone and coordinates in that order, leaving out empty attributes. The address comes from the existing `postal_address::Address`. |
| `springboard/ui/src/maps_link.rs` | Launch options of `MKMapItem.openMaps(with:launchOptions:)` (search, centre, zoom, map style) | `MapLink` writes and reads a `maps://` URL with `q`, `ll`, `z` and `t`. Parsing is strict about zoom, centre and style, and tolerant of unknown keys. No service stands behind the scheme, and the default app is not set. |
| `springboard/ui/src/location_reminder.rs` | `EKStructuredLocation` (title, geo location, radius) and location-based alarms | `StructuredLocation` is a titled coordinate with a radius in metres. `LocationAlarm` chooses arriving or leaving and produces a `CircularRegion` for the existing `geofence` monitor, plus `fires()` for the matching enter or exit event. |

Each new module has a `#[cfg(test)] mod tests` block (7 tests in `geojson.rs`, 7 in `maps_link.rs`, 6 in `map_item.rs`,
6 in `location_reminder.rs`). Expected values were worked out by hand from the rules in each module. Module registration:
one `pub mod` line in `springboard/core/src/lib.rs` and three in `springboard/ui/src/lib.rs`.

No build or test was run in this round, as the wave rules require.

## Official API names and evidence

Documentation facts come from the rendered official pages recorded with REA. The DocC JSON path
(`python3 -I tools/parity/docc.py documentation/mapkit/mkgeojsondecoder`) returned HTTP 404, so the DocC read was not used.

| Official page | REA evidence id | Facts used |
|---|---|---|
| MapKit, `MKGeoJSONDecoder` | `ev_fb2ccba9ca17118f6a4e4b57670adf6e4d3df97af963403680246b8709ab48ef` | Decodes data into `[any MKGeoJSONObject]` with `decode(_:)`. The decoded objects are native map types. |
| MapKit, `MKMapItem` | `ev_340a94c582a8dc689d3012d948f5d7a957d64fbe299246c06208e1a0fd50b937` | Attributes: name, phone number, URL, time zone, placemark or location and address. Launch with `openMaps(with:launchOptions:)`. |
| EventKit, `EKStructuredLocation` | `ev_1f8cca7cc30ae945a7cb6dae10b5fb8623ca04c2719fe99bff55251d927986c6` | Title, geo location (`CLLocation`) and radius, where the radius is the minimum distance from the location that triggers the alarm. Set on an `EKAlarm`. |

Not recorded: a REA page for the Unified Maps URL parameters, and for `EKAlarm`'s proximity values. The `maps_link.rs`
parameter names and `location_reminder.rs` proximity names are this round's own choices, and the module docs say so.

## Matrix rows closed (status `partial`)

| Framework slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| mapkit | Place information and detail sheets | partial | `springboard/ui/src/map_item.rs` |
| mapkit | Maps URLs and opening in the default map app | partial | `springboard/ui/src/maps_link.rs` |
| mapkit | Indoor maps and GeoJSON | partial | `springboard/core/src/geojson.rs` |
| eventkit | Location-based reminders | partial | `springboard/ui/src/location_reminder.rs` |

## Rows left open, with reasons

- MapKit, "Map view (MKMapView, Map)": no map renderer, tile source or view exists. Building one is outside this theme.
- MapKit, "Map snapshots (static images)": needs the map renderer first.
- WidgetKit, "Location and network access in widgets": needs the widget extension runtime and its permission model,
  which this pure-model round does not cover.

## Gaps that remain in the closed rows

- `geojson.rs` decodes only. It has no indoor map data format (IMDF) reader, no overlay or annotation objects and no
  drawing. Its positions are plain numbers.
- `map_item.rs` has no point-of-interest category, no identifier and no link to a map or placemark. `with_address`
  stores the address as given.
- `maps_link.rs` has no directions parameters, no default-app registration and no opening. The `sdk` workspace API
  still only opens plain URLs.
- `location_reminder.rs` has no reminder store, no position source and no notification. A structured location needs a
  coordinate here, while the official one can be made from a title alone.

## Verification notes

- `python3 tools/check-names.py` reports no hits for the new `.rs` files.
- Tests were not run (no-build rule). The first integration build should run `cargo test` for `bifrost-core` (the
  `geojson` module) and `bifrost-ui` (the three new modules).
