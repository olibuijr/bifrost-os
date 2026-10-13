//! Content kinds a layer can hold, each self-contained: shape, gradient, text, replicator, emitter, scroll and tiled
//! content. Every kind is plain data plus a rasteriser (`render`) that produces a premultiplied `Image`, so a layer, or
//! one of the views in `view::layer_views`, only has to hold one and draw its image. Scan conversion lives in
//! `bifrost_render::cg::raster`; `raster` here adds the pixel helpers.
pub mod emitter;
pub mod gradient;
pub mod raster;
pub mod replicator;
pub mod scroll;
pub mod shape;
pub mod text;
pub mod tiled;
