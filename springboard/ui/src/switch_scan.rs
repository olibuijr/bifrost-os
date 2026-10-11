//! Switch Control item scanning (`AccessibilityTechnology.switchControl`): a highlight walks the elements of a screen in
//! reading order, and one switch press selects the highlighted element. Auto scanning moves the highlight on a timer;
//! manual scanning moves it on each step press. Disabled elements are skipped, as they are for keyboard focus, so the
//! order comes from `focus::next`. This is a pure model: the caller supplies the clock in milliseconds and draws the
//! highlight. No switch hardware is read here.

use crate::focus::{self, Focusable};

/// How the highlight moves once scanning has started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// The highlight advances by itself every `interval_ms`, counted from the last move.
    Auto { interval_ms: u64 },
    /// The highlight advances only when the caller steps it.
    Manual,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scanner {
    items: Vec<Focusable>,
    style: Style,
    current: Option<u64>,
    running: bool,
    last_move_ms: u64,
}

impl Scanner {
    /// A scanner over `items`, in any order (reading order is applied). Nothing is highlighted until [`Scanner::start`].
    pub fn new(items: Vec<Focusable>, style: Style) -> Scanner {
        Scanner { items, style, current: None, running: false, last_move_ms: 0 }
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// The element the highlight is on, if scanning is running and there is one.
    pub fn highlighted(&self) -> Option<u64> {
        self.current
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Starts scanning at `now_ms` and highlights the first element in reading order. Returns it, or None when there are
    /// no enabled elements.
    pub fn start(&mut self, now_ms: u64) -> Option<u64> {
        self.running = true;
        self.last_move_ms = now_ms;
        self.current = focus::next(&self.items, None, true);
        self.current
    }

    /// Auto scanning: moves the highlight once `interval_ms` has passed since the last move. Returns the highlight after the
    /// move, or None when nothing moved. Manual scanning and a stopped scanner never move on a tick.
    pub fn tick(&mut self, now_ms: u64) -> Option<u64> {
        match self.style {
            Style::Auto { interval_ms } if self.running && now_ms.saturating_sub(self.last_move_ms) >= interval_ms => {
                self.last_move_ms = now_ms;
                self.advance(true)
            }
            _ => None,
        }
    }

    /// Moves the highlight to the next (`forward`) or previous element and restarts the auto-scan timer. Returns the new
    /// highlight. Does nothing while stopped.
    pub fn step(&mut self, now_ms: u64, forward: bool) -> Option<u64> {
        if !self.running {
            return None;
        }
        self.last_move_ms = now_ms;
        self.advance(forward)
    }

    /// A switch press that selects: returns the highlighted element and ends the scan, so the highlight goes back to the
    /// start. Returns None when nothing is highlighted.
    pub fn select(&mut self) -> Option<u64> {
        self.running = false;
        self.current.take()
    }

    /// Ends the scan and clears the highlight.
    pub fn stop(&mut self) {
        self.running = false;
        self.current = None;
    }

    /// Replaces the elements because the screen changed. A highlight that is gone or disabled is dropped, and the next step
    /// starts again from the first element.
    pub fn set_items(&mut self, items: Vec<Focusable>) {
        if let Some(c) = self.current {
            if !items.iter().any(|f| f.id == c && f.enabled) {
                self.current = None;
            }
        }
        self.items = items;
    }

    fn advance(&mut self, forward: bool) -> Option<u64> {
        self.current = focus::next(&self.items, self.current, forward);
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, x: f32, y: f32, enabled: bool) -> Focusable {
        Focusable { id, x, y, w: 40.0, h: 20.0, enabled }
    }

    #[test]
    fn scanning_follows_reading_order_and_wraps() {
        // Row one holds 1 then 2 (left to right); row two holds 3. Given out of order on purpose.
        let mut s = Scanner::new(vec![item(3, 0.0, 100.0, true), item(2, 80.0, 0.0, true), item(1, 0.0, 0.0, true)], Style::Manual);
        assert_eq!(s.start(0), Some(1));
        assert_eq!(s.step(10, true), Some(2));
        assert_eq!(s.step(20, true), Some(3));
        assert_eq!(s.step(30, true), Some(1));
    }

    #[test]
    fn disabled_elements_are_skipped() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, false), item(3, 0.0, 60.0, true)], Style::Manual);
        s.start(0);
        assert_eq!(s.step(1, true), Some(3));
    }

    #[test]
    fn a_backward_step_from_the_first_element_wraps_to_the_last() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, true), item(3, 0.0, 60.0, true)], Style::Manual);
        s.start(0);
        assert_eq!(s.step(1, false), Some(3));
    }

    #[test]
    fn auto_scan_moves_only_after_its_interval() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, true)], Style::Auto { interval_ms: 1000 });
        s.start(0);
        assert_eq!(s.tick(999), None);
        assert_eq!(s.highlighted(), Some(1));
        assert_eq!(s.tick(1000), Some(2));
        assert_eq!(s.tick(1500), None);
        assert_eq!(s.tick(2000), Some(1));
    }

    #[test]
    fn manual_scan_ignores_the_clock() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, true)], Style::Manual);
        s.start(0);
        assert_eq!(s.tick(10_000), None);
        assert_eq!(s.highlighted(), Some(1));
    }

    #[test]
    fn selecting_returns_the_highlight_and_ends_the_scan() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, true)], Style::Manual);
        assert_eq!(s.select(), None);
        s.start(0);
        s.step(1, true);
        assert_eq!(s.select(), Some(2));
        assert!(!s.is_running());
        assert_eq!(s.highlighted(), None);
        assert_eq!(s.step(2, true), None);
    }

    #[test]
    fn an_empty_screen_has_nothing_to_highlight() {
        let mut s = Scanner::new(Vec::new(), Style::Auto { interval_ms: 500 });
        assert_eq!(s.start(0), None);
        assert_eq!(s.tick(500), None);
        assert_eq!(s.step(600, true), None);
    }

    #[test]
    fn a_vanished_highlight_restarts_from_the_first_element() {
        let mut s = Scanner::new(vec![item(1, 0.0, 0.0, true), item(2, 0.0, 30.0, true), item(3, 0.0, 60.0, true)], Style::Manual);
        s.start(0);
        s.step(1, true);
        assert_eq!(s.highlighted(), Some(2));
        s.set_items(vec![item(1, 0.0, 0.0, true), item(3, 0.0, 60.0, true)]);
        assert_eq!(s.highlighted(), None);
        assert_eq!(s.step(2, true), Some(1));
    }
}
