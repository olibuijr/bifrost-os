//! Window scene geometry shared by the desktop and the tablet: the size restrictions a scene declares
//! (`UIWindowScene.sizeRestrictions`), the cascade that places a new scene window on a screen, and the clamp that keeps
//! a window inside its restrictions and inside the screen.
//!
//! Units are logical points, like the rest of the model. The cascade is the same on both form factors: a new window
//! lands one step to the right and down from the previous one, and wraps back to the top-left corner when the next step
//! would leave the screen.
use crate::Rect;

/// The distance (points) between two cascaded windows.
pub const CASCADE_STEP: f32 = 28.0;
/// The distance (points) between the screen edge and the first cascaded window.
pub const CASCADE_MARGIN: f32 = 24.0;

/// The minimum and maximum size of one window scene, in points. A maximum below the minimum is raised to the minimum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeRestrictions {
    pub min_w: f32,
    pub min_h: f32,
    pub max_w: f32,
    pub max_h: f32,
}

impl Default for SizeRestrictions {
    /// No restriction: any positive size, with no upper bound.
    fn default() -> Self {
        Self { min_w: 0.0, min_h: 0.0, max_w: f32::INFINITY, max_h: f32::INFINITY }
    }
}

impl SizeRestrictions {
    /// The size `(w, h)` moved into the restrictions. Non-finite or negative input is treated as the minimum.
    pub fn clamp(&self, w: f32, h: f32) -> (f32, f32) {
        (clamp_axis(w, self.min_w, self.max_w), clamp_axis(h, self.min_h, self.max_h))
    }
}

fn clamp_axis(v: f32, min: f32, max: f32) -> f32 {
    let min = min.max(0.0);
    let max = max.max(min);
    if !v.is_finite() {
        return min;
    }
    v.clamp(min, max)
}

/// The frame for a new window of size `(w, h)` on `screen`, cascaded past the `existing` windows of the same app.
///
/// The offset is `existing.len()` steps from the top-left corner of the screen, inside the margin. When the offset would
/// put the window past the screen edge, the offset wraps to zero. A window larger than the screen is shrunk to fit it.
pub fn cascade(existing: &[Rect], screen: Rect, size: (f32, f32)) -> Rect {
    let w = size.0.min(screen.w.max(0.0)).max(0.0);
    let h = size.1.min(screen.h.max(0.0)).max(0.0);
    let room_w = (screen.w - w - 2.0 * CASCADE_MARGIN).max(0.0);
    let room_h = (screen.h - h - 2.0 * CASCADE_MARGIN).max(0.0);
    let steps_w = (room_w / CASCADE_STEP).floor() as usize + 1;
    let steps_h = (room_h / CASCADE_STEP).floor() as usize + 1;
    let steps = steps_w.min(steps_h).max(1);
    let index = existing.len() % steps;
    let offset = index as f32 * CASCADE_STEP;
    let x = (screen.x + CASCADE_MARGIN + offset).min(screen.x + screen.w - w).max(screen.x);
    let y = (screen.y + CASCADE_MARGIN + offset).min(screen.y + screen.h - h).max(screen.y);
    Rect { x, y, w, h }
}

/// Whether `inner` lies entirely inside `outer` (edges included).
pub fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect { x: 0.0, y: 40.0, w: 1000.0, h: 700.0 }
    }

    #[test]
    fn restrictions_clamp_and_ignore_bad_input() {
        let r = SizeRestrictions { min_w: 320.0, min_h: 240.0, max_w: 900.0, max_h: 600.0 };
        assert_eq!(r.clamp(100.0, 100.0), (320.0, 240.0));
        assert_eq!(r.clamp(2000.0, 2000.0), (900.0, 600.0));
        assert_eq!(r.clamp(500.0, 300.0), (500.0, 300.0));
        assert_eq!(r.clamp(f32::NAN, f32::INFINITY), (320.0, 240.0));
        let unrestricted = SizeRestrictions::default();
        assert_eq!(unrestricted.clamp(-5.0, 4000.0), (0.0, 4000.0));
    }

    #[test]
    fn a_maximum_below_the_minimum_is_raised() {
        let r = SizeRestrictions { min_w: 400.0, min_h: 0.0, max_w: 100.0, max_h: 0.0 };
        assert_eq!(r.clamp(50.0, 0.0).0, 400.0);
    }

    #[test]
    fn the_first_window_sits_at_the_margin() {
        let f = cascade(&[], screen(), (400.0, 300.0));
        assert_eq!(f, Rect { x: CASCADE_MARGIN, y: 40.0 + CASCADE_MARGIN, w: 400.0, h: 300.0 });
    }

    #[test]
    fn each_new_window_steps_down_and_right() {
        let first = cascade(&[], screen(), (400.0, 300.0));
        let second = cascade(&[first], screen(), (400.0, 300.0));
        assert_eq!(second.x, first.x + CASCADE_STEP);
        assert_eq!(second.y, first.y + CASCADE_STEP);
    }

    #[test]
    fn the_cascade_wraps_before_leaving_the_screen() {
        let size = (400.0, 300.0);
        let mut placed = Vec::new();
        for _ in 0..40 {
            let f = cascade(&placed, screen(), size);
            assert!(contains(screen(), f), "window {f:?} leaves the screen");
            placed.push(f);
        }
        assert!(placed[1..].contains(&placed[0]), "the cascade never wraps back to its first frame");
    }

    #[test]
    fn a_window_larger_than_the_screen_is_shrunk_to_fit() {
        let f = cascade(&[], screen(), (5000.0, 5000.0));
        assert_eq!((f.w, f.h), (1000.0, 700.0));
        assert!(contains(screen(), f));
    }

    #[test]
    fn contains_counts_edges_as_inside() {
        let outer = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        assert!(contains(outer, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }));
        assert!(!contains(outer, Rect { x: 1.0, y: 0.0, w: 10.0, h: 10.0 }));
    }
}
