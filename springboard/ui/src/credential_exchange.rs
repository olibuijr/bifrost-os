//! Credential exchange (parity: Authentication Services, "Credential import and export"): a versioned text document that
//! carries password records out of one credential manager and into another.
//!
//! Document layout: the first line is the header `bifrost-credential-exchange 1`. Every following line is one password
//! record with three tab-separated fields: service, user name and password. A field escapes `\\` as `\\\\`, a tab as
//! `\t`, a line feed as `\n` and a carriage return as `\r`, so any value fits on one line. Blank lines are skipped.
//!
//! Import rule: a record with the same service and user name as an earlier record replaces that record's password and
//! counts as a duplicate, so the last copy in the document wins. A document with a newer version number is refused,
//! not guessed at. Pure model: no file is read or written here, and the caller decides where the text goes. Passkey
//! private keys are never part of this format.

use std::fmt;

/// The first line of every document this module writes or accepts.
pub const HEADER: &str = "bifrost-credential-exchange 1";

/// One password record in the exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordRecord {
    pub service: String,
    pub user: String,
    pub password: String,
}

/// Why a document could not be imported. Line numbers are 1-based and count the header as line 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// The document is empty or its first line is not the exchange header.
    MissingHeader,
    /// The header names a version this build does not read.
    UnsupportedVersion,
    /// A record does not have exactly three fields, or a field has a bad escape.
    MalformedRecord(usize),
    /// A record has an empty service or user name.
    EmptyField(usize),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExchangeError::MissingHeader => f.write_str("the document has no credential exchange header"),
            ExchangeError::UnsupportedVersion => f.write_str("the document uses a newer exchange version"),
            ExchangeError::MalformedRecord(line) => write!(f, "line {line} is not a valid credential record"),
            ExchangeError::EmptyField(line) => write!(f, "line {line} has an empty service or user name"),
        }
    }
}

/// The result of an import: the records to save, in document order, and how many records replaced an earlier one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub records: Vec<PasswordRecord>,
    pub duplicates: usize,
}

/// Escapes one field so that it fits on a single tab-separated line.
pub fn escape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    for c in field.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Reverses [`escape`]. Returns `None` for a backslash that is not followed by a known escape letter.
pub fn unescape(field: &str) -> Option<String> {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

/// Writes the records as an exchange document, in the order given.
pub fn export(records: &[PasswordRecord]) -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push('\n');
    for record in records {
        out.push_str(&escape(&record.service));
        out.push('\t');
        out.push_str(&escape(&record.user));
        out.push('\t');
        out.push_str(&escape(&record.password));
        out.push('\n');
    }
    out
}

/// Reads an exchange document. Any error refuses the whole document, so a partial import never happens.
pub fn import(document: &str) -> Result<ImportReport, ExchangeError> {
    let mut lines = document.lines();
    let header = lines.next().ok_or(ExchangeError::MissingHeader)?;
    if header != HEADER {
        return Err(if header.starts_with("bifrost-credential-exchange ") {
            ExchangeError::UnsupportedVersion
        } else {
            ExchangeError::MissingHeader
        });
    }

    let mut report = ImportReport::default();
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let (service, user, password) = match fields.as_slice() {
            [service, user, password] => (*service, *user, *password),
            _ => return Err(ExchangeError::MalformedRecord(number)),
        };
        let service = unescape(service).ok_or(ExchangeError::MalformedRecord(number))?;
        let user = unescape(user).ok_or(ExchangeError::MalformedRecord(number))?;
        let password = unescape(password).ok_or(ExchangeError::MalformedRecord(number))?;
        if service.is_empty() || user.is_empty() {
            return Err(ExchangeError::EmptyField(number));
        }
        match report.records.iter().position(|r| r.service == service && r.user == user) {
            Some(existing) => {
                report.records[existing].password = password;
                report.duplicates += 1;
            }
            None => report.records.push(PasswordRecord { service, user, password }),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(service: &str, user: &str, password: &str) -> PasswordRecord {
        PasswordRecord {
            service: service.to_string(),
            user: user.to_string(),
            password: password.to_string(),
        }
    }

    #[test]
    fn escape_round_trips_special_characters() {
        let raw = "a\\b\tc\nd\re";
        let escaped = escape(raw);
        assert!(!escaped.contains('\t') && !escaped.contains('\n') && !escaped.contains('\r'));
        assert_eq!(unescape(&escaped).as_deref(), Some(raw));
    }

    #[test]
    fn unescape_refuses_unknown_escape() {
        assert_eq!(unescape("bad\\x"), None);
        assert_eq!(unescape("trailing\\"), None);
    }

    #[test]
    fn export_writes_header_and_one_line_per_record() {
        let text = export(&[record("example.com", "ada", "pw1"), record("site.org", "bo", "p\tw")]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(lines[1], "example.com\tada\tpw1");
        assert_eq!(lines[2], "site.org\tbo\tp\\tw");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn export_then_import_returns_the_same_records() {
        let records = vec![record("example.com", "ada", "pass\\word\nline")];
        let report = import(&export(&records)).expect("valid document");
        assert_eq!(report.records, records);
        assert_eq!(report.duplicates, 0);
    }

    #[test]
    fn import_refuses_a_missing_or_newer_header() {
        assert_eq!(import(""), Err(ExchangeError::MissingHeader));
        assert_eq!(import("something else\n"), Err(ExchangeError::MissingHeader));
        assert_eq!(import("bifrost-credential-exchange 2\n"), Err(ExchangeError::UnsupportedVersion));
    }

    #[test]
    fn import_reports_the_line_of_a_malformed_record() {
        let doc = format!("{HEADER}\na\tb\tc\nonly-two\tfields\n");
        assert_eq!(import(&doc), Err(ExchangeError::MalformedRecord(3)));
    }

    #[test]
    fn import_refuses_an_empty_service_or_user() {
        let doc = format!("{HEADER}\n\tada\tpw\n");
        assert_eq!(import(&doc), Err(ExchangeError::EmptyField(2)));
        let doc = format!("{HEADER}\nexample.com\t\tpw\n");
        assert_eq!(import(&doc), Err(ExchangeError::EmptyField(2)));
    }

    #[test]
    fn import_skips_blank_lines_and_keeps_document_order() {
        let doc = format!("{HEADER}\n\nb.com\tu\tp1\n\na.com\tu\tp2\n");
        let report = import(&doc).expect("valid document");
        assert_eq!(report.records, vec![record("b.com", "u", "p1"), record("a.com", "u", "p2")]);
    }

    #[test]
    fn a_repeated_service_and_user_replaces_the_earlier_password() {
        let doc = format!("{HEADER}\nexample.com\tada\told\nother.com\tada\tx\nexample.com\tada\tnew\n");
        let report = import(&doc).expect("valid document");
        assert_eq!(report.duplicates, 1);
        assert_eq!(report.records.len(), 2);
        assert_eq!(report.records[0], record("example.com", "ada", "new"));
    }
}
