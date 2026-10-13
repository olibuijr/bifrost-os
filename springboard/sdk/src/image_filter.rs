//! Core Image style filters for app images: named colour filters chained in order (`CIFilter` and
//! its chaining), blend modes as two-input composites (`CISourceOverCompositing`, `CIMultiplyBlendMode`
//! and the other blend filters), a colour matrix (`CIColorMatrix`), gamma (`CIGammaAdjust`) and a
//! linear gradient generator (`CILinearGradient`). Apps decode an image with `ImageSource` and run a
//! filter on the `Image`. The pixel work lives in `bifrost_render::effect_chain` and
//! `bifrost_render::composite_filters`, which the shell uses too. See docs/bifrost-ui.md, "Core Image
//! filters", for the decisions and the gaps (no lazy `CIImage` graph and no render destination yet).
use bifrost_render::composite_filters as c;
use bifrost_render::effect_chain as e;
use bifrost_render::gfx::Image;

pub use bifrost_render::composite_filters::{blend_names, Blend, ColorMatrix};
pub use bifrost_render::effect_chain::{filter_names, filters_in, search, Category, Filter, InputRange};

/// An ordered chain of named colour filters. `apply` runs each filter over the whole image in turn,
/// as a `CIFilter` chain does when each filter takes the previous one's output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterChain {
    chain: e::Chain,
}

impl FilterChain {
    /// An empty chain, which returns its input unchanged.
    pub fn new() -> Self {
        FilterChain::default()
    }

    /// Adds `filter` to the end of the chain.
    pub fn push(&mut self, filter: Filter) {
        self.chain.push(filter);
    }

    /// The number of filters in the chain.
    pub fn len(&self) -> usize {
        self.chain.len()
    }

    /// Whether the chain has no filters.
    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// Runs the chain over `img`. Returns `None` when the image's pixel count does not match its size.
    pub fn apply(&self, img: &Image) -> Option<Image> {
        let px = self.chain.apply(&img.px, img.w, img.h)?;
        Some(Image { w: img.w, h: img.h, px })
    }
}

/// Composites `foreground` over `background` under `mode`. Both images are premultiplied and must have
/// the same size; returns `None` when they do not match. The result is the size of the inputs.
pub fn blend(foreground: &Image, background: &Image, mode: Blend) -> Option<Image> {
    if foreground.w != background.w || foreground.h != background.h {
        return None;
    }
    let px = c::composite(&foreground.px, &background.px, foreground.w, foreground.h, mode)?;
    Some(Image { w: foreground.w, h: foreground.h, px })
}

/// Applies a colour matrix to every pixel of `img` (`CIColorMatrix`).
pub fn color_matrix(img: &Image, matrix: &ColorMatrix) -> Image {
    Image { w: img.w, h: img.h, px: matrix.apply(&img.px) }
}

/// Raises each colour channel of `img` to `power` (`CIGammaAdjust`; the default power is 1).
pub fn gamma(img: &Image, power: f32) -> Image {
    Image { w: img.w, h: img.h, px: c::gamma(&img.px, power) }
}

/// A `w` by `h` image that runs from `color0` at `p0` to `color1` at `p1` (pixel coordinates), with
/// the end colours held outside the segment (`CILinearGradient`). Colours are packed RGBA.
pub fn linear_gradient(w: usize, h: usize, p0: (f32, f32), p1: (f32, f32), color0: u32, color1: u32) -> Image {
    Image { w, h, px: c::linear_gradient(w, h, p0, p1, color0, color1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: u32) -> Image {
        Image { w, h, px: vec![px; w * h] }
    }

    #[test]
    fn empty_chain_returns_the_image_unchanged() {
        let img = solid(2, 2, 0xff11_2233);
        assert_eq!(FilterChain::new().apply(&img), Some(img.clone()));
        assert!(FilterChain::new().is_empty());
    }

    #[test]
    fn chain_runs_filters_in_order() {
        // Two inverts cancel out.
        let img = solid(1, 1, 0xff0a_141e);
        let mut chain = FilterChain::new();
        chain.push(Filter::named("invert").unwrap());
        chain.push(Filter::named("invert").unwrap());
        assert_eq!(chain.len(), 2);
        assert_eq!(chain.apply(&img), Some(img));
    }

    #[test]
    fn chain_refuses_an_image_whose_pixels_do_not_match_its_size() {
        let img = Image { w: 2, h: 2, px: vec![0; 3] };
        assert_eq!(FilterChain::new().apply(&img), None);
    }

    #[test]
    fn blend_refuses_images_of_different_sizes() {
        let a = solid(2, 2, 0xff80_8080);
        let b = solid(2, 1, 0xffff_ffff);
        assert_eq!(blend(&a, &b, Blend::Multiply), None);
    }

    #[test]
    fn multiply_blend_keeps_the_size_of_its_inputs() {
        let a = solid(2, 1, 0xff80_8080);
        let b = solid(2, 1, 0xffff_ffff);
        let out = blend(&a, &b, Blend::Multiply).unwrap();
        assert_eq!((out.w, out.h, out.px.len()), (2, 1, 2));
        assert!(((out.px[0] & 0xff) as i32 - 128).abs() <= 1);
    }

    #[test]
    fn color_matrix_and_gamma_keep_the_size() {
        let img = solid(3, 2, 0xff80_8080);
        let swapped = color_matrix(&img, &ColorMatrix::identity());
        assert_eq!(swapped, img);
        let g = gamma(&img, 1.0);
        assert_eq!((g.w, g.h, g.px.len()), (3, 2, 6));
    }

    #[test]
    fn linear_gradient_returns_an_image_of_the_requested_size() {
        let g = linear_gradient(4, 3, (0.0, 0.0), (4.0, 0.0), 0xff00_0000, 0xffff_ffff);
        assert_eq!((g.w, g.h, g.px.len()), (4, 3, 12));
    }

    #[test]
    fn named_filters_are_reexported_from_the_renderer() {
        assert!(filter_names().contains(&"exposure"));
        assert_eq!(Blend::named("screen"), Some(Blend::Screen));
        assert!(blend_names().contains(&"sourceOver"));
    }
}
