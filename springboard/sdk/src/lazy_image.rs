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
