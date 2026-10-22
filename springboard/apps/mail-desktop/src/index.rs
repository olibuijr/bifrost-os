//! Core Search index of the mail: the newest messages of the mailbox on screen (up to `MAX_ROWS`), each with the
//! searchable schema in [`SCHEMA`]: subject, people (sender, recipients), date, state keywords (unread, flagged),
//! list and mailbox. Bodies are never indexed. Mail Settings "Show in Search" turns it off, and then the app removes what it
//! indexed. An item's id is `account/mailbox/uid`, so the same UID in two mailboxes never collides.
use crate::flags::Flag;
use crate::message::{address_of, split_addresses};
use crate::state::Row;
use bifrost_app::quicksearch::{CSCustomAttributeKey, CSSearchableItem, CSSearchableItemAttributeSet, Person};
use std::collections::HashMap;

/// Search domain of every message.
pub const SPOT_DOMAIN: &str = "mail";
/// The newest messages of a mailbox that are indexed.
pub const MAX_ROWS: usize = 200;
/// Title of a message without a subject.
const NO_SUBJECT: &str = "(no subject)";

/// The type identifier every message is indexed with.
pub const CONTENT_TYPE: &str = "public.email-message";

/// One field of the mail search schema: its name, the attribute of the searchable item it is stored in, and whether a
/// query matches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub attribute: &'static str,
    pub searchable: bool,
}

/// The schema of an indexed message. The shell index stores items, not typed columns, so each field maps to an item
/// attribute; the fields with no attribute of their own are custom attributes.
pub const SCHEMA: [Field; 9] = [
    Field { name: "subject", attribute: "title", searchable: true },
    Field { name: "from", attribute: "authorNames, persons", searchable: true },
    Field { name: "recipients", attribute: "persons", searchable: true },
    Field { name: "date", attribute: "contentCreationDate", searchable: true },
    Field { name: "unread", attribute: "keywords", searchable: true },
    Field { name: "flagged", attribute: "keywords", searchable: true },
    Field { name: "list", attribute: "customAttributes", searchable: true },
    Field { name: "mailbox", attribute: "customAttributes", searchable: true },
    Field { name: "account", attribute: "customAttributes", searchable: false },
];

/// The id of a message in the index.
pub fn unique_id(account: &str, mailbox: &str, uid: u32) -> String {
    format!("{account}/{mailbox}/{uid}")
}

/// The account, mailbox and UID an id names; `None` for an id this app did not make.
pub fn parse_id(id: &str) -> Option<(&str, &str, u32)> {
    let (rest, uid) = id.rsplit_once('/')?;
    let (account, mailbox) = rest.split_once('/')?;
    Some((account, mailbox, uid.parse().ok()?))
}

/// What a message is indexed with: every field of the schema and the flags. A change of any of them indexes it again.
pub fn signature(row: &Row) -> String {
    let m = &row.summary;
    let flags = format!("{}{}", u8::from(row.flags.contains(Flag::Seen)), u8::from(row.flags.contains(Flag::Flagged)));
    format!("{}\n{}\n{}\n{}\n{}\n{}\n{flags}", m.subject, m.from, m.to, m.cc, m.date, m.list_id)
}

/// Seconds since the Unix epoch of an RFC 2822 date (`Tue, 13 Oct 2026 09:30:00 +0200`; the weekday, the seconds and
/// the zone are optional; numeric zones and `GMT`/`UT` are read). `None` for anything else.
pub fn parse_date(date: &str) -> Option<i64> {
    let text = date.split_once(',').map_or(date, |(_, rest)| rest);
    let mut parts = text.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let month = parts.next()?;
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let month = months.iter().position(|m| month.len() >= 3 && month.is_char_boundary(3) && month[..3].eq_ignore_ascii_case(m))? as i64 + 1;
    let mut year: i64 = parts.next()?.parse().ok()?;
    if year < 50 {
        year += 2000;
    } else if year < 100 {
        year += 1900;
    }
    let mut clock = parts.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next().map_or(Some(0), |s| s.parse().ok())?;
    let zone = parts.next().unwrap_or("+0000");
    let offset = if (zone.starts_with('+') || zone.starts_with('-')) && zone.len() == 5 && zone[1..].bytes().all(|b| b.is_ascii_digit()) {
        let n: i64 = zone[1..].parse().ok()?;
        let secs = (n / 100) * 3600 + (n % 100) * 60;
        if zone.starts_with('-') { -secs } else { secs }
    } else {
        0
    };
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// The people of an address list (`To`, `Cc` or `From`), one per address; entries without an address are dropped.
fn persons(list: &str) -> Vec<Person> {
    split_addresses(list)
        .iter()
        .filter_map(|entry| {
            let address = address_of(entry);
            if !address.contains('@') {
                return None;
            }
            let name = entry.split('<').next().unwrap_or("").trim().trim_matches('"').trim();
            let name = if name.contains('@') { "" } else { name };
            Some(Person { display_name: name.to_string(), handles: vec![address.to_string()], handle_identifier: "email".into(), contact_identifier: String::new() })
        })
        .collect()
}

/// The searchable item of a message with the fields of [`SCHEMA`]: the subject as the title, the sender as the
/// description and as the author, every sender and recipient as a person, the date, the state keywords (`unread`,
/// `flagged`), the list it came through and the mailbox it is in.
pub fn index_item(account: &str, mailbox: &str, row: &Row) -> CSSearchableItem {
    let m = &row.summary;
    let subject = m.subject.trim();
    let title = if subject.is_empty() { NO_SUBJECT } else { subject };
    let mut set = CSSearchableItemAttributeSet::new(title).content_description(m.from.trim()).content_type(CONTENT_TYPE);
    let from = persons(&m.from);
    let names: Vec<&str> = from.iter().map(|p| if p.display_name.is_empty() { p.handles[0].as_str() } else { p.display_name.as_str() }).collect();
    set = set.author_names(&names);
    for person in from.iter().cloned().chain(persons(&m.to)).chain(persons(&m.cc)) {
        set = set.person(person);
    }
    if let Some(secs) = parse_date(&m.date) {
        set = set.content_creation_date(secs);
    }
    let unread = !row.flags.contains(Flag::Seen);
    let flagged = row.flags.contains(Flag::Flagged);
    let mut keywords = vec![];
    if unread {
        keywords.push("unread");
    }
    if flagged {
        keywords.push("flagged");
    }
    set = set.keywords(&keywords).custom_value(&CSCustomAttributeKey::new("mailbox", true), mailbox).custom_value(&CSCustomAttributeKey::new("account", false), account);
    if !m.list_id.trim().is_empty() {
        set = set.custom_value(&CSCustomAttributeKey::new("list", true), m.list_id.trim());
    }
    // unread and flagged messages rank above the rest
    set = set.ranking_hint(i32::from(unread) + 2 * i32::from(flagged));
    CSSearchableItem::new(&unique_id(account, mailbox, row.uid), SPOT_DOMAIN, set)
}

/// What to send to the index: the items to index again, and the ids to delete.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub changed: Vec<CSSearchableItem>,
    pub gone: Vec<String>,
}

/// Compares the messages of the mailbox on screen with `indexed` (the signature each id was last indexed with) and
/// updates `indexed` to match. Other mailboxes' items are left alone. With "Show in Search" off (`show` false), or
/// with no account, everything it indexed goes.
pub fn plan(indexed: &mut HashMap<String, String>, show: bool, account: Option<&str>, mailbox: &str, rows: &[Row]) -> Plan {
    let Some(account) = account.filter(|_| show) else {
        let gone = indexed.keys().cloned().collect();
        indexed.clear();
        return Plan { changed: vec![], gone };
    };
    let prefix = format!("{account}/{mailbox}/");
    let mut current = HashMap::new();
    let mut changed = vec![];
    for row in rows.iter().take(MAX_ROWS) {
        let id = unique_id(account, mailbox, row.uid);
        let sig = signature(row);
        if indexed.get(&id) != Some(&sig) {
            changed.push(index_item(account, mailbox, row));
        }
        current.insert(id, sig);
    }
    let gone: Vec<String> = indexed.keys().filter(|id| id.starts_with(&prefix) && !current.contains_key(*id)).cloned().collect();
    for id in &gone {
        indexed.remove(id);
    }
    indexed.extend(current);
    Plan { changed, gone }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;
    use crate::message::Summary;
    use crate::prefs::Prefs;

    fn row(uid: u32, subject: &str, from: &str) -> Row {
        Row { uid, flags: Flags::default(), summary: Summary { subject: subject.into(), from: from.into(), ..Default::default() } }
    }

    #[test]
    fn an_item_has_the_subject_and_the_sender_and_no_body() {
        let item = index_item("acct", "INBOX", &row(42, "  Invoice 7 ", "Ana <ana@example.org>"));
        assert_eq!(item.unique_identifier, "acct/INBOX/42");
        assert_eq!(item.domain_identifier, SPOT_DOMAIN);
        assert_eq!(item.attribute_set.title, "Invoice 7");
        assert_eq!(item.attribute_set.content_description, "Ana <ana@example.org>");
        let blank = index_item("acct", "INBOX", &row(1, "", "x"));
        assert_eq!(blank.attribute_set.title, NO_SUBJECT);
    }

    #[test]
    fn ids_round_trip_and_reject_foreign_ones() {
        assert_eq!(parse_id(&unique_id("acct", "Sent/2026", 9)), Some(("acct", "Sent/2026", 9)));
        assert_eq!(parse_id("notes-1"), None);
        assert_eq!(parse_id("acct/INBOX/x"), None);
    }

    #[test]
    fn plan_indexes_new_rows_and_removes_vanished_ones_in_this_mailbox_only() {
        let mut indexed = HashMap::new();
        let rows = vec![row(1, "Hi", "A"), row(2, "Yo", "B")];
        let p = plan(&mut indexed, true, Some("acct"), "INBOX", &rows);
        assert_eq!(p.changed.len(), 2);
        assert!(p.gone.is_empty());
        // another mailbox's items are not touched when this one is shown
        indexed.insert(unique_id("acct", "Sent", 5), "old".into());
        assert!(plan(&mut indexed, true, Some("acct"), "INBOX", &rows).changed.is_empty());
        let p = plan(&mut indexed, true, Some("acct"), "INBOX", &[row(2, "Yo", "B")]);
        assert!(p.changed.is_empty());
        assert_eq!(p.gone, vec!["acct/INBOX/1".to_string()]);
        assert!(indexed.contains_key("acct/Sent/5"));
    }

    #[test]
    fn an_edited_subject_is_indexed_again() {
        let mut indexed = HashMap::new();
        plan(&mut indexed, true, Some("acct"), "INBOX", &[row(1, "Hi", "A")]);
        let p = plan(&mut indexed, true, Some("acct"), "INBOX", &[row(1, "Hi again", "A")]);
        assert_eq!(p.changed.len(), 1);
        assert_eq!(p.changed[0].attribute_set.title, "Hi again");
    }

    #[test]
    fn only_the_newest_rows_are_indexed() {
        let mut indexed = HashMap::new();
        let rows: Vec<Row> = (0..(MAX_ROWS as u32 + 30)).map(|i| row(i, "s", "f")).collect();
        let p = plan(&mut indexed, true, Some("acct"), "INBOX", &rows);
        assert_eq!(p.changed.len(), MAX_ROWS);
    }

    #[test]
    fn switching_search_off_removes_everything_it_indexed() {
        let mut indexed = HashMap::new();
        plan(&mut indexed, true, Some("acct"), "INBOX", &[row(1, "Hi", "A")]);
        let p = plan(&mut indexed, false, Some("acct"), "INBOX", &[row(1, "Hi", "A")]);
        assert_eq!(p.gone, vec!["acct/INBOX/1".to_string()]);
        assert!(indexed.is_empty());
        assert!(plan(&mut indexed, true, None, "INBOX", &[]).gone.is_empty());
    }

    #[test]
    fn dates_parse_to_unix_seconds() {
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:00 +0000"), Some(0));
        assert_eq!(parse_date("Tue, 13 Oct 2026 09:30:00 +0000"), Some(1_791_883_800));
        // the zone is taken off; the weekday and the seconds are optional
        assert_eq!(parse_date("13 Oct 2026 11:30 +0200"), Some(1_791_883_800));
        assert_eq!(parse_date("13 Oct 2026 04:00:00 -0530"), Some(1_791_883_800));
        assert_eq!(parse_date("13 Oct 2026 09:30:00 GMT"), Some(1_791_883_800));
        assert_eq!(parse_date("29 Feb 2024 00:00:00 +0000"), Some(1_709_164_800));
        for bad in ["", "yesterday", "13 Foo 2026 09:30", "40 Oct 2026 09:30", "13 Oct 2026 25:00"] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_schema_names_each_field_once_and_the_item_fills_it() {
        let mut names: Vec<&str> = SCHEMA.iter().map(|f| f.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), SCHEMA.len());
        let mut r = row(7, "Plan", "\"Smith, Ana\" <ana@example.org>");
        r.summary.to = "Bo <bo@example.org>, cy@example.org".into();
        r.summary.cc = "Di <di@example.org>".into();
        r.summary.date = "Tue, 13 Oct 2026 09:30:00 +0000".into();
        r.summary.list_id = "<dev.example.org>".into();
        let set = index_item("acct", "Archive", &r).attribute_set;
        assert_eq!(set.content_type, CONTENT_TYPE);
        assert_eq!(set.author_names, vec!["Smith, Ana".to_string()]);
        let handles: Vec<&str> = set.persons.iter().map(|p| p.handles[0].as_str()).collect();
        assert_eq!(handles, ["ana@example.org", "bo@example.org", "cy@example.org", "di@example.org"]);
        assert!(set.persons.iter().all(|p| p.handle_identifier == "email"));
        assert_eq!(set.content_creation_date, Some(1_791_883_800));
        let custom = |k: &str| set.custom.iter().find(|c| c.key == k).cloned();
        assert_eq!(custom("mailbox").map(|c| (c.value, c.unsearchable)), Some(("Archive".to_string(), false)));
        assert_eq!(custom("account").map(|c| c.unsearchable), Some(true));
        assert_eq!(custom("list").map(|c| c.value), Some("<dev.example.org>".to_string()));
        // a message without a list or a date gets neither
        let plain = index_item("acct", "INBOX", &row(1, "x", "ana@example.org")).attribute_set;
        assert!(plain.custom.iter().all(|c| c.key != "list"));
        assert_eq!(plain.content_creation_date, None);
    }

    #[test]
    fn state_keywords_and_rank_follow_the_flags() {
        let mut r = row(1, "s", "f@example.org");
        let unread = index_item("a", "INBOX", &r).attribute_set;
        assert_eq!(unread.keywords, vec!["unread".to_string()]);
        assert_eq!(unread.ranking_hint, Some(1));
        r.flags = r.flags.with(Flag::Seen, true).with(Flag::Flagged, true);
        let flagged = index_item("a", "INBOX", &r).attribute_set;
        assert_eq!(flagged.keywords, vec!["flagged".to_string()]);
        assert_eq!(flagged.ranking_hint, Some(2));
    }

    #[test]
    fn reading_or_dating_a_message_indexes_it_again() {
        let mut indexed = HashMap::new();
        let mut r = row(1, "Hi", "a@example.org");
        plan(&mut indexed, true, Some("acct"), "INBOX", &[r.clone()]);
        r.flags = r.flags.with(Flag::Seen, true);
        let p = plan(&mut indexed, true, Some("acct"), "INBOX", &[r.clone()]);
        assert_eq!(p.changed.len(), 1);
        assert!(p.changed[0].attribute_set.keywords.is_empty());
        r.summary.date = "Tue, 13 Oct 2026 09:30:00 +0000".into();
        assert_eq!(plan(&mut indexed, true, Some("acct"), "INBOX", &[r]).changed.len(), 1);
    }

    #[test]
    fn the_setting_is_on_by_default_and_saved() {
        assert!(Prefs::default().show_in_search);
        let mut off = Prefs::default();
        off.show_in_search = false;
        assert!(!Prefs::from_json(&off.to_json()).show_in_search);
        assert!(Prefs::from_json(&Prefs::default().to_json()).show_in_search);
    }
}
