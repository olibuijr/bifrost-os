//! Recognised items from a live camera scan: text and barcode items, as a pure model.
//! The camera feeds each frame's observations in with `ScanSession::update`; the session reports which items
//! were added, updated or removed since the previous frame, the same delta a scanner delegate receives. Recognition itself (text or code decoding) is done by the caller; this module owns identity and diffing.

use std::collections::BTreeMap;

use crate::Rect;

/// A position in logical points, in the coordinates of the camera view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// The four corners of an observed item, in order around its outline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quad(pub [Point; 4]);

impl Quad {
    /// The smallest axis-aligned rectangle that holds the four corners.
    pub fn bounds(&self) -> Rect {
        let (mut x0, mut y0) = (f32::MAX, f32::MAX);
        let (mut x1, mut y1) = (f32::MIN, f32::MIN);
        for p in self.0 {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    /// True when `p` is inside the outline. Works for any convex or concave quadrilateral with a consistent
    /// winding, by counting edge crossings of a ray to the right of the point.
    pub fn contains(&self, p: Point) -> bool {
        let mut inside = false;
        for i in 0..4 {
            let a = self.0[i];
            let b = self.0[(i + 1) % 4];
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if p.x < x {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// What the scanner recognised in an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A run of recognised text.
    Text,
    /// A decoded machine-readable code (barcode or QR).
    Barcode,
}

/// One recognised item. `id` is stable across frames for the same thing on screen; the caller assigns it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: u64,
    pub kind: ItemKind,
    pub payload: String,
    pub outline: Quad,
}

/// The change from one frame to the next, with each list in ascending id order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<u64>,
    pub updated: Vec<u64>,
    pub removed: Vec<u64>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// The items visible in a live scan. Items keep their last observed state until a frame stops reporting them.
#[derive(Debug, Default)]
pub struct ScanSession {
    items: BTreeMap<u64, Item>,
}

impl ScanSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the visible set with one frame's observations and reports the delta. An item counts as
    /// updated only when its kind, payload or outline changed; a repeated identical observation is silent.
    pub fn update(&mut self, observed: Vec<Item>) -> Changes {
        let mut next: BTreeMap<u64, Item> = BTreeMap::new();
        for item in observed {
            // A duplicate id in one frame keeps the last observation.
            next.insert(item.id, item);
        }
        let mut changes = Changes::default();
        for (id, item) in &next {
            match self.items.get(id) {
                None => changes.added.push(*id),
                Some(old) if old != item => changes.updated.push(*id),
                Some(_) => {}
            }
        }
        for id in self.items.keys() {
            if !next.contains_key(id) {
                changes.removed.push(*id);
            }
        }
        self.items = next;
        changes
    }

    /// The visible items, ordered by id.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.get(&id)
    }

    /// The visible items of one kind whose outline contains `p`, topmost (highest id) first.
    pub fn hit_test(&self, p: Point, kind: ItemKind) -> Vec<&Item> {
        let mut hits: Vec<&Item> = self.items.values().filter(|i| i.kind == kind && i.outline.contains(p)).collect();
        hits.reverse();
        hits
    }

    /// Ends the scan: every visible item is removed, and the removals are reported.
    pub fn stop(&mut self) -> Changes {
        let removed: Vec<u64> = self.items.keys().copied().collect();
        self.items.clear();
        Changes { added: Vec::new(), updated: Vec::new(), removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, s: f32) -> Quad {
        Quad([
            Point { x, y },
            Point { x: x + s, y },
            Point { x: x + s, y: y + s },
            Point { x, y: y + s },
        ])
    }

    fn text(id: u64, payload: &str, outline: Quad) -> Item {
        Item { id, kind: ItemKind::Text, payload: payload.to_string(), outline }
    }

    #[test]
    fn a_new_frame_reports_added_updated_and_removed_items() {
        let mut s = ScanSession::new();
        let first = s.update(vec![text(1, "Hello", square(0.0, 0.0, 10.0)), text(2, "World", square(20.0, 0.0, 10.0))]);
        assert_eq!(first.added, vec![1, 2]);
        assert!(first.updated.is_empty() && first.removed.is_empty());

        // Item 1 changes its text, item 2 is unchanged and so silent, item 3 appears.
        let second = s.update(vec![
            text(1, "Hallo", square(0.0, 0.0, 10.0)),
            text(2, "World", square(20.0, 0.0, 10.0)),
            text(3, "New", square(40.0, 0.0, 10.0)),
        ]);
        assert_eq!(second.added, vec![3]);
        assert_eq!(second.updated, vec![1]);
        assert!(second.removed.is_empty());

        let third = s.update(vec![text(2, "World", square(20.0, 0.0, 10.0))]);
        assert_eq!(third.removed, vec![1, 3]);
        assert!(third.added.is_empty() && third.updated.is_empty());
        assert_eq!(s.items().count(), 1);
    }

    #[test]
    fn an_identical_repeat_frame_reports_nothing() {
        let mut s = ScanSession::new();
        let frame = vec![text(7, "Same", square(0.0, 0.0, 5.0))];
        s.update(frame.clone());
        assert!(s.update(frame).is_empty());
    }

    #[test]
    fn a_duplicate_id_in_one_frame_keeps_the_last_observation() {
        let mut s = ScanSession::new();
        let delta = s.update(vec![text(4, "first", square(0.0, 0.0, 5.0)), text(4, "second", square(0.0, 0.0, 5.0))]);
        assert_eq!(delta.added, vec![4]);
        assert_eq!(s.get(4).map(|i| i.payload.as_str()), Some("second"));
    }

    #[test]
    fn hit_testing_respects_the_outline_and_the_kind() {
        let mut s = ScanSession::new();
        s.update(vec![
            text(1, "under", square(0.0, 0.0, 30.0)),
            Item { id: 2, kind: ItemKind::Barcode, payload: "123".into(), outline: square(0.0, 0.0, 30.0) },
            text(3, "outside", square(100.0, 100.0, 5.0)),
        ]);
        let inside = Point { x: 10.0, y: 10.0 };
        let texts: Vec<u64> = s.hit_test(inside, ItemKind::Text).iter().map(|i| i.id).collect();
        assert_eq!(texts, vec![1], "only the text item of the two overlapping items");
        assert!(s.hit_test(Point { x: 50.0, y: 50.0 }, ItemKind::Text).is_empty());
    }

    #[test]
    fn stopping_the_scan_removes_everything_and_reports_it() {
        let mut s = ScanSession::new();
        s.update(vec![text(9, "a", square(0.0, 0.0, 5.0)), text(8, "b", square(10.0, 0.0, 5.0))]);
        let stopped = s.stop();
        assert_eq!(stopped.removed, vec![8, 9]);
        assert_eq!(s.items().count(), 0);
        assert!(s.stop().is_empty());
    }

    #[test]
    fn quad_bounds_cover_all_corners() {
        let q = Quad([
            Point { x: 4.0, y: 9.0 },
            Point { x: 1.0, y: 2.0 },
            Point { x: 7.0, y: 3.0 },
            Point { x: 5.0, y: 11.0 },
        ]);
        assert_eq!(q.bounds(), Rect { x: 1.0, y: 2.0, w: 6.0, h: 9.0 });
    }
}
