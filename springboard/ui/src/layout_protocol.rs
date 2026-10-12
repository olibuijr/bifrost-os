//! Custom layout protocol: a layout measures itself against a size proposal and places its subviews.
//!
//! This mirrors the shape of a declarative layout protocol with two operations: `size_that_fits` answers
//! "how big are you for this proposal", and `place_subviews` answers "where does each subview go inside the
//! bounds you were given". Subview sizes are their ideal sizes, measured by the caller. A proposal of
//! `f32::INFINITY` on an axis means "no limit on that axis", the same as an absent proposal.
use crate::Rect;

/// A layout that arranges subviews given their ideal `(width, height)` sizes.
pub trait Layout {
    /// The size this layout needs for `proposal` (`(width, height)`, infinity for unbounded).
    fn size_that_fits(&self, proposal: (f32, f32), subviews: &[(f32, f32)]) -> (f32, f32);

    /// The rectangle of each subview inside `bounds`, in subview order.
    fn place_subviews(&self, bounds: Rect, proposal: (f32, f32), subviews: &[(f32, f32)]) -> Vec<Rect>;
}

/// Places subviews left to right and wraps to a new row when the next subview would pass the proposed width.
/// A subview wider than the proposal still gets a row of its own, at the leading edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowLayout {
    /// Gap between subviews on one row.
    pub spacing_x: f32,
    /// Gap between rows.
    pub spacing_y: f32,
}

impl FlowLayout {
    pub fn new(spacing_x: f32, spacing_y: f32) -> Self {
        FlowLayout { spacing_x, spacing_y }
    }

    /// Positions relative to the origin, plus the overall `(right edge, bottom edge)` of the content.
    fn flow(&self, width: f32, subviews: &[(f32, f32)]) -> (Vec<Rect>, (f32, f32)) {
        const EPS: f32 = 1e-4;
        let mut rects = Vec::with_capacity(subviews.len());
        let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
        let mut right = 0.0f32;
        for &(w, h) in subviews {
            let (w, h) = (w.max(0.0), h.max(0.0));
            if x > 0.0 && x + w > width + EPS {
                x = 0.0;
                y += row_h + self.spacing_y;
                row_h = 0.0;
            }
            rects.push(Rect { x, y, w, h });
            right = right.max(x + w);
            row_h = row_h.max(h);
            x += w + self.spacing_x;
        }
        let bottom = if subviews.is_empty() { 0.0 } else { y + row_h };
        (rects, (right, bottom))
    }
}

impl Layout for FlowLayout {
    fn size_that_fits(&self, proposal: (f32, f32), subviews: &[(f32, f32)]) -> (f32, f32) {
        self.flow(proposal.0, subviews).1
    }

    fn place_subviews(&self, bounds: Rect, proposal: (f32, f32), subviews: &[(f32, f32)]) -> Vec<Rect> {
        let (rects, _) = self.flow(proposal.0, subviews);
        rects.into_iter().map(|r| Rect { x: bounds.x + r.x, y: bounds.y + r.y, w: r.w, h: r.h }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f32 = f32::INFINITY;

    #[test]
    fn unbounded_width_puts_every_subview_on_one_row() {
        let layout = FlowLayout::new(4.0, 2.0);
        let subs = [(10.0, 5.0), (20.0, 8.0), (30.0, 6.0)];
        let rects = layout.place_subviews(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, (INF, INF), &subs);
        let xs: Vec<f32> = rects.iter().map(|r| r.x).collect();
        assert_eq!(xs, vec![0.0, 14.0, 38.0], "each subview starts after the previous width plus spacing_x 4");
        assert!(rects.iter().all(|r| r.y == 0.0), "one row, so every subview sits at y 0");
        assert_eq!(layout.size_that_fits((INF, INF), &subs), (68.0, 8.0), "right edge 38 + 30, tallest height 8");
    }

    #[test]
    fn a_row_wraps_when_the_next_subview_passes_the_proposed_width() {
        let layout = FlowLayout::new(5.0, 5.0);
        let subs = [(30.0, 10.0), (30.0, 10.0), (10.0, 10.0)];
        let rects = layout.place_subviews(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, (50.0, INF), &subs);
        assert_eq!((rects[0].x, rects[0].y), (0.0, 0.0));
        assert_eq!((rects[1].x, rects[1].y), (0.0, 15.0), "35 + 30 = 65 > 50, so the second item wraps to y = 10 + 5");
        assert_eq!((rects[2].x, rects[2].y), (35.0, 15.0), "35 + 10 = 45 <= 50, so the third item joins the second row");
    }

    #[test]
    fn size_that_fits_reports_the_widest_row_and_total_height() {
        let layout = FlowLayout::new(5.0, 5.0);
        let subs = [(30.0, 10.0), (30.0, 10.0), (10.0, 10.0)];
        assert_eq!(layout.size_that_fits((50.0, INF), &subs), (45.0, 25.0), "widest right edge 45, two rows of 10 with a gap of 5");
    }

    #[test]
    fn a_subview_wider_than_the_proposal_keeps_its_own_row() {
        let layout = FlowLayout::new(5.0, 5.0);
        let subs = [(60.0, 12.0), (10.0, 4.0)];
        let rects = layout.place_subviews(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, (50.0, INF), &subs);
        assert_eq!((rects[0].x, rects[0].y, rects[0].w), (0.0, 0.0, 60.0), "the first subview never wraps, even when wider");
        assert_eq!((rects[1].x, rects[1].y), (0.0, 17.0), "65 > 50 pushes the second subview to y = 12 + 5");
        assert_eq!(layout.size_that_fits((50.0, INF), &subs), (60.0, 21.0));
    }

    #[test]
    fn placements_are_offset_by_the_bounds_origin() {
        let layout = FlowLayout::new(0.0, 0.0);
        let subs = [(20.0, 10.0), (20.0, 10.0)];
        let rects = layout.place_subviews(Rect { x: 100.0, y: 20.0, w: 300.0, h: 200.0 }, (30.0, INF), &subs);
        assert_eq!((rects[0].x, rects[0].y), (100.0, 20.0));
        assert_eq!((rects[1].x, rects[1].y), (100.0, 30.0), "the second row starts at bounds.y + 10");
    }

    #[test]
    fn no_subviews_measures_to_zero_and_places_nothing() {
        let layout = FlowLayout::new(8.0, 8.0);
        assert_eq!(layout.size_that_fits((100.0, 100.0), &[]), (0.0, 0.0));
        assert!(layout.place_subviews(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, (100.0, 100.0), &[]).is_empty());
    }
}
