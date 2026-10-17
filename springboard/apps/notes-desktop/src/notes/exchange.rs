//! Notes from an Exchange account (Settings > Internet Accounts > Notes (Exchange)).
//!
//! The account's Notes folder is mirrored in a Notes folder named after the account: each remote note is one `<id>.md`
//! file in it, so the list, the editor, search, tags and the rest work on them like on any note. A small file in the
//! notes directory (`.exchange-<account id>.tsv`, hidden) remembers, for each local note, which mailbox item it is,
//! that item's change key, and a hash of the text as it was last in step with the server.
//!
//! One round (the network part is [`bifrost_app::exchange_notes::Sync`]) is [`plan`], then the server, then [`apply`]:
//!
//! - [`plan`] lists the local changes: a note in the folder that is not linked is created on the server; a linked note
//!   whose text no longer matches its hash is updated; a linked note whose file is gone is deleted (the server moves it
//!   to Deleted Items, so it can be undone there). If the folder itself is gone, nothing is deleted on the server: the
//!   links are dropped and the next round brings the notes back.
//! - [`apply`] records what the server took, then brings the server's changes in: new notes become files, a note with a
//!   newer change key replaces the file's text (when the person also edited it since the last round, their text is kept
//!   as a separate new note and goes to the server as its own), and a note that is gone from the server moves to
//!   Recently Deleted. A note that is open in the editor is left alone until it is closed.
//!
//! Moving a note out of the account's folder is a delete on the server. Subject and body travel as plain text: the
//! first line of a note is its subject, the whole text is the body.

use super::folders;
use super::store::{self, Note};
use bifrost_app::controls::internet_accounts::Account;
use bifrost_app::exchange_notes::{ItemId, Op, Report};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// A subject is one line of at most this many characters (Exchange keeps 255).
const SUBJECT_CHARS: usize = 255;

/// FNV-1a over the text: cheap, stable between runs, and enough to tell "edited since the last round".
pub fn hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The local id of the note for a mailbox item: the item identifier has characters a file name cannot hold.
pub fn local_id(item_id: &str) -> String {
    format!("ex{:016x}", hash(item_id))
}

/// The folder the account's notes live in.
pub fn folder_name(account: &Account) -> String {
    folders::valid_name(&account.name).unwrap_or_else(|| "Exchange".to_string())
}

/// The note text for a server note. The body is the text; when the body does not start with the subject (a note made
/// elsewhere), the subject goes on top as the title.
pub fn text_of(note: &bifrost_app::exchange_notes::Note) -> String {
    let subject = note.subject.trim();
    let body = note.body.trim_end();
    if body.trim().is_empty() {
        subject.to_string()
    } else if subject.is_empty() || body.trim_start().starts_with(subject) {
        body.to_string()
    } else {
        format!("{subject}\n\n{body}")
    }
}

/// The subject and body to send for a note's text: the title (one line) and the whole text.
pub fn wire_of(text: &str) -> (String, String) {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let bare = line.trim_start_matches('#').trim_start();
    let subject: String = if bare.is_empty() { line } else { bare }.chars().filter(|c| !c.is_control()).take(SUBJECT_CHARS).collect();
    (subject, text.to_string())
}

// --- the links -----------------------------------------------------------------------------------------------------

/// What a local note is on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub item: ItemId,
    /// [`hash`] of the text when it last matched the server.
    pub stamp: u64,
}

/// The notes of one account and the mailbox items they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    pub folder: String,
    pub links: BTreeMap<String, Link>,
}

fn mirror_path(dir: &Path, account_id: &str) -> PathBuf {
    let safe: String = account_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    dir.join(format!(".exchange-{safe}.tsv"))
}

impl Mirror {
    /// Reads the links of an account; an account never synced (or an unreadable file) has none.
    pub fn load(dir: &Path, account: &Account) -> Mirror {
        let mut m = Mirror { folder: folder_name(account), links: BTreeMap::new() };
        let Ok(text) = std::fs::read_to_string(mirror_path(dir, &account.id)) else { return m };
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["folder", name] => m.folder = (*name).to_string(),
                [local, stamp, id, key] => {
                    if let Ok(stamp) = u64::from_str_radix(stamp, 16) {
                        m.links.insert((*local).to_string(), Link { item: ItemId { id: (*id).to_string(), change_key: (*key).to_string() }, stamp });
                    }
                }
                _ => {}
            }
        }
        m
    }

    pub fn save(&self, dir: &Path, account: &Account) -> std::io::Result<()> {
        let mut out = format!("folder\t{}\n", self.folder);
        for (local, l) in &self.links {
            out.push_str(&format!("{local}\t{:x}\t{}\t{}\n", l.stamp, l.item.id, l.item.change_key));
        }
        springboard::shell::fsutil::write_atomic(&mirror_path(dir, &account.id), &out)
    }

    fn dir(&self, notes_dir: &Path) -> PathBuf {
        notes_dir.join(&self.folder)
    }
}

// --- the round -----------------------------------------------------------------------------------------------------

/// The local changes to send for `account`. Unsaved edits must be on disk first (the caller flushes).
pub fn plan(dir: &Path, account: &Account) -> Vec<Op> {
    let mut mirror = Mirror::load(dir, account);
    let here = mirror.dir(dir);
    match std::fs::read_dir(&here) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // the folder was deleted: forget the links instead of deleting the server's notes
            if !mirror.links.is_empty() {
                mirror.links.clear();
                let _ = mirror.save(dir, account);
            }
            return Vec::new();
        }
        // unreadable for another reason: send nothing, a deletion could be a misread
        Err(_) => return Vec::new(),
    }
    let notes = store::load(&here);
    let mut ops = Vec::new();
    for n in &notes {
        let (subject, body) = wire_of(&n.text);
        let stamp = hash(&n.text);
        match mirror.links.get(&n.id) {
            Some(link) if link.stamp == stamp => {}
            Some(link) => ops.push(Op::Update { local: n.id.clone(), stamp, id: link.item.clone(), subject, body }),
            None if n.text.trim().is_empty() => {}
            None => ops.push(Op::Create { local: n.id.clone(), stamp, subject, body }),
        }
    }
    let present: HashSet<&str> = notes.iter().map(|n| n.id.as_str()).collect();
    for (local, link) in &mirror.links {
        if !present.contains(local.as_str()) {
            ops.push(Op::Delete { local: local.clone(), id: link.item.clone() });
        }
    }
    ops
}

/// What [`apply`] did to the notes on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// Notes edited on both sides: the local text was kept as a new note.
    pub conflicts: usize,
    /// Why part of the round did not happen, for the person to read.
    pub errors: Vec<String>,
}

impl Applied {
    /// Whether the files in the notes directory changed (the list must be read again).
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed + self.conflicts > 0
    }
}

fn set_mtime(path: &Path, unix: i64) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix.max(0) as u64));
    }
}

/// Writes a server note as a file in `folder`, dated as the server dates it.
fn write_note(folder: &Path, local: &str, text: &str, modified: &str, now: i64) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    store::save(folder, &Note { id: local.to_string(), text: text.to_string(), modified: now })?;
    if let Some(t) = bifrost_app::coding::parse_iso8601(modified) {
        set_mtime(&folder.join(format!("{local}.md")), t as i64);
    }
    Ok(())
}

/// A local id for a copy of a note, free in `folder`.
fn copy_id(folder: &Path, local: &str) -> String {
    (1..).map(|n| format!("{local}-conflict{n}")).find(|id| !folder.join(format!("{id}.md")).exists()).unwrap_or_default()
}

/// Records what the server took of the changes in [`plan`], then brings in what the server has. `busy` are the local ids
/// of notes the person has open or unsaved: they are not touched this round. `now` is the Unix time for the bin stamp.
pub fn apply(dir: &Path, account: &Account, report: &Report, busy: &[String], now: i64) -> Applied {
    let mut mirror = Mirror::load(dir, account);
    let here = mirror.dir(dir);
    let mut out = Applied::default();
    let mut just_written: HashSet<String> = HashSet::new();
    for w in &report.written {
        match &w.result {
            Ok(Some(item)) => {
                mirror.links.insert(w.local.clone(), Link { item: item.clone(), stamp: w.stamp });
                just_written.insert(w.local.clone());
            }
            Ok(None) => {
                mirror.links.remove(&w.local);
            }
            Err(e) => {
                if !out.errors.contains(e) {
                    out.errors.push(e.clone());
                }
            }
        }
    }
    let remote = match &report.notes {
        Ok(notes) => notes,
        Err(e) => {
            if !out.errors.contains(e) {
                out.errors.push(e.clone());
            }
            let _ = mirror.save(dir, account);
            return out;
        }
    };
    let by_item: BTreeMap<String, String> = mirror.links.iter().map(|(local, l)| (l.item.id.clone(), local.clone())).collect();
    for r in remote {
        match by_item.get(&r.item.id) {
            Some(local) => {
                let link = mirror.links[local].clone();
                if link.item.change_key == r.item.change_key || busy.contains(local) {
                    continue;
                }
                let file = here.join(format!("{local}.md"));
                let Ok(current) = std::fs::read_to_string(&file) else { continue };
                if hash(&current) != link.stamp {
                    // edited on both sides: the local text stays as its own note, which the next round creates
                    let copy = copy_id(&here, local);
                    if store::save(&here, &Note { id: copy, text: current, modified: now }).is_ok() {
                        out.conflicts += 1;
                    }
                }
                let text = text_of(r);
                if write_note(&here, local, &text, &r.modified, now).is_ok() {
                    mirror.links.insert(local.clone(), Link { item: r.item.clone(), stamp: hash(&text) });
                    out.updated += 1;
                }
            }
            None => {
                let local = local_id(&r.item.id);
                let text = text_of(r);
                if write_note(&here, &local, &text, &r.modified, now).is_ok() {
                    mirror.links.insert(local, Link { item: r.item.clone(), stamp: hash(&text) });
                    out.added += 1;
                }
            }
        }
    }
    // a linked note the server no longer lists goes to Recently Deleted (a note written this round may not be listed yet)
    let listed: HashSet<&str> = remote.iter().map(|r| r.item.id.as_str()).collect();
    let gone: Vec<String> = mirror
        .links
        .iter()
        .filter(|(local, l)| !listed.contains(l.item.id.as_str()) && !just_written.contains(*local) && !busy.contains(*local))
        .map(|(local, _)| local.clone())
        .collect();
    for local in gone {
        if here.join(format!("{local}.md")).exists() {
            if folders::trash(dir, &local, Some(&mirror.folder), now).is_ok() {
                out.removed += 1;
            } else {
                continue;
            }
        }
        mirror.links.remove(&local);
    }
    if let Err(e) = mirror.save(dir, account) {
        out.errors.push(format!("The Exchange links could not be saved: {e}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::internet_accounts::Kind;
    use bifrost_app::exchange_notes::{Note as Remote, Written};

    fn account() -> Account {
        Account { url: "https://mail.example.com/EWS/Exchange.asmx".into(), ..Account::new(Kind::Exchange, "exchange-1", "Work", "ann") }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("notes-exchange-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn item(n: &str, key: &str) -> ItemId {
        ItemId { id: format!("AAMk{n}"), change_key: key.into() }
    }

    fn remote(n: &str, key: &str, subject: &str, body: &str) -> Remote {
        Remote { item: item(n, key), subject: subject.into(), body: body.into(), modified: "2026-10-17T09:30:00Z".into() }
    }

    fn report(notes: Vec<Remote>) -> Report {
        Report { written: Vec::new(), notes: Ok(notes) }
    }

    fn files(dir: &Path) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = store::load(&dir.join("Work")).into_iter().map(|n| (n.id, n.text)).collect();
        v.sort();
        v
    }

    #[test]
    fn text_and_wire_round_trip_without_growing() {
        let r = remote("1", "a", "Milk", "Milk\nand eggs");
        let text = text_of(&r);
        assert_eq!(text, "Milk\nand eggs");
        let (subject, body) = wire_of(&text);
        assert_eq!((subject.as_str(), body.as_str()), ("Milk", "Milk\nand eggs"));
        // a note made elsewhere keeps its subject on top; sending it back gives a body that starts with the subject
        let elsewhere = remote("2", "a", "Plan", "step one");
        assert_eq!(text_of(&elsewhere), "Plan\n\nstep one");
        let back = remote("2", "b", "Plan", &wire_of(&text_of(&elsewhere)).1);
        assert_eq!(text_of(&back), text_of(&elsewhere));
        assert_eq!(text_of(&remote("3", "a", "Only subject", "")), "Only subject");
        assert_eq!(wire_of("# Heading\nbody").0, "Heading");
    }

    #[test]
    fn a_first_round_brings_the_server_notes_into_a_folder_named_after_the_account() {
        let dir = scratch("first");
        let a = account();
        assert!(plan(&dir, &a).is_empty(), "nothing to send before the folder exists");
        let applied = apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk\neggs"), remote("2", "k1", "Plan", "step")]), &[], 1_791_000_000);
        assert_eq!((applied.added, applied.updated, applied.removed), (2, 0, 0));
        let got = files(&dir);
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|(_, t)| t == "Milk\neggs") && got.iter().any(|(_, t)| t == "Plan\n\nstep"));
        // the file is dated as the server dates it, and the next plan has nothing to send
        let note = store::load(&dir.join("Work")).into_iter().next().unwrap();
        assert_eq!(note.modified, 1_792_229_400);
        assert!(plan(&dir, &a).is_empty());
        // the same server answer again changes nothing
        assert_eq!(apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk\neggs"), remote("2", "k1", "Plan", "step")]), &[], 1_791_000_001), Applied::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_edits_new_notes_and_deletions_become_ops() {
        let dir = scratch("ops");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk"), remote("2", "k1", "Plan", "Plan")]), &[], 10);
        let (l1, l2) = (local_id("AAMk1"), local_id("AAMk2"));
        let here = dir.join("Work");
        store::save(&here, &Note { id: l1.clone(), text: "Milk\nand bread".into(), modified: 0 }).unwrap();
        std::fs::remove_file(here.join(format!("{l2}.md"))).unwrap();
        store::save(&here, &Note { id: "100".into(), text: "Fresh\nnote".into(), modified: 0 }).unwrap();
        let ops = plan(&dir, &a);
        assert_eq!(ops.len(), 3);
        assert!(ops.contains(&Op::Update { local: l1, stamp: hash("Milk\nand bread"), id: item("1", "k1"), subject: "Milk".into(), body: "Milk\nand bread".into() }));
        assert!(ops.contains(&Op::Delete { local: l2, id: item("2", "k1") }));
        assert!(ops.contains(&Op::Create { local: "100".into(), stamp: hash("Fresh\nnote"), subject: "Fresh".into(), body: "Fresh\nnote".into() }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_the_server_took_is_linked_and_not_sent_again() {
        let dir = scratch("took");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk")]), &[], 10);
        let l1 = local_id("AAMk1");
        let here = dir.join("Work");
        store::save(&here, &Note { id: l1.clone(), text: "Milk\nmore".into(), modified: 0 }).unwrap();
        store::save(&here, &Note { id: "100".into(), text: "Fresh".into(), modified: 0 }).unwrap();
        let written = vec![
            Written { local: l1.clone(), stamp: hash("Milk\nmore"), result: Ok(Some(item("1", "k2"))), deleted: false },
            Written { local: "100".into(), stamp: hash("Fresh"), result: Ok(Some(item("5", "k1"))), deleted: false },
        ];
        // the server lists both with their new change keys, the same text
        let notes = vec![remote("1", "k2", "Milk", "Milk\nmore"), remote("5", "k1", "Fresh", "Fresh")];
        let applied = apply(&dir, &a, &Report { written, notes: Ok(notes) }, &[], 20);
        assert_eq!(applied, Applied::default(), "the round trip is silent");
        assert!(plan(&dir, &a).is_empty());
        assert_eq!(Mirror::load(&dir, &a).links["100"].item, item("5", "k1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_server_note_replaces_the_text_and_a_double_edit_keeps_both() {
        let dir = scratch("conflict");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk"), remote("2", "k1", "Plan", "Plan")]), &[], 10);
        let (l1, l2) = (local_id("AAMk1"), local_id("AAMk2"));
        let here = dir.join("Work");
        // note 2 was edited here too
        store::save(&here, &Note { id: l2.clone(), text: "Plan\nmine".into(), modified: 0 }).unwrap();
        let applied = apply(&dir, &a, &report(vec![remote("1", "k2", "Milk", "Milk\nfrom the server"), remote("2", "k2", "Plan", "Plan\ntheirs")]), &[], 20);
        assert_eq!((applied.updated, applied.conflicts), (2, 1));
        let got = files(&dir);
        assert_eq!(got.len(), 3);
        assert!(got.contains(&(l1, "Milk\nfrom the server".into())));
        assert!(got.contains(&(l2.clone(), "Plan\ntheirs".into())));
        let copy = got.iter().find(|(id, _)| id.contains("conflict")).unwrap();
        assert_eq!(copy.1, "Plan\nmine", "the local text is not lost");
        // the copy is a new note for the server
        assert!(plan(&dir, &a).iter().any(|op| matches!(op, Op::Create { body, .. } if body == "Plan\nmine")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_the_server_lost_goes_to_recently_deleted_unless_it_is_open() {
        let dir = scratch("gone");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk"), remote("2", "k1", "Plan", "Plan")]), &[], 10);
        let (l1, l2) = (local_id("AAMk1"), local_id("AAMk2"));
        let applied = apply(&dir, &a, &report(vec![]), &[l2.clone()], 20);
        assert_eq!(applied.removed, 1);
        assert!(dir.join(folders::TRASH).join(format!("{l1}.md")).exists());
        assert!(dir.join("Work").join(format!("{l2}.md")).exists(), "the open note stays");
        assert!(Mirror::load(&dir, &a).links.contains_key(&l2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_folder_never_deletes_on_the_server() {
        let dir = scratch("folder");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk")]), &[], 10);
        std::fs::remove_dir_all(dir.join("Work")).unwrap();
        assert!(plan(&dir, &a).is_empty());
        assert!(Mirror::load(&dir, &a).links.is_empty());
        // the next round brings the note back
        assert_eq!(apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk")]), &[], 30).added, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_read_or_write_is_reported_and_touches_nothing() {
        let dir = scratch("fail");
        let a = account();
        apply(&dir, &a, &report(vec![remote("1", "k1", "Milk", "Milk")]), &[], 10);
        let bad = Report {
            written: vec![Written { local: local_id("AAMk1"), stamp: 1, result: Err("The Exchange server did not accept the login".into()), deleted: false }],
            notes: Err("The Exchange server did not accept the login".into()),
        };
        let applied = apply(&dir, &a, &bad, &[], 20);
        assert_eq!(applied.errors, vec!["The Exchange server did not accept the login".to_string()], "one reason, not two");
        assert!(!applied.changed());
        assert_eq!(files(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
