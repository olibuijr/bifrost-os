//! The file importer daemon: keeps the person's Documents, Desktop and Downloads folders in the Core Search index.
//!
//! The importer plug-ins (`bifrost_ui::mdimport`) read a file into a title, description and keywords, and
//! `spotindex::reindex_folder` turns a folder into searchable items. This module is the caller: `Importer::poll` runs
//! from the shell's tick and, when the folders are due for a look, starts one background thread that
//!
//! 1. computes a cheap change signature of each folder (names, sizes and modification times, bounded by the same depth
//!    and file limits as the scan; no file is opened), and
//! 2. re-imports the folders whose signature differs from the last one it saw. The first look at start-up sees no
//!    earlier signature, so every folder is imported once; later looks import only what changed.
//!
//! Everything slow runs on that thread: the tick only checks a timer and a flag, so the draw path never waits on the
//! disk. Only one look runs at a time. The items belong to the Files app (results open there) and are bounded: each
//! folder gets a third of the per-app index budget, no folder is entered deeper than `SCAN_MAX_DEPTH`, hidden names and
//! symbolic links are skipped, and nothing leaves the machine.
use super::spotindex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The app the imported files are indexed under (Search opens a result in it).
pub const APP_ID: &str = "is.olibuijr.files.desktop";
/// The folders under the home directory that are imported, with the domain their items carry.
pub const FOLDERS: [&str; 3] = ["Documents", "Desktop", "Downloads"];
/// How long the shell waits between two looks at the folders.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);
/// The files one folder may put in the index: a third of the per-app budget.
pub const PER_FOLDER: usize = spotindex::SCAN_MAX_FILES / FOLDERS.len();
/// The entries (files and folders) one signature walk looks at before it stops.
pub const SIGNATURE_MAX_ENTRIES: usize = spotindex::SCAN_MAX_FILES * 8;

/// The folders to import under `home`: (domain, path).
pub fn roots(home: &Path) -> Vec<(&'static str, PathBuf)> {
    FOLDERS.iter().map(|name| (*name, home.join(name))).collect()
}

fn mix(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= *b as u64;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// A hash of the names, sizes and modification times under `root`, walked like `spotindex::scan_folder` (breadth first,
/// names in order, no hidden names, no links, at most `SCAN_MAX_DEPTH` levels) and stopping after
/// `SIGNATURE_MAX_ENTRIES` entries. It changes when a file is added, removed, renamed or written. A missing folder has
/// the signature 0.
pub fn signature(root: &Path) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let Ok(meta) = std::fs::symlink_metadata(root) else { return 0 };
    if !meta.is_dir() {
        return 0;
    }
    let mut seen = 0usize;
    let mut level = vec![root.to_path_buf()];
    for depth in 0..=spotindex::SCAN_MAX_DEPTH {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(read) = std::fs::read_dir(dir) else { continue };
            let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if name.starts_with('.') {
                    continue;
                }
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                if !(meta.is_dir() || meta.is_file()) {
                    continue;
                }
                seen += 1;
                if seen > SIGNATURE_MAX_ENTRIES {
                    return h;
                }
                let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                mix(&mut h, name.as_bytes());
                mix(&mut h, &meta.len().to_le_bytes());
                mix(&mut h, &stamp.to_le_bytes());
                mix(&mut h, &[depth as u8, meta.is_dir() as u8]);
                if meta.is_dir() && depth < spotindex::SCAN_MAX_DEPTH {
                    next.push(path);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    h
}

/// One look: re-imports each folder of `home` whose signature differs from the one in `seen` (every folder, the first
/// time) and records the new signatures. Returns the domains that were imported. Runs on the importer thread.
pub fn import_once(app_id: &str, home: &Path, seen: &mut BTreeMap<String, u64>) -> Vec<&'static str> {
    let mut imported = Vec::new();
    for (domain, root) in roots(home) {
        let now = signature(&root);
        if seen.get(domain) == Some(&now) {
            continue;
        }
        spotindex::reindex_folder(app_id, domain, &root, PER_FOLDER);
        seen.insert(domain.to_string(), now);
        imported.push(domain);
    }
    imported
}

/// The importer the shell keeps: a timer, a "look running" flag and the signatures of the last look.
pub struct Importer {
    next_check: Option<Instant>,
    busy: Arc<AtomicBool>,
    seen: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl Default for Importer {
    fn default() -> Self {
        Self { next_check: None, busy: Arc::new(AtomicBool::new(false)), seen: Arc::new(Mutex::new(BTreeMap::new())) }
    }
}

impl Importer {
    /// True while a look is running on its thread.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Starts a look when one is due and none is running. `ready` is false while the indexed app is not installed (the
    /// timer then keeps running but nothing is read); `home` is the person's home directory. Returns true when a look
    /// was started. The first call that is `ready` starts one at once.
    pub fn poll(&mut self, now: Instant, ready: bool, app_id: &str, home: &Path) -> bool {
        if self.next_check.is_some_and(|due| now < due) {
            return false;
        }
        if !ready || self.busy() {
            // try again at the next tick for a busy look; a not-yet-installed app waits a full period
            if !ready {
                self.next_check = Some(now + CHECK_EVERY);
            }
            return false;
        }
        self.next_check = Some(now + CHECK_EVERY);
        self.busy.store(true, Ordering::Release);
        let busy = self.busy.clone();
        let seen = self.seen.clone();
        let (app_id, home) = (app_id.to_string(), home.to_path_buf());
        crate::bare::sys::spawn_named("file-import", move || {
            let imported = {
                let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
                import_once(&app_id, &home, &mut seen)
            };
            // tests keep the index in memory: the saver would write under the real data dir
            if !imported.is_empty() && !cfg!(test) {
                spotindex::flush();
            }
            busy.store(false, Ordering::Release);
        });
        true
    }
}

impl super::Shell {
    /// Starts the background look at Documents, Desktop and Downloads when it is due (see `file_import`). The Files app
    /// owns the items, so nothing is read until it is installed.
    pub(super) fn file_import_tick(&mut self) {
        let ready = super::apphost::installed().iter().any(|a| a.manifest.id == APP_ID);
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        self.file_import.poll(Instant::now(), ready, APP_ID, &home);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-file-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in FOLDERS {
            std::fs::create_dir_all(dir.join(f)).unwrap();
        }
        dir
    }

    fn found(app: &str, query: &str) -> usize {
        spotindex::search_app(app, query, 10).len()
    }

    fn wait_idle(importer: &Importer) {
        let start = Instant::now();
        while importer.busy() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!importer.busy(), "the look finished");
    }

    #[test]
    fn the_shell_tick_drives_the_importer_timer() {
        let mut shell = crate::shell::tests::shell();
        assert!(shell.file_import.next_check.is_none());
        shell.file_import_tick();
        assert!(shell.file_import.next_check.is_some(), "the tick polled the importer");
        let due = shell.file_import.next_check;
        shell.file_import_tick();
        assert_eq!(shell.file_import.next_check, due, "a second tick inside the period does nothing");
    }

    #[test]
    fn the_first_look_imports_all_three_folders() {
        let h = home("first");
        std::fs::write(h.join("Documents/a.txt"), "Aurora report\nnorthern lights\n").unwrap();
        std::fs::write(h.join("Desktop/b.txt"), "Budget sheet\nyearly numbers\n").unwrap();
        std::fs::write(h.join("Downloads/c.txt"), "Cabin booking\nweekend\n").unwrap();
        let app = "test.import.first";
        let mut seen = BTreeMap::new();
        assert_eq!(import_once(app, &h, &mut seen), ["Documents", "Desktop", "Downloads"]);
        assert_eq!(found(app, "aurora"), 1);
        assert_eq!(found(app, "budget"), 1);
        assert_eq!(found(app, "cabin"), 1);
        assert!(import_once(app, &h, &mut seen).is_empty(), "nothing changed, nothing re-imported");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn a_changed_folder_is_imported_again_and_a_deleted_file_leaves_the_index() {
        let h = home("change");
        std::fs::write(h.join("Documents/a.txt"), "Aurora report\n").unwrap();
        std::fs::write(h.join("Downloads/c.txt"), "Cabin booking\n").unwrap();
        let app = "test.import.change";
        let mut seen = BTreeMap::new();
        import_once(app, &h, &mut seen);
        std::fs::remove_file(h.join("Documents/a.txt")).unwrap();
        std::fs::write(h.join("Documents/z.txt"), "Zebra census\n").unwrap();
        assert_eq!(import_once(app, &h, &mut seen), ["Documents"], "only the folder that changed");
        assert_eq!(found(app, "aurora"), 0, "the deleted file is gone");
        assert_eq!(found(app, "zebra"), 1);
        assert_eq!(found(app, "cabin"), 1, "the other folder is untouched");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn the_signature_ignores_hidden_names_and_follows_writes() {
        let h = home("sig");
        let dir = h.join("Documents");
        let empty = signature(&dir);
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        assert_eq!(signature(&dir), empty, "a hidden file does not count");
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        let one = signature(&dir);
        assert_ne!(one, empty);
        std::fs::write(dir.join("a.txt"), "three").unwrap();
        assert_ne!(signature(&dir), one, "a rewrite with another size changes it");
        assert_eq!(signature(&h.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn each_folder_keeps_to_its_share_of_the_budget() {
        let h = home("share");
        for i in 0..PER_FOLDER + 20 {
            std::fs::write(h.join("Downloads").join(format!("f{i:05}.txt")), format!("Item {i}\n")).unwrap();
        }
        let app = "test.import.share";
        let mut seen = BTreeMap::new();
        import_once(app, &h, &mut seen);
        let indexed = spotindex::search_app(app, "item", spotindex::SCAN_MAX_FILES).len();
        assert_eq!(indexed, PER_FOLDER);
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn the_poll_waits_for_the_app_runs_off_the_caller_and_then_waits_its_period() {
        let h = home("poll");
        std::fs::write(h.join("Documents/a.txt"), "Polled note\n").unwrap();
        let app = "test.import.poll";
        let mut importer = Importer::default();
        let t0 = Instant::now();
        assert!(!importer.poll(t0, false, app, &h), "not ready: nothing starts");
        assert_eq!(found(app, "polled"), 0);
        assert!(!importer.poll(t0 + CHECK_EVERY / 2, true, app, &h), "still inside the period");
        let t1 = t0 + CHECK_EVERY;
        assert!(importer.poll(t1, true, app, &h), "due: a look starts");
        wait_idle(&importer);
        assert_eq!(found(app, "polled"), 1);
        assert!(!importer.poll(t1 + Duration::from_secs(1), true, app, &h), "the next look is a period away");
        std::fs::write(h.join("Documents/b.txt"), "Second note polled\n").unwrap();
        assert!(importer.poll(t1 + CHECK_EVERY, true, app, &h));
        wait_idle(&importer);
        assert_eq!(found(app, "polled"), 2, "the change was picked up");
        let _ = std::fs::remove_dir_all(&h);
    }
}
