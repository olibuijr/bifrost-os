//! Visible columns and expanded rows of the log table: which columns show (and in which order), their stored form,
//! and the set of rows whose multi-line message is shown in full. Pure std.
#![allow(dead_code)] // View > Visible Columns and Expand/Collapse wire these into console.rs next

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Column {
    Type,
    Time,
    Process,
    Pid,
    Subsystem,
    Category,
    Message,
}

impl Column {
    pub const ALL: [Column; 7] =
        [Self::Type, Self::Time, Self::Process, Self::Pid, Self::Subsystem, Self::Category, Self::Message];

    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Time => "Time",
            Self::Process => "Process",
            Self::Pid => "PID",
            Self::Subsystem => "Subsystem",
            Self::Category => "Category",
            Self::Message => "Message",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Time => "time",
            Self::Process => "process",
            Self::Pid => "pid",
            Self::Subsystem => "subsystem",
            Self::Category => "category",
            Self::Message => "message",
        }
    }

    fn from_key(k: &str) -> Option<Column> {
        Self::ALL.into_iter().find(|c| c.key() == k)
    }
}

/// The columns shown, in table order. The Message column cannot be hidden, as the table would be empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visible {
    shown: BTreeSet<Column>,
}

impl Default for Visible {
    fn default() -> Self {
        Visible { shown: [Column::Type, Column::Time, Column::Process, Column::Message].into_iter().collect() }
    }
}

impl Visible {
    pub fn is_shown(&self, c: Column) -> bool {
        self.shown.contains(&c)
    }

    /// Flips a column; the Message column stays. Returns the new state.
    pub fn toggle(&mut self, c: Column) -> bool {
        if c == Column::Message {
            return true;
        }
        if !self.shown.remove(&c) {
            self.shown.insert(c);
            true
        } else {
            false
        }
    }

    /// The shown columns in table order.
    pub fn columns(&self) -> Vec<Column> {
        Column::ALL.into_iter().filter(|c| self.shown.contains(c)).collect()
    }

    /// Comma-separated keys.
    pub fn to_text(&self) -> String {
        self.columns().iter().map(|c| c.key()).collect::<Vec<_>>().join(",")
    }

    pub fn from_text(text: &str) -> Visible {
        let mut shown: BTreeSet<Column> = text.split(',').filter_map(|k| Column::from_key(k.trim())).collect();
        shown.insert(Column::Message);
        Visible { shown }
    }

    /// Widths of the shown columns for a table `total` points wide: fixed widths for the narrow columns, the rest
    /// to Message. Returns `(column, width)`.
    pub fn widths(&self, total: f32) -> Vec<(Column, f32)> {
        let fixed = |c: Column| match c {
            Column::Type => 72.0,
            Column::Time => 150.0,
            Column::Process => 130.0,
            Column::Pid => 56.0,
            Column::Subsystem => 160.0,
            Column::Category => 110.0,
            Column::Message => 0.0,
        };
        let used: f32 = self.columns().iter().map(|c| fixed(*c)).sum();
        let rest = (total - used).max(120.0);
        self.columns().into_iter().map(|c| (c, if c == Column::Message { rest } else { fixed(c) })).collect()
    }
}

/// Rows (by index in the current view) shown with their whole message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expanded {
    rows: BTreeSet<usize>,
}

impl Expanded {
    pub fn is_expanded(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }

    pub fn toggle(&mut self, row: usize) {
        if !self.rows.remove(&row) {
            self.rows.insert(row);
        }
    }

    pub fn expand(&mut self, rows: impl IntoIterator<Item = usize>) {
        self.rows.extend(rows);
    }

    pub fn collapse_all(&mut self) {
        self.rows.clear();
    }

    /// The row height: one line collapsed, one line per message line (capped) expanded.
    pub fn row_lines(&self, row: usize, message: &str, cap: usize) -> usize {
        if self.is_expanded(row) { message.lines().count().clamp(1, cap.max(1)) } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_columns_and_toggle() {
        let mut v = Visible::default();
        assert_eq!(v.columns(), vec![Column::Type, Column::Time, Column::Process, Column::Message]);
        assert!(v.toggle(Column::Pid));
        assert!(!v.toggle(Column::Time));
        assert_eq!(v.columns(), vec![Column::Type, Column::Process, Column::Pid, Column::Message]);
        assert!(v.toggle(Column::Message));
        assert!(v.is_shown(Column::Message));
    }

    #[test]
    fn text_round_trip_always_keeps_message() {
        let mut v = Visible::default();
        v.toggle(Column::Subsystem);
        assert_eq!(Visible::from_text(&v.to_text()), v);
        assert_eq!(Visible::from_text("junk,pid").columns(), vec![Column::Pid, Column::Message]);
        assert_eq!(Visible::from_text("").columns(), vec![Column::Message]);
    }

    #[test]
    fn message_gets_the_rest_of_the_width() {
        let v = Visible::default();
        let w = v.widths(800.0);
        assert_eq!(w.iter().map(|(_, x)| *x).sum::<f32>(), 800.0);
        assert_eq!(w.last().map(|(c, _)| *c), Some(Column::Message));
        let narrow = v.widths(100.0);
        assert_eq!(narrow.last().map(|(_, x)| *x), Some(120.0));
    }

    #[test]
    fn expansion() {
        let mut e = Expanded::default();
        assert_eq!(e.row_lines(3, "a\nb\nc", 10), 1);
        e.toggle(3);
        assert_eq!(e.row_lines(3, "a\nb\nc", 10), 3);
        assert_eq!(e.row_lines(3, "a\nb\nc", 2), 2);
        e.toggle(3);
        assert!(!e.is_expanded(3));
        e.expand([1, 2]);
        e.collapse_all();
        assert!(!e.is_expanded(1));
    }
}
