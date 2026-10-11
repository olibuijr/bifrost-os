//! Stack and grid layout (`NativeStackView`, `NativeStackView`, `NativeGridView`) as pure functions from a size to rectangles.
use crate::geometry::EdgeInsets;
use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// One arranged subview: its preferred length along the axis and how it takes part in spare space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StackItem {
    /// Preferred length along the stack axis.
    pub size: f32,
    /// Share of spare (or missing) space; 0 keeps the item at `size` (content hugging).
    pub flex: f32,
}

impl StackItem {
    pub fn fixed(size: f32) -> Self {
        StackItem { size, flex: 0.0 }
    }
    pub fn flexible(size: f32, flex: f32) -> Self {
        StackItem { size, flex }
    }
}

/// Lays `items` along `axis` inside `bounds` minus `insets`, `spacing` apart. Each item gets the full cross-axis extent.
/// Spare space goes to flexible items by weight; a shortfall is taken from them (never below 0), and if there is
/// still too little the fixed items shrink proportionally, so nothing leaves the bounds.
pub fn stack(bounds: Rect, axis: Axis, spacing: f32, insets: EdgeInsets, items: &[StackItem]) -> Vec<Rect> {
    let area = bounds.inset(insets);
    let main = if axis == Axis::Horizontal { area.w } else { area.h };
    let gaps = spacing * items.len().saturating_sub(1) as f32;
    let avail = (main - gaps).max(0.0);
    let mut sizes: Vec<f32> = items.iter().map(|i| i.size.max(0.0)).collect();
    let total: f32 = sizes.iter().sum();
    let weight: f32 = items.iter().map(|i| i.flex.max(0.0)).sum();
    let mut diff = avail - total;
    if weight > 0.0 {
        if diff >= 0.0 {
            for (s, it) in sizes.iter_mut().zip(items) {
                *s += diff * it.flex.max(0.0) / weight;
            }
            diff = 0.0;
        } else {
            let mut left = -diff;
            // Shrink flexible items by weight, never below zero; repeat while some still have room.
            for _ in 0..items.len() {
                let live: f32 = sizes.iter().zip(items).filter(|(s, it)| it.flex > 0.0 && **s > 0.0).map(|(_, it)| it.flex).sum();
                if live <= 0.0 || left <= 1e-4 {
                    break;
                }
                let round = left;
                for (s, it) in sizes.iter_mut().zip(items) {
                    if it.flex > 0.0 && *s > 0.0 {
                        let take = (round * it.flex / live).min(*s);
                        *s -= take;
                        left -= take;
                    }
                }
            }
            diff = -left;
        }
    }
    if diff < -1e-4 {
        let sum: f32 = sizes.iter().sum();
        if sum > 0.0 {
            let k = avail / sum;
            sizes.iter_mut().for_each(|s| *s *= k);
        }
    }
    let mut at = if axis == Axis::Horizontal { area.x } else { area.y };
    sizes
        .into_iter()
        .map(|s| {
            let r = if axis == Axis::Horizontal { Rect { x: at, y: area.y, w: s, h: area.h } } else { Rect { x: area.x, y: at, w: area.w, h: s } };
            at += s + spacing;
            r
        })
        .collect()
}

/// `NativeStackView.Distribution`. `Fill` is the flexible layout of [`stack`]; the others are computed by [`distribute`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribution {
    /// Intrinsic lengths, the stretchable views share the spare space (hugging priority decides which).
    Fill,
    /// Every view gets the same length.
    FillEqually,
    /// Lengths proportional to the intrinsic lengths, filling the stack.
    FillProportionally,
    /// Intrinsic lengths; the gaps between views grow equally (never below `spacing`).
    EqualSpacing,
    /// Intrinsic lengths; the view centres are equally far apart.
    EqualCentering,
}

/// Places views of intrinsic `sizes` along a main axis of length `main` for the four fixed distributions. Returns
/// `(offset, length)` per view from the start of the axis. `Fill` returns the intrinsic sizes packed with `spacing`.
pub fn distribute(main: f32, spacing: f32, sizes: &[f32], d: Distribution) -> Vec<(f32, f32)> {
    let n = sizes.len();
    if n == 0 {
        return Vec::new();
    }
    let gaps = spacing * (n - 1) as f32;
    let total: f32 = sizes.iter().map(|s| s.max(0.0)).sum();
    let lens: Vec<f32> = match d {
        Distribution::FillEqually => vec![((main - gaps) / n as f32).max(0.0); n],
        Distribution::FillProportionally if total > 0.0 => sizes.iter().map(|s| s.max(0.0) / total * (main - gaps).max(0.0)).collect(),
        Distribution::FillProportionally => vec![((main - gaps) / n as f32).max(0.0); n],
        _ => sizes.iter().map(|s| s.max(0.0)).collect(),
    };
    let mut out = Vec::with_capacity(n);
    match d {
        Distribution::EqualSpacing if n > 1 => {
            let gap = ((main - total) / (n - 1) as f32).max(spacing);
            let mut at = 0.0;
            for l in lens {
                out.push((at, l));
                at += l + gap;
            }
        }
        Distribution::EqualCentering if n > 1 => {
            // Centre pitch: as wide as the space allows, never so small that neighbours come closer than `spacing`.
            let want = (main - lens[0] / 2.0 - lens[n - 1] / 2.0) / (n - 1) as f32;
            let least = lens.windows(2).map(|w| (w[0] + w[1]) / 2.0 + spacing).fold(0.0, f32::max);
            let pitch = want.max(least);
            for (i, l) in lens.iter().enumerate() {
                out.push((lens[0] / 2.0 + i as f32 * pitch - l / 2.0, *l));
            }
        }
        _ => {
            let mut at = 0.0;
            for l in lens {
                out.push((at, l));
                at += l + spacing;
            }
        }
    }
    out
}

/// Number of equal columns of at least `min_cell` that fit in `width` with `spacing` between them; at least 1
/// (adaptive columns, as a compositional layout's `fractionalWidth` group does).
pub fn adaptive_columns(width: f32, min_cell: f32, spacing: f32) -> usize {
    if min_cell <= 0.0 || width <= min_cell {
        return 1;
    }
    (((width + spacing) / (min_cell + spacing)).floor() as usize).max(1)
}

/// `count` cells in `columns` equal columns, filling rows left to right (`NativeGridView`). `row_h` is the height of each row.
pub fn grid(bounds: Rect, columns: usize, spacing_x: f32, spacing_y: f32, row_h: f32, count: usize) -> Vec<Rect> {
    let columns = columns.max(1);
    let cell_w = ((bounds.w - spacing_x * (columns - 1) as f32) / columns as f32).max(0.0);
    (0..count)
        .map(|i| {
            let (c, r) = ((i % columns) as f32, (i / columns) as f32);
            Rect { x: bounds.x + c * (cell_w + spacing_x), y: bounds.y + r * (row_h + spacing_y), w: cell_w, h: row_h }
        })
        .collect()
}

/// Where a layer sits inside an overlay (`ZStack` alignment). `Leading` and `Trailing` are the left and right edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeading,
    Top,
    TopTrailing,
    Leading,
    Center,
    Trailing,
    BottomLeading,
    Bottom,
    BottomTrailing,
}

impl Anchor {
    /// Position inside the free space as `(x, y)` fractions: 0 is the leading or top edge, 1 the trailing or bottom edge.
    pub fn unit(self) -> (f32, f32) {
        match self {
            Anchor::TopLeading => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopTrailing => (1.0, 0.0),
            Anchor::Leading => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Trailing => (1.0, 0.5),
            Anchor::BottomLeading => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomTrailing => (1.0, 1.0),
        }
    }
}

/// Lays `layers` (intrinsic `(w, h)` each) over one another inside `bounds` minus `insets`, all aligned to `anchor`
/// (`ZStack`). A layer keeps its size; one larger than the area is clamped to it, so nothing leaves the bounds.
/// Returns one rect per layer in the order given, which is back to front.
pub fn overlay(bounds: Rect, insets: EdgeInsets, anchor: Anchor, layers: &[(f32, f32)]) -> Vec<Rect> {
    let area = bounds.inset(insets);
    let (fx, fy) = anchor.unit();
    layers
        .iter()
        .map(|&(w, h)| {
            let w = w.max(0.0).min(area.w);
            let h = h.max(0.0).min(area.h);
            Rect { x: area.x + (area.w - w) * fx, y: area.y + (area.h - h) * fy, w, h }
        })
        .collect()
}

/// `ViewThatFits`: the index of the first candidate, in preference order, whose size fits `available` on the
/// constrained axes (`axes` is `(horizontal, vertical)`). `None` when no candidate fits, so the caller picks the fallback.
pub fn first_fitting(available: (f32, f32), candidates: &[(f32, f32)], axes: (bool, bool)) -> Option<usize> {
    const EPS: f32 = 1e-4;
    candidates.iter().position(|&(w, h)| (!axes.0 || w <= available.0 + EPS) && (!axes.1 || h <= available.1 + EPS))
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 100.0 };

    fn lengths(v: &[Rect]) -> Vec<f32> {
        v.iter().map(|r| r.w).collect()
    }

    #[test]
    fn spare_space_goes_to_flexible_items_by_weight() {
        let r = stack(B, Axis::Horizontal, 10.0, EdgeInsets::ZERO, &[StackItem::fixed(50.0), StackItem::flexible(0.0, 1.0), StackItem::flexible(0.0, 3.0)]);
        assert_eq!(lengths(&r), vec![50.0, 57.5, 172.5], "300 - 50 - 2 gaps of 10 = 230, split 1:3");
        assert_eq!(r[1].x, 60.0, "spacing sits between items");
        assert_eq!(r[2].right(), 300.0, "the stack fills the bounds exactly");
        assert!(r.iter().all(|x| x.y == 0.0 && x.h == 100.0), "cross axis is full");
    }

    #[test]
    fn a_shortfall_is_taken_from_flexible_items_first() {
        let r = stack(B, Axis::Horizontal, 0.0, EdgeInsets::ZERO, &[StackItem::fixed(100.0), StackItem::flexible(150.0, 1.0), StackItem::flexible(150.0, 1.0)]);
        assert_eq!(lengths(&r), vec![100.0, 100.0, 100.0]);
    }

    #[test]
    fn when_even_the_fixed_items_do_not_fit_everything_shrinks_inside_the_bounds() {
        let r = stack(B, Axis::Horizontal, 0.0, EdgeInsets::ZERO, &[StackItem::fixed(400.0), StackItem::fixed(200.0)]);
        assert!((r[1].right() - 300.0).abs() < 0.01, "{r:?}");
        assert!((r[0].w / r[1].w - 2.0).abs() < 0.01, "proportions kept");
    }

    #[test]
    fn vertical_stacks_and_insets() {
        let r = stack(B, Axis::Vertical, 4.0, EdgeInsets::all(10.0), &[StackItem::fixed(20.0), StackItem::flexible(0.0, 1.0)]);
        assert_eq!(r[0], Rect { x: 10.0, y: 10.0, w: 280.0, h: 20.0 });
        assert_eq!((r[1].y, r[1].bottom()), (34.0, 90.0));
        assert!(stack(B, Axis::Vertical, 4.0, EdgeInsets::ZERO, &[]).is_empty());
    }

    #[test]
    fn distributions_follow_nsstackview() {
        let s = [20.0, 40.0, 60.0];
        let eq = distribute(300.0, 10.0, &s, Distribution::FillEqually);
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(eq.iter().all(|x| near(x.1, 280.0 / 3.0)) && near(eq[2].0, 2.0 * (280.0 / 3.0 + 10.0)));
        let p = distribute(220.0, 10.0, &s, Distribution::FillProportionally);
        assert!(near(p[0].1, 200.0 / 6.0) && near(p[1].1, 400.0 / 6.0) && near(p[2].1, 100.0), "{p:?}");
        let sp = distribute(220.0, 10.0, &s, Distribution::EqualSpacing);
        assert!(near(sp[1].0, 70.0) && near(sp[2].0, 160.0) && near(sp[2].0 + sp[2].1, 220.0), "{sp:?}");
        let tight = distribute(100.0, 10.0, &s, Distribution::EqualSpacing);
        assert_eq!(tight[1].0, 30.0, "gaps never shrink below the spacing: 20 + 10");
    }

    #[test]
    fn equal_centering_spaces_the_centres_equally() {
        let r = distribute(300.0, 0.0, &[20.0, 60.0, 20.0], Distribution::EqualCentering);
        let centres: Vec<f32> = r.iter().map(|&(o, l)| o + l / 2.0).collect();
        assert_eq!(centres, vec![10.0, 150.0, 290.0]);
        let packed = distribute(50.0, 5.0, &[20.0, 60.0, 20.0], Distribution::EqualCentering);
        assert_eq!(packed[1].0 - (packed[0].0 + 20.0), 5.0, "neighbours stay `spacing` apart when the space runs out");
        assert_eq!(distribute(100.0, 4.0, &[30.0], Distribution::EqualSpacing), vec![(0.0, 30.0)]);
        assert!(distribute(100.0, 4.0, &[], Distribution::FillEqually).is_empty());
    }

    #[test]
    fn adaptive_columns_grow_with_width_and_never_reach_zero() {
        assert_eq!(adaptive_columns(100.0, 200.0, 8.0), 1);
        assert_eq!(adaptive_columns(0.0, 200.0, 8.0), 1);
        assert_eq!(adaptive_columns(416.0, 200.0, 16.0), 2);
        assert_eq!(adaptive_columns(415.0, 200.0, 16.0), 1);
        assert!(adaptive_columns(3440.0, 200.0, 16.0) > adaptive_columns(1440.0, 200.0, 16.0));
        assert_eq!(adaptive_columns(500.0, 0.0, 8.0), 1);
    }

    #[test]
    fn grid_fills_rows_left_to_right_inside_the_bounds() {
        let cells = grid(B, 3, 10.0, 6.0, 20.0, 7);
        assert_eq!(cells.len(), 7);
        assert!(cells.iter().all(|c| c.right() <= B.right() + 0.001));
        assert_eq!(cells[3].x, cells[0].x, "the fourth cell wraps to column 0");
        assert_eq!(cells[3].y, 26.0);
        assert_eq!(cells[2].right(), 300.0);
        assert_eq!(grid(B, 0, 0.0, 0.0, 10.0, 2).len(), 2, "zero columns is treated as one");
    }

    #[test]
    fn overlay_aligns_every_layer_to_the_anchor() {
        let r = overlay(B, EdgeInsets::ZERO, Anchor::Center, &[(100.0, 40.0), (300.0, 100.0)]);
        assert_eq!(r[0], Rect { x: 100.0, y: 30.0, w: 100.0, h: 40.0 }, "centred");
        assert_eq!(r[1], B, "a layer that fills the bounds covers them exactly");
        let br = overlay(B, EdgeInsets::all(10.0), Anchor::BottomTrailing, &[(50.0, 20.0)]);
        assert_eq!(br[0], Rect { x: 240.0, y: 70.0, w: 50.0, h: 20.0 }, "inset area, bottom trailing corner");
        let tl = overlay(B, EdgeInsets::ZERO, Anchor::TopLeading, &[(20.0, 10.0)]);
        assert_eq!((tl[0].x, tl[0].y), (0.0, 0.0));
    }

    #[test]
    fn overlay_clamps_oversized_layers_inside_the_area() {
        let r = overlay(B, EdgeInsets::ZERO, Anchor::Trailing, &[(500.0, 500.0), (-5.0, 10.0)]);
        assert_eq!(r[0], B);
        assert_eq!((r[1].w, r[1].x), (0.0, 300.0), "a negative size collapses to zero at the trailing edge");
        assert!(r.iter().all(|x| x.right() <= 300.0 + 1e-3 && x.bottom() <= 100.0 + 1e-3));
    }

    #[test]
    fn first_fitting_takes_the_first_candidate_in_preference_order() {
        let wide_hstack = (320.0, 24.0);
        let progress = (100.0, 24.0);
        let label = (40.0, 14.0);
        let cands = [wide_hstack, progress, label];
        assert_eq!(first_fitting((300.0, 50.0), &cands, (true, true)), Some(1), "the 320-point row is too wide");
        assert_eq!(first_fitting((30.0, 50.0), &cands, (true, true)), None, "nothing fits 30 points wide");
        assert_eq!(first_fitting((300.0, 16.0), &cands, (true, true)), Some(2), "the 16-point height rejects the first two");
    }

    #[test]
    fn first_fitting_ignores_axes_that_are_not_constrained() {
        let cands = [(500.0, 20.0), (50.0, 900.0)];
        assert_eq!(first_fitting((100.0, 40.0), &cands, (true, false)), Some(1), "height is free, so the tall candidate fits");
        assert_eq!(first_fitting((100.0, 40.0), &cands, (false, true)), Some(0), "width is free, so the wide candidate fits");
        assert_eq!(first_fitting((100.0, 40.0), &[], (true, true)), None);
    }
}
