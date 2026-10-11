//! Folders for notes, persisted as subdirectories of the notes directory.
//! Notes outside any folder live in the root ("All Notes" shows root and
//! every folder). Deleting a note moves it to `.Recently Deleted`, stamping
//! the move time on the file so it can be purged after 30 days. Image
//! attachments sit beside their note as `<id>-<n>.png|jpg` and travel with it.

use super::store::{self, Note};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The hidden directory holding deleted notes.
pub const TRASH: &str = ".Recently Deleted";
/// Deleted notes are purged after this long.
pub const PURGE_SECS: i64 = 30 * 86_400;
const MAX_NAME: usize = 40;

/// A folder name that is safe as a directory: trimmed, no separators or
/// control chars, not hidden, not empty, at most 40 chars.
pub fn valid_name(name: &str) -> Option<String> {
    let clean: String = name.chars().filter(|c| !c.is_control() && !"/\\".contains(*c)).take(MAX_NAME).collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty() && !clean.starts_with('.')).then_some(clean)
}

/// `base`, or `base 2`, `base 3`... when that name is taken (case-insensitive).
pub fn unique_name(existing: &[String], base: &str) -> String {
    let taken = |n: &str| existing.iter().any(|e| e.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
}

/// The directory of `folder` (None is the root).
pub fn folder_dir(dir: &Path, folder: Option<&str>) -> PathBuf {
    folder.map_or_else(|| dir.to_path_buf(), |f| dir.join(f))
}

/// User folders, sorted by name; hidden directories (pins, trash, export) are left out.
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<String> = entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Creates a folder named like `name` (made unique); returns the name used.
pub fn create(dir: &Path, name: &str) -> io::Result<String> {
    let base = valid_name(name).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid folder name"))?;
    let name = unique_name(&list(dir), &base);
    std::fs::create_dir_all(dir.join(&name))?;
    Ok(name)
}

/// Renames a folder; returns the final name. Renaming to the same name
/// (or a different case of it) is allowed, any other clash is an error.
pub fn rename(dir: &Path, old: &str, new: &str) -> io::Result<String> {
    let new = valid_name(new).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid folder name"))?;
    if new != old && list(dir).iter().any(|n| n.eq_ignore_ascii_case(&new) && !n.eq_ignore_ascii_case(old)) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "A folder with that name exists"));
    }
    if new != old {
        std::fs::rename(dir.join(old), dir.join(&new))?;
    }
    Ok(new)
}

/// Attachment files of note `id` in `dir`: `<id>-<digits>.<png|jpg|jpeg>`.
pub fn attachments(dir: &Path, id: &str) -> Vec<String> {
    let prefix = format!("{id}-");
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.strip_prefix(&prefix).and_then(|rest| rest.split_once('.')).is_some_and(|(num, ext)| !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) && ["png", "jpg", "jpeg"].contains(&ext.to_ascii_lowercase().as_str()))
        })
        .collect();
    names.sort();
    names
}

/// The next free attachment name for `id` with extension `ext`.
pub fn next_attachment(dir: &Path, id: &str, ext: &str) -> String {
    let used = attachments(dir, id).len();
    (used + 1..).map(|n| format!("{id}-{n}.{ext}")).find(|n| !dir.join(n).exists()).unwrap()
}

/// Moves a note file and its attachments between directories.
fn move_files(from: &Path, to: &Path, id: &str) -> io::Result<()> {
    let md = format!("{id}.md");
    if !from.join(&md).exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "No such note"));
    }
    if to.join(&md).exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Note exists in destination"));
    }
    std::fs::create_dir_all(to)?;
    for name in attachments(from, id) {
        std::fs::rename(from.join(&name), to.join(&name))?;
    }
    std::fs::rename(from.join(&md), to.join(&md))
}

/// Moves note `id` from one folder to another (None is the root).
pub fn move_note(dir: &Path, id: &str, from: Option<&str>, to: Option<&str>) -> io::Result<()> {
    if from == to {
        return Ok(());
    }
    move_files(&folder_dir(dir, from), &folder_dir(dir, to), id)
}

fn stamp(path: &Path, unix: i64) -> io::Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_modified(UNIX_EPOCH + Duration::from_secs(unix.max(0) as u64))
}

/// Moves a note to Recently Deleted and stamps the deletion time.
pub fn trash(dir: &Path, id: &str, from: Option<&str>, now: i64) -> io::Result<()> {
    let to = dir.join(TRASH);
    move_files(&folder_dir(dir, from), &to, id)?;
    stamp(&to.join(format!("{id}.md")), now)
}

/// Brings a deleted note back to the root.
pub fn restore(dir: &Path, id: &str) -> io::Result<()> {
    move_files(&dir.join(TRASH), dir, id)
}

/// Permanently removes a note (and its attachments) from `folder`.
pub fn erase(dir: &Path, id: &str, folder: Option<&str>) {
    let base = folder_dir(dir, folder);
    for name in attachments(&base, id) {
        let _ = std::fs::remove_file(base.join(name));
    }
    let _ = std::fs::remove_file(base.join(format!("{id}.md")));
}

/// Copies a note's attachments under `new_id` in the same folder and returns
/// the text with its image references pointing at the copies. The caller
/// writes the copy's `.md` file.
pub fn duplicate(dir: &Path, id: &str, new_id: &str, folder: Option<&str>, text: &str) -> io::Result<String> {
    let base = folder_dir(dir, folder);
    let mut text = text.to_string();
    for name in attachments(&base, id) {
        let Some(rest) = name.strip_prefix(id) else { continue };
        let renamed = format!("{new_id}{rest}");
        std::fs::copy(base.join(&name), base.join(&renamed))?;
        text = text.replace(&format!("({name})"), &format!("({renamed})"));
    }
    Ok(text)
}

/// Deletes a folder: its notes go to Recently Deleted.
pub fn delete_folder(dir: &Path, name: &str, now: i64) -> io::Result<()> {
    for note in store::load(&dir.join(name)) {
        trash(dir, &note.id, Some(name), now)?;
    }
    std::fs::remove_dir_all(dir.join(name))
}

/// Permanently removes deleted notes older than 30 days; returns how many.
pub fn purge(dir: &Path, now: i64) -> usize {
    let trash = dir.join(TRASH);
    let mut gone = 0;
    for note in store::load(&trash) {
        if note.modified > 0 && now - note.modified > PURGE_SECS {
            erase(dir, &note.id, Some(TRASH));
            gone += 1;
        }
    }
    gone
}

/// Every note with the folder it lives in (None for the root), in
/// `store::sort` order; trash is not included.
pub fn load_all(dir: &Path) -> Vec<(Option<String>, Note)> {
    let mut all: Vec<(Option<String>, Note)> = store::load(dir).into_iter().map(|n| (None, n)).collect();
    for f in list(dir) {
        all.extend(store::load(&dir.join(&f)).into_iter().map(|n| (Some(f.clone()), n)));
    }
    all
}

/// Seconds since the epoch for a file time.
pub fn unix(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes-folders")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, id: &str, text: &str) {
        store::save(dir, &Note { id: id.into(), text: text.into(), modified: 0 }).unwrap();
    }

    #[test]
    fn names_are_sanitised_and_made_unique() {
        assert_eq!(valid_name("  Work  ").as_deref(), Some("Work"));
        assert_eq!(valid_name("a/b\\c").as_deref(), Some("abc"));
        assert_eq!(valid_name(".hidden"), None);
        assert_eq!(valid_name("  "), None);
        assert_eq!(valid_name(&"x".repeat(80)).unwrap().len(), 40);
        let have = vec!["New Folder".to_string(), "new folder 2".to_string()];
        assert_eq!(unique_name(&have, "New Folder"), "New Folder 3");
        assert_eq!(unique_name(&have, "Other"), "Other");
    }

    #[test]
    fn create_list_rename_and_hidden_dirs() {
        let dir = scratch("crud");
        assert_eq!(create(&dir, "Work").unwrap(), "Work");
        assert_eq!(create(&dir, "Work").unwrap(), "Work 2");
        assert!(create(&dir, "...").is_err());
        std::fs::create_dir_all(dir.join(".springboard-pins-v1")).unwrap();
        std::fs::create_dir_all(dir.join(TRASH)).unwrap();
        assert_eq!(list(&dir), vec!["Work".to_string(), "Work 2".to_string()]);
        assert_eq!(rename(&dir, "Work", "Home").unwrap(), "Home");
        assert!(rename(&dir, "Work 2", "home").is_err(), "clash");
        assert_eq!(rename(&dir, "Home", "Home").unwrap(), "Home");
        assert!(rename(&dir, "Home", "").is_err());
        assert_eq!(list(&dir), vec!["Home".to_string(), "Work 2".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_a_note_carries_its_attachments() {
        let dir = scratch("move");
        create(&dir, "Work").unwrap();
        put(&dir, "100", "A");
        std::fs::write(dir.join("100-1.png"), b"x").unwrap();
        std::fs::write(dir.join("1000-1.png"), b"other note").unwrap();
        assert_eq!(attachments(&dir, "100"), vec!["100-1.png".to_string()]);
        assert_eq!(next_attachment(&dir, "100", "png"), "100-2.png");
        move_note(&dir, "100", None, Some("Work")).unwrap();
        assert!(dir.join("Work/100.md").exists() && dir.join("Work/100-1.png").exists());
        assert!(!dir.join("100.md").exists() && !dir.join("100-1.png").exists());
        assert!(dir.join("1000-1.png").exists(), "another note's file stays");
        assert!(move_note(&dir, "100", None, Some("Work")).is_err(), "no longer in the root");
        move_note(&dir, "100", Some("Work"), None).unwrap();
        assert!(dir.join("100.md").exists());
        assert_eq!(load_all(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_notes_wait_thirty_days_then_purge() {
        let dir = scratch("trash");
        create(&dir, "Work").unwrap();
        put(&dir.join("Work"), "1", "old");
        put(&dir, "2", "recent");
        put(&dir, "3", "kept");
        std::fs::write(dir.join("Work/1-1.jpg"), b"x").unwrap();
        let now = 1_791_073_380;
        trash(&dir, "1", Some("Work"), now - 31 * 86_400).unwrap();
        trash(&dir, "2", None, now - 29 * 86_400).unwrap();
        assert!(dir.join(TRASH).join("1-1.jpg").exists());
        assert_eq!(store::load(&dir.join(TRASH)).len(), 2);
        assert_eq!(store::load(&dir.join(TRASH)).iter().find(|n| n.id == "2").unwrap().modified, now - 29 * 86_400);
        assert_eq!(purge(&dir, now), 1);
        let left = store::load(&dir.join(TRASH));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "2");
        assert!(!dir.join(TRASH).join("1-1.jpg").exists(), "attachments go too");
        restore(&dir, "2").unwrap();
        assert_eq!(store::load(&dir).len(), 2);
        assert_eq!(purge(&dir, now), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_folder_trashes_its_notes() {
        let dir = scratch("rmdir");
        create(&dir, "Work").unwrap();
        put(&dir.join("Work"), "7", "x");
        put(&dir.join("Work"), "8", "y");
        delete_folder(&dir, "Work", 5_000_000).unwrap();
        assert!(list(&dir).is_empty());
        assert_eq!(store::load(&dir.join(TRASH)).len(), 2);
        erase(&dir, "7", Some(TRASH));
        assert_eq!(store::load(&dir.join(TRASH)).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
