//! Path tessellation on the CPU: curves flattened to within 0.25 px, fills triangulated (non-zero and even-odd via
//! a trapezoid sweep), strokes expanded with round joins and caps. Output is a triangle list in device pixels with
//! per-vertex coverage for 1 px antialiased edges. Pure functions, host-tested.
//!
//! Method. Subpaths are flattened into straight edges (curves to `FLATTEN_TOLERANCE`). Every edge endpoint and every
//! pairwise crossing gives a horizontal breakpoint, so inside a band between two breakpoints no two edges cross and
//! the left-to-right order of the active edges is fixed. Walking that order with the winding count (non-zero: count
//! != 0; even-odd: count odd) gives the inside spans of each band as trapezoids (coverage 1). Each edge where the
//! inside status flips gets a 1 px fringe on its outside (coverage 1 on the outline, 0 one pixel out), and each
//! horizontal boundary piece between two bands gets the same fringe. A stroke is the union of its segment rectangles
//! and round joins and caps, all oriented the same way, filled non-zero.
//!
//! Output triangles are unoriented: the consumer must not cull them. Cost is O(breakpoints x active edges) plus the
//! O(n^2) crossing search (with a y-sorted sweep to prune it), fine for UI paths, not for whole-page vector art.
//!
//! Owner: wave G1 agent `gfx2d-images` (with the image cache).

use bifrost_render::canvas::{PathData, PathPaint, PathVerb};

/// Flattening tolerance for curves, in device pixels (the maximum chord error).
pub const FLATTEN_TOLERANCE: f64 = 0.25;
/// Width of the antialiased fringe, in device pixels, outside the exact outline.
pub const FRINGE: f64 = 1.0;

const MAX_CURVE_SEGMENTS: usize = 512;
const MIN_CIRCLE_SEGMENTS: usize = 24;
const MAX_CIRCLE_SEGMENTS: usize = 512;
/// Relative tolerance for merging breakpoints and points.
const EPS: f64 = 1e-9;

/// A vertex: position and edge coverage (1 inside, 0 at the antialiased fringe).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathVertex {
    pub x: f32,
    pub y: f32,
    pub coverage: f32,
}

/// A flattened subpath: its points in order, and whether it was closed by `Close`. Fills always treat a subpath as
/// closed.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub points: Vec<(f64, f64)>,
    pub closed: bool,
}

type Pt = (f64, f64);

/// Tessellates `path` for `paint` into a triangle list.
pub fn tessellate(path: &PathData, paint: PathPaint) -> Vec<PathVertex> {
    let subpaths = flatten(path);
    match paint {
        PathPaint::FillNonZero => fill(&subpaths, Rule::NonZero),
        PathPaint::FillEvenOdd => fill(&subpaths, Rule::EvenOdd),
        PathPaint::Stroke(width) => {
            if width.is_finite() && width > 0.0 {
                stroke(&subpaths, width as f64)
            } else {
                Vec::new()
            }
        }
    }
}

/// Flattens the path into subpaths of straight points. A `Line` with no current subpath starts one at the current
/// point, as the path model does after a `Close`.
pub fn flatten(path: &PathData) -> Vec<Subpath> {
    let mut out = Vec::new();
    let mut cur: Option<Subpath> = None;
    let mut last: Pt = (0.0, 0.0);
    let mut start: Pt = (0.0, 0.0);
    for verb in &path.verbs {
        match *verb {
            PathVerb::Move(x, y) => {
                if let Some(s) = cur.take() {
                    out.push(s);
                }
                last = (x as f64, y as f64);
                start = last;
                cur = Some(Subpath { points: vec![last], closed: false });
            }
            PathVerb::Line(x, y) => {
                let p = (x as f64, y as f64);
                current(&mut cur, last).points.push(p);
                last = p;
            }
            PathVerb::Quad(cx, cy, x, y) => {
                let (p0, c, p1) = (last, (cx as f64, cy as f64), (x as f64, y as f64));
                let n = quad_segments(p0, c, p1);
                let sub = current(&mut cur, last);
                for i in 1..=n {
                    sub.points.push(quad_at(p0, c, p1, i as f64 / n as f64));
                }
                last = p1;
            }
            PathVerb::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                let (p0, c1, c2, p1) = (last, (c1x as f64, c1y as f64), (c2x as f64, c2y as f64), (x as f64, y as f64));
                let n = cubic_segments(p0, c1, c2, p1);
                let sub = current(&mut cur, last);
                for i in 1..=n {
                    sub.points.push(cubic_at(p0, c1, c2, p1, i as f64 / n as f64));
                }
                last = p1;
            }
            PathVerb::Close => {
                if let Some(mut s) = cur.take() {
                    s.closed = true;
                    out.push(s);
                }
                last = start;
            }
        }
    }
    if let Some(s) = cur.take() {
        out.push(s);
    }
    out
}

fn current(cur: &mut Option<Subpath>, at: Pt) -> &mut Subpath {
    cur.get_or_insert_with(|| Subpath { points: vec![at], closed: false })
}

fn norm(a: Pt) -> f64 {
    (a.0 * a.0 + a.1 * a.1).sqrt()
}

/// Segments for a quadratic: the chord error of n uniform segments is at most |p0 - 2c + p1| / (4 n^2).
fn quad_segments(p0: Pt, c: Pt, p1: Pt) -> usize {
    let a = (p0.0 - 2.0 * c.0 + p1.0, p0.1 - 2.0 * c.1 + p1.1);
    let n = (norm(a) / (4.0 * FLATTEN_TOLERANCE)).sqrt().ceil();
    (n as usize).clamp(1, MAX_CURVE_SEGMENTS)
}

/// Segments for a cubic: the second derivative is bounded by 6 max(|A|, |B|), so the chord error is at most
/// 3 M / (4 n^2) with A = p0 - 2 p1 + p2, B = p1 - 2 p2 + p3.
fn cubic_segments(p0: Pt, p1: Pt, p2: Pt, p3: Pt) -> usize {
    let a = (p0.0 - 2.0 * p1.0 + p2.0, p0.1 - 2.0 * p1.1 + p2.1);
    let b = (p1.0 - 2.0 * p2.0 + p3.0, p1.1 - 2.0 * p2.1 + p3.1);
    let m = norm(a).max(norm(b));
    let n = (3.0 * m / (4.0 * FLATTEN_TOLERANCE)).sqrt().ceil();
    (n as usize).clamp(1, MAX_CURVE_SEGMENTS)
}

fn quad_at(p0: Pt, c: Pt, p1: Pt, t: f64) -> Pt {
    let mt = 1.0 - t;
    (
        mt * mt * p0.0 + 2.0 * mt * t * c.0 + t * t * p1.0,
        mt * mt * p0.1 + 2.0 * mt * t * c.1 + t * t * p1.1,
    )
}

fn cubic_at(p0: Pt, p1: Pt, p2: Pt, p3: Pt, t: f64) -> Pt {
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
}

fn push_tri(out: &mut Vec<PathVertex>, a: (f64, f64, f32), b: (f64, f64, f32), c: (f64, f64, f32)) {
    for (x, y, coverage) in [a, b, c] {
        out.push(PathVertex { x: x as f32, y: y as f32, coverage });
    }
}

/// Two triangles for the quad a, b, c, d (given in boundary order).
fn push_quad(out: &mut Vec<PathVertex>, a: (f64, f64, f32), b: (f64, f64, f32), c: (f64, f64, f32), d: (f64, f64, f32)) {
    push_tri(out, a, b, c);
    push_tri(out, a, c, d);
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Rule {
    NonZero,
    EvenOdd,
}

/// A non-horizontal edge, stored top to bottom (`y0 < y1`). `dir` is the winding contribution.
#[derive(Debug, Clone, Copy)]
struct Edge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    dir: i32,
}

impl Edge {
    fn x_at(&self, y: f64) -> f64 {
        self.x0 + (self.x1 - self.x0) * ((y - self.y0) / (self.y1 - self.y0))
    }

    fn mid_x_between(&self, lo: f64, hi: f64) -> f64 {
        0.5 * (self.x_at(lo) + self.x_at(hi))
    }
}

fn tol(v: f64) -> f64 {
    EPS * v.abs().max(1.0)
}

/// Sorts and merges values closer than the relative tolerance.
fn sorted_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.total_cmp(b));
    let mut out: Vec<f64> = Vec::with_capacity(v.len());
    for x in v {
        match out.last() {
            Some(&last) if (x - last).abs() <= tol(last) => {}
            _ => out.push(x),
        }
    }
    out
}

fn edges_of(subpaths: &[Subpath]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for sp in subpaths {
        // A subpath with a non-finite point is dropped whole: dropping one point would change the shape.
        if sp.points.iter().any(|p| !(p.0.is_finite() && p.1.is_finite())) {
            continue;
        }
        let pts = &sp.points;
        let n = pts.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            if a.1 == b.1 {
                continue;
            }
            if a.1 < b.1 {
                edges.push(Edge { x0: a.0, y0: a.1, x1: b.0, y1: b.1, dir: 1 });
            } else {
                edges.push(Edge { x0: b.0, y0: b.1, x1: a.0, y1: a.1, dir: -1 });
            }
        }
    }
    edges
}

/// The y of the crossing of two edges, if their segments cross.
fn crossing_y(a: &Edge, b: &Edge) -> Option<f64> {
    let (d1x, d1y) = (a.x1 - a.x0, a.y1 - a.y0);
    let (d2x, d2y) = (b.x1 - b.x0, b.y1 - b.y0);
    let denom = d1x * d2y - d1y * d2x;
    if denom == 0.0 {
        return None;
    }
    let (wx, wy) = (b.x0 - a.x0, b.y0 - a.y0);
    let t = (wx * d2y - wy * d2x) / denom;
    let u = (wx * d1y - wy * d1x) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(a.y0 + t * d1y)
    } else {
        None
    }
}

/// All horizontal breakpoints: edge endpoints and crossings.
fn breakpoints(edges: &[Edge]) -> Vec<f64> {
    let mut ys = Vec::with_capacity(edges.len() * 2);
    for e in edges {
        ys.push(e.y0);
        ys.push(e.y1);
    }
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by(|&a, &b| edges[a].y0.total_cmp(&edges[b].y0));
    for (oi, &i) in order.iter().enumerate() {
        let a = edges[i];
        for &j in &order[oi + 1..] {
            let b = edges[j];
            if b.y0 >= a.y1 {
                break;
            }
            if let Some(y) = crossing_y(&a, &b) {
                ys.push(y);
            }
        }
    }
    sorted_unique(ys)
}

fn covered(spans: &[(usize, usize)], edges: &[Edge], y: f64, x: f64) -> bool {
    spans.iter().any(|&(l, r)| edges[l].x_at(y) < x && x < edges[r].x_at(y))
}

/// Fills subpaths (each closed) with the given rule.
fn fill(subpaths: &[Subpath], rule: Rule) -> Vec<PathVertex> {
    let edges = edges_of(subpaths);
    let mut out = Vec::new();
    if edges.is_empty() {
        return out;
    }
    let ys = breakpoints(&edges);

    let mut by_top: Vec<usize> = (0..edges.len()).collect();
    by_top.sort_by(|&a, &b| edges[a].y0.total_cmp(&edges[b].y0));
    let mut next = 0;
    let mut active: Vec<usize> = Vec::new();
    // Per band: the inside spans as (left edge, right edge) indices into `edges`.
    let mut bands: Vec<Vec<(usize, usize)>> = Vec::with_capacity(ys.len().saturating_sub(1));

    for w in ys.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        while next < by_top.len() && edges[by_top[next]].y0 <= lo + tol(lo) {
            active.push(by_top[next]);
            next += 1;
        }
        active.retain(|&i| edges[i].y1 >= hi - tol(hi));
        active.sort_by(|&a, &b| edges[a].mid_x_between(lo, hi).total_cmp(&edges[b].mid_x_between(lo, hi)));

        let m = active.len();
        let mut inside_after = Vec::with_capacity(m);
        let mut winding: i32 = 0;
        for &i in &active {
            winding += edges[i].dir;
            inside_after.push(match rule {
                Rule::NonZero => winding != 0,
                Rule::EvenOdd => winding.rem_euclid(2) == 1,
            });
        }

        let mut spans = Vec::new();
        for k in 0..m {
            let before = k > 0 && inside_after[k - 1];
            let after = inside_after[k];
            let e = edges[active[k]];
            if before != after {
                // The outside is to the right of the edge when the inside is to its left. The edge runs top to bottom
                // (dy > 0), so (dy, -dx) / len points right; the fringe is that unit normal times FRINGE.
                let s = if before { 1.0 } else { -1.0 };
                let (dx, dy) = (e.x1 - e.x0, e.y1 - e.y0);
                let len = (dx * dx + dy * dy).sqrt();
                let (ox, oy) = (s * FRINGE * dy / len, -s * FRINGE * dx / len);
                let (xl, xh) = (e.x_at(lo), e.x_at(hi));
                push_quad(&mut out, (xl, lo, 1.0), (xh, hi, 1.0), (xh + ox, hi + oy, 0.0), (xl + ox, lo + oy, 0.0));
            }
            if after && k + 1 < m {
                let (l, r) = (active[k], active[k + 1]);
                spans.push((l, r));
                let (el, er) = (edges[l], edges[r]);
                push_quad(
                    &mut out,
                    (el.x_at(lo), lo, 1.0),
                    (er.x_at(lo), lo, 1.0),
                    (er.x_at(hi), hi, 1.0),
                    (el.x_at(hi), hi, 1.0),
                );
            }
        }
        bands.push(spans);
    }

    // Horizontal boundaries: at each breakpoint, compare the inside status just above and just below, piece by piece
    // between consecutive span endpoints.
    for k in 0..ys.len() {
        let y = ys[k];
        let above: &[(usize, usize)] = if k >= 1 { &bands[k - 1] } else { &[] };
        let below: &[(usize, usize)] = if k < bands.len() { &bands[k] } else { &[] };
        let mut xs = Vec::with_capacity(2 * (above.len() + below.len()));
        for &(l, r) in above.iter().chain(below.iter()) {
            xs.push(edges[l].x_at(y));
            xs.push(edges[r].x_at(y));
        }
        let xs = sorted_unique(xs);
        for pair in xs.windows(2) {
            let (xa, xb) = (pair[0], pair[1]);
            let mid = 0.5 * (xa + xb);
            let above_in = covered(above, &edges, y, mid);
            let below_in = covered(below, &edges, y, mid);
            if above_in != below_in {
                let dy = if below_in { -FRINGE } else { FRINGE };
                push_quad(&mut out, (xa, y, 1.0), (xb, y, 1.0), (xb, y + dy, 0.0), (xa, y + dy, 0.0));
            }
        }
    }
    out
}

/// Removes repeated points (and a closing point equal to the first one) from a subpath.
fn dedupe_points(points: &[Pt], closed: bool) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(points.len());
    for &p in points {
        if !(p.0.is_finite() && p.1.is_finite()) {
            continue;
        }
        match out.last() {
            Some(&q) if norm((p.0 - q.0, p.1 - q.1)) <= EPS => {}
            _ => out.push(p),
        }
    }
    if closed && out.len() > 1 {
        let (first, last) = (out[0], out[out.len() - 1]);
        if norm((first.0 - last.0, first.1 - last.1)) <= EPS {
            out.pop();
        }
    }
    out
}

/// Makes a polygon counter-clockwise (positive shoelace area), so that a non-zero fill of several outlines is their
/// union.
fn oriented_ccw(mut pts: Vec<Pt>) -> Vec<Pt> {
    let n = pts.len();
    let area: f64 = (0..n).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        a.0 * b.1 - b.0 * a.1
    })
    .sum();
    if area < 0.0 {
        pts.reverse();
    }
    pts
}

/// Segments for a circle of radius `r` whose inscribed polygon stays within the flattening tolerance.
fn circle_segments(r: f64) -> usize {
    let arg = (1.0 - FLATTEN_TOLERANCE / r).clamp(-1.0, 1.0);
    let step = arg.acos().max(1e-6);
    ((std::f64::consts::PI / step).ceil() as usize).clamp(MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS)
}

fn circle_outline(c: Pt, r: f64) -> Vec<Pt> {
    let n = circle_segments(r);
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

fn rect_outline(p: Pt, q: Pt, hw: f64) -> Vec<Pt> {
    let len = norm((q.0 - p.0, q.1 - p.1));
    let (nx, ny) = (-(q.1 - p.1) / len * hw, (q.0 - p.0) / len * hw);
    vec![(p.0 + nx, p.1 + ny), (q.0 + nx, q.1 + ny), (q.0 - nx, q.1 - ny), (p.0 - nx, p.1 - ny)]
}

/// A join needs a round piece only when the notch it leaves (about hw * theta^2 / 8 deep) is visible.
fn join_needed(prev: Pt, at: Pt, next: Pt, hw: f64) -> bool {
    let d1 = (at.0 - prev.0, at.1 - prev.1);
    let d2 = (next.0 - at.0, next.1 - at.1);
    let (l1, l2) = (norm(d1), norm(d2));
    if l1 == 0.0 || l2 == 0.0 {
        return false;
    }
    let cos = ((d1.0 * d2.0 + d1.1 * d2.1) / (l1 * l2)).clamp(-1.0, 1.0);
    let theta = cos.acos();
    hw * theta * theta >= 0.25
}

/// Expands the subpaths into a stroke of `width` with round joins and caps, then fills the union non-zero.
fn stroke(subpaths: &[Subpath], width: f64) -> Vec<PathVertex> {
    let hw = width * 0.5;
    let mut outlines: Vec<Subpath> = Vec::new();
    let mut push_outline = |pts: Vec<Pt>| {
        outlines.push(Subpath { points: oriented_ccw(pts), closed: true });
    };
    for sp in subpaths {
        let pts = dedupe_points(&sp.points, sp.closed);
        let n = pts.len();
        match n {
            0 => continue,
            1 => {
                // A single point (zero-length subpath) gets a dot of the stroke width.
                push_outline(circle_outline(pts[0], hw));
                continue;
            }
            _ => {}
        }
        let segments = if sp.closed { n } else { n - 1 };
        for i in 0..segments {
            push_outline(rect_outline(pts[i], pts[(i + 1) % n], hw));
        }
        let joins: Vec<usize> = if sp.closed { (0..n).collect() } else { (1..n - 1).collect() };
        for i in joins {
            let prev = pts[(i + n - 1) % n];
            let next = pts[(i + 1) % n];
            if join_needed(prev, pts[i], next, hw) {
                push_outline(circle_outline(pts[i], hw));
            }
        }
        if !sp.closed {
            push_outline(circle_outline(pts[0], hw));
            push_outline(circle_outline(pts[n - 1], hw));
        }
    }
    fill(&outlines, Rule::NonZero)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::canvas::PathVerb::{Close, Cubic, Line, Move, Quad};

    fn square(x0: f32, y0: f32, s: f32) -> Vec<PathVerb> {
        vec![Move(x0, y0), Line(x0 + s, y0), Line(x0 + s, y0 + s), Line(x0, y0 + s), Close]
    }

    fn path(verbs: Vec<PathVerb>) -> PathData {
        PathData { verbs }
    }

    fn tri_area(t: &[PathVertex]) -> f64 {
        let (a, b, c) = (t[0], t[1], t[2]);
        let cross = (b.x as f64 - a.x as f64) * (c.y as f64 - a.y as f64) - (c.x as f64 - a.x as f64) * (b.y as f64 - a.y as f64);
        cross.abs() * 0.5
    }

    /// Area of the triangles whose three vertices are fully covered: the interior.
    fn core_area(v: &[PathVertex]) -> f64 {
        v.chunks(3).filter(|t| t.iter().all(|p| p.coverage == 1.0)).map(tri_area).sum()
    }

    /// Area of the triangles with any uncovered vertex: the fringe.
    fn fringe_area(v: &[PathVertex]) -> f64 {
        v.chunks(3).filter(|t| t.iter().any(|p| p.coverage < 1.0)).map(tri_area).sum()
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "got {a}, want {b} (tolerance {tol})");
    }

    #[test]
    fn flatten_makes_one_closed_subpath_for_a_square() {
        let subs = flatten(&path(square(0.0, 0.0, 10.0)));
        assert_eq!(subs.len(), 1);
        assert!(subs[0].closed);
        assert_eq!(subs[0].points.len(), 4);
    }

    #[test]
    fn flatten_keeps_open_subpaths_open_and_splits_on_move() {
        let subs = flatten(&path(vec![Move(0.0, 0.0), Line(5.0, 0.0), Move(1.0, 1.0), Line(2.0, 2.0)]));
        assert_eq!(subs.len(), 2);
        assert!(!subs[0].closed && !subs[1].closed);
        assert_eq!(subs[1].points[0], (1.0, 1.0));
    }

    #[test]
    fn quad_segment_count_follows_the_chord_bound() {
        // A = (0,0) - 2(8,8) + (16,0) = (0,-16), |A| = 16, n = ceil(sqrt(16 / (4 * 0.25))) = 4.
        let subs = flatten(&path(vec![Move(0.0, 0.0), Quad(8.0, 8.0, 16.0, 0.0)]));
        assert_eq!(subs[0].points.len(), 1 + 4);
        assert_eq!(*subs[0].points.last().unwrap(), (16.0, 0.0));
    }

    #[test]
    fn cubic_segment_count_follows_the_chord_bound() {
        // A = (8,-8), B = (-8,-8), M = 11.3137, n = ceil(sqrt(3 M / (4 * 0.25))) = ceil(5.83) = 6.
        let subs = flatten(&path(vec![Move(0.0, 0.0), Cubic(0.0, 8.0, 8.0, 8.0, 8.0, 0.0)]));
        assert_eq!(subs[0].points.len(), 1 + 6);
        assert_eq!(*subs[0].points.last().unwrap(), (8.0, 0.0));
    }

    #[test]
    fn a_straight_cubic_is_one_segment() {
        let subs = flatten(&path(vec![Move(0.0, 0.0), Cubic(3.0, 0.0, 6.0, 0.0, 9.0, 0.0)]));
        assert_eq!(subs[0].points.len(), 2);
    }

    #[test]
    fn square_fill_has_area_100_and_a_one_pixel_fringe() {
        let p = path(square(0.0, 0.0, 10.0));
        for paint in [PathPaint::FillNonZero, PathPaint::FillEvenOdd] {
            let v = tessellate(&p, paint);
            close(core_area(&v), 100.0, 1e-6);
            // Perimeter 40 times the 1 px fringe.
            close(fringe_area(&v), 40.0, 1e-6);
            assert!(v.iter().all(|p| p.coverage == 0.0 || p.coverage == 1.0));
        }
    }

    #[test]
    fn slanted_fringe_lies_one_pixel_outside_the_edge() {
        // Hypotenuse x + y = 10 of the triangle (0,0),(10,0),(0,10): the outside normal is (1,1)/sqrt(2), so the
        // fringe vertices at the hypotenuse ends are (10,0) + (1,1)/sqrt(2) and (0,10) + (1,1)/sqrt(2).
        let p = path(vec![Move(0.0, 0.0), Line(10.0, 0.0), Line(0.0, 10.0), Close]);
        let v = tessellate(&p, PathPaint::FillNonZero);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        for want in [(10.0 + s, s), (s, 10.0 + s)] {
            let found = v.iter().any(|p| p.coverage == 0.0 && (p.x - want.0).abs() < 1e-3 && (p.y - want.1).abs() < 1e-3);
            assert!(found, "no fringe vertex at {want:?}");
        }
    }

    #[test]
    fn triangle_area_is_the_same_in_either_winding() {
        let ccw = path(vec![Move(0.0, 0.0), Line(10.0, 0.0), Line(0.0, 10.0), Close]);
        let cw = path(vec![Move(0.0, 0.0), Line(0.0, 10.0), Line(10.0, 0.0), Close]);
        for p in [&ccw, &cw] {
            close(core_area(&tessellate(p, PathPaint::FillNonZero)), 50.0, 1e-6);
            close(core_area(&tessellate(p, PathPaint::FillEvenOdd)), 50.0, 1e-6);
        }
    }

    #[test]
    fn overlapping_squares_union_nonzero_and_xor_evenodd() {
        // [0,10]^2 and [5,15]^2: union 175, overlap 25, XOR 150.
        let mut verbs = square(0.0, 0.0, 10.0);
        verbs.extend(square(5.0, 5.0, 10.0));
        let p = path(verbs);
        close(core_area(&tessellate(&p, PathPaint::FillNonZero)), 175.0, 1e-6);
        close(core_area(&tessellate(&p, PathPaint::FillEvenOdd)), 150.0, 1e-6);
    }

    #[test]
    fn same_direction_hole_is_filled_by_nonzero_and_cut_by_evenodd() {
        let mut verbs = square(0.0, 0.0, 10.0);
        verbs.extend(square(2.0, 2.0, 6.0));
        let p = path(verbs);
        close(core_area(&tessellate(&p, PathPaint::FillNonZero)), 100.0, 1e-6);
        close(core_area(&tessellate(&p, PathPaint::FillEvenOdd)), 64.0, 1e-6);
    }

    #[test]
    fn reversed_hole_is_cut_by_both_rules() {
        // Outer CCW square, inner square with the opposite winding.
        let mut verbs = square(0.0, 0.0, 10.0);
        verbs.extend(vec![Move(2.0, 2.0), Line(2.0, 8.0), Line(8.0, 8.0), Line(8.0, 2.0), Close]);
        let p = path(verbs);
        close(core_area(&tessellate(&p, PathPaint::FillNonZero)), 64.0, 1e-6);
        close(core_area(&tessellate(&p, PathPaint::FillEvenOdd)), 64.0, 1e-6);
    }

    #[test]
    fn pentagram_difference_between_rules_is_the_inner_pentagon() {
        // Tips of a {5/2} star at radius R; the inner vertices are at R / phi^2 on the bisecting angles, and they
        // bound the centre pentagon that non-zero fills and even-odd leaves empty.
        let r_outer = 10.0f64;
        let tip = |k: usize| {
            let a = std::f64::consts::PI / 180.0 * (90.0 + 144.0 * k as f64);
            (r_outer * a.cos(), r_outer * a.sin())
        };
        let mut verbs = Vec::new();
        for k in 0..5 {
            let p = tip(k);
            verbs.push(if k == 0 { Move(p.0 as f32, p.1 as f32) } else { Line(p.0 as f32, p.1 as f32) });
        }
        verbs.push(Close);
        let p = path(verbs);
        let nz = core_area(&tessellate(&p, PathPaint::FillNonZero));
        let eo = core_area(&tessellate(&p, PathPaint::FillEvenOdd));
        let phi2 = ((1.0 + 5f64.sqrt()) / 2.0).powi(2);
        let r_inner = r_outer / phi2;
        let pentagon = 2.5 * r_inner * r_inner * (72f64.to_radians()).sin();
        close(nz - eo, pentagon, 0.05);
        assert!(eo < nz);
    }

    #[test]
    fn circle_from_cubics_has_the_circle_area() {
        // Four cubics with the usual kappa; radius 50, area pi r^2 = 7853.98.
        let r = 50.0f32;
        let k = 0.552_284_8f32 * r;
        let verbs = vec![
            Move(r, 0.0),
            Cubic(r, k, k, r, 0.0, r),
            Cubic(-k, r, -r, k, -r, 0.0),
            Cubic(-r, -k, -k, -r, 0.0, -r),
            Cubic(k, -r, r, -k, r, 0.0),
            Close,
        ];
        let v = tessellate(&path(verbs), PathPaint::FillNonZero);
        // The chord error is at most 0.25 px, which costs about 0.3% of the area here.
        let want = std::f64::consts::PI * 50.0 * 50.0;
        close(core_area(&v), want, want * 0.01);
    }

    #[test]
    fn stroke_of_a_segment_with_round_caps() {
        // Body 10 x 4, two caps of radius 2 (24-gon). Expected: 40 + area of the 24-gon disc.
        let p = path(vec![Move(0.0, 0.0), Line(10.0, 0.0)]);
        let v = tessellate(&p, PathPaint::Stroke(4.0));
        let disc = 12.0 * 4.0 * (2.0 * std::f64::consts::PI / 24.0).sin();
        close(core_area(&v), 40.0 + disc, 0.05);
        // The cap is round: the outline reaches x = -2 and x = 12, and the 1 px fringe takes the vertices to -3 and 13.
        let min_x = v.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let max_x = v.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        close(min_x as f64, -3.0, 0.02);
        close(max_x as f64, 13.0, 0.02);
    }

    #[test]
    fn stroke_round_join_adds_the_outer_quarter_disc() {
        // (0,0)-(10,0)-(10,10), width 2: rects 20 + 20 - overlap 1 = 39, joins and caps:
        // quarter disc of the outer corner (pi/4), half discs at the two ends (pi/2 each).
        let p = path(vec![Move(0.0, 0.0), Line(10.0, 0.0), Line(10.0, 10.0)]);
        let v = tessellate(&p, PathPaint::Stroke(2.0));
        close(core_area(&v), 39.0 + std::f64::consts::PI * 1.25, 0.1);
    }

    #[test]
    fn closed_stroke_is_a_ring_with_rounded_outer_corners() {
        // Ring around [0,10]^2 with width 2: outer 10x10 + 4 * 10 * 1 + pi * 1^2 = 143.14 minus the hole 8x8 = 64.
        let p = path(square(0.0, 0.0, 10.0));
        let v = tessellate(&p, PathPaint::Stroke(2.0));
        close(core_area(&v), 76.0 + std::f64::consts::PI, 0.1);
    }

    #[test]
    fn zero_length_stroke_is_a_dot_and_zero_width_is_nothing() {
        let dot = path(vec![Move(5.0, 5.0), Line(5.0, 5.0)]);
        let v = tessellate(&dot, PathPaint::Stroke(4.0));
        close(core_area(&v), 12.0 * 4.0 * (2.0 * std::f64::consts::PI / 24.0).sin(), 0.05);
        assert!(tessellate(&dot, PathPaint::Stroke(0.0)).is_empty());
        assert!(tessellate(&path(square(0.0, 0.0, 10.0)), PathPaint::Stroke(-1.0)).is_empty());
    }

    #[test]
    fn empty_and_zero_area_paths_produce_no_interior() {
        assert!(tessellate(&path(vec![]), PathPaint::FillNonZero).is_empty());
        // A horizontal line has no non-horizontal edges at all.
        let line = path(vec![Move(0.0, 3.0), Line(10.0, 3.0), Close]);
        assert!(tessellate(&line, PathPaint::FillNonZero).is_empty());
        // A vertical line has zero width: no interior area.
        let vline = path(vec![Move(0.0, 0.0), Line(0.0, 10.0), Close]);
        close(core_area(&tessellate(&vline, PathPaint::FillNonZero)), 0.0, 1e-9);
    }

    #[test]
    fn all_vertices_are_finite_and_coverage_is_in_range() {
        let p = path(vec![Move(0.0, 0.0), Quad(50.0, -20.0, 100.0, 0.0), Cubic(100.0, 40.0, 0.0, 40.0, 0.0, 0.0), Close]);
        for paint in [PathPaint::FillNonZero, PathPaint::FillEvenOdd, PathPaint::Stroke(3.0)] {
            for v in tessellate(&p, paint) {
                assert!(v.x.is_finite() && v.y.is_finite());
                assert!((0.0..=1.0).contains(&v.coverage));
            }
        }
    }
}
