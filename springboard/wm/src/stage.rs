//! The focus strip: the window in use fills a large frame on the right, and the windows used before it sit as small
//! thumbnails in a column on the left, most recent first. Using a window in the strip brings it to the large frame.
//! Pure model with no Smithay types; the shell draws the frames it returns.

use crate::wm::{Rect, WindowId};

/// Width of a strip thumbnail, px.
pub const THUMB_W: i32 = 168;
/// Height of a strip thumbnail, px (the 16:10 of the thumbnail, not of the window).
pub const THUMB_H: i32 = 105;
/// Space between thumbnails and from the edges, px.
pub const GAP: i32 = 12;

/// Most recent first; the first id is the window in the large frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusStrip {
    order: Vec<WindowId>,
}

impl FocusStrip {
    pub fn new() -> Self { Self::default() }

    /// The window was used: it becomes the one in the large frame, and the rest move down the strip.
    pub fn touch(&mut self, id: WindowId) {
        self.order.retain(|w| *w != id);
        self.order.insert(0, id);
    }

    /// The window closed.
    pub fn forget(&mut self, id: WindowId) { self.order.retain(|w| *w != id); }

    /// The window in the large frame.
    pub fn current(&self) -> Option<WindowId> { self.order.first().copied() }

    /// The windows in the strip, most recent first (every window but the current one).
    pub fn strip_ids(&self) -> &[WindowId] { self.order.get(1..).unwrap_or_default() }

    /// Thumbnail frames down the left edge of `area`, most recent first. A thumbnail that does not fit in the height is
    /// left out; the shell can scroll or count it.
    pub fn thumbnails(&self, area: Rect) -> Vec<(WindowId, Rect)> {
        let bottom = area.y + area.h - GAP;
        let mut y = area.y + GAP;
        let mut out = Vec::new();
        for &id in self.strip_ids() {
            if y + THUMB_H > bottom {
                break;
            }
            out.push((id, Rect::new(area.x + GAP, y, THUMB_W, THUMB_H)));
            y += THUMB_H + GAP;
        }
        out
    }

    /// The large frame of the current window: `area` with the strip column cut off the left. Never narrower than 1 px.
    pub fn stage_frame(area: Rect) -> Rect {
        let x = area.x + THUMB_W + 2 * GAP;
        Rect::new(x, area.y, (area.x + area.w - x).max(1), area.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 28, 1920, 1052);

    #[test]
    fn the_used_window_takes_the_large_frame_and_the_others_line_up_in_the_strip() {
        let mut s = FocusStrip::new();
        s.touch(1);
        s.touch(2);
        s.touch(3);
        assert_eq!(s.current(), Some(3));
        assert_eq!(s.strip_ids(), &[2, 1], "most recent first");
        s.touch(1);
        assert_eq!(s.current(), Some(1));
        assert_eq!(s.strip_ids(), &[3, 2]);
    }

    #[test]
    fn strip_thumbnails_stack_down_the_left_edge_with_gaps() {
        let mut s = FocusStrip::new();
        for id in 1..=3 {
            s.touch(id);
        }
        let t = s.thumbnails(AREA);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], (2, Rect::new(GAP, 28 + GAP, THUMB_W, THUMB_H)));
        assert_eq!(t[1].1.y, 28 + GAP + THUMB_H + GAP);
    }

    #[test]
    fn thumbnails_that_do_not_fit_the_height_are_left_out() {
        let mut s = FocusStrip::new();
        for id in 1..=40 {
            s.touch(id);
        }
        let t = s.thumbnails(AREA);
        assert!(t.len() < 39);
        let last = t.last().unwrap().1;
        assert!(last.y + last.h <= AREA.y + AREA.h - GAP);
    }

    #[test]
    fn the_large_frame_sits_right_of_the_strip_column() {
        let f = FocusStrip::stage_frame(AREA);
        assert_eq!(f.x, THUMB_W + 2 * GAP);
        assert_eq!((f.y, f.h), (AREA.y, AREA.h));
        assert_eq!(f.x + f.w, AREA.x + AREA.w);
        assert_eq!(FocusStrip::stage_frame(Rect::new(0, 0, 10, 10)).w, 1, "never narrower than a pixel");
    }

    #[test]
    fn a_closed_window_leaves_the_strip() {
        let mut s = FocusStrip::new();
        s.touch(1);
        s.touch(2);
        s.forget(2);
        assert_eq!(s.current(), Some(1));
        assert!(s.strip_ids().is_empty());
    }
}
