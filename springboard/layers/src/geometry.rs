//! Geometry and colour values used by layers. f32 points in the surface's logical coordinate space (the window
//! server multiplies by the output scale).

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn max_x(&self) -> f32 {
        self.x + self.w
    }

    pub fn max_y(&self) -> f32 {
        self.y + self.h
    }

    /// The smallest rectangle containing both (an empty rectangle contributes nothing).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.w <= 0.0 || self.h <= 0.0 {
            return *other;
        }
        if other.w <= 0.0 || other.h <= 0.0 {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect { x, y, w: self.max_x().max(other.max_x()) - x, h: self.max_y().max(other.max_y()) - y }
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.max_x() && other.x < self.max_x() && self.y < other.max_y() && other.y < self.max_y()
    }
}

/// A 2D affine transform: `[a c tx; b d ty]` applied as `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translate(x: f32, y: f32) -> Affine {
        Affine { tx: x, ty: y, ..Affine::IDENTITY }
    }

    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::IDENTITY }
    }

    pub fn rotate(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
    }

    /// `self` after `other`: applying the result equals applying `other` first, then `self`.
    pub fn concat(&self, other: &Affine) -> Affine {
        Affine {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            tx: self.a * other.tx + self.c * other.ty + self.tx,
            ty: self.b * other.tx + self.d * other.ty + self.ty,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point { x: self.a * p.x + self.c * p.y + self.tx, y: self.b * p.x + self.d * p.y + self.ty }
    }
}

/// A straight (not premultiplied) RGBA colour in 0..1, the way layer colours are specified.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const CLEAR: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// From the shell's packed premultiplied RGBA8 (`gfx::pack` order: r in the low byte).
    pub fn from_packed(p: u32) -> Color {
        let a = ((p >> 24) & 0xff) as f32 / 255.0;
        let un = |v: u32| if a > 0.0 { ((v & 0xff) as f32 / 255.0 / a).min(1.0) } else { 0.0 };
        Color { r: un(p), g: un(p >> 8), b: un(p >> 16), a }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_applies_right_operand_first() {
        let m = Affine::translate(10.0, 0.0).concat(&Affine::scale(2.0, 2.0));
        assert_eq!(m.apply(Point { x: 1.0, y: 1.0 }), Point { x: 12.0, y: 2.0 });
    }

    #[test]
    fn union_ignores_empty() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(a.union(&Rect::ZERO), a);
        assert_eq!(a.union(&Rect::new(5.0, 5.0, 10.0, 10.0)), Rect::new(0.0, 0.0, 15.0, 15.0));
    }
}
