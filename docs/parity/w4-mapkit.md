# Wave 4: MapKit (w4-mapkit)

Two MapKit gap rows are now partial, both as pure Rust models with host unit tests. Neither is wired into an app or
the shell yet, and neither has been run on the VM or Midget.

## What was built

- `springboard/core/src/geojson.rs` (`bifrost_core::geojson`): decodes UTF-8 GeoJSON bytes into `GeoJsonObject`
  values (`Feature`, `FeatureCollection`, bare `Geometry`). A document is one object or an array of objects. Positions
  are checked for range (longitude -180..=180, latitude -90..=90). Line strings need two positions, and polygon rings
  need four positions and must be closed. Unsupported geometry types are rejected, including `GeometryCollection`.
  The module uses the crate's own JSON reader (`springboard/core/src/json.rs`).
- `springboard/ui/src/map_item.rs` (`bifrost_ui::map_item`): `MapItem`, a placemark (the existing `reverse_lookup::Place`)
  with optional phone number, web address and point-of-interest category, plus a current-location flag. Setters return
  `None` for invalid input. Categories are checked against `poi_filter::KNOWN_CATEGORIES`.

Each module has its own `#[cfg(test)] mod tests`. The core crate has 8 GeoJSON tests and the UI crate has 6 map item
tests. Their expected values were worked out by hand from the rules above.

## Official API names mirrored

- `MKGeoJSONDecoder.decode(_:)` (returns `[any MKGeoJSONObject]`), `MKGeoJSONFeature` (`identifier`, `geometry`,
  `properties`), `MKGeoJSONObject`. Source: https://developer.apple.com/documentation/mapkit/mkgeojsondecoder
- `MKMapItem` (`name`, `placemark`, `phoneNumber`, `url`, `pointOfInterestCategory`, `isCurrentLocation`,
  `init(placemark:)`, `forCurrentLocation()`). Source: https://developer.apple.com/documentation/mapkit/mkmapitem
- The landing page https://developer.apple.com/documentation/mapkit, used to confirm the MapKit article list.

Not built: IMDF parsing, `openMaps` and the Maps app handoff (no maps scheme or default navigation app exists in
the workspace API), and the detail sheet UI.

## REA evidence

| Page | evidence_id |
|---|---|
| https://developer.apple.com/documentation/mapkit | ev_4930e9e24b211f8e80632bd2453f52615ea9fd7ba23578f9af6e4079715b5dac |
| https://developer.apple.com/documentation/mapkit/mkgeojsondecoder | ev_9cca0a34dd518343ab957ce98a7d3a5a75d00fb892c6f904fd62746f0053301d |
| https://developer.apple.com/documentation/mapkit/mkmapitem | ev_5503be2654c1bfbd4c0ab152ea57c1671ad1dc84eba224380bd6d6ec98c2384c |

The Unified Maps URLs article (`mapkit/adopting-unified-maps-urls`) was not fetched: the DocC path returned 404, so the
maps-URL row stays a gap.

## Matrix rows closed (status partial)

| framework | feature | status | BifrOSt path |
|---|---|---|---|
| MapKit | Indoor maps and GeoJSON | partial | springboard/core/src/geojson.rs |
| MapKit | Place information and detail sheets | partial | springboard/ui/src/map_item.rs |

Wiring: `pub mod geojson;` in `springboard/core/src/lib.rs`, and `pub mod map_item;` in `springboard/ui/src/lib.rs`.
