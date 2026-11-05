//! Extra card fields for contacts: nickname, job title, department, URL, birthday, pronouns, phonetic name, a related
//! name and a profile, plus the company flag. The contact store has fixed fields, so these live beside it in
//! `details.txt` in the Contacts folder, keyed by the contact's identifier. A missing file means no details.
//!
//! ```text
//! springboard-contact-details-v1
//! <id><TAB><code><TAB><value>
//! ```
//!
//! `code` is a field's code (`nickname`, `job`, ...), `company`, whose value is `1`, or `family`, the family name the
//! card was saved with (the store keeps one name, so a family name of several words is only known from this). Text is escaped the way
//! `values.txt` escapes it. Pure std.

use crate::extras::{escape, unescape};
use springboard::shell::contactstore::MAX_VALUE;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const HEADER: &str = "springboard-contact-details-v1";
const MAX_BYTES: usize = 1024 * 1024;
const COMPANY_CODE: &str = "company";
const FAMILY_CODE: &str = "family";

/// The fields a card can add through Card > Add Field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    Nickname,
    JobTitle,
    Department,
    Url,
    Birthday,
    Pronouns,
    Phonetic,
    Related,
    Profile,
}

impl Detail {
    /// Every field, in the order the card and the Add Field menu list them.
    pub const ALL: [Detail; 9] = [
        Detail::Nickname,
        Detail::JobTitle,
        Detail::Department,
        Detail::Url,
        Detail::Birthday,
        Detail::Pronouns,
        Detail::Phonetic,
        Detail::Related,
        Detail::Profile,
    ];

    /// The title the card, the form and the menu show.
    pub fn title(self) -> &'static str {
        match self {
            Detail::Nickname => "Nickname",
            Detail::JobTitle => "Job Title",
            Detail::Department => "Department",
            Detail::Url => "URL",
            Detail::Birthday => "Birthday",
            Detail::Pronouns => "Pronouns",
            Detail::Phonetic => "Phonetic Name",
            Detail::Related => "Related Name",
            Detail::Profile => "Profile",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Detail::Nickname => "nickname",
            Detail::JobTitle => "job",
            Detail::Department => "department",
            Detail::Url => "url",
            Detail::Birthday => "birthday",
            Detail::Pronouns => "pronouns",
            Detail::Phonetic => "phonetic",
            Detail::Related => "related",
            Detail::Profile => "profile",
        }
    }

    pub fn from_code(code: &str) -> Option<Detail> {
        Detail::ALL.into_iter().find(|d| d.code() == code)
    }

    /// The position in `ALL`.
    pub fn index(self) -> usize {
        Detail::ALL.iter().position(|d| *d == self).unwrap_or(0)
    }
}

/// The details of one contact: the company flag and the fields it filled in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardDetails {
    /// Card > Mark as a Company: the card shows its company as the name.
    pub company: bool,
    /// The family name the card was saved with, empty when the name's last word is it. See `names::split_with`.
    pub family: String,
    values: BTreeMap<Detail, String>,
}

impl CardDetails {
    /// The value of a field, empty when the card has none.
    pub fn get(&self, detail: Detail) -> &str {
        self.values.get(&detail).map(String::as_str).unwrap_or("")
    }

    /// Sets a field. The value is trimmed; an empty one clears the field. Returns false, and changes nothing, for a
    /// value that is too long or holds a control character.
    pub fn set(&mut self, detail: Detail, value: &str) -> bool {
        let value = value.trim();
        if value.chars().count() > MAX_VALUE || value.chars().any(char::is_control) {
            return false;
        }
        if value.is_empty() {
            self.values.remove(&detail);
        } else {
            self.values.insert(detail, value.to_string());
        }
        true
    }

    /// True when the card has no field and is not marked as a company.
    pub fn is_empty(&self) -> bool {
        !self.company && self.family.is_empty() && self.values.is_empty()
    }

    /// The fields the card filled in, in `Detail::ALL` order.
    pub fn filled(&self) -> Vec<(Detail, &str)> {
        Detail::ALL.into_iter().filter_map(|d| self.values.get(&d).map(|v| (d, v.as_str()))).collect()
    }
}

/// The details of every contact, by contact identifier. Contacts with none have no entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details(BTreeMap<u64, CardDetails>);

impl Details {
    /// The details of a contact (empty when it has none).
    pub fn of(&self, id: u64) -> CardDetails {
        self.0.get(&id).cloned().unwrap_or_default()
    }

    /// The nickname of a contact, empty when it has none.
    pub fn nickname(&self, id: u64) -> &str {
        self.0.get(&id).map_or("", |d| d.get(Detail::Nickname))
    }

    /// The family name a contact was saved with, empty when it has none recorded.
    pub fn family(&self, id: u64) -> &str {
        self.0.get(&id).map_or("", |d| d.family.as_str())
    }

    /// Whether the contact is marked as a company.
    pub fn is_company(&self, id: u64) -> bool {
        self.0.get(&id).is_some_and(|d| d.company)
    }

    /// Replaces the details of a contact. Empty details remove the entry.
    pub fn set(&mut self, id: u64, details: CardDetails) {
        if details.is_empty() {
            self.0.remove(&id);
        } else {
            self.0.insert(id, details);
        }
    }

    /// Removes a contact's details. Returns true when there were any.
    pub fn remove(&mut self, id: u64) -> bool {
        self.0.remove(&id).is_some()
    }

    /// Keeps the details of the contacts `keep` accepts.
    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.0.retain(|&id, _| keep(id));
    }

    /// True when one of the contact's fields contains `query`, ignoring case. A blank query matches any contact with a
    /// field.
    pub fn matches(&self, id: u64, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        self.0.get(&id).is_some_and(|d| d.values.values().any(|v| v.to_lowercase().contains(&query)))
    }

    /// The file text: the header, then one line per field and per company flag, contacts in identifier order.
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER}\n");
        for (id, details) in &self.0 {
            if details.company {
                out.push_str(&format!("{id}\t{COMPANY_CODE}\t1\n"));
            }
            if !details.family.is_empty() {
                out.push_str(&format!("{id}\t{FAMILY_CODE}\t{}\n", escape(&details.family)));
            }
            for (detail, value) in details.filled() {
                out.push_str(&format!("{id}\t{}\t{}\n", detail.code(), escape(value)));
            }
        }
        out
    }
}

/// Reads the file text. Returns `None` when the header is not this format's. Lines that do not read are skipped.
pub fn parse(text: &str) -> Option<Details> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return None;
    }
    let mut out = Details::default();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let &[id, code, value] = fields.as_slice() else { continue };
        let Ok(id) = id.parse::<u64>() else { continue };
        let mut card = out.of(id);
        if code == COMPANY_CODE {
            card.company = value == "1";
        } else if code == FAMILY_CODE {
            card.family = unescape(value).trim().to_string();
        } else if let Some(detail) = Detail::from_code(code) {
            card.set(detail, &unescape(value));
        } else {
            continue;
        }
        out.set(id, card);
    }
    Some(out)
}

/// Reads `details.txt`. A missing file is no details. A file that cannot be read, is too large or has another header is
/// an error, so the app does not overwrite it.
pub fn load_checked(path: &Path) -> io::Result<Details> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Details::default()),
        Err(e) => return Err(e),
        Ok(meta) if meta.len() > MAX_BYTES as u64 => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Contact details file too large"));
        }
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(path)?;
    parse(&text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unknown contact details format"))
}

/// Writes the file atomically: a temporary file is written beside it and then renamed over it.
pub fn save(path: &Path, details: &Details) -> io::Result<()> {
    let text = details.to_text();
    if text.len() > MAX_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Too many contact details"));
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// A birthday as `YYYY-MM-DD`. Accepts `19900517`, `1990-05-17` and the year-less `--0517` and `--05-17`; anything else
/// is kept as it was typed.
pub fn normalize_birthday(text: &str) -> String {
    let text = text.trim();
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let plain = text.chars().all(|c| c.is_ascii_digit() || c == '-');
    if !plain {
        return text.to_string();
    }
    if text.starts_with("--") && digits.len() == 4 {
        return format!("--{}-{}", &digits[..2], &digits[2..]);
    }
    if digits.len() == 8 && (text.len() == 8 || text.len() == 10) {
        return format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..]);
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anna() -> CardDetails {
        let mut card = CardDetails::default();
        assert!(card.set(Detail::Nickname, "Annie"));
        assert!(card.set(Detail::Birthday, "1990-05-17"));
        card
    }

    #[test]
    fn a_field_is_trimmed_and_an_empty_value_clears_it() {
        let mut card = CardDetails::default();
        assert!(card.set(Detail::JobTitle, "  Engineer "));
        assert_eq!(card.get(Detail::JobTitle), "Engineer");
        assert!(card.set(Detail::JobTitle, "   "));
        assert_eq!(card.get(Detail::JobTitle), "");
        assert!(card.is_empty());
    }

    #[test]
    fn long_values_and_control_characters_are_refused() {
        let mut card = CardDetails::default();
        assert!(!card.set(Detail::Url, &"x".repeat(MAX_VALUE + 1)));
        assert!(!card.set(Detail::Url, "a\tb"));
        assert!(card.is_empty(), "a refused value changes nothing");
    }

    #[test]
    fn filled_fields_come_back_in_menu_order() {
        let mut card = CardDetails::default();
        card.set(Detail::Profile, "@anna");
        card.set(Detail::Nickname, "Annie");
        let titles: Vec<&str> = card.filled().into_iter().map(|(d, _)| d.title()).collect();
        assert_eq!(titles, ["Nickname", "Profile"]);
    }

    #[test]
    fn the_file_text_round_trips_with_the_company_flag_and_escapes() {
        let mut details = Details::default();
        let mut company = anna();
        company.company = true;
        company.set(Detail::Related, "Bo\\Berg");
        details.set(7, company);
        details.set(9, anna());
        let text = details.to_text();
        assert!(text.contains("7\tcompany\t1\n"), "{text}");
        assert!(text.contains("7\trelated\tBo\\\\Berg\n"), "{text}");
        assert_eq!(parse(&text), Some(details));
    }

    #[test]
    fn the_family_name_round_trips_and_counts_as_a_detail() {
        let mut details = Details::default();
        let mut card = CardDetails::default();
        card.family = "Test person".into();
        assert!(!card.is_empty(), "a card with only a family name keeps its entry");
        details.set(4, card);
        let text = details.to_text();
        assert!(text.contains("4\tfamily\tTest person\n"), "{text}");
        let read = parse(&text).unwrap();
        assert_eq!(read.family(4), "Test person");
        assert_eq!(read.family(5), "");
    }

    #[test]
    fn a_foreign_header_and_unreadable_lines_are_not_details() {
        assert_eq!(parse("springboard-contact-values-v1\n"), None);
        let read = parse("springboard-contact-details-v1\nx\tjob\tA\n1\tnope\tA\n2\tjob\n3\tjob\tB\n").unwrap();
        assert_eq!(read.of(3).get(Detail::JobTitle), "B");
        assert!(read.of(1).is_empty() && read.of(2).is_empty());
    }

    #[test]
    fn empty_details_leave_no_entry_and_removing_reports_it() {
        let mut details = Details::default();
        details.set(1, anna());
        details.set(1, CardDetails::default());
        assert_eq!(details, Details::default());
        details.set(2, anna());
        assert!(details.remove(2));
        assert!(!details.remove(2));
    }

    #[test]
    fn search_reads_the_filled_fields_only() {
        let mut details = Details::default();
        details.set(1, anna());
        assert!(details.matches(1, "ANNIE"));
        assert!(details.matches(1, "1990"));
        assert!(!details.matches(1, "zzz"));
        assert!(!details.matches(2, "annie"));
    }

    #[test]
    fn the_nickname_and_company_flag_are_read_without_copying() {
        let mut details = Details::default();
        let mut card = anna();
        card.company = true;
        details.set(1, card);
        assert_eq!(details.nickname(1), "Annie");
        assert!(details.is_company(1));
        assert_eq!(details.nickname(2), "");
        assert!(!details.is_company(2));
    }

    #[test]
    fn retain_drops_the_details_of_deleted_contacts() {
        let mut details = Details::default();
        details.set(1, anna());
        details.set(2, anna());
        details.retain(|id| id == 2);
        assert!(details.of(1).is_empty());
        assert!(!details.of(2).is_empty());
    }

    #[test]
    fn a_missing_file_is_no_details_and_a_saved_one_loads_back() {
        let dir = std::env::temp_dir().join(format!("details-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("details.txt");
        assert_eq!(load_checked(&path).unwrap(), Details::default());
        let mut details = Details::default();
        details.set(4, anna());
        save(&path, &details).unwrap();
        assert_eq!(load_checked(&path).unwrap(), details);
        std::fs::write(&path, "nope\n").unwrap();
        assert!(load_checked(&path).is_err(), "a foreign file is not overwritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn birthdays_are_written_as_year_month_day() {
        assert_eq!(normalize_birthday("19900517"), "1990-05-17");
        assert_eq!(normalize_birthday("1990-05-17"), "1990-05-17");
        assert_eq!(normalize_birthday("--0517"), "--05-17");
        assert_eq!(normalize_birthday("--05-17"), "--05-17");
        assert_eq!(normalize_birthday("May 17"), "May 17");
        assert_eq!(normalize_birthday("  "), "");
    }

    #[test]
    fn every_field_has_a_unique_code_that_reads_back() {
        for (i, detail) in Detail::ALL.into_iter().enumerate() {
            assert_eq!(detail.index(), i);
            assert_eq!(Detail::from_code(detail.code()), Some(detail));
            assert!(!detail.title().is_empty());
        }
    }
}
