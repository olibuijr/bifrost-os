//! Maps: the map model behind the Maps app, with no window or network code.
//!
//! - `tiles`: Web Mercator tile math (lat/lon, world pixels, tile ids, the tiles that cover a view).
//! - `source`: the configurable OpenStreetMap-compatible tile URL and the request policy (user agent, pacing).
//! - `cache`: the on-disk tile cache under the app data directory.
//! - `geocode`: a Nominatim-style search request builder and reply parser.
//! - `view`: the map view (centre, fractional zoom, pan, anchored zoom for pinch) and pins.

pub mod cache;
pub mod geocode;
pub mod source;
pub mod tiles;
pub mod view;
