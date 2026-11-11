//! The files and folders Search lists: the Files root ("On My PC") and the person's home folder (`search_home`), walked
//! on a worker thread when Search opens (`QuickSearch::refresh_sources`) and reused for a while after. The shell has no
//! file-system metadata index, so this is a bounded name walk: breadth first (shallow entries are reached before a deep
//! tree uses up the budget), hidden folders and build or package trees skipped (neither entered nor listed), symbolic
//! links not followed, at most `VISIT_BUDGET` entries looked at, and the newest `LIMIT` files and folders kept. The
//! roots themselves are not listed.

use crate::files::fs::{kind_of, Entry};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// The most files and folders kept for Search, newest first.
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

/// Where the volumes mounted for the account are (`/run/media/<account>/<volume>`).
pub const VOLUMES_ROOT: &str = "/run/media";

/// True when `path` is in a place Search lists files from: under one of the walk's `roots` (the Files root, the
/// account's home) or under a mounted volume. A file anywhere else on the disk (the system's own trees) is never a
/// result, whichever app indexed it. A path with a `..` in it is in no place: nothing is resolved here.
pub fn in_listed_place(path: &Path, roots: &[PathBuf]) -> bool {
    let plain = path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
    plain && (path.starts_with(VOLUMES_ROOT) || roots.iter().any(|r| path.starts_with(r)))
}

/// The home folder Search walks: the installed account's home, else `$HOME`. The desktop shell runs as root from a
/// system service that sets no `$HOME`, so the variable alone never named the person's home there and no file under
/// it was listed (E2E5: "file-37" found nothing). Read on the walk's worker thread at every scan, so an account made
/// after the shell started is picked up.
///
/// None in the shell's own tests, whatever the machine has: a test never walks the home folder of the machine it runs
/// on (its files would come in as results, and the walk would make the tests slow and dependent on that folder). A
/// test that wants a home folder walked gives the panel one (`QuickSearch::home`).
pub fn search_home() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    #[cfg(feature = "desktop-wayland")]
    let account = crate::bare::app_policy::Account::installed().ok().map(|a| a.home);
    #[cfg(not(feature = "desktop-wayland"))]
    let account = None;
    pick_home(account, std::env::var_os("HOME").map(PathBuf::from))
}

/// The home folder from its sources, best first: the account's, else `$HOME`; an empty path counts as none.
pub fn pick_home(account: Option<PathBuf>, env: Option<PathBuf>) -> Option<PathBuf> {
    account.filter(|p| !p.as_os_str().is_empty()).or_else(|| env.filter(|p| !p.as_os_str().is_empty()))
}

/// True for a folder the walk does not enter: hidden ones and generated trees.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// The files and folders under `roots`, newest first, at most `limit`, after looking at no more than `budget` entries.
/// A folder the walk does not enter because of its name (`skipped`) is not listed either; one it does not enter because
/// it is too deep is.
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
            let is_dir = ft.is_dir();
            // neither a link nor a special file; no hidden file, and no folder the walk stays out of
            let left_out = if is_dir { skipped(&name) } else { !ft.is_file() || name.starts_with('.') };
            if left_out {
                continue;
            }
            if is_dir && depth < MAX_DEPTH {
                queue.push_back((e.path(), depth + 1));
            }
            let Ok(md) = e.metadata() else { continue };
            let mtime = md.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            out.push(Entry { kind: kind_of(&name, is_dir), name, path: e.path(), is_dir, size: if is_dir { 0 } else { md.len() }, mtime, items: None });
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
    fn the_account_home_is_walked_whatever_the_shell_has_for_home() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(pick_home(p("/home/me"), None), p("/home/me"), "a root shell with no $HOME still walks the account's home");
        assert_eq!(pick_home(p("/home/me"), p("/root")), p("/home/me"), "the account's home, not the shell's own");
        assert_eq!(pick_home(None, p("/home/dev")), p("/home/dev"), "no account (a host preview): $HOME");
        assert_eq!(pick_home(p(""), p("")), None);
        assert_eq!(pick_home(None, None), None);
    }

    /// E2E5 1: forty files two folders below the home, made minutes ago, are each found by name.
    #[test]
    fn a_folder_of_many_new_files_two_levels_down_is_listed_whole() {
        let home = scratch("many");
        fs::create_dir_all(home.join("E2E/many")).unwrap();
        fs::write(home.join("E2E/note.txt"), "n").unwrap();
        for i in 1..=40 {
            fs::write(home.join(format!("E2E/many/file-{i:02}.txt")), "x").unwrap();
        }
        let found = walk(&roots(Path::new("/nonexistent/Files"), Some(&home)), LIMIT, VISIT_BUDGET);
        assert_eq!(found.iter().filter(|e| !e.is_dir).count(), 41, "{found:?}");
        let hit = found.iter().find(|e| e.name == "file-37.txt").expect("file-37.txt is listed");
        assert_eq!(hit.path, home.join("E2E/many/file-37.txt"));
        let _ = fs::remove_dir_all(&home);
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
        let mut names: Vec<String> = walk(&[dir.clone()], 100, 1000).into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect();
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
        assert!(few.len() <= 6, "folders count against the budget like files: {few:?}");
        assert_eq!(walk(&[dir.clone()], 2, 1000).len(), 2);
        // five files, the deep one, and the three folders on the way to it
        assert_eq!(walk(&[dir.clone()], 100, 1000).len(), 9);
        let _ = fs::remove_dir_all(&dir);
    }

    /// E2E6 4: "pictures" did not find the home folder "Pictures": the walk kept files only.
    #[test]
    fn folders_are_listed_by_name_within_the_same_bounds() {
        let home = scratch("folders");
        fs::create_dir_all(home.join("Pictures/Trips")).unwrap();
        fs::create_dir_all(home.join(".config/Pictures")).unwrap();
        fs::create_dir_all(home.join("proj/target/Pictures")).unwrap();
        fs::write(home.join("Pictures/pic.png"), "p").unwrap();
        let found = walk(&[home.clone()], LIMIT, VISIT_BUDGET);
        let folders: Vec<&Entry> = found.iter().filter(|e| e.is_dir).collect();
        let mut paths: Vec<PathBuf> = folders.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        assert_eq!(paths, vec![home.join("Pictures"), home.join("Pictures/Trips"), home.join("proj")], "the root, hidden and generated trees are not listed");
        let pictures = folders.iter().find(|e| e.name == "Pictures").unwrap();
        assert_eq!(pictures.kind, crate::files::fs::Kind::Folder);
        assert!(pictures.mtime > 0 && pictures.size == 0);
        assert!(found.iter().any(|e| !e.is_dir && e.name == "pic.png"), "files are listed as before");
        // a folder at the depth limit is listed, though not entered
        let mut deep = home.clone();
        for i in 0..=MAX_DEPTH {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(deep.join("below")).unwrap();
        let found = walk(&[home.clone()], LIMIT, VISIT_BUDGET);
        assert!(found.iter().any(|e| e.path == deep), "the deepest folder looked at is listed");
        assert!(found.iter().all(|e| e.path != deep.join("below")), "nothing below the depth limit is");
        // a link to a folder is neither followed nor listed
        std::os::unix::fs::symlink(home.join("Pictures"), home.join("Link")).unwrap();
        assert!(walk(&[home.clone()], LIMIT, VISIT_BUDGET).iter().all(|e| e.name != "Link"));
        let _ = fs::remove_dir_all(&home);
    }

    /// E2E7 2: "color.jpg" of /usr/share/cups/ipptool was listed, a file the Files app had indexed when it was opened.
    #[test]
    fn only_the_home_the_files_root_and_mounted_volumes_are_places_search_lists_from() {
        let roots = roots(Path::new("/var/lib/springboard/Files"), Some(Path::new("/home/me")));
        assert!(in_listed_place(Path::new("/home/me/Pictures/pic.png"), &roots));
        assert!(in_listed_place(Path::new("/var/lib/springboard/Files/a.txt"), &roots));
        assert!(in_listed_place(Path::new("/run/media/me/Stick/photo.jpg"), &roots), "a mounted volume");
        assert!(!in_listed_place(Path::new("/usr/share/cups/ipptool/color.jpg"), &roots), "a system tree");
        assert!(!in_listed_place(Path::new("/home/meg/secret.txt"), &roots), "another home that starts with the same letters");
        assert!(!in_listed_place(Path::new("/home/me/../../etc/passwd"), &roots), "nothing is resolved, so a path that climbs is out");
        assert!(!in_listed_place(Path::new("Pictures/pic.png"), &roots));
        assert!(!in_listed_place(Path::new("/home/me/a.txt"), &[]), "before the first scan no place is known");
    }

    /// The tests never walk the home folder of the machine they run on, whatever `$HOME` or its accounts say.
    #[test]
    fn no_home_folder_is_looked_up_in_tests() {
        assert_eq!(search_home(), None);
    }
}
