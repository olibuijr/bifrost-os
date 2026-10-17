//! Text-file import for contacts: comma, semicolon or tab separated tables (CSV) and LDIF directory exports.
//!
//! A table's first row names its columns. Each name is matched to a contact field by a table of known spellings
//! (`Name`, `First Name`, `Mobile Phone`, `E-mail Address`, `Home Street`, ...); a column that matches nothing is
//! ignored and reported, so the caller can show how the columns were mapped. LDIF records map their attributes the
//! same way (`cn`, `givenName`, `mail`, `telephoneNumber`, ...). Both produce the same `Import` as a vCard file, so the
//! caller merges them the same way. Pure std.

use crate::details::{self, CardDetails, Detail};
use crate::exchange::Import;
use crate::extras::{Extra, ValueKind, MAX_EXTRAS};
use springboard::shell::contactstore::{Contact, LABELS, MAX_NOTES};
use std::collections::BTreeMap;

/// Largest text accepted, in bytes.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Most contacts read from one text.
pub const MAX_CONTACTS: usize = 10_000;

/// What a text import read, and how the columns or attributes were mapped.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TextImport {
    pub import: Import,
    /// The column or attribute names that were mapped to a field, in first-seen order.
    pub mapped: Vec<String>,
    /// The names that matched no field, in first-seen order.
    pub ignored: Vec<String>,
}

impl TextImport {
    /// One line that says how the names were mapped, for the status line.
    pub fn mapping_summary(&self) -> String {
        let mut text = format!("{} columns mapped", self.mapped.len());
        if !self.ignored.is_empty() {
            text.push_str(&format!(", ignored: {}", self.ignored.join(", ")));
        }
        text
    }
}

/// A part of a postal address. The order is the order the parts are joined in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Street,
    City,
    State,
    Post,
    Country,
}

/// What a column or an attribute holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    First,
    Last,
    Full,
    Company,
    Department,
    Title,
    Nickname,
    Birthday,
    Url,
    Notes,
    Phone(&'static str),
    Email(&'static str),
    Address(&'static str, Part),
    Ignored,
}

/// A name with only letters and digits, lower case: `E-mail Address` becomes `emailaddress`.
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The store label for a phone column or attribute: mobile, home, work or other.
fn phone_label(key: &str) -> &'static str {
    if key.contains("mobile") || key.contains("cell") {
        "mobile"
    } else if key.contains("home") {
        "home"
    } else if key.contains("work") || key.contains("business") || key.contains("office") {
        "work"
    } else {
        "other"
    }
}

/// The store label for an email or address column: home, work or other.
fn place_label(key: &str) -> &'static str {
    if key.contains("home") || key.contains("private") || key.contains("personal") {
        "home"
    } else if key.contains("work") || key.contains("business") || key.contains("office") {
        "work"
    } else {
        "other"
    }
}

/// The address part a column name stands for, if it is one.
fn address_part(key: &str) -> Option<Part> {
    if key.contains("street") || key.ends_with("address") || key == "address" {
        Some(Part::Street)
    } else if key.contains("city") || key.contains("town") {
        Some(Part::City)
    } else if key.contains("state") || key.contains("province") || key.contains("region") {
        Some(Part::State)
    } else if key.contains("postal") || key.contains("zip") || key.contains("postcode") {
        Some(Part::Post)
    } else if key.contains("country") {
        Some(Part::Country)
    } else {
        None
    }
}

/// Matches a table column name to a field.
fn classify(name: &str) -> Column {
    let key = key(name);
    match key.as_str() {
        "" => return Column::Ignored,
        "firstname" | "givenname" | "given" | "first" => return Column::First,
        "lastname" | "familyname" | "surname" | "family" | "last" => return Column::Last,
        "name" | "fullname" | "displayname" | "contactname" => return Column::Full,
        "company" | "companyname" | "organization" | "organisation" | "org" | "employer" => return Column::Company,
        "department" | "dept" => return Column::Department,
        "jobtitle" | "title" | "position" => return Column::Title,
        "nickname" | "nick" => return Column::Nickname,
        "birthday" | "birthdate" | "bday" | "dateofbirth" => return Column::Birthday,
        "url" | "website" | "webpage" | "homepage" => return Column::Url,
        "notes" | "note" | "comments" | "comment" | "description" => return Column::Notes,
        _ => {}
    }
    if key.contains("fax") || key.contains("pager") {
        return Column::Ignored;
    }
    // `Mailing Address` is a postal address; `Email`, `E-mail Address` and `Work Mail` are email.
    if key.contains("email") || key.ends_with("mail") {
        return Column::Email(place_label(&key));
    }
    if key.contains("phone") || key.contains("mobile") || key.contains("cell") || key == "tel" || key.starts_with("tel") {
        return Column::Phone(phone_label(&key));
    }
    match address_part(&key) {
        Some(part) => Column::Address(place_label(&key), part),
        None => Column::Ignored,
    }
}

/// Matches an LDIF attribute name (lower case, without options) to a field.
fn classify_attribute(name: &str) -> Column {
    match name {
        "cn" | "displayname" => Column::Full,
        "givenname" => Column::First,
        "sn" | "surname" => Column::Last,
        "o" | "organization" | "organizationname" => Column::Company,
        "ou" | "department" => Column::Department,
        "title" => Column::Title,
        "mozillanickname" | "nickname" => Column::Nickname,
        "description" | "note" => Column::Notes,
        "labeleduri" | "url" | "mozillahomeurl" | "mozillaworkurl" => Column::Url,
        "mail" | "mailalternateaddress" => Column::Email("other"),
        "telephonenumber" => Column::Phone("work"),
        "homephone" => Column::Phone("home"),
        "mobile" | "cellphone" | "mobiletelephonenumber" => Column::Phone("mobile"),
        "street" | "postaladdress" => Column::Address("work", Part::Street),
        "homepostaladdress" | "mozillahomestreet" => Column::Address("home", Part::Street),
        "l" | "locality" => Column::Address("work", Part::City),
        "st" => Column::Address("work", Part::State),
        "postalcode" => Column::Address("work", Part::Post),
        "c" | "co" => Column::Address("work", Part::Country),
        _ => Column::Ignored,
    }
}

/// The contact being read from one row or record.
#[derive(Default)]
struct Draft {
    first: String,
    last: String,
    full: String,
    company: String,
    department: String,
    title: String,
    nickname: String,
    birthday: String,
    url: String,
    notes: String,
    phones: Vec<(&'static str, String)>,
    emails: Vec<(&'static str, String)>,
    places: Vec<(&'static str, BTreeMap<Part, String>)>,
}

fn keep_first(slot: &mut String, value: &str) {
    if slot.is_empty() {
        *slot = value.to_string();
    }
}

impl Draft {
    /// Puts a value in its field. Empty values are skipped; the first value of a single-valued field wins.
    fn apply(&mut self, column: Column, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        match column {
            Column::First => keep_first(&mut self.first, value),
            Column::Last => keep_first(&mut self.last, value),
            Column::Full => keep_first(&mut self.full, value),
            Column::Company => keep_first(&mut self.company, value),
            Column::Department => keep_first(&mut self.department, value),
            Column::Title => keep_first(&mut self.title, value),
            Column::Nickname => keep_first(&mut self.nickname, value),
            Column::Birthday => keep_first(&mut self.birthday, value),
            Column::Url => keep_first(&mut self.url, value.split_whitespace().next().unwrap_or(value)),
            Column::Notes => {
                if !self.notes.is_empty() {
                    self.notes.push('\n');
                }
                self.notes.push_str(value);
            }
            Column::Phone(label) => self.phones.push((label, value.to_string())),
            Column::Email(label) => self.emails.push((label, value.to_string())),
            Column::Address(label, part) => {
                let at = match self.places.iter().position(|(l, _)| *l == label) {
                    Some(at) => at,
                    None => {
                        self.places.push((label, BTreeMap::new()));
                        self.places.len() - 1
                    }
                };
                self.places[at].1.entry(part).or_insert_with(|| value.to_string());
            }
            Column::Ignored => {}
        }
    }

    /// The store contact, its extra values (the second and later phones, emails and addresses, up to `MAX_EXTRAS`), its
    /// card fields, and how many extra values were dropped.
    fn finish(self) -> (Contact, Vec<Extra>, CardDetails, usize) {
        let person = [self.first.as_str(), self.last.as_str()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        let name = if self.full.is_empty() { person } else { self.full.clone() };
        let addresses: Vec<(&'static str, String)> = self
            .places
            .iter()
            .map(|(label, parts)| (*label, parts.values().cloned().collect::<Vec<_>>().join(", ")))
            .filter(|(_, address)| !address.is_empty())
            .collect();
        let index = |kind: usize, label: &str| LABELS[kind].iter().position(|l| *l == label).unwrap_or(LABELS[kind].len() - 1) as u8;
        let contact = Contact {
            id: 0,
            name,
            phone: self.phones.first().map(|p| p.1.clone()).unwrap_or_default(),
            email: self.emails.first().map(|e| e.1.clone()).unwrap_or_default(),
            notes: self.notes.chars().take(MAX_NOTES).collect(),
            favorite: false,
            company: self.company.clone(),
            address: addresses.first().map(|a| a.1.clone()).unwrap_or_default(),
            uid: String::new(),
            labels: [
                self.phones.first().map_or(3, |p| index(0, p.0)),
                self.emails.first().map_or(2, |e| index(1, e.0)),
                addresses.first().map_or(2, |a| index(2, a.0)),
            ],
        };
        let spare = self
            .phones
            .iter()
            .skip(1)
            .map(|(label, value)| Extra { kind: ValueKind::Phone, label: (*label).into(), value: value.clone() })
            .chain(self.emails.iter().skip(1).map(|(label, value)| Extra { kind: ValueKind::Email, label: (*label).into(), value: value.clone() }))
            .chain(addresses.iter().skip(1).map(|(label, value)| Extra { kind: ValueKind::Address, label: (*label).into(), value: value.clone() }));
        let mut values = Vec::new();
        let mut dropped = 0;
        for extra in spare {
            if values.len() < MAX_EXTRAS && extra.well_formed() {
                values.push(extra);
            } else {
                dropped += 1;
            }
        }
        let mut card = CardDetails::default();
        card.set(Detail::Nickname, &self.nickname);
        card.set(Detail::JobTitle, &self.title);
        card.set(Detail::Department, &self.department);
        card.set(Detail::Url, &self.url);
        card.set(Detail::Birthday, &details::normalize_birthday(&self.birthday));
        (contact, values, card, dropped)
    }
}

/// Adds a finished draft to the import: valid contacts are kept, the rest are counted.
fn push(out: &mut Import, draft: Draft) {
    let (contact, values, card, dropped) = draft.finish();
    out.extra_values += dropped;
    if contact.valid() {
        out.contacts.push(contact);
        out.values.push(values);
        out.details.push(card);
    } else {
        out.invalid += 1;
    }
}

fn note_name(list: &mut Vec<String>, name: &str) {
    if !name.is_empty() && !list.iter().any(|n| n == name) {
        list.push(name.to_string());
    }
}

// ---- CSV ----

/// The separator a table uses: the one of comma, semicolon and tab that is most frequent in the first line outside
/// quotes. A comma when none appears.
fn delimiter(text: &str) -> char {
    let mut counts = [(',', 0usize), (';', 0), ('\t', 0)];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            c if !quoted => {
                if let Some(slot) = counts.iter_mut().find(|slot| slot.0 == c) {
                    slot.1 += 1;
                }
            }
            _ => {}
        }
    }
    counts.iter().fold((',', 0), |best, &(d, n)| if n > best.1 { (d, n) } else { best }).0
}

/// Ends the row being read: the open cell joins it, and a row with text in any cell joins the table.
fn end_row(row: &mut Vec<String>, cell: &mut String, out: &mut Vec<Vec<String>>) {
    row.push(std::mem::take(cell));
    let finished = std::mem::take(row);
    if finished.iter().any(|c| !c.trim().is_empty()) {
        out.push(finished);
    }
}

/// The rows of a table. A field in double quotes may hold the separator and line breaks; `""` is a quote. Rows with no
/// text in any cell are dropped.
fn rows(text: &str, separator: char) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' && cell.is_empty() {
            quoted = true;
        } else if c == separator {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' {
            end_row(&mut row, &mut cell, &mut out);
        } else if c != '\r' {
            cell.push(c);
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        end_row(&mut row, &mut cell, &mut out);
    }
    out
}

/// Reads a table whose first row names the columns.
pub fn import_csv(text: &str) -> Result<TextImport, String> {
    if text.len() > MAX_BYTES {
        return Err("The file is too large to import".into());
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let table = rows(text, delimiter(text));
    let Some((header, body)) = table.split_first() else { return Err("The file is empty".into()) };
    let columns: Vec<Column> = header.iter().map(|name| classify(name)).collect();
    if columns.iter().all(|c| *c == Column::Ignored) {
        return Err("The first row needs column names such as Name, Phone and Email".into());
    }
    let mut out = TextImport::default();
    for (name, column) in header.iter().zip(&columns) {
        let name = name.trim();
        if *column == Column::Ignored {
            note_name(&mut out.ignored, name);
        } else {
            note_name(&mut out.mapped, name);
        }
    }
    for row in body.iter().take(MAX_CONTACTS) {
        let mut draft = Draft::default();
        for (column, value) in columns.iter().zip(row) {
            draft.apply(*column, value);
        }
        push(&mut out.import, draft);
    }
    Ok(out)
}

// ---- LDIF ----

/// Decodes standard base64 (padding optional, white space skipped). `None` for any other character.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut bits, mut have) = (0u32, 0u32);
    for c in text.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | value;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    Some(out)
}

/// The logical lines of an LDIF text: a line that starts with one space continues the line before it. Comment lines
/// are dropped.
fn ldif_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if line.starts_with('#') {
            continue;
        }
        out.push(line.to_string());
    }
    out
}

/// Reads an LDIF text: records separated by blank lines, each a list of `attribute: value` lines (`attribute:: base64`
/// for encoded values). Records of a group object class are skipped and counted.
pub fn import_ldif(text: &str) -> Result<TextImport, String> {
    if text.len() > MAX_BYTES {
        return Err("The file is too large to import".into());
    }
    let mut out = TextImport::default();
    let mut record: Vec<(String, String)> = Vec::new();
    let mut records: Vec<Vec<(String, String)>> = Vec::new();
    for line in ldif_lines(text.strip_prefix('\u{feff}').unwrap_or(text)) {
        if line.trim().is_empty() {
            if !record.is_empty() {
                records.push(std::mem::take(&mut record));
            }
            continue;
        }
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.split(';').next().unwrap_or("").trim().to_lowercase();
        let value = if let Some(encoded) = rest.strip_prefix(':') {
            match base64_decode(encoded) {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None => continue,
            }
        } else if rest.starts_with('<') {
            continue; // a value fetched from a URL is not read
        } else {
            rest.trim().to_string()
        };
        record.push((name, value));
    }
    if !record.is_empty() {
        records.push(record);
    }
    for record in records.into_iter().take(MAX_CONTACTS) {
        if record.iter().all(|(name, _)| name == "version") {
            continue;
        }
        if record.iter().any(|(name, value)| name == "objectclass" && value.to_lowercase().contains("group")) {
            out.import.groups += 1;
            continue;
        }
        let mut draft = Draft::default();
        for (name, value) in &record {
            if matches!(name.as_str(), "dn" | "objectclass" | "version") {
                continue;
            }
            let column = classify_attribute(name);
            if column == Column::Ignored {
                note_name(&mut out.ignored, name);
                continue;
            }
            note_name(&mut out.mapped, name);
            // postalAddress joins its lines with `$`
            let value = if matches!(column, Column::Address(..)) { value.replace('$', ", ") } else { value.clone() };
            draft.apply(column, &value);
        }
        push(&mut out.import, draft);
    }
    Ok(out)
}

/// Which reader a file name needs, from its extension: `csv`, `tsv` and `txt` are tables, `ldif` and `ldf` are LDIF.
pub fn reader_for(extension: &str) -> Option<Reader> {
    match extension.to_ascii_lowercase().as_str() {
        "csv" | "tsv" | "txt" => Some(import_csv as Reader),
        "ldif" | "ldf" => Some(import_ldif as Reader),
        _ => None,
    }
}

/// A function that reads one text file kind.
pub type Reader = fn(&str) -> Result<TextImport, String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names_map_to_fields_whatever_their_spelling() {
        assert_eq!(classify("First Name"), Column::First);
        assert_eq!(classify("given_name"), Column::First);
        assert_eq!(classify("Surname"), Column::Last);
        assert_eq!(classify("E-mail Address"), Column::Email("other"));
        assert_eq!(classify("Work Email"), Column::Email("work"));
        assert_eq!(classify("Mobile Phone"), Column::Phone("mobile"));
        assert_eq!(classify("Home phone"), Column::Phone("home"));
        assert_eq!(classify("Business Phone"), Column::Phone("work"));
        assert_eq!(classify("Home Street"), Column::Address("home", Part::Street));
        assert_eq!(classify("Zip Code"), Column::Address("other", Part::Post));
        assert_eq!(classify("Business Fax"), Column::Ignored);
        assert_eq!(classify("Favourite colour"), Column::Ignored);
        assert_eq!(classify(""), Column::Ignored);
    }

    #[test]
    fn a_table_is_read_with_labelled_phones_emails_and_an_address() {
        let text = "First Name,Last Name,Company,Mobile Phone,Work Phone,Home Email,Work Email,Home Street,Home City,Notes,Fax\n\
Anna,Lindqvist,Acme,555 0100,555 0200,anna@home.is,anna@acme.is,Gata 1,Reykjavik,Met at a fair,555 9999\n";
        let got = import_csv(text).unwrap();
        assert_eq!(got.import.invalid, 0);
        let c = &got.import.contacts[0];
        assert_eq!((c.name.as_str(), c.company.as_str()), ("Anna Lindqvist", "Acme"));
        assert_eq!((c.phone.as_str(), c.email.as_str(), c.address.as_str()), ("555 0100", "anna@home.is", "Gata 1, Reykjavik"));
        assert_eq!(c.labels, [0, 0, 0], "mobile, home email, home address");
        assert_eq!(c.notes, "Met at a fair");
        assert_eq!(
            got.import.values[0],
            vec![
                Extra { kind: ValueKind::Phone, label: "work".into(), value: "555 0200".into() },
                Extra { kind: ValueKind::Email, label: "work".into(), value: "anna@acme.is".into() },
            ]
        );
        assert_eq!(got.ignored, vec!["Fax".to_string()]);
        assert!(got.mapping_summary().contains("ignored: Fax"));
    }

    #[test]
    fn quotes_separators_and_line_breaks_inside_a_field_are_kept() {
        let text = "Name;Notes\r\n\"Bo; Berg\";\"He said \"\"hi\"\"\nand left\"\r\nCara Cole;\r\n";
        let got = import_csv(text).unwrap();
        assert_eq!(got.import.contacts.len(), 2);
        assert_eq!(got.import.contacts[0].name, "Bo; Berg");
        assert_eq!(got.import.contacts[0].notes, "He said \"hi\"\nand left");
        assert_eq!(got.import.contacts[1].name, "Cara Cole");
    }

    #[test]
    fn the_separator_is_found_from_the_first_line() {
        assert_eq!(delimiter("a,b,c\n1;2;3"), ',');
        assert_eq!(delimiter("a;b;c,d\n"), ';');
        assert_eq!(delimiter("a\tb\tc\n"), '\t');
        assert_eq!(delimiter("\"a,b,c\";d\n"), ';', "separators inside quotes do not count");
        assert_eq!(delimiter("single\n"), ',');
    }

    #[test]
    fn blank_rows_and_a_leading_byte_order_mark_are_skipped_and_empty_people_counted() {
        let text = "\u{feff}Name,Phone\n\n,\nAnna As,555\n,   \nBo,\n";
        let got = import_csv(text).unwrap();
        assert_eq!(got.import.contacts.len(), 2);
        assert_eq!(got.import.invalid, 0);
        let got = import_csv("Name,Phone\n   ,555 0100\n").unwrap();
        assert_eq!(got.import.contacts.len(), 1, "a phone number alone is enough for a contact");
        let got = import_csv("Name,Fax\nAnna,1\n").unwrap();
        assert_eq!(got.import.contacts[0].name, "Anna");
    }

    #[test]
    fn a_table_without_known_columns_or_text_is_refused() {
        assert!(import_csv("Colour,Shape\nred,round\n").is_err());
        assert!(import_csv("").is_err());
        assert!(import_csv("\n\n").is_err());
    }

    #[test]
    fn card_fields_come_from_their_columns() {
        let got = import_csv("Name,Job Title,Department,Nickname,Birthday,Website\nAnna As,Engineer,Sales,Annie,19900517,https://a.example\n").unwrap();
        let card = &got.import.details[0];
        assert_eq!(card.get(Detail::JobTitle), "Engineer");
        assert_eq!(card.get(Detail::Department), "Sales");
        assert_eq!(card.get(Detail::Nickname), "Annie");
        assert_eq!(card.get(Detail::Birthday), "1990-05-17");
        assert_eq!(card.get(Detail::Url), "https://a.example");
    }

    #[test]
    fn extra_values_beyond_the_limit_are_counted() {
        let mut header = String::from("Name");
        let mut row = String::from("Anna");
        for i in 0..9 {
            header.push_str(&format!(",Phone {i}"));
            row.push_str(&format!(",555 010{i}"));
        }
        let got = import_csv(&format!("{header}\n{row}\n")).unwrap();
        assert_eq!(got.import.values[0].len(), MAX_EXTRAS);
        assert_eq!(got.import.extra_values, 2, "nine phones: one in the store, six kept, two dropped");
    }

    #[test]
    fn base64_text_decodes_with_or_without_padding() {
        assert_eq!(base64_decode("SGVsbG8="), Some(b"Hello".to_vec()));
        assert_eq!(base64_decode("SGVsbG8"), Some(b"Hello".to_vec()));
        assert_eq!(base64_decode("w7NsYWZ1cg=="), Some("\u{f3}lafur".as_bytes().to_vec()));
        assert_eq!(base64_decode("SGV s\nbG8="), Some(b"Hello".to_vec()));
        assert_eq!(base64_decode("***"), None);
    }

    #[test]
    fn an_ldif_record_maps_its_attributes() {
        let text = "version: 1\n\n\
dn: cn=Anna Lindqvist,dc=example\n\
objectClass: inetOrgPerson\n\
cn: Anna Lindqvist\n\
givenName: Anna\n\
sn: Lindqvist\n\
o: Acme\n\
ou: Sales\n\
title: Engineer\n\
mail: anna@acme.is\n\
mail: anna@home.is\n\
telephoneNumber: 555 0200\n\
mobile: 555 0100\n\
postalAddress: Gata 1$Reykjavik\n\
description: Met at\n  a fair\n\
jpegPhoto:: AAAA\n";
        let got = import_ldif(text).unwrap();
        assert_eq!(got.import.contacts.len(), 1);
        let c = &got.import.contacts[0];
        assert_eq!((c.name.as_str(), c.company.as_str()), ("Anna Lindqvist", "Acme"));
        assert_eq!((c.phone.as_str(), c.email.as_str()), ("555 0200", "anna@acme.is"));
        assert_eq!(c.labels[0], 2, "telephoneNumber is a work phone");
        assert_eq!(c.address, "Gata 1, Reykjavik");
        assert_eq!(c.notes, "Met at a fair", "the continuation line joins without its leading space");
        assert_eq!(got.import.values[0].len(), 2, "the second email and the mobile phone");
        assert_eq!(got.import.details[0].get(Detail::JobTitle), "Engineer");
        assert_eq!(got.import.details[0].get(Detail::Department), "Sales");
        assert_eq!(got.ignored, vec!["jpegphoto".to_string()]);
    }

    #[test]
    fn encoded_values_comments_and_groups_are_handled() {
        let text = "# exported\ndn: cn=x\nobjectClass: person\ncn:: w7NsYWZ1ciDDk2xhZnNzb24=\nmail: o@x.is\n\n\
dn: cn=Friends\nobjectClass: groupOfNames\ncn: Friends\n\n\
dn: cn=Nothing\nobjectClass: person\nuid: nothing\n";
        let got = import_ldif(text).unwrap();
        assert_eq!(got.import.contacts.len(), 1);
        assert_eq!(got.import.contacts[0].name, "\u{f3}lafur \u{d3}lafsson");
        assert_eq!(got.import.groups, 1);
        assert_eq!(got.import.invalid, 1, "a record with no name, company, phone or email");
    }

    #[test]
    fn readers_follow_the_file_extension() {
        assert!(reader_for("CSV").is_some());
        assert!(reader_for("ldif").is_some());
        assert!(reader_for("vcf").is_none());
    }
}
