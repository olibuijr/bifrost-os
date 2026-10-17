//! Map tiles, shared by the Maps app and the shell (Photos draws its Places pins over them):
//!
//! - `tiles`: Web Mercator tile math (lat/lon, world pixels, tile ids, the tiles that cover a view).
//! - `source`: the OpenStreetMap-compatible tile URL and the request policy (user agent, pacing).
//! - `cache`: the on-disk tile cache and the offline area rectangle.
//! - `net`: the one-GET network trait, its live client and the test fake.
//! - `loader`: the tile loader (cache first, then the network at the policy's pace, decoded into images).
//!
//! The loader fetches on its own threads; the caller polls `TileLoader::pump` from its UI loop.

pub mod cache;
pub mod loader;
pub mod net;
pub mod source;
pub mod tiles;
