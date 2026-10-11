//! Bounded local contacts with an escaped, atomically replaced text store.
use super::prefs::{escape_field as escape, unescape_field as unescape};
use std::collections::HashSet;
use std::io;
use std::path::Path;

const HEADER: &str = "springboard-contacts-v1";
pub const MAX_NAME: usize = 160;
pub const MAX_VALUE: usize = 320;
pub const MAX_NOTES: usize = 4000;
const MAX_ITEMS: usize = 10_000;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub id: u64,
    pub name: String,
    pub phone: String,
    pub email: String,
    pub notes: String,
    pub favorite: bool,
    pub company: String,
    pub address: String,
    /// Label index of the phone, email and address rows (into `LABELS`).
    pub labels: [u8; 3],
}

/// Row labels offered by the edit sheet: phone, email, address.
pub const LABELS: [&[&str]; 3] = [
    &["mobile", "home", "work", "other"],
    &["home", "work", "other"],
    &["home", "work", "other"],
];

impl Contact {
    pub fn display_name(&self) -> &str {
        [&self.name, &self.company, &self.phone, &self.email]
            .into_iter()
            .find(|value| !value.trim().is_empty())
            .map(String::as_str)
            .unwrap_or("No Name")
    }
    pub fn initials(&self) -> String {
        let source = if self.name.trim().is_empty() { &self.company } else { &self.name };
        let words: Vec<_> = source.split_whitespace().collect();
        let letters: String = match words.as_slice() {
            [] => "?".into(),
            [one] => one.chars().take(1).collect(),
            _ => [words[0], words[words.len() - 1]]
                .into_iter()
                .filter_map(|word| word.chars().next())
                .collect(),
        };
        letters.to_uppercase().chars().take(2).collect()
    }
    pub fn valid(&self) -> bool {
        (!self.name.trim().is_empty()
            || !self.company.trim().is_empty()
            || !self.address.trim().is_empty()
            || !self.phone.trim().is_empty()
            || !self.email.trim().is_empty())
            && self.labels.iter().zip(LABELS).all(|(&l, set)| (l as usize) < set.len())
            && [&self.name, &self.phone, &self.email, &self.notes, &self.company, &self.address]
                .into_iter()
                .zip([MAX_NAME, MAX_VALUE, MAX_VALUE, MAX_NOTES, MAX_NAME, MAX_VALUE])
                .all(|(text, max)| {
                    text.chars().count() <= max
                        && text
                            .chars()
                            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
                })
    }
}

pub fn parse(text: &str) -> Vec<Contact> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return vec![];
    }
    let mut out = vec![];
    let mut ids = HashSet::new();
    for line in lines.take(MAX_ITEMS) {
        let f: Vec<_> = line.split('\t').collect();
        if !(5..=11).contains(&f.len()) {
            continue;
        }
        let (Ok(id), Some(name), Some(phone), Some(email), Some(notes)) = (
            f[0].parse(),
            unescape(f[1], MAX_NAME),
            unescape(f[2], MAX_VALUE),
            unescape(f[3], MAX_VALUE),
            unescape(f[4], MAX_NOTES),
        ) else {
            continue;
        };
        let contact = Contact {
            id,
            name,
            phone,
            email,
            notes,
            favorite: f.get(5) == Some(&"1"),
            company: f.get(6).and_then(|v| unescape(v, MAX_NAME)).unwrap_or_default(),
            address: f.get(7).and_then(|v| unescape(v, MAX_VALUE)).unwrap_or_default(),
            labels: [8, 9, 10].map(|i| {
                f.get(i).and_then(|v| v.parse().ok()).unwrap_or(0)
            }),
        };
        if contact.valid() && ids.insert(id) {
            out.push(contact);
        }
    }
    sort(&mut out);
    out
}

pub fn sort(contacts: &mut [Contact]) {
    contacts.sort_by(|a, b| {
        a.display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase())
            .then(a.id.cmp(&b.id))
    });
}

pub fn search(contacts: &[Contact], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    contacts
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            [&c.name, &c.phone, &c.email, &c.notes, &c.company, &c.address]
                .into_iter()
                .any(|value| value.to_lowercase().contains(&query))
                .then_some(i)
        })
        .collect()
}

pub fn new_id(contacts: &[Contact]) -> Option<u64> {
    if contacts.len() >= MAX_ITEMS {
        return None;
    }
    let ids: HashSet<_> = contacts.iter().map(|c| c.id).collect();
    (1..=MAX_ITEMS as u64 + 1).find(|id| !ids.contains(id))
}

pub fn load_checked(path: &Path) -> io::Result<Vec<Contact>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
        Ok(meta) if meta.len() > MAX_BYTES as u64 => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Contacts file too large",
            ))
        }
        _ => {}
    }
    let text = std::fs::read_to_string(path)?;
    if text.lines().next() != Some(HEADER) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unknown contacts format",
        ));
    }
    Ok(parse(&text))
}

pub fn load(path: &Path) -> Vec<Contact> {
    load_checked(path).unwrap_or_default()
}

pub fn save(path: &Path, contacts: &[Contact]) -> io::Result<()> {
    let mut ids = HashSet::new();
    if contacts.len() > MAX_ITEMS || contacts.iter().any(|c| !c.valid() || !ids.insert(c.id)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid contact",
        ));
    }
    let mut text = format!("{HEADER}\n");
    for c in contacts {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.id,
            escape(&c.name),
            escape(&c.phone),
            escape(&c.email),
            escape(&c.notes),
            if c.favorite { "1" } else { "0" },
            escape(&c.company),
            escape(&c.address),
            c.labels[0],
            c.labels[1],
            c.labels[2]
        ));
        if text.len() > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Contacts file too large",
            ));
        }
    }
    super::fsutil::write_atomic(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_unicode_values_round_trip_and_bad_rows_are_skipped() {
        let c = Contact {
            id: 1,
            name: "Ása Jóns".into(),
            phone: "+354 555 1234".into(),
            email: "asa@example.com".into(),
            notes: "Line 1\n世界\t\\".into(),
            ..Default::default()
        };
        let line = format!(
            "{HEADER}\n1\t{}\t{}\t{}\t{}\n",
            escape(&c.name),
            escape(&c.phone),
            escape(&c.email),
            escape(&c.notes)
        );
        let text = format!("{line}1\tDuplicate\t\t\t\n2\tBad\\q\t\t\t\n3\t\t\t\t\n4\tGood\t\t\t\n");
        let contacts = parse(&text);
        assert_eq!(contacts.len(), 2);
        assert!(contacts.contains(&c));
        assert!(parse("unknown\n").is_empty());
    }
    #[test]
    fn alphabetical_search_and_name_fallback() {
        let mut c = vec![
            Contact {
                id: 1,
                name: "Zoe".into(),
                notes: "Dentist".into(),
                ..Default::default()
            },
            Contact {
                id: 2,
                name: "Amy Smith".into(),
                email: "AMY@example.com".into(),
                ..Default::default()
            },
        ];
        sort(&mut c);
        assert_eq!(c[0].initials(), "AS");
        assert_eq!(search(&c, "amy@"), vec![0]);
        assert_eq!(search(&c, "DENTIST"), vec![1]);
        let phone = Contact {
            phone: "+354 555".into(),
            ..Default::default()
        };
        assert_eq!(phone.display_name(), "+354 555");
        assert!(phone.valid());
        assert_eq!(new_id(&c), Some(3));
    }
    #[test]
    fn invalid_save_preserves_the_existing_file() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/test-contacts-store-{}/contacts.tsv",
            std::process::id()
        ));
        let good = Contact {
            id: 1,
            name: "Anna".into(),
            ..Default::default()
        };
        save(&path, &[good.clone()]).unwrap();
        let bad = Contact {
            name: "x".repeat(MAX_NAME + 1),
            ..good.clone()
        };
        assert!(save(&path, &[bad]).is_err());
        assert_eq!(load_checked(&path).unwrap(), vec![good.clone()]);
        assert!(save(&path, &[good.clone(), good]).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

#[cfg(test)]
mod format_compat_tests {
    use super::*;
    #[test]
    fn old_five_and_six_column_rows_load_and_new_fields_round_trip() {
        let text = format!("{HEADER}\n1\tAnna\t1\t\t\n2\tBob\t2\t\t\t1\n");
        let c = parse(&text);
        assert_eq!(c.len(), 2);
        assert!(c[1].favorite && c[0].company.is_empty());
        let full = Contact { id: 3, name: "Cy".into(), company: "Co".into(), address: "St 1".into(), labels: [2, 1, 2], favorite: true, ..Default::default() };
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-contacts-compat-{}/c.tsv", std::process::id()));
        save(&path, &[full.clone()]).unwrap();
        assert_eq!(load_checked(&path).unwrap(), vec![full]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
