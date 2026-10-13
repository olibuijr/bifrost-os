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

/// A displacement in the plane: how far to move along x and y (`CGVector`). It is a direction and a length, not a
/// position, so it has no origin; add it to a point to move the point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector {
    pub dx: f32,
    pub dy: f32,
}

impl Vector {
    pub const ZERO: Vector = Vector { dx: 0.0, dy: 0.0 };

    pub fn new(dx: f32, dy: f32) -> Self {
        Vector { dx, dy }
    }
    /// The displacement from point `a` to point `b`.
    pub fn between(a: (f32, f32), b: (f32, f32)) -> Self {
        Vector { dx: b.0 - a.0, dy: b.1 - a.1 }
    }
    pub fn length(self) -> f32 {
        self.dx.hypot(self.dy)
    }
    pub fn dot(self, o: Vector) -> f32 {
        self.dx * o.dx + self.dy * o.dy
    }
    pub fn scaled(self, k: f32) -> Vector {
        Vector { dx: self.dx * k, dy: self.dy * k }
    }
    /// The vector of length 1 in the same direction; the zero vector stays zero.
    pub fn normalized(self) -> Vector {
        let l = self.length();
        if l == 0.0 {
            Vector::ZERO
        } else {
            self.scaled(1.0 / l)
        }
    }
    /// The point `p` moved by this vector.
    pub fn apply(self, p: (f32, f32)) -> (f32, f32) {
        (p.0 + self.dx, p.1 + self.dy)
    }
}

impl std::ops::Add for Vector {
    type Output = Vector;
    fn add(self, o: Vector) -> Vector {
        Vector { dx: self.dx + o.dx, dy: self.dy + o.dy }
    }
}

impl std::ops::Sub for Vector {
    type Output = Vector;
    fn sub(self, o: Vector) -> Vector {
        Vector { dx: self.dx - o.dx, dy: self.dy - o.dy }
    }
}

impl std::ops::Neg for Vector {
    type Output = Vector;
    fn neg(self) -> Vector {
        Vector { dx: -self.dx, dy: -self.dy }
    }
}

impl Rect {
    /// The rectangle moved by a vector; the size is unchanged.
    pub fn offset_by(self, v: Vector) -> Rect {
        Rect { x: self.x + v.dx, y: self.y + v.dy, ..self }
    }
}

/// A set of points kept as disjoint rectangles: the shape a hit area, a clip or a window outline is built from.
/// Every operation returns a new region. Empty rectangles are dropped and neighbouring pieces with the same edge are joined.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    rects: Vec<Rect>,
}

impl Region {
    pub fn new() -> Self {
        Region::default()
    }
    /// The region covering one rectangle; a rectangle with no area gives the empty region.
    pub fn from_rect(r: Rect) -> Self {
        if r.w > 0.0 && r.h > 0.0 {
            Region { rects: vec![r] }
        } else {
            Region::default()
        }
    }
    /// The disjoint rectangles that make up the region.
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rects.iter().any(|r| r.contains(x, y))
    }
    /// The smallest rectangle holding the whole region, or `None` when it is empty.
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(|a, b| a.union(b))
    }
    pub fn area(&self) -> f32 {
        self.rects.iter().map(|r| r.w * r.h).sum()
    }
    pub fn translated(&self, dx: f32, dy: f32) -> Region {
        Region { rects: self.rects.iter().map(|r| Rect { x: r.x + dx, y: r.y + dy, ..*r }).collect() }
    }
    /// Everything covered by either region.
    pub fn union(&self, o: &Region) -> Region {
        combine(&self.rects, &o.rects, |a, b| a || b)
    }
    /// What both regions cover.
    pub fn intersection(&self, o: &Region) -> Region {
        combine(&self.rects, &o.rects, |a, b| a && b)
    }
    /// What `self` covers and `o` does not.
    pub fn subtract(&self, o: &Region) -> Region {
        combine(&self.rects, &o.rects, |a, b| a && !b)
    }
    /// What exactly one of the two regions covers.
    pub fn xor(&self, o: &Region) -> Region {
        combine(&self.rects, &o.rects, |a, b| a != b)
    }
}

/// Boolean combination of two rectangle sets. Cuts the plane into vertical slabs at every edge, cuts each slab into runs at
/// every horizontal edge, and keeps the runs where `keep(in_a, in_b)` holds.
fn combine(a: &[Rect], b: &[Rect], keep: impl Fn(bool, bool) -> bool) -> Region {
    let mut xs: Vec<f32> = a.iter().chain(b).flat_map(|r| [r.x, r.right()]).collect();
    xs.sort_by(|p, q| p.total_cmp(q));
    xs.dedup();
    let mut out: Vec<Rect> = Vec::new();
    for pair in xs.windows(2) {
        let (x0, x1) = (pair[0], pair[1]);
        let xm = (x0 + x1) / 2.0;
        let (ya, yb) = (spans_at(a, xm), spans_at(b, xm));
        let mut ys: Vec<f32> = ya.iter().chain(&yb).flat_map(|&(s, e)| [s, e]).collect();
        ys.sort_by(|p, q| p.total_cmp(q));
        ys.dedup();
        for seg in ys.windows(2) {
            let ym = (seg[0] + seg[1]) / 2.0;
            let in_a = ya.iter().any(|&(s, e)| s <= ym && ym < e);
            let in_b = yb.iter().any(|&(s, e)| s <= ym && ym < e);
            if keep(in_a, in_b) {
                push_joined(&mut out, Rect { x: x0, y: seg[0], w: x1 - x0, h: seg[1] - seg[0] });
            }
        }
    }
    Region { rects: out }
}

/// The vertical spans `(top, bottom)` of the rectangles that cross the vertical line `x`, sorted by top.
fn spans_at(rects: &[Rect], x: f32) -> Vec<(f32, f32)> {
    let mut spans: Vec<(f32, f32)> = rects.iter().filter(|r| r.x <= x && x < r.right()).map(|r| (r.y, r.bottom())).collect();
    spans.sort_by(|p, q| p.0.total_cmp(&q.0));
    spans
}

/// Adds `r`, widening an existing piece instead when it continues that piece's edge (same height and touching on the
/// side, or same width and touching below). The covered set is unchanged; the piece count stays small.
fn push_joined(out: &mut Vec<Rect>, r: Rect) {
    if let Some(p) = out.iter_mut().rev().find(|p| p.y == r.y && p.h == r.h && p.right() == r.x) {
        p.w += r.w;
    } else if let Some(p) = out.iter_mut().rev().find(|p| p.x == r.x && p.w == r.w && p.bottom() == r.y) {
        p.h += r.h;
    } else {
        out.push(r);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_vector_measures_and_moves() {
        let v = Vector::between((1.0, 2.0), (4.0, 6.0));
        assert_eq!(v, Vector::new(3.0, 4.0));
        assert_eq!(v.length(), 5.0);
        assert_eq!(v.apply((1.0, 2.0)), (4.0, 6.0));
        assert_eq!(v.dot(Vector::new(1.0, 0.0)), 3.0);
        assert_eq!(v.scaled(2.0), Vector::new(6.0, 8.0));
        assert_eq!(-v, Vector::new(-3.0, -4.0));
        assert_eq!(v + Vector::new(1.0, 1.0), Vector::new(4.0, 5.0));
        assert_eq!(v - v, Vector::ZERO);
    }

    #[test]
    fn normalizing_gives_unit_length_and_keeps_zero() {
        let n = Vector::new(0.0, -8.0).normalized();
        assert_eq!(n, Vector::new(0.0, -1.0));
        assert_eq!(Vector::ZERO.normalized(), Vector::ZERO);
    }

    #[test]
    fn a_rectangle_moves_by_a_vector() {
        let r = Rect { x: 1.0, y: 1.0, w: 5.0, h: 6.0 };
        assert_eq!(r.offset_by(Vector::new(2.0, -1.0)), Rect { x: 3.0, y: 0.0, w: 5.0, h: 6.0 });
    }
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

    fn sq(x: f32, y: f32, w: f32, h: f32) -> Region {
        Region::from_rect(Rect { x, y, w, h })
    }

    #[test]
    fn a_region_from_a_rectangle_is_that_rectangle_and_empty_has_no_area() {
        let r = sq(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.bounds(), Some(R));
        assert_eq!(r.area(), 5000.0);
        assert!(r.contains(10.0, 20.0) && !r.contains(110.0, 20.0), "the right edge is outside");
        assert!(sq(0.0, 0.0, 0.0, 5.0).is_empty(), "no width gives the empty region");
        assert!(Region::new().bounds().is_none());
    }

    #[test]
    fn union_covers_both_once_and_intersection_keeps_the_overlap() {
        let a = sq(0.0, 0.0, 10.0, 10.0);
        let b = sq(5.0, 5.0, 10.0, 10.0);
        let u = a.union(&b);
        assert_eq!(u.area(), 175.0, "100 + 100 - the 25 they share");
        assert!(u.contains(12.0, 12.0) && u.contains(2.0, 2.0) && !u.contains(16.0, 2.0));
        let i = a.intersection(&b);
        assert_eq!(i.bounds(), Some(Rect { x: 5.0, y: 5.0, w: 5.0, h: 5.0 }));
        assert_eq!(i.area(), 25.0);
        assert!(a.intersection(&sq(50.0, 50.0, 1.0, 1.0)).is_empty(), "apart regions share nothing");
    }

    #[test]
    fn subtract_leaves_the_part_not_covered_and_xor_the_part_covered_once() {
        let a = sq(0.0, 0.0, 10.0, 10.0);
        let left = sq(0.0, 0.0, 5.0, 10.0);
        let rest = a.subtract(&left);
        assert_eq!(rest.bounds(), Some(Rect { x: 5.0, y: 0.0, w: 5.0, h: 10.0 }));
        assert_eq!(rest.area(), 50.0);
        assert!(!rest.contains(2.0, 2.0) && rest.contains(7.0, 2.0));
        assert!(a.subtract(&a).is_empty(), "a region minus itself is empty");
        let x = a.xor(&sq(5.0, 5.0, 10.0, 10.0));
        assert_eq!(x.area(), 150.0, "union 175 minus intersection 25");
        assert!(!x.contains(7.0, 7.0), "the shared corner is not in the xor");
    }

    #[test]
    fn neighbouring_pieces_with_the_same_edge_are_joined() {
        let u = sq(0.0, 0.0, 5.0, 5.0).union(&sq(5.0, 0.0, 5.0, 5.0));
        assert_eq!(u.rects().len(), 1);
        assert_eq!(u.rects()[0], Rect { x: 0.0, y: 0.0, w: 10.0, h: 5.0 });
        let stacked = sq(0.0, 0.0, 4.0, 4.0).union(&sq(0.0, 4.0, 4.0, 4.0));
        assert_eq!(stacked.rects().len(), 1, "a vertical stack of equal width joins too");
    }

    #[test]
    fn translation_moves_every_piece_and_keeps_the_area() {
        let r = sq(1.0, 1.0, 2.0, 2.0).union(&sq(10.0, 10.0, 2.0, 2.0)).translated(10.0, 20.0);
        assert_eq!(r.area(), 8.0);
        assert!(r.contains(12.0, 22.0) && r.contains(20.0, 30.0));
        assert_eq!(r.bounds(), Some(Rect { x: 11.0, y: 21.0, w: 11.0, h: 11.0 }));
    }
}
