//! Pressure-sensitive ink strokes. A pen stroke is a list of sampled points, each with a pressure in
//! 0..=1. `outline` turns it into a closed polygon whose width follows the pressure, so a painter can
//! fill it with the bezier fill rule. Pure geometry: the stroke is computed here, never drawn.

/// Sampling tolerance: two consecutive points closer than this are treated as the same sample.
const EPS: f32 = 1e-6;

/// One sampled pen position in stroke space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    /// Pen pressure, 0 (lightest) to 1 (full). Values outside the range are clamped.
    pub pressure: f32,
}

/// The closed outline of a stroke whose width runs from `min_width` at zero pressure to `max_width`
/// at full pressure. The outline walks one side of the stroke forward and the other side back.
/// `None` when the widths are not finite or ordered, a sample is not finite, or fewer than two
/// distinct samples remain.
pub fn outline(points: &[InkPoint], min_width: f32, max_width: f32) -> Option<Vec<(f32, f32)>> {
    if !min_width.is_finite() || !max_width.is_finite() || min_width < 0.0 || max_width < min_width {
        return None;
    }
    let mut pts: Vec<InkPoint> = Vec::with_capacity(points.len());
    for p in points {
        if !(p.x.is_finite() && p.y.is_finite() && p.pressure.is_finite()) {
            return None;
        }
        if let Some(last) = pts.last() {
            if (last.x - p.x).hypot(last.y - p.y) < EPS {
                continue;
            }
        }
        pts.push(*p);
    }
    let n = pts.len();
    if n < 2 {
        return None;
    }

    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let cur = pts[i];
        let prev = pts[i.saturating_sub(1)];
        let next = pts[(i + 1).min(n - 1)];
        // The tangent spans the neighbours; where they coincide (a doubled-back sample) it falls
        // back to the segment ahead, which is never zero because consecutive samples differ.
        let (mut dx, mut dy) = (next.x - prev.x, next.y - prev.y);
        if dx.hypot(dy) < EPS {
            dx = next.x - cur.x;
            dy = next.y - cur.y;
        }
        let len = dx.hypot(dy);
        let (tx, ty) = (dx / len, dy / len);
        let (nx, ny) = (-ty, tx);
        let width = min_width + (max_width - min_width) * cur.pressure.clamp(0.0, 1.0);
        let half = width * 0.5;
        left.push((cur.x + nx * half, cur.y + ny * half));
        right.push((cur.x - nx * half, cur.y - ny * half));
    }
    right.reverse();
    left.extend(right);
    Some(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn p(x: f32, y: f32, pressure: f32) -> InkPoint {
        InkPoint { x, y, pressure }
    }

    #[test]
    fn rejects_strokes_without_two_distinct_samples() {
        assert!(outline(&[], 1.0, 4.0).is_none());
        assert!(outline(&[p(0.0, 0.0, 1.0)], 1.0, 4.0).is_none());
        assert!(outline(&[p(3.0, 3.0, 1.0), p(3.0, 3.0, 0.5)], 1.0, 4.0).is_none());
    }

    #[test]
    fn rejects_bad_widths_and_non_finite_samples() {
        let line = [p(0.0, 0.0, 1.0), p(10.0, 0.0, 1.0)];
        assert!(outline(&line, 5.0, 2.0).is_none());
        assert!(outline(&line, -1.0, 2.0).is_none());
        assert!(outline(&line, 1.0, f32::INFINITY).is_none());
        assert!(outline(&[p(0.0, 0.0, f32::NAN), p(10.0, 0.0, 1.0)], 1.0, 4.0).is_none());
    }

    #[test]
    fn a_full_pressure_line_is_as_wide_as_the_maximum() {
        let line = [p(0.0, 0.0, 1.0), p(10.0, 0.0, 1.0)];
        let poly = outline(&line, 1.0, 4.0).unwrap();
        assert_eq!(poly.len(), 4);
        // Walking one side forward and the other back: the top edge at +2, the bottom at -2.
        assert!(close(poly[0].1, 2.0) && close(poly[0].0, 0.0));
        assert!(close(poly[1].1, 2.0) && close(poly[1].0, 10.0));
        assert!(close(poly[2].1, -2.0) && close(poly[2].0, 10.0));
        assert!(close(poly[3].1, -2.0) && close(poly[3].0, 0.0));
    }

    #[test]
    fn width_follows_the_pressure_at_each_sample() {
        // Zero pressure gives the minimum width of 1 (half-width 0.5); full pressure gives 5 (2.5).
        let line = [p(0.0, 0.0, 0.0), p(10.0, 0.0, 1.0)];
        let poly = outline(&line, 1.0, 5.0).unwrap();
        assert!(close(poly[0].1, 0.5));
        assert!(close(poly[1].1, 2.5));
    }

    #[test]
    fn pressure_outside_the_unit_range_is_clamped() {
        let soft = [p(0.0, 0.0, 1.0), p(10.0, 0.0, 0.0)];
        let hard = [p(0.0, 0.0, 3.0), p(10.0, 0.0, -2.0)];
        let a = outline(&soft, 1.0, 4.0).unwrap();
        let b = outline(&hard, 1.0, 4.0).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| close(x.0, y.0) && close(x.1, y.1)));
    }

    #[test]
    fn repeated_samples_are_collapsed() {
        let doubled = [p(0.0, 0.0, 1.0), p(0.0, 0.0, 1.0), p(10.0, 0.0, 1.0)];
        let plain = [p(0.0, 0.0, 1.0), p(10.0, 0.0, 1.0)];
        assert_eq!(outline(&doubled, 1.0, 4.0), outline(&plain, 1.0, 4.0));
    }

    #[test]
    fn a_bent_stroke_keeps_its_width_on_the_turn() {
        // An L: right then up. Each side stays half the width away from the centre line.
        let stroke = [p(0.0, 0.0, 1.0), p(10.0, 0.0, 1.0), p(10.0, 10.0, 1.0)];
        let poly = outline(&stroke, 2.0, 2.0).unwrap();
        assert_eq!(poly.len(), 6);
        // At the corner the inner side sits half a width off the bisector of the two segments.
        let s = 1.0 / 2f32.sqrt();
        assert!(close(poly[1].0, 10.0 - s) && close(poly[1].1, s));
        assert!(close(poly[2].0, 9.0) && close(poly[2].1, 10.0));
        assert!(close(poly[3].0, 11.0) && close(poly[3].1, 10.0));
    }
}
