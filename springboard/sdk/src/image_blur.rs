//! Blur filters for app images (Core Image's blur category) and the SwiftUI `.blur(radius:opaque:)`
//! effect. A [`BlurFilter`] names the filter and its parameters; `apply` runs it on a decoded
//! `Image`, for example the one from `ImageSource::decode` or `thumbnail`. [`ViewBlur`] is the
//! view-level effect value: radius in points, the `opaque` flag, and the display scale that turns
//! points into pixels. The pixel work lives in `bifrost_render::blur_filters`.
//! Gap: the retained-view tree has no offscreen layer yet, so a `View` cannot be blurred as a
//! group; an app draws the view into an `Image` and applies `ViewBlur::apply` (see docs/bifrost-ui.md).
use bifrost_render::blur_filters as f;
pub use bifrost_render::blur_filters::Edge;
use bifrost_render::gfx::Image;

/// One of the Core Image blur filters with its input parameters (pixels, degrees).
#[derive(Clone, Debug, PartialEq)]
pub enum BlurFilter {
    /// `CIGaussianBlur`, `inputRadius` is the standard deviation.
    Gaussian { radius: f32 },
    /// `CIBoxBlur`.
    Box { radius: f32 },
    /// `CIMotionBlur`, angle counter-clockwise from +x.
    Motion { radius: f32, angle_deg: f32 },
    /// `CIZoomBlur`, `center` in pixels, `amount` the shift at the farthest corner.
    Zoom { center: (f32, f32), amount: f32 },
    /// `CIDiscBlur`.
    Disc { radius: f32 },
    /// `CIBokehBlur`; the reference defaults are radius 20, ring amount 0, ring size 0.1, softness 1.
    Bokeh { radius: f32, ring_amount: f32, ring_size: f32, softness: f32 },
}

impl BlurFilter {
    /// `CIBokehBlur` with the reference default inputs.
    pub fn bokeh_default() -> Self {
        BlurFilter::Bokeh { radius: 20.0, ring_amount: 0.0, ring_size: 0.1, softness: 1.0 }
    }

    /// Runs the filter. `Edge::Clamp` is `clampedToExtent`; `Edge::Transparent` fades the border.
    pub fn apply(&self, img: &Image, edge: Edge) -> Image {
        match *self {
            BlurFilter::Gaussian { radius } => f::gaussian_blur(img, radius, edge),
            BlurFilter::Box { radius } => f::box_blur(img, radius, edge),
            BlurFilter::Motion { radius, angle_deg } => f::motion_blur(img, radius, angle_deg, edge),
            BlurFilter::Zoom { center, amount } => f::zoom_blur(img, center, amount, edge),
            BlurFilter::Disc { radius } => f::disc_blur(img, radius, edge),
            BlurFilter::Bokeh { radius, ring_amount, ring_size, softness } => f::bokeh_blur(img, radius, ring_amount, ring_size, softness, edge),
        }
    }
}

/// `CIMaskedVariableBlur`: the blur radius at each pixel is `radius` times the mask's alpha.
pub fn masked_variable_blur(img: &Image, mask: &Image, radius: f32, edge: Edge) -> Image {
    f::masked_variable_blur(img, mask, radius, edge)
}

/// The `.blur(radius:opaque:)` view effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewBlur {
    /// Radius in points.
    pub radius: f32,
    /// Fully opaque result with clamped edges (the reference's `opaque: true`).
    pub opaque: bool,
}

impl ViewBlur {
    pub fn new(radius: f32) -> Self {
        ViewBlur { radius, opaque: false }
    }

    pub fn opaque(mut self, opaque: bool) -> Self {
        self.opaque = opaque;
        self
    }

    /// Blurs the rendered view. `scale` is device pixels per point.
    pub fn apply(&self, rendered: &Image, scale: f32) -> Image {
        f::view_blur(rendered, self.radius, scale, self.opaque)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_dispatch_and_zero_radius_is_identity() {
        let mut img = Image::new(9, 9);
        img.px[40] = 0xffff_ffff;
        for flt in [BlurFilter::Gaussian { radius: 0.0 }, BlurFilter::Box { radius: 0.0 }, BlurFilter::Disc { radius: 0.0 }, BlurFilter::Motion { radius: 0.0, angle_deg: 0.0 }] {
            assert_eq!(flt.apply(&img, Edge::Clamp), img);
        }
        assert_ne!(BlurFilter::Gaussian { radius: 1.5 }.apply(&img, Edge::Transparent), img);
        assert_eq!(BlurFilter::bokeh_default(), BlurFilter::Bokeh { radius: 20.0, ring_amount: 0.0, ring_size: 0.1, softness: 1.0 });
    }

    #[test]
    fn view_blur_uses_points_times_scale() {
        let mut img = Image::new(33, 33);
        img.px[16 * 33 + 16] = 0xffff_ffff;
        let a = ViewBlur::new(1.0).apply(&img, 2.0);
        let b = BlurFilter::Gaussian { radius: 2.0 }.apply(&img, Edge::Transparent);
        assert_eq!(a, b);
        assert!(ViewBlur::new(1.0).opaque(true).apply(&img, 1.0).px.iter().all(|p| p >> 24 == 255));
    }
}
