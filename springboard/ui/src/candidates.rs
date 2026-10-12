//! The candidates window (`NativeCandidates`): the alternate text an input method offers for a composition, the
//! selection and paging rules for that list, the panel style (`NativeCandidatePanelType`), the side of the caret the
//! window prefers (`NativeCandidatesLocationHint`), and its opacity. The model draws nothing; the shell paints it.
//! Positions are logical points, origin top-left, y grows downward.

/// How the candidates are laid out (`NativeCandidatePanelType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelType {
    /// One column that scrolls, the usual list.
    SingleColumnScrolling,
    /// A grid of rows and columns that scrolls.
    ScrollingGrid,
    /// One row that steps left and right.
    SingleRowStepping,
}

/// The side of the caret the window prefers (`NativeCandidatesLocationHint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationHint {
    Above,
    Below,
    Left,
    Right,
}

/// Opacity of the window, clamped to 0.0 to 1.0. A NaN value means fully opaque.
pub fn clamp_opacity(v: f32) -> f32 {
    if v.is_nan() {
        1.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Keeps a window of length `len` inside a screen of length `screen` on one axis, starting at `v`.
fn fit(v: f32, len: f32, screen: f32) -> f32 {
    v.min(screen - len).max(0.0)
}

/// Top-left of a `w` x `h` window for the caret rectangle `caret` (x, y, w, h) on the `hint` side, `gap` px away.
/// Above and below centre on the caret's x; left and right centre on its y. A side that would leave the screen flips
/// to the opposite side. The result is kept inside the `screen_w` x `screen_h` screen.
pub fn place(
    hint: LocationHint,
    caret: (f32, f32, f32, f32),
    w: f32,
    h: f32,
    screen_w: f32,
    screen_h: f32,
    gap: f32,
) -> (f32, f32) {
    let (cx, cy, cw, ch) = caret;
    let (x, y) = match hint {
        LocationHint::Below => {
            let y = cy + ch + gap;
            let y = if y + h > screen_h { cy - gap - h } else { y };
            (cx, y)
        }
        LocationHint::Above => {
            let y = cy - gap - h;
            let y = if y < 0.0 { cy + ch + gap } else { y };
            (cx, y)
        }
        LocationHint::Right => {
            let x = cx + cw + gap;
            let x = if x + w > screen_w { cx - gap - w } else { x };
            (x, cy + ch / 2.0 - h / 2.0)
        }
        LocationHint::Left => {
            let x = cx - gap - w;
            let x = if x < 0.0 { cx + cw + gap } else { x };
            (x, cy + ch / 2.0 - h / 2.0)
        }
    };
    (fit(x, w, screen_w), fit(y, h, screen_h))
}

/// The candidates window: the list, the selection, the panel style and the visibility state.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidates {
    items: Vec<String>,
    selected: Option<usize>,
    panel: PanelType,
    /// Columns of a scrolling grid, at least 1.
    columns: usize,
    visible: bool,
    hint: LocationHint,
    dismisses: bool,
    opacity: f32,
}

impl Candidates {
    pub fn new(panel: PanelType) -> Self {
        Candidates {
            items: Vec::new(),
            selected: None,
            panel,
            columns: 1,
            visible: false,
            hint: LocationHint::Below,
            dismisses: true,
            opacity: 1.0,
        }
    }

    /// Sets the grid's column count; zero is treated as one.
    pub fn with_columns(mut self, columns: usize) -> Self {
        self.columns = columns.max(1);
        self
    }

    pub fn panel_type(&self) -> PanelType {
        self.panel
    }

    pub fn set_panel_type(&mut self, panel: PanelType) {
        self.panel = panel;
    }

    /// Replaces the candidates (`setCandidateData` then `update`). A new list always clears the selection.
    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.selected = None;
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// Selects candidate `index`. Returns false and leaves the selection alone when the index is out of range.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.items.len() {
            self.selected = Some(index);
            true
        } else {
            false
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The text of the selected candidate (`selectedCandidateString`).
    pub fn selected_string(&self) -> Option<&str> {
        self.selected.and_then(|i| self.items.get(i)).map(|s| s.as_str())
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Moves the selection by `delta` candidates. With nothing selected, a positive step selects the first candidate
    /// and a negative step the last. The move stops at both ends; it does not wrap.
    pub fn move_by(&mut self, delta: isize) {
        let len = self.items.len();
        if len == 0 || delta == 0 {
            return;
        }
        self.selected = Some(match self.selected {
            None if delta > 0 => 0,
            None => len - 1,
            Some(cur) => (cur as isize + delta).clamp(0, len as isize - 1) as usize,
        });
    }

    /// Moves the selection one row down or up. In a grid a row is `columns` candidates; in the other styles it is one.
    pub fn move_row(&mut self, down: bool) {
        let step: isize = if self.panel == PanelType::ScrollingGrid { self.columns as isize } else { 1 };
        self.move_by(if down { step } else { -step });
    }

    /// The candidates on a page of `rows` items that starts at `first`, clamped to the list.
    pub fn page_range(&self, first: usize, rows: usize) -> std::ops::Range<usize> {
        let len = self.items.len();
        let start = first.min(len);
        let end = first.saturating_add(rows).min(len);
        start..end.max(start)
    }

    /// The page that holds candidate `index` when pages have `rows` items. Zero rows counts as one.
    pub fn page_of(&self, index: usize, rows: usize) -> usize {
        index / rows.max(1)
    }

    /// The first visible index that keeps the selection on screen when `rows` items fit, scrolling the least needed
    /// from `first`. With no selection, `first` is returned unchanged.
    pub fn ensure_visible(&self, first: usize, rows: usize) -> usize {
        let rows = rows.max(1);
        match self.selected {
            Some(s) if s < first => s,
            Some(s) if s >= first + rows => s + 1 - rows,
            _ => first,
        }
    }

    /// Shows the window on the side `hint` (`show(_:)` with a location hint).
    pub fn show(&mut self, hint: LocationHint) {
        self.hint = hint;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn location_hint(&self) -> LocationHint {
        self.hint
    }

    /// Where the window goes for the caret rectangle and the screen (see [`place`]), using the last shown hint.
    pub fn position(&self, caret: (f32, f32, f32, f32), w: f32, h: f32, screen_w: f32, screen_h: f32, gap: f32) -> (f32, f32) {
        place(self.hint, caret, w, h, screen_w, screen_h, gap)
    }

    pub fn dismisses_automatically(&self) -> bool {
        self.dismisses
    }

    pub fn set_dismisses_automatically(&mut self, on: bool) {
        self.dismisses = on;
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn set_opacity(&mut self, v: f32) {
        self.opacity = clamp_opacity(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("w{i}")).collect()
    }

    #[test]
    fn a_new_list_clears_the_selection_and_select_checks_the_range() {
        let mut c = Candidates::new(PanelType::SingleColumnScrolling);
        c.set_items(words(3));
        assert!(c.select(2));
        assert_eq!(c.selected_string(), Some("w2"));
        assert!(!c.select(3));
        assert_eq!(c.selected(), Some(2));
        c.set_items(words(5));
        assert_eq!(c.selected(), None);
    }

    #[test]
    fn stepping_stops_at_both_ends_and_starts_from_the_right_end() {
        let mut c = Candidates::new(PanelType::SingleColumnScrolling);
        c.set_items(words(3));
        c.move_by(1);
        assert_eq!(c.selected(), Some(0));
        c.move_by(-1);
        assert_eq!(c.selected(), Some(0));
        c.move_by(-5);
        assert_eq!(c.selected(), Some(0));
        c.move_by(7);
        assert_eq!(c.selected(), Some(2));
        c.clear_selection();
        c.move_by(-1);
        assert_eq!(c.selected(), Some(2));
    }

    #[test]
    fn grid_rows_move_by_the_column_count() {
        let mut c = Candidates::new(PanelType::ScrollingGrid).with_columns(3);
        c.set_items(words(7));
        c.select(1);
        c.move_row(true);
        assert_eq!(c.selected(), Some(4));
        c.move_row(true);
        assert_eq!(c.selected(), Some(6));
        c.move_row(false);
        assert_eq!(c.selected(), Some(3));
        let mut z = Candidates::new(PanelType::ScrollingGrid).with_columns(0);
        assert_eq!(z.panel_type(), PanelType::ScrollingGrid);
        z.set_items(words(2));
        z.select(0);
        z.move_row(true);
        assert_eq!(z.selected(), Some(1));
    }

    #[test]
    fn pages_clamp_to_the_list_and_scrolling_keeps_the_selection_visible() {
        let mut c = Candidates::new(PanelType::SingleColumnScrolling);
        c.set_items(words(7));
        assert_eq!(c.page_range(2, 3), 2..5);
        assert_eq!(c.page_range(6, 3), 6..7);
        assert_eq!(c.page_range(9, 3), 7..7);
        assert_eq!(c.page_of(5, 3), 1);
        assert_eq!(c.page_of(5, 0), 5);
        c.select(4);
        assert_eq!(c.ensure_visible(0, 3), 2);
        c.select(1);
        assert_eq!(c.ensure_visible(3, 3), 1);
        c.clear_selection();
        assert_eq!(c.ensure_visible(3, 3), 3);
    }

    #[test]
    fn opacity_is_clamped_and_nan_is_opaque() {
        assert_eq!(clamp_opacity(1.5), 1.0);
        assert_eq!(clamp_opacity(-0.2), 0.0);
        assert_eq!(clamp_opacity(f32::NAN), 1.0);
        assert_eq!(clamp_opacity(0.5), 0.5);
        let mut c = Candidates::new(PanelType::SingleRowStepping);
        assert_eq!(c.opacity(), 1.0);
        c.set_opacity(0.25);
        assert_eq!(c.opacity(), 0.25);
    }

    #[test]
    fn the_window_sits_on_the_preferred_side_and_flips_when_it_does_not_fit() {
        let caret = (100.0, 200.0, 1.0, 20.0);
        // screen 800 x 600, window 150 x 80, gap 4
        assert_eq!(place(LocationHint::Below, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 224.0));
        assert_eq!(place(LocationHint::Above, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 116.0));
        // no room below the caret at the bottom of the screen: flip above
        assert_eq!(place(LocationHint::Below, (100.0, 560.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 476.0));
        // no room above at the top: flip below
        assert_eq!(place(LocationHint::Above, (100.0, 20.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 44.0));
        // right of the caret, centred on its height (200 + 10 - 40 = 170)
        assert_eq!(place(LocationHint::Right, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (105.0, 170.0));
        // left of the caret would start at -54: flips to the right side
        assert_eq!(place(LocationHint::Left, caret, 150.0, 80.0, 800.0, 600.0, 4.0), (105.0, 170.0));
        assert_eq!(place(LocationHint::Left, (400.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (246.0, 170.0));
        // right of a caret at the right edge: flips to the left
        assert_eq!(place(LocationHint::Right, (780.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (626.0, 170.0));
        // a caret near the right edge keeps the window on screen
        assert_eq!(place(LocationHint::Below, (760.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (650.0, 224.0));
    }

    #[test]
    fn show_and_hide_remember_the_hint_and_the_dismiss_flag() {
        let mut c = Candidates::new(PanelType::SingleColumnScrolling);
        assert!(!c.is_visible());
        assert!(c.dismisses_automatically());
        c.show(LocationHint::Above);
        assert!(c.is_visible());
        assert_eq!(c.location_hint(), LocationHint::Above);
        assert_eq!(c.position((100.0, 200.0, 1.0, 20.0), 150.0, 80.0, 800.0, 600.0, 4.0), (100.0, 116.0));
        c.hide();
        assert!(!c.is_visible());
        c.set_dismisses_automatically(false);
        assert!(!c.dismisses_automatically());
    }
}
