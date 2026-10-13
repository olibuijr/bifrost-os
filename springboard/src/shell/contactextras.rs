//! The multi-value fields a contact holds beyond the store's one phone, one email and one address: more phones, more
//! emails, URLs, a birthday and labeled dates. They live in a sidecar file next to the contacts file, so the
//! contacts file keeps its format and an older shell ignores the sidecar. A missing sidecar is an empty one.
use super::prefs::{escape_field as escape, unescape_field as unescape};
use bifrost_ui::contact_fetch::{DateComponents, LabeledValue};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

const HEADER: &str = "springboard-contact-extras-v1";
const MAX_FIELD: usize = 320;

/// The extra fields of one contact.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub phones: Vec<LabeledValue>,
    pub emails: Vec<LabeledValue>,
    pub urls: Vec<LabeledValue>,
    pub birthday: Option<DateComponents>,
    pub dates: Vec<(String, DateComponents)>,
}

/// The extras of every contact that has any, by contact id.
pub type Book = BTreeMap<u64, Extras>;

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contacts-extra.tsv")
}

/// Rows are `id TAB kind TAB label TAB value`; kind is `phone`, `email`, `url`, `birthday` or `date`. Bad rows are
/// skipped.
pub fn parse(text: &str) -> Book {
    let mut book = Book::new();
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return book;
    }
    for line in lines {
        let f: Vec<_> = line.split('\t').collect();
        let [id, kind, label, value] = f.as_slice() else { continue };
        let (Ok(id), Some(label), Some(value)) = (id.parse::<u64>(), unescape(label, MAX_FIELD), unescape(value, MAX_FIELD)) else {
            continue;
        };
        let entry = book.entry(id).or_default();
        match *kind {
            "phone" => entry.phones.push(LabeledValue::new(&label, &value)),
            "email" => entry.emails.push(LabeledValue::new(&label, &value)),
            "url" => entry.urls.push(LabeledValue::new(&label, &value)),
            "birthday" => entry.birthday = DateComponents::from_text(&value),
            "date" => {
                if let Some(d) = DateComponents::from_text(&value) {
                    entry.dates.push((label, d));
                }
            }
            _ => {}
        }
    }
    book
}

pub fn render(book: &Book) -> String {
    let mut text = format!("{HEADER}\n");
    let mut row = |id: u64, kind: &str, label: &str, value: &str| {
        text.push_str(&format!("{id}\t{kind}\t{}\t{}\n", escape(label), escape(value)));
    };
    for (id, e) in book {
        for p in &e.phones {
            row(*id, "phone", &p.label, &p.value);
        }
        for m in &e.emails {
            row(*id, "email", &m.label, &m.value);
        }
        for u in &e.urls {
            row(*id, "url", &u.label, &u.value);
        }
        if let Some(b) = e.birthday {
            row(*id, "birthday", "", &b.to_text());
        }
        for (label, d) in &e.dates {
            row(*id, "date", label, &d.to_text());
        }
    }
    text
}

/// The sidecar at `path`; empty when it is missing or unreadable.
pub fn load(path: &Path) -> Book {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, book: &Book) -> io::Result<()> {
    let tmp = path.with_extension("tsv.tmp");
    std::fs::write(&tmp, render(book))?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extras_round_trip_through_the_sidecar_text() {
        let mut book = Book::new();
        book.insert(
            4,
            Extras {
                phones: vec![LabeledValue::new("work", "+354 555 0101")],
                emails: vec![LabeledValue::new("home", "a@x.is"), LabeledValue::new("work", "b@x.is")],
                urls: vec![LabeledValue::new("other", "https://x.is/a\tb")],
                birthday: DateComponents::new(None, 12, 10),
                dates: vec![("anniversary".into(), DateComponents::new(Some(2001), 5, 6).unwrap())],
            },
        );
        assert_eq!(parse(&render(&book)), book);
    }

    #[test]
    fn a_missing_header_or_bad_rows_give_nothing() {
        assert!(parse("junk\n1\temail\thome\ta@x.is\n").is_empty());
        let book = parse(&format!("{HEADER}\nx\temail\thome\ta\n2\tbirthday\t\t--02-30x\n3\tphone\tmobile\t1\n"));
        assert_eq!(book.len(), 2);
        assert_eq!(book[&3].phones.len(), 1);
        assert_eq!(book[&2].birthday, None);
    }

    #[test]
    fn the_sidecar_sits_next_to_the_contacts_file() {
        assert_eq!(path_for(Path::new("/d/Contacts/contacts.tsv")), Path::new("/d/Contacts/contacts-extra.tsv"));
    }
}
