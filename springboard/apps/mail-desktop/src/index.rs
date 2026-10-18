//! Core Search index of the mail: the subject and the sender of the newest messages of the mailbox on screen (up to
//! `MAX_ROWS`). Bodies are never indexed. Mail Settings "Show in Search" turns it off, and then the app removes what it
//! indexed. An item's id is `account/mailbox/uid`, so the same UID in two mailboxes never collides.
use crate::state::Row;
use bifrost_app::quicksearch::{CSSearchableItem, CSSearchableItemAttributeSet};
use std::collections::HashMap;

/// Search domain of every message.
pub const SPOT_DOMAIN: &str = "mail";
/// The newest messages of a mailbox that are indexed.
pub const MAX_ROWS: usize = 200;
/// Title of a message without a subject.
const NO_SUBJECT: &str = "(no subject)";

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

/// What a message is indexed with: its subject and sender. A change of either indexes it again.
pub fn signature(row: &Row) -> String {
    format!("{}\n{}", row.summary.subject, row.summary.from)
}

/// The searchable item of a message: its subject as the title, its sender as the description.
pub fn index_item(account: &str, mailbox: &str, row: &Row) -> CSSearchableItem {
    let subject = row.summary.subject.trim();
    let title = if subject.is_empty() { NO_SUBJECT } else { subject };
    let set = CSSearchableItemAttributeSet::new(title).content_description(row.summary.from.trim());
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
    fn the_setting_is_on_by_default_and_saved() {
        assert!(Prefs::default().show_in_search);
        let mut off = Prefs::default();
        off.show_in_search = false;
        assert!(!Prefs::from_json(&off.to_json()).show_in_search);
        assert!(Prefs::from_json(&Prefs::default().to_json()).show_in_search);
    }
}
