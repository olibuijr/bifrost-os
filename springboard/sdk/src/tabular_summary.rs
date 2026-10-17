//! Column summaries for tables read with `bifrost_app::csv::parse_table`: the kind of each column (number or text), how
//! many cells are empty, the minimum, maximum and mean of a numeric column, and the distinct values of a text column
//! with the most common one.
//!
//! A column is numeric when every non-empty cell parses as a number (a leading `+` or `-`, a decimal point and an
//! exponent are allowed). Empty cells are counted as missing and left out of every statistic. Otherwise the column is
//! text. Ties in the most common value go to the value that sorts first.
//!
//! ```ignore
//! let table = bifrost_app::csv::parse_table("city,temp\nReykjavik,4\nAkureyri,\n")?;
//! let columns = summarize(&table);   // "temp" is numeric with one missing cell and mean 4
//! ```
//!
//! Limits: no DataFrame and no typed column storage; the summaries are computed from the table's text on each call.
//! Dates and booleans are text columns, and no categorical or histogram statistics are computed.

use std::collections::BTreeMap;

use bifrost_core::csv::Table;

/// Whether a column holds numbers or text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Number,
    Text,
}

/// Min, max and mean of the non-empty cells of a numeric column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

/// The summary of one column.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnSummary {
    /// The header name of the column.
    pub name: String,
    pub kind: ColumnKind,
    /// Cells that are empty.
    pub missing: usize,
    /// Distinct non-empty values.
    pub distinct: usize,
    /// The most common non-empty value and how many cells hold it.
    pub most_common: Option<(String, usize)>,
    /// Present only for a numeric column with at least one non-empty cell.
    pub numeric: Option<NumericStats>,
}

fn parses_as_number(cell: &str) -> Option<f64> {
    cell.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// One summary per column, in header order. A table with no data records gives empty text columns.
pub fn summarize(table: &Table) -> Vec<ColumnSummary> {
    table
        .header()
        .iter()
        .enumerate()
        .map(|(column, name)| {
            let cells: Vec<&str> = (0..table.len()).filter_map(|i| table.record(i).map(|r| r[column].as_str())).collect();
            summarize_column(name, &cells)
        })
        .collect()
}

fn summarize_column(name: &str, cells: &[&str]) -> ColumnSummary {
    let filled: Vec<&str> = cells.iter().copied().filter(|c| !c.is_empty()).collect();
    let missing = cells.len() - filled.len();

    let numbers: Option<Vec<f64>> = filled.iter().map(|c| parses_as_number(c)).collect();
    let kind = if numbers.is_some() && !filled.is_empty() { ColumnKind::Number } else { ColumnKind::Text };

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for cell in &filled {
        *counts.entry(*cell).or_insert(0) += 1;
    }
    let mut most_common: Option<(String, usize)> = None;
    for (value, n) in &counts {
        if most_common.as_ref().map_or(true, |(_, best)| n > best) {
            most_common = Some(((*value).to_string(), *n));
        }
    }

    let numeric = match (kind, numbers) {
        (ColumnKind::Number, Some(values)) => {
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            Some(NumericStats { min, max, mean })
        }
        _ => None,
    };

    ColumnSummary { name: name.to_string(), kind, missing, distinct: counts.len(), most_common, numeric }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        bifrost_core::csv::parse_table(text).expect("valid table")
    }

    #[test]
    fn numeric_column_has_min_max_mean_and_skips_missing_cells() {
        let columns = summarize(&table("city,temp\nReykjavik,4\nAkureyri,\nHofn,-2\n"));
        let temp = &columns[1];
        assert_eq!(temp.name, "temp");
        assert_eq!(temp.kind, ColumnKind::Number);
        assert_eq!(temp.missing, 1);
        assert_eq!(temp.numeric, Some(NumericStats { min: -2.0, max: 4.0, mean: 1.0 }));
    }

    #[test]
    fn text_column_counts_distinct_values_and_most_common() {
        let columns = summarize(&table("city,temp\nReykjavik,4\nReykjavik,5\nHofn,\n"));
        let city = &columns[0];
        assert_eq!(city.kind, ColumnKind::Text);
        assert_eq!(city.distinct, 2);
        assert_eq!(city.most_common, Some(("Reykjavik".to_string(), 2)));
        assert_eq!(city.numeric, None);
    }

    #[test]
    fn a_single_non_number_makes_the_column_text() {
        let columns = summarize(&table("code\n12\nA7\n"));
        assert_eq!(columns[0].kind, ColumnKind::Text);
        assert_eq!(columns[0].numeric, None);
    }

    #[test]
    fn ties_in_the_most_common_value_go_to_the_first_sorted_value() {
        let columns = summarize(&table("word\nbeta\nalpha\n"));
        assert_eq!(columns[0].most_common, Some(("alpha".to_string(), 1)));
    }

    #[test]
    fn an_all_empty_column_is_text_with_no_statistics() {
        let columns = summarize(&table("a,b\n,1\n,2\n"));
        assert_eq!(columns[0].kind, ColumnKind::Text);
        assert_eq!(columns[0].missing, 2);
        assert_eq!(columns[0].most_common, None);
        assert_eq!(columns[0].distinct, 0);
    }

    #[test]
    fn a_table_without_data_records_gives_empty_columns() {
        let columns = summarize(&table("a,b\n"));
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].missing, 0);
        assert_eq!(columns[0].kind, ColumnKind::Text);
    }

    #[test]
    fn infinite_and_nan_text_is_not_a_number() {
        let columns = summarize(&table("v\ninf\n1\n"));
        assert_eq!(columns[0].kind, ColumnKind::Text);
    }
}
