//! Window tiling: the frames a window takes when it is put into a half or a quarter of the usable area, the edge and
//! corner zones a window drag snaps into, and the arrow-key layouts (Fn+Ctrl+arrows). Pure geometry, no Smithay types.

use crate::wm::Rect;

/// A tiled frame inside the usable area. `Maximize` fills the whole area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Maximize,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The arrow keys of the tiling shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// How close (px) the pointer must be to an edge to snap into it while dragging.
pub const EDGE: i32 = 8;
/// How close (px) to two edges at once the pointer must be to snap into a corner quarter.
pub const CORNER: i32 = 64;

/// The frame of `layout` inside `area`. Halves split exactly; the odd pixel goes to the right or bottom frame.
pub fn frame(area: Rect, layout: Layout) -> Rect {
    let hw = area.w / 2;
    let hh = area.h / 2;
    let rw = area.w - hw;
    let rh = area.h - hh;
    let (x, y) = (area.x, area.y);
    match layout {
        Layout::Maximize => area,
        Layout::Left => Rect::new(x, y, hw, area.h),
        Layout::Right => Rect::new(x + hw, y, rw, area.h),
        Layout::Top => Rect::new(x, y, area.w, hh),
        Layout::Bottom => Rect::new(x, y + hh, area.w, rh),
        Layout::TopLeft => Rect::new(x, y, hw, hh),
        Layout::TopRight => Rect::new(x + hw, y, rw, hh),
        Layout::BottomLeft => Rect::new(x, y + hh, hw, rh),
        Layout::BottomRight => Rect::new(x + hw, y + hh, rw, rh),
    }
}

/// The layout an arrow key gives: Left and Right tile to that half, Up maximizes, and Down returns the window to its
/// own size (no tiling, so `None`).
pub fn for_arrow(arrow: Arrow) -> Option<Layout> {
    match arrow {
        Arrow::Left => Some(Layout::Left),
        Arrow::Right => Some(Layout::Right),
        Arrow::Up => Some(Layout::Maximize),
        Arrow::Down => None,
    }
}

/// The layout a window dragged to the pointer (`x`, `y`) snaps into, if any. Within `corner` px of two edges is a
/// corner quarter; within `edge` px of the top is maximize, of the left or right is that half. The bottom edge alone
/// snaps to nothing. The pointer is clamped vertically to the area, so a drag up past the menu bar still counts as the
/// top edge; a pointer outside the area's columns is not a drop zone.
pub fn drop_zone(area: Rect, x: i32, y: i32, edge: i32, corner: i32) -> Option<Layout> {
    if area.w <= 0 || area.h <= 0 || x < area.x || x >= area.x + area.w {
        return None;
    }
    let y = y.clamp(area.y, area.y + area.h - 1);
    let dl = x - area.x;
    let dr = area.x + area.w - 1 - x;
    let dt = y - area.y;
    let db = area.y + area.h - 1 - y;
    if dt < edge && dl < corner {
        Some(Layout::TopLeft)
    } else if dt < edge && dr < corner {
        Some(Layout::TopRight)
    } else if db < edge && dl < corner {
        Some(Layout::BottomLeft)
    } else if db < edge && dr < corner {
        Some(Layout::BottomRight)
    } else if dt < edge {
        Some(Layout::Maximize)
    } else if dl < edge {
        Some(Layout::Left)
    } else if dr < edge {
        Some(Layout::Right)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 28, 1921, 1052);

    #[test]
    fn halves_and_quarters_tile_the_area_with_the_odd_pixel_on_the_right_and_bottom() {
        assert_eq!(frame(AREA, Layout::Left), Rect::new(0, 28, 960, 1052));
        assert_eq!(frame(AREA, Layout::Right), Rect::new(960, 28, 961, 1052));
        assert_eq!(frame(AREA, Layout::Top), Rect::new(0, 28, 1921, 526));
        assert_eq!(frame(AREA, Layout::Bottom), Rect::new(0, 554, 1921, 526));
        assert_eq!(frame(AREA, Layout::TopLeft), Rect::new(0, 28, 960, 526));
        assert_eq!(frame(AREA, Layout::BottomRight), Rect::new(960, 554, 961, 526));
        assert_eq!(frame(AREA, Layout::Maximize), AREA);
    }

    #[test]
    fn the_four_quarters_cover_the_area_exactly() {
        let parts = [Layout::TopLeft, Layout::TopRight, Layout::BottomLeft, Layout::BottomRight].map(|l| frame(AREA, l));
        let area: i32 = parts.iter().map(|r| r.w * r.h).sum();
        assert_eq!(area, AREA.w * AREA.h);
        assert_eq!(parts.iter().map(|r| r.x + r.w).max(), Some(AREA.x + AREA.w));
    }

    #[test]
    fn arrow_keys_give_halves_maximize_or_nothing() {
        assert_eq!(for_arrow(Arrow::Left), Some(Layout::Left));
        assert_eq!(for_arrow(Arrow::Right), Some(Layout::Right));
        assert_eq!(for_arrow(Arrow::Up), Some(Layout::Maximize));
        assert_eq!(for_arrow(Arrow::Down), None);
    }

    #[test]
    fn dragging_to_an_edge_or_a_corner_picks_its_layout() {
        let a = Rect::new(0, 28, 1920, 1052);
        assert_eq!(drop_zone(a, 0, 28, EDGE, CORNER), Some(Layout::TopLeft));
        assert_eq!(drop_zone(a, 1919, 28, EDGE, CORNER), Some(Layout::TopRight));
        assert_eq!(drop_zone(a, 1915, 1079, EDGE, CORNER), Some(Layout::BottomRight));
        assert_eq!(drop_zone(a, 960, 30, EDGE, CORNER), Some(Layout::Maximize));
        assert_eq!(drop_zone(a, 2, 500, EDGE, CORNER), Some(Layout::Left));
        assert_eq!(drop_zone(a, 1917, 500, EDGE, CORNER), Some(Layout::Right));
        assert_eq!(drop_zone(a, 960, 1079, EDGE, CORNER), None, "the bottom edge alone snaps to nothing");
        assert_eq!(drop_zone(a, 960, 500, EDGE, CORNER), None);
    }

    #[test]
    fn a_drag_above_the_menu_bar_still_counts_as_the_top_edge() {
        let a = Rect::new(0, 28, 1920, 1052);
        assert_eq!(drop_zone(a, 960, 0, EDGE, CORNER), Some(Layout::Maximize));
        assert_eq!(drop_zone(a, 5000, 0, EDGE, CORNER), None, "outside the columns of the area");
    }
}
