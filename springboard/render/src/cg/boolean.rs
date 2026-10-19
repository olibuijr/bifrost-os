//! Boolean operations on paths: union, intersection, subtraction, symmetric difference, normalising a self-crossing
//! path, and clipping open lines to a filled region.
//!
//! Curves are flattened first. Every vertex is snapped to a 1/1024 unit grid, so the edges of both operands can be
//! cut at each crossing with exact integer arithmetic and the cut points are shared by name between the edges that
//! meet there. Each piece between two cut points lies entirely inside or outside each operand; a probe just left and
//! just right of the piece (winding number under the operand's fill rule) says whether the result lies on one side
//! only, and only such pieces are boundary of the result. The kept pieces are linked head to tail into closed loops
//! (interior on the left of every piece, so non-zero and even-odd fills of the result agree).
use super::path::{Path, Point};
use super::raster::FillRule;
use std::collections::{HashMap, HashSet};

/// Which area the result keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    /// Inside either operand.
    Union,
    /// Inside both.
    Intersection,
    /// Inside the first and not the second.
    Subtract,
    /// Inside exactly one.
    SymmetricDifference,
}

impl BooleanOp {
    fn keeps(self, in_a: bool, in_b: bool) -> bool {
        match self {
            BooleanOp::Union => in_a || in_b,
            BooleanOp::Intersection => in_a && in_b,
            BooleanOp::Subtract => in_a && !in_b,
            BooleanOp::SymmetricDifference => in_a != in_b,
        }
    }
}

/// Grid points per unit.
const GRID: f64 = 1024.0;
/// Flattening tolerance for curves, in path units.
const TOLERANCE: f32 = 0.02;
/// How far from a piece the probes sit, in grid units.
const PROBE: f64 = 0.01;

/// A snapped vertex in grid units.
type G = (i64, i64);

fn snap(p: Point) -> G {
    ((p.0 as f64 * GRID).round() as i64, (p.1 as f64 * GRID).round() as i64)
}

fn unsnap(g: G) -> Point {
    ((g.0 as f64 / GRID) as f32, (g.1 as f64 / GRID) as f32)
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    a: G,
    b: G,
    /// 0 for the first operand, 1 for the second
    src: u8,
}

fn cross(o: G, a: G, b: G) -> i128 {
    (a.0 - o.0) as i128 * (b.1 - o.1) as i128 - (a.1 - o.1) as i128 * (b.0 - o.0) as i128
}

/// The edges of `path`: every subpath is closed when `close_all`, else only the ones the path closes.
fn polygon_edges(path: &Path, src: u8, close_all: bool) -> Vec<Edge> {
    let mut out = Vec::new();
    for s in path.flatten_subpaths(TOLERANCE) {
        let mut pts: Vec<G> = Vec::new();
        for p in &s.points {
            let g = snap(*p);
            if pts.last() != Some(&g) {
                pts.push(g);
            }
        }
        let n = pts.len();
        if n < 2 {
            continue;
        }
        for i in 0..n - 1 {
            out.push(Edge { a: pts[i], b: pts[i + 1], src });
        }
        if (close_all || s.closed) && n > 2 && pts[n - 1] != pts[0] {
            out.push(Edge { a: pts[n - 1], b: pts[0], src });
        }
    }
    out
}

fn on_segment(e: &Edge, p: G) -> bool {
    p.0 >= e.a.0.min(e.b.0) && p.0 <= e.a.0.max(e.b.0) && p.1 >= e.a.1.min(e.b.1) && p.1 <= e.a.1.max(e.b.1)
}

/// For every edge, the points where other edges cross or touch it (and the ends of collinear overlaps).
fn split_points(edges: &[Edge]) -> Vec<Vec<G>> {
    let n = edges.len();
    let mut splits: Vec<Vec<G>> = vec![Vec::new(); n];
    for i in 0..n {
        let p = edges[i];
        for j in i + 1..n {
            let q = edges[j];
            if p.a.0.max(p.b.0) < q.a.0.min(q.b.0)
                || q.a.0.max(q.b.0) < p.a.0.min(p.b.0)
                || p.a.1.max(p.b.1) < q.a.1.min(q.b.1)
                || q.a.1.max(q.b.1) < p.a.1.min(p.b.1)
            {
                continue;
            }
            let r = ((p.b.0 - p.a.0) as i128, (p.b.1 - p.a.1) as i128);
            let s = ((q.b.0 - q.a.0) as i128, (q.b.1 - q.a.1) as i128);
            let w = ((q.a.0 - p.a.0) as i128, (q.a.1 - p.a.1) as i128);
            let d = r.0 * s.1 - r.1 * s.0;
            if d != 0 {
                let tn = w.0 * s.1 - w.1 * s.0;
                let un = w.0 * r.1 - w.1 * r.0;
                let within = |num: i128| if d > 0 { num >= 0 && num <= d } else { num <= 0 && num >= d };
                if within(tn) && within(un) {
                    let t = tn as f64 / d as f64;
                    let pt = (
                        (p.a.0 as f64 + r.0 as f64 * t).round() as i64,
                        (p.a.1 as f64 + r.1 as f64 * t).round() as i64,
                    );
                    splits[i].push(pt);
                    splits[j].push(pt);
                }
            } else if cross(p.a, p.b, q.a) == 0 {
                for pt in [q.a, q.b] {
                    if on_segment(&p, pt) {
                        splits[i].push(pt);
                    }
                }
                for pt in [p.a, p.b] {
                    if on_segment(&q, pt) {
                        splits[j].push(pt);
                    }
                }
            }
        }
    }
    splits
}

/// The edges cut at their split points, in the order of the original edges and along each of them.
fn atomize(edges: &[Edge], splits: Vec<Vec<G>>) -> Vec<Edge> {
    let mut out = Vec::new();
    for (e, mut pts) in edges.iter().zip(splits) {
        pts.push(e.a);
        pts.push(e.b);
        let r = (e.b.0 - e.a.0, e.b.1 - e.a.1);
        pts.sort_by_key(|p| ((p.0 - e.a.0) as i128 * r.0 as i128 + (p.1 - e.a.1) as i128 * r.1 as i128, p.0, p.1));
        pts.dedup();
        for w in pts.windows(2) {
            out.push(Edge { a: w[0], b: w[1], src: e.src });
        }
    }
    out
}

/// The winding number of the polygon edges of operand `src` around `p` (a ray towards +x).
fn winding(edges: &[Edge], src: u8, p: (f64, f64)) -> i32 {
    let mut w = 0;
    for e in edges.iter().filter(|e| e.src == src) {
        let (ax, ay) = (e.a.0 as f64, e.a.1 as f64);
        let (bx, by) = (e.b.0 as f64, e.b.1 as f64);
        let side = (bx - ax) * (p.1 - ay) - (p.0 - ax) * (by - ay);
        if ay <= p.1 {
            if by > p.1 && side > 0.0 {
                w += 1;
            }
        } else if by <= p.1 && side < 0.0 {
            w -= 1;
        }
    }
    w
}

fn inside(w: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w & 1 != 0,
    }
}

/// Removes vertices that lie on the straight line between their neighbours (and spikes).
fn drop_collinear(mut v: Vec<G>) -> Vec<G> {
    loop {
        let mut removed = false;
        let mut i = 0;
        while i < v.len() {
            let n = v.len();
            if n < 3 {
                return v;
            }
            if cross(v[(i + n - 1) % n], v[i], v[(i + 1) % n]) == 0 {
                v.remove(i);
                removed = true;
            } else {
                i += 1;
            }
        }
        if !removed {
            return v;
        }
    }
}

/// Links directed pieces head to tail into closed loops.
fn build_loops(kept: &[(G, G)]) -> Vec<Vec<G>> {
    let mut outgoing: HashMap<G, Vec<usize>> = HashMap::new();
    for (i, e) in kept.iter().enumerate() {
        outgoing.entry(e.0).or_default().push(i);
    }
    let mut used = vec![false; kept.len()];
    let mut loops = Vec::new();
    for s in 0..kept.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let start = kept[s].0;
        let mut pts = vec![start];
        let mut cur = kept[s].1;
        let mut closed = true;
        while cur != start {
            pts.push(cur);
            let next = outgoing.get(&cur).and_then(|v| v.iter().copied().find(|&i| !used[i]));
            match next {
                Some(i) => {
                    used[i] = true;
                    cur = kept[i].1;
                }
                None => {
                    closed = false;
                    break;
                }
            }
        }
        if closed {
            let pts = drop_collinear(pts);
            if pts.len() >= 3 {
                loops.push(pts);
            }
        }
    }
    loops
}

/// Combines the filled areas of `a` (under `rule_a`) and `b` (under `rule_b`) with `op`. The result is made of
/// closed polygon outlines (curves are flattened); holes wind against the outer outlines.
pub fn combine(a: &Path, b: &Path, op: BooleanOp, rule_a: FillRule, rule_b: FillRule) -> Path {
    let mut edges = polygon_edges(a, 0, true);
    edges.extend(polygon_edges(b, 1, true));
    let atoms = atomize(&edges, split_points(&edges));
    let mut seen: HashSet<(G, G)> = HashSet::new();
    let mut kept: Vec<(G, G)> = Vec::new();
    for e in &atoms {
        if e.a == e.b {
            continue;
        }
        let key = if e.a <= e.b { (e.a, e.b) } else { (e.b, e.a) };
        if !seen.insert(key) {
            continue;
        }
        let (dx, dy) = ((e.b.0 - e.a.0) as f64, (e.b.1 - e.a.1) as f64);
        let len = dx.hypot(dy);
        let (nx, ny) = (-dy / len, dx / len);
        let mid = ((e.a.0 + e.b.0) as f64 / 2.0, (e.a.1 + e.b.1) as f64 / 2.0);
        // the probes measure against the cut pieces, not the original edges: a cut point is rounded to the grid, so a
        // piece can leave its original edge by half a grid unit, far more than the probe distance, and the probes
        // would then both land on one side of the piece
        let keeps = |pt: (f64, f64)| {
            op.keeps(inside(winding(&atoms, 0, pt), rule_a), inside(winding(&atoms, 1, pt), rule_b))
        };
        let left = keeps((mid.0 + nx * PROBE, mid.1 + ny * PROBE));
        let right = keeps((mid.0 - nx * PROBE, mid.1 - ny * PROBE));
        match (left, right) {
            (true, false) => kept.push((e.a, e.b)),
            (false, true) => kept.push((e.b, e.a)),
            _ => {}
        }
    }
    let mut out = Path::new();
    for l in build_loops(&kept) {
        out.move_to(unsnap(l[0]));
        for p in &l[1..] {
            out.line_to(unsnap(*p));
        }
        out.close();
    }
    out
}

/// The parts of the open lines of `lines` that lie inside `region` (`keep_inside`) or outside it, as open polylines.
pub fn clip_lines(lines: &Path, region: &Path, rule: FillRule, keep_inside: bool) -> Path {
    let mut edges = polygon_edges(lines, 0, false);
    edges.extend(polygon_edges(region, 1, true));
    let atoms = atomize(&edges, split_points(&edges));
    let mut out = Path::new();
    let mut pen: Option<G> = None;
    for e in atoms.iter().filter(|e| e.src == 0 && e.a != e.b) {
        let mid = ((e.a.0 + e.b.0) as f64 / 2.0, (e.a.1 + e.b.1) as f64 / 2.0);
        if inside(winding(&atoms, 1, mid), rule) == keep_inside {
            if pen != Some(e.a) {
                out.move_to(unsnap(e.a));
            }
            out.line_to(unsnap(e.b));
            pen = Some(e.b);
        } else {
            pen = None;
        }
    }
    out
}

impl Path {
    /// The area covered by this path or `other` (`CGPath.union(_:using:)`).
    pub fn union(&self, other: &Path, rule: FillRule) -> Path {
        combine(self, other, BooleanOp::Union, rule, rule)
    }

    /// The area covered by both (`CGPath.intersection(_:using:)`).
    pub fn intersection(&self, other: &Path, rule: FillRule) -> Path {
        combine(self, other, BooleanOp::Intersection, rule, rule)
    }

    /// The area of this path that `other` does not cover (`CGPath.subtracting(_:using:)`).
    pub fn subtracting(&self, other: &Path, rule: FillRule) -> Path {
        combine(self, other, BooleanOp::Subtract, rule, rule)
    }

    /// The area covered by exactly one of the two (`CGPath.symmetricDifference(_:using:)`).
    pub fn symmetric_difference(&self, other: &Path, rule: FillRule) -> Path {
        combine(self, other, BooleanOp::SymmetricDifference, rule, rule)
    }

    /// The parts of this path's open lines inside the filled area of `other` (`CGPath.lineIntersection(_:using:)`).
    pub fn line_intersection(&self, other: &Path, rule: FillRule) -> Path {
        clip_lines(self, other, rule, true)
    }

    /// The parts of this path's open lines outside the filled area of `other` (`CGPath.lineSubtracting(_:using:)`).
    pub fn line_subtracting(&self, other: &Path, rule: FillRule) -> Path {
        clip_lines(self, other, rule, false)
    }

    /// This path with self-crossings and overlaps resolved into plain outlines (`CGPath.normalized(using:)`): the
    /// same filled area under `rule`, but every outline is a simple loop.
    pub fn normalized(&self, rule: FillRule) -> Path {
        combine(self, &Path::new(), BooleanOp::Union, rule, FillRule::NonZero)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::path::Rect;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Path {
        Path::from_rect(Rect::new(x, y, w, h))
    }

    /// The filled area: outer loops add, holes (opposite winding) subtract.
    fn area(p: &Path) -> f32 {
        let mut total = 0.0;
        for s in p.flatten_subpaths(0.05) {
            let n = s.points.len();
            total += (0..n).map(|i| s.points[i].0 * s.points[(i + 1) % n].1 - s.points[(i + 1) % n].0 * s.points[i].1).sum::<f32>() / 2.0;
        }
        total.abs()
    }

    fn loops(p: &Path) -> usize {
        p.flatten_subpaths(0.05).len()
    }

    const NZ: FillRule = FillRule::NonZero;

    #[test]
    fn overlapping_squares_union_to_one_outline() {
        let u = rect(0.0, 0.0, 10.0, 10.0).union(&rect(5.0, 5.0, 10.0, 10.0), NZ);
        assert_eq!(loops(&u), 1);
        assert!((area(&u) - 175.0).abs() < 0.01, "{}", area(&u));
        assert!(u.contains((2.0, 2.0), NZ) && u.contains((12.0, 12.0), NZ));
        assert!(!u.contains((12.0, 2.0), NZ));
    }

    #[test]
    fn intersection_is_the_overlap() {
        let i = rect(0.0, 0.0, 10.0, 10.0).intersection(&rect(5.0, 5.0, 10.0, 10.0), NZ);
        assert_eq!(loops(&i), 1);
        assert!((area(&i) - 25.0).abs() < 0.01);
        assert!(i.contains((7.0, 7.0), NZ) && !i.contains((2.0, 2.0), NZ));
        let none = rect(0.0, 0.0, 2.0, 2.0).intersection(&rect(5.0, 5.0, 2.0, 2.0), NZ);
        assert!(none.is_empty());
    }

    #[test]
    fn subtracting_cuts_a_notch_and_a_hole() {
        let notch = rect(0.0, 0.0, 10.0, 10.0).subtracting(&rect(5.0, 5.0, 10.0, 10.0), NZ);
        assert!((area(&notch) - 75.0).abs() < 0.01);
        assert!(!notch.contains((7.0, 7.0), NZ) && notch.contains((2.0, 7.0), NZ));
        let ring = rect(0.0, 0.0, 10.0, 10.0).subtracting(&rect(3.0, 3.0, 4.0, 4.0), NZ);
        assert_eq!(loops(&ring), 2);
        assert!((area(&ring) - 84.0).abs() < 0.01, "{}", area(&ring));
        assert!(!ring.contains((5.0, 5.0), NZ) && ring.contains((1.0, 5.0), NZ));
        assert!(!ring.contains((5.0, 5.0), FillRule::EvenOdd) && ring.contains((1.0, 5.0), FillRule::EvenOdd));
    }

    #[test]
    fn symmetric_difference_drops_the_overlap() {
        let x = rect(0.0, 0.0, 10.0, 10.0).symmetric_difference(&rect(5.0, 5.0, 10.0, 10.0), NZ);
        assert!((area(&x) - 150.0).abs() < 0.01, "{}", area(&x));
        assert!(!x.contains((7.0, 7.0), FillRule::EvenOdd));
        assert!(x.contains((2.0, 2.0), FillRule::EvenOdd) && x.contains((12.0, 12.0), FillRule::EvenOdd));
    }

    #[test]
    fn disjoint_union_keeps_both_loops() {
        let u = rect(0.0, 0.0, 2.0, 2.0).union(&rect(5.0, 5.0, 2.0, 2.0), NZ);
        assert_eq!(loops(&u), 2);
        assert!((area(&u) - 8.0).abs() < 0.01);
    }

    #[test]
    fn squares_sharing_an_edge_merge_without_a_seam() {
        let u = rect(0.0, 0.0, 10.0, 10.0).union(&rect(10.0, 0.0, 10.0, 10.0), NZ);
        assert_eq!(loops(&u), 1);
        assert_eq!(u.flatten_subpaths(0.05)[0].points.len(), 4, "the seam vertices are gone");
        assert!((area(&u) - 200.0).abs() < 0.01);
    }

    #[test]
    fn a_contained_shape_changes_nothing_in_a_union_and_vanishes_in_an_intersection() {
        let big = rect(0.0, 0.0, 10.0, 10.0);
        let small = rect(2.0, 2.0, 3.0, 3.0);
        assert!((area(&big.union(&small, NZ)) - 100.0).abs() < 0.01);
        assert_eq!(loops(&big.union(&small, NZ)), 1);
        assert!((area(&big.intersection(&small, NZ)) - 9.0).abs() < 0.01);
        assert!(small.subtracting(&big, NZ).is_empty());
    }

    #[test]
    fn circles_combine_through_flattened_curves() {
        let a = Path::from_ellipse(Rect::new(0.0, 0.0, 20.0, 20.0));
        let b = Path::from_ellipse(Rect::new(10.0, 0.0, 20.0, 20.0));
        let u = a.union(&b, NZ);
        let i = a.intersection(&b, NZ);
        let circle = std::f32::consts::PI * 100.0;
        assert!(area(&u) > circle && area(&u) < 2.0 * circle);
        assert!((area(&u) + area(&i) - 2.0 * circle).abs() < 6.0, "inclusion-exclusion: {} {}", area(&u), area(&i));
        assert!(i.contains((15.0, 10.0), NZ) && !i.contains((2.0, 10.0), NZ));
    }

    #[test]
    fn normalizing_resolves_a_self_crossing_bow_tie() {
        let mut bow = Path::new();
        bow.add_lines(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
        bow.close();
        let n = bow.normalized(FillRule::EvenOdd);
        // two triangles that meet at the crossing (5, 5), one on the left and one on the right
        assert!(n.contains((1.0, 5.0), NZ) && n.contains((9.0, 5.0), NZ));
        assert!(!n.contains((5.0, 1.0), NZ) && !n.contains((5.0, 9.0), NZ));
        // the outline no longer crosses itself: every vertex of the result is a corner of a triangle
        let corners = [(0.0, 0.0), (0.0, 10.0), (10.0, 0.0), (10.0, 10.0), (5.0, 5.0)];
        for s in n.flatten_subpaths(0.05) {
            assert!(s.points.iter().all(|p| corners.contains(p)), "{:?}", s.points);
        }
    }

    #[test]
    fn lines_clip_to_a_region() {
        let mut line = Path::new();
        line.add_lines(&[(-5.0, 5.0), (15.0, 5.0)]);
        let region = rect(0.0, 0.0, 10.0, 10.0);
        let inside = line.line_intersection(&region, NZ);
        let pts = inside.flatten(0.05);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0], vec![(0.0, 5.0), (10.0, 5.0)]);
        let outside = line.line_subtracting(&region, NZ);
        let pieces = outside.flatten(0.05);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0], vec![(-5.0, 5.0), (0.0, 5.0)]);
        assert_eq!(pieces[1], vec![(10.0, 5.0), (15.0, 5.0)]);
    }

    #[test]
    fn empty_operands_are_handled() {
        let r = rect(0.0, 0.0, 4.0, 4.0);
        assert!((area(&r.union(&Path::new(), NZ)) - 16.0).abs() < 0.01);
        assert!(r.intersection(&Path::new(), NZ).is_empty());
        assert!((area(&r.subtracting(&Path::new(), NZ)) - 16.0).abs() < 0.01);
        assert!(Path::new().subtracting(&r, NZ).is_empty());
    }
}
