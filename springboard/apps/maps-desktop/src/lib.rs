//! Maps: the map model behind the Maps app, with no window or network code.
//!
//! - `tiles`: Web Mercator tile math (lat/lon, world pixels, tile ids, the tiles that cover a view).
//! - `view`: the map view (centre, fractional zoom, pan, anchored zoom for pinch) and pins.

pub mod tiles;
pub mod view;
