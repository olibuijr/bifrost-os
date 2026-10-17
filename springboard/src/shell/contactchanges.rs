//! The change log of the contacts store, kept in a sidecar beside the contacts file. Each applied save appends one event
//! per changed contact (added, updated or deleted) and gets the next sequence number. A client keeps the token it last
//! saw, which is the last sequence number, and asks for the events after it. The log keeps the newest `KEEP` events; a
//! token older than the oldest kept event is expired and the client must read the whole book again.
use super::contactstore::Contact;
use bifrost_ui::contact_groups::Groups;
use std::io;
use std::path::{Path, PathBuf};

const HEADER: &str = "springboard-contact-changes-v1";
/// Most events kept; older ones are dropped from the log.
pub const KEEP: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Added,
    Updated,
    Deleted,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Added => "added",
            Kind::Updated => "updated",
            Kind::Deleted => "deleted",
        }
    }

    fn from_name(name: &str) -> Option<Kind> {
        match name {
            "added" => Some(Kind::Added),
            "updated" => Some(Kind::Updated),
            "deleted" => Some(Kind::Deleted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: Kind,
    pub id: u64,
}

/// The retained events and the last sequence number handed out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    pub last_seq: u64,
    pub events: Vec<Event>,
}

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contact-changes.tsv")
}

/// The changes between two books, by contact identifier: added, updated or deleted, in identifier order.
pub fn diff(before: &[Contact], after: &[Contact]) -> Vec<(Kind, u64)> {
    let mut out = Vec::new();
    for contact in after {
        match before.iter().find(|old| old.id == contact.id) {
            None => out.push((Kind::Added, contact.id)),
            Some(old) if old != contact => out.push((Kind::Updated, contact.id)),
            Some(_) => {}
        }
    }
    for old in before {
        if !after.iter().any(|c| c.id == old.id) {
            out.push((Kind::Deleted, old.id));
        }
    }
    out.sort_by_key(|(_, id)| *id);
    out
}

impl Log {
    /// Appends the changes as events, numbering each, and drops the oldest events beyond `KEEP`.
    pub fn append(&mut self, changes: &[(Kind, u64)]) {
        for (kind, id) in changes {
            self.last_seq += 1;
            self.events.push(Event { seq: self.last_seq, kind: *kind, id: *id });
        }
        let excess = self.events.len().saturating_sub(KEEP);
        self.events.drain(..excess);
    }

    /// The events after `token`. Refused as expired when the token is older than the oldest kept event.
    pub fn since(&self, token: u64) -> Result<Vec<&Event>, String> {
        let oldest_kept = self.events.first().map(|e| e.seq).unwrap_or(self.last_seq + 1);
        if token + 1 < oldest_kept && token < self.last_seq {
            return Err("the change token has expired; read the whole book again".into());
        }
        Ok(self.events.iter().filter(|e| e.seq > token).collect())
    }
}

/// Writes the log as sidecar text.
pub fn render(log: &Log) -> String {
    let mut out = format!("{HEADER}\nlast\t{}\n", log.last_seq);
    for e in &log.events {
        out.push_str(&format!("event\t{}\t{}\t{}\n", e.seq, e.kind.name(), e.id));
    }
    out
}

/// Reads the sidecar text. Anything that is not a change log is an error.
pub fn parse(text: &str) -> Result<Log, String> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err("Unknown contact changes format".into());
    }
    let mut log = Log::default();
    let mut saw_last = false;
    for line in lines.filter(|l| !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["last", n] => {
                log.last_seq = n.parse().map_err(|_| "bad change counter".to_string())?;
                saw_last = true;
            }
            ["event", seq, kind, id] => {
                let seq = seq.parse().map_err(|_| "bad change sequence".to_string())?;
                let kind = Kind::from_name(kind).ok_or_else(|| "bad change kind".to_string())?;
                let id = id.parse().map_err(|_| "bad change id".to_string())?;
                log.events.push(Event { seq, kind, id });
            }
            _ => return Err("bad contact changes line".into()),
        }
    }
    if !saw_last {
        return Err("missing change counter".into());
    }
    Ok(log)
}

/// Appends changes to the log beside `contacts_file`, and writes it. Nothing is written when there are no changes.
pub fn record_for(contacts_file: &Path, changes: &[(Kind, u64)]) -> io::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let path = path_for(contacts_file);
    let mut log = load(&path);
    log.append(changes);
    save(&path, &log)
}

/// The contacts whose groups changed between two sets of groups: each is an update, so the change log tells apps
/// that the contact changed. A deleted group's members count too.
pub fn group_changes(before: &Groups, after: &Groups, contacts: &[Contact]) -> Vec<(Kind, u64)> {
    contacts
        .iter()
        .filter(|c| before.groups_of(c.id) != after.groups_of(c.id))
        .map(|c| (Kind::Updated, c.id))
        .collect()
}

/// The log at `path`; empty when it is missing or unreadable.
pub fn load(path: &Path) -> Log {
    std::fs::read_to_string(path).ok().and_then(|t| parse(&t).ok()).unwrap_or_default()
}

/// Writes the log atomically.
pub fn save(path: &Path, log: &Log) -> io::Result<()> {
    super::fsutil::write_atomic(path, render(log))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: u64, name: &str) -> Contact {
        Contact { id, name: name.into(), ..Contact::default() }
    }

    #[test]
    fn a_save_is_diffed_into_added_updated_and_deleted() {
        let before = vec![contact(1, "Ada"), contact(2, "Grace")];
        let after = vec![contact(1, "Ada L."), contact(3, "Alan")];
        assert_eq!(diff(&before, &after), vec![(Kind::Updated, 1), (Kind::Added, 3), (Kind::Deleted, 2)]);
    }

    #[test]
    fn events_are_numbered_and_fetched_after_a_token() {
        let mut log = Log::default();
        log.append(&[(Kind::Added, 1), (Kind::Added, 2)]);
        log.append(&[(Kind::Deleted, 1)]);
        assert_eq!(log.last_seq, 3);
        let after_first = log.since(1).unwrap();
        assert_eq!(after_first.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert!(log.since(3).unwrap().is_empty());
    }

    #[test]
    fn a_token_older_than_the_kept_history_is_expired() {
        let mut log = Log::default();
        let many: Vec<(Kind, u64)> = (0..KEEP as u64 + 5).map(|i| (Kind::Updated, i)).collect();
        log.append(&many);
        assert_eq!(log.events.len(), KEEP);
        assert!(log.since(0).is_err());
        assert_eq!(log.since(log.last_seq).unwrap().len(), 0);
    }

    #[test]
    fn the_log_round_trips_through_its_sidecar_text() {
        let mut log = Log::default();
        log.append(&[(Kind::Added, 1), (Kind::Deleted, 1)]);
        let back = parse(&render(&log)).unwrap();
        assert_eq!(back, log);
        assert!(parse("garbage").is_err());
        assert_eq!(path_for(Path::new("/d/Contacts/contacts.tsv")), PathBuf::from("/d/Contacts/contact-changes.tsv"));
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;

    #[test]
    fn a_group_membership_change_is_an_update_of_that_contact() {
        let contacts = vec![Contact { id: 1, name: "Ada".into(), ..Contact::default() }, Contact { id: 2, name: "Grace".into(), ..Contact::default() }];
        let before = Groups::new();
        let mut after = before.clone();
        let family = after.add("Family", None).unwrap();
        assert!(group_changes(&before, &after, &contacts).is_empty(), "a new empty group changes no contact");
        let snapshot = after.clone();
        after.add_member(family, 2).unwrap();
        assert_eq!(group_changes(&snapshot, &after, &contacts), vec![(Kind::Updated, 2)]);
    }

    #[test]
    fn record_for_appends_to_the_log_beside_the_contacts_file() {
        let dir = std::env::temp_dir().join(format!("contact-changes-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let contacts_file = dir.join("contacts.tsv");
        record_for(&contacts_file, &[(Kind::Updated, 7)]).unwrap();
        record_for(&contacts_file, &[]).unwrap();
        let log = load(&path_for(&contacts_file));
        assert_eq!(log.last_seq, 1);
        assert_eq!(log.events, vec![Event { seq: 1, kind: Kind::Updated, id: 7 }]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
