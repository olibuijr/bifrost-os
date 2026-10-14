//! Where things sit in the window, in points. The sidebar holds the search, the bookmarks filter and the timeline;
//! the editor pane holds the date, the buttons, the text and the photo bar. No drawing and no Ui here, so the hit
//! tests can be tested without a display.

/// Width of the timeline sidebar.
pub const SIDEBAR_PT: f32 = 260.0;
pub const PAD_PT: f32 = 12.0;
pub const SEARCH_H_PT: f32 = 28.0;
pub const FILTER_TOP_PT: f32 = 46.0;
pub const FILTER_H_PT: f32 = 26.0;
/// The first row of the timeline.
pub const LIST_TOP_PT: f32 = 82.0;
pub const ROW_H_PT: f32 = 58.0;
pub const HEADER_H_PT: f32 = 52.0;
pub const BUTTON_H_PT: f32 = 26.0;
pub const BUTTON_W_PT: f32 = 96.0;
pub const BUTTON_GAP_PT: f32 = 8.0;
pub const PHOTO_BAR_H_PT: f32 = 92.0;
pub const FIELD_H_PT: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What a touch point is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    Search,
    Filter,
    /// A timeline row, by its index in the timeline (not the slot on screen).
    Row(usize),
    New,
    Bookmark,
    Delete,
    Editor,
    PhotoField,
    Attach,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
}

impl Layout {
    pub fn new(w: f32, h: f32) -> Layout {
        Layout { w: w.max(0.0), h: h.max(0.0) }
    }

    /// Width of the editor pane to the right of the sidebar.
    pub fn pane_w(&self) -> f32 {
        (self.w - SIDEBAR_PT).max(0.0)
    }

    pub fn search(&self) -> Rect {
        Rect { x: PAD_PT, y: PAD_PT, w: SIDEBAR_PT - 2.0 * PAD_PT, h: SEARCH_H_PT }
    }

    pub fn filter(&self) -> Rect {
        Rect { x: PAD_PT, y: FILTER_TOP_PT, w: SIDEBAR_PT - 2.0 * PAD_PT, h: FILTER_H_PT }
    }

    /// How many timeline rows fit under the sidebar header.
    pub fn visible_rows(&self) -> usize {
        ((self.h - LIST_TOP_PT).max(0.0) / ROW_H_PT).floor() as usize
    }

    pub fn row(&self, slot: usize) -> Rect {
        Rect { x: 0.0, y: LIST_TOP_PT + slot as f32 * ROW_H_PT, w: SIDEBAR_PT, h: ROW_H_PT }
    }

    /// The buttons of the editor header, in order: New Entry, Bookmark, Delete. They sit at the right edge.
    pub fn buttons(&self) -> [Rect; 3] {
        let y = (HEADER_H_PT - BUTTON_H_PT) / 2.0;
        let delete_x = self.w - PAD_PT - BUTTON_W_PT;
        let bookmark_x = delete_x - BUTTON_GAP_PT - BUTTON_W_PT;
        let new_x = bookmark_x - BUTTON_GAP_PT - BUTTON_W_PT;
        let at = |x: f32| Rect { x, y, w: BUTTON_W_PT, h: BUTTON_H_PT };
        [at(new_x), at(bookmark_x), at(delete_x)]
    }

    pub fn editor(&self) -> Rect {
        Rect {
            x: SIDEBAR_PT + PAD_PT,
            y: HEADER_H_PT + PAD_PT,
            w: (self.pane_w() - 2.0 * PAD_PT).max(0.0),
            h: (self.h - HEADER_H_PT - PHOTO_BAR_H_PT - 2.0 * PAD_PT).max(0.0),
        }
    }

    pub fn photo_bar(&self) -> Rect {
        Rect { x: SIDEBAR_PT, y: (self.h - PHOTO_BAR_H_PT).max(0.0), w: self.pane_w(), h: PHOTO_BAR_H_PT }
    }

    /// Width of the Attach button at the right of the photo bar.
    pub fn attach(&self) -> Rect {
        let bar = self.photo_bar();
        Rect { x: bar.x + bar.w - PAD_PT - BUTTON_W_PT, y: bar.y + 40.0, w: BUTTON_W_PT, h: FIELD_H_PT }
    }

    /// The field where the path of an image file is typed.
    pub fn photo_field(&self) -> Rect {
        let bar = self.photo_bar();
        let attach = self.attach();
        Rect {
            x: bar.x + PAD_PT,
            y: attach.y,
            w: (attach.x - BUTTON_GAP_PT - (bar.x + PAD_PT)).max(0.0),
            h: FIELD_H_PT,
        }
    }

    /// The touch target at (`x`, `y`) in points. `first_row` is the timeline index of the top row and `count` the
    /// number of timeline rows.
    pub fn hit(&self, x: f32, y: f32, first_row: usize, count: usize) -> Hit {
        if self.search().contains(x, y) {
            return Hit::Search;
        }
        if self.filter().contains(x, y) {
            return Hit::Filter;
        }
        if x < SIDEBAR_PT {
            if y >= LIST_TOP_PT {
                let slot = ((y - LIST_TOP_PT) / ROW_H_PT) as usize;
                let index = first_row + slot;
                if slot < self.visible_rows() && index < count {
                    return Hit::Row(index);
                }
            }
            return Hit::None;
        }
        let [new, bookmark, delete] = self.buttons();
        if new.contains(x, y) {
            return Hit::New;
        }
        if bookmark.contains(x, y) {
            return Hit::Bookmark;
        }
        if delete.contains(x, y) {
            return Hit::Delete;
        }
        if self.attach().contains(x, y) {
            return Hit::Attach;
        }
        if self.photo_field().contains(x, y) {
            return Hit::PhotoField;
        }
        if self.editor().contains(x, y) {
            return Hit::Editor;
        }
        Hit::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_rows_map_to_entries_below_the_header() {
        let l = Layout::new(900.0, 600.0);
        assert_eq!(l.hit(10.0, LIST_TOP_PT + 1.0, 0, 3), Hit::Row(0));
        assert_eq!(l.hit(10.0, LIST_TOP_PT + ROW_H_PT + 1.0, 0, 3), Hit::Row(1));
        assert_eq!(l.hit(10.0, LIST_TOP_PT + ROW_H_PT + 1.0, 5, 8), Hit::Row(6), "scrolled list");
        assert_eq!(l.hit(10.0, LIST_TOP_PT + 2.0 * ROW_H_PT + 1.0, 0, 2), Hit::None, "past the last entry");
        assert_eq!(l.hit(20.0, 20.0, 0, 3), Hit::Search);
    }

    #[test]
    fn filter_and_search_are_in_the_sidebar_header() {
        let l = Layout::new(900.0, 600.0);
        assert_eq!(l.hit(20.0, FILTER_TOP_PT + 2.0, 0, 0), Hit::Filter);
        assert_eq!(l.hit(20.0, PAD_PT + 2.0, 0, 0), Hit::Search);
    }

    #[test]
    fn editor_header_buttons_do_not_overlap() {
        let l = Layout::new(900.0, 600.0);
        let [new, bookmark, delete] = l.buttons();
        assert!(new.x + new.w < bookmark.x);
        assert!(bookmark.x + bookmark.w < delete.x);
        assert!(delete.x + delete.w <= l.w);
        assert_eq!(l.hit(new.x + 1.0, new.y + 1.0, 0, 0), Hit::New);
        assert_eq!(l.hit(bookmark.x + 1.0, bookmark.y + 1.0, 0, 0), Hit::Bookmark);
        assert_eq!(l.hit(delete.x + 1.0, delete.y + 1.0, 0, 0), Hit::Delete);
    }

    #[test]
    fn photo_bar_holds_the_field_and_the_attach_button() {
        let l = Layout::new(900.0, 600.0);
        let field = l.photo_field();
        let attach = l.attach();
        assert!(field.x + field.w < attach.x);
        assert_eq!(l.hit(field.x + 1.0, field.y + 1.0, 0, 0), Hit::PhotoField);
        assert_eq!(l.hit(attach.x + 1.0, attach.y + 1.0, 0, 0), Hit::Attach);
        let editor = l.editor();
        assert_eq!(l.hit(editor.x + 1.0, editor.y + 1.0, 0, 0), Hit::Editor);
        assert!(editor.y + editor.h <= l.photo_bar().y);
    }

    #[test]
    fn small_windows_have_no_negative_sizes() {
        let l = Layout::new(100.0, 40.0);
        assert_eq!(l.visible_rows(), 0);
        assert_eq!(l.editor().h, 0.0);
        assert_eq!(l.pane_w(), 0.0);
    }
}
