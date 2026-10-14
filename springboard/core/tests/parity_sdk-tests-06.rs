//! Parity tests for the CSV reader (`bifrost_core::csv`). Every check is pure: fixed CSV text in, concrete values out.

use bifrost_core::csv::parse_rows;

#[test]
fn csv_splits_plain_records_with_lf_and_crlf() {
    assert_eq!(parse_rows("a,b,c\n1,2,3\n"), vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]);
    assert_eq!(parse_rows("a,b\r\n1,2\r\n"), vec![vec!["a", "b"], vec!["1", "2"]], "CRLF ends a record too");
}

#[test]
fn csv_quoted_fields_keep_commas_and_doubled_quotes() {
    assert_eq!(
        parse_rows("name,note\n\"Doe, J\",\"said \"\"hi\"\"\"\n"),
        vec![vec!["name", "note"], vec!["Doe, J", "said \"hi\""]]
    );
}

#[test]
fn csv_quoted_fields_keep_line_breaks() {
    assert_eq!(parse_rows("x,\"line1\nline2\"\n"), vec![vec!["x", "line1\nline2"]]);
}

#[test]
fn csv_empty_fields_and_blank_lines_are_records() {
    assert_eq!(parse_rows("a,,c\n"), vec![vec!["a", "", "c"]], "an empty field is an empty cell");
    assert_eq!(parse_rows("a\n\nb"), vec![vec!["a"], vec![""], vec!["b"]], "a blank line in the middle is one empty field");
    assert_eq!(parse_rows("\"\""), vec![vec![""]], "a quoted empty field at the end still makes a record");
}

#[test]
fn csv_without_a_final_line_break_and_empty_input() {
    assert_eq!(parse_rows("a,b"), vec![vec!["a", "b"]]);
    assert!(parse_rows("").is_empty());
}

#[test]
fn csv_keeps_a_quote_inside_an_unquoted_field() {
    assert_eq!(parse_rows("a\"b,c"), vec![vec!["a\"b", "c"]]);
}
