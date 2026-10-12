//! PDF page overlays and markup mode as a pure model. An overlay is a view an app places over one page of a document.
//! It is registered per page with a frame in page points, and it is shown while its page is on screen. In markup mode
//! the overlays take the pointer; outside markup mode they are passive and never receive hits.
//!
//! The model keeps the overlay per page (one overlay for each page, as the provider API gives one view per page), the
//! set of pages currently displayed, and the z order between overlays. `show_pages` reports which overlays start and
//! which end displaying, so a view can call its display hooks. Nothing here draws or receives real events.
//!
//! Rules: a page has at most one overlay. A frame with a width or height of zero or less is refused. Hit testing
//! returns the page's overlay when its frame contains the point (left and top edges inclusive, right and bottom
//! edges exclusive), and only in markup mode. The z value is kept for the view that stacks overlays against page
//! content; the model does not sort by it.

use std::collections::BTreeSet;

/// A rectangle in page points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    /// True when the point lies inside the frame, with the left and top edges inclusive.
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// One overlay placed over a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    pub id: u32,
    pub page: usize,
    pub frame: Frame,
    pub z: i32,
}

/// Why an overlay was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayError {
    /// The page already has an overlay.
    PageTaken(usize),
    /// The frame has no area.
    EmptyFrame,
}

/// The overlays of one document view.
#[derive(Clone, Debug, Default)]
pub struct OverlayStack {
    overlays: Vec<Overlay>,
    displayed: BTreeSet<usize>,
    markup: bool,
    next_id: u32,
}

/// The overlays that started or stopped displaying after a change of visible pages.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DisplayChange {
    /// Overlay ids whose page came on screen, in page order.
    pub started: Vec<u32>,
    /// Overlay ids whose page left the screen, in page order.
    pub ended: Vec<u32>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether markup mode is on. Only then do overlays take the pointer.
    pub fn is_in_markup_mode(&self) -> bool {
        self.markup
    }

    /// Turns markup mode on or off.
    pub fn set_markup_mode(&mut self, on: bool) {
        self.markup = on;
    }

    /// Places an overlay over `page` and returns its id.
    pub fn add(&mut self, page: usize, frame: Frame, z: i32) -> Result<u32, OverlayError> {
        if frame.w <= 0.0 || frame.h <= 0.0 {
            return Err(OverlayError::EmptyFrame);
        }
        if self.overlays.iter().any(|o| o.page == page) {
            return Err(OverlayError::PageTaken(page));
        }
        self.next_id += 1;
        let id = self.next_id;
        self.overlays.push(Overlay { id, page, frame, z });
        Ok(id)
    }

    /// Removes the overlay on `page`, if any. Returns its id.
    pub fn remove_page(&mut self, page: usize) -> Option<u32> {
        let pos = self.overlays.iter().position(|o| o.page == page)?;
        Some(self.overlays.remove(pos).id)
    }

    /// The overlay on `page`, if one is placed.
    pub fn overlay_for_page(&self, page: usize) -> Option<&Overlay> {
        self.overlays.iter().find(|o| o.page == page)
    }

    /// Replaces the set of displayed pages and reports which overlays started or ended displaying. Pages without an
    /// overlay do not appear in the change.
    pub fn show_pages(&mut self, pages: &[usize]) -> DisplayChange {
        let next: BTreeSet<usize> = pages.iter().copied().collect();
        let mut change = DisplayChange::default();
        for &p in next.difference(&self.displayed) {
            if let Some(o) = self.overlay_for_page(p) {
                change.started.push(o.id);
            }
        }
        for &p in self.displayed.difference(&next) {
            if let Some(o) = self.overlay_for_page(p) {
                change.ended.push(o.id);
            }
        }
        self.displayed = next;
        change
    }

    /// The overlay that takes a pointer at (`page`, `px`, `py`). None outside markup mode, and None when no
    /// overlay on the page contains the point.
    pub fn hit_test(&self, page: usize, px: f64, py: f64) -> Option<u32> {
        if !self.markup {
            return None;
        }
        let o = self.overlay_for_page(page)?;
        if o.frame.contains(px, py) {
            Some(o.id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f64, y: f64, w: f64, h: f64) -> Frame {
        Frame { x, y, w, h }
    }

    #[test]
    fn one_overlay_per_page_and_no_empty_frames() {
        let mut s = OverlayStack::new();
        assert_eq!(s.add(0, frame(0.0, 0.0, 100.0, 50.0), 0), Ok(1));
        assert_eq!(s.add(0, frame(0.0, 0.0, 10.0, 10.0), 0), Err(OverlayError::PageTaken(0)));
        assert_eq!(s.add(1, frame(0.0, 0.0, 0.0, 10.0), 0), Err(OverlayError::EmptyFrame));
        assert_eq!(s.add(1, frame(0.0, 0.0, 10.0, -1.0), 0), Err(OverlayError::EmptyFrame));
        assert_eq!(s.add(1, frame(0.0, 0.0, 10.0, 10.0), 0), Ok(2));
    }

    #[test]
    fn overlays_take_hits_only_in_markup_mode() {
        let mut s = OverlayStack::new();
        s.add(0, frame(10.0, 10.0, 100.0, 50.0), 0).unwrap();
        assert!(!s.is_in_markup_mode());
        assert_eq!(s.hit_test(0, 20.0, 20.0), None);
        s.set_markup_mode(true);
        assert_eq!(s.hit_test(0, 20.0, 20.0), Some(1));
    }

    #[test]
    fn edges_are_left_top_inclusive_and_right_bottom_exclusive() {
        let mut s = OverlayStack::new();
        s.add(0, frame(10.0, 10.0, 100.0, 50.0), 0).unwrap();
        s.set_markup_mode(true);
        assert_eq!(s.hit_test(0, 10.0, 10.0), Some(1));
        assert_eq!(s.hit_test(0, 109.9, 59.9), Some(1));
        assert_eq!(s.hit_test(0, 110.0, 30.0), None);
        assert_eq!(s.hit_test(0, 50.0, 60.0), None);
        assert_eq!(s.hit_test(0, 9.9, 20.0), None);
    }

    #[test]
    fn hits_go_to_the_page_that_has_the_overlay() {
        let mut s = OverlayStack::new();
        s.add(2, frame(0.0, 0.0, 50.0, 50.0), 0).unwrap();
        s.set_markup_mode(true);
        assert_eq!(s.hit_test(2, 5.0, 5.0), Some(1));
        assert_eq!(s.hit_test(1, 5.0, 5.0), None);
        assert_eq!(s.hit_test(3, 5.0, 5.0), None);
    }

    #[test]
    fn display_changes_report_started_and_ended_overlays() {
        let mut s = OverlayStack::new();
        s.add(0, frame(0.0, 0.0, 10.0, 10.0), 0).unwrap(); // id 1
        s.add(1, frame(0.0, 0.0, 10.0, 10.0), 0).unwrap(); // id 2
        s.add(4, frame(0.0, 0.0, 10.0, 10.0), 0).unwrap(); // id 3
        let first = s.show_pages(&[0, 1]);
        assert_eq!(first, DisplayChange { started: vec![1, 2], ended: vec![] });
        let second = s.show_pages(&[1, 4, 7]);
        assert_eq!(second, DisplayChange { started: vec![3], ended: vec![1] });
        // Page 7 has no overlay, so it adds nothing.
        let third = s.show_pages(&[1, 4, 7]);
        assert_eq!(third, DisplayChange::default());
    }

    #[test]
    fn removing_a_page_frees_it_for_a_new_overlay() {
        let mut s = OverlayStack::new();
        s.add(3, frame(0.0, 0.0, 10.0, 10.0), 0).unwrap();
        assert_eq!(s.remove_page(3), Some(1));
        assert_eq!(s.remove_page(3), None);
        assert_eq!(s.overlay_for_page(3), None);
        assert_eq!(s.add(3, frame(0.0, 0.0, 10.0, 10.0), 0), Ok(2));
    }
}
