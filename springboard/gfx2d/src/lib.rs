//! `bifrost-2d`: draws a `bifrost_render::display_list::DisplayList` into a `bifrost_gpu::Texture` (a layer's
//! backing store or a window's buffer) (docs/graphics-architecture.md, "bifrost-2d").
//!
//! Module owners (wave G1): `shapes` (SDF rects, rounded rects, strokes, shadows, gradients, clips, transforms and
//! opacity: the instance encoder and its shaders), `text` (glyph atlas and glyph quads), `images` (texture cache) and
//! `paths` (CPU tessellation), `renderer` (the executor that walks ops, batches and submits).

pub mod images;
pub mod paths;
pub mod renderer;
pub mod shapes;
pub mod text;
pub mod uploads;

pub use renderer::{FrameStats, Renderer};
