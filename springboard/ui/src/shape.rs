//! The built-in shapes (rectangle, rounded rectangle, capsule, ellipse), each as a [`cg::path::Path`] inside a rect.
//! The path type itself, with length, trimming and dashing, lives in `render::cg::path`.
use crate::cg::path::{Path, Rect as PathRect};
use crate::Rect;

impl From<Rect> for PathRect {
    fn from(r: Rect) -> PathRect {
        PathRect { x: r.x, y: r.y, w: r.w, h: r.h }
    }
}

impl From<PathRect> for Rect {
    fn from(r: PathRect) -> Rect {
        Rect { x: r.x, y: r.y, w: r.w, h: r.h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Rectangle,
    /// Corner radius in the same units as the rect.
    RoundedRectangle(f32),
    /// A rounded rectangle whose ends are half circles.
    Capsule,
    Ellipse,
}

impl Shape {
    pub fn path(self, r: Rect) -> Path {
        let r: PathRect = r.into();
        let mut p = Path::new();
        match self {
            Shape::Rectangle => p.add_rect(r),
            Shape::RoundedRectangle(radius) => p.add_rounded_rect(r, radius, radius),
            Shape::Capsule => {
                let rad = r.w.min(r.h) / 2.0;
                p.add_rounded_rect(r, rad, rad)
            }
            Shape::Ellipse => p.add_ellipse(r),
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn a_rectangle_has_its_perimeter_and_its_own_bounds() {
        let r = Rect { x: 5.0, y: 7.0, w: 10.0, h: 20.0 };
        let p = Shape::Rectangle.path(r);
        assert_eq!(p.bounds().map(Rect::from), Some(r));
        assert!(near(p.length(0.1), 60.0));
    }

    #[test]
    fn an_ellipse_is_inscribed_and_has_the_ellipse_length() {
        let r = Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 };
        let p = Shape::Ellipse.path(r);
        let b: Rect = p.bounds().unwrap().into();
        assert!(near(b.w, 20.0) && near(b.h, 20.0));
        assert!((p.length(0.01) - std::f32::consts::PI * 20.0).abs() < 0.2);
    }

    #[test]
    fn capsule_rounds_the_shorter_side_and_zero_radius_is_a_rectangle() {
        let r = Rect { x: 0.0, y: 0.0, w: 40.0, h: 10.0 };
        let cap = Shape::Capsule.path(r);
        let rounded = Shape::RoundedRectangle(10.0).path(r);
        assert_eq!(cap, rounded, "a radius past half the short side clamps to a capsule");
        assert!(cap.length(0.05) < Shape::Rectangle.path(r).length(0.05));
        assert_eq!(Shape::RoundedRectangle(0.0).path(r), Shape::Rectangle.path(r));
    }
}
