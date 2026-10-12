//! Lazily evaluated images: an image is a source plus the filters applied to it, and no pixel is computed until a
//! `RenderContext` renders it.
//!
//! `LazyImage::apply` takes the image and returns a new one with one more filter; the filters come from
//! `effect_chain`, so they are the same named filters apps already get. Applying a filter never changes the source
//! or the image it came from, because the image is consumed and its chain is extended. `RenderContext::render` runs
//! the whole chain over the source in one pass and returns packed pixels. The context holds no device and no cache,
//! so a render is always a fresh computation over the current chain.

use crate::effect_chain::{Chain, Filter};
use crate::gfx::Image;

/// A source image and the chain of filters that will be applied to it when it is rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct LazyImage {
    source: Image,
    chain: Chain,
}

impl LazyImage {
    /// An image that shows `source` unchanged until a filter is applied.
    pub fn new(source: Image) -> Self {
        LazyImage { source, chain: Chain::new() }
    }

    /// The image with `filter` added to the end of its chain. The receiver is consumed; clone it first to keep it.
    pub fn apply(mut self, filter: Filter) -> Self {
        self.chain.push(filter);
        self
    }

    /// The width and height of the output, which equal those of the source (filters do not resize).
    pub fn extent(&self) -> (usize, usize) {
        (self.source.w, self.source.h)
    }

    /// The number of filters that a render will run.
    pub fn filter_count(&self) -> usize {
        self.chain.len()
    }
}

/// Renders lazy images to pixels. A context is cheap and holds no state of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderContext;

impl RenderContext {
    /// A render context.
    pub fn new() -> Self {
        RenderContext
    }

    /// Runs the image's filter chain over its source. Returns `None` when the source's pixel count does not match
    /// its width and height.
    pub fn render(&self, image: &LazyImage) -> Option<Image> {
        let (w, h) = image.extent();
        let px = image.chain.apply(&image.source.px, w, h)?;
        Some(Image { w, h, px })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pixel, alpha 255, red 200, green 100, blue 52 (packed red in the low byte).
    const PIXEL: u32 = 0xff34_64c8;

    fn exposure(ev: f32) -> Filter {
        let mut f = Filter::named("exposure").unwrap();
        assert!(f.set("ev", ev));
        f
    }

    fn one_pixel() -> Image {
        Image { w: 1, h: 1, px: vec![PIXEL] }
    }

    #[test]
    fn an_image_with_no_filters_renders_its_source() {
        let ctx = RenderContext::new();
        let image = LazyImage::new(Image { w: 2, h: 1, px: vec![PIXEL, 0] });
        assert_eq!(image.filter_count(), 0);
        assert_eq!(ctx.render(&image), Some(Image { w: 2, h: 1, px: vec![PIXEL, 0] }));
    }

    #[test]
    fn one_stop_of_exposure_halves_each_channel() {
        // 200, 100 and 52 halve to 100, 50 and 26: 0x64, 0x32 and 0x1a.
        let image = LazyImage::new(one_pixel()).apply(exposure(-1.0));
        let out = RenderContext::new().render(&image).unwrap();
        assert_eq!(out.px, vec![0xff1a_3264]);
    }

    #[test]
    fn two_filters_run_in_order_and_quarter_each_channel() {
        // 200, 100 and 52 quarter to 50, 25 and 13: 0x32, 0x19 and 0x0d.
        let image = LazyImage::new(one_pixel()).apply(exposure(-1.0)).apply(exposure(-1.0));
        assert_eq!(image.filter_count(), 2);
        let out = RenderContext::new().render(&image).unwrap();
        assert_eq!(out.px, vec![0xff0d_1932]);
    }

    #[test]
    fn applying_a_filter_leaves_the_earlier_image_unchanged() {
        let ctx = RenderContext::new();
        let base = LazyImage::new(one_pixel());
        let darker = base.clone().apply(exposure(-1.0));
        assert_eq!(base.filter_count(), 0);
        assert_eq!(darker.filter_count(), 1);
        assert_eq!(ctx.render(&base).unwrap().px, vec![PIXEL]);
        assert_eq!(ctx.render(&darker).unwrap().px, vec![0xff1a_3264]);
    }

    #[test]
    fn extent_is_the_source_size_and_a_short_buffer_does_not_render() {
        let image = LazyImage::new(Image { w: 3, h: 2, px: vec![0; 6] });
        assert_eq!(image.extent(), (3, 2));
        let short = LazyImage::new(Image { w: 2, h: 2, px: vec![0; 3] });
        assert_eq!(short.extent(), (2, 2));
        assert!(RenderContext::new().render(&short).is_none());
    }
}
