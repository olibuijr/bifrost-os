//! A reader for comma-separated text in the RFC 4180 shape: a quoted field may
//! hold commas, line breaks and doubled quotes, and records end in LF or CRLF.
//! The reader does not treat the first record as a header; callers decide.
//! Writing CSV is in the SDK (`csv_writer`).

/// Splits `text` into records, each a list of fields.
///
/// A final line break does not start an empty record. An empty line in the
/// middle is a record with one empty field. A quote opens a quoted field only
/// at the start of a field; a quote inside an unquoted field is kept as text.
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
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
    rows
}
