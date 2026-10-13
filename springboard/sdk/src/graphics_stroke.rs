//! Path stroker for `GraphicsContext`: turns flattened polylines into filled outline polygons
//! (`CGContext.strokePath` with line width, cap, join, miter limit and dash).
//!
//! Every piece (segment quad, join wedge, cap) is emitted with positive orientation, so filling the set with the
//! non-zero rule yields their union.

/// `CGLineCap`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// `CGLineJoin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

pub type Pt = (f32, f32);
/// A flattened subpath and whether it is closed.
pub type Poly = (Vec<Pt>, bool);

/// Stroke parameters in device units.
#[derive(Clone, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash_phase: f32,
    pub dash: Vec<f32>,
}

impl StrokeStyle {
    pub fn new(width: f32) -> StrokeStyle {
        StrokeStyle { width, cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 10.0, dash_phase: 0.0, dash: Vec::new() }
    }
}

fn positive(mut poly: Vec<Pt>) -> Vec<Pt> {
    let n = poly.len();
    let area: f32 = (0..n).map(|i| poly[i].0 * poly[(i + 1) % n].1 - poly[(i + 1) % n].0 * poly[i].1).sum();
    if area < 0.0 {
        poly.reverse();
    }
    poly
}

fn arc_points(c: Pt, r: f32, from: f32, to: f32) -> Vec<Pt> {
    let steps = ((to - from).abs() / (std::f32::consts::TAU / 24.0)).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

fn dedupe(pts: &[Pt], closed: bool) -> Vec<Pt> {
    let mut v: Vec<Pt> = Vec::with_capacity(pts.len());
    for &p in pts {
        if v.last().map_or(true, |q| (q.0 - p.0).abs() > 1e-5 || (q.1 - p.1).abs() > 1e-5) {
            v.push(p);
        }
    }
    if closed && v.len() > 1 && v[0] == v[v.len() - 1] {
        v.pop();
    }
    v
}

/// Splits polylines into dash pieces. An empty or all-zero pattern returns the input unchanged.
pub fn dash_polylines(polys: &[Poly], dash: &[f32], phase: f32) -> Vec<Poly> {
    let total: f32 = dash.iter().sum();
    if dash.is_empty() || total <= 0.0 || dash.iter().any(|d| *d < 0.0) {
        return polys.to_vec();
    }
    let period = total * if dash.len() % 2 == 1 { 2.0 } else { 1.0 };
    let mut out = Vec::new();
    for (pts, closed) in polys {
        let mut pts = dedupe(pts, *closed);
        if pts.len() < 2 {
            continue;
        }
        if *closed {
            pts.push(pts[0]);
        }
        // Find the pattern entry that `phase` falls into.
        let mut idx = 0usize;
        let mut ph = phase.rem_euclid(period);
        while ph >= dash[idx % dash.len()] {
            ph -= dash[idx % dash.len()];
            idx += 1;
        }
        let mut left = dash[idx % dash.len()] - ph;
        let mut cur: Vec<Pt> = Vec::new();
        if idx % 2 == 0 {
            cur.push(pts[0]);
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let mut pos = 0.0;
            while len - pos > left {
                pos += left;
                let t = pos / len;
                let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                if idx % 2 == 0 {
                    cur.push(p);
                    out.push((std::mem::take(&mut cur), false));
                } else {
                    cur = vec![p];
                }
                idx += 1;
                left = dash[idx % dash.len()];
            }
            left -= len - pos;
            if idx % 2 == 0 {
                cur.push(b);
            }
        }
        if cur.len() > 1 {
            out.push((cur, false));
        }
    }
    out
}

/// Outline polygons of the stroke, to be filled with the non-zero rule.
pub fn stroke_outline(polys: &[Poly], style: &StrokeStyle) -> Vec<Vec<Pt>> {
    let hw = (style.width * 0.5).max(0.0);
    let mut out = Vec::new();
    if hw <= 0.0 {
        return out;
    }
    for (raw, closed) in dash_polylines(polys, &style.dash, style.dash_phase) {
        let pts = dedupe(&raw, closed);
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let closed = closed && n > 2;
        let nseg = if closed { n } else { n - 1 };
        let dir = |i: usize| {
            let (a, b) = (pts[i % n], pts[(i + 1) % n]);
            let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            ((b.0 - a.0) / l, (b.1 - a.1) / l)
        };
        for i in 0..nseg {
            let (a, b, d) = (pts[i], pts[(i + 1) % n], dir(i));
            let nm = (-d.1 * hw, d.0 * hw);
            out.push(positive(vec![(a.0 + nm.0, a.1 + nm.1), (b.0 + nm.0, b.1 + nm.1), (b.0 - nm.0, b.1 - nm.1), (a.0 - nm.0, a.1 - nm.1)]));
        }
        let joins: Vec<usize> = if closed { (0..n).collect() } else { (1..n - 1).collect() };
        for v in joins {
            let (d0, d1) = (dir((v + nseg - 1) % nseg), dir(v % nseg));
            out.extend(join_piece(pts[v], d0, d1, hw, style));
        }
        if !closed {
            let d0 = dir(0);
            out.extend(cap_piece(pts[0], (-d0.0, -d0.1), hw, style.cap));
            out.extend(cap_piece(pts[n - 1], dir(n - 2), hw, style.cap));
        }
    }
    out
}

fn join_piece(p: Pt, d0: Pt, d1: Pt, hw: f32, style: &StrokeStyle) -> Option<Vec<Pt>> {
    let cross = d0.0 * d1.1 - d0.1 * d1.0;
    let dot = d0.0 * d1.0 + d0.1 * d1.1;
    if cross.abs() < 1e-6 && dot > 0.0 {
        return None;
    }
    // The outer side of the turn is opposite to the turn direction.
    let s = if cross > 0.0 { -1.0 } else { 1.0 };
    let n0 = (-d0.1 * hw * s, d0.0 * hw * s);
    let n1 = (-d1.1 * hw * s, d1.0 * hw * s);
    let (a, b) = ((p.0 + n0.0, p.1 + n0.1), (p.0 + n1.0, p.1 + n1.1));
    let poly = match style.join {
        LineJoin::Bevel => vec![p, a, b],
        LineJoin::Round => {
            let (a0, mut a1) = (n0.1.atan2(n0.0), n1.1.atan2(n1.0));
            // Sweep the short way round from n0 to n1.
            while a1 - a0 > std::f32::consts::PI {
                a1 -= std::f32::consts::TAU;
            }
            while a0 - a1 > std::f32::consts::PI {
                a1 += std::f32::consts::TAU;
            }
            let mut v = vec![p];
            v.extend(arc_points(p, hw, a0, a1));
            v
        }
        LineJoin::Miter => {
            let cos_half = ((1.0 + dot) * 0.5).max(0.0).sqrt();
            if cos_half > 1e-4 && 1.0 / cos_half <= style.miter_limit {
                let k = 1.0 / (1.0 + dot);
                vec![p, a, (p.0 + (n0.0 + n1.0) * k, p.1 + (n0.1 + n1.1) * k), b]
            } else {
                vec![p, a, b]
            }
        }
    };
    Some(positive(poly))
}

/// Cap at `p`; `d` points away from the line.
fn cap_piece(p: Pt, d: Pt, hw: f32, cap: LineCap) -> Option<Vec<Pt>> {
    let nm = (-d.1 * hw, d.0 * hw);
    match cap {
        LineCap::Butt => None,
        LineCap::Square => Some(positive(vec![
            (p.0 + nm.0, p.1 + nm.1),
            (p.0 + nm.0 + d.0 * hw, p.1 + nm.1 + d.1 * hw),
            (p.0 - nm.0 + d.0 * hw, p.1 - nm.1 + d.1 * hw),
            (p.0 - nm.0, p.1 - nm.1),
        ])),
        LineCap::Round => {
            let a = d.1.atan2(d.0);
            Some(positive(arc_points(p, hw, a - std::f32::consts::FRAC_PI_2, a + std::f32::consts::FRAC_PI_2)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(p: &[Pt]) -> f32 {
        let n = p.len();
        (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum::<f32>() * 0.5
    }

    fn line() -> Vec<Poly> {
        vec![(vec![(0.0, 0.0), (10.0, 0.0)], false)]
    }

    #[test]
    fn butt_cap_is_one_quad_of_length_times_width() {
        let o = stroke_outline(&line(), &StrokeStyle::new(2.0));
        assert_eq!(o.len(), 1);
        assert!((area(&o[0]) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn square_caps_extend_by_half_width() {
        let mut s = StrokeStyle::new(2.0);
        s.cap = LineCap::Square;
        let total: f32 = stroke_outline(&line(), &s).iter().map(|p| area(p)).sum();
        assert!((total - 24.0).abs() < 1e-4, "{total}");
    }

    #[test]
    fn every_piece_is_positively_oriented() {
        let mut s = StrokeStyle::new(2.0);
        s.cap = LineCap::Round;
        s.join = LineJoin::Round;
        let l = vec![(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], false)];
        assert!(stroke_outline(&l, &s).iter().all(|p| area(p) >= 0.0));
    }

    #[test]
    fn miter_limit_falls_back_to_bevel_on_sharp_corners() {
        let sharp = vec![(vec![(0.0, 0.0), (10.0, 0.0), (0.0, 1.0)], false)];
        let far = |o: &Vec<Vec<Pt>>| o.iter().flatten().map(|p| p.0).fold(f32::MIN, f32::max);
        let mut s = StrokeStyle::new(2.0);
        s.miter_limit = 100.0;
        assert!(far(&stroke_outline(&sharp, &s)) > 15.0);
        s.miter_limit = 2.0;
        assert!(far(&stroke_outline(&sharp, &s)) < 12.0);
    }

    #[test]
    fn dash_splits_a_line_with_phase() {
        let d = dash_polylines(&line(), &[2.0, 3.0], 0.0);
        let spans: Vec<(f32, f32)> = d.iter().map(|(p, _)| (p[0].0, p[p.len() - 1].0)).collect();
        assert_eq!(spans, vec![(0.0, 2.0), (5.0, 7.0)]);
        let d = dash_polylines(&line(), &[2.0, 3.0], 1.0);
        assert_eq!(d[0].0[0].0, 0.0);
        assert!((d[0].0[1].0 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn empty_dash_pattern_keeps_the_line() {
        assert_eq!(dash_polylines(&line(), &[], 0.0), line());
    }
}
