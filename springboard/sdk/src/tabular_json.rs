//! Column-aware JSON for tables read with `bifrost_app::csv::parse_table`. A table becomes an array of objects, one
//! object per data record, keyed by the header names. A numeric column (see `tabular_summary`) is written as JSON
//! numbers, and an empty cell in a numeric column becomes `null`; a text column is written as strings, an empty text
//! cell as `""`.
//!
//! Reading goes the other way: every object must have the same keys, which become the header in the order of the
//! first object. Numbers, booleans and strings are read back as their text, and `null` as an empty cell. Nested arrays
//! and objects are refused, since a table cell holds one value.
//!
//! ```ignore
//! let table = bifrost_app::csv::parse_table("city,temp\nReykjavik,4\nAkureyri,\n")?;
//! let text = to_json(&table);   // [{"city":"Reykjavik","temp":4},{"city":"Akureyri","temp":null}]
//! ```
//!
//! Limits: reading gives text cells, not typed columns, so the column kinds are derived again on the next export. A
//! `null` in a text column reads back as an empty cell, the same as in a numeric one.

use bifrost_core::csv::Table;
use bifrost_core::json::{self, Json};

use crate::tabular_summary::{summarize, ColumnKind};

/// Why JSON text is not a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableJsonError {
    /// The text is not valid JSON.
    Parse(String),
    /// The top-level value is not an array of objects.
    NotAnArray,
    /// Record `record` (counted from 1) is not an object.
    NotAnObject { record: usize },
    /// Record `record` has a different set of keys from the first record.
    KeysDiffer { record: usize },
    /// Record `record` holds a nested array or object in the field `key`.
    NestedValue { record: usize, key: String },
    /// The first record has the same key twice, so the columns would be ambiguous.
    DuplicateKey(String),
    /// There are no records, so the header cannot be read.
    NoColumns,
}

/// The table as a JSON array of objects, with numeric columns written as numbers.
pub fn to_json(table: &Table) -> String {
    to_value(table).encode()
}

fn to_value(table: &Table) -> Json {
    let columns = summarize(table);
    let records = (0..table.len())
        .map(|i| {
            let row = table.record(i).unwrap_or(&[]);
            let fields = columns
                .iter()
                .enumerate()
                .map(|(column, summary)| {
                    let cell = row.get(column).map(String::as_str).unwrap_or("");
                    let value = match summary.kind {
                        ColumnKind::Number if cell.is_empty() => Json::Null,
                        ColumnKind::Number => cell.parse::<f64>().map(Json::Num).unwrap_or(Json::Null),
                        ColumnKind::Text => Json::Str(cell.to_string()),
                    };
                    (summary.name.clone(), value)
                })
                .collect();
            Json::Obj(fields)
        })
        .collect();
    Json::Arr(records)
}

/// Reads JSON text written by [`to_json`] (or any array of flat objects with the same keys) into a table.
pub fn from_json(text: &str) -> Result<Table, TableJsonError> {
    let value = json::parse(text).map_err(TableJsonError::Parse)?;
    let Json::Arr(records) = value else {
        return Err(TableJsonError::NotAnArray);
    };
    let first = match records.first() {
        None => return Err(TableJsonError::NoColumns),
        Some(Json::Obj(fields)) => fields,
        Some(_) => return Err(TableJsonError::NotAnObject { record: 1 }),
    };
    let header: Vec<String> = first.iter().map(|(key, _)| key.clone()).collect();
    for (i, key) in header.iter().enumerate() {
        if header[..i].contains(key) {
            return Err(TableJsonError::DuplicateKey(key.clone()));
        }
    }

    let mut rows = Vec::with_capacity(records.len());
    for (i, record) in records.iter().enumerate() {
        let number = i + 1;
        let Json::Obj(fields) = record else {
            return Err(TableJsonError::NotAnObject { record: number });
        };
        if fields.len() != header.len() {
            return Err(TableJsonError::KeysDiffer { record: number });
        }
        let mut row = Vec::with_capacity(header.len());
        for key in &header {
            let value = fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .ok_or(TableJsonError::KeysDiffer { record: number })?;
            row.push(match value {
                Json::Null => String::new(),
                Json::Str(s) => s.clone(),
                Json::Num(_) | Json::Bool(_) => value.encode(),
                Json::Arr(_) | Json::Obj(_) => {
                    return Err(TableJsonError::NestedValue { record: number, key: key.clone() });
                }
            });
        }
        rows.push(row);
    }
    Table::from_parts(header, rows).map_err(|_| TableJsonError::NoColumns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        bifrost_core::csv::parse_table(text).expect("valid table")
    }

    #[test]
    fn numeric_columns_are_written_as_numbers_and_empty_ones_as_null() {
        let text = to_json(&table("city,temp\nReykjavik,4\nAkureyri,\nHofn,-2.5\n"));
        assert_eq!(
            text,
            r#"[{"city":"Reykjavik","temp":4},{"city":"Akureyri","temp":null},{"city":"Hofn","temp":-2.5}]"#
        );
    }

    #[test]
    fn text_columns_keep_empty_cells_as_empty_strings() {
        let text = to_json(&table("a,b\nx,\ny,z\n"));
        assert_eq!(text, r#"[{"a":"x","b":""},{"a":"y","b":"z"}]"#);
    }

    #[test]
    fn a_table_without_data_records_is_an_empty_array() {
        assert_eq!(to_json(&table("a,b\n")), "[]");
    }

    #[test]
    fn export_then_import_keeps_the_cell_text() {
        let original = table("name,count\n\"Smith, J\",3\nLee,\n");
        let back = from_json(&to_json(&original)).expect("round trip");
        assert_eq!(back.header(), original.header());
        assert_eq!(back.value(0, "name"), Some("Smith, J"));
        assert_eq!(back.value(0, "count"), Some("3"));
        assert_eq!(back.value(1, "count"), Some(""));
    }

    #[test]
    fn keys_in_a_later_record_may_come_in_another_order() {
        let table = from_json(r#"[{"a":"1","b":"2"},{"b":"4","a":"3"}]"#).expect("valid");
        assert_eq!(table.value(1, "a"), Some("3"));
        assert_eq!(table.value(1, "b"), Some("4"));
    }

    #[test]
    fn a_record_with_a_different_key_set_is_refused() {
        assert_eq!(
            from_json(r#"[{"a":1,"b":2},{"a":3,"c":4}]"#),
            Err(TableJsonError::KeysDiffer { record: 2 })
        );
        assert_eq!(from_json(r#"[{"a":1,"b":2},{"a":3}]"#), Err(TableJsonError::KeysDiffer { record: 2 }));
    }

    #[test]
    fn the_top_level_value_must_be_an_array_of_objects() {
        assert_eq!(from_json(r#"{"a":1}"#), Err(TableJsonError::NotAnArray));
        assert_eq!(from_json("[1,2]"), Err(TableJsonError::NotAnObject { record: 1 }));
        assert_eq!(from_json(r#"[{"a":1},5]"#), Err(TableJsonError::NotAnObject { record: 2 }));
    }

    #[test]
    fn an_empty_array_has_no_columns_to_read() {
        assert_eq!(from_json("[]"), Err(TableJsonError::NoColumns));
    }

    #[test]
    fn nested_values_and_duplicate_keys_are_refused() {
        assert_eq!(
            from_json(r#"[{"a":[1]}]"#),
            Err(TableJsonError::NestedValue { record: 1, key: "a".to_string() })
        );
        assert_eq!(from_json(r#"[{"a":1,"a":2}]"#), Err(TableJsonError::DuplicateKey("a".to_string())));
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        assert!(matches!(from_json("[{"), Err(TableJsonError::Parse(_))));
    }

    #[test]
    fn booleans_and_numbers_are_read_as_their_text() {
        let table = from_json(r#"[{"flag":true,"n":2.5}]"#).expect("valid");
        assert_eq!(table.value(0, "flag"), Some("true"));
        assert_eq!(table.value(0, "n"), Some("2.5"));
    }
}
