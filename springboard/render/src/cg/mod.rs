//! The 2D drawing stack: paths, rasteriser, colour, paint, images and the drawing context.
//! `path`, `raster`, `color` and `paint` carry only the minimal surface the context needs until the
//! agents that own them merge (see the notes at the top of each file).
pub mod color;
pub mod context;
pub mod paint;
pub mod path;
pub mod raster;
