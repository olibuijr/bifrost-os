//! Clipping: the clip area of a drawing context as a pure model. Each clip call intersects the current clip with a
//! rectangle, so a shape can only paint inside every rectangle added since the context was last reset. Save and restore
//! keep the clip with the rest of the drawing state, and a test tells a painter whether a rectangle can paint at all.
//! The model holds no pixels; a surface asks it before it draws. Edges are half-open: a point on the right or bottom
//! edge lies outside.

use crate::Rect;

/// The clip area in force.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clip {
    /// Nothing clips: the whole surface paints.
    Unclipped,
    /// Only this rectangle paints.
    Bounded(Rect),
    /// Two clip rectangles did not overlap, so nothing paints.
    Empty,
}

/// The clip area and its save and restore stack.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipState {
    current: Clip,
    saved: Vec<Clip>,
}

impl Default for ClipState {
    fn default() -> Self {
        Self::new()
    }
}

/// The overlap of two rectangles, or `None` when they share no area. A rectangle with a zero or negative side is empty.
fn overlap(a: Rect, b: Rect) -> Option<Rect> {
    if !(a.w > 0.0 && a.h > 0.0 && b.w > 0.0 && b.h > 0.0) {
        return None;
    }
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

impl ClipState {
    /// An unclipped context with an empty save stack.
    pub fn new() -> Self {
        ClipState { current: Clip::Unclipped, saved: Vec::new() }
    }

    /// The clip area in force.
    pub fn clip(&self) -> Clip {
        self.current
    }

    /// Intersects the clip with `rect`. Clipping an unclipped context gives `rect` itself; a rectangle that shares no
    /// area with the current clip makes it `Empty`, and an empty clip stays empty.
    pub fn clip_to(&mut self, rect: Rect) {
        self.current = match self.current {
            Clip::Empty => Clip::Empty,
            Clip::Unclipped => match overlap(rect, rect) {
                Some(r) => Clip::Bounded(r),
                None => Clip::Empty,
            },
            Clip::Bounded(c) => match overlap(c, rect) {
                Some(r) => Clip::Bounded(r),
                None => Clip::Empty,
            },
        };
    }

    /// Stores the current clip so a later `restore` brings it back.
    pub fn save(&mut self) {
        self.saved.push(self.current);
    }

    /// Brings back the clip stored by the last `save`. Returns `false` and changes nothing when nothing was saved.
    pub fn restore(&mut self) -> bool {
        match self.saved.pop() {
            Some(c) => {
                self.current = c;
                true
            }
            None => false,
        }
    }

    /// Removes every clip, back to the whole surface. The save stack is kept.
    pub fn reset_clip(&mut self) {
        self.current = Clip::Unclipped;
    }

    /// Whether the point `(x, y)` may paint: inside the clip, with the right and bottom edges excluded.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self.current {
            Clip::Unclipped => true,
            Clip::Empty => false,
            Clip::Bounded(r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h,
        }
    }

    /// Whether `rect` shares any area with the clip, so a painter need not draw it when this is `false`.
    pub fn may_paint(&self, rect: Rect) -> bool {
        match self.current {
            Clip::Unclipped => rect.w > 0.0 && rect.h > 0.0,
            Clip::Empty => false,
            Clip::Bounded(c) => overlap(c, rect).is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn clipping_an_unclipped_context_gives_the_rectangle() {
        let mut c = ClipState::new();
        c.clip_to(r(10.0, 20.0, 30.0, 40.0));
        assert_eq!(c.clip(), Clip::Bounded(r(10.0, 20.0, 30.0, 40.0)));
    }

    #[test]
    fn successive_clips_intersect() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 100.0, 100.0));
        c.clip_to(r(50.0, 60.0, 100.0, 100.0));
        assert_eq!(c.clip(), Clip::Bounded(r(50.0, 60.0, 50.0, 40.0)));
    }

    #[test]
    fn disjoint_clip_is_empty_and_stays_empty() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 10.0, 10.0));
        c.clip_to(r(20.0, 20.0, 10.0, 10.0));
        assert_eq!(c.clip(), Clip::Empty);
        c.clip_to(r(0.0, 0.0, 100.0, 100.0));
        assert_eq!(c.clip(), Clip::Empty, "clipping cannot widen an empty clip");
        assert!(!c.contains(0.0, 0.0));
    }

    #[test]
    fn restore_brings_back_the_saved_clip() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 100.0, 100.0));
        c.save();
        c.clip_to(r(0.0, 0.0, 10.0, 10.0));
        assert_eq!(c.clip(), Clip::Bounded(r(0.0, 0.0, 10.0, 10.0)));
        assert!(c.restore());
        assert_eq!(c.clip(), Clip::Bounded(r(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn restore_with_nothing_saved_changes_nothing() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 5.0, 5.0));
        assert!(!c.restore());
        assert_eq!(c.clip(), Clip::Bounded(r(0.0, 0.0, 5.0, 5.0)));
    }

    #[test]
    fn containment_excludes_the_right_and_bottom_edges() {
        let mut c = ClipState::new();
        c.clip_to(r(10.0, 10.0, 20.0, 20.0));
        assert!(c.contains(10.0, 10.0));
        assert!(c.contains(29.9, 29.9));
        assert!(!c.contains(30.0, 15.0), "the right edge is outside");
        assert!(!c.contains(15.0, 30.0), "the bottom edge is outside");
    }

    #[test]
    fn may_paint_culls_rectangles_outside_the_clip() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 100.0, 100.0));
        assert!(c.may_paint(r(90.0, 90.0, 50.0, 50.0)), "an overlapping corner paints");
        assert!(!c.may_paint(r(100.0, 0.0, 10.0, 10.0)), "a rectangle touching only the edge does not");
        assert!(!c.may_paint(r(0.0, 0.0, 0.0, 10.0)), "a zero-width rectangle does not");
    }

    #[test]
    fn reset_clip_removes_every_clip_but_keeps_the_saved_state() {
        let mut c = ClipState::new();
        c.clip_to(r(0.0, 0.0, 10.0, 10.0));
        c.save();
        c.clip_to(r(0.0, 0.0, 5.0, 5.0));
        c.reset_clip();
        assert_eq!(c.clip(), Clip::Unclipped);
        assert!(c.contains(500.0, 500.0));
        assert!(c.restore());
        assert_eq!(c.clip(), Clip::Bounded(r(0.0, 0.0, 10.0, 10.0)));
    }
}
