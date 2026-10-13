//! Renders a [`Layer`] tree through the styled-layer rasteriser. Style that the plain layer properties do not cover
//! is read from the layer's extra values (`Layer::value` / `Layer::set_value`) under these keys:
//! `cornerRadius` (Float), `maskedCorners` (Float, a bit set: 1 min-min, 2 max-min, 4 min-max, 8 max-max),
//! `continuousCorners` (Bool), `shadowOpacity` and `shadowRadius` (Float), `shadowOffset` (Size), `shadowColor`
//! (Color), `sublayerTransform` (Transform), `blur` and `backgroundBlur` (Float, standard deviation in points),
//! `compositingFilter` (Text: a blend mode name such as "multiply"), `maskImage` (Image, an alpha mask the size of
//! the bounds), `doubleSided` (Bool) and `transformLayer` (Bool).
use crate::layer_render::{render_tree, BlendMode, CornerCurve, LayerFilter, Pixmap, StyledLayer};
use crate::layer_style::{CornerMask, Shadow};
use crate::layer_tree::{gravity_rect, Layer, LayerValue};
use crate::transform3d::Transform3D;
use crate::Rect;

/// Extra-value keys that make a layer need the styled path.
const STYLE_KEYS: [&str; 8] =
    ["cornerRadius", "shadowOpacity", "sublayerTransform", "blur", "backgroundBlur", "compositingFilter", "maskImage", "transformLayer"];

fn float(l: &Layer, key: &str) -> Option<f32> {
    match l.value(key) {
        Some(LayerValue::Float(v)) => Some(v),
        _ => None,
    }
}

fn flag(l: &Layer, key: &str) -> Option<bool> {
    match l.value(key) {
        Some(LayerValue::Bool(v)) => Some(v),
        _ => None,
    }
}

/// True when any layer in the tree uses style beyond what the plain command renderer draws (a corner radius, shadow,
/// blur, blend, mask, sublayer transform or transform layer, or a non-affine transform).
pub fn needs_styled_path(root: &Layer) -> bool {
    let own = STYLE_KEYS.iter().any(|k| root.value(k).is_some()) || !root.transform().is_affine();
    own || root.sublayers().iter().any(needs_styled_path)
}

/// The blend mode named by a `compositingFilter` value.
pub fn blend_from_name(name: &str) -> Option<BlendMode> {
    Some(match name {
        "normal" | "sourceOver" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "colorDodge" => BlendMode::ColorDodge,
        "colorBurn" => BlendMode::ColorBurn,
        "hardLight" => BlendMode::HardLight,
        "softLight" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "plusLighter" => BlendMode::PlusLighter,
        "plusDarker" => BlendMode::PlusDarker,
        _ => return None,
    })
}

fn unpremultiply(c: u32) -> u32 {
    let a = c >> 24;
    if a == 0 || a == 255 {
        return c;
    }
    let u = |v: u32| ((v * 255 + a / 2) / a).min(255);
    u(c & 255) | u((c >> 8) & 255) | u((c >> 16) & 255) << 16 | a << 24
}

fn size_of(b: Rect) -> (usize, usize) {
    (b.w.max(0.0).ceil() as usize, b.h.max(0.0).ceil() as usize)
}

/// Draws the layer's contents image into a buffer the size of its bounds, honouring gravity and the contents rect.
fn contents_pixmap(l: &Layer) -> Pixmap {
    let b = l.bounds();
    let (w, h) = size_of(b);
    let mut out = Pixmap::new(w, h);
    let Some(img) = l.contents() else { return out };
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 {
        return out;
    }
    let cr = l.contents_rect();
    let (sx0, sy0) = ((cr.x * img.w as f32).max(0.0), (cr.y * img.h as f32).max(0.0));
    let (sw, sh) = ((cr.w * img.w as f32).max(1.0), (cr.h * img.h as f32).max(1.0));
    let dst = gravity_rect(l.contents_gravity(), Rect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 }, sw, sh);
    if dst.w <= 0.0 || dst.h <= 0.0 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if fx < dst.x || fy < dst.y || fx >= dst.x + dst.w || fy >= dst.y + dst.h {
                continue;
            }
            let u = (sx0 + (fx - dst.x) / dst.w * sw).min(img.w as f32 - 1.0) as usize;
            let v = (sy0 + (fy - dst.y) / dst.h * sh).min(img.h as f32 - 1.0) as usize;
            out.px[y * w + x] = img.px[v * img.w + u];
        }
    }
    out
}

/// Converts a layer subtree to a styled layer tree. Hidden layers are dropped. Sublayers keep z order.
pub fn styled_from_layer(l: &Layer) -> StyledLayer {
    let b = l.bounds();
    let mut s = StyledLayer::new(contents_pixmap(l));
    let (w, h) = (s.content.w as f32, s.content.h as f32);
    let (ax, ay) = l.anchor_point();
    let pos = l.position();
    s.position = (pos.0 - ax * b.w, pos.1 - ay * b.h);
    // The styled layer rotates about its centre; shift the transform so it acts about the anchor point.
    let (c, a) = ((w / 2.0, h / 2.0), (ax * w, ay * h));
    s.transform = Transform3D::make_translation(c.0 - a.0, c.1 - a.1, 0.0)
        .concat(l.transform())
        .concat(Transform3D::make_translation(a.0 - c.0, a.1 - c.1, 0.0));
    if let Some(LayerValue::Transform(t)) = l.value("sublayerTransform") {
        s.sublayer_transform = t;
    }
    s.transform_layer = flag(l, "transformLayer").unwrap_or(false);
    s.double_sided = flag(l, "doubleSided").unwrap_or(true);
    let st = &mut s.style;
    st.background = l.background_color().map(unpremultiply).unwrap_or(0);
    st.masks_to_bounds = l.masks_to_bounds();
    st.border_width = l.border_width();
    st.border_color = unpremultiply(l.border_color());
    st.opacity = l.opacity();
    if let Some(r) = float(l, "cornerRadius") {
        st.corner_radius = r;
    }
    if let Some(m) = float(l, "maskedCorners") {
        let bits = m as u32;
        let mut mask = CornerMask::NONE;
        for (bit, corner) in [
            (1, CornerMask::MIN_X_MIN_Y),
            (2, CornerMask::MAX_X_MIN_Y),
            (4, CornerMask::MIN_X_MAX_Y),
            (8, CornerMask::MAX_X_MAX_Y),
        ] {
            if bits & bit != 0 {
                mask = mask.union(corner);
            }
        }
        st.masked_corners = mask;
    }
    if flag(l, "continuousCorners").unwrap_or(false) {
        st.corner_curve = CornerCurve::Continuous;
    }
    let offset = match l.value("shadowOffset") {
        Some(LayerValue::Size(x, y)) | Some(LayerValue::Point(x, y)) => (x, y),
        _ => (0.0, -3.0),
    };
    if let Some(o) = float(l, "shadowOpacity") {
        st.shadow = Shadow { opacity: o, radius: float(l, "shadowRadius").unwrap_or(3.0), offset };
    }
    if let Some(LayerValue::Color(c)) = l.value("shadowColor") {
        st.shadow_color = unpremultiply(c);
    }
    if let Some(r) = float(l, "blur") {
        st.filters.push(LayerFilter::GaussianBlur(r));
    }
    if let Some(r) = float(l, "backgroundBlur") {
        st.filters.push(LayerFilter::BackgroundBlur(r));
    }
    if let Some(LayerValue::Text(t)) = l.value("compositingFilter") {
        if let Some(m) = blend_from_name(&t) {
            st.blend = m;
        }
    }
    if let Some(LayerValue::Image(Some(img))) = l.value("maskImage") {
        st.mask = Some(Pixmap { w: img.w, h: img.h, px: img.px.as_ref().clone() });
    }
    let mut subs = l.sublayers();
    subs.sort_by(|a, b| a.z_position().partial_cmp(&b.z_position()).unwrap_or(std::cmp::Ordering::Equal));
    s.children = subs.iter().filter(|c| !c.is_hidden()).map(styled_from_layer).collect();
    s
}

/// Renders a layer tree into a `w` by `h` pixel buffer. `to_dst` maps the root's parent space (points) to pixels;
/// pass a scale for device pixels.
pub fn render_layer_tree(root: &Layer, w: usize, h: usize, to_dst: Transform3D) -> Pixmap {
    let mut dst = Pixmap::new(w, h);
    if !root.is_hidden() {
        render_tree(&mut dst, &styled_from_layer(root), to_dst);
    }
    dst
}

/// The front-most layer under `p` (in the root's parent space), following transforms and perspective.
pub fn hit_test_layer(root: &Layer, p: (f32, f32)) -> Option<Layer> {
    let tree = styled_from_layer(root);
    let path = tree.hit_test(p, Transform3D::IDENTITY)?;
    let mut cur = root.clone();
    for i in path {
        let mut subs = cur.sublayers();
        subs.sort_by(|a, b| a.z_position().partial_cmp(&b.z_position()).unwrap_or(std::cmp::Ordering::Equal));
        subs.retain(|c| !c.is_hidden());
        cur = subs.get(i)?.clone();
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_layers_skip_the_styled_path_and_styled_ones_use_it() {
        let l = Layer::with_frame(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
        assert!(!needs_styled_path(&l));
        l.set_value("cornerRadius", LayerValue::Float(4.0)).unwrap();
        assert!(needs_styled_path(&l));
    }

    #[test]
    fn corner_radius_and_background_render_through_the_tree() {
        let l = Layer::with_frame(Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 });
        l.set_background_color(Some(0xff00_00ff));
        l.set_value("cornerRadius", LayerValue::Float(8.0)).unwrap();
        l.set_masks_to_bounds(true);
        let p = render_layer_tree(&l, 20, 20, Transform3D::IDENTITY);
        assert_eq!(p.get(10, 10), 0xff00_00ff);
        assert_eq!(p.get(0, 0) >> 24, 0);
    }

    #[test]
    fn sublayer_transform_and_hit_test_follow_the_tree() {
        let root = Layer::with_frame(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
        root.set_value("sublayerTransform", LayerValue::Transform(Transform3D::make_translation(20.0, 0.0, 0.0))).unwrap();
        let child = Layer::with_frame(Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 });
        child.set_background_color(Some(0xff00_00ff));
        root.add_sublayer(&child);
        let p = render_layer_tree(&root, 40, 10, Transform3D::IDENTITY);
        assert_eq!(p.get(22, 2), 0xff00_00ff);
        assert_eq!(hit_test_layer(&root, (22.0, 2.0)), Some(child.clone()));
        assert_eq!(hit_test_layer(&root, (2.0, 2.0)), Some(root.clone()));
        assert_eq!(hit_test_layer(&root, (12.0, 2.0)), None);
    }

    #[test]
    fn blend_names_parse() {
        assert_eq!(blend_from_name("multiply"), Some(BlendMode::Multiply));
        assert_eq!(blend_from_name("nope"), None);
    }
}
