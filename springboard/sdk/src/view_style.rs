//! View style properties for apps: corner radius, border, shadow, transform, mask and filters on a view, drawn
//! through any canvas. The state is the shared [`StyledLayer`] model from the UI crate; this adds a builder and a
//! draw call that renders the styled view and hands the pixels to the canvas.
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_render::gfx::Image;
use bifrost_ui::layer_render::{
    render_tree, BlendMode, ColorMatrix, CornerCurve, LayerFilter, Pixmap, ShadowPath, StyledLayer,
};
use bifrost_ui::layer_style::{CornerMask, Shadow};
use bifrost_ui::transform3d::Transform3D;

/// A view's style properties, built fluently and drawn with [`ViewStyle::draw`].
#[derive(Clone, Debug)]
pub struct ViewStyle {
    pub layer: StyledLayer,
}

impl ViewStyle {
    /// A style over the view's own pixels (premultiplied RGBA, red in the low byte).
    pub fn new(content: Pixmap) -> ViewStyle {
        ViewStyle { layer: StyledLayer::new(content) }
    }

    pub fn corner_radius(mut self, r: f32) -> Self {
        self.layer.style.corner_radius = r;
        self.layer.style.masks_to_bounds = true;
        self
    }
    pub fn masked_corners(mut self, m: CornerMask) -> Self {
        self.layer.style.masked_corners = m;
        self
    }
    pub fn continuous_corners(mut self) -> Self {
        self.layer.style.corner_curve = CornerCurve::Continuous;
        self
    }
    pub fn border(mut self, width: f32, color: u32) -> Self {
        self.layer.style.border_width = width;
        self.layer.style.border_color = color;
        self
    }
    pub fn shadow(mut self, color: u32, opacity: f32, radius: f32, offset: (f32, f32)) -> Self {
        self.layer.style.shadow = Shadow { opacity, radius, offset };
        self.layer.style.shadow_color = color;
        self
    }
    pub fn shadow_path(mut self, path: ShadowPath) -> Self {
        self.layer.style.shadow_path = Some(path);
        self
    }
    pub fn transform(mut self, t: Transform3D) -> Self {
        self.layer.transform = t;
        self
    }
    pub fn sublayer_transform(mut self, t: Transform3D) -> Self {
        self.layer.sublayer_transform = t;
        self
    }
    pub fn mask(mut self, mask: Pixmap) -> Self {
        self.layer.style.mask = Some(mask);
        self
    }
    pub fn filter(mut self, f: LayerFilter) -> Self {
        self.layer.style.filters.push(f);
        self
    }
    pub fn blur(self, sigma: f32) -> Self {
        self.filter(LayerFilter::GaussianBlur(sigma))
    }
    pub fn background_blur(self, sigma: f32) -> Self {
        self.filter(LayerFilter::BackgroundBlur(sigma))
    }
    pub fn color_matrix(self, m: ColorMatrix) -> Self {
        self.filter(LayerFilter::ColorMatrix(m))
    }
    pub fn blend(mut self, m: BlendMode) -> Self {
        self.layer.style.blend = m;
        self
    }
    pub fn opacity(mut self, o: f32) -> Self {
        self.layer.style.opacity = o;
        self
    }
    pub fn child(mut self, c: ViewStyle) -> Self {
        self.layer.children.push(c.layer);
        self
    }

    /// Renders the view (and children) with its top-left at `(x, y)` in a `w` by `h` pixel buffer.
    pub fn render(&self, w: usize, h: usize, x: f32, y: f32) -> Pixmap {
        let mut dst = Pixmap::new(w, h);
        let mut layer = self.layer.clone();
        layer.position = (layer.position.0 + x, layer.position.1 + y);
        render_tree(&mut dst, &layer, Transform3D::IDENTITY);
        dst
    }

    /// Draws the styled view into `area` of the canvas, rendering at the area's size.
    pub fn draw<C: Canvas + ?Sized>(&self, canvas: &mut C, area: RectF) {
        let (w, h) = (area.w.ceil().max(1.0) as usize, area.h.ceil().max(1.0) as usize);
        let p = self.render(w, h, 0.0, 0.0);
        let id = p.px.iter().fold(0xcbf2_9ce4_8422_2325u64, |a, v| (a ^ *v as u64).wrapping_mul(0x100_0000_01b3));
        let img = Image { w: p.w, h: p.h, px: p.px };
        canvas.image(ImageRef { id, image: &img }, area, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_properties_and_renders() {
        let v = ViewStyle::new(Pixmap::filled(10, 10, 0xFF00_00FF))
            .corner_radius(5.0)
            .border(1.0, 0xFFFF_0000)
            .transform(Transform3D::make_translation(2.0, 2.0, 0.0));
        assert_eq!(v.layer.style.corner_radius, 5.0);
        let p = v.render(20, 20, 0.0, 0.0);
        assert_eq!(p.get(7, 7), 0xFF00_00FF);
        assert_eq!(p.get(2, 2) >> 24, 0);
        assert_eq!(p.get(0, 0), 0);
    }
}
