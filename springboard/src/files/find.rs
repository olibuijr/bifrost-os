//! Search by name across the current folder tree. A `Files` walks the tree
//! in small steps so the UI can show results as they arrive and stop at any
//! time; type filters narrow the hits.

use super::fs::{self, Entry, Kind};
use std::path::{Path, PathBuf};

/// Deepest folder level searched (guards against pathological trees).
pub const MAX_DEPTH: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Folders,
    Documents,
    Images,
    Video,
    Audio,
    Archives,
}

impl TypeFilter {
    pub const ALL: [TypeFilter; 7] = [TypeFilter::All, TypeFilter::Folders, TypeFilter::Documents, TypeFilter::Images, TypeFilter::Video, TypeFilter::Audio, TypeFilter::Archives];

    pub fn label(self) -> &'static str {
        match self {
            TypeFilter::All => "All",
            TypeFilter::Folders => "Folders",
            TypeFilter::Documents => "Documents",
            TypeFilter::Images => "Images",
            TypeFilter::Video => "Video",
            TypeFilter::Audio => "Audio",
            TypeFilter::Archives => "Archives",
        }
    }

    pub fn accepts(self, kind: Kind) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Folders => kind == Kind::Folder,
            TypeFilter::Documents => matches!(kind, Kind::Text | Kind::Other),
            TypeFilter::Images => kind == Kind::Image,
            TypeFilter::Video => kind == Kind::Video,
            TypeFilter::Audio => kind == Kind::Audio,
            TypeFilter::Archives => kind == Kind::Archive,
        }
    }
}

/// Whether `name` matches a search query: every word must appear, ignoring case.
pub fn name_matches(name: &str, query: &str) -> bool {
    let n = name.to_lowercase();
    query.split_whitespace().all(|w| n.contains(&w.to_lowercase()))
}

pub struct FileManager {
    pending: Vec<(PathBuf, usize)>,
    query: String,
    filter: TypeFilter,
    /// folders read so far
    pub scanned: usize,
}

impl FileManager {
    pub fn new(root: &Path, query: &str, filter: TypeFilter) -> FileManager {
        FileManager { pending: vec![(root.to_path_buf(), 0)], query: query.trim().to_string(), filter, scanned: 0 }
    }

    pub fn done(&self) -> bool {
        self.pending.is_empty()
    }

    /// Reads up to `folders` folders and returns the new matches. Symlinked
    /// folders are not followed and the trash folder is skipped. An empty
    /// query matches nothing (the caller shows the plain listing).
    pub fn step(&mut self, folders: usize) -> Vec<Entry> {
        let mut hits = vec![];
        if self.query.is_empty() {
            self.pending.clear();
            return hits;
        }
        for _ in 0..folders {
            let Some((dir, depth)) = self.pending.pop() else { break };
            self.scanned += 1;
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut kids = vec![];
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == super::trash::DIR || name == super::tags::SIDECAR {
                    continue;
                }
                let path = e.path();
                let Ok(link) = std::fs::symlink_metadata(&path) else { continue };
                let md = if link.file_type().is_symlink() { std::fs::metadata(&path).unwrap_or(link.clone()) } else { link.clone() };
                let is_dir = md.is_dir();
                if is_dir && !link.file_type().is_symlink() && depth < MAX_DEPTH {
                    kids.push((path.clone(), depth + 1));
                }
                let kind = fs::kind_of(&name, is_dir);
                if self.filter.accepts(kind) && name_matches(&name, &self.query) {
                    let mtime = md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                    hits.push(Entry { name, path, is_dir, size: if is_dir { 0 } else { md.len() }, mtime, kind, items: None });
                }
            }
            // keep a stable breadth-ish order: sorted so the first folder pops first
            kids.sort_by(|a, b| b.0.cmp(&a.0));
            self.pending.extend(kids);
        }
        hits.sort_by(|a, b| fs::natural_cmp(&a.name, &b.name));
        hits
    }
}

/// Runs a whole search at once (tests and small trees).
pub fn find_all(root: &Path, query: &str, filter: TypeFilter) -> Vec<Entry> {
    let mut f = FileManager::new(root, query, filter);
    let mut out = vec![];
    while !f.done() {
        out.extend(f.step(16));
    }
    out
}

/// The path of `entry` relative to the folder searched, for the row's subtitle.
pub fn relative_to<'a>(root: &Path, entry: &'a Entry) -> &'a Path {
    entry.path.strip_prefix(root).unwrap_or(&entry.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let d = fs::tests::temp(tag);
        std::fs::create_dir_all(d.join("Trips/Spain")).unwrap();
        std::fs::create_dir_all(d.join("Docs")).unwrap();
        std::fs::create_dir_all(d.join(".RecentlyDeleted")).unwrap();
        std::fs::write(d.join("Trips/Spain/beach.png"), [1]).unwrap();
        std::fs::write(d.join("Trips/beach notes.txt"), "x").unwrap();
        std::fs::write(d.join("Docs/Beach Plan.md"), "x").unwrap();
        std::fs::write(d.join("Docs/song.mp3"), "x").unwrap();
        std::fs::write(d.join(".RecentlyDeleted/1__beach.txt"), "x").unwrap();
        d
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn matches_all_words_ignoring_case() {
        assert!(name_matches("Beach Plan.md", "plan beach"));
        assert!(!name_matches("Beach Plan.md", "plan sea"));
        assert!(name_matches("anything", "  "));
    }

    #[test]
    fn finds_across_the_tree_without_the_trash() {
        let d = tree("find-all");
        let all = find_all(&d, "beach", TypeFilter::All);
        let mut got = names(&all);
        got.sort();
        assert_eq!(got, ["Beach Plan.md", "beach notes.txt", "beach.png"]);
    }

    #[test]
    fn type_filters_narrow() {
        let d = tree("find-filter");
        assert_eq!(names(&find_all(&d, "beach", TypeFilter::Images)), ["beach.png"]);
        assert_eq!(names(&find_all(&d, "beach", TypeFilter::Documents)).len(), 2);
        assert_eq!(names(&find_all(&d, "spain", TypeFilter::Folders)), ["Spain"]);
        assert_eq!(names(&find_all(&d, "song", TypeFilter::Audio)), ["song.mp3"]);
        assert!(find_all(&d, "song", TypeFilter::Video).is_empty());
    }

    #[test]
    fn results_arrive_incrementally() {
        let d = tree("find-step");
        let mut f = FileManager::new(&d, "beach", TypeFilter::All);
        let first = f.step(1);
        assert!(!f.done());
        assert_eq!(f.scanned, 1);
        let mut total = first.len();
        while !f.done() {
            total += f.step(1).len();
        }
        assert_eq!(total, 3);
        assert!(FileManager::new(&d, "", TypeFilter::All).step(5).is_empty());
    }

    #[test]
    fn relative_path_for_subtitle() {
        let d = tree("find-rel");
        let hits = find_all(&d, "spain", TypeFilter::Folders);
        assert_eq!(relative_to(&d, &hits[0]), Path::new("Trips/Spain"));
    }
}
