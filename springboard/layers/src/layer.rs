//! A layer and its properties (the reference platform's layer object, model values only).
//!
//! Owner: wave G1 agent `layers-model`.

use crate::geometry::{Affine, Color, Point, Rect};

/// A layer's id, unique within one tree (one layer context). 0 is never used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub u32);

/// The id a context binds a backing store (`bind_contents`, a dma-buf `wl_buffer`) to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentsId(pub u32);

/// What fills a layer's bounds beneath its sublayers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LayerContents {
    #[default]
    None,
    /// A bound backing store, drawn into the bounds per `contents_gravity`.
    Buffer(ContentsId),
}

/// How contents of another size map onto the bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentsGravity {
    /// Stretch to fill (default).
    #[default]
    Resize,
    ResizeAspect,
    ResizeAspectFill,
    Center,
    TopLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// A drop shadow drawn by the renderer behind the layer's rounded bounds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shadow {
    pub color: Color,
    pub opacity: f32,
    pub radius: f32,
    pub offset: Point,
}

/// A behind-layer material: the renderer blurs whatever is composited behind the layer's rounded bounds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Material {
    #[default]
    None,
    /// Live glass: blur radius in points, a tint drawn over the blur, saturation boost (1 = none).
    Glass { blur: f32, tint: Color, saturation: f32 },
}

/// The animatable and plain properties, by key (animations and transactions name them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Property {
    Bounds,
    Position,
    AnchorPoint,
    Transform,
    Opacity,
    Hidden,
    CornerRadius,
    MasksToBounds,
    BackgroundColor,
    Border,
    Shadow,
    Material,
    Contents,
    ContentsGravity,
    ZPosition,
}

impl Property {
    /// Whether the property interpolates (others change at the animation's end, like the reference platform).
    pub fn animatable(self) -> bool {
        !matches!(self, Property::Hidden | Property::MasksToBounds | Property::Contents | Property::ContentsGravity)
    }
}

/// A layer's model values.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    /// Size (and contents origin) in the layer's own coordinate space.
    pub bounds: Rect,
    /// Where the anchor point sits in the superlayer's coordinate space.
    pub position: Point,
    /// The point of the bounds (unit coordinates, 0.5/0.5 is the centre) that `position` and `transform` act on.
    pub anchor_point: Point,
    pub transform: Affine,
    pub z_position: f32,
    pub opacity: f32,
    pub hidden: bool,
    pub corner_radius: f32,
    pub masks_to_bounds: bool,
    pub background_color: Color,
    pub border: Border,
    pub shadow: Shadow,
    pub material: Material,
    pub contents: LayerContents,
    pub contents_gravity: ContentsGravity,
    pub sublayers: Vec<LayerId>,
    pub superlayer: Option<LayerId>,
}

impl Layer {
    /// A layer with the reference platform's defaults: zero bounds, anchor at the centre, opaque, visible.
    pub fn new(id: LayerId) -> Layer {
        Layer {
            id,
            bounds: Rect::ZERO,
            position: Point::default(),
            anchor_point: Point { x: 0.5, y: 0.5 },
            transform: Affine::IDENTITY,
            z_position: 0.0,
            opacity: 1.0,
            hidden: false,
            corner_radius: 0.0,
            masks_to_bounds: false,
            background_color: Color::CLEAR,
            border: Border::default(),
            shadow: Shadow::default(),
            material: Material::None,
            contents: LayerContents::None,
            contents_gravity: ContentsGravity::Resize,
            sublayers: Vec::new(),
            superlayer: None,
        }
    }

    /// The layer's frame in its superlayer: bounds placed so the anchor point lands on `position`, before
    /// `transform` (the transform acts about the anchor point).
    pub fn frame(&self) -> Rect {
        Rect {
            x: self.position.x - self.anchor_point.x * self.bounds.w,
            y: self.position.y - self.anchor_point.y * self.bounds.h,
            w: self.bounds.w,
            h: self.bounds.h,
        }
    }

    /// The transform from this layer's coordinate space to its superlayer's.
    pub fn to_superlayer(&self) -> Affine {
        let ax = self.bounds.x + self.anchor_point.x * self.bounds.w;
        let ay = self.bounds.y + self.anchor_point.y * self.bounds.h;
        Affine::translate(self.position.x, self.position.y).concat(&self.transform).concat(&Affine::translate(-ax, -ay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_puts_the_anchor_on_the_position() {
        let mut l = Layer::new(LayerId(1));
        l.bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        l.position = Point { x: 50.0, y: 20.0 };
        assert_eq!(l.frame(), Rect::new(0.0, 0.0, 100.0, 40.0));
        l.anchor_point = Point { x: 0.0, y: 0.0 };
        assert_eq!(l.frame(), Rect::new(50.0, 20.0, 100.0, 40.0));
    }

    #[test]
    fn to_superlayer_maps_the_bounds_origin_to_the_frame_origin() {
        let mut l = Layer::new(LayerId(1));
        l.bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        l.position = Point { x: 150.0, y: 120.0 };
        assert_eq!(l.to_superlayer().apply(Point::default()), Point { x: 100.0, y: 100.0 });
    }
}
