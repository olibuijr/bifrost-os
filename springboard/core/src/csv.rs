//! A reader for comma-separated text in the RFC 4180 shape: a quoted field may
//! hold commas, line breaks and doubled quotes, and records end in LF or CRLF.
//! [`parse_rows`] does not treat the first record as a header and accepts an
//! unclosed quote; [`try_parse_rows`] refuses it, and [`parse_table`] reads a
//! header and checks every record against it. Writing CSV is in the SDK
//! (`csv_writer`).

/// Why text is not a well-formed table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CsvError {
    /// A quoted field is never closed.
    UnterminatedQuote,
    /// A data record has a different number of fields from the header. `record`
    /// counts the data records from 1.
    RaggedRecord { record: usize, expected: usize, found: usize },
    /// The text has no record to read as the header.
    NoHeader,
}

/// Splits `text` into records, each a list of fields.
///
/// A final line break does not start an empty record. An empty line in the
/// middle is a record with one empty field. A quote opens a quoted field only
/// at the start of a field; a quote inside an unquoted field is kept as text.
/// An unclosed quote runs to the end of the text.
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
    scan(text).0
}

/// Like [`parse_rows`], but an unclosed quote is an error.
pub fn try_parse_rows(text: &str) -> Result<Vec<Vec<String>>, CsvError> {
    match scan(text) {
        (_, true) => Err(CsvError::UnterminatedQuote),
        (rows, false) => Ok(rows),
    }
}

/// Text read as a header record and data records of the same width. Blank lines
/// are skipped; a record with one empty field counts as blank, so a single-column
/// table cannot hold an empty value on its own line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

/// Reads `text` as a table: the first non-blank record is the header, and every
/// later non-blank record must have as many fields as the header.
pub fn parse_table(text: &str) -> Result<Table, CsvError> {
    let mut records = try_parse_rows(text)?.into_iter().filter(|record| !is_blank(record));
    let header = records.next().ok_or(CsvError::NoHeader)?;
    let mut rows = Vec::new();
    for (i, record) in records.enumerate() {
        if record.len() != header.len() {
            return Err(CsvError::RaggedRecord { record: i + 1, expected: header.len(), found: record.len() });
        }
        rows.push(record);
    }
    Ok(Table { header, rows })
}

fn is_blank(record: &[String]) -> bool {
    matches!(record, [only] if only.is_empty())
}

impl Table {
    pub fn header(&self) -> &[String] {
        &self.header
    }

    /// The number of data records.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Data record `i`, counted from 0.
    pub fn record(&self, i: usize) -> Option<&[String]> {
        self.rows.get(i).map(Vec::as_slice)
    }

    /// The value of column `name` in data record `i`. None when the column or the record does not exist.
    pub fn value(&self, i: usize, name: &str) -> Option<&str> {
        let column = self.header.iter().position(|h| h == name)?;
        self.rows.get(i).map(|row| row[column].as_str())
    }
}

/// The scanner behind every reader: the records, and whether a quoted field was still open at the end.
fn scan(text: &str) -> (Vec<Vec<String>>, bool) {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // True once the current record has consumed a character other than its line break.
    let mut touched = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        touched = c != '\n';
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if touched || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    (rows, quoted)
}
