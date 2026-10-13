//! Layer delegates, layout managers and the constraint layout manager.
//!
//! A [`LayerDelegate`] draws a layer's content into a [`LayerContext`], lays out its sublayers and answers
//! [`LayerDelegate::action_for`] (what a property change should do; `Some(LayerAction::Null)` means "nothing").
//! A [`LayoutManager`] lays out the sublayers of the layer it is set on. [`ConstraintLayoutManager`] does it from
//! per-sublayer [`Constraint`]s: an attribute of the sublayer is tied to an attribute of the superlayer or of a
//! named sibling, scaled and offset.
use crate::layer_tree::{Layer, LayerImage};
use crate::Rect;

/// What a property change does, as answered by a delegate or the layer's `actions` table.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerAction {
    /// No action: the change applies at once, without an animation.
    Null,
    /// A named action (an animation key, for example) for the animation layer to resolve.
    Named(String),
}

/// One recorded drawing operation of a layer's own content, in the layer's bounds coordinates (points).
#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    FillRect { rect: Rect, color: u32 },
    StrokeRect { rect: Rect, width: f32, color: u32 },
    Image { image: LayerImage, dst: Rect, alpha: f32 },
}

/// The drawing context a delegate draws into. Colours are premultiplied RGBA packed in a `u32`.
#[derive(Clone, Debug, Default)]
pub struct LayerContext {
    size: (f32, f32),
    ops: Vec<DrawOp>,
}

impl LayerContext {
    pub fn new(width: f32, height: f32) -> Self {
        LayerContext { size: (width, height), ops: Vec::new() }
    }
    /// The layer's bounds size.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }
    pub fn fill_rect(&mut self, rect: Rect, color: u32) {
        self.ops.push(DrawOp::FillRect { rect, color });
    }
    pub fn stroke_rect(&mut self, rect: Rect, width: f32, color: u32) {
        self.ops.push(DrawOp::StrokeRect { rect, width, color });
    }
    pub fn draw_image(&mut self, image: LayerImage, dst: Rect, alpha: f32) {
        self.ops.push(DrawOp::Image { image, dst, alpha });
    }
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }
    pub fn into_ops(self) -> Vec<DrawOp> {
        self.ops
    }
}

/// The object a layer asks to draw itself, lay out its sublayers and answer property-change actions.
pub trait LayerDelegate {
    /// Draws the layer's content. Called when the layer is displayed (`set_needs_display`).
    fn draw_layer(&self, _layer: &Layer, _ctx: &mut LayerContext) {}
    /// Lays out the sublayers; used when the layer has no layout manager.
    fn layout_sublayers(&self, _layer: &Layer) {}
    /// The action for a change of `key`, or `None` to fall through to the layer's `actions` table.
    fn action_for(&self, _layer: &Layer, _key: &str) -> Option<LayerAction> {
        None
    }
}

/// Lays out the sublayers of a layer.
pub trait LayoutManager {
    fn layout_sublayers(&self, layer: &Layer);
    /// The size the layer would like, when the manager can tell.
    fn preferred_size(&self, _layer: &Layer) -> Option<(f32, f32)> {
        None
    }
    /// True when a change to a sublayer's geometry means this layer's sublayers need laying out again.
    fn reacts_to_sublayer_changes(&self) -> bool {
        false
    }
}

/// An attribute a constraint ties together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintAttr {
    MinX,
    MidX,
    MaxX,
    Width,
    MinY,
    MidY,
    MaxY,
    Height,
}

impl ConstraintAttr {
    fn is_x(self) -> bool {
        matches!(self, ConstraintAttr::MinX | ConstraintAttr::MidX | ConstraintAttr::MaxX | ConstraintAttr::Width)
    }
    /// The value of this attribute of `r`.
    fn of(self, r: Rect) -> f32 {
        match self {
            ConstraintAttr::MinX => r.x,
            ConstraintAttr::MidX => r.x + r.w / 2.0,
            ConstraintAttr::MaxX => r.x + r.w,
            ConstraintAttr::Width => r.w,
            ConstraintAttr::MinY => r.y,
            ConstraintAttr::MidY => r.y + r.h / 2.0,
            ConstraintAttr::MaxY => r.y + r.h,
            ConstraintAttr::Height => r.h,
        }
    }
    fn kind(self) -> Kind {
        match self {
            ConstraintAttr::MinX | ConstraintAttr::MinY => Kind::Min,
            ConstraintAttr::MidX | ConstraintAttr::MidY => Kind::Mid,
            ConstraintAttr::MaxX | ConstraintAttr::MaxY => Kind::Max,
            ConstraintAttr::Width | ConstraintAttr::Height => Kind::Size,
        }
    }
}

/// What a constraint is relative to.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstraintSource {
    /// The superlayer's bounds.
    Superlayer,
    /// The sibling with this layer name.
    Sibling(String),
}

impl ConstraintSource {
    /// `"superlayer"` is the superlayer; any other string names a sibling.
    pub fn from_name(name: &str) -> Self {
        if name == "superlayer" {
            ConstraintSource::Superlayer
        } else {
            ConstraintSource::Sibling(name.to_string())
        }
    }
}

/// `attr = source.source_attr * scale + offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub attr: ConstraintAttr,
    pub source: ConstraintSource,
    pub source_attr: ConstraintAttr,
    pub scale: f32,
    pub offset: f32,
}

impl Constraint {
    pub fn new(attr: ConstraintAttr, source: ConstraintSource, source_attr: ConstraintAttr) -> Self {
        Constraint { attr, source, source_attr, scale: 1.0, offset: 0.0 }
    }
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
    pub fn with_offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Min,
    Mid,
    Max,
    Size,
}

/// Resolves one axis: the new `(origin, size)` from the current ones and the constrained values. Later values of the
/// same kind win.
fn solve_axis(origin: f32, size: f32, values: &[(Kind, f32)]) -> (f32, f32) {
    let get = |k: Kind| values.iter().rev().find(|(kind, _)| *kind == k).map(|(_, v)| *v);
    let (min, mid, max, sz) = (get(Kind::Min), get(Kind::Mid), get(Kind::Max), get(Kind::Size));
    if let Some(s) = sz {
        let o = match (min, mid, max) {
            (Some(v), _, _) => v,
            (None, Some(v), _) => v - s / 2.0,
            (None, None, Some(v)) => v - s,
            (None, None, None) => origin,
        };
        return (o, s);
    }
    match (min, mid, max) {
        (Some(a), _, Some(b)) => (a, b - a),
        (Some(a), Some(m), None) => (a, 2.0 * (m - a)),
        (None, Some(m), Some(b)) => (2.0 * m - b, 2.0 * (b - m)),
        (Some(a), None, None) => (a, size),
        (None, Some(m), None) => (m - size / 2.0, size),
        (None, None, Some(b)) => (b - size, size),
        (None, None, None) => (origin, size),
    }
}

/// Lays out each sublayer from its constraints. Sublayers without constraints keep their frame. A constraint whose
/// sibling is missing is skipped.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConstraintLayoutManager;

impl LayoutManager for ConstraintLayoutManager {
    fn layout_sublayers(&self, layer: &Layer) {
        let siblings = layer.sublayers();
        let parent_bounds = layer.bounds();
        for sub in &siblings {
            let cs = sub.constraints();
            if cs.is_empty() {
                continue;
            }
            let frame = sub.frame();
            let (mut xs, mut ys) = (Vec::new(), Vec::new());
            for c in &cs {
                let src = match &c.source {
                    ConstraintSource::Superlayer => Some(parent_bounds),
                    ConstraintSource::Sibling(name) => {
                        siblings.iter().find(|s| *s != sub && s.name() == *name).map(|s| s.frame())
                    }
                };
                let Some(src) = src else { continue };
                let v = c.source_attr.of(src) * c.scale + c.offset;
                let entry = (c.attr.kind(), v);
                if c.attr.is_x() {
                    xs.push(entry);
                } else {
                    ys.push(entry);
                }
            }
            let (x, w) = solve_axis(frame.x, frame.w, &xs);
            let (y, h) = solve_axis(frame.y, frame.h, &ys);
            sub.set_frame(Rect { x, y, w: w.max(0.0), h: h.max(0.0) });
        }
    }
    fn reacts_to_sublayer_changes(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn axis_resolution() {
        assert_eq!(solve_axis(1.0, 2.0, &[]), (1.0, 2.0));
        assert_eq!(solve_axis(0.0, 5.0, &[(Kind::Min, 10.0), (Kind::Max, 30.0)]), (10.0, 20.0));
        assert_eq!(solve_axis(0.0, 5.0, &[(Kind::Mid, 50.0), (Kind::Size, 20.0)]), (40.0, 20.0));
        assert_eq!(solve_axis(0.0, 5.0, &[(Kind::Max, 50.0)]), (45.0, 5.0));
        assert_eq!(solve_axis(0.0, 5.0, &[(Kind::Mid, 10.0), (Kind::Max, 20.0)]), (0.0, 20.0));
    }

    #[test]
    fn constrain_to_superlayer_with_inset() {
        let parent = Layer::new();
        parent.set_bounds(r(0.0, 0.0, 200.0, 100.0));
        parent.set_layout_manager(Some(Rc::new(ConstraintLayoutManager)));
        let child = Layer::new();
        child.set_frame(r(0.0, 0.0, 10.0, 10.0));
        for (a, s) in [
            (ConstraintAttr::MinX, ConstraintAttr::MinX),
            (ConstraintAttr::MaxX, ConstraintAttr::MaxX),
            (ConstraintAttr::MinY, ConstraintAttr::MinY),
            (ConstraintAttr::MaxY, ConstraintAttr::MaxY),
        ] {
            let sign = if matches!(a, ConstraintAttr::MaxX | ConstraintAttr::MaxY) { -8.0 } else { 8.0 };
            child.add_constraint(Constraint::new(a, ConstraintSource::Superlayer, s).with_offset(sign));
        }
        parent.add_sublayer(&child);
        parent.layout_if_needed();
        assert_eq!(child.frame(), r(8.0, 8.0, 184.0, 84.0));
        parent.set_bounds(r(0.0, 0.0, 300.0, 100.0));
        assert!(parent.needs_layout());
        parent.layout_if_needed();
        assert_eq!(child.frame(), r(8.0, 8.0, 284.0, 84.0));
    }

    #[test]
    fn constrain_to_a_named_sibling() {
        let parent = Layer::new();
        parent.set_bounds(r(0.0, 0.0, 200.0, 100.0));
        parent.set_layout_manager(Some(Rc::new(ConstraintLayoutManager)));
        let a = Layer::new();
        a.set_name("a");
        a.set_frame(r(10.0, 10.0, 50.0, 20.0));
        let b = Layer::new();
        b.set_frame(r(0.0, 0.0, 30.0, 20.0));
        b.add_constraint(Constraint::new(ConstraintAttr::MinX, ConstraintSource::from_name("a"), ConstraintAttr::MaxX).with_offset(5.0));
        b.add_constraint(Constraint::new(ConstraintAttr::MidY, ConstraintSource::from_name("a"), ConstraintAttr::MidY));
        b.add_constraint(Constraint::new(ConstraintAttr::Width, ConstraintSource::Superlayer, ConstraintAttr::Width).with_scale(0.5));
        parent.add_sublayer(&a);
        parent.add_sublayer(&b);
        parent.layout_if_needed();
        assert_eq!(b.frame(), r(65.0, 10.0, 100.0, 20.0));
        assert_eq!(a.frame(), r(10.0, 10.0, 50.0, 20.0));
    }

    #[test]
    fn missing_sibling_is_skipped() {
        let parent = Layer::new();
        parent.set_bounds(r(0.0, 0.0, 100.0, 100.0));
        parent.set_layout_manager(Some(Rc::new(ConstraintLayoutManager)));
        let b = Layer::new();
        b.set_frame(r(1.0, 2.0, 3.0, 4.0));
        b.add_constraint(Constraint::new(ConstraintAttr::MinX, ConstraintSource::from_name("nope"), ConstraintAttr::MinX));
        parent.add_sublayer(&b);
        parent.layout_if_needed();
        assert_eq!(b.frame(), r(1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn context_records_ops() {
        let mut c = LayerContext::new(10.0, 20.0);
        c.fill_rect(r(0.0, 0.0, 1.0, 1.0), 5);
        c.stroke_rect(r(0.0, 0.0, 1.0, 1.0), 2.0, 6);
        assert_eq!(c.size(), (10.0, 20.0));
        assert_eq!(c.ops().len(), 2);
    }
}
