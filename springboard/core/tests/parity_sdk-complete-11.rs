//! Parity sdk-complete-11: headless tests of the CSV reader's strict mode, its header table and its errors.

use bifrost_core::csv::{parse_table, try_parse_rows, CsvError};

#[test]
fn an_unclosed_quote_is_an_error_for_the_strict_reader() {
    assert_eq!(try_parse_rows("a,\"b"), Err(CsvError::UnterminatedQuote));
    assert_eq!(try_parse_rows("a,\"b\"\n"), Ok(vec![vec!["a".to_string(), "b".to_string()]]));
}

#[test]
fn a_table_maps_each_record_to_its_header() {
    let t = parse_table("name,price\r\nAlma,\"1,250\"\r\nBo,900\r\n").unwrap();
    assert_eq!(t.header().len(), 2);
    assert_eq!(t.len(), 2);
    assert_eq!(t.value(0, "name"), Some("Alma"));
    assert_eq!(t.value(0, "price"), Some("1,250"));
    assert_eq!(t.value(1, "price"), Some("900"));
    assert_eq!(t.value(2, "name"), None, "a record past the end does not exist");
    assert_eq!(t.value(0, "missing"), None, "an unknown column has no value");
}

#[test]
fn blank_lines_are_skipped_in_a_table() {
    let t = parse_table("\nsymbol,close\n\nAAPL,190.5\n\n").unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t.value(0, "symbol"), Some("AAPL"));
    assert_eq!(t.record(0).map(|r| r.len()), Some(2));
}

#[test]
fn a_record_with_the_wrong_width_names_its_position() {
    assert_eq!(parse_table("a,b\n1,2\n3\n"), Err(CsvError::RaggedRecord { record: 2, expected: 2, found: 1 }));
}

#[test]
fn a_table_needs_a_header_and_an_unclosed_quote_fails_it() {
    assert_eq!(parse_table("\n\n"), Err(CsvError::NoHeader));
    assert_eq!(parse_table("a,b\n\"1"), Err(CsvError::UnterminatedQuote));
}
