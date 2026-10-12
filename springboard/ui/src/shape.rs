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

    /// Whether the point lies inside the filled path. Every subpath is closed for the test, as a fill closes it.
    /// Non-zero counts signed crossings of a ray to the right, even-odd counts all of them.
    pub fn contains(&self, x: f32, y: f32, rule: FillRule) -> bool {
        let mut winding = 0i32;
        let mut crossings = 0u32;
        for s in &self.subpaths {
            let n = s.points.len();
            if n < 2 {
                continue;
            }
            for i in 0..n {
                let (a, b) = (s.points[i], s.points[(i + 1) % n]);
                if (a.y <= y) != (b.y <= y) {
                    let xi = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                    if xi > x {
                        crossings += 1;
                        winding += if b.y > a.y { 1 } else { -1 };
                    }
                }
            }
        }
        match rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => crossings % 2 == 1,
        }
    }

    /// The path split into dashes, following a dash pattern (the reference SDK's line dash pattern). `pattern` lists
    /// on and off lengths in path units and repeats; `phase` moves the start into the pattern. An empty pattern, a
    /// negative length or a pattern that sums to zero returns the path unchanged. Each dash is an open subpath, and
    /// a dash never joins across subpaths.
    pub fn dashed(&self, pattern: &[f32], phase: f32) -> Path {
        let total: f32 = pattern.iter().sum();
        if pattern.is_empty() || pattern.iter().any(|&d| d < 0.0) || total <= 0.0 {
            return self.clone();
        }
        let n = pattern.len();
        // Walk the phase into the pattern: `idx` is the entry, `left` what remains of it.
        let (mut idx, mut left, mut p) = (0usize, pattern[0], phase.rem_euclid(total));
        while p > 0.0 {
            if p >= left {
                p -= left;
                idx = (idx + 1) % n;
                left = pattern[idx];
            } else {
                left -= p;
                p = 0.0;
            }
        }
        let mut out = Path::new();
        for s in &self.subpaths {
            let k = s.points.len();
            let mut edges: Vec<(Point, Point)> = s.points.windows(2).map(|w| (w[0], w[1])).collect();
            if s.closed && k > 1 {
                edges.push((s.points[k - 1], s.points[0]));
            }
            let mut open = false;
            for (p0, p1) in edges {
                let len = p0.dist(p1);
                let mut t0 = 0.0f32;
                while len > 0.0 && t0 < len {
                    let step = left.min(len - t0);
                    if idx % 2 == 0 {
                        let (a, b) = (p0.lerp(p1, t0 / len), p0.lerp(p1, (t0 + step) / len));
                        if !open {
                            out.move_to(a);
                            open = true;
                        }
                        out.line_to(b);
                    }
                    t0 += step;
                    left -= step;
                    if left <= 0.0 {
                        idx = (idx + 1) % n;
                        left = pattern[idx];
                        if idx % 2 == 1 {
                            open = false;
                        }
                    }
                }
            }
        }
        out
    }
}

/// How overlapping subpaths decide what is inside a filled path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// How the open ends of a stroke are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// How two stroke edges meet at a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// The stroke settings of a shape (the reference SDK's line width, cap, join and miter limit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    /// The longest miter, as a multiple of the line width, before the corner falls back to a bevel.
    pub miter_limit: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        StrokeStyle { width: 1.0, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 10.0 }
    }
}

impl StrokeStyle {
    /// The join drawn at a corner whose interior angle is `angle` radians. A miter reaches `1 / sin(angle / 2)`
    /// line widths from the corner; past `miter_limit` that becomes a bevel. Round and bevel joins stay as set.
    pub fn effective_join(&self, angle: f32) -> LineJoin {
        match self.join {
            LineJoin::Miter => {
                let s = (angle / 2.0).sin().abs();
                if s <= 1e-6 || 1.0 / s > self.miter_limit {
                    LineJoin::Bevel
                } else {
                    LineJoin::Miter
                }
            }
            other => other,
        }
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
        let circumference = std::f32::consts::PI * (3.0 * (50.0 + 25.0) - ((3.0f32 * 50.0 + 25.0) * (50.0 + 3.0 * 25.0)).sqrt());
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

    fn square(x0: f32, size: f32) -> Path {
        let mut p = Path::new();
        p.add_rect(Rect { x: x0, y: 0.0, w: size, h: size });
        p
    }

    #[test]
    fn contains_follows_the_fill_rule_for_nested_subpaths() {
        // Two same-direction squares, one inside the other: the centre has winding 2.
        let mut p = square(0.0, 10.0);
        p.add_rect(Rect { x: 2.0, y: 2.0, w: 6.0, h: 6.0 });
        assert!(p.contains(5.0, 5.0, FillRule::NonZero));
        assert!(!p.contains(5.0, 5.0, FillRule::EvenOdd), "two crossings cancel under even-odd");
        assert!(p.contains(1.0, 1.0, FillRule::EvenOdd), "the ring between the squares is inside");
        assert!(!p.contains(12.0, 5.0, FillRule::NonZero), "outside both");
    }

    #[test]
    fn contains_reverses_winding_for_a_hole() {
        // Outer square one way, inner square the other way: the hole has winding 0 under non-zero.
        let mut p = square(0.0, 10.0);
        let mut hole = Path::new();
        hole.move_to(Point { x: 2.0, y: 2.0 });
        hole.line_to(Point { x: 2.0, y: 8.0 });
        hole.line_to(Point { x: 8.0, y: 8.0 });
        hole.line_to(Point { x: 8.0, y: 2.0 });
        hole.close();
        p.subpaths.push(hole.subpaths[0].clone());
        assert!(!p.contains(5.0, 5.0, FillRule::NonZero));
        assert!(!p.contains(5.0, 5.0, FillRule::EvenOdd));
        assert!(p.contains(1.0, 5.0, FillRule::NonZero));
    }

    #[test]
    fn miter_joins_fall_back_to_bevel_past_the_limit() {
        let style = StrokeStyle::default();
        let right = std::f32::consts::FRAC_PI_2;
        assert_eq!(style.effective_join(right), LineJoin::Miter, "a right angle reaches 1.41 widths");
        // 10 degrees: 1 / sin(5 degrees) is about 11.5, above the limit of 10.
        assert_eq!(style.effective_join(10f32.to_radians()), LineJoin::Bevel);
        assert_eq!(style.effective_join(30f32.to_radians()), LineJoin::Miter, "3.86 widths is within the limit");
        let round = StrokeStyle { join: LineJoin::Round, ..style };
        assert_eq!(round.effective_join(10f32.to_radians()), LineJoin::Round, "only miter joins fall back");
    }

    #[test]
    fn dashes_follow_the_pattern_along_a_line() {
        let mut line = Path::new();
        line.move_to(Point { x: 0.0, y: 0.0 });
        line.line_to(Point { x: 10.0, y: 0.0 });
        // On 2, off 3: dashes over 0 to 2 and 5 to 7, the rest is off.
        let d = line.dashed(&[2.0, 3.0], 0.0);
        assert_eq!(d.subpaths().len(), 2);
        assert_eq!(d.subpaths()[0].points, vec![Point { x: 0.0, y: 0.0 }, Point { x: 2.0, y: 0.0 }]);
        assert_eq!(d.subpaths()[1].points, vec![Point { x: 5.0, y: 0.0 }, Point { x: 7.0, y: 0.0 }]);
        assert!(near(d.length(), 4.0));
    }

    #[test]
    fn dash_phase_moves_the_start_into_the_pattern() {
        let mut line = Path::new();
        line.move_to(Point { x: 0.0, y: 0.0 });
        line.line_to(Point { x: 10.0, y: 0.0 });
        // A phase of 2 uses up the first on-length, so the line starts with 3 units off.
        let d = line.dashed(&[2.0, 3.0], 2.0);
        assert_eq!(d.subpaths().len(), 2);
        assert_eq!(d.subpaths()[0].points[0], Point { x: 3.0, y: 0.0 });
        assert_eq!(d.subpaths()[1].points[0], Point { x: 8.0, y: 0.0 });
    }

    #[test]
    fn invalid_dash_patterns_return_the_path_whole() {
        let mut line = Path::new();
        line.move_to(Point { x: 0.0, y: 0.0 });
        line.line_to(Point { x: 10.0, y: 0.0 });
        assert_eq!(line.dashed(&[], 0.0), line);
        assert_eq!(line.dashed(&[0.0, 0.0], 0.0), line);
        assert_eq!(line.dashed(&[2.0, -1.0], 0.0), line);
    }
}
