//! The magnifier's geometry and the Color Meter's pointer state: which display pixels the loupe shows around the
//! pointer, which pixel the readout describes, how the position locks, and how the arrow keys nudge it by one pixel.
//! Pure: the caller reads the pixels and draws them.

use crate::source::PixelRect;

/// The display pixel under `(x, y)`, clamped to the display. `None` when the display has no area.
pub fn pixel_at(x: f32, y: f32, display_w: u32, display_h: u32) -> Option<(u32, u32)> {
    if display_w == 0 || display_h == 0 {
        return None;
    }
    let px = (x.floor().max(0.0) as u32).min(display_w - 1);
    let py = (y.floor().max(0.0) as u32).min(display_h - 1);
    Some((px, py))
}

/// The square of `cells` display pixels the loupe shows around `(x, y)`. It moves inward at the display edges so it
/// stays on the display, and it shrinks to the display when the display is smaller than the loupe.
pub fn sample_rect(x: u32, y: u32, cells: u32, display_w: u32, display_h: u32) -> PixelRect {
    let w = cells.min(display_w);
    let h = cells.min(display_h);
    let half = cells / 2;
    let left = x.saturating_sub(half).min(display_w.saturating_sub(w));
    let top = y.saturating_sub(half).min(display_h.saturating_sub(h));
    PixelRect { x: left, y: top, w, h }
}

/// The cell index inside `rect` of the pixel `(x, y)`, when that pixel is inside `rect`.
pub fn cell_of(rect: PixelRect, x: u32, y: u32) -> Option<(u32, u32)> {
    if x < rect.x || y < rect.y || x >= rect.x + rect.w || y >= rect.y + rect.h {
        return None;
    }
    Some((x - rect.x, y - rect.y))
}

/// The part of the loupe `aperture` covers, in cells of `view` (cell `(0, 0)` is the top left pixel of `view`). `None`
/// when the two do not overlap. The loupe outlines this when the aperture is wider than one pixel.
pub fn aperture_cells(view: PixelRect, aperture: PixelRect) -> Option<PixelRect> {
    let x0 = aperture.x.max(view.x);
    let y0 = aperture.y.max(view.y);
    let x1 = (aperture.x + aperture.w).min(view.x + view.w);
    let y1 = (aperture.y + aperture.h).min(view.y + view.h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(PixelRect { x: x0 - view.x, y: y0 - view.y, w: x1 - x0, h: y1 - y0 })
}

/// The pixel the readout describes. A locked axis does not follow the mouse: with both locked the readout stays put, with
/// one locked it slides along the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pointer {
    pub x: u32,
    pub y: u32,
    pub lock_x: bool,
    pub lock_y: bool,
}

impl Pointer {
    /// Whether the position is locked on both axes.
    pub fn locked(&self) -> bool {
        self.lock_x && self.lock_y
    }

    /// Moves to `(x, y)` on the axes that are not locked. True when the position changed.
    pub fn follow(&mut self, x: u32, y: u32) -> bool {
        let nx = if self.lock_x { self.x } else { x };
        let ny = if self.lock_y { self.y } else { y };
        if (self.x, self.y) == (nx, ny) {
            return false;
        }
        self.x = nx;
        self.y = ny;
        true
    }

    /// Locks both axes, or unlocks both when both are locked already.
    pub fn toggle_lock(&mut self) {
        let on = !self.locked();
        self.lock_x = on;
        self.lock_y = on;
    }

    pub fn toggle_lock_x(&mut self) {
        self.lock_x = !self.lock_x;
    }

    pub fn toggle_lock_y(&mut self) {
        self.lock_y = !self.lock_y;
    }

    /// Moves by one pixel or more, staying on the display. True when the position changed.
    pub fn nudge(&mut self, dx: i32, dy: i32, display_w: u32, display_h: u32) -> bool {
        if display_w == 0 || display_h == 0 {
            return false;
        }
        let nx = (self.x as i64 + dx as i64).clamp(0, display_w as i64 - 1) as u32;
        let ny = (self.y as i64 + dy as i64).clamp(0, display_h as i64 - 1) as u32;
        let moved = (nx, ny) != (self.x, self.y);
        self.x = nx;
        self.y = ny;
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_pixel_is_clamped_to_the_display() {
        assert_eq!(pixel_at(3.7, 0.2, 10, 10), Some((3, 0)));
        assert_eq!(pixel_at(-4.0, 99.0, 10, 10), Some((0, 9)));
        assert_eq!(pixel_at(1.0, 1.0, 0, 10), None);
    }

    #[test]
    fn the_loupe_is_centred_on_the_pointer_away_from_edges() {
        let r = sample_rect(50, 40, 11, 100, 100);
        assert_eq!(r, PixelRect { x: 45, y: 35, w: 11, h: 11 });
        assert_eq!(cell_of(r, 50, 40), Some((5, 5)));
    }

    #[test]
    fn the_loupe_moves_inward_at_the_edges() {
        let r = sample_rect(0, 99, 11, 100, 100);
        assert_eq!(r, PixelRect { x: 0, y: 89, w: 11, h: 11 });
        assert_eq!(cell_of(r, 0, 99), Some((0, 10)));
        let far = sample_rect(99, 0, 11, 100, 100);
        assert_eq!(far, PixelRect { x: 89, y: 0, w: 11, h: 11 });
    }

    #[test]
    fn a_small_display_shrinks_the_loupe() {
        let r = sample_rect(1, 1, 11, 4, 3);
        assert_eq!(r, PixelRect { x: 0, y: 0, w: 4, h: 3 });
        assert_eq!(cell_of(r, 1, 1), Some((1, 1)));
    }

    #[test]
    fn a_locked_pointer_does_not_follow() {
        let mut p = Pointer::default();
        assert!(p.follow(5, 6));
        assert!(!p.follow(5, 6));
        p.toggle_lock();
        assert!(!p.follow(9, 9));
        assert_eq!((p.x, p.y), (5, 6));
        p.toggle_lock();
        assert!(p.follow(9, 9));
    }

    #[test]
    fn locking_one_axis_lets_the_other_follow() {
        let mut p = Pointer { x: 5, y: 6, ..Pointer::default() };
        p.toggle_lock_x();
        assert!(!p.locked());
        assert!(p.follow(9, 2));
        assert_eq!((p.x, p.y), (5, 2));
        // nothing moves on the free axis: no change
        assert!(!p.follow(9, 2));
        p.toggle_lock_x();
        p.toggle_lock_y();
        assert!(p.follow(9, 3));
        assert_eq!((p.x, p.y), (9, 2));
    }

    #[test]
    fn lock_position_locks_both_axes_then_releases_both() {
        let mut p = Pointer::default();
        p.toggle_lock_x();
        // one axis locked already: the combined lock completes it rather than flipping it off
        p.toggle_lock();
        assert!(p.lock_x && p.lock_y && p.locked());
        p.toggle_lock();
        assert!(!p.lock_x && !p.lock_y && !p.locked());
    }

    #[test]
    fn the_aperture_is_clipped_to_the_loupe() {
        let view = PixelRect { x: 10, y: 10, w: 11, h: 11 };
        // fully inside: cell coordinates are relative to the view
        assert_eq!(aperture_cells(view, PixelRect { x: 13, y: 14, w: 3, h: 3 }), Some(PixelRect { x: 3, y: 4, w: 3, h: 3 }));
        // wider than the view: the whole view
        assert_eq!(aperture_cells(view, PixelRect { x: 0, y: 0, w: 31, h: 31 }), Some(PixelRect { x: 0, y: 0, w: 11, h: 11 }));
        // overlapping one corner
        assert_eq!(aperture_cells(view, PixelRect { x: 19, y: 8, w: 5, h: 5 }), Some(PixelRect { x: 9, y: 0, w: 2, h: 3 }));
        // no overlap
        assert_eq!(aperture_cells(view, PixelRect { x: 30, y: 30, w: 3, h: 3 }), None);
    }

    #[test]
    fn nudge_steps_one_pixel_and_stops_at_the_edge() {
        let mut p = Pointer { x: 0, y: 4, lock_x: true, lock_y: true };
        assert!(p.nudge(1, 0, 10, 10));
        assert_eq!((p.x, p.y), (1, 4));
        assert!(p.nudge(-5, 0, 10, 10));
        assert_eq!(p.x, 0);
        assert!(!p.nudge(-1, 0, 10, 10));
        assert!(p.nudge(0, 100, 10, 10));
        assert_eq!(p.y, 9);
        assert!(!p.nudge(1, 1, 0, 0));
    }
}
