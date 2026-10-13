//! Style properties of a view, stored on its backing layer (created on first use): corner radius and masked
//! corners, border, shadow, transform, sublayer transform, mask and filters. A view with any of these draws its layer
//! tree through the styled rasteriser (see `layer_backed`). Values use the layer extra-value keys documented in
//! `bifrost_ui::layer_bridge`.
use super::ViewBase;
use bifrost_ui::layer_tree::{LayerImage, LayerValue};
use bifrost_ui::layer_style::CornerMask;
use bifrost_ui::transform3d::Transform3D;

fn corner_bits(m: CornerMask) -> f32 {
    let mut bits = 0u32;
    for (bit, c) in [
        (1, CornerMask::MIN_X_MIN_Y),
        (2, CornerMask::MAX_X_MIN_Y),
        (4, CornerMask::MIN_X_MAX_Y),
        (8, CornerMask::MAX_X_MAX_Y),
    ] {
        if m.contains(c) {
            bits |= bit;
        }
    }
    bits as f32
}

impl ViewBase {
    fn put(&mut self, key: &str, v: LayerValue) {
        let _ = self.layer().set_value(key, v);
    }

    /// Corner radius in points; the layer also clips its content to the rounded shape.
    pub fn set_corner_radius(&mut self, r: f32) {
        self.put("cornerRadius", LayerValue::Float(r));
        self.layer().set_masks_to_bounds(true);
    }
    /// Which corners the radius applies to.
    pub fn set_masked_corners(&mut self, m: CornerMask) {
        self.put("maskedCorners", LayerValue::Float(corner_bits(m)));
    }
    /// Continuous (squircle-like) corner curve instead of circular.
    pub fn set_continuous_corners(&mut self, on: bool) {
        self.put("continuousCorners", LayerValue::Bool(on));
    }
    /// Border width (points) and premultiplied colour.
    pub fn set_border(&mut self, width: f32, color: u32) {
        let l = self.layer();
        l.set_border_width(width);
        l.set_border_color(color);
    }
    /// Shadow: premultiplied colour, opacity 0..1, blur radius and offset in points.
    pub fn set_shadow(&mut self, color: u32, opacity: f32, radius: f32, offset: (f32, f32)) {
        self.put("shadowColor", LayerValue::Color(color));
        self.put("shadowRadius", LayerValue::Float(radius));
        self.put("shadowOffset", LayerValue::Size(offset.0, offset.1));
        self.put("shadowOpacity", LayerValue::Float(opacity));
    }
    /// The view's 3D transform, about the layer's anchor point.
    pub fn set_transform(&mut self, t: Transform3D) {
        self.layer().set_transform(t);
    }
    /// A transform applied to the view's sublayers.
    pub fn set_sublayer_transform(&mut self, t: Transform3D) {
        self.put("sublayerTransform", LayerValue::Transform(t));
    }
    /// An alpha mask the size of the view (premultiplied pixels): the layer shows where the mask has alpha.
    pub fn set_mask(&mut self, w: usize, h: usize, px: Vec<u32>) {
        self.put("maskImage", LayerValue::Image(Some(LayerImage::new(w, h, px))));
    }
    /// Gaussian blur of the layer's pixels (standard deviation in points).
    pub fn set_blur(&mut self, sigma: f32) {
        self.put("blur", LayerValue::Float(sigma));
    }
    /// Blur of what is behind the layer, inside its shape.
    pub fn set_background_blur(&mut self, sigma: f32) {
        self.put("backgroundBlur", LayerValue::Float(sigma));
    }
    /// A blend mode name (`multiply`, `screen`, `overlay`, ...) for compositing the layer over what is behind.
    pub fn set_blend(&mut self, name: &str) {
        self.put("compositingFilter", LayerValue::Text(name.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::Rect;

    #[test]
    fn setters_store_on_the_backing_layer() {
        let mut b = ViewBase::default();
        b.set_corner_radius(6.0);
        b.set_shadow(0xff00_0000, 0.5, 4.0, (0.0, 2.0));
        b.set_blend("multiply");
        let l = b.layer();
        assert_eq!(l.value("cornerRadius"), Some(LayerValue::Float(6.0)));
        assert!(l.masks_to_bounds());
        assert_eq!(l.value("shadowOffset"), Some(LayerValue::Size(0.0, 2.0)));
        assert!(bifrost_ui::layer_bridge::needs_styled_path(&l));
        let _ = Rect::default();
    }
}
