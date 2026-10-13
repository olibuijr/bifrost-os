//! Scan conversion and stroking: paths and polygons to an 8-bit coverage mask (non-zero or even-odd, exact
//! horizontal spans over several sub-scanlines), and a stroker (caps, joins, miter limit, dashes) that turns a path
//! into an outline path to fill with the non-zero rule. Nothing here touches a pixel buffer. Device pixels, origin
//! at the top left.
use super::path::{Affine, Path};

/// Sub-scanlines per pixel row. Horizontal coverage is exact, vertical coverage is sampled this many times.
const SUB: usize = 8;

pub type Pt = (f32, f32);

/// The shape of an open end of a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// The shape of a corner where two stroked segments meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// Stroke settings. The default is a solid 1 unit line with butt caps and miter joins.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Longest miter, as a multiple of the line width, before a miter join falls back to a bevel.
    pub miter_limit: f32,
    /// On and off lengths in turn. An odd count is read as if it were written twice over.
    pub dash: Vec<f32>,
    pub dash_phase: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        StrokeStyle { width: 1.0, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 10.0, dash: Vec::new(), dash_phase: 0.0 }
    }
}

/// The join a corner is drawn with: a miter stays one while its length (1 / sin(angle / 2) times the width) is
/// within `miter_limit`, else it is a bevel. `interior_angle` is the angle between the two segments, in radians.
pub fn effective_join(join: LineJoin, miter_limit: f32, interior_angle: f32) -> LineJoin {
    if join != LineJoin::Miter {
        return join;
    }
    let s = (interior_angle / 2.0).sin();
    if s > 0.0 && 1.0 / s <= miter_limit {
        LineJoin::Miter
    } else {
        LineJoin::Bevel
    }
}

/// Splits an open polyline into the pieces a dash pattern leaves on. `pattern` lists on and off lengths in turn,
/// `phase` shifts the start into the pattern (a negative phase wraps). An empty, negative, non-finite or zero-sum
/// pattern leaves the line solid. Every piece has at least two points.
pub fn dash_polyline(points: &[Pt], pattern: &[f32], phase: f32) -> Vec<Vec<Pt>> {
    if points.len() < 2 {
        return Vec::new();
    }
    let valid = pattern.iter().all(|v| v.is_finite() && *v >= 0.0);
    let base: f32 = pattern.iter().sum();
    if pattern.is_empty() || !valid || base.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return vec![points.to_vec()];
    }
    let pat: Vec<f32> = if pattern.len() % 2 == 1 { pattern.iter().chain(pattern.iter()).copied().collect() } else { pattern.to_vec() };
    let total: f32 = pat.iter().sum();
    let phase = if phase.is_finite() { phase.rem_euclid(total) } else { 0.0 };

    let mut idx = 0usize;
    let mut left = pat[0];
    let mut skip = phase;
    while skip > 0.0 {
        if skip >= left {
            skip -= left;
            idx = (idx + 1) % pat.len();
            left = pat[idx];
        } else {
            left -= skip;
            skip = 0.0;
        }
    }

    let mut on = idx.is_multiple_of(2);
    let mut at = points[0];
    let mut cur: Vec<Pt> = if on { vec![at] } else { Vec::new() };
    let mut out: Vec<Vec<Pt>> = Vec::new();
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        let len = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        if len.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        let mut pos = 0.0f32;
        loop {
            while left <= 0.0 {
                if on {
                    push_unique(&mut cur, at);
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                } else {
                    cur.clear();
                    cur.push(at);
                }
                on = !on;
                idx = (idx + 1) % pat.len();
                left = pat[idx];
            }
            if pos >= len {
                break;
            }
            let step = left.min(len - pos);
            if step >= len - pos {
                pos = len;
            } else {
                pos += step;
            }
            left -= step;
            at = if pos >= len { q } else { (p.0 + (q.0 - p.0) * pos / len, p.1 + (q.1 - p.1) * pos / len) };
        }
        if on {
            push_unique(&mut cur, q);
        }
    }
    if on && cur.len() >= 2 {
        out.push(cur);
    }
    out
}

fn push_unique(v: &mut Vec<Pt>, p: Pt) {
    if v.last() != Some(&p) {
        v.push(p);
    }
}

/// How overlapping or nested outlines decide what lies inside a fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    /// A point is inside when the outline winds around it at least once, in either direction.
    #[default]
    NonZero,
    /// A point is inside when a ray from it crosses the outline an odd number of times.
    EvenOdd,
}

/// Per-pixel coverage, 0 to 255, for the pixel box whose top left is `(x, y)` in device space.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub a: Vec<u8>,
}

impl Coverage {
    pub fn new(w: usize, h: usize) -> Self {
        Coverage { x: 0, y: 0, w, h, a: vec![0; w * h] }
    }
    pub fn empty() -> Self {
        Self::new(0, 0)
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    /// Coverage at a mask-local pixel.
    pub fn at(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h {
            self.a[y * self.w + x]
        } else {
            0
        }
    }
    /// Coverage at a device pixel (0 outside the mask box).
    pub fn get(&self, dx: i32, dy: i32) -> u8 {
        let (lx, ly) = (dx - self.x, dy - self.y);
        if lx >= 0 && ly >= 0 {
            self.at(lx as usize, ly as usize)
        } else {
            0
        }
    }
    /// The mask scaled by `clip` pixel by pixel; the result covers the overlap of the two boxes.
    pub fn clipped_by(&self, clip: &Coverage) -> Coverage {
        let x0 = self.x.max(clip.x);
        let y0 = self.y.max(clip.y);
        let x1 = (self.x + self.w as i32).min(clip.x + clip.w as i32);
        let y1 = (self.y + self.h as i32).min(clip.y + clip.h as i32);
        if x1 <= x0 || y1 <= y0 {
            return Coverage::empty();
        }
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut out = Coverage { x: x0, y: y0, w, h, a: vec![0; w * h] };
        for yy in 0..h {
            for xx in 0..w {
                let (dx, dy) = (x0 + xx as i32, y0 + yy as i32);
                let v = self.get(dx, dy) as u32 * clip.get(dx, dy) as u32;
                out.a[yy * w + xx] = ((v + 127) / 255) as u8;
            }
        }
        out
    }
}

/// Fills `path` mapped through `t` into a coverage mask limited to the `w` x `h` device box and to `clip`.
pub fn fill(path: &Path, rule: FillRule, t: &Affine, clip: Option<&Coverage>, w: usize, h: usize) -> Coverage {
    let polys: Vec<Vec<Pt>> = path
        .transformed(t)
        // 0.05 px: a 0.1 px flattening leaves a 20 px circle about 1% short of its area
        .flatten_subpaths(0.05)
        .into_iter()
        .filter(|s| s.points.len() >= 3)
        .map(|s| s.points)
        .collect();
    let mut x0 = 0i32;
    let mut y0 = 0i32;
    let mut x1 = w as i32;
    let mut y1 = h as i32;
    if let Some(c) = clip {
        x0 = x0.max(c.x);
        y0 = y0.max(c.y);
        x1 = x1.min(c.x + c.w as i32);
        y1 = y1.min(c.y + c.h as i32);
    }
    let (mut bx0, mut by0, mut bx1, mut by1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for p in polys.iter().flatten() {
        if p.0.is_finite() && p.1.is_finite() {
            bx0 = bx0.min(p.0);
            by0 = by0.min(p.1);
            bx1 = bx1.max(p.0);
            by1 = by1.max(p.1);
        }
    }
    if !matches!(bx0.partial_cmp(&bx1), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)) {
        return Coverage::empty();
    }
    let x0 = x0.max(bx0.floor().max(-1e9) as i32);
    let y0 = y0.max(by0.floor().max(-1e9) as i32);
    let x1 = x1.min(bx1.ceil().min(1e9) as i32);
    let y1 = y1.min(by1.ceil().min(1e9) as i32);
    if x1 <= x0 || y1 <= y0 {
        return Coverage::empty();
    }
    let shifted: Vec<Vec<Pt>> = polys.iter().map(|p| p.iter().map(|q| (q.0 - x0 as f32, q.1 - y0 as f32)).collect()).collect();
    let mut cov = fill_polygons(&shifted, rule, (x1 - x0) as usize, (y1 - y0) as usize);
    cov.x = x0;
    cov.y = y0;
    match clip {
        Some(c) => cov.clipped_by(c),
        None => cov,
    }
}

/// Strokes `path` with `style` (in the path's own space, so a later transform scales the pen too) and fills the
/// outline with the non-zero rule.
pub fn fill_stroke(path: &Path, style: &StrokeStyle, t: &Affine, clip: Option<&Coverage>, w: usize, h: usize) -> Coverage {
    fill(&stroke_path(path, style), FillRule::NonZero, t, clip, w, h)
}

/// The outline of `path` stroked with `style`, as a path of overlapping closed polygons (fill it non-zero).
pub fn stroke_path(path: &Path, style: &StrokeStyle) -> Path {
    let tol = (style.width * 0.05).clamp(0.005, 0.1);
    stroke_path_with_tolerance(path, style, tol)
}

/// [`stroke_path`] with the curve flattening tolerance given.
pub fn stroke_path_with_tolerance(path: &Path, style: &StrokeStyle, tolerance: f32) -> Path {
    let mut runs: Vec<Run> = Vec::new();
    for s in path.flatten_subpaths(tolerance) {
        if style.dash.is_empty() {
            runs.push(Run { points: s.points, closed: s.closed });
        } else {
            let mut pts = s.points;
            if s.closed && pts.len() > 1 {
                pts.push(pts[0]);
            }
            for piece in dash_polyline(&pts, &style.dash, style.dash_phase) {
                runs.push(Run { points: piece, closed: false });
            }
        }
    }
    let mut out = Path::new();
    for poly in stroke_polygons(&runs, style) {
        out.add_lines(&poly);
        out.close();
    }
    out
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
    for (y, row) in rows.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        acc.iter_mut().for_each(|v| *v = 0.0);
        for sub in 0..SUB {
            let sy = y as f32 + (sub as f32 + 0.5) / SUB as f32;
            xs.clear();
            for &ei in row {
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
            if pts.last().is_none_or(|l| (l.0 - p.0).abs() > 1e-4 || (l.1 - p.1).abs() > 1e-4) {
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
mod tests_extra {
    use super::*;
    use crate::cg::path::{Path, Rect};

    #[test]
    fn fill_path_with_transform_and_origin() {
        let p = Path::from_rect(Rect::new(0.0, 0.0, 4.0, 4.0));
        let c = fill(&p, FillRule::NonZero, &Affine::translation(10.0, 5.0), None, 32, 32);
        assert_eq!((c.x, c.y, c.w, c.h), (10, 5, 4, 4));
        assert_eq!(c.get(10, 5), 255);
        assert_eq!(c.get(9, 5), 0);
        assert_eq!(c.get(14, 5), 0);
    }

    #[test]
    fn fill_clamps_to_the_device_box_and_clip() {
        let p = Path::from_rect(Rect::new(-10.0, -10.0, 100.0, 100.0));
        let c = fill(&p, FillRule::NonZero, &Affine::IDENTITY, None, 8, 6);
        assert_eq!((c.x, c.y, c.w, c.h), (0, 0, 8, 6));
        let clip = fill(&Path::from_ellipse(Rect::new(0.0, 0.0, 8.0, 6.0)), FillRule::NonZero, &Affine::IDENTITY, None, 8, 6);
        let both = fill(&p, FillRule::NonZero, &Affine::IDENTITY, Some(&clip), 8, 6);
        assert_eq!(both.get(0, 0), 0, "outside the ellipse");
        assert_eq!(both.get(4, 3), 255);
    }

    #[test]
    fn circle_area_is_close_to_pi_r_squared() {
        let c = fill(&Path::from_ellipse(Rect::new(1.0, 1.0, 20.0, 20.0)), FillRule::NonZero, &Affine::IDENTITY, None, 24, 24);
        let area: f32 = c.a.iter().map(|&v| v as f32 / 255.0).sum();
        assert!((area - std::f32::consts::PI * 100.0).abs() < 3.0, "{area}");
    }

    #[test]
    fn stroke_path_of_a_dashed_line() {
        let mut p = Path::new();
        p.add_lines(&[(0.0, 5.0), (20.0, 5.0)]);
        let st = StrokeStyle { width: 2.0, dash: vec![4.0, 4.0], ..Default::default() };
        let c = fill_stroke(&p, &st, &Affine::IDENTITY, None, 24, 10);
        assert_eq!(c.get(1, 5), 255);
        assert_eq!(c.get(5, 5), 0);
        assert_eq!(c.get(9, 5), 255);
    }

    #[test]
    fn dash_pieces_and_joins() {
        let line = [(0.0, 0.0), (10.0, 0.0)];
        assert_eq!(dash_polyline(&line, &[4.0, 2.0], 0.0), vec![vec![(0.0, 0.0), (4.0, 0.0)], vec![(6.0, 0.0), (10.0, 0.0)]]);
        assert_eq!(dash_polyline(&line, &[], 0.0), vec![line.to_vec()]);
        let right = std::f32::consts::FRAC_PI_2;
        assert_eq!(effective_join(LineJoin::Miter, 1.5, right), LineJoin::Miter);
        assert_eq!(effective_join(LineJoin::Miter, 1.2, right), LineJoin::Bevel);
    }
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
