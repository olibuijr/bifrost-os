//! Affine transforms: a 2x3 matrix that maps points and rectangles (the transform type of the reference SDK, without
//! the graphics-context stack). Layout is `a b c d tx ty`, so `(x, y)` maps to `(a*x + c*y + tx, b*x + d*y + ty)`.
//! Pure maths: the renderer applies the result to its own drawing calls.
use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AffineTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl AffineTransform {
    pub const IDENTITY: AffineTransform = AffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translate(tx: f32, ty: f32) -> Self {
        AffineTransform { tx, ty, ..Self::IDENTITY }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        AffineTransform { a: sx, d: sy, ..Self::IDENTITY }
    }

    /// Rotates about the origin. A positive angle turns the x axis towards the y axis.
    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        AffineTransform { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    /// The transform that applies `self` first and then `next`.
    pub fn then(self, next: AffineTransform) -> AffineTransform {
        let (p, n) = (self, next);
        let (tx, ty) = n.apply(p.tx, p.ty);
        AffineTransform {
            a: n.a * p.a + n.c * p.b,
            b: n.b * p.a + n.d * p.b,
            c: n.a * p.c + n.c * p.d,
            d: n.b * p.c + n.d * p.d,
            tx,
            ty,
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// The bounding box of the four transformed corners of `r`.
    pub fn apply_rect(self, r: Rect) -> Rect {
        let corners = [
            self.apply(r.x, r.y),
            self.apply(r.right(), r.y),
            self.apply(r.x, r.bottom()),
            self.apply(r.right(), r.bottom()),
        ];
        let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let max_x = corners.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        let max_y = corners.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
    }

    /// The inverse transform, or `None` when the matrix is singular (it collapses an axis).
    pub fn invert(self) -> Option<AffineTransform> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        Some(AffineTransform {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            tx: (self.c * self.ty - self.d * self.tx) / det,
            ty: (self.b * self.tx - self.a * self.ty) / det,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    #[test]
    fn identity_and_translate_map_points() {
        assert!(AffineTransform::IDENTITY.is_identity());
        assert!(near(AffineTransform::IDENTITY.apply(3.0, -2.0), (3.0, -2.0)));
        assert!(near(AffineTransform::translate(10.0, 5.0).apply(1.0, 1.0), (11.0, 6.0)));
    }

    #[test]
    fn then_applies_the_first_transform_first() {
        let scale_then_move = AffineTransform::scale(2.0, 2.0).then(AffineTransform::translate(1.0, 0.0));
        assert!(near(scale_then_move.apply(1.0, 1.0), (3.0, 2.0)));
        let move_then_scale = AffineTransform::translate(1.0, 0.0).then(AffineTransform::scale(2.0, 2.0));
        assert!(near(move_then_scale.apply(1.0, 1.0), (4.0, 2.0)));
    }

    #[test]
    fn rotate_quarter_turn_takes_x_to_y() {
        let q = AffineTransform::rotate(std::f32::consts::FRAC_PI_2);
        assert!(near(q.apply(1.0, 0.0), (0.0, 1.0)));
        assert!(near(q.apply(0.0, 1.0), (-1.0, 0.0)));
    }

    #[test]
    fn apply_rect_is_the_bounds_of_the_rotated_corners() {
        let r = Rect { x: 0.0, y: 0.0, w: 10.0, h: 4.0 };
        let b = AffineTransform::rotate(std::f32::consts::FRAC_PI_2).apply_rect(r);
        assert!((b.x + 4.0).abs() < 1e-3 && b.y.abs() < 1e-3, "origin at {},{}", b.x, b.y);
        assert!((b.w - 4.0).abs() < 1e-3 && (b.h - 10.0).abs() < 1e-3, "size {}x{}", b.w, b.h);
    }

    #[test]
    fn invert_undoes_the_transform() {
        let t = AffineTransform::rotate(0.7).then(AffineTransform::scale(2.0, 3.0)).then(AffineTransform::translate(5.0, -8.0));
        let inv = t.invert().expect("non-singular");
        let (x, y) = t.apply(1.5, -2.5);
        assert!(near(inv.apply(x, y), (1.5, -2.5)));
        assert!(near(t.then(inv).apply(4.0, 4.0), (4.0, 4.0)));
    }

    #[test]
    fn singular_transform_has_no_inverse() {
        assert!(AffineTransform::scale(0.0, 1.0).invert().is_none());
        assert!(AffineTransform::scale(1.0, 1.0).invert().is_some());
    }
}
