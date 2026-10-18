//! A small DataFrame over the typed columns of `tabular_columns`: named columns of one length, with the row operations
//! apps need to reshape a table without reparsing its text. Build one from a table read with
//! `bifrost_app::csv::parse_table`, select or filter rows and columns, sort by a column, and write the result back
//! as a table for `tabular_json` or `csv_writer`.
//!
//! ```ignore
//! let table = bifrost_app::csv::parse_table("city,temp\nReykjavik,4\nAkureyri,\nHofn,-2\n")?;
//! let frame = DataFrame::from_table(&table);
//! let warm = frame.filter(|i| frame.column("temp").and_then(|c| c.numbers()).map_or(false, |v| v[i].map_or(false, |t| t > 0.0)));
//! let sorted = frame.sort_by("temp", true)?;   // numbers descending, empty cells last
//! ```
//!
//! Limits: rows have no labels and there are no joins, group-bys or per-column types beyond the two kinds of
//! `tabular_columns`; dates and booleans stay text. Reading a spreadsheet file is not part of this module.

use std::cmp::Ordering;

use bifrost_core::csv::{CsvError, Table};

use crate::tabular_columns::{typed_columns, Column};

/// Why a frame cannot be built or reshaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Two columns share the name `0`.
    DuplicateName(String),
    /// Column `column` has `found` cells where the frame has `expected` rows.
    LengthMismatch { column: String, expected: usize, found: usize },
    /// No column is named `0`.
    UnknownColumn(String),
}

/// Named columns of one length, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct DataFrame {
    columns: Vec<Column>,
    rows: usize,
}

impl DataFrame {
    /// The frame of a table, with each column typed as `typed_columns` types it.
    pub fn from_table(table: &Table) -> DataFrame {
        DataFrame { columns: typed_columns(table), rows: table.len() }
    }

    /// A frame of the given columns. Names must be unique, and every column must have the length of the first.
    pub fn from_columns(columns: Vec<Column>) -> Result<DataFrame, FrameError> {
        let rows = columns.first().map_or(0, Column::len);
        for (i, column) in columns.iter().enumerate() {
            if columns[..i].iter().any(|c| c.name() == column.name()) {
                return Err(FrameError::DuplicateName(column.name().to_string()));
            }
            if column.len() != rows {
                return Err(FrameError::LengthMismatch {
                    column: column.name().to_string(),
                    expected: rows,
                    found: column.len(),
                });
            }
        }
        Ok(DataFrame { columns, rows })
    }

    /// The number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The columns, in order.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The column named `name`, if any.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name() == name)
    }

    /// A frame of only the named columns, in the order given.
    pub fn select(&self, names: &[&str]) -> Result<DataFrame, FrameError> {
        let columns = names
            .iter()
            .map(|name| self.column(name).cloned().ok_or_else(|| FrameError::UnknownColumn(name.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DataFrame { columns, rows: self.rows })
    }

    /// A frame of the rows for which `keep` returns true, given the row index counted from 0. Order is kept.
    pub fn filter(&self, keep: impl Fn(usize) -> bool) -> DataFrame {
        let order: Vec<usize> = (0..self.rows).filter(|&i| keep(i)).collect();
        self.take(&order)
    }

    /// A frame with the rows sorted by column `name`. Numbers sort by value and text by its bytes; empty cells come
    /// last whichever way the order runs. The sort is stable, so equal rows keep their order.
    pub fn sort_by(&self, name: &str, descending: bool) -> Result<DataFrame, FrameError> {
        let column = self.column(name).ok_or_else(|| FrameError::UnknownColumn(name.to_string()))?;
        let mut order: Vec<usize> = (0..self.rows).collect();
        order.sort_by(|&a, &b| compare_cells(column, a, b, descending));
        Ok(self.take(&order))
    }

    /// The frame as a table, with numbers written in their shortest form and empty numeric cells as empty text.
    pub fn to_table(&self) -> Result<Table, CsvError> {
        let header = self.columns.iter().map(|c| c.name().to_string()).collect();
        let rows = (0..self.rows)
            .map(|i| {
                self.columns
                    .iter()
                    .map(|column| match column {
                        Column::Number { values, .. } => values[i].map(|v| v.to_string()).unwrap_or_default(),
                        Column::Text { values, .. } => values[i].clone(),
                    })
                    .collect()
            })
            .collect();
        Table::from_parts(header, rows)
    }

    /// Every column reduced to the cells at `order`, in that order.
    fn take(&self, order: &[usize]) -> DataFrame {
        let columns = self
            .columns
            .iter()
            .map(|column| match column {
                Column::Number { name, values } => {
                    Column::Number { name: name.clone(), values: order.iter().map(|&i| values[i]).collect() }
                }
                Column::Text { name, values } => {
                    Column::Text { name: name.clone(), values: order.iter().map(|&i| values[i].clone()).collect() }
                }
            })
            .collect();
        DataFrame { columns, rows: order.len() }
    }
}

/// Orders rows `a` and `b` by one column. Empty cells are always last.
fn compare_cells(column: &Column, a: usize, b: usize, descending: bool) -> Ordering {
    let ordering = match column {
        Column::Number { values, .. } => match (values[a], values[b]) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        },
        Column::Text { values, .. } => match (values[a].is_empty(), values[b].is_empty()) {
            (false, false) => values[a].cmp(&values[b]),
            (false, true) => return Ordering::Less,
            (true, false) => return Ordering::Greater,
            (true, true) => return Ordering::Equal,
        },
    };
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &str) -> DataFrame {
        DataFrame::from_table(&bifrost_core::csv::parse_table(text).expect("valid table"))
    }

    #[test]
    fn a_frame_keeps_the_table_rows_and_typed_columns() {
        let f = frame("city,temp\nReykjavik,4\nAkureyri,\n");
        assert_eq!(f.rows(), 2);
        assert_eq!(f.columns().len(), 2);
        assert_eq!(f.column("temp").and_then(Column::numbers), Some(&[Some(4.0), None][..]));
        assert!(f.column("missing").is_none());
    }

    #[test]
    fn from_columns_refuses_duplicate_names_and_uneven_lengths() {
        let a = Column::Number { name: "a".into(), values: vec![Some(1.0), Some(2.0)] };
        let short = Column::Number { name: "b".into(), values: vec![Some(1.0)] };
        let dup = Column::Number { name: "a".into(), values: vec![Some(3.0), Some(4.0)] };
        assert_eq!(
            DataFrame::from_columns(vec![a.clone(), short]),
            Err(FrameError::LengthMismatch { column: "b".into(), expected: 2, found: 1 })
        );
        assert_eq!(DataFrame::from_columns(vec![a, dup]), Err(FrameError::DuplicateName("a".into())));
    }

    #[test]
    fn select_keeps_only_the_named_columns_in_the_order_given() {
        let f = frame("city,temp,wind\nHofn,-2,7\n");
        let s = f.select(&["wind", "city"]).expect("known columns");
        let names: Vec<&str> = s.columns().iter().map(Column::name).collect();
        assert_eq!(names, vec!["wind", "city"]);
        assert_eq!(s.rows(), 1);
        assert_eq!(f.select(&["nope"]), Err(FrameError::UnknownColumn("nope".into())));
    }

    #[test]
    fn filter_keeps_matching_rows_in_their_order() {
        let f = frame("city,temp\nA,4\nB,-2\nC,1\n");
        let warm = f.filter(|i| f.column("temp").and_then(Column::numbers).map_or(false, |v| v[i].map_or(false, |t| t > 0.0)));
        assert_eq!(warm.rows(), 2);
        assert_eq!(warm.column("city"), Some(&Column::Text { name: "city".into(), values: vec!["A".into(), "C".into()] }));
    }

    #[test]
    fn sort_by_a_number_puts_empty_cells_last_in_both_directions() {
        let f = frame("city,temp\nA,4\nB,\nC,-2\n");
        let up = f.sort_by("temp", false).expect("known column");
        assert_eq!(up.column("temp").and_then(Column::numbers), Some(&[Some(-2.0), Some(4.0), None][..]));
        let down = f.sort_by("temp", true).expect("known column");
        assert_eq!(down.column("temp").and_then(Column::numbers), Some(&[Some(4.0), Some(-2.0), None][..]));
    }

    #[test]
    fn sort_by_text_is_stable_for_equal_keys() {
        let f = frame("city,n\nB,1\nA,2\nB,3\n");
        let sorted = f.sort_by("city", false).expect("known column");
        assert_eq!(sorted.column("n").and_then(Column::numbers), Some(&[Some(2.0), Some(1.0), Some(3.0)][..]));
        assert_eq!(f.sort_by("nope", false), Err(FrameError::UnknownColumn("nope".into())));
    }

    #[test]
    fn to_table_round_trips_numbers_and_blank_cells() {
        let f = frame("city,temp\nA,4\nB,\nC,-2.5\n");
        let table = f.to_table().expect("rows match the header");
        assert_eq!(table.header(), &["city".to_string(), "temp".to_string()]);
        assert_eq!(table.value(0, "temp"), Some("4"));
        assert_eq!(table.value(1, "temp"), Some(""));
        assert_eq!(table.value(2, "temp"), Some("-2.5"));
    }
}
