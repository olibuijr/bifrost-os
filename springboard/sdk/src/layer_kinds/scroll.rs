//! Scroll content: which axes may scroll, and the offset of a viewport over larger content, with "scroll to point" and
//! "scroll to rect" (the smallest move that brings a rect into view). `ScrollLayerView` drives this with touch input.
use bifrost_ui::Rect;

/// The axes the content may be scrolled along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollMode {
    /// The offset never changes.
    None,
    #[default]
    Vertically,
    Horizontally,
    Both,
}

impl ScrollMode {
    pub fn horizontal(self) -> bool {
        matches!(self, ScrollMode::Horizontally | ScrollMode::Both)
    }
    pub fn vertical(self) -> bool {
        matches!(self, ScrollMode::Vertically | ScrollMode::Both)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollContent {
    pub mode: ScrollMode,
    content: (f32, f32),
    viewport: (f32, f32),
    offset: (f32, f32),
}

impl ScrollContent {
    pub fn new(mode: ScrollMode) -> Self {
        ScrollContent { mode, content: (0.0, 0.0), viewport: (0.0, 0.0), offset: (0.0, 0.0) }
    }

    /// Sets the content and viewport sizes and keeps the offset inside the new range.
    pub fn set_sizes(&mut self, content: (f32, f32), viewport: (f32, f32)) {
        self.content = content;
        self.viewport = viewport;
        self.offset = self.clamped(self.offset);
    }

    pub fn content_size(&self) -> (f32, f32) {
        self.content
    }

    pub fn viewport_size(&self) -> (f32, f32) {
        self.viewport
    }

    /// The largest offset on each axis: content minus viewport, never negative.
    pub fn max_offset(&self) -> (f32, f32) {
        ((self.content.0 - self.viewport.0).max(0.0), (self.content.1 - self.viewport.1).max(0.0))
    }

    /// The content point at the viewport's top left.
    pub fn offset(&self) -> (f32, f32) {
        self.offset
    }

    /// The part of the content in view.
    pub fn visible(&self) -> Rect {
        Rect { x: self.offset.0, y: self.offset.1, w: self.viewport.0, h: self.viewport.1 }
    }

    /// `p` brought inside the scroll range; an axis the mode locks keeps the current offset.
    fn clamped(&self, p: (f32, f32)) -> (f32, f32) {
        let max = self.max_offset();
        let x = if self.mode.horizontal() { p.0.clamp(0.0, max.0) } else { self.offset.0.clamp(0.0, max.0) };
        let y = if self.mode.vertical() { p.1.clamp(0.0, max.1) } else { self.offset.1.clamp(0.0, max.1) };
        (x, y)
    }

    /// Scrolls so the content point `p` is at the viewport's top left, as far as the mode and the content allow.
    pub fn scroll_to_point(&mut self, p: (f32, f32)) {
        self.offset = self.clamped(p);
    }

    /// Scrolls the least that brings `r` fully into view. A rect larger than the viewport is aligned by its top left.
    pub fn scroll_to_rect(&mut self, r: Rect) {
        let fit = |start: f32, len: f32, view: f32, cur: f32| {
            if len >= view || start < cur {
                start
            } else if start + len > cur + view {
                start + len - view
            } else {
                cur
            }
        };
        let target = (fit(r.x, r.w, self.viewport.0, self.offset.0), fit(r.y, r.h, self.viewport.1, self.offset.1));
        self.offset = self.clamped(target);
    }

    /// Moves by `d`, clamped.
    pub fn scroll_by(&mut self, d: (f32, f32)) {
        self.offset = self.clamped((self.offset.0 + d.0, self.offset.1 + d.1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(mode: ScrollMode) -> ScrollContent {
        let mut s = ScrollContent::new(mode);
        s.set_sizes((500.0, 400.0), (100.0, 100.0));
        s
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn scroll_to_point_clamps_to_the_range() {
        let mut s = content(ScrollMode::Both);
        s.scroll_to_point((50.0, 20.0));
        assert_eq!(s.offset(), (50.0, 20.0));
        s.scroll_to_point((9999.0, -5.0));
        assert_eq!(s.offset(), (400.0, 0.0));
    }

    #[test]
    fn the_mode_locks_axes() {
        let mut v = content(ScrollMode::Vertically);
        v.scroll_to_point((50.0, 60.0));
        assert_eq!(v.offset(), (0.0, 60.0));
        let mut h = content(ScrollMode::Horizontally);
        h.scroll_to_point((50.0, 60.0));
        assert_eq!(h.offset(), (50.0, 0.0));
        let mut n = content(ScrollMode::None);
        n.scroll_to_point((50.0, 60.0));
        assert_eq!(n.offset(), (0.0, 0.0));
    }

    #[test]
    fn scroll_to_rect_moves_the_least() {
        let mut s = content(ScrollMode::Both);
        s.scroll_to_rect(r(10.0, 10.0, 20.0, 20.0));
        assert_eq!(s.offset(), (0.0, 0.0), "already visible");
        s.scroll_to_rect(r(150.0, 10.0, 20.0, 20.0));
        assert_eq!(s.offset(), (70.0, 0.0), "its right edge lands on the viewport's right edge");
        s.scroll_to_rect(r(20.0, 300.0, 20.0, 20.0));
        assert_eq!(s.offset(), (20.0, 220.0), "scrolls back left to its left edge, and down");
    }

    #[test]
    fn a_rect_larger_than_the_viewport_aligns_by_its_origin() {
        let mut s = content(ScrollMode::Both);
        s.scroll_to_rect(r(200.0, 100.0, 250.0, 250.0));
        assert_eq!(s.offset(), (200.0, 100.0));
    }

    #[test]
    fn resizing_keeps_the_offset_in_range() {
        let mut s = content(ScrollMode::Both);
        s.scroll_to_point((400.0, 300.0));
        s.set_sizes((200.0, 200.0), (100.0, 100.0));
        assert_eq!(s.offset(), (100.0, 100.0));
        s.set_sizes((50.0, 50.0), (100.0, 100.0));
        assert_eq!(s.offset(), (0.0, 0.0));
        assert_eq!(s.max_offset(), (0.0, 0.0));
    }

    #[test]
    fn visible_is_the_viewport_at_the_offset() {
        let mut s = content(ScrollMode::Both);
        s.scroll_by((30.0, 40.0));
        assert_eq!(s.visible(), r(30.0, 40.0, 100.0, 100.0));
    }
}
