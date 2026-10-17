//! Switch Control scanning: the highlight moves through the items of a screen one at a time, and a switch picks the item
//! the highlight is on.
//!
//! Pure model. The shell owns one `Scan` per screen, sized to the number of items the reader can reach. A "next" switch
//! moves the highlight forward and wraps at the end; "select" answers the highlighted item and the shell acts on it. The
//! highlight starts on the first item, and a scan with no items highlights nothing.

/// The highlight over `count` items: `cursor` is the item it is on, `None` before the first step and for an empty scan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    count: usize,
    cursor: Option<usize>,
}

impl Scan {
    /// A scan over `count` items. Nothing is highlighted until the first `next`.
    pub fn new(count: usize) -> Self {
        Scan { count, cursor: None }
    }

    /// The item the highlight is on, if any.
    pub fn highlighted(&self) -> Option<usize> {
        self.cursor.filter(|i| *i < self.count)
    }

    /// The "next" switch: moves the highlight to the following item, wrapping after the last one, and answers it. The first
    /// step highlights item 0. None when there are no items.
    pub fn next(&mut self) -> Option<usize> {
        if self.count == 0 {
            self.cursor = None;
            return None;
        }
        let at = match self.cursor {
            Some(i) if i < self.count => (i + 1) % self.count,
            _ => 0,
        };
        self.cursor = Some(at);
        Some(at)
    }

    /// The "select" switch: the item to act on (the highlighted one), or None when nothing is highlighted.
    pub fn select(&self) -> Option<usize> {
        self.highlighted()
    }

    /// The items changed (a screen opened, a list grew or shrank). The highlight stays on its item when it still exists and
    /// otherwise returns to the start.
    pub fn resize(&mut self, count: usize) {
        self.count = count;
        if self.highlighted().is_none() {
            self.cursor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_step_highlights_the_first_item_and_steps_wrap() {
        let mut s = Scan::new(3);
        assert_eq!(s.highlighted(), None, "nothing is highlighted before the first step");
        assert_eq!(s.next(), Some(0));
        assert_eq!(s.next(), Some(1));
        assert_eq!(s.next(), Some(2));
        assert_eq!(s.next(), Some(0), "the scan wraps after the last item");
    }

    #[test]
    fn select_answers_the_highlighted_item_only() {
        let mut s = Scan::new(2);
        assert_eq!(s.select(), None);
        s.next();
        assert_eq!(s.select(), Some(0));
        assert_eq!(s.select(), Some(0), "selecting does not move the highlight");
    }

    #[test]
    fn an_empty_scan_highlights_and_selects_nothing() {
        let mut s = Scan::new(0);
        assert_eq!(s.next(), None);
        assert_eq!(s.highlighted(), None);
        assert_eq!(s.select(), None);
    }

    #[test]
    fn resizing_keeps_the_highlight_while_its_item_exists() {
        let mut s = Scan::new(5);
        s.next();
        s.next();
        s.next(); // on item 2
        s.resize(4);
        assert_eq!(s.highlighted(), Some(2));
        s.resize(2);
        assert_eq!(s.highlighted(), None, "an item that is gone drops the highlight");
        assert_eq!(s.next(), Some(0), "the next step starts again from the top");
    }
}
