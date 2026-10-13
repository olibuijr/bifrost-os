//! Maps: the map model behind the Maps app, with no window or network code.
//!
//! - `tiles`: Web Mercator tile math (lat/lon, world pixels, tile ids, the tiles that cover a view).
//! - `source`: the configurable OpenStreetMap-compatible tile URL and the request policy (user agent, pacing).
//! - `cache`: the on-disk tile cache under the app data directory.
//! - `geocode`: a Nominatim-style search request builder and reply parser.
//! - `view`: the map view (centre, fractional zoom, pan, anchored zoom for pinch) and pins.
//! - `route`: directions from an OSRM-compatible server when one is configured, a labelled straight-line estimate
//!   otherwise, and offline place search over a local list.
//! - `units`, `measure`, `gpx`, `links`, `store`: distance units and the scale bar, Measure Distance, GPX import,
//!   `maps:` and `geo:` links, and the saved places (Home, Work, Favorites, Recents, Collections).
//! - `net`, `loader`: the HTTP client with the identifying user agent and the tile loader (cache, pacing, decode).
//! - `state`: everything the window shows and every command, with no drawing, so it is tested without a screen.

pub mod cache;
pub mod geocode;
pub mod gpx;
pub mod links;
pub mod loader;
pub mod measure;
pub mod net;
pub mod route;
pub mod rpc;
pub mod source;
pub mod state;
pub mod store;
pub mod tiles;
pub mod units;
pub mod view;
pub mod window;
