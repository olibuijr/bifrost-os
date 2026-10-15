//! CSV and LDIF import for contacts. CSV is read with quoted fields and doubled quotes, and its columns are matched
//! by header name (full name, first and last name, company, phone, email, address, notes); the mapping can be
//! reviewed and changed before the import. LDIF reads `dn`-led entries with folded lines and base64 values.
//! Both return the same `exchange::Import` the vCard reader does. Pure std.
#![allow(dead_code)] // File > Import and the column-mapping sheet call these next

use crate::exchange::Import;
use springboard::shell::contactstore::Contact;

/// What a CSV column holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Skip,
    FullName,
    FirstName,
    LastName,
    Company,
    Phone,
    Email,
    Address,
    Notes,
}

impl Column {
    pub const ALL: [Column; 9] = [
        Column::Skip,
        Column::FullName,
        Column::FirstName,
        Column::LastName,
        Column::Company,
        Column::Phone,
        Column::Email,
        Column::Address,
        Column::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Column::Skip => "Do not import",
            Column::FullName => "Name",
            Column::FirstName => "First Name",
            Column::LastName => "Last Name",
            Column::Company => "Company",
            Column::Phone => "Phone",
            Column::Email => "Email",
            Column::Address => "Address",
            Column::Notes => "Notes",
        }
    }
}

/// Rows of fields from a CSV text. Handles `""` inside quotes, commas, semicolons (when the first line has more of
/// them than commas) and line breaks inside quotes. Blank lines are dropped.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first = text.lines().next().unwrap_or("");
    let sep = if first.matches(';').count() > first.matches(',').count() { ';' } else { ',' };
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, c) if c == sep => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

/// A guess of what each header names, for the mapping sheet's initial state.
pub fn guess_columns(header: &[String]) -> Vec<Column> {
    header
        .iter()
        .map(|h| {
            let h = h.trim().to_lowercase();
            let has = |words: &[&str]| words.iter().any(|w| h.contains(w));
            if has(&["first", "given", "fornafn"]) {
                Column::FirstName
            } else if has(&["last", "family", "surname", "eftirnafn"]) {
                Column::LastName
            } else if has(&["company", "organization", "organisation", "org", "fyrirtæki"]) {
                Column::Company
            } else if has(&["mail"]) {
                Column::Email
            } else if has(&["phone", "mobile", "tel", "sími", "gsm"]) {
                Column::Phone
            } else if has(&["address", "street", "heimilisfang"]) {
                Column::Address
            } else if has(&["note", "comment", "athugasemd"]) {
                Column::Notes
            } else if h == "name" || has(&["full name", "display name", "nafn"]) {
                Column::FullName
            } else {
                Column::Skip
            }
        })
        .collect()
}

/// Reads CSV `text` into contacts. When `mapping` is None the first row is a header and the columns are guessed;
/// with a mapping the first row is still skipped as the header.
pub fn import_csv(text: &str, mapping: Option<&[Column]>) -> Import {
    let rows = parse_csv(text);
    let mut out = Import::default();
    let Some((header, body)) = rows.split_first() else { return out };
    let guessed;
    let mapping = match mapping {
        Some(m) => m,
        None => {
            guessed = guess_columns(header);
            &guessed
        }
    };
    for row in body {
        let get = |col: Column| -> String {
            mapping
                .iter()
                .zip(row)
                .filter(|(m, _)| **m == col)
                .map(|(_, v)| v.trim())
                .find(|v| !v.is_empty())
                .unwrap_or("")
                .to_string()
        };
        let mut name = get(Column::FullName);
        if name.is_empty() {
            name = [get(Column::FirstName), get(Column::LastName)]
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
        // a second phone, email or address column is not kept; the count says how many were dropped
        let extra = |col: Column| {
            mapping.iter().zip(row).filter(|(m, v)| **m == col && !v.trim().is_empty()).count().saturating_sub(1)
        };
        out.extra_values += extra(Column::Phone) + extra(Column::Email);
        push(&mut out, contact(name, get(Column::Company), get(Column::Phone), get(Column::Email), get(Column::Address), get(Column::Notes)));
    }
    out
}

fn contact(name: String, company: String, phone: String, email: String, address: String, notes: String) -> Contact {
    Contact { id: 0, name, phone, email, notes, favorite: false, company, address, labels: [0; 3] }
}

fn push(out: &mut Import, c: Contact) {
    if c.valid() {
        out.contacts.push(c);
        out.values.push(vec![]);
    } else {
        out.invalid += 1;
    }
}

/// Decodes standard base64 (padding optional); None for invalid input.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = vec![];
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.trim().bytes().filter(|b| *b != b'=' && !b.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Reads an LDIF text. Each entry (separated by a blank line) becomes one contact; entries that are not people
/// (no `cn`, `givenName`, `sn`, `mail` or `o`) count as invalid.
pub fn import_ldif(text: &str) -> Import {
    let mut out = Import::default();
    // unfold continuation lines
    let mut lines: Vec<String> = vec![];
    for line in text.replace("\r\n", "\n").split('\n') {
        match line.strip_prefix(' ') {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(line.to_string()),
        }
    }
    let mut entry: Vec<(String, String)> = vec![];
    for line in lines.iter().map(String::as_str).chain(std::iter::once("")) {
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            if !entry.is_empty() {
                from_entry(&mut out, std::mem::take(&mut entry));
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = match value.strip_prefix(':') {
            Some(b64) => base64_decode(b64).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
            None => value.trim_start().to_string(),
        };
        entry.push((key.trim().to_lowercase(), value));
    }
    out
}

fn from_entry(out: &mut Import, entry: Vec<(String, String)>) {
    let get = |keys: &[&str]| -> String {
        entry.iter().find(|(k, v)| keys.contains(&k.as_str()) && !v.trim().is_empty()).map(|(_, v)| v.trim().to_string()).unwrap_or_default()
    };
    let count = |keys: &[&str]| entry.iter().filter(|(k, v)| keys.contains(&k.as_str()) && !v.trim().is_empty()).count();
    let is_person = entry.iter().any(|(k, _)| matches!(k.as_str(), "cn" | "givenname" | "sn" | "mail" | "o"));
    if !is_person {
        out.invalid += 1;
        return;
    }
    let mut name = get(&["cn", "displayname"]);
    if name.is_empty() {
        name = [get(&["givenname"]), get(&["sn"])].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    }
    let address = {
        let postal = get(&["postaladdress", "homepostaladdress"]);
        if postal.is_empty() {
            [get(&["street"]), get(&["postalcode"]), get(&["l"])].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ")
        } else {
            postal.replace('$', ", ")
        }
    };
    out.extra_values += count(&["mail"]).saturating_sub(1) + count(&["telephonenumber", "mobile", "homephone"]).saturating_sub(1);
    push(
        out,
        contact(
            name,
            get(&["o", "organizationname"]),
            get(&["telephonenumber", "mobile", "homephone"]),
            get(&["mail"]),
            address,
            get(&["description", "note"]),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_commas_and_newlines() {
        let rows = parse_csv("a,b\n\"x, y\",\"he said \"\"hi\"\"\"\n\"two\nlines\",z\n\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["x, y", "he said \"hi\""]);
        assert_eq!(rows[2], vec!["two\nlines", "z"]);
    }

    #[test]
    fn csv_semicolons_and_bom() {
        let rows = parse_csv("\u{feff}name;email\r\nAnn;ann@x.is\r\n");
        assert_eq!(rows, vec![vec!["name", "email"], vec!["Ann", "ann@x.is"]]);
    }

    #[test]
    fn header_guessing() {
        let h: Vec<String> = ["First Name", "Last Name", "E-mail", "Mobile phone", "Company", "Street address", "Notes", "Id"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            guess_columns(&h),
            vec![
                Column::FirstName,
                Column::LastName,
                Column::Email,
                Column::Phone,
                Column::Company,
                Column::Address,
                Column::Notes,
                Column::Skip
            ]
        );
    }

    #[test]
    fn csv_import_builds_contacts() {
        let text = "First Name,Last Name,Email,Phone,Company\nAnn,Lee,ann@x.is,555-1234,Shop\n,,,,\nBob,,bob@y.is,,\n,,,,Orphan Co\n";
        let i = import_csv(text, None);
        assert_eq!(i.contacts.len(), 3);
        assert_eq!(i.values.len(), 3);
        assert_eq!(i.contacts[0].name, "Ann Lee");
        assert_eq!(i.contacts[0].company, "Shop");
        assert_eq!(i.contacts[0].phone, "555-1234");
        assert_eq!(i.contacts[1].name, "Bob");
        assert_eq!(i.contacts[2].display_name(), "Orphan Co");
    }

    #[test]
    fn csv_mapping_override_and_extra_counts() {
        let text = "a,b,c\nAnn,111,222\n";
        let i = import_csv(text, Some(&[Column::FullName, Column::Phone, Column::Phone]));
        assert_eq!(i.contacts[0].phone, "111");
        assert_eq!(i.extra_values, 1);
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("SMOpbGzDsw==").unwrap(), "Hélló".as_bytes());
        assert_eq!(base64_decode("aGk").unwrap(), b"hi");
        assert!(base64_decode("a$b").is_none());
    }

    #[test]
    fn ldif_entries_folded_lines_and_base64() {
        let text = "version: 1\n\ndn: cn=Ann Lee,dc=x\nobjectClass: person\ncn: Ann Lee\nmail: ann@x.is\nmail: ann2@x.is\ntelephoneNumber: 555\no: Sho\n p\ncn:: SMOpbGzDsw==\n\ndn: ou=people,dc=x\nobjectClass: organizationalUnit\nou: people\n\ndn: cn=B\nsn: Bee\ngivenName: Bo\n";
        let i = import_ldif(text);
        assert_eq!(i.contacts.len(), 2);
        assert_eq!(i.invalid, 2); // the version header block and the unit
        assert_eq!(i.contacts[0].name, "Ann Lee");
        assert_eq!(i.contacts[0].company, "Shop");
        assert_eq!(i.contacts[0].email, "ann@x.is");
        assert_eq!(i.extra_values, 1);
        assert_eq!(i.contacts[1].name, "Bo Bee");
    }
}
