//! Lasso selection (`NativeLassoTool`): a closed loop drawn over a drawing selects the strokes it reaches, and the
//! selected strokes can be moved or scaled together. This is pure geometry over `drawing::Drawing`. The touch loop,
//! the selection outline and the tool palette belong to the shell and apps.
//!
//! Rule: a stroke is selected when at least one of its points lies inside the loop. The loop closes itself, from the
//! last point back to the first, and the test is even-odd, so a loop may be concave.
use crate::cg::path::Affine;
use crate::drawing::{union, Drawing};
use crate::Rect;

/// The loop being drawn, in drawing coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lasso {
    points: Vec<(f32, f32)>,
}

impl Lasso {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the next point of the loop. A point with a NaN or infinite coordinate is ignored.
    pub fn add(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.points.push((x, y));
        }
    }

    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// Whether `(x, y)` is inside the loop. A loop with fewer than three points encloses nothing.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let p = &self.points;
        if p.len() < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = p.len() - 1;
        for i in 0..p.len() {
            let (xi, yi) = p[i];
            let (xj, yj) = p[j];
            // Edges that straddle the horizontal line through y. Their endpoints differ in y, so the divisor is not zero.
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// The strokes of `drawing` that the loop reaches, in drawing order.
    pub fn select(&self, drawing: &Drawing) -> Selection {
        let indices = drawing
            .strokes()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.points.iter().any(|p| self.contains(p.x, p.y)))
            .map(|(i, _)| i)
            .collect();
        Selection { indices }
    }
}

/// The selected strokes of a drawing, as ascending stroke indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    indices: Vec<usize>,
}

impl Selection {
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn contains_stroke(&self, index: usize) -> bool {
        self.indices.binary_search(&index).is_ok()
    }

    /// The box around the selected strokes, or `None` when nothing selected has a point.
    pub fn bounds(&self, drawing: &Drawing) -> Option<Rect> {
        self.indices
            .iter()
            .filter_map(|&i| drawing.strokes().get(i).and_then(|s| s.bounds()))
            .fold(None, |acc, r| match acc {
                None => Some(r),
                Some(a) => Some(union(a, r)),
            })
    }

    /// Maps the selected strokes through `t` and leaves the rest of the drawing alone. Indices that are no
    /// longer in the drawing are skipped.
    pub fn transform(&self, drawing: &mut Drawing, t: Affine) {
        for &i in &self.indices {
            if let Some(s) = drawing.stroke_mut(i) {
                s.transform(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{Ink, InkKind, Stroke, StrokePoint};

    fn square() -> Lasso {
        let mut l = Lasso::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            l.add(x, y);
        }
        l
    }

    fn stroke(points: &[(f32, f32)]) -> Stroke {
        let points = points.iter().map(|&(x, y)| StrokePoint::at(x, y)).collect();
        Stroke { ink: Ink { kind: InkKind::Pen, color: 0xff00_0000, width: 2.0 }, points }
    }

    #[test]
    fn square_loop_contains_its_interior_only() {
        let l = square();
        assert!(l.contains(5.0, 5.0));
        assert!(!l.contains(15.0, 5.0));
        assert!(!l.contains(-1.0, 5.0));
        assert!(!l.contains(5.0, 11.0));
    }

    #[test]
    fn concave_loop_leaves_its_notch_outside() {
        // A U shape: two arms on the left and right, the base along the bottom.
        let mut l = Lasso::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (7.0, 10.0), (7.0, 3.0), (3.0, 3.0), (3.0, 10.0), (0.0, 10.0)] {
            l.add(x, y);
        }
        assert!(l.contains(1.0, 5.0), "left arm");
        assert!(l.contains(5.0, 1.0), "base");
        assert!(!l.contains(5.0, 5.0), "notch between the arms");
    }

    #[test]
    fn a_loop_of_fewer_than_three_points_selects_nothing() {
        let mut l = Lasso::new();
        l.add(0.0, 0.0);
        l.add(10.0, 10.0);
        assert!(!l.contains(5.0, 5.0));
        let d = Drawing::from_strokes(vec![stroke(&[(5.0, 5.0)])]);
        assert!(l.select(&d).is_empty());
    }

    #[test]
    fn select_keeps_strokes_with_any_point_inside() {
        let d = Drawing::from_strokes(vec![
            stroke(&[(2.0, 2.0), (3.0, 3.0)]),   // 0: inside
            stroke(&[(20.0, 20.0)]),             // 1: outside
            stroke(&[(-5.0, 5.0), (5.0, 5.0)]),  // 2: crosses the left edge, one point inside
        ]);
        let sel = square().select(&d);
        assert_eq!(sel.indices(), &[0usize, 2]);
        assert!(sel.contains_stroke(2));
        assert!(!sel.contains_stroke(1));
    }

    #[test]
    fn selection_bounds_join_the_selected_strokes() {
        let d = Drawing::from_strokes(vec![
            stroke(&[(2.0, 2.0), (3.0, 3.0)]),
            stroke(&[(20.0, 20.0)]),
            stroke(&[(-5.0, 5.0), (5.0, 5.0)]),
        ]);
        let sel = square().select(&d);
        // stroke 0 is x 2..3, y 2..3; stroke 2 is x -5..5, y 5
        assert_eq!(sel.bounds(&d), Some(Rect { x: -5.0, y: 2.0, w: 10.0, h: 3.0 }));
        assert_eq!(Selection::default().bounds(&d), None);
    }

    #[test]
    fn transform_moves_only_the_selected_strokes() {
        let mut d = Drawing::from_strokes(vec![stroke(&[(2.0, 2.0)]), stroke(&[(20.0, 20.0)])]);
        let sel = square().select(&d);
        sel.transform(&mut d, Affine::translate(100.0, 0.0));
        assert_eq!((d.strokes()[0].points[0].x, d.strokes()[0].points[0].y), (102.0, 2.0));
        assert_eq!((d.strokes()[1].points[0].x, d.strokes()[1].points[0].y), (20.0, 20.0));
    }

    #[test]
    fn non_finite_loop_points_are_ignored() {
        let mut l = Lasso::new();
        l.add(f32::NAN, 1.0);
        l.add(1.0, f32::INFINITY);
        assert!(l.points().is_empty());
        l.add(0.0, 0.0);
        assert_eq!(l.points(), &[(0.0f32, 0.0f32)]);
    }
}
