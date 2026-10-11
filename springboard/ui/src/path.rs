//! Paths: move, line, cubic curve and close elements, built by the app and flattened to polylines for filling,
//! stroking and bounds (the path type of the reference SDK, without the context that draws it). A path is data;
//! the renderer rasterises the polylines it gets from `polylines`.
use crate::affine::AffineTransform;
use crate::Rect;

pub type Point = (f32, f32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Element {
    Move(Point),
    Line(Point),
    Cubic { c1: Point, c2: Point, to: Point },
    /// Closes the current subpath back to its move point.
    Close,
}

/// Control points of a quarter ellipse arc, as a fraction of the radius (4/3 * (sqrt(2) - 1)).
const KAPPA: f32 = 0.552_284_8;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    elements: Vec<Element>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn move_to(&mut self, p: Point) {
        self.elements.push(Element::Move(p));
    }

    pub fn line_to(&mut self, p: Point) {
        self.elements.push(Element::Line(p));
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, to: Point) {
        self.elements.push(Element::Cubic { c1, c2, to });
    }

    pub fn close(&mut self) {
        self.elements.push(Element::Close);
    }

    /// A closed rectangle subpath, starting at the top left corner.
    pub fn add_rect(&mut self, r: Rect) {
        self.move_to((r.x, r.y));
        self.line_to((r.right(), r.y));
        self.line_to((r.right(), r.bottom()));
        self.line_to((r.x, r.bottom()));
        self.close();
    }

    /// A closed ellipse subpath inscribed in `r`, drawn as four cubic quarter arcs.
    pub fn add_ellipse_in(&mut self, r: Rect) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (rx, ry) = (r.w / 2.0, r.h / 2.0);
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        self.move_to((cx + rx, cy));
        self.cubic_to((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry));
        self.cubic_to((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy));
        self.cubic_to((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry));
        self.cubic_to((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy));
        self.close();
    }

    /// A copy with every point mapped through `t`.
    pub fn transformed(&self, t: AffineTransform) -> Path {
        let map = |p: Point| t.apply(p.0, p.1);
        let elements = self
            .elements
            .iter()
            .map(|e| match *e {
                Element::Move(p) => Element::Move(map(p)),
                Element::Line(p) => Element::Line(map(p)),
                Element::Cubic { c1, c2, to } => Element::Cubic { c1: map(c1), c2: map(c2), to: map(to) },
                Element::Close => Element::Close,
            })
            .collect();
        Path { elements }
    }

    /// The path as polylines, one per subpath. Curves are split until each piece is within about `tolerance`
    /// points of its estimated length. A subpath with a single point (a lone move) produces nothing.
    pub fn polylines(&self, tolerance: f32) -> Vec<Vec<Point>> {
        let tol = tolerance.max(1e-3);
        let mut out: Vec<Vec<Point>> = Vec::new();
        let mut cur: Vec<Point> = Vec::new();
        let mut start: Point = (0.0, 0.0);
        let mut last: Point = (0.0, 0.0);
        for e in &self.elements {
            match *e {
                Element::Move(p) => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.push(p);
                    start = p;
                    last = p;
                }
                Element::Line(p) => {
                    open_at(&mut cur, last);
                    cur.push(p);
                    last = p;
                }
                Element::Cubic { c1, c2, to } => {
                    open_at(&mut cur, last);
                    let est = dist(last, c1) + dist(c1, c2) + dist(c2, to);
                    let steps = ((est / tol).ceil() as usize).clamp(1, 512);
                    for i in 1..=steps {
                        cur.push(cubic_point(last, c1, c2, to, i as f32 / steps as f32));
                    }
                    last = to;
                }
                Element::Close => {
                    if !cur.is_empty() {
                        cur.push(start);
                        out.push(std::mem::take(&mut cur));
                    }
                    last = start;
                }
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out.retain(|line| line.len() > 1);
        out
    }

    /// The bounding box of the flattened path, or `None` when nothing is drawn.
    pub fn bounds(&self) -> Option<Rect> {
        let mut min = (f32::INFINITY, f32::INFINITY);
        let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for p in self.polylines(0.1).iter().flatten() {
            min = (min.0.min(p.0), min.1.min(p.1));
            max = (max.0.max(p.0), max.1.max(p.1));
        }
        (min.0 <= max.0).then(|| Rect { x: min.0, y: min.1, w: max.0 - min.0, h: max.1 - min.1 })
    }
}

fn open_at(cur: &mut Vec<Point>, at: Point) {
    if cur.is_empty() {
        cur.push(at);
    }
}

fn dist(a: Point, b: Point) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

fn cubic_point(p0: Point, c1: Point, c2: Point, p3: Point, t: f32) -> Point {
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    (a * p0.0 + b * c1.0 + c * c2.0 + d * p3.0, a * p0.1 + b * c1.1 + c * c2.1 + d * p3.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    #[test]
    fn rect_path_is_one_closed_polyline_with_the_rect_bounds() {
        let mut p = Path::new();
        p.add_rect(R);
        let lines = p.polylines(1.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 5, "four corners and the closing point");
        assert_eq!(lines[0].first(), lines[0].last(), "a closed subpath returns to its start");
        assert_eq!(p.bounds(), Some(R));
    }

    #[test]
    fn ellipse_bounds_match_the_inscribed_rect() {
        let mut p = Path::new();
        p.add_ellipse_in(R);
        let b = p.bounds().expect("an ellipse draws");
        assert!(near(b.x, R.x) && near(b.y, R.y) && near(b.w, R.w) && near(b.h, R.h), "{:?}", b);
        // Every flattened point stays on the ellipse, so none falls outside the rect.
        for pt in p.polylines(0.25).iter().flatten() {
            assert!(pt.0 >= R.x - 0.01 && pt.0 <= R.right() + 0.01 && pt.1 >= R.y - 0.01 && pt.1 <= R.bottom() + 0.01);
        }
    }

    #[test]
    fn straight_cubic_flattens_onto_its_line_and_ends_at_its_end_point() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.cubic_to((10.0, 0.0), (20.0, 0.0), (30.0, 0.0));
        let lines = p.polylines(1.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() > 2, "a curve is split into several points");
        assert!(lines[0].iter().all(|pt| pt.1.abs() < 1e-4), "all points stay on the line");
        assert_eq!(*lines[0].last().unwrap(), (30.0, 0.0));
    }

    #[test]
    fn transformed_path_moves_its_bounds() {
        let mut p = Path::new();
        p.add_rect(R);
        let moved = p.transformed(AffineTransform::translate(5.0, -5.0));
        assert_eq!(moved.bounds(), Some(Rect { x: 15.0, y: 15.0, ..R }));
        let scaled = p.transformed(AffineTransform::scale(2.0, 1.0));
        assert_eq!(scaled.bounds().map(|b| b.w), Some(200.0));
    }

    #[test]
    fn empty_path_and_lone_move_draw_nothing() {
        let empty = Path::new();
        assert!(empty.is_empty());
        assert!(empty.polylines(1.0).is_empty());
        assert_eq!(empty.bounds(), None);
        let mut lone = Path::new();
        lone.move_to((4.0, 4.0));
        assert!(lone.polylines(1.0).is_empty());
        assert_eq!(lone.bounds(), None);
    }

    #[test]
    fn line_after_close_starts_at_the_subpath_start() {
        let mut p = Path::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.close();
        p.line_to((10.0, 10.0));
        let lines = p.polylines(1.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], vec![(0.0, 0.0), (10.0, 10.0)]);
    }
}
