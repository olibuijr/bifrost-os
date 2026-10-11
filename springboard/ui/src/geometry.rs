//! Geometry helpers every layout builds on: `NativeEdgeInsets`, `CGRectDivide`, rectangle algebra.
use crate::Rect;

/// Which side of a rectangle (`NativeRectEdge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Left,
    Bottom,
    Right,
}

impl Edge {
    pub fn opposite(self) -> Edge {
        match self {
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
        }
    }
}

/// Space around content on each side (`NativeEdgeInsets`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeInsets {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl EdgeInsets {
    pub const ZERO: EdgeInsets = EdgeInsets { top: 0.0, left: 0.0, bottom: 0.0, right: 0.0 };
    pub fn all(v: f32) -> Self {
        EdgeInsets { top: v, left: v, bottom: v, right: v }
    }
    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        EdgeInsets { top: vertical, left: horizontal, bottom: vertical, right: horizontal }
    }
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

impl Rect {
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    /// Shrinks by `insets`; the size never goes negative (`NativeEdgeInsetsInsetRect`).
    pub fn inset(self, i: EdgeInsets) -> Rect {
        let w = (self.w - i.horizontal()).max(0.0);
        let h = (self.h - i.vertical()).max(0.0);
        // When the insets exceed the size, collapse to the centre of the original rather than past it.
        let x = if self.w >= i.horizontal() { self.x + i.left } else { self.x + self.w / 2.0 };
        let y = if self.h >= i.vertical() { self.y + i.top } else { self.y + self.h / 2.0 };
        Rect { x, y, w, h }
    }
    /// Slices `amount` off `edge`; returns `(slice, remainder)` (`CGRectDivide`). `amount` is clamped to the size.
    pub fn divide(self, amount: f32, edge: Edge) -> (Rect, Rect) {
        match edge {
            Edge::Top => {
                let a = amount.clamp(0.0, self.h);
                (Rect { h: a, ..self }, Rect { y: self.y + a, h: self.h - a, ..self })
            }
            Edge::Bottom => {
                let a = amount.clamp(0.0, self.h);
                (Rect { y: self.y + self.h - a, h: a, ..self }, Rect { h: self.h - a, ..self })
            }
            Edge::Left => {
                let a = amount.clamp(0.0, self.w);
                (Rect { w: a, ..self }, Rect { x: self.x + a, w: self.w - a, ..self })
            }
            Edge::Right => {
                let a = amount.clamp(0.0, self.w);
                (Rect { x: self.x + self.w - a, w: a, ..self }, Rect { w: self.w - a, ..self })
            }
        }
    }
    /// The overlap, or `None` when the rectangles only touch or are apart (`CGRectIntersection`).
    pub fn intersection(self, o: Rect) -> Option<Rect> {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        (r > x && b > y).then_some(Rect { x, y, w: r - x, h: b - y })
    }
    /// The smallest rectangle holding both (`CGRectUnion`).
    pub fn union(self, o: Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect { x, y, w: self.right().max(o.right()) - x, h: self.bottom().max(o.bottom()) - y }
    }
    /// A `w` x `h` rectangle centred in `self`.
    pub fn centered(self, w: f32, h: f32) -> Rect {
        Rect { x: self.x + (self.w - w) / 2.0, y: self.y + (self.h - h) / 2.0, w, h }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: Rect = Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };

    #[test]
    fn inset_shrinks_and_never_goes_negative() {
        let r = R.inset(EdgeInsets { top: 5.0, left: 10.0, bottom: 5.0, right: 20.0 });
        assert_eq!(r, Rect { x: 20.0, y: 25.0, w: 70.0, h: 40.0 });
        let tiny = R.inset(EdgeInsets::all(80.0));
        assert_eq!((tiny.w, tiny.h), (0.0, 0.0));
        assert!(R.contains(tiny.x, tiny.y), "a collapsed rect stays inside the original");
    }

    #[test]
    fn divide_slices_each_edge_and_the_parts_tile_the_whole() {
        for e in [Edge::Top, Edge::Left, Edge::Bottom, Edge::Right] {
            let (a, b) = R.divide(30.0, e);
            assert_eq!(a.union(b), R, "{e:?}");
            assert!(a.intersection(b).is_none(), "{e:?}: parts do not overlap");
            assert_eq!((a.w * a.h + b.w * b.h), R.w * R.h);
        }
        assert_eq!(R.divide(30.0, Edge::Left).0, Rect { x: 10.0, y: 20.0, w: 30.0, h: 50.0 });
        assert_eq!(R.divide(30.0, Edge::Right).0, Rect { x: 80.0, y: 20.0, w: 30.0, h: 50.0 });
        assert_eq!(R.divide(999.0, Edge::Top).1.h, 0.0, "amount is clamped");
        assert_eq!(R.divide(-5.0, Edge::Top).0.h, 0.0);
    }

    #[test]
    fn intersection_and_union() {
        let o = Rect { x: 60.0, y: 40.0, w: 100.0, h: 100.0 };
        assert_eq!(R.intersection(o), Some(Rect { x: 60.0, y: 40.0, w: 50.0, h: 30.0 }));
        assert_eq!(R.intersection(Rect { x: 110.0, y: 20.0, w: 5.0, h: 5.0 }), None, "touching is not overlapping");
        assert_eq!(R.union(o), Rect { x: 10.0, y: 20.0, w: 150.0, h: 120.0 });
    }

    #[test]
    fn centered_and_opposite() {
        assert_eq!(R.centered(20.0, 10.0), Rect { x: 50.0, y: 40.0, w: 20.0, h: 10.0 });
        assert_eq!(Edge::Top.opposite(), Edge::Bottom);
        assert_eq!(Edge::Left.opposite().opposite(), Edge::Left);
    }
}
