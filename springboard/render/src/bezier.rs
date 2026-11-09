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
        self.flatten_with_close(tolerance).into_iter().map(|(pts, _)| pts).collect()
    }

    /// Like [`flatten`](Self::flatten), but each polyline also says whether its subpath was explicitly closed, which
    /// strokes need to join the last segment to the first.
    pub fn flatten_with_close(&self, tolerance: f32) -> Vec<(Vec<Point>, bool)> {
        let tol = if tolerance > 0.0 { tolerance } else { 0.25 };
        let mut out: Vec<(Vec<Point>, bool)> = Vec::new();
        let mut cur: Point = (0.0, 0.0);
        for op in &self.ops {
            match *op {
                Op::Move(p) => {
                    out.push((vec![p], false));
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
                    if let Some(sub) = out.last_mut() {
                        sub.1 = true;
                        cur = sub.0[0];
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
        let (x0, y0, mw, mh, part) = self.fill_mask_in_bounds(w, h, rule);
        let mut mask = vec![0u8; w * h];
        for r in 0..mh {
            mask[(y0 + r) * w + x0..][..mw].copy_from_slice(&part[r * mw..][..mw]);
        }
        mask
    }

    /// The coverage of [`fill_mask`](Self::fill_mask) cut down to the part of the `w` by `h` box the path can touch:
    /// `(x0, y0, mw, mh, mask)`, where `mask` is the `mw` by `mh` rectangle at `(x0, y0)`, row-major, and every
    /// pixel outside it is 0. Only that rectangle is tested, so the cost follows the size of the path, not of the box.
    pub fn fill_mask_in_bounds(&self, w: usize, h: usize, rule: WindingRule) -> (usize, usize, usize, usize, Vec<u8>) {
        let polys = self.flatten(0.25);
        let (x0, y0, x1, y1) = pixel_span(&polys, w, h);
        let (mw, mh) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        let mut mask = vec![0u8; mw * mh];
        for y in 0..mh {
            for x in 0..mw {
                if inside(&polys, (x0 + x) as f32 + 0.5, (y0 + y) as f32 + 0.5, rule) {
                    mask[y * mw + x] = 255;
                }
            }
        }
        (x0, y0, mw, mh, mask)
    }

    /// Starts a new subpath at the last subpath start when a segment follows a close or no move.
    fn ensure_subpath(&mut self) {
        if self.current.is_none() || self.closed {
            let p = self.start;
            self.move_to(p);
        }
    }
}

fn push_point(out: &mut Vec<(Vec<Point>, bool)>, p: Point) {
    match out.last_mut() {
        Some(sub) => sub.0.push(p),
        None => out.push((vec![p], false)),
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

/// The pixels `x0..x1`, `y0..y1` of a `w` by `h` box whose centres can lie inside `polys`: the bounding box of the
/// points, one pixel wider on each side, clipped to the box. A centre outside it has no edge beside it, or only the
/// edges of closed subpaths all on one side, so [`inside`] is false there under either rule. Points that are not
/// finite give the whole box.
fn pixel_span(polys: &[Vec<Point>], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let mut lo = (f32::INFINITY, f32::INFINITY);
    let mut hi = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in polys.iter().flatten() {
        if !x.is_finite() || !y.is_finite() {
            return (0, 0, w, h);
        }
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    if lo.0 > hi.0 {
        return (0, 0, 0, 0);
    }
    let cut = |v: f32, max: usize| (v.clamp(0.0, max as f32) as usize).min(max);
    (cut(lo.0.floor() - 1.0, w), cut(lo.1.floor() - 1.0, h), cut(hi.0.ceil() + 1.0, w), cut(hi.1.ceil() + 1.0, h))
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

    #[test]
    fn flatten_with_close_reports_which_subpaths_were_closed() {
        let mut p = BezierPath::rect(0.0, 0.0, 2.0, 2.0);
        p.move_to((5.0, 5.0));
        p.line_to((6.0, 5.0));
        let subs = p.flatten_with_close(0.25);
        assert_eq!(subs.len(), 2);
        assert!(subs[0].1);
        assert!(!subs[1].1);
        assert_eq!(p.flatten(0.25).len(), 2);
    }

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

    /// The mask [`BezierPath::fill_mask`] gave before it was limited to the path's bounds: every pixel tested.
    fn every_pixel(p: &BezierPath, w: usize, h: usize, rule: WindingRule) -> Vec<u8> {
        let polys = p.flatten(0.25);
        (0..w * h).map(|i| if inside(&polys, (i % w) as f32 + 0.5, (i / w) as f32 + 0.5, rule) { 255 } else { 0 }).collect()
    }

    #[test]
    fn fill_tests_only_the_pixels_the_path_can_touch() {
        // A small rectangle in a large box: the mask covers its neighbourhood, not the 16 million pixel box.
        let (x0, y0, mw, mh, mask) = BezierPath::rect(2000.0, 3000.0, 4.0, 5.0).fill_mask_in_bounds(4000, 4000, WindingRule::NonZero);
        assert!(mw <= 6 && mh <= 7, "{mw} x {mh}");
        assert!(x0 <= 2000 && y0 <= 3000);
        assert_eq!(mask.len(), mw * mh);
        assert_eq!(count(&mask), 20);
        assert_eq!(mask[(3000 - y0) * mw + (2000 - x0)], 255);
        // A path wholly outside the box gives an empty mask.
        let (_, _, mw, mh, mask) = BezierPath::rect(-50.0, -50.0, 10.0, 10.0).fill_mask_in_bounds(20, 20, WindingRule::NonZero);
        assert_eq!((mw * mh, mask.len()), (0, 0));
    }

    #[test]
    fn bounded_fill_matches_testing_every_pixel() {
        let mut holed = BezierPath::rect(3.0, 2.0, 10.0, 10.0);
        holed.append(&BezierPath::rect(6.0, 5.0, 4.0, 4.0));
        let mut open = BezierPath::new();
        open.move_to((1.5, 1.5));
        open.line_to((17.2, 4.0));
        open.line_to((6.0, 18.7));
        let paths = [holed, open, BezierPath::oval(-6.0, 4.5, 20.0, 11.0), BezierPath::oval(9.0, 12.0, 30.0, 30.0), BezierPath::rect(0.0, 0.0, 20.0, 20.0)];
        for (i, p) in paths.iter().enumerate() {
            for rule in [WindingRule::NonZero, WindingRule::EvenOdd] {
                assert_eq!(p.fill_mask(20, 20, rule), every_pixel(p, 20, 20, rule), "path {i}");
            }
        }
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
