//! The files Search lists: the Files root ("On My PC") and the person's home folder, walked on a worker thread when
//! Search opens (`QuickSearch::refresh_sources`) and reused for a while after. The shell has no file-system metadata
//! index, so this is a bounded name walk: breadth first (shallow files are reached before a deep tree uses up the
//! budget), hidden folders and build or package trees skipped, symbolic links not followed, at most `VISIT_BUDGET`
//! entries looked at, and the newest `LIMIT` files kept.

use crate::files::fs::{kind_of, Entry};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// The most files kept for Search, newest first.
pub const LIMIT: usize = 6000;
/// The most directory entries one walk looks at.
pub const VISIT_BUDGET: usize = 60_000;
/// How deep below a root the walk goes.
const MAX_DEPTH: usize = 8;
/// Folders that hold generated trees rather than the person's files.
const SKIP_DIRS: [&str; 4] = ["node_modules", "target", "__pycache__", "lost+found"];

/// The folders Search walks: the Files root, then `home` unless it is `/` or holds the Files root's tree already.
/// A root inside another root is walked once, as part of the outer one.
pub fn roots(files_root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![files_root.to_path_buf()];
    if let Some(h) = home.filter(|h| h.parent().is_some() && !h.as_os_str().is_empty()) {
        if files_root.starts_with(h) {
            out = vec![h.to_path_buf()];
        } else if !h.starts_with(files_root) {
            out.push(h.to_path_buf());
        }
    }
    out
}

/// The user's home folder (`$HOME`), when it is set.
pub fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// True for a folder the walk does not enter: hidden ones and generated trees.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// The files under `roots`, newest first, at most `limit`, after looking at no more than `budget` entries.
pub fn walk(roots: &[PathBuf], limit: usize, budget: usize) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut seen = 0usize;
    'walk: while let Some((dir, depth)) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > budget {
                break 'walk;
            }
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if depth < MAX_DEPTH && !skipped(&name) {
                    queue.push_back((e.path(), depth + 1));
                }
            } else if ft.is_file() && !name.starts_with('.') {
                let Ok(md) = e.metadata() else { continue };
                let mtime = md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                out.push(Entry { kind: kind_of(&name, false), name, path: e.path(), is_dir: false, size: md.len(), mtime, items: None });
            }
        }
    }
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qs-files-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_home_folder_is_walked_besides_the_files_root_once() {
        let files = Path::new("/var/lib/springboard/Files");
        assert_eq!(roots(files, Some(Path::new("/home/me"))), vec![files.to_path_buf(), PathBuf::from("/home/me")]);
        assert_eq!(roots(files, None), vec![files.to_path_buf()]);
        assert_eq!(roots(files, Some(Path::new("/"))), vec![files.to_path_buf()], "a home of / is not walked");
        assert_eq!(roots(Path::new("/home/me/Files"), Some(Path::new("/home/me"))), vec![PathBuf::from("/home/me")], "the outer root covers the inner");
        assert_eq!(roots(files, Some(Path::new("/var/lib/springboard/Files/me"))), vec![files.to_path_buf()]);
    }

    #[test]
    fn the_walk_finds_nested_files_and_skips_hidden_and_generated_trees() {
        let dir = scratch("walk");
        fs::create_dir_all(dir.join("Documents/Work")).unwrap();
        fs::create_dir_all(dir.join(".cache")).unwrap();
        fs::create_dir_all(dir.join("proj/node_modules/x")).unwrap();
        fs::write(dir.join("Documents/Work/e2e-notes.txt"), "a").unwrap();
        fs::write(dir.join("top.pdf"), "b").unwrap();
        fs::write(dir.join(".hidden"), "c").unwrap();
        fs::write(dir.join(".cache/e2e.bin"), "d").unwrap();
        fs::write(dir.join("proj/node_modules/x/e2e.js"), "e").unwrap();
        let mut names: Vec<String> = walk(&[dir.clone()], 100, 1000).into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, vec!["e2e-notes.txt".to_string(), "top.pdf".to_string()]);
        let first = walk(&[dir.clone()], 100, 1000).into_iter().find(|e| e.name == "top.pdf").unwrap();
        assert!(first.mtime > 0, "the modification time is kept for the row's date");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_budget_and_the_limit_bound_the_walk_shallow_files_first() {
        let dir = scratch("budget");
        fs::create_dir_all(dir.join("a/b/c")).unwrap();
        for i in 0..5 {
            fs::write(dir.join(format!("f{i}.txt")), "x").unwrap();
        }
        fs::write(dir.join("a/b/c/deep.txt"), "x").unwrap();
        let few = walk(&[dir.clone()], 100, 6);
        assert!(few.iter().all(|e| e.name != "deep.txt"), "the budget runs out before the deep file: {few:?}");
        assert_eq!(walk(&[dir.clone()], 2, 1000).len(), 2);
        assert_eq!(walk(&[dir.clone()], 100, 1000).len(), 6);
        let _ = fs::remove_dir_all(&dir);
    }
}
