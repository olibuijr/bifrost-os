//! `CGLayer` for apps: an offscreen group of drawing that is built once and drawn into a context many times. A layer
//! keeps its own premultiplied pixels and a draw count. Draw it into a context with
//! `GraphicsContext::draw_layer_at(layer.pixels(), dx, dy)`, which applies the context's clip, opacity and blend mode.
//!
//! ```ignore
//! let mut badge = CgLayer::new(16, 16).unwrap();
//! badge.draw(|ctx| ctx.fill(&Path::rect(0.0, 0.0, 16.0, 16.0), 0xff00_0000, FillRule::NonZero));
//! page.draw_layer_at(badge.pixels(), 20, 8);
//! ```

use crate::graphics_context::GraphicsContext;
use bifrost_render::effects::Layer;

/// An offscreen layer with a draw count.
#[derive(Clone, Debug, PartialEq)]
pub struct CgLayer {
    pixels: Layer,
    draws: usize,
}

impl CgLayer {
    /// A transparent layer of `w` by `h` pixels. `None` when either side is zero or the pixel count overflows.
    pub fn new(w: usize, h: usize) -> Option<CgLayer> {
        if w == 0 || h == 0 {
            return None;
        }
        w.checked_mul(h)?;
        Some(CgLayer { pixels: Layer::new(w, h), draws: 0 })
    }

    /// The width and height of the layer, in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.pixels.w, self.pixels.h)
    }

    /// The number of completed draw passes. Each pass adds to what earlier passes left in the layer.
    pub fn draws(&self) -> usize {
        self.draws
    }

    /// The layer's premultiplied pixels, for `GraphicsContext::draw_layer_at`.
    pub fn pixels(&self) -> &Layer {
        &self.pixels
    }

    /// Runs `f` on a context whose target is this layer's pixels, then keeps the result. The context starts with an
    /// identity transform and no clip; whatever `f` does not paint stays as it was.
    pub fn draw(&mut self, f: impl FnOnce(&mut GraphicsContext)) {
        let (w, h) = self.size();
        let mut ctx = GraphicsContext::new(w, h);
        ctx.layer = self.pixels.clone();
        f(&mut ctx);
        self.pixels = ctx.layer;
        self.draws += 1;
    }
}
