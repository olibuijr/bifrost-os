//! Stabilised camera region of interest: the part of a live camera frame that a view keeps on screen.
//!
//! An app points the region at what it is looking for (a document outline, a code). The raw target moves with
//! every frame, so [`Stabilizer::observe`] smooths it. A move smaller than the dead band is ignored, and a larger
//! move closes half of the remaining distance per frame. The result is always inside the frame, and a region
//! larger than the frame is cut down to the frame's size.

use crate::Rect;

/// Edge movement in points below which the region does not move.
pub const DEFAULT_DEAD_BAND: f32 = 4.0;

/// Clamps `r` so it fits inside `frame`: width and height are cut to the frame's size, then the position is pulled
/// back inside the frame's edges.
fn fit(r: Rect, frame: Rect) -> Rect {
    let fw = frame.w.max(0.0);
    let fh = frame.h.max(0.0);
    let w = r.w.max(0.0).min(fw);
    let h = r.h.max(0.0).min(fh);
    let x = r.x.max(frame.x).min(frame.x + fw - w);
    let y = r.y.max(frame.y).min(frame.y + fh - h);
    Rect { x, y, w, h }
}

/// The largest movement of any of the four edges between two rectangles.
fn edge_delta(a: Rect, b: Rect) -> f32 {
    let dx0 = (a.x - b.x).abs();
    let dy0 = (a.y - b.y).abs();
    let dx1 = (a.x + a.w - (b.x + b.w)).abs();
    let dy1 = (a.y + a.h - (b.y + b.h)).abs();
    dx0.max(dy0).max(dx1).max(dy1)
}

/// Holds the region on screen for one camera frame at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stabilizer {
    frame: Rect,
    dead_band: f32,
    current: Option<Rect>,
}

impl Stabilizer {
    /// A stabilizer for a frame with the default dead band. Nothing is shown until the first [`observe`](Self::observe).
    pub fn new(frame: Rect) -> Self {
        Self::with_dead_band(frame, DEFAULT_DEAD_BAND)
    }

    /// A stabilizer with its own dead band. A negative band is treated as zero.
    pub fn with_dead_band(frame: Rect, dead_band: f32) -> Self {
        Self { frame, dead_band: dead_band.max(0.0), current: None }
    }

    /// Changes the frame size (for example after a rotation). The shown region is clamped to the new frame.
    pub fn set_frame(&mut self, frame: Rect) {
        self.frame = frame;
        self.current = self.current.map(|r| fit(r, frame));
    }

    /// Takes the raw target for this frame and returns the region to draw.
    pub fn observe(&mut self, target: Rect) -> Rect {
        let target = fit(target, self.frame);
        let next = match self.current {
            None => target,
            Some(cur) if edge_delta(cur, target) < self.dead_band => cur,
            Some(cur) => Rect {
                x: cur.x + (target.x - cur.x) * 0.5,
                y: cur.y + (target.y - cur.y) * 0.5,
                w: cur.w + (target.w - cur.w) * 0.5,
                h: cur.h + (target.h - cur.h) * 0.5,
            },
        };
        self.current = Some(next);
        next
    }

    /// The region last returned by [`observe`](Self::observe), or None before the first frame.
    pub fn current(&self) -> Option<Rect> {
        self.current
    }

    /// Forgets the shown region, so the next target is shown at once.
    pub fn reset(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn frame() -> Rect {
        r(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn first_target_is_shown_at_once() {
        let mut s = Stabilizer::new(frame());
        assert_eq!(s.current(), None);
        assert_eq!(s.observe(r(10.0, 10.0, 50.0, 40.0)), r(10.0, 10.0, 50.0, 40.0));
        assert_eq!(s.current(), Some(r(10.0, 10.0, 50.0, 40.0)));
    }

    #[test]
    fn jitter_inside_the_dead_band_is_ignored() {
        let mut s = Stabilizer::new(frame());
        s.observe(r(10.0, 10.0, 50.0, 40.0));
        // Largest edge movement is 2 points, below the 4 point band.
        assert_eq!(s.observe(r(12.0, 10.0, 50.0, 40.0)), r(10.0, 10.0, 50.0, 40.0));
        // A movement of exactly the band (4 points) is not below it, so the region moves.
        assert_eq!(s.observe(r(14.0, 10.0, 50.0, 40.0)), r(12.0, 10.0, 50.0, 40.0));
    }

    #[test]
    fn larger_moves_close_half_the_distance() {
        let mut s = Stabilizer::new(frame());
        s.observe(r(10.0, 10.0, 50.0, 40.0));
        // x moves 10 to 30 (edge delta 20): halfway is 20. The right edge moves the same way, so width stays 50.
        assert_eq!(s.observe(r(30.0, 10.0, 50.0, 40.0)), r(20.0, 10.0, 50.0, 40.0));
        // Next frame the remaining distance is 10: halfway again gives 25.
        assert_eq!(s.observe(r(30.0, 10.0, 50.0, 40.0)), r(25.0, 10.0, 50.0, 40.0));
    }

    #[test]
    fn targets_outside_the_frame_are_pulled_inside() {
        let mut s = Stabilizer::new(frame());
        // 50 x 40 at (180, 90) runs off the right and bottom edges: x becomes 200 - 50 = 150, y becomes 100 - 40 = 60.
        assert_eq!(s.observe(r(180.0, 90.0, 50.0, 40.0)), r(150.0, 60.0, 50.0, 40.0));
    }

    #[test]
    fn oversized_targets_are_cut_to_the_frame() {
        let mut s = Stabilizer::new(frame());
        assert_eq!(s.observe(r(-20.0, -5.0, 300.0, 150.0)), r(0.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn shrinking_the_frame_clamps_the_shown_region() {
        let mut s = Stabilizer::new(frame());
        s.observe(r(180.0, 90.0, 50.0, 40.0));
        // Shown region is 150,60 50x40. In a 100 x 50 frame: x = min(150, 100 - 50) = 50, y = min(60, 50 - 40) = 10.
        s.set_frame(r(0.0, 0.0, 100.0, 50.0));
        assert_eq!(s.current(), Some(r(50.0, 10.0, 50.0, 40.0)));
    }

    #[test]
    fn reset_shows_the_next_target_at_once() {
        let mut s = Stabilizer::with_dead_band(frame(), 100.0);
        s.observe(r(10.0, 10.0, 50.0, 40.0));
        // A huge dead band would hold the region still, but reset forgets it.
        assert_eq!(s.observe(r(90.0, 10.0, 50.0, 40.0)), r(10.0, 10.0, 50.0, 40.0));
        s.reset();
        assert_eq!(s.current(), None);
        assert_eq!(s.observe(r(90.0, 10.0, 50.0, 40.0)), r(90.0, 10.0, 50.0, 40.0));
    }
}
