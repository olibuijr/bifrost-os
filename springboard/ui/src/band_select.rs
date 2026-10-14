//! Band selection (`UIBandSelectionInteraction`): a drag that starts on empty space draws a rubber band and selects every
//! item the band touches. The same model serves a pointer drag and a finger drag, since both report a start point and a
//! series of points in view points. A short movement is still a tap, so the band only becomes active once the pointer
//! has moved `START_DISTANCE_PT` from where it started.

use crate::Rect;

/// Movement, in points, before a press turns into a band. Smaller movements stay a tap.
pub const START_DISTANCE_PT: f32 = 6.0;

/// A selectable item and its frame, in the same space as the band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub id: u64,
    pub frame: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BandSelection {
    start: (f32, f32),
    current: (f32, f32),
    /// Items selected before the drag began. Kept when `extend` is set.
    base: Vec<u64>,
    /// Add to the existing selection (Shift or Command held) instead of replacing it.
    extend: bool,
    active: bool,
}

impl BandSelection {
    /// A band that starts at `(x, y)`. `base` is the selection before the drag, and `extend` keeps it.
    pub fn begin(x: f32, y: f32, base: Vec<u64>, extend: bool) -> Self {
        BandSelection { start: (x, y), current: (x, y), base, extend, active: false }
    }

    /// Moves the band's free corner to `(x, y)`. Returns the band rectangle once the band is active, so the caller can
    /// draw it.
    pub fn update(&mut self, x: f32, y: f32) -> Option<Rect> {
        self.current = (x, y);
        let dx = (x - self.start.0).abs();
        let dy = (y - self.start.1).abs();
        if dx.max(dy) >= START_DISTANCE_PT {
            self.active = true;
        }
        self.rect()
    }

    /// The band rectangle, normalised so its width and height are positive. `None` while the band is not active.
    pub fn rect(&self) -> Option<Rect> {
        if !self.active {
            return None;
        }
        let x = self.start.0.min(self.current.0);
        let y = self.start.1.min(self.current.1);
        let w = (self.start.0 - self.current.0).abs();
        let h = (self.start.1 - self.current.1).abs();
        Some(Rect { x, y, w, h })
    }

    /// The ids the band selects right now, in ascending order with no repeats. Before the band is active the selection is
    /// the base when extending, and empty otherwise.
    pub fn selected(&self, items: &[Item]) -> Vec<u64> {
        let mut out: Vec<u64> = if self.extend { self.base.clone() } else { Vec::new() };
        if let Some(band) = self.rect() {
            out.extend(items.iter().filter(|i| overlaps(band, i.frame)).map(|i| i.id));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Ends the drag and returns the final selection. A drag that never became a band leaves the base selection when
    /// extending and clears it otherwise, which is what a tap on empty space does.
    pub fn finish(self, items: &[Item]) -> Vec<u64> {
        self.selected(items)
    }
}

/// True when the two rectangles share some area. Touching edges do not count.
pub fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, x: f32, y: f32) -> Item {
        Item { id, frame: Rect { x, y, w: 10.0, h: 10.0 } }
    }

    #[test]
    fn a_short_press_is_not_a_band() {
        let items = [item(1, 0.0, 0.0)];
        let mut b = BandSelection::begin(0.0, 0.0, vec![], false);
        assert_eq!(b.update(3.0, 2.0), None);
        assert!(b.selected(&items).is_empty());
    }

    #[test]
    fn a_drag_selects_what_the_band_touches() {
        let items = [item(1, 0.0, 0.0), item(2, 50.0, 0.0), item(3, 0.0, 50.0)];
        let mut b = BandSelection::begin(-5.0, -5.0, vec![], false);
        let r = b.update(30.0, 30.0).unwrap();
        assert_eq!(r, Rect { x: -5.0, y: -5.0, w: 35.0, h: 35.0 });
        assert_eq!(b.selected(&items), vec![1]);
    }

    #[test]
    fn the_band_is_normalised_when_dragged_up_and_left() {
        let mut b = BandSelection::begin(40.0, 40.0, vec![], false);
        let r = b.update(10.0, 20.0).unwrap();
        assert_eq!(r, Rect { x: 10.0, y: 20.0, w: 30.0, h: 20.0 });
    }

    #[test]
    fn extending_keeps_the_base_selection() {
        let items = [item(1, 0.0, 0.0), item(2, 100.0, 0.0)];
        let mut b = BandSelection::begin(95.0, -5.0, vec![1], true);
        b.update(120.0, 20.0);
        assert_eq!(b.selected(&items), vec![1, 2]);
    }

    #[test]
    fn a_tap_on_empty_space_clears_unless_extending() {
        let items = [item(1, 0.0, 0.0)];
        assert!(BandSelection::begin(90.0, 90.0, vec![1], false).finish(&items).is_empty());
        assert_eq!(BandSelection::begin(90.0, 90.0, vec![1], true).finish(&items), vec![1]);
    }

    #[test]
    fn touching_edges_do_not_select() {
        let a = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let b = Rect { x: 10.0, y: 0.0, w: 10.0, h: 10.0 };
        assert!(!overlaps(a, b));
        assert!(overlaps(a, Rect { x: 9.0, ..b }));
    }

    #[test]
    fn selection_is_sorted_and_unique() {
        let items = [item(7, 0.0, 0.0), item(2, 0.0, 0.0)];
        let mut b = BandSelection::begin(-1.0, -1.0, vec![7], true);
        b.update(50.0, 50.0);
        assert_eq!(b.selected(&items), vec![2, 7]);
    }
}
