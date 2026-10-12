//! SDF shapes: one instanced-quad pipeline (`gpu/shaders/shape.vert`, `shape.frag`) draws rects, rounded rects,
//! strokes, shadows and linear gradients. Each instance carries its rect, radius, kind, colours, the transform and
//! the active rounded clip; the fragment shader evaluates the rounded-rect distance for coverage (1 px antialiasing),
//! the clip distance, and the Gaussian-shadow approximation (erf of the distance).
//!
//! Owner: wave G1 agent `gfx2d-shapes`.

use bifrost_gpu::{Device, RenderPipeline, Result};

/// Shape kinds (the shader's `kind` switch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Fill = 0,
    Stroke = 1,
    Shadow = 2,
    LinearGradient = 3,
    /// Samples a texture (images and glyphs reuse the instance layout with `kind` image or mask).
    Image = 4,
    Mask = 5,
}

/// One instance, laid out for the vertex buffer (std430-compatible, 112 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Instance {
    /// x, y, w, h of the shape in local pixels.
    pub rect: [f32; 4],
    /// corner radius, stroke width (or shadow sigma), kind, unused
    pub params: [f32; 4],
    /// straight RGBA colour 0..1 (premultiplied in the shader); gradient start colour
    pub color: [f32; 4],
    /// gradient end colour
    pub color2: [f32; 4],
    /// transform a, b, c, d
    pub xform: [f32; 4],
    /// transform tx, ty; gradient start x, y
    pub xform_t: [f32; 4],
    /// clip x, y, w, h in device pixels (w = 0: no clip)
    pub clip: [f32; 4],
}

/// The pipelines for shapes.
pub struct Shapes {
    pub(crate) pipeline: RenderPipeline,
}

impl Shapes {
    pub fn new(device: &Device, target: bifrost_gpu::PixelFormat) -> Result<Shapes> {
        let _ = (device, target);
        todo!("gfx2d-shapes")
    }
}
