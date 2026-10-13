//! Pure geometry for Freeform's Arrange menu and the selection handles: aligning and distributing boxes, the
//! resize and rotate handles around a selection, snapping to the grid, and grouping strokes into units. Nothing here
//! touches the screen, so the host tests cover it.
use bifrost_app::controls::cg::path::Affine;
use bifrost_app::controls::Rect;

/// Which edge or centre line Arrange > Align lines the selected items up on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}

/// The box around all of `boxes`, or `None` when there are none.
pub fn union_of(boxes: &[Rect]) -> Option<Rect> {
    let first = *boxes.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for b in &boxes[1..] {
        x0 = x0.min(b.x);
        y0 = y0.min(b.y);
        x1 = x1.max(b.x + b.w);
        y1 = y1.max(b.y + b.h);
    }
    Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// The move (dx, dy) that takes each box to the chosen edge or centre line of the whole selection.
pub fn align_offsets(boxes: &[Rect], how: Align) -> Vec<(f32, f32)> {
    let Some(all) = union_of(boxes) else { return Vec::new() };
    boxes
        .iter()
        .map(|b| match how {
            Align::Left => (all.x - b.x, 0.0),
            Align::Center => (all.x + all.w / 2.0 - (b.x + b.w / 2.0), 0.0),
            Align::Right => (all.x + all.w - (b.x + b.w), 0.0),
            Align::Top => (0.0, all.y - b.y),
            Align::Middle => (0.0, all.y + all.h / 2.0 - (b.y + b.h / 2.0)),
            Align::Bottom => (0.0, all.y + all.h - (b.y + b.h)),
        })
        .collect()
}

/// The move that spaces the boxes evenly along one axis: the outermost two stay, the gaps between all boxes become
/// equal. Fewer than three boxes need no change.
pub fn distribute_offsets(boxes: &[Rect], horizontal: bool) -> Vec<(f32, f32)> {
    let n = boxes.len();
    let mut out = vec![(0.0, 0.0); n];
    if n < 3 {
        return out;
    }
    let (pos, len): (fn(&Rect) -> f32, fn(&Rect) -> f32) = if horizontal { (|r| r.x, |r| r.w) } else { (|r| r.y, |r| r.h) };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| pos(&boxes[a]).partial_cmp(&pos(&boxes[b])).unwrap_or(std::cmp::Ordering::Equal));
    let first = &boxes[order[0]];
    let last = &boxes[order[n - 1]];
    let span = pos(last) + len(last) - pos(first);
    let total: f32 = boxes.iter().map(len).sum();
    let gap = (span - total) / (n - 1) as f32;
    let mut at = pos(first) + len(first) + gap;
    for &i in &order[1..n - 1] {
        let d = at - pos(&boxes[i]);
        out[i] = if horizontal { (d, 0.0) } else { (0.0, d) };
        at += len(&boxes[i]) + gap;
    }
    out
}

/// Strokes that move together: the strokes of one group, or a single ungrouped stroke. `groups[i]` is the group of
/// stroke `i` (0 for none); `indices` are the selected strokes. Units come out in order of their first stroke.
pub fn units(indices: &[usize], groups: &[u32]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for &i in indices {
        let g = groups.get(i).copied().unwrap_or(0);
        if g == 0 {
            out.push(vec![i]);
        } else if let Some(at) = seen.iter().position(|&s| s == g) {
            out[out.iter().position(|u| groups.get(u[0]).copied() == Some(g)).unwrap_or(at)].push(i);
        } else {
            seen.push(g);
            out.push(vec![i]);
        }
    }
    out
}

/// A handle on the box around the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Above the top edge's middle: drags to turn the selection.
    Rotate,
}

/// Where each handle is for the box `b`; the rotate handle sits `gap` above the top edge.
pub fn handle_points(b: Rect, gap: f32) -> [(Handle, (f32, f32)); 5] {
    [
        (Handle::TopLeft, (b.x, b.y)),
        (Handle::TopRight, (b.x + b.w, b.y)),
        (Handle::BottomLeft, (b.x, b.y + b.h)),
        (Handle::BottomRight, (b.x + b.w, b.y + b.h)),
        (Handle::Rotate, (b.x + b.w / 2.0, b.y - gap)),
    ]
}

/// The handle within `reach` of (x, y), nearest first.
pub fn hit_handle(b: Rect, gap: f32, reach: f32, x: f32, y: f32) -> Option<Handle> {
    handle_points(b, gap)
        .iter()
        .map(|&(h, (hx, hy))| (h, (hx - x).hypot(hy - y)))
        .filter(|&(_, d)| d <= reach)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(h, _)| h)
}

/// The corner that stays put while `h` is dragged (the opposite one). The rotate handle has none.
pub fn anchor_of(b: Rect, h: Handle) -> Option<(f32, f32)> {
    Some(match h {
        Handle::TopLeft => (b.x + b.w, b.y + b.h),
        Handle::TopRight => (b.x, b.y + b.h),
        Handle::BottomLeft => (b.x + b.w, b.y),
        Handle::BottomRight => (b.x, b.y),
        Handle::Rotate => return None,
    })
}

/// The smallest factor a drag can shrink a selection by in one step, so it never collapses or flips.
const MIN_FACTOR: f32 = 0.05;

/// The scale about `anchor` that carries the dragged corner from `prev` to `cur`. An axis with no distance between
/// the anchor and the corner keeps its size.
pub fn resize_step(anchor: (f32, f32), prev: (f32, f32), cur: (f32, f32)) -> Affine {
    let factor = |a: f32, p: f32, c: f32| {
        let d = p - a;
        if d.abs() < 1.0 {
            1.0
        } else {
            ((c - a) / d).max(MIN_FACTOR)
        }
    };
    let (sx, sy) = (factor(anchor.0, prev.0, cur.0), factor(anchor.1, prev.1, cur.1));
    Affine::translate(-anchor.0, -anchor.1).then(Affine::scale(sx, sy)).then(Affine::translate(anchor.0, anchor.1))
}

/// The turn about `centre` that carries the pointer from `prev` to `cur`.
pub fn rotate_step(centre: (f32, f32), prev: (f32, f32), cur: (f32, f32)) -> Affine {
    let a0 = (prev.1 - centre.1).atan2(prev.0 - centre.0);
    let a1 = (cur.1 - centre.1).atan2(cur.0 - centre.0);
    Affine::translate(-centre.0, -centre.1).then(Affine::rotate(a1 - a0)).then(Affine::translate(centre.0, centre.1))
}

/// `v` moved to the nearest multiple of `step` (unchanged when `step` is not positive).
pub fn snap(v: f32, step: f32) -> f32 {
    if step > 0.0 && step.is_finite() {
        (v / step).round() * step
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn align_left_and_right_use_the_selections_outer_edges() {
        let boxes = [r(10.0, 0.0, 20.0, 5.0), r(50.0, 10.0, 40.0, 5.0)];
        assert_eq!(align_offsets(&boxes, Align::Left), vec![(0.0, 0.0), (-40.0, 0.0)]);
        assert_eq!(align_offsets(&boxes, Align::Right), vec![(60.0, 0.0), (0.0, 0.0)]);
    }

    #[test]
    fn align_centre_and_middle_meet_on_the_selections_centre_line() {
        let boxes = [r(0.0, 0.0, 10.0, 10.0), r(30.0, 20.0, 10.0, 10.0)];
        // union is (0,0) 40 x 30: centre x 20, centre y 15
        assert_eq!(align_offsets(&boxes, Align::Center), vec![(15.0, 0.0), (-15.0, 0.0)]);
        assert_eq!(align_offsets(&boxes, Align::Middle), vec![(0.0, 10.0), (0.0, -10.0)]);
        assert_eq!(align_offsets(&boxes, Align::Top), vec![(0.0, 0.0), (0.0, -20.0)]);
        assert_eq!(align_offsets(&boxes, Align::Bottom), vec![(0.0, 20.0), (0.0, 0.0)]);
        assert!(align_offsets(&[], Align::Left).is_empty());
    }

    #[test]
    fn distribute_makes_the_gaps_equal_and_keeps_the_outer_boxes() {
        // widths 10 each at x 0, 12, 90: span 100, total 30, gap 35
        let boxes = [r(0.0, 0.0, 10.0, 10.0), r(12.0, 0.0, 10.0, 10.0), r(90.0, 0.0, 10.0, 10.0)];
        let d = distribute_offsets(&boxes, true);
        assert_eq!(d[0], (0.0, 0.0));
        assert_eq!(d[2], (0.0, 0.0));
        assert_eq!(d[1], (33.0, 0.0), "the middle box moves to x = 45");
    }

    #[test]
    fn distribute_works_on_the_given_axis_whatever_the_order_and_needs_three() {
        let boxes = [r(0.0, 90.0, 10.0, 10.0), r(0.0, 0.0, 10.0, 10.0), r(0.0, 20.0, 10.0, 10.0)];
        let d = distribute_offsets(&boxes, false);
        assert_eq!(d[1], (0.0, 0.0));
        assert_eq!(d[0], (0.0, 0.0));
        assert_eq!(d[2], (0.0, 25.0), "gap is (100 - 30) / 2 = 35, so y = 45");
        assert_eq!(distribute_offsets(&boxes[..2], false), vec![(0.0, 0.0); 2]);
    }

    #[test]
    fn units_join_a_group_and_keep_loose_strokes_apart() {
        let groups = [0, 7, 0, 7, 9];
        assert_eq!(units(&[0, 1, 2, 3, 4], &groups), vec![vec![0], vec![1, 3], vec![2], vec![4]]);
        assert_eq!(units(&[1], &groups), vec![vec![1]]);
        assert!(units(&[], &groups).is_empty());
    }

    #[test]
    fn the_handles_sit_on_the_corners_and_the_rotate_handle_above() {
        let pts = handle_points(r(10.0, 20.0, 40.0, 30.0), 18.0);
        assert_eq!(pts[0], (Handle::TopLeft, (10.0, 20.0)));
        assert_eq!(pts[3], (Handle::BottomRight, (50.0, 50.0)));
        assert_eq!(pts[4], (Handle::Rotate, (30.0, 2.0)));
    }

    #[test]
    fn a_press_near_a_handle_hits_it_and_one_far_away_does_not() {
        let b = r(10.0, 20.0, 40.0, 30.0);
        assert_eq!(hit_handle(b, 18.0, 6.0, 12.0, 22.0), Some(Handle::TopLeft));
        assert_eq!(hit_handle(b, 18.0, 6.0, 30.0, 3.0), Some(Handle::Rotate));
        assert_eq!(hit_handle(b, 18.0, 6.0, 30.0, 35.0), None, "the middle of the box is not a handle");
    }

    #[test]
    fn dragging_a_corner_scales_about_the_opposite_corner() {
        let b = r(0.0, 0.0, 40.0, 20.0);
        let anchor = anchor_of(b, Handle::BottomRight).unwrap();
        assert_eq!(anchor, (0.0, 0.0));
        assert!(anchor_of(b, Handle::Rotate).is_none());
        let t = resize_step(anchor, (40.0, 20.0), (80.0, 30.0));
        let (x, y) = t.apply(40.0, 20.0);
        assert!((x - 80.0).abs() < 1e-3 && (y - 30.0).abs() < 1e-3, "the dragged corner follows the pointer");
        let (ax, ay) = t.apply(0.0, 0.0);
        assert!(ax.abs() < 1e-3 && ay.abs() < 1e-3, "the anchor stays");
    }

    #[test]
    fn a_resize_never_flips_or_collapses_the_selection() {
        let t = resize_step((0.0, 0.0), (40.0, 20.0), (-100.0, -100.0));
        let (x, y) = t.apply(40.0, 20.0);
        assert!(x > 0.0 && y > 0.0);
        let flat = resize_step((0.0, 0.0), (40.0, 0.5), (80.0, 9.0));
        let (_, y) = flat.apply(0.0, 10.0);
        assert!((y - 10.0).abs() < 1e-3, "an axis with no extent keeps its size");
    }

    #[test]
    fn rotating_carries_the_pointer_round_the_centre() {
        let t = rotate_step((0.0, 0.0), (10.0, 0.0), (0.0, 10.0));
        let (x, y) = t.apply(10.0, 0.0);
        assert!(x.abs() < 1e-3 && (y - 10.0).abs() < 1e-3);
        let (cx, cy) = t.apply(0.0, 0.0);
        assert!(cx.abs() < 1e-3 && cy.abs() < 1e-3, "the centre stays");
    }

    #[test]
    fn snap_rounds_to_the_nearest_grid_line() {
        assert_eq!(snap(13.0, 10.0), 10.0);
        assert_eq!(snap(16.0, 10.0), 20.0);
        assert_eq!(snap(-4.0, 10.0), 0.0);
        assert_eq!(snap(13.0, 0.0), 13.0, "no grid: unchanged");
    }
}
