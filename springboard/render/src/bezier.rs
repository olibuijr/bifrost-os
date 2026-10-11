//! Bezier paths: a pure geometry model behind app-facing path drawing. A path
//! is a list of move, line, cubic-curve and close commands. Curves flatten to
//! polylines, and fills are tested with the non-zero or even-odd winding rule.
//! Nothing here touches a pixel buffer, so any painter can reuse the model.

/// A point in path space.
pub type Point = (f32, f32);

/// How overlapping subpaths decide what lies inside a filled path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindingRule {
    NonZero,
    EvenOdd,
}

/// Control-point distance, as a fraction of the radius, that approximates a quarter circle with one cubic.
const KAPPA: f32 = 0.552_284_8;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

/// A path built from move, line, cubic-curve and close commands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BezierPath {
    ops: Vec<Op>,
    current: Option<Point>,
    start: Point,
    closed: bool,
}

impl BezierPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// Closed rectangle with its top-left corner at `(x, y)`.
    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        let mut p = Self::new();
        p.move_to((x, y));
        p.line_to((x + w, y));
        p.line_to((x + w, y + h));
        p.line_to((x, y + h));
        p.close();
        p
    }

    /// Closed rounded rectangle. Radii are clamped to half the box, so a
    /// radius that is too large yields a pill or an ellipse inside the box.
    pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Self {
        let rx = rx.clamp(0.0, w.max(0.0) / 2.0);
        let ry = ry.clamp(0.0, h.max(0.0) / 2.0);
        let (r, b) = (x + w, y + h);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let mut p = Self::new();
        p.move_to((x + rx, y));
        p.line_to((r - rx, y));
        p.curve_to((r - rx + kx, y), (r, y + ry - ky), (r, y + ry));
        p.line_to((r, b - ry));
        p.curve_to((r, b - ry + ky), (r - rx + kx, b), (r - rx, b));
        p.line_to((x + rx, b));
        p.curve_to((x + rx - kx, b), (x, b - ry + ky), (x, b - ry));
        p.line_to((x, y + ry));
        p.curve_to((x, y + ry - ky), (x + rx - kx, y), (x + rx, y));
        p.close();
        p
    }

    /// Closed ellipse inscribed in the box at `(x, y)` of size `w` by `h`.
    pub fn oval(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::rounded_rect(x, y, w, h, w / 2.0, h / 2.0)
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn current_point(&self) -> Option<Point> {
        self.current
    }

    pub fn move_to(&mut self, p: Point) {
        self.ops.push(Op::Move(p));
        self.current = Some(p);
        self.start = p;
        self.closed = false;
    }

    pub fn line_to(&mut self, p: Point) {
        self.ensure_subpath();
        self.ops.push(Op::Line(p));
        self.current = Some(p);
    }

    pub fn curve_to(&mut self, c1: Point, c2: Point, p: Point) {
        self.ensure_subpath();
        self.ops.push(Op::Cubic(c1, c2, p));
        self.current = Some(p);
    }

    /// Closes the open subpath; the current point returns to its start.
    pub fn close(&mut self) {
        if self.current.is_some() && !self.closed {
            self.ops.push(Op::Close);
            self.closed = true;
            self.current = Some(self.start);
        }
    }

    /// Appends the subpaths of `other`. An empty `other` changes nothing.
    pub fn append(&mut self, other: &BezierPath) {
        if other.is_empty() {
            return;
        }
        self.ops.extend_from_slice(&other.ops);
        self.current = other.current;
        self.start = other.start;
        self.closed = other.closed;
    }

    /// Flattens every subpath to a polyline. `tolerance` bounds how far a
    /// straight chord may stray from a curve, in path units.
    pub fn flatten(&self, tolerance: f32) -> Vec<Vec<Point>> {
        let tol = if tolerance > 0.0 { tolerance } else { 0.25 };
        let mut out: Vec<Vec<Point>> = Vec::new();
        let mut cur: Point = (0.0, 0.0);
        for op in &self.ops {
            match *op {
                Op::Move(p) => {
                    out.push(vec![p]);
                    cur = p;
                }
                Op::Line(p) => {
                    push_point(&mut out, p);
                    cur = p;
                }
                Op::Cubic(c1, c2, p) => {
                    let len = dist(cur, c1) + dist(c1, c2) + dist(c2, p);
                    let n = ((len / tol).sqrt().ceil() as usize).clamp(1, 256);
                    for i in 1..=n {
                        push_point(&mut out, cubic_at(cur, c1, c2, p, i as f32 / n as f32));
                    }
                    cur = p;
                }
                Op::Close => {
                    if let Some(first) = out.last().and_then(|s| s.first().copied()) {
                        cur = first;
                    }
                }
            }
        }
        out
    }

    /// Bounding box of the flattened path as `(x, y, w, h)`, or `None` when empty.
    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let mut lo = (f32::INFINITY, f32::INFINITY);
        let mut hi = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut any = false;
        for sub in self.flatten(0.25) {
            for (x, y) in sub {
                any = true;
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
        }
        if any {
            Some((lo.0, lo.1, hi.0 - lo.0, hi.1 - lo.1))
        } else {
            None
        }
    }

    /// Whether the point lies inside the filled path.
    pub fn contains(&self, x: f32, y: f32, rule: WindingRule) -> bool {
        inside(&self.flatten(0.25), x, y, rule)
    }

    /// Hard-edged mask of a `w` by `h` box: 255 where the pixel centre is
    /// inside the path, 0 elsewhere, row-major.
    pub fn fill_mask(&self, w: usize, h: usize, rule: WindingRule) -> Vec<u8> {
        let polys = self.flatten(0.25);
        let mut mask = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                if inside(&polys, x as f32 + 0.5, y as f32 + 0.5, rule) {
                    mask[y * w + x] = 255;
                }
            }
        }
        mask
    }

    /// Starts a new subpath at the last subpath start when a segment follows a close or no move.
    fn ensure_subpath(&mut self) {
        if self.current.is_none() || self.closed {
            let p = self.start;
            self.move_to(p);
        }
    }
}

fn push_point(out: &mut Vec<Vec<Point>>, p: Point) {
    match out.last_mut() {
        Some(sub) => sub.push(p),
        None => out.push(vec![p]),
    }
}

fn dist(a: Point, b: Point) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

fn cubic_at(p0: Point, p1: Point, p2: Point, p3: Point, t: f32) -> Point {
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// Crossing test along a ray to the right of `(x, y)`. Subpaths close implicitly.
fn inside(polys: &[Vec<Point>], x: f32, y: f32, rule: WindingRule) -> bool {
    let mut winding = 0i32;
    let mut crossings = 0u32;
    for sub in polys {
        let n = sub.len();
        for i in 0..n {
            let a = sub[i];
            let b = sub[(i + 1) % n];
            if (a.1 <= y) == (b.1 <= y) {
                continue;
            }
            let t = (y - a.1) / (b.1 - a.1);
            if a.0 + t * (b.0 - a.0) > x {
                crossings += 1;
                winding += if b.1 > a.1 { 1 } else { -1 };
            }
        }
    }
    match rule {
        WindingRule::NonZero => winding != 0,
        WindingRule::EvenOdd => crossings % 2 == 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(mask: &[u8]) -> usize {
        mask.iter().filter(|&&v| v == 255).count()
    }

    #[test]
    fn rect_bounds_and_fill_cover_whole_pixels() {
        let p = BezierPath::rect(2.0, 3.0, 4.0, 5.0);
        assert_eq!(p.bounds(), Some((2.0, 3.0, 4.0, 5.0)));
        let mask = p.fill_mask(10, 10, WindingRule::NonZero);
        assert_eq!(count(&mask), 20);
        assert_eq!(mask[3 * 10 + 2], 255);
        assert_eq!(mask[2 * 10 + 2], 0);
    }

    #[test]
    fn oval_fill_approximates_its_area() {
        let mask = BezierPath::oval(0.0, 0.0, 20.0, 20.0).fill_mask(20, 20, WindingRule::NonZero);
        let area = count(&mask) as f32;
        let circle = std::f32::consts::PI * 100.0;
        assert!((area - circle).abs() < 16.0, "area {area}");
    }

    #[test]
    fn even_odd_leaves_a_hole_that_non_zero_fills() {
        let mut p = BezierPath::rect(0.0, 0.0, 10.0, 10.0);
        p.append(&BezierPath::rect(3.0, 3.0, 4.0, 4.0));
        assert_eq!(count(&p.fill_mask(10, 10, WindingRule::NonZero)), 100);
        assert_eq!(count(&p.fill_mask(10, 10, WindingRule::EvenOdd)), 84);
    }

    #[test]
    fn rounded_rect_radii_clamp_to_the_box() {
        let p = BezierPath::rounded_rect(0.0, 0.0, 10.0, 4.0, 100.0, 100.0);
        assert_eq!(p.bounds(), Some((0.0, 0.0, 10.0, 4.0)));
        assert!(p.contains(5.0, 2.0, WindingRule::NonZero));
        // The corner is rounded away, so the box corner is outside the shape.
        assert!(!p.contains(0.1, 0.1, WindingRule::NonZero));
    }

    #[test]
    fn cubic_flatten_ends_on_the_last_control_point() {
        let mut p = BezierPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, 10.0), (10.0, 10.0), (10.0, 0.0));
        let polys = p.flatten(0.1);
        assert_eq!(polys.len(), 1);
        let pts = &polys[0];
        assert!(pts.len() > 2);
        assert_eq!(*pts.last().unwrap(), (10.0, 0.0));
        // The curve peaks at y = 7.5 (t = 0.5) and stays inside its control hull.
        let max_y = pts.iter().map(|p| p.1).fold(0.0f32, f32::max);
        assert!((max_y - 7.5).abs() < 0.05, "peak {max_y}");
    }

    #[test]
    fn line_after_close_starts_a_new_subpath_at_the_old_start() {
        let mut p = BezierPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((4.0, 0.0));
        p.line_to((0.0, 4.0));
        p.close();
        p.line_to((9.0, 9.0));
        let polys = p.flatten(0.25);
        assert_eq!(polys.len(), 2);
        assert_eq!(polys[1][0], (0.0, 0.0));
        assert_eq!(polys[1][1], (9.0, 9.0));
    }

    #[test]
    fn empty_path_has_no_bounds_or_fill() {
        let p = BezierPath::new();
        assert!(p.is_empty());
        assert_eq!(p.bounds(), None);
        assert_eq!(count(&p.fill_mask(4, 4, WindingRule::EvenOdd)), 0);
    }
}
