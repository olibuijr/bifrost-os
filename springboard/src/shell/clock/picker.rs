//! A scrolling picker column (NativePickerView): drag to scroll, release to
//! snap to the nearest row with a short ease-out, tap a row to select it.

/// Snap animation length in ms.
const SNAP_MS: f64 = 200.0;
/// How far a flick carries on after release, in ms of velocity.
const FLING_MS: f32 = 120.0;
/// Below this travel a touch is a tap.
const TAP_PX: f32 = 6.0;
/// A pause this long before lifting cancels the fling.
const REST_MS: f64 = 80.0;

#[derive(Debug, PartialEq)]
pub struct Picker {
    pub count: usize,
    pub row_h: f32,
    /// scroll position in rows (0 = first row centred)
    pos: f32,
    /// (last y, last t, velocity px/ms, total travel) while dragging
    drag: Option<(f32, f64, f32, f32)>,
    /// (from, to, start) of the snap in progress
    snap: Option<(f32, f32, f64)>,
}

impl Picker {
    pub fn new(count: usize, row_h: f32, selected: usize) -> Self {
        Picker { count, row_h, pos: selected.min(count.saturating_sub(1)) as f32, drag: None, snap: None }
    }

    pub fn selected(&self) -> usize {
        self.pos.round().clamp(0.0, (self.count.max(1) - 1) as f32) as usize
    }

    pub fn select(&mut self, i: usize) {
        self.pos = i.min(self.count.saturating_sub(1)) as f32;
        self.snap = None;
    }

    /// Current scroll position in rows, fractional while moving.
    pub fn position(&self) -> f32 {
        self.pos
    }

    pub fn animating(&self) -> bool {
        self.snap.is_some()
    }

    fn max(&self) -> f32 {
        (self.count.max(1) - 1) as f32
    }

    /// `y` is relative to the picker's vertical centre (the selection row).
    pub fn down(&mut self, y: f32, t: f64) {
        self.snap = None;
        self.drag = Some((y, t, 0.0, 0.0));
    }

    pub fn moved(&mut self, y: f32, t: f64) {
        let Some((ly, lt, v, travel)) = self.drag else { return };
        let dy = y - ly;
        let dt = (t - lt).max(1.0) as f32;
        // dragging down reveals earlier rows; overscroll is damped
        let mut pos = self.pos - dy / self.row_h;
        if pos < 0.0 || pos > self.max() {
            pos = self.pos - dy / self.row_h / 3.0;
        }
        self.pos = pos;
        self.drag = Some((y, t, 0.7 * v + 0.3 * (-dy / dt), travel + dy.abs()));
    }

    /// Ends the touch: a tap selects the touched row, a drag snaps (carried
    /// on by its velocity).
    pub fn up(&mut self, y: f32, t: f64) {
        let Some((_, lt, v, travel)) = self.drag.take() else { return };
        // a finger that rested before lifting has no fling left
        let v = if t - lt > REST_MS { 0.0 } else { v };
        let target = if travel < TAP_PX {
            self.pos.round() + (y / self.row_h).round()
        } else {
            (self.pos + v * FLING_MS / self.row_h).round()
        };
        self.snap_to(target.clamp(0.0, self.max()), t);
    }

    fn snap_to(&mut self, target: f32, t: f64) {
        if (target - self.pos).abs() < 1e-3 {
            self.pos = target;
        } else {
            self.snap = Some((self.pos, target, t));
        }
    }

    /// Advances the snap; true while it is still moving.
    pub fn tick(&mut self, t: f64) -> bool {
        let Some((from, to, start)) = self.snap else { return false };
        let p = ((t - start) / SNAP_MS).clamp(0.0, 1.0) as f32;
        let e = 1.0 - (1.0 - p).powi(3);
        self.pos = from + (to - from) * e;
        if p >= 1.0 {
            self.pos = to;
            self.snap = None;
        }
        self.snap.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_scrolls_and_snaps_to_the_nearest_row() {
        let mut p = Picker::new(60, 50.0, 15);
        assert_eq!(p.selected(), 15);
        p.down(0.0, 0.0);
        p.moved(-70.0, 16.0);
        p.moved(-70.0, 100.0);
        assert!((p.position() - 16.4).abs() < 1e-3, "finger up by 1.4 rows");
        p.up(-70.0, 200.0);
        assert!(p.animating());
        p.tick(300.0);
        assert!(p.animating() && p.position() > 16.0 && p.position() < 16.4);
        assert!(!p.tick(400.0));
        assert_eq!(p.selected(), 16);
        assert_eq!(p.position(), 16.0);
    }

    #[test]
    fn tap_selects_the_touched_row_and_clamps() {
        let mut p = Picker::new(24, 50.0, 0);
        p.down(120.0, 0.0);
        p.up(122.0, 10.0);
        p.tick(1000.0);
        assert_eq!(p.selected(), 2, "two rows below the centre");
        p.down(-120.0, 2000.0);
        p.up(-120.0, 2010.0);
        p.tick(3000.0);
        assert_eq!(p.selected(), 0, "cannot go above the first row");
        p.select(23);
        p.down(0.0, 4000.0);
        p.moved(-500.0, 4016.0);
        assert!(p.position() < 23.0 + 4.0, "overscroll is damped");
        p.up(-500.0, 4100.0);
        p.tick(5000.0);
        assert_eq!(p.selected(), 23);
    }

    #[test]
    fn a_flick_carries_on() {
        let mut p = Picker::new(60, 50.0, 10);
        p.down(0.0, 0.0);
        p.moved(-50.0, 16.0);
        p.moved(-100.0, 32.0);
        p.up(-100.0, 32.0);
        p.tick(1000.0);
        assert!(p.selected() > 12, "velocity adds rows: {}", p.selected());
        assert_eq!(Picker::new(0, 50.0, 3).selected(), 0);
    }
}
