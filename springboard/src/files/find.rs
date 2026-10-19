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
    /// folders that are listed but never entered
    skip: Vec<PathBuf>,
    /// the most hits the walk returns; it stops once they are found
    cap: usize,
    /// hits returned so far
    found: usize,
    /// folders read so far
    pub scanned: usize,
}

impl FileManager {
    pub fn new(root: &Path, query: &str, filter: TypeFilter) -> FileManager {
        FileManager::over(&[root.to_path_buf()], &[], query, filter, usize::MAX)
    }

    /// A search over several `roots`, read in order (the first is finished before the second starts). The `skip`
    /// folders show up as hits when their name matches but are not entered, and the walk stops at `cap` hits.
    pub fn over(roots: &[PathBuf], skip: &[PathBuf], query: &str, filter: TypeFilter, cap: usize) -> FileManager {
        // the pending list is a stack: the first root goes on last, so it pops first
        let pending = roots.iter().rev().map(|r| (r.clone(), 0)).collect();
        FileManager { pending, query: query.trim().to_string(), filter, skip: skip.to_vec(), cap, found: 0, scanned: 0 }
    }

    pub fn done(&self) -> bool {
        self.pending.is_empty() || self.found >= self.cap
    }

    /// True once the walk has returned as many hits as it was allowed.
    pub fn capped(&self) -> bool {
        self.found >= self.cap
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
            if self.found >= self.cap {
                self.pending.clear();
                break;
            }
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
                if is_dir && !link.file_type().is_symlink() && depth < MAX_DEPTH && !self.skip.contains(&path) {
                    kids.push((path.clone(), depth + 1));
                }
                let kind = fs::kind_of(&name, is_dir);
                if self.found < self.cap && self.filter.accepts(kind) && name_matches(&name, &self.query) {
                    self.found += 1;
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
    fn several_roots_are_read_in_order_and_skipped_folders_are_not_entered() {
        let d = tree("find-over");
        let other = fs::tests::temp("find-over-other");
        std::fs::write(other.join("beach hut.txt"), "x").unwrap();
        let mut f = FileManager::over(&[d.clone(), other.clone()], &[d.join("Trips/Spain")], "beach", TypeFilter::All, usize::MAX);
        let mut first = vec![];
        // the first root is finished before the second is opened: its hits come before "beach hut.txt"
        while !f.done() && !first.iter().any(|e: &Entry| e.name == "beach notes.txt") {
            first.extend(f.step(1));
        }
        assert!(!first.iter().any(|e| e.name == "beach hut.txt"), "the second root has not been read yet");
        let mut all = first;
        while !f.done() {
            all.extend(f.step(1));
        }
        let mut got = names(&all);
        got.sort();
        // Spain is not entered, so its beach.png is not found; "beach hut.txt" comes from the second root
        assert_eq!(got, ["Beach Plan.md", "beach hut.txt", "beach notes.txt"]);
        // a skipped folder is still listed when its own name matches
        let listed = find_all_over(&[d.clone()], &[d.join("Trips/Spain")], "spain");
        assert_eq!(names(&listed), ["Spain"]);
    }

    /// Runs a whole search over `roots` at once.
    fn find_all_over(roots: &[PathBuf], skip: &[PathBuf], query: &str) -> Vec<Entry> {
        let mut f = FileManager::over(roots, skip, query, TypeFilter::All, usize::MAX);
        let mut out = vec![];
        while !f.done() {
            out.extend(f.step(16));
        }
        out
    }

    #[test]
    fn the_walk_stops_at_the_result_cap() {
        let d = fs::tests::temp("find-cap");
        for i in 0..10 {
            std::fs::write(d.join(format!("beach {i}.txt")), "x").unwrap();
        }
        let mut f = FileManager::over(&[d.clone()], &[], "beach", TypeFilter::All, 4);
        let mut out = vec![];
        while !f.done() {
            out.extend(f.step(16));
        }
        assert_eq!(out.len(), 4, "no more than the cap");
        assert!(f.capped());
        // under the cap the walk finishes without reaching it
        let mut f = FileManager::over(&[d], &[], "beach", TypeFilter::All, 50);
        let mut n = 0;
        while !f.done() {
            n += f.step(16).len();
        }
        assert_eq!(n, 10);
        assert!(!f.capped());
    }

    #[test]
    fn relative_path_for_subtitle() {
        let d = tree("find-rel");
        let hits = find_all(&d, "spain", TypeFilter::Folders);
        assert_eq!(relative_to(&d, &hits[0]), Path::new("Trips/Spain"));
    }
}
