//! Clip rectangles for drawing. A `ClipStack` holds the current clip rectangle and a stack of saved
//! ones, so a caller can narrow the clip, draw, and go back to the earlier clip. Every `clip_to`
//! intersects the new rectangle with the current clip, so a clip never grows back on its own.
//! Pure: a renderer asks `contains` or `clip_span` before it writes a pixel.

/// An integer rectangle (x, y, w, h). A width or height at or below zero is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The overlap of two rectangles. Disjoint rectangles give an empty rectangle at the
    /// top-left corner of the overlap.
    pub fn intersect(self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.x.saturating_add(self.w.max(0)).min(o.x.saturating_add(o.w.max(0)));
        let y1 = self.y.saturating_add(self.h.max(0)).min(o.y.saturating_add(o.h.max(0)));
        if x1 <= x0 || y1 <= y0 {
            return Rect::new(x0, y0, 0, 0);
        }
        Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }
}

/// The current clip and the clips saved by `save`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipStack {
    current: Rect,
    saved: Vec<Rect>,
}

impl ClipStack {
    /// A stack whose clip starts as `frame`, usually the whole surface.
    pub fn new(frame: Rect) -> Self {
        ClipStack { current: frame, saved: Vec::new() }
    }

    /// The clip in effect.
    pub fn clip(&self) -> Rect {
        self.current
    }

    /// Narrows the clip to its overlap with `r`. The clip never grows.
    pub fn clip_to(&mut self, r: Rect) {
        self.current = self.current.intersect(r);
    }

    /// Saves the current clip so a later `restore` can bring it back.
    pub fn save(&mut self) {
        self.saved.push(self.current);
    }

    /// Restores the most recently saved clip. Returns false when nothing was saved.
    pub fn restore(&mut self) -> bool {
        match self.saved.pop() {
            Some(r) => {
                self.current = r;
                true
            }
            None => false,
        }
    }

    /// Whether the pixel (x, y) lies inside the clip.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let c = self.current;
        !c.is_empty() && x >= c.x && x < c.x.saturating_add(c.w) && y >= c.y && y < c.y.saturating_add(c.h)
    }

    /// The part of the horizontal run starting at `x` with width `w` that lies inside the clip,
    /// as (start, width). `None` when nothing is left.
    pub fn clip_span(&self, x: i32, w: i32) -> Option<(i32, i32)> {
        let c = self.current;
        if c.is_empty() || w <= 0 {
            return None;
        }
        let x0 = x.max(c.x);
        let x1 = x.saturating_add(w).min(c.x.saturating_add(c.w));
        if x1 <= x0 {
            None
        } else {
            Some((x0, x1 - x0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_to_intersects_and_never_grows() {
        let mut s = ClipStack::new(Rect::new(0, 0, 100, 50));
        s.clip_to(Rect::new(10, 10, 20, 20));
        assert_eq!(s.clip(), Rect::new(10, 10, 20, 20));
        // Overlap of (10,10,20,20) and (0,0,15,15) is (10,10,5,5).
        s.clip_to(Rect::new(0, 0, 15, 15));
        assert_eq!(s.clip(), Rect::new(10, 10, 5, 5));
        // A rectangle that does not overlap leaves the clip empty.
        s.clip_to(Rect::new(50, 50, 5, 5));
        assert!(s.clip().is_empty());
    }

    #[test]
    fn save_and_restore_bring_back_the_earlier_clip() {
        let mut s = ClipStack::new(Rect::new(0, 0, 100, 50));
        s.clip_to(Rect::new(10, 10, 20, 20));
        s.save();
        s.clip_to(Rect::new(0, 0, 15, 15));
        assert_eq!(s.clip(), Rect::new(10, 10, 5, 5));
        assert!(s.restore());
        assert_eq!(s.clip(), Rect::new(10, 10, 20, 20));
        assert!(!s.restore());
    }

    #[test]
    fn contains_excludes_the_right_and_bottom_edges() {
        let mut s = ClipStack::new(Rect::new(0, 0, 100, 50));
        s.clip_to(Rect::new(10, 10, 5, 5));
        assert!(s.contains(10, 10));
        assert!(s.contains(14, 14));
        // x = 15 and y = 15 are one past the right and bottom edges of (10,10,5,5).
        assert!(!s.contains(15, 12));
        assert!(!s.contains(12, 15));
        assert!(!s.contains(9, 12));
    }

    #[test]
    fn clip_span_trims_a_horizontal_run() {
        let mut s = ClipStack::new(Rect::new(0, 0, 100, 50));
        s.clip_to(Rect::new(10, 10, 5, 5));
        // Run 0..100 is cut to 10..15.
        assert_eq!(s.clip_span(0, 100), Some((10, 5)));
        // Run 12..17 is cut to 12..15.
        assert_eq!(s.clip_span(12, 5), Some((12, 3)));
        // Run 20..30 lies outside.
        assert_eq!(s.clip_span(20, 10), None);
        assert_eq!(s.clip_span(0, 0), None);
    }

    #[test]
    fn intersect_of_disjoint_rectangles_is_empty() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(20, 20, 5, 5);
        assert!(a.intersect(b).is_empty());
        // Touching edges share no pixel.
        assert!(a.intersect(Rect::new(10, 0, 5, 5)).is_empty());
    }
}
