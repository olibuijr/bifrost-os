//! The list a Today widget shows its content in (the Notification Center framework's list controller). Rows of
//! content with a minimum visible row count, optional divider lines, and an editing mode with an Add button.
//! Pure model: which content sits in which row, how many rows a height shows, and what the editing state allows.

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetList<T> {
    contents: Vec<T>,
    /// Height of one row, in points.
    pub row_h: f32,
    /// Fewest rows the list shows, even when it has less room or less content (`minimumVisibleRowCount`).
    pub minimum_visible_rows: usize,
    pub has_divider_lines: bool,
    pub editing: bool,
    pub shows_add_button_when_editing: bool,
}

impl<T: PartialEq> WidgetList<T> {
    pub fn new(row_h: f32) -> Self {
        WidgetList {
            contents: Vec::new(),
            row_h,
            minimum_visible_rows: 0,
            has_divider_lines: false,
            editing: false,
            shows_add_button_when_editing: false,
        }
    }

    /// The content in row order (`contents`).
    pub fn contents(&self) -> &[T] {
        &self.contents
    }

    pub fn set_contents(&mut self, items: Vec<T>) {
        self.contents = items;
    }

    /// The row that holds `item`, if it is in the list (`row(for:)`).
    pub fn row_for(&self, item: &T) -> Option<usize> {
        self.contents.iter().position(|c| c == item)
    }

    /// The content shown at `row` (`viewController(atRow:)`).
    pub fn item_at(&self, row: usize) -> Option<&T> {
        self.contents.get(row)
    }

    /// Rows drawn in a list `height` points tall: the rows that fit, never fewer than the minimum and never more
    /// than there is content for. A row height of zero or less lets every row fit.
    pub fn visible_rows(&self, height: f32) -> usize {
        let fit = if self.row_h > 0.0 && height > 0.0 {
            (height / self.row_h).floor() as usize
        } else if self.row_h > 0.0 {
            0
        } else {
            self.contents.len()
        };
        fit.max(self.minimum_visible_rows).min(self.contents.len())
    }

    /// The Add (+) button shows only while the list is editing and the list asks for it.
    pub fn add_button_visible(&self) -> bool {
        self.editing && self.shows_add_button_when_editing
    }

    /// Removes the content in `row` while editing and returns it; outside editing the list is unchanged and `None`
    /// comes back.
    pub fn delete(&mut self, row: usize) -> Option<T> {
        if !self.editing || row >= self.contents.len() {
            return None;
        }
        Some(self.contents.remove(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> WidgetList<String> {
        let mut l = WidgetList::new(44.0);
        l.set_contents((0..n).map(|i| format!("row {i}")).collect());
        l
    }

    #[test]
    fn a_row_is_found_by_its_content_and_content_is_read_by_row() {
        let l = list(3);
        assert_eq!(l.row_for(&"row 1".to_string()), Some(1));
        assert_eq!(l.row_for(&"missing".to_string()), None);
        assert_eq!(l.item_at(2).map(String::as_str), Some("row 2"));
        assert_eq!(l.item_at(3), None);
    }

    #[test]
    fn the_rows_that_fit_are_drawn_and_no_more_than_the_content() {
        let l = list(10);
        assert_eq!(l.visible_rows(132.0), 3, "132 / 44 = 3 rows fit");
        assert_eq!(l.visible_rows(1000.0), 10, "capped at the content count");
        assert_eq!(list(2).visible_rows(1000.0), 2);
    }

    #[test]
    fn the_minimum_row_count_wins_over_a_short_height_but_not_over_the_content() {
        let mut l = list(10);
        l.minimum_visible_rows = 4;
        assert_eq!(l.visible_rows(44.0), 4);
        let mut short = list(2);
        short.minimum_visible_rows = 4;
        assert_eq!(short.visible_rows(44.0), 2);
    }

    #[test]
    fn the_add_button_needs_both_editing_and_the_list_asking_for_it() {
        let mut l = list(1);
        assert!(!l.add_button_visible());
        l.editing = true;
        assert!(!l.add_button_visible());
        l.shows_add_button_when_editing = true;
        assert!(l.add_button_visible());
    }

    #[test]
    fn content_can_only_be_deleted_while_editing() {
        let mut l = list(3);
        assert_eq!(l.delete(0), None, "not editing: nothing is removed");
        assert_eq!(l.contents().len(), 3);
        l.editing = true;
        assert_eq!(l.delete(1), Some("row 1".to_string()));
        assert_eq!(l.contents(), &["row 0".to_string(), "row 2".to_string()]);
        assert_eq!(l.delete(5), None, "row past the end");
    }
}
