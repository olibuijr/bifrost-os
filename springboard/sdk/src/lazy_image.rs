//! Lazily evaluated images for apps: an image is a source plus the named filters applied to it, and its pixels are computed
//! only when a render context renders it. The model is `bifrost_render::lazy_image`; this module is the app-facing face of
//! it. It re-exports the image and the context, and it builds a filter with its parameters set in one call, so an app
//! composes a chain without reaching into the renderer's effect table.
//!
//! The pixels come from the CPU renderer. No GPU texture is made here, and the shell does not yet draw a lazy image in a
//! window; an app renders it to pixels and draws those with the kit's surface.
//!
//! ```ignore
//! let ctx = RenderContext::new();
//! let darker = LazyImage::new(photo).apply(filter("exposure", &[("ev", -1.0)]).unwrap());
//! let pixels = ctx.render(&darker); // None only when the source's pixel count does not match its size
//! ```

pub use bifrost_render::effect_chain::Filter;
pub use bifrost_render::lazy_image::{LazyImage, RenderContext};

/// The named filter `name` with each parameter in `params` set, or `None` when the name is not a filter or a parameter is
/// not one that filter has. Values are stored as given; see [`Filter::set_clamped`] for the range-limited form.
pub fn filter(name: &str, params: &[(&str, f32)]) -> Option<Filter> {
    let mut built = Filter::named(name)?;
    for (key, value) in params {
        if !built.set(key, *value) {
            return None;
        }
    }
    Some(built)
}

// ---------------------------------------------------------------- the general graph

/// The general lazy graph: nodes with any number of inputs (a source, a generator, one input mapped through an
/// operation, two inputs combined) and a context that renders them with a cache. `LazyImage` above is the
/// single-chain form; use a graph when two images meet (a blend, a transition, a displacement map) or when the
/// operations are the SDK's own filters rather than the named chain's.
pub use bifrost_render::lazy_graph::{GraphContext, GraphImage, MAX_DEPTH};

use crate::image_filter::{blend, Blend, FilterChain};

/// A graph node that shows the lazy image `lazy`, rendered by the chain renderer when the graph is rendered.
pub fn graph_from_lazy(lazy: &LazyImage) -> GraphImage {
    let lazy = lazy.clone();
    GraphImage::generate("lazy image", move || RenderContext::new().render(&lazy))
}

/// A graph node that applies the named filter `name` with `params` to `image`, or `None` when the name or a
/// parameter is not one the chain knows (the same rule as [`filter`]).
pub fn graph_filter(image: &GraphImage, name: &str, params: &[(&str, f32)]) -> Option<GraphImage> {
    let mut chain = FilterChain::new();
    chain.push(filter(name, params)?);
    Some(image.map(name, move |img| chain.apply(img)))
}

/// A graph node that composites `foreground` over `background` under `mode`. The two must have the same size
/// when rendered, or the render gives `None`.
pub fn graph_blend(foreground: &GraphImage, background: &GraphImage, mode: Blend) -> GraphImage {
    foreground.combine(background, "blend", move |fg, bg| blend(fg, bg, mode))
}

#[cfg(test)]
mod graph_tests {
    use super::*;
    use bifrost_render::gfx::Image;

    const PIXEL: u32 = 0xff34_64c8;

    fn one(px: u32) -> Image {
        Image { w: 1, h: 1, px: vec![px] }
    }

    #[test]
    fn a_named_filter_node_matches_the_chain_renderer() {
        let node = graph_filter(&GraphImage::from_image(one(PIXEL)), "exposure", &[("ev", -1.0)]).unwrap();
        assert_eq!(GraphContext::new().render(&node).unwrap().px, vec![0xff1a_3264]);
        assert!(graph_filter(&GraphImage::from_image(one(PIXEL)), "no such filter", &[]).is_none());
        assert!(graph_filter(&GraphImage::from_image(one(PIXEL)), "exposure", &[("nope", 1.0)]).is_none());
    }

    #[test]
    fn a_lazy_image_can_feed_a_graph() {
        let lazy = LazyImage::new(one(PIXEL)).apply(filter("exposure", &[("ev", -1.0)]).unwrap());
        let node = graph_from_lazy(&lazy);
        assert_eq!(GraphContext::new().render(&node).unwrap().px, vec![0xff1a_3264]);
    }

    #[test]
    fn an_opaque_foreground_covers_the_background_and_a_size_mismatch_fails() {
        let fg = GraphImage::from_image(one(0xff00_00ff));
        let bg = GraphImage::from_image(one(0xff00_ff00));
        let out = GraphContext::new().render(&graph_blend(&fg, &bg, Blend::SourceOver)).unwrap();
        assert_eq!(out.px, vec![0xff00_00ff]);
        let big = GraphImage::from_image(Image { w: 2, h: 1, px: vec![0; 2] });
        assert!(GraphContext::new().render(&graph_blend(&fg, &big, Blend::SourceOver)).is_none());
    }
}
