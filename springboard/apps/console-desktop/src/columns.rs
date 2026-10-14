//! The table's columns and which of them the View > Visible Columns submenu shows. Pure layout: no drawing and no kit.

/// The table's columns, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Type,
    Time,
    Process,
    Pid,
    Subsystem,
    Category,
    Message,
}

/// Below this table width (points) Subsystem and Category give way.
const NARROW_PT: f32 = 620.0;

impl Col {
    /// Every column in table order, which is also the order of the Visible Columns submenu.
    pub const ALL: [Col; 7] = [Col::Type, Col::Time, Col::Process, Col::Pid, Col::Subsystem, Col::Category, Col::Message];

    /// The header text and the menu item.
    pub fn title(self) -> &'static str {
        match self {
            Col::Type => "Type",
            Col::Time => "Time",
            Col::Process => "Process",
            Col::Pid => "PID",
            Col::Subsystem => "Subsystem",
            Col::Category => "Category",
            Col::Message => "Message",
        }
    }

    /// The column's width in points. Message has none of its own: it takes what the others leave.
    pub fn width(self) -> f32 {
        match self {
            Col::Type => 84.0,
            Col::Time => 96.0,
            Col::Process => 128.0,
            Col::Pid => 56.0,
            Col::Subsystem => 150.0,
            Col::Category => 96.0,
            Col::Message => 0.0,
        }
    }

    fn bit(self) -> u8 {
        1 << Col::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// Whether the column is dropped on a narrow table.
    fn gives_way(self) -> bool {
        matches!(self, Col::Subsystem | Col::Category)
    }
}

/// The columns the user has switched off. PID starts off, so the table opens as it always has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnSet {
    hidden: u8,
}

impl Default for ColumnSet {
    fn default() -> Self {
        ColumnSet { hidden: Col::Pid.bit() }
    }
}

impl ColumnSet {
    pub fn shows(self, col: Col) -> bool {
        self.hidden & col.bit() == 0
    }

    /// Shows or hides a column. The last visible column stays: a table without columns is nothing. True when it changed.
    pub fn toggle(&mut self, col: Col) -> bool {
        if self.shows(col) && Col::ALL.iter().filter(|c| self.shows(**c)).count() == 1 {
            return false;
        }
        self.hidden ^= col.bit();
        true
    }
}

/// The columns across the table, from `x0` over `table_w` points: each shown column at its width, Subsystem and
/// Category left out on a narrow table, and the last column taking the rest. A narrow table whose only shown
/// columns would give way shows Message instead.
pub fn layout(x0: f32, table_w: f32, set: ColumnSet) -> Vec<(Col, f32, f32)> {
    let narrow = table_w < NARROW_PT;
    let mut cols: Vec<Col> = Col::ALL.iter().copied().filter(|c| set.shows(*c) && !(narrow && c.gives_way())).collect();
    if cols.is_empty() {
        cols.push(Col::Message);
    }
    let end = x0 + table_w;
    let mut x = x0;
    let mut out = Vec::with_capacity(cols.len());
    for (i, col) in cols.iter().enumerate() {
        let w = if i + 1 == cols.len() { (end - x).max(0.0) } else { col.width() };
        out.push((*col, x, w));
        x += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(l: &[(Col, f32, f32)]) -> Vec<Col> {
        l.iter().map(|c| c.0).collect()
    }

    #[test]
    fn the_default_set_is_the_table_as_it_always_was() {
        let wide = layout(190.0, 900.0, ColumnSet::default());
        assert_eq!(kinds(&wide), vec![Col::Type, Col::Time, Col::Process, Col::Subsystem, Col::Category, Col::Message]);
    }

    #[test]
    fn narrow_tables_drop_subsystem_and_category() {
        let narrow = layout(190.0, 500.0, ColumnSet::default());
        assert_eq!(kinds(&narrow), vec![Col::Type, Col::Time, Col::Process, Col::Message]);
        let last = narrow.last().unwrap();
        assert!((last.1 - (190.0 + 84.0 + 96.0 + 128.0)).abs() < 0.01 && last.2 > 0.0);
    }

    #[test]
    fn a_shown_pid_column_sits_after_process() {
        let mut set = ColumnSet::default();
        assert!(set.toggle(Col::Pid));
        let wide = layout(0.0, 900.0, set);
        assert_eq!(kinds(&wide)[2..4], [Col::Process, Col::Pid]);
        assert_eq!(wide[3].1, 84.0 + 96.0 + 128.0);
    }

    #[test]
    fn a_hidden_column_leaves_no_gap_and_the_last_column_takes_the_rest() {
        let mut set = ColumnSet::default();
        set.toggle(Col::Time);
        set.toggle(Col::Message);
        let l = layout(10.0, 900.0, set);
        assert_eq!(kinds(&l), vec![Col::Type, Col::Process, Col::Subsystem, Col::Category]);
        assert_eq!(l[1].1, 10.0 + 84.0);
        let last = l.last().unwrap();
        assert!((last.1 + last.2 - 910.0).abs() < 0.01, "the table is filled to its right edge");
    }

    #[test]
    fn the_last_visible_column_cannot_be_hidden() {
        let mut set = ColumnSet::default();
        for col in Col::ALL {
            if col != Col::Message && set.shows(col) {
                assert!(set.toggle(col));
            }
        }
        assert!(!set.toggle(Col::Message), "Message is the only column left");
        assert!(set.shows(Col::Message));
        assert!(set.toggle(Col::Pid), "showing a column always works");
        assert!(set.toggle(Col::Message), "and then Message is no longer the last one");
    }

    #[test]
    fn a_narrow_table_with_only_giving_way_columns_shows_message() {
        let mut set = ColumnSet::default();
        for col in [Col::Type, Col::Time, Col::Process, Col::Message] {
            set.toggle(col);
        }
        assert_eq!(kinds(&layout(0.0, 500.0, set)), vec![Col::Message]);
        assert_eq!(kinds(&layout(0.0, 900.0, set)), vec![Col::Subsystem, Col::Category]);
    }
}
