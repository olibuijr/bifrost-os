//! The fields a contact holds beyond the store's one name, one phone, one email and one address: more phones, more
//! emails, URLs, a birthday and labeled dates, the name prefix and nickname, the job title and department, related
//! people, social profiles and instant message addresses. They live in a sidecar file next to the contacts file, so the
//! contacts file keeps its format and an older shell ignores the sidecar. A missing sidecar is an empty one.
use super::prefs::{escape_field as escape, unescape_field as unescape};
use bifrost_ui::contact_fetch::{DateComponents, LabeledValue, Profile};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

const HEADER: &str = "springboard-contact-extras-v1";
/// Longest text one sidecar label or value may hold; a longer one is refused on save and dropped on read.
pub const MAX_FIELD: usize = 320;

/// The extra fields of one contact.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub phones: Vec<LabeledValue>,
    pub emails: Vec<LabeledValue>,
    pub urls: Vec<LabeledValue>,
    pub birthday: Option<DateComponents>,
    pub dates: Vec<(String, DateComponents)>,
    /// `Dr.`, `Prof.`: the title before the name.
    pub name_prefix: String,
    pub nickname: String,
    pub job_title: String,
    pub department: String,
    /// Related people: the label is the relation (`spouse`, `manager`), the value the person's name.
    pub relations: Vec<LabeledValue>,
    pub social_profiles: Vec<Profile>,
    /// Instant message addresses: a service and a user name; `url` and `user_identifier` stay empty.
    pub instant_messages: Vec<Profile>,
}

impl Extras {
    /// Whether the contact has no extra field at all, so the sidecar need not keep a row for it.
    pub fn is_empty(&self) -> bool {
        *self == Extras::default()
    }
}

/// The extras of every contact that has any, by contact id.
pub type Book = BTreeMap<u64, Extras>;

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contacts-extra.tsv")
}

/// Rows are `id TAB kind TAB label TAB value`; kind is `phone`, `email`, `url`, `birthday`, `date`, `prefix`,
/// `nickname`, `job_title`, `department` or `relation`. A `social` row is `id TAB social TAB label TAB service TAB
/// username TAB url TAB user_identifier` and an `im` row is `id TAB im TAB label TAB service TAB username`. Bad rows and
/// unknown kinds are skipped, so an older shell reads the rows it knows.
pub fn parse(text: &str) -> Book {
    let mut book = Book::new();
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return book;
    }
    for line in lines {
        let f: Vec<_> = line.split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        let (id, kind) = (f[0], f[1]);
        let Ok(id) = id.parse::<u64>() else { continue };
        let Some(rest) = f[2..].iter().map(|v| unescape(v, MAX_FIELD)).collect::<Option<Vec<String>>>() else { continue };
        let profile = |rest: &[String]| Profile {
            label: rest[0].clone(),
            service: rest[1].clone(),
            username: rest[2].clone(),
            url: rest.get(3).cloned().unwrap_or_default(),
            user_identifier: rest.get(4).cloned().unwrap_or_default(),
            identifier: String::new(),
        };
        match (kind, rest.len()) {
            ("social", 5) => {
                book.entry(id).or_default().social_profiles.push(profile(&rest));
                continue;
            }
            ("im", 3) => {
                book.entry(id).or_default().instant_messages.push(profile(&rest));
                continue;
            }
            (_, 2) => {}
            _ => continue,
        }
        let (label, value) = (rest[0].clone(), rest[1].clone());
        let entry = book.entry(id).or_default();
        match kind {
            "phone" => entry.phones.push(LabeledValue::new(&label, &value)),
            "email" => entry.emails.push(LabeledValue::new(&label, &value)),
            "url" => entry.urls.push(LabeledValue::new(&label, &value)),
            "birthday" => entry.birthday = DateComponents::from_text(&value),
            "date" => {
                if let Some(d) = DateComponents::from_text(&value) {
                    entry.dates.push((label, d));
                }
            }
            "prefix" => entry.name_prefix = value,
            "nickname" => entry.nickname = value,
            "job_title" => entry.job_title = value,
            "department" => entry.department = value,
            "relation" => entry.relations.push(LabeledValue::new(&label, &value)),
            _ => {}
        }
    }
    book
}

pub fn render(book: &Book) -> String {
    fn push(text: &mut String, id: u64, kind: &str, fields: &[&str]) {
        let fields: Vec<String> = fields.iter().map(|f| escape(f)).collect();
        text.push_str(&format!("{id}\t{kind}\t{}\n", fields.join("\t")));
    }
    let mut text = format!("{HEADER}\n");
    for (id, e) in book {
        for p in &e.phones {
            push(&mut text, *id, "phone", &[&p.label, &p.value]);
        }
        for m in &e.emails {
            push(&mut text, *id, "email", &[&m.label, &m.value]);
        }
        for u in &e.urls {
            push(&mut text, *id, "url", &[&u.label, &u.value]);
        }
        if let Some(b) = e.birthday {
            push(&mut text, *id, "birthday", &["", &b.to_text()]);
        }
        for (label, d) in &e.dates {
            push(&mut text, *id, "date", &[label, &d.to_text()]);
        }
        for (kind, value) in [("prefix", &e.name_prefix), ("nickname", &e.nickname), ("job_title", &e.job_title), ("department", &e.department)] {
            if !value.trim().is_empty() {
                push(&mut text, *id, kind, &["", value]);
            }
        }
        for r in &e.relations {
            push(&mut text, *id, "relation", &[&r.label, &r.value]);
        }
        for p in &e.social_profiles {
            push(&mut text, *id, "social", &[&p.label, &p.service, &p.username, &p.url, &p.user_identifier]);
        }
        for p in &e.instant_messages {
            push(&mut text, *id, "im", &[&p.label, &p.service, &p.username]);
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
                ..Default::default()
            },
        );
        assert_eq!(parse(&render(&book)), book);
    }

    #[test]
    fn names_work_relations_and_profiles_round_trip_through_the_sidecar_text() {
        let mut social = Profile::new("", "mastodon", "@ada");
        social.url = "https://example.social/@ada".into();
        social.user_identifier = "4711".into();
        let mut book = Book::new();
        book.insert(
            9,
            Extras {
                name_prefix: "Dr.".into(),
                nickname: "Countess\tof numbers".into(),
                job_title: "Analyst".into(),
                department: "Engines".into(),
                relations: vec![LabeledValue::new("spouse", "William King"), LabeledValue::new("child", "Byron")],
                social_profiles: vec![social],
                instant_messages: vec![Profile::new("home", "xmpp", "ada@chat.example")],
                ..Default::default()
            },
        );
        let text = render(&book);
        assert!(text.contains("\tsocial\t\tmastodon\t@ada\t"));
        assert_eq!(parse(&text), book);
    }

    #[test]
    fn a_short_social_row_or_an_unknown_kind_is_skipped_and_an_empty_contact_keeps_no_entry() {
        let text = format!("{HEADER}\n1\tsocial\t\tmastodon\t@a\n2\tshoe\t\t42\n3\tnickname\t\tBo\n");
        let book = parse(&text);
        assert!(!book.contains_key(&1), "a social row needs all five fields");
        assert!(book.get(&2).map_or(true, Extras::is_empty));
        assert_eq!(book[&3].nickname, "Bo");
        assert!(Extras::default().is_empty());
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
