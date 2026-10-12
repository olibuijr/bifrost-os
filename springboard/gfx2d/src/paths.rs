//! Path tessellation on the CPU: curves flattened to within 0.25 px, fills triangulated (non-zero and even-odd via
//! a trapezoid sweep), strokes expanded with round joins and caps. Output is a triangle list in device pixels with
//! per-vertex coverage for 1 px antialiased edges. Pure functions, host-tested.
//!
//! Owner: wave G1 agent `gfx2d-images` (with the image cache).

use bifrost_render::canvas::{PathData, PathPaint};

/// A vertex: position and edge coverage (1 inside, 0 at the antialiased fringe).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathVertex {
    pub x: f32,
    pub y: f32,
    pub coverage: f32,
}

/// Tessellates `path` for `paint` into a triangle list.
pub fn tessellate(path: &PathData, paint: PathPaint) -> Vec<PathVertex> {
    let _ = (path, paint);
    todo!("gfx2d-images")
}
