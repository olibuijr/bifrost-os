//! Typed columns for tables read with `bifrost_app::csv::parse_table`. Each column is stored once as its kind decides
//! (see `tabular_summary`): a numeric column holds `Option<f64>` cells, where `None` is an empty cell, and a text column
//! holds its strings, with an empty string for an empty cell. Apps that compute over a column read these values
//! instead of parsing the cell text again.
//!
//! ```ignore
//! let table = bifrost_app::csv::parse_table("city,temp\nReykjavik,4\nAkureyri,\n")?;
//! let columns = typed_columns(&table);   // "temp" holds [Some(4.0), None]
//! ```
//!
//! Limits: a column is a plain vector, not a DataFrame: there are no row labels, joins, or per-column types beyond the
//! two kinds, and dates and booleans are text.

use bifrost_core::csv::Table;

use crate::tabular_summary::{summarize, ColumnKind};

/// One column of a table, typed by its kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    /// A numeric column; `None` marks an empty cell.
    Number { name: String, values: Vec<Option<f64>> },
    /// A text column; an empty cell is an empty string.
    Text { name: String, values: Vec<String> },
}

impl Column {
    /// The header name of the column.
    pub fn name(&self) -> &str {
        match self {
            Column::Number { name, .. } | Column::Text { name, .. } => name,
        }
    }

    /// The number of cells, which is the number of data records.
    pub fn len(&self) -> usize {
        match self {
            Column::Number { values, .. } => values.len(),
            Column::Text { values, .. } => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of empty cells.
    pub fn missing(&self) -> usize {
        match self {
            Column::Number { values, .. } => values.iter().filter(|v| v.is_none()).count(),
            Column::Text { values, .. } => values.iter().filter(|v| v.is_empty()).count(),
        }
    }

    /// The cells of a numeric column, or None for a text column.
    pub fn numbers(&self) -> Option<&[Option<f64>]> {
        match self {
            Column::Number { values, .. } => Some(values),
            Column::Text { .. } => None,
        }
    }

    /// The sum of the non-empty cells of a numeric column; None for a text column.
    pub fn sum(&self) -> Option<f64> {
        self.numbers().map(|values| values.iter().flatten().sum())
    }
}

/// Every column of `table`, in header order, each typed by [`summarize`].
pub fn typed_columns(table: &Table) -> Vec<Column> {
    summarize(table)
        .into_iter()
        .enumerate()
        .map(|(column, summary)| {
            let cell = |i: usize| table.record(i).map(|r| r[column].as_str()).unwrap_or("");
            match summary.kind {
                ColumnKind::Number => Column::Number {
                    name: summary.name,
                    values: (0..table.len()).map(|i| cell(i).parse::<f64>().ok()).collect(),
                },
                ColumnKind::Text => Column::Text {
                    name: summary.name,
                    values: (0..table.len()).map(|i| cell(i).to_string()).collect(),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        bifrost_core::csv::parse_table(text).expect("valid table")
    }

    #[test]
    fn a_numeric_column_holds_numbers_with_none_for_empty_cells() {
        let columns = typed_columns(&table("city,temp\nReykjavik,4\nAkureyri,\nHofn,-2.5\n"));
        let temp = &columns[1];
        assert_eq!(temp.name(), "temp");
        assert_eq!(temp.numbers(), Some(&[Some(4.0), None, Some(-2.5)][..]));
        assert_eq!(temp.missing(), 1);
        assert_eq!(temp.sum(), Some(1.5));
    }

    #[test]
    fn a_text_column_keeps_the_strings_and_empty_cells_as_empty_strings() {
        let columns = typed_columns(&table("city,temp\nReykjavik,4\nAkureyri,\n"));
        let city = &columns[0];
        assert_eq!(city.numbers(), None);
        assert_eq!(city.sum(), None);
        assert_eq!(
            city,
            &Column::Text { name: "city".to_string(), values: vec!["Reykjavik".to_string(), "Akureyri".to_string()] }
        );
        assert_eq!(columns[1].missing(), 1);
    }

    #[test]
    fn a_table_without_data_gives_empty_columns_in_header_order() {
        let columns = typed_columns(&table("a,b\n"));
        assert_eq!(columns.len(), 2);
        assert!(columns.iter().all(Column::is_empty));
        assert_eq!(columns[0].name(), "a");
        assert_eq!(columns[1].name(), "b");
    }

    #[test]
    fn an_all_empty_column_is_text() {
        let columns = typed_columns(&table("a,b\n,1\n,2\n"));
        assert!(matches!(columns[0], Column::Text { .. }));
        assert_eq!(columns[0].missing(), 2);
        assert_eq!(columns[1].sum(), Some(3.0));
    }
}
