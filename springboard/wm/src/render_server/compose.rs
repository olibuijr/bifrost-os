//! Scene composition: turns the window stack (background and bottom layer-shell surfaces, windows back to front,
//! top and overlay layer-shell surfaces, the cursor) into textured quads with transforms, opacity, rounded clips and
//! materials, culled and scissored to the output's damage, and records them into one render pass with
//! `gpu/shaders/compose.vert` + `compose.frag` (premultiplied textured quad with rounded-rect clip and opacity).
//! Layer-tree surfaces are drawn from `layers::Context::presentation` (each presented layer with contents is a quad
//! of its bound texture; background colours and borders are SDF quads; shadows and materials come from `material`).
//! Damage: per output, the union of surface damage (transformed), layer tree damage and moved/opened/closed windows,
//! accumulated over the buffer age of the target.
//!
//! Owner: wave G1 agent `wm-compose`.

use bifrost_gpu::Texture;

/// One thing to draw, in output pixels, back to front.
#[derive(Clone)]
pub struct DrawItem {
    pub texture: Option<Texture>,
    /// Destination quad corners after transform: top-left, top-right, bottom-left, bottom-right.
    pub quad: [[f32; 2]; 4],
    /// Source rectangle in texels.
    pub src: [f32; 4],
    pub opacity: f32,
    /// Solid colour (premultiplied RGBA) when there is no texture.
    pub color: [f32; 4],
    /// Rounded clip in output pixels (x, y, w, h, radius); w = 0 disables it.
    pub clip: [f32; 5],
    /// A material drawn behind this item first (blur of what is already composited).
    pub material: Option<crate::render_server::material::MaterialRegion>,
    /// A server-drawn drop shadow behind this item.
    pub shadow: Option<crate::render_server::material::ShadowSpec>,
}

/// The output's damage as rectangles in output pixels.
pub type DamageRects = Vec<[i32; 4]>;

/// Builds the damage for a frame from the pieces (pure, host-tested): union, clipped to the output, merged when
/// overlapping, collapsed to the bounding box above 16 rectangles.
pub fn merge_damage(rects: &[[i32; 4]], output: (i32, i32)) -> DamageRects {
    let _ = (rects, output);
    todo!("wm-compose")
}
