//! Reordering a row of icons by dragging (the Dock): which slot the pointer is over, the order the others show
//! while one is lifted, and per-icon eased offsets so neighbours slide aside instead of jumping.
use crate::anim::{Animation, Curve};

/// The slot whose centre is nearest `(x, y)`. Works for a horizontal or vertical dock.
pub fn slot_at(centres: &[(f32, f32)], x: f32, y: f32) -> Option<usize> {
    centres
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let d = |c: &(f32, f32)| (c.0 - x).powi(2) + (c.1 - y).powi(2);
            d(a.1).total_cmp(&d(b.1))
        })
        .map(|(i, _)| i)
}

/// `order[slot]` = the original index that shows in `slot` while `from` is lifted over `to`.
pub fn preview_order(n: usize, from: usize, to: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    if from < n && to < n {
        let item = order.remove(from);
        order.insert(to, item);
    }
    order
}

/// Eased x/y offsets, one pair per icon, retargeted without a jump when the preview changes.
#[derive(Default)]
pub struct Slide {
    x: Vec<Animation>,
    y: Vec<Animation>,
}

const SLIDE_SECS: f32 = 0.18;

impl Slide {
    pub fn new() -> Self {
        Slide::default()
    }
    /// Sets where each icon should be, as an offset from its resting slot. Call when the preview changes.
    pub fn set_targets(&mut self, now: f64, targets: &[(f32, f32)]) {
        let make = |from: f32, to: f32| Animation::new(from, to, now, SLIDE_SECS, Curve::EaseInOut);
        if self.x.len() != targets.len() {
            self.x = targets.iter().map(|_| make(0.0, 0.0)).collect();
            self.y = self.x.clone();
        }
        for (i, t) in targets.iter().enumerate() {
            if self.x[i].to != t.0 {
                self.x[i] = self.x[i].retarget(now, t.0);
            }
            if self.y[i].to != t.1 {
                self.y[i] = self.y[i].retarget(now, t.1);
            }
        }
    }
    pub fn offset(&self, i: usize, now: f64) -> (f32, f32) {
        match (self.x.get(i), self.y.get(i)) {
            (Some(x), Some(y)) => (x.value(now), y.value(now)),
            _ => (0.0, 0.0),
        }
    }
    /// True while any icon is still moving or has not returned to rest.
    pub fn active(&self, now: f64) -> bool {
        self.x.iter().chain(&self.y).any(|a| (!a.is_done(now) && a.from != a.to) || a.to != 0.0)
    }
    /// Everything returns to its resting slot (a drag ended or was cancelled).
    pub fn release(&mut self, now: f64) {
        let n = self.x.len();
        if n > 0 {
            self.set_targets(now, &vec![(0.0, 0.0); n]);
        }
    }
    /// Stops all motion at once (the dock changed under us).
    pub fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_slot_in_a_row_and_a_column() {
        let row: Vec<(f32, f32)> = (0..4).map(|i| (i as f32 * 60.0 + 30.0, 20.0)).collect();
        assert_eq!(slot_at(&row, 100.0, 500.0), Some(1));
        assert_eq!(slot_at(&row, -50.0, 0.0), Some(0));
        let col: Vec<(f32, f32)> = (0..4).map(|i| (20.0, i as f32 * 60.0 + 30.0)).collect();
        assert_eq!(slot_at(&col, 0.0, 200.0), Some(3));
        assert_eq!(slot_at(&[], 0.0, 0.0), None);
    }
    #[test]
    fn preview_shifts_the_neighbours() {
        assert_eq!(preview_order(4, 0, 2), vec![1, 2, 0, 3]);
        assert_eq!(preview_order(4, 3, 1), vec![0, 3, 1, 2]);
        assert_eq!(preview_order(4, 2, 2), vec![0, 1, 2, 3]);
        assert_eq!(preview_order(3, 9, 1), vec![0, 1, 2]);
    }
    #[test]
    fn slide_eases_retargets_without_a_jump_and_settles() {
        let mut s = Slide::new();
        s.set_targets(0.0, &[(0.0, 0.0), (0.0, 0.0)]);
        assert!(!s.active(0.0));
        s.set_targets(0.0, &[(60.0, 0.0), (-60.0, 0.0)]);
        assert_eq!(s.offset(0, 0.0).0, 0.0);
        let mid = s.offset(0, 0.09).0;
        assert!(mid > 5.0 && mid < 55.0, "{mid}");
        assert!(s.active(0.09));
        s.set_targets(0.09, &[(0.0, 0.0), (0.0, 0.0)]); // retarget mid-flight
        assert!((s.offset(0, 0.09).0 - mid).abs() < 1e-3);
        assert_eq!(s.offset(0, 1.0).0, 0.0);
        assert!(!s.active(1.0));
    }
    #[test]
    fn a_held_offset_stays_active_until_released() {
        let mut s = Slide::new();
        s.set_targets(0.0, &[(60.0, 0.0)]);
        assert!(s.active(5.0));
        s.release(5.0);
        assert!(s.active(5.1));
        assert!(!s.active(6.0));
    }
}
