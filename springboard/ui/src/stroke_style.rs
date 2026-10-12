//! Stroke settings for shape drawing: line caps, line joins, the miter limit and dash patterns. A shape layer of the
//! reference SDK carries these next to its path. This module holds the rules as pure functions: which join a corner
//! gets, and which pieces of a polyline a dash pattern leaves on. The fill rule is `render::bezier::WindingRule`, and
//! the stroke start and end are `shape::Path::trimmed`, so neither is repeated here.
use crate::path::Point;

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

/// The stroke settings a shape carries. The default is a solid 1 point line with butt caps and miter joins.
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
        StrokeStyle {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
        }
    }
}

/// The join a corner is drawn with. A miter join stays a miter while its length, as a multiple of the line width
/// (1 / sin(angle / 2)), is within `miter_limit`. Past that, or at a degenerate angle, it falls back to a bevel.
/// Round and bevel joins are returned as they are. `interior_angle` is the angle between the two segments, in radians.
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

/// Splits an open polyline into the pieces a dash pattern leaves on. `pattern` lists on and off lengths in turn, and
/// `phase` shifts the start into the pattern (a negative phase wraps around). An empty pattern, a negative or
/// non-finite length, or a pattern that sums to zero leaves the line solid. Each returned piece has at least two
/// points, and a corner that falls inside a dash stays a corner of that piece.
pub fn dash_polyline(points: &[Point], pattern: &[f32], phase: f32) -> Vec<Vec<Point>> {
    if points.len() < 2 {
        return Vec::new();
    }
    let valid = pattern.iter().all(|v| v.is_finite() && *v >= 0.0);
    let base: f32 = pattern.iter().sum();
    if pattern.is_empty() || !valid || !(base > 0.0) {
        return vec![points.to_vec()];
    }
    let pat: Vec<f32> = if pattern.len() % 2 == 1 {
        pattern.iter().chain(pattern.iter()).copied().collect()
    } else {
        pattern.to_vec()
    };
    let total: f32 = pat.iter().sum();
    let phase = if phase.is_finite() { phase.rem_euclid(total) } else { 0.0 };

    // Find the entry the phase lands in, and how much of that entry is left.
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

    let mut on = idx % 2 == 0;
    let mut at = points[0];
    let mut cur: Vec<Point> = if on { vec![at] } else { Vec::new() };
    let mut out: Vec<Vec<Point>> = Vec::new();
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        let len = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
        if !(len > 0.0) {
            continue;
        }
        let mut pos = 0.0f32;
        loop {
            // Move past every entry that is used up here; zero-length entries switch at once.
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
        // A corner between two segments belongs to the dash that is still on.
        if on {
            push_unique(&mut cur, q);
        }
    }
    if on && cur.len() >= 2 {
        out.push(cur);
    }
    out
}

fn push_unique(v: &mut Vec<Point>, p: Point) {
    if v.last() != Some(&p) {
        v.push(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: [Point; 2] = [(0.0, 0.0), (10.0, 0.0)];

    #[test]
    fn default_style_is_a_solid_mitred_hairline() {
        let s = StrokeStyle::default();
        assert_eq!(s.width, 1.0);
        assert_eq!(s.cap, LineCap::Butt);
        assert_eq!(s.join, LineJoin::Miter);
        assert_eq!(s.miter_limit, 10.0);
        assert!(s.dash.is_empty());
    }

    #[test]
    fn miter_join_falls_back_to_bevel_past_the_limit() {
        // A right angle has a miter 1 / sin(45 deg) = 1.414 times the line width.
        let right = std::f32::consts::FRAC_PI_2;
        assert_eq!(effective_join(LineJoin::Miter, 1.5, right), LineJoin::Miter);
        assert_eq!(effective_join(LineJoin::Miter, 1.2, right), LineJoin::Bevel);
        // A 60 degree corner has a miter of 2, so a limit of 2.5 keeps it and 1.9 does not.
        let sixty = std::f32::consts::FRAC_PI_3;
        assert_eq!(effective_join(LineJoin::Miter, 2.5, sixty), LineJoin::Miter);
        assert_eq!(effective_join(LineJoin::Miter, 1.9, sixty), LineJoin::Bevel);
        // A straight continuation has a miter of 1, and a zero angle can never be mitred.
        assert_eq!(effective_join(LineJoin::Miter, 1.5, std::f32::consts::PI), LineJoin::Miter);
        assert_eq!(effective_join(LineJoin::Miter, 10.0, 0.0), LineJoin::Bevel);
    }

    #[test]
    fn round_and_bevel_joins_are_not_changed() {
        assert_eq!(effective_join(LineJoin::Round, 1.0, 0.1), LineJoin::Round);
        assert_eq!(effective_join(LineJoin::Bevel, 10.0, std::f32::consts::FRAC_PI_2), LineJoin::Bevel);
    }

    #[test]
    fn dash_splits_a_line_into_on_and_off_runs() {
        // On for 4, off for 2, on for 4, off for 2 (cut at the end of the line).
        let got = dash_polyline(&LINE, &[4.0, 2.0], 0.0);
        assert_eq!(got, vec![vec![(0.0, 0.0), (4.0, 0.0)], vec![(6.0, 0.0), (10.0, 0.0)]]);
    }

    #[test]
    fn odd_pattern_is_read_as_twice_over() {
        // [3] reads as [3, 3]: on 0..3, off 3..6, on 6..9.
        let line = [(0.0, 0.0), (9.0, 0.0)];
        assert_eq!(dash_polyline(&line, &[3.0], 0.0), vec![vec![(0.0, 0.0), (3.0, 0.0)], vec![(6.0, 0.0), (9.0, 0.0)]]);
    }

    #[test]
    fn phase_moves_the_start_and_a_negative_phase_wraps() {
        // Phase 5 is one unit into the 2 unit off run, so the first dash is 1..5, then 7..10 (cut at the end).
        let want = vec![vec![(1.0, 0.0), (5.0, 0.0)], vec![(7.0, 0.0), (10.0, 0.0)]];
        assert_eq!(dash_polyline(&LINE, &[4.0, 2.0], 5.0), want);
        // A phase of -1 is the same as 5 in a pattern that sums to 6.
        assert_eq!(dash_polyline(&LINE, &[4.0, 2.0], -1.0), want);
    }

    #[test]
    fn dash_restarts_across_a_corner() {
        // The path is 10 right, then 10 down; [5, 5] puts dashes at 0..5 and 10..15, which is (10,0) to (10,5).
        let l_shape = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(
            dash_polyline(&l_shape, &[5.0, 5.0], 0.0),
            vec![vec![(0.0, 0.0), (5.0, 0.0)], vec![(10.0, 0.0), (10.0, 5.0)]]
        );
    }

    #[test]
    fn empty_zero_or_invalid_patterns_leave_the_line_solid() {
        let solid = vec![LINE.to_vec()];
        assert_eq!(dash_polyline(&LINE, &[], 0.0), solid);
        assert_eq!(dash_polyline(&LINE, &[0.0, 0.0], 0.0), solid);
        assert_eq!(dash_polyline(&LINE, &[-1.0, 2.0], 0.0), solid);
        assert_eq!(dash_polyline(&LINE, &[f32::NAN], 0.0), solid);
        // A line with fewer than two points has nothing to draw.
        assert!(dash_polyline(&[(0.0, 0.0)], &[1.0], 0.0).is_empty());
    }
}
