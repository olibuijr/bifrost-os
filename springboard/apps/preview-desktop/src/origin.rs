//! Where a working copy came from. A file chosen in the open panel, or opened with the app from Files, arrives as an
//! open file with a grant, never as a path; the app works on a copy under `opened/` (`recents`). File > Save has to
//! reach the person's own file, so the grant of each copy is kept here, one `grant<TAB>stamp<TAB>copy` per line under
//! the app's data directory, and a save goes through it: the shell hands over a temporary file beside the original and
//! renames it into place on commit (`Ctx::save_document_in_place`). The copy is then written too, so Open Recent and
//! Revert to Saved read what was saved.
//!
//! The stamp is what the original was when the copy was made or last saved: which file it is (device and inode) and
//! its modification time and length. It tells two files of one name in different folders apart (each gets a copy of
//! its own, `Origins::place`), and it tells a save that the original was changed since (`changed_since`).

use bifrost_app::proto::{DocErrorCode, GrantId};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A file as it was at one moment: which file it is, and its modification time and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub dev: u64,
    pub ino: u64,
    pub secs: i64,
    pub nanos: i64,
    pub len: u64,
}

impl Stamp {
    fn of_metadata(m: &std::fs::Metadata) -> Stamp {
        Stamp { dev: m.dev(), ino: m.ino(), secs: m.mtime(), nanos: m.mtime_nsec(), len: m.len() }
    }

    /// The stamp of an open file (the one the shell handed over).
    pub fn of(file: &std::fs::File) -> Option<Stamp> {
        file.metadata().ok().map(|m| Stamp::of_metadata(&m))
    }

    /// The stamp of the file at `path`; None when it is not there.
    pub fn at(path: &Path) -> Option<Stamp> {
        std::fs::metadata(path).ok().map(|m| Stamp::of_metadata(&m))
    }

    /// Whether both are the same file, whatever was written into it in between.
    pub fn same_file(&self, other: &Stamp) -> bool {
        (self.dev, self.ino) == (other.dev, other.ino)
    }

    fn to_text(self) -> String {
        format!("{}.{}.{}.{}.{}", self.dev, self.ino, self.secs, self.nanos, self.len)
    }

    fn parse(text: &str) -> Option<Stamp> {
        let mut parts = text.split('.');
        let stamp = Stamp {
            dev: parts.next()?.parse().ok()?,
            ino: parts.next()?.parse().ok()?,
            secs: parts.next()?.parse().ok()?,
            nanos: parts.next()?.parse().ok()?,
            len: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(stamp)
    }
}

/// Whether a file was changed between two looks at it: `recorded` is what it was when it was opened or last saved,
/// `now` what it is. A file with no record (a copy made by an earlier version) or that cannot be read now counts as
/// unchanged: the save then goes on, and the shell's own checks answer for it.
pub fn changed_since(recorded: Option<Stamp>, now: Option<Stamp>) -> bool {
    matches!((recorded, now), (Some(then), Some(now)) if then != now)
}

/// One working copy: `key` is its place under `opened/` (a file name, or a numbered folder and a file name).
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    key: String,
    grant: GrantId,
    stamp: Option<Stamp>,
}

/// The originals of the working copies, by the copy's place under `opened/`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Origins {
    entries: Vec<Entry>,
}

fn hex(grant: &GrantId) -> String {
    grant.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<GrantId> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut grant = [0u8; 16];
    for (i, byte) in grant.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(grant)
}

/// The file name of a copy's key.
fn file_of(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// The key of the working copy at `path`; None for a file that is not under `opened/`.
fn key_of(data_dir: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(data_dir.join("opened")).ok()?;
    let parts: Vec<&str> = rest.iter().map(|c| c.to_str()).collect::<Option<_>>()?;
    matches!(parts.len(), 1 | 2).then(|| parts.join("/"))
}

impl Origins {
    /// Reads the list from its text. A line is `grant<TAB>stamp<TAB>copy`, or `grant<TAB>copy` as earlier versions
    /// wrote it (no stamp is known for those). Other lines are dropped.
    pub fn parse(text: &str) -> Origins {
        let mut origins = Origins::default();
        for line in text.lines() {
            let Some((grant, rest)) = line.split_once('\t') else { continue };
            let (stamp, key) = match rest.split_once('\t').and_then(|(stamp, key)| Some((Stamp::parse(stamp)?, key))) {
                Some((stamp, key)) => (Some(stamp), key),
                None => (None, rest),
            };
            if let (Some(grant), false) = (unhex(grant), key.is_empty()) {
                origins.set(key, grant, stamp);
            }
        }
        origins
    }

    pub fn to_text(&self) -> String {
        let line = |e: &Entry| match e.stamp {
            Some(stamp) => format!("{}\t{}\t{}\n", hex(&e.grant), stamp.to_text(), e.key),
            None => format!("{}\t{}\n", hex(&e.grant), e.key),
        };
        self.entries.iter().map(line).collect()
    }

    /// Where the copy of a chosen file named `name` goes under `opened/`, given what the file is (`stamp`). The file
    /// that a copy was already made from keeps that copy, so opening it again does not make a second one. Another
    /// file of the same name (the same name in another folder) gets a place of its own, `2/name`, `3/name` and so on,
    /// instead of taking over the first one's copy, markup and original. A copy an earlier version made has no stamp:
    /// it is taken to be the same file, as it was then.
    pub fn place(&self, name: &str, stamp: Option<&Stamp>) -> String {
        let same = |e: &&Entry| file_of(&e.key) == name && matches!((e.stamp.as_ref(), stamp), (Some(a), Some(b)) if a.same_file(b));
        if let Some(entry) = self.entries.iter().find(same) {
            return entry.key.clone();
        }
        let taken = |key: &str| self.entries.iter().any(|e| e.key == key && e.stamp.is_some());
        if !taken(name) {
            return name.to_string();
        }
        (2u32..).map(|n| format!("{n}/{name}")).find(|key| !taken(key)).unwrap_or_else(|| name.to_string())
    }

    /// Notes that the copy `key` was made from the file `grant` names, which is `stamp` now. Returns the grant the
    /// copy had before, when it was another one: the app no longer needs it.
    pub fn set(&mut self, key: &str, grant: GrantId, stamp: Option<Stamp>) -> Option<GrantId> {
        let old = self.forget(key);
        self.entries.push(Entry { key: key.to_string(), grant, stamp });
        old.filter(|old| *old != grant)
    }

    /// Forgets the copy `key` (it was replaced by a file that has no original, or removed). Returns its grant.
    pub fn forget(&mut self, key: &str) -> Option<GrantId> {
        let at = self.entries.iter().position(|e| e.key == key)?;
        Some(self.entries.remove(at).grant)
    }

    fn entry(&self, data_dir: &Path, path: &Path) -> Option<&Entry> {
        let key = key_of(data_dir, path)?;
        self.entries.iter().find(|e| e.key == key)
    }

    /// The grant of the original of the working copy at `path`; None for a file that is not a working copy (a
    /// clipboard picture, a screenshot) or whose original is not known.
    pub fn grant_of(&self, data_dir: &Path, path: &Path) -> Option<GrantId> {
        self.entry(data_dir, path).map(|e| e.grant)
    }

    /// What the original of the working copy at `path` was when the copy was made or last saved, when that is known.
    pub fn stamp_of(&self, data_dir: &Path, path: &Path) -> Option<Stamp> {
        self.entry(data_dir, path).and_then(|e| e.stamp)
    }

    /// Notes what the original of the working copy at `path` is now (a save just wrote it). True when it changed.
    pub fn restamp(&mut self, data_dir: &Path, path: &Path, stamp: Option<Stamp>) -> bool {
        let Some(key) = key_of(data_dir, path) else { return false };
        match self.entries.iter_mut().find(|e| e.key == key) {
            Some(entry) if entry.stamp != stamp => {
                entry.stamp = stamp;
                true
            }
            _ => false,
        }
    }

    /// Drops the entries whose copy is gone (`recents::sweep_copies` removed it) and returns their grants.
    pub fn prune(&mut self, data_dir: &Path) -> Vec<GrantId> {
        let dir = data_dir.join("opened");
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries).into_iter().partition(|e| dir.join(&e.key).exists());
        self.entries = kept;
        gone.into_iter().map(|e| e.grant).collect()
    }
}

fn list_path(data_dir: &Path) -> PathBuf {
    data_dir.join("origins.txt")
}

/// Loads the list; a missing or unreadable file gives an empty one.
pub fn load(data_dir: &Path) -> Origins {
    std::fs::read_to_string(list_path(data_dir)).map(|t| Origins::parse(&t)).unwrap_or_default()
}

/// Saves the list.
pub fn save(data_dir: &Path, origins: &Origins) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(list_path(data_dir), origins.to_text())
}

/// Writes the saved document `written` into `file`, the temporary file the shell made beside the original, and
/// flushes it to the disk. The shell's commit then renames it over the original.
pub fn fill(written: &Path, file: &mut std::fs::File) -> std::io::Result<()> {
    let mut from = std::fs::File::open(written)?;
    file.set_len(0)?;
    std::io::copy(&mut from, file)?;
    file.flush()?;
    file.sync_all()
}

/// Puts the saved document `written` in place of the working copy `copy`, once the original holds it: a rename (both
/// are in the app's data folder), or a copy where a rename cannot be done.
pub fn update_copy(written: &Path, copy: &Path) -> std::io::Result<()> {
    if std::fs::rename(written, copy).is_ok() {
        return Ok(());
    }
    std::fs::copy(written, copy).map(|_| ())
}

/// The reason the alert gives when the original holds something else than it did when it was opened or last saved.
pub const CHANGED: &str = "The file was changed by another app since it was opened.";

/// Why the original could not be written, for the alert. `None` is a failure on the way to the shell's file.
pub fn failure_reason(code: Option<DocErrorCode>) -> &'static str {
    match code {
        Some(DocErrorCode::Denied) => "You do not have permission to change the file, or its disk is read-only.",
        Some(DocErrorCode::NotFound) => "The file or its folder is no longer there.",
        Some(DocErrorCode::Stale | DocErrorCode::Revoked) => "The file was replaced or moved since it was opened.",
        _ => "The file could not be written.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("viewer-origin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("opened")).unwrap();
        dir
    }

    /// A file of 10 bytes written at second 100: inode `ino` on device `dev`.
    fn stamp(dev: u64, ino: u64) -> Stamp {
        Stamp { dev, ino, secs: 100, nanos: 5, len: 10 }
    }

    /// Two files named alike in different folders each get a copy; the same file opened again keeps its copy.
    #[test]
    fn files_of_one_name_in_different_folders_get_copies_of_their_own() {
        let mut o = Origins::default();
        let (first, second, third) = (stamp(1, 10), stamp(1, 20), stamp(2, 10));
        assert_eq!(o.place("a.png", Some(&first)), "a.png");
        o.set("a.png", [1; 16], Some(first));
        assert_eq!(o.place("a.png", Some(&second)), "2/a.png", "another a.png does not take the first one's copy");
        o.set("2/a.png", [2; 16], Some(second));
        assert_eq!(o.place("a.png", Some(&third)), "3/a.png", "the same inode number on another disk is another file");
        // the same file again, saved in between (its time and length moved on): its own copy
        let later = Stamp { secs: 200, len: 30, ..second };
        assert_eq!(o.place("a.png", Some(&later)), "2/a.png");
        assert_eq!(o.place("a.png", Some(&first)), "a.png");
        assert_eq!(o.place("b.png", Some(&first)), "b.png", "another name is another copy");
        // a place that was given up is used again
        o.forget("a.png");
        assert_eq!(o.place("a.png", Some(&third)), "a.png");
        // a file that cannot be told apart never takes a copy whose original is known
        assert_eq!(o.place("a.png", None), "a.png");
        o.set("a.png", [3; 16], Some(third));
        assert_eq!(o.place("a.png", None), "3/a.png");
    }

    #[test]
    fn a_copy_of_an_earlier_version_is_taken_to_be_the_same_file() {
        let o = Origins::parse(&format!("{}\ta.png\n", "01".repeat(16)));
        assert_eq!(o.place("a.png", Some(&stamp(1, 10))), "a.png");
    }

    #[test]
    fn a_save_sees_that_the_original_was_changed_since_it_was_opened() {
        let opened = stamp(1, 10);
        assert!(!changed_since(Some(opened), Some(opened)));
        assert!(changed_since(Some(opened), Some(Stamp { secs: 101, ..opened })), "written again");
        assert!(changed_since(Some(opened), Some(Stamp { nanos: 6, ..opened })));
        assert!(changed_since(Some(opened), Some(Stamp { len: 11, ..opened })));
        assert!(changed_since(Some(opened), Some(Stamp { ino: 11, ..opened })), "another file in its place");
        assert!(!changed_since(None, Some(opened)), "nothing is known about a copy of an earlier version");
        assert!(!changed_since(Some(opened), None));
    }

    #[test]
    fn a_stamp_follows_the_file_and_a_save_records_the_new_one() {
        let dir = scratch("stamp");
        let file = dir.join("a.png");
        assert_eq!(Stamp::at(&file), None, "a file that is not there has no stamp");
        std::fs::write(&file, b"as opened").unwrap();
        let opened = Stamp::at(&file).unwrap();
        assert_eq!(Stamp::of(&std::fs::File::open(&file).unwrap()), Some(opened), "the open file and its path agree");
        // replaced, as a save by rename does it: another inode under the same name
        std::fs::write(dir.join("new"), b"replaced!!").unwrap();
        std::fs::rename(dir.join("new"), &file).unwrap();
        let replaced = Stamp::at(&file).unwrap();
        assert!(!opened.same_file(&replaced) && changed_since(Some(opened), Some(replaced)));
        let mut o = Origins::default();
        o.set("a.png", [1; 16], Some(opened));
        let copy = dir.join("opened/a.png");
        assert!(o.restamp(&dir, &copy, Some(replaced)));
        assert!(!o.restamp(&dir, &copy, Some(replaced)), "nothing changed the second time");
        assert_eq!(o.stamp_of(&dir, &copy), Some(replaced));
        assert!(!o.restamp(&dir, &dir.join("opened/none.png"), Some(replaced)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_round_trips_and_bad_lines_are_dropped() {
        let mut o = Origins::default();
        assert_eq!(o.set("a.png", [1; 16], None), None);
        assert_eq!(o.set("2/b c.pdf", [0xab; 16], Some(stamp(7, 3))), None);
        assert_eq!(Origins::parse(&o.to_text()), o);
        let text = format!("nonsense\n{}\t\nzz\tx.png\n{}\tok.png\n", "01".repeat(16), "02".repeat(16));
        assert_eq!(Origins::parse(&text).entries, [Entry { key: "ok.png".into(), grant: [2; 16], stamp: None }], "a line of an earlier version has no stamp");
        // a name of an earlier version's line may hold a tab: what stands before it is not a stamp, so it is the name
        let odd = Origins::parse(&format!("{}\tnot.a.stamp\tx.png\n", "03".repeat(16)));
        assert_eq!(odd.entries[0].key, "not.a.stamp\tx.png");
        assert_eq!(Stamp::parse(&stamp(7, 3).to_text()), Some(stamp(7, 3)));
        assert_eq!(Stamp::parse("1.2.3.4"), None);
        assert_eq!(Stamp::parse("1.2.3.4.5.6"), None);
    }

    #[test]
    fn a_copy_made_again_takes_the_new_grant_and_hands_back_the_old_one() {
        let mut o = Origins::default();
        o.set("a.png", [1; 16], None);
        assert_eq!(o.set("a.png", [1; 16], None), None, "the same grant is still needed");
        assert_eq!(o.set("a.png", [2; 16], None), Some([1; 16]));
        assert_eq!(o.entries.len(), 1);
        assert_eq!(o.forget("a.png"), Some([2; 16]));
        assert_eq!(o.forget("a.png"), None);
    }

    #[test]
    fn only_a_working_copy_has_an_original() {
        let dir = Path::new("/data");
        let mut o = Origins::default();
        o.set("a.png", [7; 16], None);
        o.set("2/a.png", [8; 16], Some(stamp(1, 2)));
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/a.png")), Some([7; 16]));
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/2/a.png")), Some([8; 16]), "the copy of another a.png");
        assert_eq!(o.stamp_of(dir, Path::new("/data/opened/2/a.png")), Some(stamp(1, 2)));
        assert_eq!(o.stamp_of(dir, Path::new("/data/opened/a.png")), None);
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/2/3/a.png")), None);
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/other.png")), None, "a clipboard picture");
        assert_eq!(o.grant_of(dir, Path::new("/home/me/Pictures/a.png")), None, "a file opened by its path");
        assert_eq!(o.grant_of(dir, Path::new("/data/save/a.png")), None);
    }

    #[test]
    fn the_list_is_kept_under_the_data_folder_and_pruned_with_the_copies() {
        let dir = scratch("prune");
        std::fs::write(dir.join("opened/kept.png"), b"x").unwrap();
        let mut o = Origins::default();
        std::fs::create_dir_all(dir.join("opened/2")).unwrap();
        std::fs::write(dir.join("opened/2/kept.png"), b"y").unwrap();
        o.set("kept.png", [1; 16], None);
        o.set("swept.png", [2; 16], None);
        o.set("2/kept.png", [3; 16], Some(stamp(4, 5)));
        save(&dir, &o).unwrap();
        let mut o = load(&dir);
        assert_eq!(o.prune(&dir), [[2; 16]], "the grant of a removed copy is handed back");
        assert_eq!(o.grant_of(&dir, &dir.join("opened/kept.png")), Some([1; 16]));
        assert_eq!(o.grant_of(&dir, &dir.join("opened/2/kept.png")), Some([3; 16]));
        assert_eq!(load(&dir.join("none")), Origins::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The save as the shell runs it: a temporary file beside the original, filled by the app, renamed on commit.
    #[test]
    fn a_save_fills_the_file_beside_the_original_and_then_brings_the_copy_in_step() {
        let dir = scratch("save");
        let documents = dir.join("Documents");
        std::fs::create_dir_all(dir.join("save")).unwrap();
        std::fs::create_dir_all(&documents).unwrap();
        let (original, copy, written) = (documents.join("a.png"), dir.join("opened/a.png"), dir.join("save/a.png"));
        std::fs::write(&original, b"as opened").unwrap();
        std::fs::write(&copy, b"as opened").unwrap();
        std::fs::write(&written, b"rotated").unwrap();

        let temp = documents.join(".a.png.tmp");
        let mut file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&temp).unwrap();
        // something an earlier attempt left in the file goes
        file.write_all(b"a longer earlier attempt").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        fill(&written, &mut file).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"as opened", "nothing is replaced before the commit");
        assert_eq!(std::fs::read(&copy).unwrap(), b"as opened", "the copy waits for the original");
        std::fs::rename(&temp, &original).unwrap();
        update_copy(&written, &copy).unwrap();

        let mut saved = String::new();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_to_string(&mut saved).unwrap();
        assert_eq!(saved, "rotated");
        assert_eq!(std::fs::read(&original).unwrap(), b"rotated", "the person's file has the edit");
        assert_eq!(std::fs::read(&copy).unwrap(), b"rotated", "and so has the copy Open Recent reads");
        assert!(!written.exists(), "the saved file moved into the copy's place");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_cannot_be_read_back_fails_and_leaves_the_file_alone() {
        let dir = scratch("fail");
        let temp = dir.join("temp");
        let mut file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&temp).unwrap();
        file.write_all(b"kept").unwrap();
        assert!(fill(&dir.join("save/none.png"), &mut file).is_err());
        assert_eq!(std::fs::read(&temp).unwrap(), b"kept");
        assert!(update_copy(&dir.join("save/none.png"), &dir.join("opened/a.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_refusal_has_its_reason() {
        assert!(failure_reason(Some(DocErrorCode::Denied)).contains("read-only"));
        assert!(failure_reason(Some(DocErrorCode::NotFound)).contains("no longer there"));
        assert!(failure_reason(Some(DocErrorCode::Stale)).contains("replaced"));
        assert_eq!(failure_reason(None), "The file could not be written.");
    }
}
