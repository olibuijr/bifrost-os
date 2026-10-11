//! The file info sheet and the folder summary: size, item count, kind,
//! dates and location, as rows of (label, value) the sheet draws.

use super::fs::{self, Entry};
use super::tags::{Tag, TagIndex};
use std::path::Path;

/// Totals under a folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub bytes: u64,
    pub files: usize,
    pub folders: usize,
}

impl Summary {
    pub fn items(&self) -> usize {
        self.files + self.folders
    }

    /// "12 items, 4.2 MB"
    pub fn text(&self) -> String {
        let n = self.items();
        format!("{n} {}, {}", if n == 1 { "item" } else { "items" }, fs::size_text(self.bytes))
    }
}

/// Counts `dir` recursively (the folder itself excluded; links are neither
/// followed nor counted). `keep_going` is asked once per folder read and
/// returns false to stop early with what has been counted so far.
pub fn summarize(dir: &Path, keep_going: &mut dyn FnMut() -> bool) -> Summary {
    let mut s = Summary::default();
    walk(dir, &mut s, keep_going, 0);
    s
}

fn walk(dir: &Path, s: &mut Summary, keep_going: &mut dyn FnMut() -> bool, depth: usize) -> bool {
    if depth > super::find::MAX_DEPTH || !keep_going() {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return true };
    for e in rd.flatten() {
        let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
        if md.file_type().is_symlink() {
            continue;
        }
        if md.is_dir() {
            s.folders += 1;
            if !walk(&e.path(), s, keep_going, depth + 1) {
                return false;
            }
        } else {
            s.files += 1;
            s.bytes += md.len();
        }
    }
    true
}

/// The rows of the info sheet for `entry`; `summary` fills Size and Items
/// for folders, `now` and `offset` format the dates.
pub fn rows(entry: &Entry, summary: Option<Summary>, created: Option<i64>, tags: &TagIndex, now: i64, offset: i32) -> Vec<(&'static str, String)> {
    let mut r = vec![("Kind", fs::kind_description(&entry.name, entry.is_dir, entry.kind).to_string())];
    if entry.is_dir {
        match summary {
            Some(s) => {
                r.push(("Size", fs::size_text(s.bytes)));
                r.push(("Items", s.items().to_string()));
            }
            None => r.push(("Size", "Calculating...".into())),
        }
    } else {
        r.push(("Size", format!("{} ({} bytes)", fs::size_text(entry.size), entry.size)));
    }
    r.push(("Where", entry.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()));
    if let Some(c) = created {
        r.push(("Created", fs::date_text(c, now, offset)));
    }
    r.push(("Modified", fs::date_text(entry.mtime, now, offset)));
    let t: Vec<&str> = tags.tags(&entry.path).into_iter().map(Tag::label).collect();
    if !t.is_empty() {
        r.push(("Tags", t.join(", ")));
    }
    r
}

/// Creation time of `path` in Unix seconds, where the file system keeps one.
pub fn created_at(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.created().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn summarises_a_tree() {
        let d = fs::tests::temp("info-sum");
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("a/one"), vec![0u8; 1500]).unwrap();
        std::fs::write(d.join("a/b/two"), vec![0u8; 500]).unwrap();
        std::fs::write(d.join("three"), "x").unwrap();
        let s = summarize(&d, &mut || true);
        assert_eq!(s, Summary { bytes: 2001, files: 3, folders: 2 });
        assert_eq!(s.text(), "5 items, 2 KB");
        assert_eq!(Summary { bytes: 1, files: 1, folders: 0 }.text(), "1 item, 1 byte");
    }

    #[test]
    fn summary_can_be_stopped() {
        let d = fs::tests::temp("info-stop");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::write(d.join("a/f"), "x").unwrap();
        let s = summarize(&d, &mut || false);
        assert_eq!(s, Summary::default());
    }

    #[test]
    fn info_rows_for_file_and_folder() {
        let mut tags = TagIndex::default();
        let f = Entry { name: "n.txt".into(), path: PathBuf::from("/m/Docs/n.txt"), is_dir: false, size: 2500, mtime: 0, kind: fs::Kind::Text, items: None };
        tags.toggle(&f.path, Tag::Red);
        tags.toggle(&f.path, Tag::Blue);
        let r = rows(&f, None, Some(0), &tags, 0, 0);
        let get = |k: &str| r.iter().find(|(l, _)| *l == k).map(|(_, v)| v.clone());
        assert_eq!(get("Kind").as_deref(), Some("Plain Text"));
        assert!(get("Size").unwrap().ends_with("(2500 bytes)"));
        assert_eq!(get("Where").as_deref(), Some("/m/Docs"));
        assert_eq!(get("Tags").as_deref(), Some("Red, Blue"));
        assert!(get("Modified").unwrap().contains(" at "));
        let dir = Entry { is_dir: true, kind: fs::Kind::Folder, ..f };
        assert_eq!(rows(&dir, None, None, &tags, 0, 0)[1].1, "Calculating...");
        let r = rows(&dir, Some(Summary { bytes: 10, files: 2, folders: 0 }), None, &tags, 0, 0);
        assert_eq!(r[2], ("Items", "2".to_string()));
    }
}
