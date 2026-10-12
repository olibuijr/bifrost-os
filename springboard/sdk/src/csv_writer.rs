//! Tabular data CSV writing (`CSVWritingError` and the writer that reports it): turns rows of fields into RFC 4180
//! text, with the delimiter, quoting and line ending chosen by the caller.
//!
//! A field is quoted when it holds the delimiter, a double quote, or a line break. Quotes inside a quoted field are
//! doubled. Every other field is written as it is, so an empty field is an empty cell. Fields given as bytes must be
//! valid UTF-8; the first bad field stops the write with `CsvWritingError::BadEncoding`, which names the row, the
//! column (its header name when a header was written) and the bytes that failed.

/// `CSVWritingError`: an error raised while writing a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CsvWritingError {
    /// `badEncoding(row:column:_:)`: a field is not valid UTF-8. `column` is the header name of the column, when the
    /// writer has a header for it.
    BadEncoding { row: usize, column: Option<String>, data: Vec<u8> },
}

impl std::fmt::Display for CsvWritingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CsvWritingError::BadEncoding { row, column, data } => match column {
                Some(name) => write!(f, "invalid UTF-8 in column {} of row {} ({} bytes)", name, row, data.len()),
                None => write!(f, "invalid UTF-8 at row {} ({} bytes)", row, data.len()),
            },
        }
    }
}

/// Writes CSV text one record at a time. Records are counted from 0, and a header counts as the first record.
#[derive(Clone, Debug)]
pub struct CsvWriter {
    delimiter: char,
    line_ending: &'static str,
    header: Vec<String>,
    row: usize,
    out: String,
}

impl Default for CsvWriter {
    fn default() -> Self {
        CsvWriter::new()
    }
}

impl CsvWriter {
    /// A writer with a comma delimiter and `\n` line endings.
    pub fn new() -> CsvWriter {
        CsvWriter { delimiter: ',', line_ending: "\n", header: Vec::new(), row: 0, out: String::new() }
    }

    /// Sets the field delimiter. Fields that contain it are quoted.
    pub fn with_delimiter(mut self, delimiter: char) -> CsvWriter {
        self.delimiter = delimiter;
        self
    }

    /// Sets the line ending, for example `\r\n` for RFC 4180 output.
    pub fn with_line_ending(mut self, line_ending: &'static str) -> CsvWriter {
        self.line_ending = line_ending;
        self
    }

    /// Writes the header record and keeps its names for error reports.
    pub fn write_header(&mut self, names: &[&str]) {
        self.header = names.iter().map(|n| n.to_string()).collect();
        let record: Vec<String> = self.header.clone();
        self.write_record(record);
    }

    /// Writes one record of text fields.
    pub fn write_row(&mut self, fields: &[&str]) {
        let record: Vec<String> = fields.iter().map(|f| f.to_string()).collect();
        self.write_record(record);
    }

    /// Writes one record of byte fields. A field that is not valid UTF-8 stops the write: nothing of this record is
    /// written, and the row counter does not move.
    pub fn write_byte_row(&mut self, fields: &[&[u8]]) -> Result<(), CsvWritingError> {
        let mut record = Vec::with_capacity(fields.len());
        for (column, bytes) in fields.iter().enumerate() {
            match std::str::from_utf8(bytes) {
                Ok(text) => record.push(text.to_string()),
                Err(_) => {
                    return Err(CsvWritingError::BadEncoding {
                        row: self.row,
                        column: self.header.get(column).cloned(),
                        data: bytes.to_vec(),
                    });
                }
            }
        }
        self.write_record(record);
        Ok(())
    }

    /// The text written so far.
    pub fn as_str(&self) -> &str {
        &self.out
    }

    /// The number of records written so far, the header included.
    pub fn record_count(&self) -> usize {
        self.row
    }

    /// Ends the write and returns the CSV text.
    pub fn finish(self) -> String {
        self.out
    }

    fn write_record(&mut self, fields: Vec<String>) {
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                self.out.push(self.delimiter);
            }
            self.push_field(field);
        }
        self.out.push_str(self.line_ending);
        self.row += 1;
    }

    fn push_field(&mut self, field: &str) {
        let needs_quotes = field.chars().any(|c| c == self.delimiter || c == '"' || c == '\n' || c == '\r');
        if !needs_quotes {
            self.out.push_str(field);
            return;
        }
        self.out.push('"');
        for c in field.chars() {
            if c == '"' {
                self.out.push('"');
            }
            self.out.push(c);
        }
        self.out.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_fields_are_written_with_commas_and_newlines() {
        let mut w = CsvWriter::new();
        w.write_header(&["name", "count"]);
        w.write_row(&["pears", "3"]);
        assert_eq!(w.as_str(), "name,count\npears,3\n");
        assert_eq!(w.record_count(), 2);
    }

    #[test]
    fn a_field_with_a_comma_or_a_quote_is_quoted_and_the_quotes_are_doubled() {
        let mut w = CsvWriter::new();
        w.write_row(&["he said \"hi\", ok", "plain"]);
        assert_eq!(w.finish(), "\"he said \"\"hi\"\", ok\",plain\n");
    }

    #[test]
    fn a_line_break_inside_a_field_is_quoted() {
        let mut w = CsvWriter::new();
        w.write_row(&["a\nb", "c"]);
        assert_eq!(w.finish(), "\"a\nb\",c\n");
    }

    #[test]
    fn a_custom_delimiter_and_line_ending_change_the_quoting_rule() {
        let mut w = CsvWriter::new().with_delimiter(';').with_line_ending("\r\n");
        w.write_row(&["a,b", "c;d"]);
        w.write_row(&["", "x"]);
        assert_eq!(w.finish(), "a,b;\"c;d\"\r\n;x\r\n");
    }

    #[test]
    fn invalid_utf8_stops_the_write_and_names_the_column() {
        let mut w = CsvWriter::new();
        w.write_header(&["x", "y"]);
        let err = w.write_byte_row(&[b"ok", &[0xff, 0x41]]).unwrap_err();
        assert_eq!(
            err,
            CsvWritingError::BadEncoding { row: 1, column: Some("y".to_string()), data: vec![0xff, 0x41] }
        );
        assert_eq!(w.as_str(), "x,y\n");
        assert_eq!(w.record_count(), 1);
    }

    #[test]
    fn valid_bytes_are_written_like_text() {
        let mut w = CsvWriter::new();
        assert!(w.write_byte_row(&[b"a", "é,".as_bytes()]).is_ok());
        assert_eq!(w.finish(), "a,\"é,\"\n");
    }
}
