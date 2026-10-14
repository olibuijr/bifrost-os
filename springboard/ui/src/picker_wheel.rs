//! Spinning-wheel picker model (the reference platform's picker view): the components of a picker, the rows of each, the selected row of each
//! and the position of each wheel while it is dragged or settles on a row. The data source supplies counts and titles
//! (the reference platform's data source protocol) and optional component widths (the delegate's width query). Positions are counted
//! in rows, so the model does not depend on the row height in points. Nothing here draws; the SDK picker view maps
//! touches and keys onto it.

/// Seconds a settle animation takes from the release point to the row the wheel stops on.
pub const SETTLE_S: f64 = 0.35;
/// Seconds of motion a release velocity is projected over when the row is chosen, so a flick carries further.
pub const FLING_PROJECTION_S: f32 = 0.25;
/// Share of the finger's travel past the first or last row that the wheel follows (the rubber band).
const RUBBER_BAND: f32 = 0.4;

/// What a picker asks its data source for (the data source, and the width and title parts of the delegate).
pub trait PickerDataSource {
    /// `numberOfComponents(in:)`
    fn components(&self) -> usize;
    /// `pickerView(_:numberOfRowsInComponent:)`
    fn rows(&self, component: usize) -> usize;
    /// `pickerView(_:titleForRow:forComponent:)`
    fn title(&self, component: usize, row: usize) -> String;
    /// `pickerView(_:widthForComponent:)` in points; 0 lets the components share the width evenly.
    fn width(&self, _component: usize) -> f32 {
        0.0
    }
}

/// A wheel that settles on a row: the animation runs from `from` to `to` between `start` and `start + SETTLE_S`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settle {
    from: f32,
    to: f32,
    start: f64,
}

/// One spinning column: its row count, the selected row and the wheel's position in rows (the row under the centre
/// line, continuous while the wheel moves).
#[derive(Clone, Debug, PartialEq)]
pub struct WheelColumn {
    rows: usize,
    selected: usize,
    position: f32,
    /// the finger's position in rows before the rubber band
    finger: f32,
    settle: Option<Settle>,
}

fn clamp_row(row: usize, rows: usize) -> usize {
    row.min(rows.saturating_sub(1))
}

/// Maps a position past the first or last row (`last` is the last row index) through the rubber band.
fn rubber(p: f32, last: f32) -> f32 {
    if p < 0.0 {
        p * RUBBER_BAND
    } else if p > last {
        last + (p - last) * RUBBER_BAND
    } else {
        p
    }
}

impl WheelColumn {
    /// A column of `rows` rows with `selected` chosen (clamped into the rows).
    pub fn new(rows: usize, selected: usize) -> Self {
        let selected = clamp_row(selected, rows);
        WheelColumn { rows, selected, position: selected as f32, finger: selected as f32, settle: None }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The selected row; `None` when the column has no rows.
    pub fn selected(&self) -> Option<usize> {
        (self.rows > 0).then_some(self.selected)
    }

    /// The row under the centre line; may lie past the ends while the rubber band holds it.
    pub fn position(&self) -> f32 {
        self.position
    }

    pub fn is_settling(&self) -> bool {
        self.settle.is_some()
    }

    fn last(&self) -> f32 {
        self.rows.saturating_sub(1) as f32
    }

    /// `selectRow(_:inComponent:animated:)` without animation: selects `row` and moves the wheel there. Returns true
    /// when the selection changed; a row outside the column is ignored.
    pub fn select(&mut self, row: usize) -> bool {
        if row >= self.rows {
            return false;
        }
        self.settle = None;
        self.finger = row as f32;
        self.position = row as f32;
        let changed = row != self.selected;
        self.selected = row;
        changed
    }

    /// `reloadComponent(_:)`: a new row count. The selection stays when it still exists and is clamped otherwise.
    pub fn reload(&mut self, rows: usize) {
        self.rows = rows;
        self.selected = clamp_row(self.selected, rows);
        self.settle = None;
        self.finger = self.selected as f32;
        self.position = self.selected as f32;
    }

    /// Stops a settle animation where the wheel is, so a finger can take it over.
    pub fn grab(&mut self) {
        self.settle = None;
        self.finger = self.position;
    }

    /// The finger moved `delta` rows (positive towards later rows: the content follows the finger).
    pub fn drag(&mut self, delta: f32) {
        if self.is_settling() {
            self.grab();
        }
        self.finger += delta;
        self.position = rubber(self.finger, self.last());
    }

    /// The finger lifted with `velocity` rows per second: the wheel settles on the row the flick carries it to. Returns
    /// the new selection when it changed.
    pub fn release(&mut self, velocity: f32, now: f64) -> Option<usize> {
        if self.rows == 0 {
            return None;
        }
        let projected = self.position + velocity * FLING_PROJECTION_S;
        let target = projected.round().clamp(0.0, self.last()) as usize;
        self.settle_to(target, now)
    }

    /// Settles the wheel on `row` (clamped into the column) from where it is. Returns the new selection when it changed.
    pub fn settle_to(&mut self, row: usize, now: f64) -> Option<usize> {
        if self.rows == 0 {
            return None;
        }
        let target = clamp_row(row, self.rows);
        self.settle = Some(Settle { from: self.position, to: target as f32, start: now });
        self.finger = target as f32;
        if target == self.selected {
            return None;
        }
        self.selected = target;
        Some(target)
    }

    /// Advances a settle animation to `now` (seconds). Returns true while the wheel is still moving.
    pub fn tick(&mut self, now: f64) -> bool {
        let Some(s) = self.settle else {
            return false;
        };
        let t = ((now - s.start) / SETTLE_S).clamp(0.0, 1.0) as f32;
        let eased = 1.0 - (1.0 - t).powi(3);
        self.position = s.from + (s.to - s.from) * eased;
        if t >= 1.0 {
            self.position = s.to;
            self.finger = s.to;
            self.settle = None;
            return false;
        }
        true
    }

    /// Moves the selection by `delta` rows, clamped to the column (a key step). Returns true when it changed.
    pub fn step(&mut self, delta: i32) -> bool {
        if self.rows == 0 {
            return false;
        }
        let next = (self.selected as i64 + delta as i64).clamp(0, self.rows as i64 - 1) as usize;
        self.select(next)
    }

    /// The rows within `radius` of the centre line that exist, for drawing.
    pub fn visible_rows(&self, radius: usize) -> std::ops::Range<usize> {
        let centre = self.position.round().max(0.0) as usize;
        let lo = centre.saturating_sub(radius).min(self.rows);
        let hi = (centre + radius + 1).min(self.rows);
        lo..hi
    }
}

/// A picker: one wheel per component of its data source.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PickerWheel {
    columns: Vec<WheelColumn>,
    /// per component: the fixed width in points, 0 for an even share
    widths: Vec<f32>,
}

impl PickerWheel {
    /// A picker with the data source's components, the first row of each selected.
    pub fn new(source: &dyn PickerDataSource) -> Self {
        let mut wheel = PickerWheel::default();
        wheel.reload_all(source);
        wheel
    }

    /// `reloadAllComponents()`: counts and widths come from the source; existing selections are kept where they fit.
    pub fn reload_all(&mut self, source: &dyn PickerDataSource) {
        let n = source.components();
        self.columns.truncate(n);
        for c in 0..n {
            let rows = source.rows(c);
            match self.columns.get_mut(c) {
                Some(col) => col.reload(rows),
                None => self.columns.push(WheelColumn::new(rows, 0)),
            }
        }
        self.widths = (0..n).map(|c| source.width(c).max(0.0)).collect();
    }

    /// `reloadComponent(_:)` for one component; ignored when the picker has no such component.
    pub fn reload_component(&mut self, source: &dyn PickerDataSource, component: usize) {
        if let Some(col) = self.columns.get_mut(component) {
            col.reload(source.rows(component));
            self.widths[component] = source.width(component).max(0.0);
        }
    }

    /// `numberOfComponents`
    pub fn number_of_components(&self) -> usize {
        self.columns.len()
    }

    /// `numberOfRows(inComponent:)`; 0 for a component the picker does not have.
    pub fn number_of_rows(&self, component: usize) -> usize {
        self.columns.get(component).map_or(0, WheelColumn::rows)
    }

    pub fn columns(&self) -> &[WheelColumn] {
        &self.columns
    }

    pub fn column(&self, component: usize) -> Option<&WheelColumn> {
        self.columns.get(component)
    }

    pub fn column_mut(&mut self, component: usize) -> Option<&mut WheelColumn> {
        self.columns.get_mut(component)
    }

    /// `selectedRow(inComponent:)`
    pub fn selected_row(&self, component: usize) -> Option<usize> {
        self.columns.get(component).and_then(WheelColumn::selected)
    }

    /// `selectRow(_:inComponent:animated: false)`; true when the selection changed.
    pub fn select_row(&mut self, component: usize, row: usize) -> bool {
        self.columns.get_mut(component).is_some_and(|col| col.select(row))
    }

    /// Advances every settling wheel to `now`; true while any is still moving.
    pub fn tick(&mut self, now: f64) -> bool {
        self.columns.iter_mut().fold(false, |moving, col| col.tick(now) | moving)
    }

    /// The width of each component inside `total` points: fixed widths as given, the rest shares what is left evenly.
    pub fn column_widths(&self, total: f32) -> Vec<f32> {
        let fixed: f32 = self.widths.iter().filter(|w| **w > 0.0).sum();
        let free_count = self.widths.iter().filter(|w| **w <= 0.0).count();
        let share = if free_count == 0 { 0.0 } else { (total - fixed).max(0.0) / free_count as f32 };
        self.widths.iter().map(|&w| if w > 0.0 { w } else { share }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Source {
        cols: Vec<Vec<&'static str>>,
        widths: Vec<f32>,
    }

    impl PickerDataSource for Source {
        fn components(&self) -> usize {
            self.cols.len()
        }
        fn rows(&self, component: usize) -> usize {
            self.cols.get(component).map_or(0, Vec::len)
        }
        fn title(&self, component: usize, row: usize) -> String {
            self.cols[component][row].to_string()
        }
        fn width(&self, component: usize) -> f32 {
            self.widths.get(component).copied().unwrap_or(0.0)
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn new_column_clamps_the_selection_and_an_empty_column_has_none() {
        assert_eq!(WheelColumn::new(3, 9).selected(), Some(2));
        assert_eq!(WheelColumn::new(0, 0).selected(), None);
        assert_eq!(WheelColumn::new(4, 1).position(), 1.0);
    }

    #[test]
    fn select_reports_only_real_changes_and_ignores_rows_outside_the_column() {
        let mut col = WheelColumn::new(5, 1);
        assert!(!col.select(1));
        assert!(col.select(3));
        assert!(!col.select(7));
        assert_eq!(col.selected(), Some(3));
    }

    #[test]
    fn reload_keeps_the_selection_inside_the_new_rows() {
        let mut col = WheelColumn::new(10, 8);
        col.reload(5);
        assert_eq!(col.selected(), Some(4));
        col.reload(0);
        assert_eq!(col.selected(), None);
    }

    #[test]
    fn drag_follows_the_finger_and_rubber_bands_past_the_ends() {
        let mut col = WheelColumn::new(10, 0);
        col.drag(3.0);
        assert!(near(col.position(), 3.0));
        col.drag(-10.0);
        assert!(near(col.position(), -7.0 * RUBBER_BAND));
    }

    #[test]
    fn release_snaps_to_the_nearest_row_and_reports_the_change() {
        let mut col = WheelColumn::new(10, 0);
        col.drag(2.4);
        assert_eq!(col.release(0.0, 0.0), Some(2));
        assert_eq!(col.selected(), Some(2));
        assert!(col.is_settling());
    }

    #[test]
    fn a_flick_carries_the_wheel_further_than_a_slow_drag() {
        let mut col = WheelColumn::new(20, 0);
        col.drag(2.0);
        // 2 rows plus 20 rows per second over the projection window: 2 + 5
        assert_eq!(col.release(20.0, 0.0), Some(7));
    }

    #[test]
    fn release_past_the_first_row_settles_on_it_without_a_change() {
        let mut col = WheelColumn::new(4, 0);
        col.drag(-1.0);
        assert_eq!(col.release(0.0, 0.0), None);
        assert_eq!(col.selected(), Some(0));
        col.tick(1.0);
        assert!(near(col.position(), 0.0));
    }

    #[test]
    fn settle_eases_to_the_target_and_stops_exactly_on_it() {
        let mut col = WheelColumn::new(10, 0);
        col.drag(3.6);
        assert_eq!(col.release(0.0, 1.0), Some(4));
        assert!(col.tick(1.0 + SETTLE_S / 2.0));
        assert!(col.position() > 3.6 && col.position() < 4.0);
        assert!(!col.tick(1.0 + SETTLE_S + 0.1));
        assert_eq!(col.position(), 4.0);
        assert!(!col.is_settling());
    }

    #[test]
    fn grab_stops_a_settle_where_the_wheel_is() {
        let mut col = WheelColumn::new(10, 0);
        col.settle_to(6, 0.0);
        col.tick(SETTLE_S / 2.0);
        let mid = col.position();
        col.grab();
        assert!(!col.is_settling());
        assert!(near(col.position(), mid));
    }

    #[test]
    fn step_moves_the_selection_and_clamps_at_the_ends() {
        let mut col = WheelColumn::new(3, 0);
        assert!(col.step(1));
        assert_eq!(col.selected(), Some(1));
        assert!(col.step(5));
        assert_eq!(col.selected(), Some(2));
        assert!(!col.step(1));
        assert!(col.step(-9));
        assert_eq!(col.selected(), Some(0));
    }

    #[test]
    fn visible_rows_follow_the_position_and_stay_inside_the_column() {
        assert_eq!(WheelColumn::new(20, 10).visible_rows(2), 8..13);
        assert_eq!(WheelColumn::new(20, 0).visible_rows(2), 0..3);
        assert_eq!(WheelColumn::new(2, 0).visible_rows(5), 0..2);
        assert_eq!(WheelColumn::new(0, 0).visible_rows(2), 0..0);
    }

    #[test]
    fn picker_takes_components_rows_and_widths_from_its_source() {
        let source = Source { cols: vec![vec!["a", "b", "c"], vec!["w", "x", "y", "z"]], widths: vec![0.0, 100.0] };
        let wheel = PickerWheel::new(&source);
        assert_eq!(wheel.number_of_components(), 2);
        assert_eq!(wheel.number_of_rows(1), 4);
        assert_eq!(wheel.number_of_rows(9), 0);
        assert_eq!(wheel.selected_row(0), Some(0));
        assert_eq!(wheel.column_widths(300.0), vec![200.0, 100.0]);
    }

    #[test]
    fn reload_all_keeps_selections_and_drops_components_that_went_away() {
        let source = Source { cols: vec![vec!["a", "b", "c"], vec!["x", "y"]], widths: vec![] };
        let mut wheel = PickerWheel::new(&source);
        assert!(wheel.select_row(0, 2));
        let fewer = Source { cols: vec![vec!["a", "b"]], widths: vec![] };
        wheel.reload_all(&fewer);
        assert_eq!(wheel.number_of_components(), 1);
        assert_eq!(wheel.selected_row(0), Some(1));
        assert!(!wheel.select_row(3, 0));
    }

    #[test]
    fn picker_tick_reports_whether_any_wheel_still_moves() {
        let source = Source { cols: vec![vec!["a", "b", "c"], vec!["x", "y"]], widths: vec![] };
        let mut wheel = PickerWheel::new(&source);
        wheel.column_mut(1).unwrap().settle_to(1, 0.0);
        assert!(wheel.tick(0.0));
        assert!(!wheel.tick(SETTLE_S + 0.1));
        assert_eq!(wheel.column(1).unwrap().position(), 1.0);
    }
}
