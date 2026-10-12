//! The executor: walks a display list in order, keeps the clip/transform/opacity stacks, turns ops into batched
//! instances (`shapes::Instance`, `text` glyph quads, `images` quads, `paths` triangles), uploads what is new and
//! records one render pass into the target. Consecutive ops with the same pipeline and textures share a draw.
//!
//! Owner: wave G1 agent `gfx2d-renderer`.

use bifrost_gpu::{CommandBuffer, Device, Result, Texture};
use bifrost_render::canvas::RectF;
use bifrost_render::display_list::DisplayList;

/// What one `draw` did, for the bench and the perf HUD.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameStats {
    pub ops: u32,
    pub draws: u32,
    pub instances: u32,
    pub glyphs_uploaded: u32,
    pub images_uploaded: u32,
    /// CPU time spent encoding, in microseconds.
    pub encode_us: u32,
}

/// Owns the pipelines, the glyph atlas and the image cache for one device. One per process is enough.
pub struct Renderer {
    pub(crate) device: Device,
    pub(crate) shapes: crate::shapes::Shapes,
    pub(crate) text: crate::text::GlyphAtlas,
    pub(crate) images: crate::images::ImageCache,
}

impl Renderer {
    pub fn new(device: &Device) -> Result<Renderer> {
        let _ = device;
        todo!("gfx2d-renderer")
    }

    /// Records drawing `list` into `target` (premultiplied BGRA8 or RGBA8). With `damage` only that region is
    /// cleared and redrawn (scissored); `None` redraws everything.
    pub fn draw(&mut self, cb: &mut CommandBuffer, list: &DisplayList, target: &Texture, damage: Option<RectF>) -> Result<FrameStats> {
        let _ = (cb, list, target, damage);
        todo!("gfx2d-renderer")
    }
}
