//! Geometry and filter models for scene nodes: a crop node that clips its children to a rectangle, a transform node
//! that holds a two-dimensional affine transform, and an effect node that holds one filter and a rasterize flag. The
//! models compute what is visible and what a transform maps to. They draw nothing; the presenter applies them.

/// An axis-aligned rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The overlap of two rectangles, or None when they share no area.
    pub fn intersection(self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width).min(other.x + other.width);
        let y1 = (self.y + self.height).min(other.y + other.height);
        (x1 > x0 && y1 > y0).then(|| Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }
}

/// A two-dimensional affine transform: `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Affine {
    /// The transform that leaves points unchanged.
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translation(tx: f64, ty: f64) -> Affine {
        Affine { tx, ty, ..Affine::IDENTITY }
    }

    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }

    /// A rotation by `radians`, counter-clockwise in a y-up space.
    pub fn rotation(radians: f64) -> Affine {
        let (sin, cos) = radians.sin_cos();
        Affine { a: cos, b: sin, c: -sin, d: cos, tx: 0.0, ty: 0.0 }
    }

    /// The transform that applies `self` first and then `next`.
    pub fn then(self, next: Affine) -> Affine {
        Affine {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            tx: next.a * self.tx + next.c * self.ty + next.tx,
            ty: next.b * self.tx + next.d * self.ty + next.ty,
        }
    }

    /// Maps a point through the transform.
    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
}

/// A node that clips its children to `mask`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropNode {
    pub mask: Rect,
}

impl CropNode {
    /// The part of a child's frame that is visible through the mask, or None when none of it is.
    pub fn visible(&self, child: Rect) -> Option<Rect> {
        self.mask.intersection(child)
    }
}

/// A node that moves its children with an affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformNode {
    pub transform: Affine,
}

/// A filter an effect node applies to its children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// Gaussian blur with a radius in points.
    Blur { radius: f64 },
}

/// Why an effect node was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectError {
    /// A blur radius that is negative or not finite.
    InvalidRadius(f64),
}

/// A node that renders its children through a filter, optionally into an offscreen buffer first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectNode {
    pub filter: Option<Filter>,
    /// Render the children once into an offscreen buffer and reuse it while they do not change.
    pub rasterize: bool,
}

impl EffectNode {
    /// Checks the filter. A blur radius must be a finite number of zero or more.
    pub fn validate(&self) -> Result<(), EffectError> {
        match self.filter {
            Some(Filter::Blur { radius }) if !(radius.is_finite() && radius >= 0.0) => Err(EffectError::InvalidRadius(radius)),
            _ => Ok(()),
        }
    }
}
