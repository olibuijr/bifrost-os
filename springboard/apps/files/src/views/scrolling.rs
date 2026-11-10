//! How the item views scroll: the line geometry they share, the scroll range, bringing an item into view, the view
//! switch that keeps the selection and the place, and the scroll knob.
use super::column::ROW_H;
use crate::*;
use bifrost_app::controls::scroll::{indicator, INDICATOR_INSET_PT, INDICATOR_MIN_PT, INDICATOR_W_PT};

/// Room kept under the last line when the view is scrolled to its end (points).
const END_ROOM: f32 = 70.;

impl Files {
    /// The lines of the view that scrolls: the first line's top at scroll 0, the line pitch and the items on a line.
    /// None for the gallery, whose strip follows the focused item instead of scrolling.
    pub(crate) fn lines(&self) -> Option<(f32, f32, usize)> {
        if self.vo.gallery {
            None
        } else if self.columns {
            Some((self.rows_top(), ROW_H, 1))
        } else if self.grid {
            Some((self.rows_top() + 16., self.grid_h(), self.grid_cols()))
        } else {
            Some((self.rows_top(), 32., 1))
        }
    }
    /// How far a view of `count` items scrolls: until the last line has `END_ROOM` under it.
    pub(crate) fn scroll_range(&self, count: usize) -> f32 {
        let Some((_, step, per)) = self.lines() else { return 0. };
        let total = count.div_ceil(per) as f32 * step;
        (total - (self.h - self.rows_top() - END_ROOM)).max(0.)
    }
    /// How far the shown view scrolls.
    pub(crate) fn scroll_max(&self) -> f32 {
        self.scroll_range(self.visible().len())
    }
    /// The scroll that brings a line (its top at scroll 0 and its height) fully into view by the shortest way, from
    /// `scroll`: unchanged when the line is in view already.
    fn nearest(&self, scroll: f32, top: f32, height: f32, gap: f32) -> f32 {
        let (view_top, view_bottom) = (self.rows_top(), self.bottom());
        if top - gap - scroll < view_top {
            (top - gap - view_top).max(0.)
        } else if top + height - scroll > view_bottom {
            (top + height - view_bottom).max(0.)
        } else {
            scroll
        }
    }
    /// The top (at scroll 0) and height of the line that holds row `i` (an index into `rows`), and the gap the icon
    /// view leaves above its first line.
    fn line_of(&self, i: usize) -> Option<(f32, f32, f32)> {
        let (top0, step, per) = self.lines()?;
        let n = self.visible().iter().position(|&j| j == i)?;
        Some((top0 + (n / per) as f32 * step, step, top0 - self.rows_top()))
    }
    /// Scrolls just far enough that row `i` is fully on screen: what the arrow keys and a rename do.
    pub(crate) fn reveal_nearest(&mut self, i: usize) {
        if let Some((top, height, gap)) = self.line_of(i) {
            self.scroll = self.nearest(self.scroll, top, height, gap);
        }
    }
    /// Brings row `i` into view for someone who did not scroll there (Show in Files, a new folder, a view switch): a
    /// row that is fully in view stays where it is, any other lands in the middle of the view, or as near to it as
    /// the scroll range allows.
    pub(crate) fn reveal_comfortably(&mut self, i: usize) {
        let Some((top, height, gap)) = self.line_of(i) else { return };
        if self.nearest(self.scroll, top, height, gap) == self.scroll {
            return;
        }
        let middle = (self.rows_top() + self.bottom()) / 2.;
        let wanted = (top + height / 2. - middle).clamp(0., self.scroll_max());
        // a view shorter than its scroll range's end room: the row is in view whatever else
        self.scroll = self.nearest(wanted, top, height, gap);
    }
    /// The first item that is fully in view, when the view has been scrolled: where a view switch keeps its place.
    fn first_shown(&self) -> Option<PathBuf> {
        if self.scroll <= 0. {
            return None;
        }
        let (top0, step, per) = self.lines()?;
        let line = ((self.scroll - (top0 - self.rows_top())) / step).ceil().max(0.) as usize;
        let visible = self.visible();
        let i = visible.get(line * per).or(visible.last())?;
        self.rows.get(*i).map(|e| e.path.clone())
    }
    /// Shows the folder in another view. The selection stays (by path: the rows of an inline folder only exist in the
    /// list view, so the indices move), the focused item stays the focus, and the new view is scrolled so that the
    /// focused item is in view, or else the item that was first in view before.
    pub(crate) fn switch_view(&mut self, grid: bool, columns: bool, gallery: bool) {
        let keep = self.selected_paths();
        let focus = self.focus_entry().map(|e| e.path.clone());
        let first = self.first_shown();
        self.grid = grid;
        self.columns = columns;
        self.vo.gallery = gallery;
        self.refresh_rows(); // inline folders only exist in the list view
        self.reselect(&keep);
        // `reselect` orders the selection by row: the focus is the last one picked, so it goes back to the end
        let row_of = |files: &Files, path: &PathBuf| files.rows.iter().position(|e| e.path == *path);
        if let Some(at) = focus.and_then(|p| row_of(&*self, &p)).and_then(|i| self.selection.iter().position(|&j| j == i)) {
            let i = self.selection.remove(at);
            self.selection.push(i);
        }
        self.scroll = 0.;
        if let Some(i) = self.focus() {
            self.reveal_comfortably(i);
        } else if let Some((top, _, gap)) = first.and_then(|p| row_of(&*self, &p)).and_then(|i| self.line_of(i)) {
            self.scroll = (top - gap - self.rows_top()).clamp(0., self.scroll_max());
        }
        self.revision += 1;
    }
    /// The scroll knob of the list and icon views (x, y, width, length in points), while there is more than fits.
    pub(crate) fn scroll_knob(&self, count: usize) -> Option<(f32, f32, f32, f32)> {
        if self.columns || self.vo.gallery {
            return None;
        }
        let top = (if self.grid { self.body_top() } else { self.rows_top() }) + INDICATOR_INSET_PT;
        let track = self.bottom() - INDICATOR_INSET_PT - top;
        let (y, len) = indicator(track, self.scroll_range(count), self.scroll, INDICATOR_MIN_PT)?;
        Some((self.w - INDICATOR_INSET_PT - INDICATOR_W_PT, top + y, INDICATOR_W_PT, len))
    }
    /// Draws the scroll knob over the trailing edge of the list or icon view.
    pub(crate) fn paint_scroll_knob(&self, ui: &Ui, s: &mut Surface, count: usize) {
        if let Some((x, y, w, len)) = self.scroll_knob(count) {
            s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(len), ui.pt(w / 2.), LABEL_2.now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, folder};

    fn items(n: usize) -> Files {
        fixture((0..n).map(|i| folder(&format!("f{i:03}"))).collect())
    }
    /// The top and bottom of row `i`'s line on screen.
    fn on_screen(files: &Files, i: usize) -> (f32, f32) {
        let (top, height, _) = files.line_of(i).unwrap();
        (top - files.scroll, top + height - files.scroll)
    }
    fn fully_in_view(files: &Files, i: usize) -> bool {
        let (top, bottom) = on_screen(files, i);
        top >= files.rows_top() && bottom <= files.bottom()
    }

    #[test]
    fn a_revealed_row_far_down_the_list_lands_in_the_middle_and_not_on_the_bottom_edge() {
        let mut files = items(100);
        files.grid = false;
        files.reveal_comfortably(3);
        assert_eq!(files.scroll, 0., "a row in view already does not move the list");
        files.reveal_comfortably(60);
        let (top, bottom) = on_screen(&files, 60);
        assert!(fully_in_view(&files, 60));
        let middle = (files.rows_top() + files.bottom()) / 2.;
        assert!(((top + bottom) / 2. - middle).abs() < 0.01, "the row sits in the middle of the view: {top}..{bottom}");
        assert!(top - files.rows_top() >= 64. && files.bottom() - bottom >= 64., "with rows above and below it");
        let seen = files.scroll;
        files.reveal_comfortably(61);
        assert_eq!(files.scroll, seen, "a neighbour that is in view does not move it");
        // the last row cannot reach the middle: it lands as far up as the list scrolls, whole and above the bottom edge
        files.reveal_comfortably(99);
        assert_eq!(files.scroll, files.scroll_max());
        assert!(fully_in_view(&files, 99));
        assert!(files.bottom() - on_screen(&files, 99).1 > 0., "not on the last pixel row");
    }

    #[test]
    fn the_arrow_keys_reveal_still_scrolls_no_further_than_it_must() {
        let mut files = items(100);
        files.grid = false;
        files.reveal_nearest(60);
        assert_eq!(on_screen(&files, 60).1, files.bottom());
        files.reveal_nearest(2);
        assert_eq!(on_screen(&files, 2).0, files.rows_top());
    }

    #[test]
    fn every_scrolling_view_brings_a_revealed_item_into_view() {
        // (grid, columns): icon, list and columns views
        for (grid, columns) in [(true, false), (false, false), (false, true)] {
            let mut files = items(100);
            (files.grid, files.columns) = (grid, columns);
            files.select_only(99);
            files.reveal_comfortably(99);
            assert!(files.scroll > 0., "the view scrolled (grid {grid}, columns {columns})");
            assert!(fully_in_view(&files, 99), "the item is in view (grid {grid}, columns {columns})");
            if !grid {
                assert_eq!(files.item_at(files.side() + 30., on_screen(&files, 99).0 + 4.), Some(99), "a click on the row hits it (columns {columns})");
            }
        }
        // the gallery has no scroll: its strip follows the focus
        let mut files = items(100);
        files.set_gallery();
        files.reveal_comfortably(99);
        assert_eq!(files.scroll, 0.);
        assert!(files.lines().is_none());
    }

    #[test]
    fn a_view_switch_keeps_the_selection_and_scrolls_to_it() {
        let mut files = items(100);
        files.set_view(false, false);
        files.selection = vec![70, 40, 55];
        files.reveal_nearest(55);
        let paths = |files: &Files| files.selected_paths();
        let before = paths(&files);
        // list to icons, columns, gallery and back to the list
        for (grid, columns, gallery) in [(true, false, false), (false, true, false), (false, false, true), (false, false, false)] {
            if gallery {
                files.set_gallery();
            } else {
                files.set_view(grid, columns);
            }
            let mut now = paths(&files);
            now.sort();
            let mut was = before.clone();
            was.sort();
            assert_eq!(now, was, "the selection survives (grid {grid}, columns {columns}, gallery {gallery})");
            assert_eq!(files.focus_entry().map(|e| e.name.clone()).as_deref(), Some("f055"), "and its focus is the same item");
            if !gallery {
                assert!(fully_in_view(&files, files.focus().unwrap()), "the focused item is in view (grid {grid}, columns {columns})");
            }
        }
    }

    #[test]
    fn a_view_switch_with_no_selection_keeps_the_first_item_that_was_in_view() {
        let mut files = items(200);
        files.set_view(false, false);
        files.scroll = 32. * 80. + 10.;
        // row 80 is cut by the header: row 81 is the first one wholly in view
        files.set_view(true, false);
        assert!(files.selection.is_empty());
        assert!(files.scroll > 0., "the icon view does not start over at the top");
        assert!(fully_in_view(&files, 81), "the first row that was in view is in view");
        let cols = files.grid_cols();
        assert_eq!(on_screen(&files, 81 / cols * cols).0, files.rows_top() + 16., "its line is the first line of the view");
        // a view that was not scrolled stays at the top
        files.scroll = 0.;
        files.set_view(false, false);
        assert_eq!(files.scroll, 0.);
    }

    #[test]
    fn a_view_switch_keeps_the_order_the_items_were_picked_in() {
        let mut files = items(10);
        files.set_view(false, false);
        files.selection = vec![2, 7];
        files.set_view(true, false);
        assert_eq!(files.selection, vec![2, 7]);
        assert_eq!(files.focus(), Some(7));
        // the focus is the item picked last, not the one furthest down
        files.selection = vec![7, 2];
        files.set_view(false, true);
        assert_eq!(files.selection, vec![7, 2]);
    }

    #[test]
    fn the_scroll_knob_shows_only_while_the_list_or_icon_view_scrolls() {
        let mut files = items(3);
        files.grid = false;
        assert!(files.scroll_knob(3).is_none(), "everything fits: no knob");
        let mut files = items(200);
        for grid in [false, true] {
            files.grid = grid;
            files.scroll = 0.;
            let top = if grid { files.body_top() } else { files.rows_top() };
            let (x, y, w, len) = files.scroll_knob(200).expect("more than fits");
            assert!(x + w < files.w && x > files.w - 12., "at the trailing edge");
            assert!(y >= top && len >= INDICATOR_MIN_PT, "under the header (grid {grid})");
            files.scroll = files.scroll_max();
            let (_, y, _, len) = files.scroll_knob(200).unwrap();
            assert!(y + len <= files.bottom() && y + len > files.bottom() - 8., "at the end it reaches the bottom of the view");
        }
        files.grid = false;
        files.columns = true;
        assert!(files.scroll_knob(200).is_none(), "the columns view has no knob");
    }
}
