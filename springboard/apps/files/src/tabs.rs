//! Tabs: several folder views in one window (File > New Tab, ⌘T; ⌘W closes; ⌃Tab and ⌃⇧Tab step through).
//! The open tab's navigation (path, history) lives in `Files`; the other tabs are parked as `TabState` in the
//! `TabStrip`. Switching parks the open tab's state, then shows the chosen one through `navigate`.
use crate::*;

/// The strip's height under the toolbar, in points. It is drawn only while there are two or more tabs.
pub(crate) const TAB_STRIP_H: f32 = 30.;
/// The widest a tab cell is drawn, in points.
const TAB_MAX_W: f32 = 180.;
/// The close box at a tab's right end: its width and its inset from the cell's edge.
const CLOSE_W: f32 = 16.;
const CLOSE_INSET: f32 = 6.;

/// Where each of `n` tab cells sits between `left` and `right`, as `(x, width)`; each is at most `TAB_MAX_W` wide.
pub(crate) fn cells(n: usize, left: f32, right: f32) -> Vec<(f32, f32)> {
    if n == 0 {
        return Vec::new();
    }
    let w = ((right - left) / n as f32).clamp(0., TAB_MAX_W);
    (0..n).map(|i| (left + i as f32 * w, w)).collect()
}

/// What a click on the strip lands on: a tab's body (select it) or its close box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabHit {
    Select(usize),
    Close(usize),
}

/// The hit at `x` across the cells from `cells`, or None between or beyond them.
pub(crate) fn hit(cells: &[(f32, f32)], x: f32) -> Option<TabHit> {
    let (i, &(x0, w)) = cells.iter().enumerate().find(|(_, (x0, w))| x >= *x0 && x < x0 + w)?;
    let close_x = x0 + w - CLOSE_INSET - CLOSE_W;
    Some(if x >= close_x && x < close_x + CLOSE_W { TabHit::Close(i) } else { TabHit::Select(i) })
}

/// The navigation a parked tab keeps: where it is and the history to go back and forward through.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabState {
    pub(crate) path: PathBuf,
    pub(crate) history: Vec<PathBuf>,
    pub(crate) at: usize,
}

/// The tab strip model: the tabs in order and which one is open. Holds the open tab too, kept current by `park`.
#[derive(Debug)]
pub(crate) struct TabStrip<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> TabStrip<T> {
    /// A strip with one tab, which is open.
    pub(crate) fn new(first: T) -> Self {
        Self { items: vec![first], active: 0 }
    }
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
    pub(crate) fn active(&self) -> usize {
        self.active
    }
    pub(crate) fn items(&self) -> &[T] {
        &self.items
    }
    /// Replaces the open tab's stored state with `item` (called before the open tab is left).
    pub(crate) fn park(&mut self, item: T) {
        self.items[self.active] = item;
    }
    /// Adds a tab right after the open one and makes it the open tab. Returns its index.
    pub(crate) fn insert_after_active(&mut self, item: T) -> usize {
        let at = self.active + 1;
        self.items.insert(at, item);
        self.active = at;
        at
    }
    /// Makes tab `i` the open tab. Returns false when there is no such tab.
    pub(crate) fn select(&mut self, i: usize) -> bool {
        if i >= self.items.len() {
            return false;
        }
        self.active = i;
        true
    }
    /// Steps to the next (or previous) tab, wrapping around. Returns the new open tab.
    pub(crate) fn step(&mut self, forward: bool) -> usize {
        let n = self.items.len();
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
        self.active
    }
    /// Closes tab `i`; the last tab cannot be closed. The open tab moves to the tab that takes its place,
    /// or to the one before it when the last tab closes. Returns the removed item.
    pub(crate) fn close(&mut self, i: usize) -> Option<T> {
        if self.items.len() <= 1 || i >= self.items.len() {
            return None;
        }
        let item = self.items.remove(i);
        if i < self.active {
            self.active -= 1;
        } else if i == self.active {
            self.active = i.min(self.items.len() - 1);
        }
        Some(item)
    }
}

impl Files {
    /// The open tab's navigation, as the strip stores it.
    fn tab_state(&self) -> TabState {
        TabState { path: self.path.clone(), history: self.history.clone(), at: self.at }
    }
    /// The strip's height now: 0 while there is only one tab.
    pub(crate) fn tab_strip_h(&self) -> f32 {
        if self.tabs.len() > 1 { TAB_STRIP_H } else { 0. }
    }
    /// The top of the content under the toolbar, the strip and the search scope bar.
    pub(crate) fn body_top(&self) -> f32 {
        HEADER_H + self.tab_strip_h() + self.scope_bar_h()
    }
    /// The top of the first row: the column headers and the list header are 34 points tall.
    pub(crate) fn rows_top(&self) -> f32 {
        self.body_top() + 34.
    }
    /// The tab cells across the content column, right of the sidebar.
    pub(crate) fn tab_cells(&self) -> Vec<(f32, f32)> {
        cells(self.tabs.len(), self.side() + 8., self.w - 8.)
    }
    /// What a click at (x, y) lands on in the strip, when the strip is drawn there.
    pub(crate) fn tab_hit(&self, x: f32, y: f32) -> Option<TabHit> {
        if self.tab_strip_h() == 0. || x < self.side() || !(HEADER_H..HEADER_H + TAB_STRIP_H).contains(&y) {
            return None;
        }
        hit(&self.tab_cells(), x)
    }
    /// A tab's name: the window title for the open tab, else its folder's name.
    pub(crate) fn tab_label(&self, i: usize) -> String {
        if i == self.tabs.active() {
            return self.title();
        }
        let path = self.tabs.items().get(i).map_or(Path::new("/"), |t| t.path.as_path());
        path.file_name().map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned())
    }
    /// Draws the strip under the toolbar: one cell per tab, the open one raised, each with a close box.
    pub(crate) fn paint_tab_strip(&self, ui: &Ui, s: &mut Surface) {
        if self.tab_strip_h() == 0. {
            return;
        }
        let (x0, y0) = (self.side(), HEADER_H);
        rect(s, ui, x0, y0, self.w - x0, TAB_STRIP_H, CARD_BG.now());
        rect(s, ui, x0, y0 + TAB_STRIP_H - 1., self.w - x0, 1., SEPARATOR_C.now());
        for (i, (x, w)) in self.tab_cells().into_iter().enumerate() {
            let open = i == self.tabs.active();
            if open {
                rect(s, ui, x + 2., y0 + 4., w - 4., TAB_STRIP_H - 5., CONTENT_BG.now());
            }
            let ink = if open { LABEL.now() } else { LABEL_2.now() };
            let room = w - 12. - CLOSE_W - 2. * CLOSE_INSET;
            text(s, ui, &self.tab_label(i), 13., ink, x + 12., y0 + 7., room);
            let close_x = x + w - CLOSE_INSET - CLOSE_W;
            text(s, ui, "×", 13., LABEL_2.now(), close_x + 4., y0 + 6., CLOSE_W);
        }
    }
    /// Makes tab `i` the open tab (a click on its cell). The open tab's navigation is parked first.
    pub(crate) fn select_tab(&mut self, i: usize) {
        if i == self.tabs.active() || i >= self.tabs.len() {
            return;
        }
        let state = self.tab_state();
        self.tabs.park(state);
        self.tabs.select(i);
        self.show_tab();
        self.revision += 1;
    }
    /// Closes tab `i` from its close box. Closing the open tab is ⌘W; another tab closes and the open one stays.
    pub(crate) fn close_tab_at(&mut self, i: usize) {
        if i == self.tabs.active() {
            return self.close_tab();
        }
        let state = self.tab_state();
        self.tabs.park(state);
        if self.tabs.close(i).is_some() {
            self.revision += 1;
        }
    }
    /// Shows the open tab's stored navigation: its folder with its own history.
    fn show_tab(&mut self) {
        let Some(tab) = self.tabs.items().get(self.tabs.active()).cloned() else {
            return;
        };
        self.history = tab.history;
        self.at = tab.at;
        self.navigate(tab.path, false);
    }
    /// File > New Tab (⌘T): a tab at the open folder, which becomes the open tab.
    pub(crate) fn new_tab(&mut self) {
        let state = self.tab_state();
        self.tabs.park(state);
        let fresh = TabState { path: self.path.clone(), history: vec![self.path.clone()], at: 0 };
        self.tabs.insert_after_active(fresh);
        self.show_tab();
        self.revision += 1;
    }
    /// Closes the open tab (⌘W). The last tab stays open.
    pub(crate) fn close_tab(&mut self) {
        let open = self.tabs.active();
        if self.tabs.close(open).is_some() {
            self.show_tab();
            self.revision += 1;
        }
    }
    /// Steps to the next or previous tab (⌃Tab and ⌃⇧Tab).
    pub(crate) fn step_tab(&mut self, forward: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let state = self.tab_state();
        self.tabs.park(state);
        self.tabs.step(forward);
        self.show_tab();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tab_opens_after_the_open_one_and_becomes_open() {
        let mut strip = TabStrip::new("a");
        assert_eq!(strip.insert_after_active("b"), 1);
        assert_eq!(strip.active(), 1);
        strip.select(0);
        assert_eq!(strip.insert_after_active("c"), 1);
        assert_eq!(strip.items(), &["a", "c", "b"]);
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut strip = TabStrip::new(1);
        strip.insert_after_active(2);
        strip.insert_after_active(3);
        assert_eq!(strip.active(), 2);
        assert_eq!(strip.step(true), 0);
        assert_eq!(strip.step(false), 2);
        assert_eq!(strip.step(false), 1);
    }

    #[test]
    fn the_last_tab_cannot_close_and_the_open_tab_moves_to_a_neighbour() {
        let mut strip = TabStrip::new('x');
        assert_eq!(strip.close(0), None);
        strip.insert_after_active('y');
        strip.insert_after_active('z');
        // open is 'z' (index 2); closing an earlier tab keeps 'z' open
        assert_eq!(strip.close(0), Some('x'));
        assert_eq!(strip.items(), &['y', 'z']);
        assert_eq!(strip.active(), 1);
        // closing the open last tab falls back to the one before it
        assert_eq!(strip.close(1), Some('z'));
        assert_eq!(strip.active(), 0);
        assert!(!strip.select(5));
    }

    #[test]
    fn cells_split_the_strip_and_cap_each_width() {
        assert!(cells(0, 0., 100.).is_empty());
        assert_eq!(cells(2, 0., 200.), vec![(0., 100.), (100., 100.)]);
        assert_eq!(cells(1, 0., 1000.), vec![(0., TAB_MAX_W)]);
    }

    #[test]
    fn a_click_lands_on_the_tab_or_on_its_close_box() {
        let c = cells(2, 0., 200.);
        // tab 0's close box spans [78, 94): 6 points in from the cell's right edge, 16 wide
        assert_eq!(hit(&c, 10.), Some(TabHit::Select(0)));
        assert_eq!(hit(&c, 80.), Some(TabHit::Close(0)));
        assert_eq!(hit(&c, 93.9), Some(TabHit::Close(0)));
        assert_eq!(hit(&c, 94.), Some(TabHit::Select(0)));
        assert_eq!(hit(&c, 150.), Some(TabHit::Select(1)));
        assert_eq!(hit(&c, 200.), None);
        assert_eq!(hit(&c, -1.), None);
    }

    #[test]
    fn parking_keeps_each_tabs_own_state() {
        let mut strip = TabStrip::new(TabState { path: "/a".into(), history: vec!["/a".into()], at: 0 });
        strip.park(TabState { path: "/a/b".into(), history: vec!["/a".into(), "/a/b".into()], at: 1 });
        assert_eq!(strip.items()[0].path, PathBuf::from("/a/b"));
        assert_eq!(strip.items()[0].at, 1);
    }
}
