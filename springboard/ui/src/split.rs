//! A two-pane split view with a draggable divider (`NativeSplitView`) and a popover placer (`NativePopover`).
use crate::geometry::Edge;
use crate::Rect;

/// Leading pane / trailing pane along the x axis. `position` is the width of the leading pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitView {
    position: f32,
    total: f32,
    min_a: f32,
    min_b: f32,
    collapsed: bool,
}

impl SplitView {
    pub fn new(total: f32, position: f32, min_a: f32, min_b: f32) -> Self {
        let mut s = SplitView { position, total: total.max(0.0), min_a, min_b, collapsed: false };
        s.position = s.clamp(position);
        s
    }
    fn clamp(&self, p: f32) -> f32 {
        let hi = (self.total - self.min_b).max(self.min_a.min(self.total));
        p.clamp(self.min_a.min(self.total), hi)
    }
    /// Width of the leading pane now (0 when collapsed).
    pub fn leading(&self) -> f32 {
        if self.collapsed { 0.0 } else { self.position }
    }
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
    /// The window was resized: the leading pane keeps its width as far as the minimums allow.
    pub fn set_total(&mut self, total: f32) {
        self.total = total.max(0.0);
        self.position = self.clamp(self.position);
    }
    /// The divider is dragged to `x`; the position respects both minimums. Dragging reopens a collapsed pane.
    pub fn drag_to(&mut self, x: f32) {
        self.collapsed = false;
        self.position = self.clamp(x);
    }
    /// Hides or shows the leading pane (toggling a sidebar).
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }
    /// Shows a hidden leading pane or hides a shown one (`NativeSplitViewController.toggleSidebar(_:)`); returns
    /// whether it is now collapsed.
    pub fn toggle(&mut self) -> bool {
        self.collapsed = !self.collapsed;
        self.collapsed
    }
    /// `(leading, divider, trailing)` rectangles inside `bounds`, the divider `thickness` wide.
    pub fn rects(&self, bounds: Rect, thickness: f32) -> (Rect, Rect, Rect) {
        let lead = self.leading().min(bounds.w);
        let div = if self.collapsed { 0.0 } else { thickness.min(bounds.w - lead).max(0.0) };
        (
            Rect { w: lead, ..bounds },
            Rect { x: bounds.x + lead, w: div, ..bounds },
            Rect { x: bounds.x + lead + div, w: (bounds.w - lead - div).max(0.0), ..bounds },
        )
    }
    /// True when `x` (in bounds coordinates) is within `slop` of the divider: where the resize cursor shows.
    pub fn hits_divider(&self, x: f32, slop: f32) -> bool {
        !self.collapsed && (x - self.position).abs() <= slop
    }
}

/// Where a popover of `size` goes next to `anchor` inside `bounds` (`NativePopover.show(relativeTo:preferredEdge:)`).
/// Tries `preferred`, flips to the opposite edge when it does not fit, then picks whichever side has more room.
/// The popover is centred on the anchor and clamped to stay on screen. Returns the rectangle and the edge used.
pub fn place_popover(anchor: Rect, size: (f32, f32), bounds: Rect, preferred: Edge, gap: f32) -> (Rect, Edge) {
    let (w, h) = size;
    let room = |e: Edge| match e {
        Edge::Top => anchor.y - bounds.y,
        Edge::Bottom => bounds.bottom() - anchor.bottom(),
        Edge::Left => anchor.x - bounds.x,
        Edge::Right => bounds.right() - anchor.right(),
    };
    let need = |e: Edge| if matches!(e, Edge::Top | Edge::Bottom) { h + gap } else { w + gap };
    let edge = if room(preferred) >= need(preferred) {
        preferred
    } else if room(preferred.opposite()) >= need(preferred.opposite()) {
        preferred.opposite()
    } else if room(preferred) >= room(preferred.opposite()) {
        preferred
    } else {
        preferred.opposite()
    };
    let (mut x, mut y) = match edge {
        Edge::Top => (anchor.x + (anchor.w - w) / 2.0, anchor.y - gap - h),
        Edge::Bottom => (anchor.x + (anchor.w - w) / 2.0, anchor.bottom() + gap),
        Edge::Left => (anchor.x - gap - w, anchor.y + (anchor.h - h) / 2.0),
        Edge::Right => (anchor.right() + gap, anchor.y + (anchor.h - h) / 2.0),
    };
    x = x.min(bounds.right() - w).max(bounds.x);
    y = y.min(bounds.bottom() - h).max(bounds.y);
    (Rect { x, y, w, h }, edge)
}

/// The slice of `count` rows of `row_h` that must exist for a viewport at `offset` of height `viewport`, plus
/// `overscan` rows each side: the window a table keeps cells for (`NativeTableView` cell reuse).
pub fn visible_rows(offset: f32, viewport: f32, row_h: f32, count: usize, overscan: usize) -> std::ops::Range<usize> {
    if row_h <= 0.0 || count == 0 || viewport <= 0.0 {
        return 0..0;
    }
    let first = (offset.max(0.0) / row_h).floor() as usize;
    let last = ((offset.max(0.0) + viewport) / row_h).ceil() as usize;
    first.saturating_sub(overscan).min(count)..(last + overscan).min(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    const B: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };

    #[test]
    fn dragging_respects_both_minimums() {
        let mut s = SplitView::new(800.0, 250.0, 180.0, 300.0);
        s.drag_to(50.0);
        assert_eq!(s.leading(), 180.0);
        s.drag_to(790.0);
        assert_eq!(s.leading(), 500.0, "the trailing pane keeps its minimum");
    }

    #[test]
    fn resizing_keeps_the_sidebar_width_until_the_minimums_collide() {
        let mut s = SplitView::new(800.0, 250.0, 180.0, 300.0);
        s.set_total(1200.0);
        assert_eq!(s.leading(), 250.0);
        s.set_total(400.0);
        assert_eq!(s.leading(), 180.0, "min_a wins when the window is too small for both");
    }

    #[test]
    fn toggling_hides_and_shows_the_sidebar_at_its_old_width() {
        let mut s = SplitView::new(800.0, 220.0, 180.0, 300.0);
        assert!(s.toggle());
        assert_eq!(s.leading(), 0.0);
        assert!(!s.toggle());
        assert_eq!(s.leading(), 220.0);
    }

    #[test]
    fn rects_tile_the_bounds_and_collapse_gives_everything_to_the_trailing_pane() {
        let mut s = SplitView::new(800.0, 250.0, 180.0, 300.0);
        let (a, d, b) = s.rects(B, 1.0);
        assert_eq!((a.w, d.w, b.w), (250.0, 1.0, 549.0));
        assert_eq!(b.right(), 800.0);
        s.set_collapsed(true);
        let (a, d, b) = s.rects(B, 1.0);
        assert_eq!((a.w, d.w, b.w), (0.0, 0.0, 800.0));
        assert!(!s.hits_divider(250.0, 4.0));
        s.drag_to(300.0);
        assert!(!s.is_collapsed() && s.hits_divider(302.0, 4.0));
    }

    #[test]
    fn popovers_flip_when_the_preferred_edge_has_no_room_and_stay_on_screen() {
        let anchor = Rect { x: 100.0, y: 10.0, w: 40.0, h: 20.0 };
        let (r, e) = place_popover(anchor, (200.0, 100.0), B, Edge::Top, 6.0);
        assert_eq!(e, Edge::Bottom, "no room above, so it opens below");
        assert_eq!(r.y, 36.0);
        let (r, e) = place_popover(anchor, (200.0, 100.0), B, Edge::Bottom, 6.0);
        assert_eq!(e, Edge::Bottom);
        assert!(r.x >= 0.0 && r.right() <= 800.0);
        let near_edge = Rect { x: 790.0, y: 300.0, w: 10.0, h: 10.0 };
        let (r, _) = place_popover(near_edge, (200.0, 100.0), B, Edge::Bottom, 6.0);
        assert!(r.right() <= 800.0 && r.x >= 0.0, "clamped horizontally: {r:?}");
    }

    #[test]
    fn visible_rows_cover_the_viewport_with_overscan_and_clamp_to_the_list() {
        assert_eq!(visible_rows(0.0, 100.0, 20.0, 1000, 0), 0..5);
        assert_eq!(visible_rows(50.0, 100.0, 20.0, 1000, 2), 0..10);
        assert_eq!(visible_rows(990.0 * 20.0, 100.0, 20.0, 1000, 3), 987..998);
        assert_eq!(visible_rows(999.0 * 20.0, 100.0, 20.0, 1000, 3), 996..1000, "clamped to the list end");
        assert_eq!(visible_rows(-30.0, 100.0, 20.0, 1000, 0), 0..5, "rubber-band overscroll");
        assert_eq!(visible_rows(0.0, 100.0, 20.0, 0, 2), 0..0);
        assert_eq!(visible_rows(0.0, 100.0, 0.0, 10, 2), 0..0);
    }
}
