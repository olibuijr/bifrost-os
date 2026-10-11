//! Shape geometry as pure data: subpaths of points, bounds, length and trimming. Nothing is drawn here; the renderer
//! walks the points. Curves are flattened into straight segments when the path is built, so length and trimming are
//! exact for the polyline that is stored.
use crate::Rect;

/// Segments per quarter circle of a rounded corner.
const CORNER_STEPS: usize = 8;
/// Segments for a full ellipse.
const ELLIPSE_SEGMENTS: usize = 64;

/// A point in the same space as [`Rect`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    fn lerp(self, to: Point, t: f32) -> Point {
        Point { x: self.x + (to.x - self.x) * t, y: self.y + (to.y - self.y) * t }
    }

    fn dist(self, to: Point) -> f32 {
        ((to.x - self.x).powi(2) + (to.y - self.y).powi(2)).sqrt()
    }
}

/// One run of connected points. A closed run has an implicit edge from its last point back to its first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subpath {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// A path made of subpaths. Built from straight segments; use the `add_*` helpers for rectangles and rounded shapes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    subpaths: Vec<Subpath>,
}

impl Path {
    pub fn new() -> Self {
        Path::default()
    }

    /// Starts a new subpath at `p`.
    pub fn move_to(&mut self, p: Point) -> &mut Self {
        self.subpaths.push(Subpath { points: vec![p], closed: false });
        self
    }

    /// Adds a straight edge from the last point of the open subpath to `p`. Starts a subpath at `p` when none is open.
    pub fn line_to(&mut self, p: Point) -> &mut Self {
        let open = self.subpaths.last().map_or(false, |s| !s.closed);
        if open {
            if let Some(s) = self.subpaths.last_mut() {
                s.points.push(p);
            }
        } else {
            self.move_to(p);
        }
        self
    }

    /// Closes the current subpath, adding the edge back to its first point.
    pub fn close(&mut self) -> &mut Self {
        if let Some(s) = self.subpaths.last_mut() {
            s.closed = true;
        }
        self
    }

    /// Adds a closed rectangle covering `r`.
    pub fn add_rect(&mut self, r: Rect) -> &mut Self {
        let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
        self.move_to(Point { x: x0, y: y0 });
        self.line_to(Point { x: x1, y: y0 });
        self.line_to(Point { x: x1, y: y1 });
        self.line_to(Point { x: x0, y: y1 });
        self.close()
    }

    /// Adds a closed rectangle with rounded corners. `radius` is clamped to half the shorter side; each corner is
    /// flattened into `steps` segments per quarter circle (at least one).
    pub fn add_rounded_rect(&mut self, r: Rect, radius: f32, steps: usize) -> &mut Self {
        let rad = radius.max(0.0).min(r.w.min(r.h).max(0.0) / 2.0);
        let steps = steps.max(1);
        let (l, t, rt, b) = (r.x, r.y, r.x + r.w, r.y + r.h);
        // Each corner: centre, and the start angle of its quarter arc (y grows downwards, so the walk is clockwise).
        let corners = [(rt - rad, t + rad, -90.0f32), (rt - rad, b - rad, 0.0), (l + rad, b - rad, 90.0), (l + rad, t + rad, 180.0)];
        self.move_to(Point { x: l + rad, y: t });
        for (cx, cy, start) in corners {
            for i in 0..=steps {
                let a = (start + 90.0 * i as f32 / steps as f32).to_radians();
                self.line_to(Point { x: cx + rad * a.cos(), y: cy + rad * a.sin() });
            }
        }
        self.close()
    }

    /// Adds a closed ellipse inscribed in `r`, flattened into `segments` edges (at least eight).
    pub fn add_ellipse(&mut self, r: Rect, segments: usize) -> &mut Self {
        let n = segments.max(8);
        let (cx, cy, rx, ry) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            let p = Point { x: cx + rx * a.cos(), y: cy + ry * a.sin() };
            if i == 0 {
                self.move_to(p);
            } else {
                self.line_to(p);
            }
        }
        self.close()
    }

    /// The subpaths in drawing order.
    pub fn subpaths(&self) -> &[Subpath] {
        &self.subpaths
    }

    /// True when no subpath has a point.
    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|s| s.points.is_empty())
    }

    /// The smallest rect that holds every point, or `None` for an empty path.
    pub fn bounds(&self) -> Option<Rect> {
        let mut points = self.subpaths.iter().flat_map(|s| s.points.iter());
        let first = *points.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
        for p in points {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// Total length of every edge, closing edges included.
    pub fn length(&self) -> f32 {
        self.edges().map(|(p, q)| p.dist(q)).sum()
    }

    /// Every edge in order, with the closing edge of each closed subpath last.
    fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.subpaths.iter().flat_map(|s| {
            let n = s.points.len();
            let open = s.points.windows(2).map(|w| (w[0], w[1]));
            let closing = (s.closed && n > 1).then(|| (s.points[n - 1], s.points[0]));
            open.chain(closing)
        })
    }

    /// The part of the path between `from` and `to`, given as fractions of its length. Fractions are clamped to
    /// 0..=1, and an empty range or an empty path gives an empty path. The result is open polylines in path order.
    pub fn trimmed(&self, from: f32, to: f32) -> Path {
        let total = self.length();
        let mut out = Path::new();
        let (a, b) = (from.clamp(0.0, 1.0) * total, to.clamp(0.0, 1.0) * total);
        if total <= 0.0 || b <= a {
            return out;
        }
        let mut at = 0.0f32;
        let mut last: Option<Point> = None;
        for (p, q) in self.edges() {
            let len = p.dist(q);
            if len <= 0.0 {
                continue;
            }
            let (s0, s1) = (at, at + len);
            at = s1;
            let (lo, hi) = (s0.max(a), s1.min(b));
            if hi <= lo {
                continue;
            }
            let start = p.lerp(q, (lo - s0) / len);
            let end = p.lerp(q, (hi - s0) / len);
            let joined = last.map_or(false, |l| l.dist(start) < 1e-4);
            if !joined {
                out.move_to(start);
            }
            out.line_to(end);
            last = Some(end);
        }
        out
    }
}

/// The built-in shapes, each as a path inside a rect.
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
        let mut p = Path::new();
        match self {
            Shape::Rectangle => {
                p.add_rect(r);
            }
            Shape::RoundedRectangle(radius) => {
                p.add_rounded_rect(r, radius, CORNER_STEPS);
            }
            Shape::Capsule => {
                p.add_rounded_rect(r, r.w.min(r.h) / 2.0, CORNER_STEPS);
            }
            Shape::Ellipse => {
                p.add_ellipse(r, ELLIPSE_SEGMENTS);
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_rectangle_has_its_perimeter_and_its_own_bounds() {
        let r = Rect { x: 5.0, y: 7.0, w: 10.0, h: 20.0 };
        let p = Shape::Rectangle.path(r);
        assert!(near(p.length(), 60.0));
        assert_eq!(p.bounds(), Some(r));
        assert_eq!(p.subpaths().len(), 1);
        assert!(p.subpaths()[0].closed);
    }

    #[test]
    fn an_empty_path_has_no_bounds_and_no_length() {
        let p = Path::new();
        assert!(p.is_empty());
        assert_eq!(p.bounds(), None);
        assert_eq!(p.length(), 0.0);
        assert!(p.trimmed(0.0, 1.0).is_empty());
    }

    #[test]
    fn trimming_keeps_the_part_of_the_outline_between_the_fractions() {
        // Perimeter 40: the first half runs along the top edge and down the right edge to the bottom-right corner.
        let p = Shape::Rectangle.path(B);
        let half = p.trimmed(0.0, 0.5);
        assert!(near(half.length(), 20.0));
        let pts = &half.subpaths()[0].points;
        assert_eq!(pts.first(), Some(&Point { x: 0.0, y: 0.0 }));
        assert_eq!(pts.last(), Some(&Point { x: 10.0, y: 10.0 }));
        assert!(!half.subpaths()[0].closed, "a trimmed outline is open");
    }

    #[test]
    fn trimming_across_a_corner_joins_the_edges_into_one_run() {
        let mid = Shape::Rectangle.path(B).trimmed(0.25, 0.75);
        assert_eq!(mid.subpaths().len(), 1, "the two edges meet at the corner, so they stay one run");
        assert!(near(mid.length(), 20.0));
        let pts = &mid.subpaths()[0].points;
        assert_eq!(pts.as_slice(), &[Point { x: 10.0, y: 0.0 }, Point { x: 10.0, y: 10.0 }, Point { x: 0.0, y: 10.0 }]);
    }

    #[test]
    fn empty_and_reversed_ranges_give_nothing_and_fractions_are_clamped() {
        let p = Shape::Rectangle.path(B);
        assert!(p.trimmed(0.6, 0.4).is_empty(), "from after to");
        assert!(p.trimmed(0.5, 0.5).is_empty(), "no span");
        let whole = p.trimmed(-3.0, 9.0);
        assert!(near(whole.length(), 40.0), "fractions outside 0..=1 are clamped to the whole outline");
    }

    #[test]
    fn an_ellipse_touches_the_sides_of_its_rect() {
        let r = Rect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let b = Shape::Ellipse.path(r).bounds().unwrap();
        assert!(near(b.x, 0.0) && near(b.y, 0.0) && near(b.w, 100.0) && near(b.h, 50.0), "{b:?}");
        let circumference = std::f32::consts::PI * (3.0 * (50.0 + 25.0) - ((3.0 * 50.0 + 25.0) * (50.0 + 3.0 * 25.0)).sqrt());
        assert!((Shape::Ellipse.path(r).length() - circumference).abs() < 0.5, "flattened length tracks the ellipse");
    }

    #[test]
    fn a_rounded_rect_clamps_its_radius_and_stays_inside_the_rect() {
        let r = Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 };
        let p = Path::new().add_rounded_rect(r, 100.0, 4).clone();
        let b = p.bounds().unwrap();
        assert!(near(b.w, 40.0) && near(b.h, 20.0), "a radius larger than half the side is clamped");
        let perimeter = 2.0 * (40.0 + 20.0);
        assert!(p.length() < perimeter && p.length() > perimeter - 25.0, "corners cut the perimeter by a little");
    }

    #[test]
    fn a_capsule_is_a_rounded_rect_with_half_circle_ends() {
        let r = Rect { x: 0.0, y: 0.0, w: 100.0, h: 20.0 };
        let cap = Shape::Capsule.path(r);
        assert!(near(cap.bounds().unwrap().w, 100.0));
        let rounded = Shape::RoundedRectangle(10.0).path(r);
        assert!((cap.length() - rounded.length()).abs() < 0.01, "a radius of half the height is the capsule");
        assert!(Shape::RoundedRectangle(0.0).path(r).length() > 0.0);
    }

    #[test]
    fn line_to_without_an_open_subpath_starts_one() {
        let mut p = Path::new();
        p.line_to(Point { x: 1.0, y: 1.0 }).line_to(Point { x: 4.0, y: 5.0 });
        assert_eq!(p.subpaths().len(), 1);
        assert!(near(p.length(), 5.0), "a 3-4-5 segment");
        p.close();
        assert!(near(p.length(), 10.0), "closing adds the edge back, 5 more");
        p.line_to(Point { x: 0.0, y: 0.0 });
        assert_eq!(p.subpaths().len(), 2, "a closed subpath is not extended");
    }
}
