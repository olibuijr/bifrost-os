//! Where a working copy came from. A file chosen in the open panel, or opened with the app from Files, arrives as an
//! open file with a grant, never as a path; the app works on a copy under `opened/` (`recents`). File > Save has to
//! reach the person's own file, so the grant of each copy is kept here, one `grant<TAB>copy name` per line under the
//! app's data directory, and a save goes through it: the shell hands over a temporary file beside the original and
//! renames it into place on commit (`Ctx::save_document_in_place`). The copy is then written too, so Open Recent and
//! Revert to Saved read what was saved.

use bifrost_app::proto::{DocErrorCode, GrantId};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The grants of the working copies, by the copy's file name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Origins {
    entries: Vec<(String, GrantId)>,
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

impl Origins {
    /// Reads the list from its text. Lines that are not `grant<TAB>name` are dropped.
    pub fn parse(text: &str) -> Origins {
        let mut origins = Origins::default();
        for line in text.lines() {
            let Some((grant, name)) = line.split_once('\t') else { continue };
            if let (Some(grant), false) = (unhex(grant), name.is_empty()) {
                origins.set(name, grant);
            }
        }
        origins
    }

    pub fn to_text(&self) -> String {
        self.entries.iter().map(|(name, grant)| format!("{}\t{name}\n", hex(grant))).collect()
    }

    /// Notes that the copy named `name` was made from the file `grant` names. Returns the grant the name had before,
    /// when it was another one: the app no longer needs it.
    pub fn set(&mut self, name: &str, grant: GrantId) -> Option<GrantId> {
        let old = self.forget(name);
        self.entries.push((name.to_string(), grant));
        old.filter(|old| *old != grant)
    }

    /// Forgets the copy named `name` (it was replaced by a file that has no original, or removed). Returns its grant.
    pub fn forget(&mut self, name: &str) -> Option<GrantId> {
        let at = self.entries.iter().position(|(n, _)| n == name)?;
        Some(self.entries.remove(at).1)
    }

    /// The grant of the original of the working copy at `path`; None for a file that is not a working copy (a
    /// clipboard picture, a screenshot) or whose original is not known.
    pub fn grant_of(&self, data_dir: &Path, path: &Path) -> Option<GrantId> {
        if path.parent() != Some(data_dir.join("opened").as_path()) {
            return None;
        }
        let name = path.file_name()?.to_str()?;
        self.entries.iter().find(|(n, _)| n == name).map(|(_, grant)| *grant)
    }

    /// Drops the entries whose copy is gone (`recents::sweep_copies` removed it) and returns their grants.
    pub fn prune(&mut self, data_dir: &Path) -> Vec<GrantId> {
        let dir = data_dir.join("opened");
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries).into_iter().partition(|(name, _)| dir.join(name).exists());
        self.entries = kept;
        gone.into_iter().map(|(_, grant)| grant).collect()
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

    #[test]
    fn the_list_round_trips_and_bad_lines_are_dropped() {
        let mut o = Origins::default();
        assert_eq!(o.set("a.png", [1; 16]), None);
        assert_eq!(o.set("b c.pdf", [0xab; 16]), None);
        assert_eq!(Origins::parse(&o.to_text()), o);
        let text = format!("nonsense\n{}\t\nzz\tx.png\n{}\tok.png\n", "01".repeat(16), "02".repeat(16));
        assert_eq!(Origins::parse(&text).entries, [("ok.png".to_string(), [2; 16])]);
    }

    #[test]
    fn a_copy_made_again_takes_the_new_grant_and_hands_back_the_old_one() {
        let mut o = Origins::default();
        o.set("a.png", [1; 16]);
        assert_eq!(o.set("a.png", [1; 16]), None, "the same grant is still needed");
        assert_eq!(o.set("a.png", [2; 16]), Some([1; 16]));
        assert_eq!(o.entries.len(), 1);
        assert_eq!(o.forget("a.png"), Some([2; 16]));
        assert_eq!(o.forget("a.png"), None);
    }

    #[test]
    fn only_a_working_copy_has_an_original() {
        let dir = Path::new("/data");
        let mut o = Origins::default();
        o.set("a.png", [7; 16]);
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/a.png")), Some([7; 16]));
        assert_eq!(o.grant_of(dir, Path::new("/data/opened/other.png")), None, "a clipboard picture");
        assert_eq!(o.grant_of(dir, Path::new("/home/me/Pictures/a.png")), None, "a file opened by its path");
        assert_eq!(o.grant_of(dir, Path::new("/data/save/a.png")), None);
    }

    #[test]
    fn the_list_is_kept_under_the_data_folder_and_pruned_with_the_copies() {
        let dir = scratch("prune");
        std::fs::write(dir.join("opened/kept.png"), b"x").unwrap();
        let mut o = Origins::default();
        o.set("kept.png", [1; 16]);
        o.set("swept.png", [2; 16]);
        save(&dir, &o).unwrap();
        let mut o = load(&dir);
        assert_eq!(o.prune(&dir), [[2; 16]], "the grant of a removed copy is handed back");
        assert_eq!(o.grant_of(&dir, &dir.join("opened/kept.png")), Some([1; 16]));
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
