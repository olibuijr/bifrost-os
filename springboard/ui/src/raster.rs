//! Scan conversion shared by the shell, the SDK and apps: polygons to an 8-bit coverage mask (non-zero or even-odd), a
//! stroker that turns polylines into polygons (caps, joins, miter limit), and nothing that touches a pixel buffer.
//! Everything works in device pixels with the origin at the top left.
use crate::stroke_style::{effective_join, LineCap, LineJoin, StrokeStyle};

/// Sub-scanlines per pixel row. Horizontal coverage is exact, vertical coverage is sampled this many times.
const SUB: usize = 4;

pub type Pt = (f32, f32);

/// How overlapping or nested outlines decide what lies inside a fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    /// A point is inside when the outline winds around it at least once, in either direction.
    #[default]
    NonZero,
    /// A point is inside when a ray from it crosses the outline an odd number of times.
    EvenOdd,
}

/// Per-pixel coverage, 0 to 255.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Coverage {
    pub fn new(w: usize, h: usize) -> Self {
        Coverage { w, h, a: vec![0; w * h] }
    }
    pub fn at(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h {
            self.a[y * self.w + x]
        } else {
            0
        }
    }
}

struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dir: i32,
}

/// Fills closed polygons (the last point joins back to the first) into a `w` x `h` coverage mask.
pub fn fill_polygons(polys: &[Vec<Pt>], rule: FillRule, w: usize, h: usize) -> Coverage {
    let mut cov = Coverage::new(w, h);
    if w == 0 || h == 0 {
        return cov;
    }
    let mut edges: Vec<Edge> = Vec::new();
    for poly in polys {
        let n = poly.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) || a.1 == b.1 {
                continue;
            }
            if a.1 < b.1 {
                edges.push(Edge { x0: a.0, y0: a.1, x1: b.0, y1: b.1, dir: 1 });
            } else {
                edges.push(Edge { x0: b.0, y0: b.1, x1: a.0, y1: a.1, dir: -1 });
            }
        }
    }
    if edges.is_empty() {
        return cov;
    }
    // Edges that touch each pixel row.
    let mut rows: Vec<Vec<u32>> = vec![Vec::new(); h];
    for (i, e) in edges.iter().enumerate() {
        let r0 = e.y0.floor().max(0.0) as usize;
        let r1 = (e.y1.ceil().min(h as f32).max(0.0)) as usize;
        for row in rows.iter_mut().take(r1).skip(r0) {
            row.push(i as u32);
        }
    }
    let weight = 1.0 / SUB as f32;
    let mut acc = vec![0.0f32; w];
    let mut xs: Vec<(f32, i32)> = Vec::new();
    for y in 0..h {
        if rows[y].is_empty() {
            continue;
        }
        acc.iter_mut().for_each(|v| *v = 0.0);
        for sub in 0..SUB {
            let sy = y as f32 + (sub as f32 + 0.5) / SUB as f32;
            xs.clear();
            for &ei in &rows[y] {
                let e = &edges[ei as usize];
                if sy >= e.y0 && sy < e.y1 {
                    xs.push((e.x0 + (sy - e.y0) * (e.x1 - e.x0) / (e.y1 - e.y0), e.dir));
                }
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut wind = 0i32;
            let mut start = 0.0f32;
            for &(x, d) in &xs {
                let was_in = inside(wind, rule);
                wind += d;
                let now_in = inside(wind, rule);
                if !was_in && now_in {
                    start = x;
                } else if was_in && !now_in {
                    add_span(&mut acc, start, x, weight);
                }
            }
        }
        let out = &mut cov.a[y * w..(y + 1) * w];
        for (o, v) in out.iter_mut().zip(&acc) {
            *o = (v.min(1.0) * 255.0 + 0.5) as u8;
        }
    }
    cov
}

fn inside(wind: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => wind != 0,
        FillRule::EvenOdd => wind & 1 != 0,
    }
}

fn add_span(acc: &mut [f32], xa: f32, xb: f32, weight: f32) {
    let w = acc.len() as f32;
    let (xa, xb) = (xa.max(0.0), xb.min(w));
    if xb <= xa {
        return;
    }
    let (ia, ib) = (xa.floor() as usize, xb.floor() as usize);
    if ia == ib {
        acc[ia] += (xb - xa) * weight;
        return;
    }
    acc[ia] += (ia as f32 + 1.0 - xa) * weight;
    for v in acc.iter_mut().take(ib).skip(ia + 1) {
        *v += weight;
    }
    if ib < acc.len() {
        acc[ib] += (xb - ib as f32) * weight;
    }
}

fn area(poly: &[Pt]) -> f32 {
    let n = poly.len();
    (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum::<f32>() / 2.0
}

/// Orients a polygon positively (so a union of strokes under the non-zero rule never cancels) and drops degenerate ones.
fn positive(mut poly: Vec<Pt>, out: &mut Vec<Vec<Pt>>) {
    let a = area(&poly);
    if !a.is_finite() || a.abs() < 1e-6 {
        return;
    }
    if a < 0.0 {
        poly.reverse();
    }
    out.push(poly);
}

fn circle(c: Pt, r: f32, out: &mut Vec<Vec<Pt>>) {
    let n = ((r * 3.0).ceil() as usize).clamp(12, 64);
    let poly = (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect();
    positive(poly, out);
}

/// One polyline to stroke. A closed run has an implicit last edge back to its first point.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub points: Vec<Pt>,
    pub closed: bool,
}

/// The polygons that cover a stroke of `style.width` around `runs`. Dashing happens before this (the runs are the
/// dashes). The polygons overlap; fill them together with the non-zero rule.
pub fn stroke_polygons(runs: &[Run], style: &StrokeStyle) -> Vec<Vec<Pt>> {
    let hw = style.width.max(0.0) / 2.0;
    let mut out = Vec::new();
    if hw <= 0.0 {
        return out;
    }
    for run in runs {
        let mut pts: Vec<Pt> = Vec::with_capacity(run.points.len() + 1);
        for &p in &run.points {
            if pts.last().map_or(true, |l| (l.0 - p.0).abs() > 1e-4 || (l.1 - p.1).abs() > 1e-4) {
                pts.push(p);
            }
        }
        let mut closed = run.closed;
        if closed && pts.len() > 1 && same(pts[0], pts[pts.len() - 1]) {
            pts.pop();
        }
        if pts.len() < 2 {
            // A lone point shows only with a round or square cap.
            if let Some(&p) = pts.first() {
                match style.cap {
                    LineCap::Round => circle(p, hw, &mut out),
                    LineCap::Square => positive(vec![(p.0 - hw, p.1 - hw), (p.0 + hw, p.1 - hw), (p.0 + hw, p.1 + hw), (p.0 - hw, p.1 + hw)], &mut out),
                    LineCap::Butt => {}
                }
            }
            continue;
        }
        if pts.len() < 3 {
            closed = false;
        }
        if closed {
            pts.push(pts[0]);
        }
        let n = pts.len();
        let dirs: Vec<Pt> = (0..n - 1).map(|i| unit(pts[i], pts[i + 1])).collect();
        for i in 0..n - 1 {
            let (p, q, d) = (pts[i], pts[i + 1], dirs[i]);
            let nrm = (-d.1 * hw, d.0 * hw);
            let (mut a, mut b) = (p, q);
            if !closed && style.cap == LineCap::Square {
                if i == 0 {
                    a = (p.0 - d.0 * hw, p.1 - d.1 * hw);
                }
                if i == n - 2 {
                    b = (q.0 + d.0 * hw, q.1 + d.1 * hw);
                }
            }
            positive(vec![(a.0 + nrm.0, a.1 + nrm.1), (b.0 + nrm.0, b.1 + nrm.1), (b.0 - nrm.0, b.1 - nrm.1), (a.0 - nrm.0, a.1 - nrm.1)], &mut out);
        }
        if !closed && style.cap == LineCap::Round {
            circle(pts[0], hw, &mut out);
            circle(pts[n - 1], hw, &mut out);
        }
        // Joins at interior vertices; a closed run also joins where it began.
        for i in 1..n - 1 {
            join(pts[i], dirs[i - 1], dirs[i], hw, style, &mut out);
        }
        if closed {
            join(pts[0], dirs[n - 2], dirs[0], hw, style, &mut out);
        }
    }
    out
}

fn same(a: Pt, b: Pt) -> bool {
    (a.0 - b.0).abs() <= 1e-4 && (a.1 - b.1).abs() <= 1e-4
}

fn unit(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = (dx * dx + dy * dy).sqrt();
    if l > 0.0 {
        (dx / l, dy / l)
    } else {
        (1.0, 0.0)
    }
}

fn join(v: Pt, d0: Pt, d1: Pt, hw: f32, style: &StrokeStyle, out: &mut Vec<Vec<Pt>>) {
    let cross = d0.0 * d1.1 - d0.1 * d1.0;
    let dot = d0.0 * d1.0 + d0.1 * d1.1;
    if cross.abs() < 1e-5 && dot > 0.0 {
        return;
    }
    if style.join == LineJoin::Round {
        circle(v, hw, out);
        return;
    }
    // The outer side of the turn is on the negative normal when the path turns towards positive y.
    let s = if cross > 0.0 { -1.0 } else { 1.0 };
    let n0 = (-d0.1 * hw * s, d0.0 * hw * s);
    let n1 = (-d1.1 * hw * s, d1.0 * hw * s);
    let (a, b) = ((v.0 + n0.0, v.1 + n0.1), (v.0 + n1.0, v.1 + n1.1));
    // Interior angle between the two segments, at the vertex.
    let interior = (-dot).clamp(-1.0, 1.0).acos();
    if effective_join(style.join, style.miter_limit, interior) == LineJoin::Miter {
        let m = (n0.0 + n1.0, n0.1 + n1.1);
        let m2 = m.0 * m.0 + m.1 * m.1;
        if m2 > 1e-9 {
            let k = 2.0 * hw * hw / m2;
            let tip = (v.0 + m.0 * k, v.1 + m.1 * k);
            positive(vec![v, a, tip, b], out);
            return;
        }
    }
    positive(vec![v, a, b], out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Pt> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn a_pixel_aligned_square_covers_exactly_its_pixels() {
        let c = fill_polygons(&[square(2.0, 1.0, 5.0, 3.0)], FillRule::NonZero, 8, 4);
        assert_eq!(c.at(2, 1), 255);
        assert_eq!(c.at(4, 2), 255);
        assert_eq!(c.at(5, 1), 0);
        assert_eq!(c.at(1, 1), 0);
        assert_eq!(c.at(2, 3), 0);
    }

    #[test]
    fn a_half_covered_pixel_gets_half_coverage() {
        let c = fill_polygons(&[square(0.5, 0.0, 2.0, 2.0)], FillRule::NonZero, 3, 2);
        assert!((c.at(0, 0) as i32 - 128).abs() <= 2);
        assert_eq!(c.at(1, 0), 255);
    }

    #[test]
    fn even_odd_leaves_a_hole_where_non_zero_fills_it() {
        let polys = [square(0.0, 0.0, 6.0, 6.0), square(2.0, 2.0, 4.0, 4.0)];
        let nz = fill_polygons(&polys, FillRule::NonZero, 6, 6);
        let eo = fill_polygons(&polys, FillRule::EvenOdd, 6, 6);
        assert_eq!(nz.at(3, 3), 255);
        assert_eq!(eo.at(3, 3), 0);
        assert_eq!(eo.at(1, 1), 255);
    }

    #[test]
    fn opposite_windings_cancel_under_non_zero() {
        let mut inner = square(2.0, 2.0, 4.0, 4.0);
        inner.reverse();
        let nz = fill_polygons(&[square(0.0, 0.0, 6.0, 6.0), inner], FillRule::NonZero, 6, 6);
        assert_eq!(nz.at(3, 3), 0);
    }

    #[test]
    fn a_butt_stroke_ends_at_the_point_and_a_square_cap_extends_it() {
        let run = Run { points: vec![(2.0, 4.0), (8.0, 4.0)], closed: false };
        let butt = StrokeStyle { width: 2.0, ..Default::default() };
        let cov = fill_polygons(&stroke_polygons(&[run.clone()], &butt), FillRule::NonZero, 12, 8);
        assert_eq!(cov.at(2, 3), 255);
        assert_eq!(cov.at(2, 4), 255);
        assert_eq!(cov.at(1, 4), 0);
        assert_eq!(cov.at(2, 2), 0);
        let sq = StrokeStyle { width: 2.0, cap: LineCap::Square, ..Default::default() };
        let cov = fill_polygons(&stroke_polygons(&[run], &sq), FillRule::NonZero, 12, 8);
        assert_eq!(cov.at(1, 4), 255);
        assert_eq!(cov.at(8, 4), 255);
        assert_eq!(cov.at(9, 4), 0);
    }

    #[test]
    fn a_round_cap_rounds_the_end() {
        let run = Run { points: vec![(6.0, 6.0), (14.0, 6.0)], closed: false };
        let st = StrokeStyle { width: 8.0, cap: LineCap::Round, ..Default::default() };
        let cov = fill_polygons(&stroke_polygons(&[run], &st), FillRule::NonZero, 20, 12);
        assert_eq!(cov.at(3, 6), 255, "the cap reaches half a width past the start");
        assert_eq!(cov.at(2, 2), 0, "the corner of the square cap box stays empty");
    }

    fn corner() -> Run {
        Run { points: vec![(2.0, 10.0), (10.0, 10.0), (10.0, 2.0)], closed: false }
    }

    #[test]
    fn miter_bevel_and_round_joins_differ_at_the_outer_corner() {
        let cov = |join: LineJoin| {
            let st = StrokeStyle { width: 4.0, join, ..Default::default() };
            fill_polygons(&stroke_polygons(&[corner()], &st), FillRule::NonZero, 16, 16)
        };
        let (m, b, r) = (cov(LineJoin::Miter), cov(LineJoin::Bevel), cov(LineJoin::Round));
        assert_eq!(m.at(11, 11), 255);
        assert_eq!(b.at(11, 11), 0);
        assert!(r.at(11, 11) < 255);
    }

    #[test]
    fn a_low_miter_limit_turns_a_sharp_miter_into_a_bevel() {
        let sharp = Run { points: vec![(2.0, 4.0), (30.0, 6.0), (2.0, 8.0)], closed: false };
        let loose = StrokeStyle { width: 2.0, miter_limit: 100.0, ..Default::default() };
        let tight = StrokeStyle { width: 2.0, miter_limit: 1.0, ..Default::default() };
        let area_of = |st: &StrokeStyle| fill_polygons(&stroke_polygons(&[sharp.clone()], st), FillRule::NonZero, 40, 12).a.iter().map(|&v| v as u32).sum::<u32>();
        assert!(area_of(&loose) > area_of(&tight));
    }

    #[test]
    fn a_closed_run_joins_where_it_began() {
        let run = Run { points: vec![(2.0, 2.0), (10.0, 2.0), (10.0, 10.0), (2.0, 10.0)], closed: true };
        let st = StrokeStyle { width: 2.0, ..Default::default() };
        let cov = fill_polygons(&stroke_polygons(&[run], &st), FillRule::NonZero, 14, 14);
        assert_eq!(cov.at(1, 1), 255, "the miter fills the first corner");
        assert_eq!(cov.at(6, 6), 0);
    }

    #[test]
    fn a_lone_point_shows_with_a_round_cap_only() {
        let run = Run { points: vec![(5.0, 5.0)], closed: false };
        let round = StrokeStyle { width: 4.0, cap: LineCap::Round, ..Default::default() };
        assert!(fill_polygons(&stroke_polygons(&[run.clone()], &round), FillRule::NonZero, 10, 10).at(5, 5) > 0);
        assert!(stroke_polygons(&[run], &StrokeStyle::default()).is_empty());
    }
}
