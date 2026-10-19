//! Folder Actions: an app intent attached to a folder runs once for each file
//! that appears in that folder.
//!
//! The model is `Registry` of attachments. `Watcher` owns one and rescans each
//! attached folder once a second (`poll`): the folder's listing goes to
//! `Registry::on_listing`, and the answer lists the runs to start. Each run calls
//! the attached intent (`shell::intents`) with the new file's name as the `file`
//! parameter. Attaching records the folder's current contents first, so an action
//! never runs on files that were already there. Hidden entries (names starting
//! with a dot) are ignored.
//!
//! The shell cannot use the SDK's `bifrost_app::fswatch` (the SDK depends on this
//! crate, not the other way round), so it polls. The attachments are saved as
//! text under the shell data dir (`file_path`), one `folder TAB app TAB intent` line each.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a run waits for its app to start (an intent call blocks until the app answers).
const RUN_WAIT: Duration = Duration::from_secs(8);

/// Parameter name that carries the new file's name to the intent.
pub const FILE_PARAM: &str = "file";

/// How often the attached folders are rescanned.
pub const SCAN_EVERY: Duration = Duration::from_secs(1);

/// The attachments file under the shell data dir `root`.
pub fn file_path(root: &Path) -> PathBuf {
    root.join("folder-actions.tsv")
}

/// One intent attached to one folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub folder: PathBuf,
    pub app_id: String,
    pub intent_id: String,
}

impl Attachment {
    /// `None` when the folder is not absolute, is not UTF-8 or has a tab or newline (the saved form needs them),
    /// the app id is not a reverse-DNS style id (see `valid_app`), or the intent id is empty or uses a character
    /// outside ASCII letters, digits, `-` and `_`.
    pub fn new(folder: &Path, app_id: &str, intent_id: &str) -> Option<Self> {
        let text = folder.to_str()?;
        if !folder.is_absolute() || text.contains(['\t', '\n']) || !valid_app(app_id) || !valid_id(intent_id) {
            return None;
        }
        Some(Self { folder: folder.to_path_buf(), app_id: app_id.to_string(), intent_id: intent_id.to_string() })
    }
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// App ids are reverse-DNS names such as `is.olibuijr.folderactions.desktop`, the same rule the privacy daemon uses.
fn valid_app(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// A run the shell should start: call `intent_id` of `app_id` for `file`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub app_id: String,
    pub intent_id: String,
    pub file: String,
}

/// An attachment with the names seen in its folder at the last listing.
#[derive(Clone, Debug, Default)]
struct Watch {
    attachment: Option<Attachment>,
    seen: BTreeSet<String>,
    primed: bool,
}

/// Every folder action the user has set up, in the order they were added.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    watches: Vec<Watch>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an attachment. Returns `false` when the same folder, app and
    /// intent are already attached.
    pub fn add(&mut self, a: Attachment) -> bool {
        if self.attachments().any(|x| *x == a) {
            return false;
        }
        self.watches.push(Watch { attachment: Some(a), seen: BTreeSet::new(), primed: false });
        true
    }

    /// Removes an attachment. Returns `false` when it was not attached.
    pub fn remove(&mut self, a: &Attachment) -> bool {
        let before = self.watches.len();
        self.watches.retain(|w| w.attachment.as_ref() != Some(a));
        self.watches.len() != before
    }

    /// Removes every attachment on `folder` (the setup screen's "stop watching this folder"). Returns how many were removed.
    pub fn remove_folder(&mut self, folder: &Path) -> usize {
        let before = self.watches.len();
        self.watches.retain(|w| w.attachment.as_ref().map_or(true, |a| a.folder != folder));
        before - self.watches.len()
    }

    /// The attachments on `folder`.
    pub fn for_folder<'a>(&'a self, folder: &'a Path) -> impl Iterator<Item = &'a Attachment> + 'a {
        self.attachments().filter(move |a| a.folder == folder)
    }

    /// Takes the full listing of `folder` (file names only) and returns the runs for
    /// the names that were not in the previous listing of that folder.
    pub fn on_listing(&mut self, folder: &Path, names: &[String]) -> Vec<Run> {
        let now: BTreeSet<String> = names.iter().filter(|n| !n.starts_with('.') && !n.is_empty()).cloned().collect();
        let mut runs = Vec::new();
        for w in self.watches.iter_mut() {
            let Some(a) = w.attachment.as_ref() else { continue };
            if a.folder != folder {
                continue;
            }
            if w.primed {
                for name in now.difference(&w.seen) {
                    runs.push(Run { app_id: a.app_id.clone(), intent_id: a.intent_id.clone(), file: name.clone() });
                }
            }
            w.seen = now.clone();
            w.primed = true;
        }
        runs
    }

    /// The folders with at least one attachment, once each.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.attachments().map(|a| a.folder.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// One attachment per line: folder, app id and intent id, separated by tabs.
    pub fn to_text(&self) -> String {
        self.attachments().map(|a| format!("{}\t{}\t{}\n", a.folder.display(), a.app_id, a.intent_id)).collect()
    }

    /// Reads `to_text` output. Lines that do not parse as an attachment are skipped.
    pub fn from_text(text: &str) -> Registry {
        let mut r = Registry::new();
        for line in text.lines() {
            let mut parts = line.split('\t');
            if let (Some(folder), Some(app), Some(intent), None) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                if let Some(a) = Attachment::new(Path::new(folder), app, intent) {
                    r.add(a);
                }
            }
        }
        r
    }

    fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.watches.iter().filter_map(|w| w.attachment.as_ref())
    }
}

/// The names in `folder` (file names only; an unreadable folder has none).
pub fn list_names(folder: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(folder) else { return Vec::new() };
    dir.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
}

/// The registry with its folders rescanned on `poll`. The shell keeps one and calls `poll` from its tick.
#[derive(Debug, Default)]
pub struct Watcher {
    registry: Registry,
    next_scan: Option<Instant>,
}

impl Watcher {
    /// Takes the saved registry; the folders are recorded as they are now, so nothing runs for them until new files arrive.
    pub fn new(registry: Registry) -> Self {
        let mut w = Self { registry, next_scan: None };
        for folder in w.registry.folders() {
            w.registry.on_listing(&folder, &list_names(&folder));
        }
        w
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Adds an attachment and records its folder's contents. False when it was already attached.
    pub fn attach(&mut self, a: Attachment) -> bool {
        let folder = a.folder.clone();
        if !self.registry.add(a) {
            return false;
        }
        self.registry.on_listing(&folder, &list_names(&folder));
        true
    }

    /// Removes an attachment. False when it was not attached.
    pub fn detach(&mut self, a: &Attachment) -> bool {
        self.registry.remove(a)
    }

    /// Removes every attachment on `folder`. Returns how many were removed.
    pub fn detach_folder(&mut self, folder: &Path) -> usize {
        self.registry.remove_folder(folder)
    }

    /// Saves the attachments to `file_path(root)`, replacing the old file atomically, so a restart finds what the setup
    /// app last left. The folder is created when it is missing.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = file_path(root);
        std::fs::create_dir_all(root)?;
        let tmp = path.with_extension("tsv.tmp");
        std::fs::write(&tmp, self.registry.to_text())?;
        std::fs::rename(&tmp, &path)
    }

    /// The runs for the new files in the attached folders. Rescans only once `SCAN_EVERY` has passed since the last scan.
    pub fn poll(&mut self, now: Instant) -> Vec<Run> {
        if self.next_scan.is_some_and(|due| now < due) {
            return Vec::new();
        }
        self.next_scan = Some(now + SCAN_EVERY);
        let mut runs = Vec::new();
        for folder in self.registry.folders() {
            runs.extend(self.registry.on_listing(&folder, &list_names(&folder)));
        }
        runs
    }
}

impl super::Shell {
    /// Starts the runs for new files in the attached folders. Each run is its own thread, because an intent call blocks
    /// while its app starts.
    pub(super) fn folder_actions_tick(&mut self) {
        for run in self.folder_actions.poll(Instant::now()) {
            std::thread::spawn(move || {
                if let Err(e) = super::intents::run_with(&run.app_id, &run.intent_id, &[(FILE_PARAM, run.file.as_str())], RUN_WAIT) {
                    eprintln!("springboard: folder action {}: {e}", run.intent_id);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn attach(folder: &str, app: &str, intent: &str) -> Attachment {
        Attachment::new(Path::new(folder), app, intent).unwrap()
    }

    #[test]
    fn rejects_relative_folders_and_bad_ids() {
        assert!(Attachment::new(Path::new("relative"), "files", "tag").is_none());
        assert!(Attachment::new(Path::new("/srv/in"), "", "tag").is_none());
        assert!(Attachment::new(Path::new("/srv/in"), "files", "tag/x").is_none());
        assert!(Attachment::new(Path::new("/srv/in"), "files", "tag-1_b").is_some());
    }

    #[test]
    fn first_listing_only_records() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        assert!(r.on_listing(Path::new("/srv/in"), &names(&["a.txt", "b.txt"])).is_empty());
    }

    #[test]
    fn new_names_run_once_each() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        let dir = Path::new("/srv/in");
        r.on_listing(dir, &names(&["a.txt"]));
        let runs = r.on_listing(dir, &names(&["a.txt", "c.pdf"]));
        assert_eq!(runs, vec![Run { app_id: "files".into(), intent_id: "tag".into(), file: "c.pdf".into() }]);
        // the same listing again starts nothing
        assert!(r.on_listing(dir, &names(&["a.txt", "c.pdf"])).is_empty());
    }

    #[test]
    fn hidden_entries_are_ignored() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        let dir = Path::new("/srv/in");
        r.on_listing(dir, &names(&[]));
        assert!(r.on_listing(dir, &names(&[".DS_tmp", ""])).is_empty());
    }

    #[test]
    fn a_file_that_leaves_and_returns_runs_again() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        let dir = Path::new("/srv/in");
        r.on_listing(dir, &names(&[]));
        assert_eq!(r.on_listing(dir, &names(&["x"])).len(), 1);
        assert!(r.on_listing(dir, &names(&[])).is_empty());
        assert_eq!(r.on_listing(dir, &names(&["x"])).len(), 1);
    }

    #[test]
    fn other_folders_do_not_run_this_action() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        r.on_listing(Path::new("/srv/in"), &names(&[]));
        assert!(r.on_listing(Path::new("/srv/out"), &names(&["x"])).is_empty());
        assert_eq!(r.on_listing(Path::new("/srv/in"), &names(&["x"])).len(), 1);
    }

    #[test]
    fn duplicate_add_and_remove() {
        let mut r = Registry::new();
        let a = attach("/srv/in", "files", "tag");
        assert!(r.add(a.clone()));
        assert!(!r.add(a.clone()));
        assert_eq!(r.for_folder(Path::new("/srv/in")).count(), 1);
        assert!(r.remove(&a));
        assert!(!r.remove(&a));
        assert_eq!(r.for_folder(Path::new("/srv/in")).count(), 0);
    }

    #[test]
    fn remove_folder_detaches_every_action_on_that_folder_only() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        r.add(attach("/srv/in", "notes", "import"));
        r.add(attach("/srv/out", "files", "tag"));
        assert_eq!(r.remove_folder(Path::new("/srv/in")), 2);
        assert_eq!(r.for_folder(Path::new("/srv/in")).count(), 0);
        assert_eq!(r.for_folder(Path::new("/srv/out")).count(), 1);
        assert_eq!(r.remove_folder(Path::new("/srv/in")), 0, "nothing left to remove");
    }

    #[test]
    fn attachments_round_trip_through_text() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        r.add(attach("/srv/in", "notes", "import"));
        r.add(attach("/srv/out", "files", "tag"));
        let text = r.to_text();
        assert_eq!(text, "/srv/in\tfiles\ttag\n/srv/in\tnotes\timport\n/srv/out\tfiles\ttag\n");
        assert_eq!(Registry::from_text(&text).to_text(), text);
        assert_eq!(r.folders(), vec![PathBuf::from("/srv/in"), PathBuf::from("/srv/out")]);
    }

    #[test]
    fn saved_text_skips_bad_lines() {
        let text = "relative\tfiles\ttag\n/srv/in\tfiles\ttag\textra\n/srv/in\tfiles\n\n/srv/ok\tfiles\ttag\n";
        assert_eq!(Registry::from_text(text).to_text(), "/srv/ok\tfiles\ttag\n");
    }

    #[test]
    fn folders_with_tabs_or_newlines_are_refused() {
        assert!(Attachment::new(Path::new("/srv/a\tb"), "files", "tag").is_none());
        assert!(Attachment::new(Path::new("/srv/a\nb"), "files", "tag").is_none());
    }

    #[test]
    fn watcher_runs_for_a_file_added_after_attach() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        std::fs::write(folder.join("old.txt"), b"x").unwrap();
        let mut w = Watcher::new(Registry::new());
        let a = Attachment::new(&folder, "files", "tag").unwrap();
        assert!(w.attach(a.clone()));
        assert!(!w.attach(a.clone()));
        let t0 = Instant::now();
        assert!(w.poll(t0).is_empty());
        std::fs::write(folder.join("new.pdf"), b"y").unwrap();
        // not rescanned again until a second has passed
        assert!(w.poll(t0 + Duration::from_millis(10)).is_empty());
        let runs = w.poll(t0 + SCAN_EVERY);
        assert_eq!(runs, vec![Run { app_id: "files".into(), intent_id: "tag".into(), file: "new.pdf".into() }]);
        assert!(w.detach(&a));
        std::fs::write(folder.join("later.txt"), b"z").unwrap();
        assert!(w.poll(t0 + SCAN_EVERY * 3).is_empty());
    }

    #[test]
    fn loaded_watcher_does_not_run_for_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        std::fs::write(folder.join("a.txt"), b"x").unwrap();
        let saved = Registry::from_text(&format!("{}\tfiles\ttag\n", folder.display()));
        let mut w = Watcher::new(saved);
        assert!(w.poll(Instant::now()).is_empty());
        std::fs::write(folder.join("b.txt"), b"y").unwrap();
        let runs = w.poll(Instant::now() + SCAN_EVERY);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].file, "b.txt");
    }

    #[test]
    fn saved_attachments_come_back_after_a_restart_and_detaching_a_folder_is_saved() {
        let root = tempfile::tempdir().unwrap();
        let in_dir = tempfile::tempdir().unwrap();
        let out_dir = tempfile::tempdir().unwrap();
        let mut w = Watcher::new(Registry::new());
        assert!(w.attach(Attachment::new(in_dir.path(), "files", "tag").unwrap()));
        assert!(w.attach(Attachment::new(in_dir.path(), "notes", "import").unwrap()));
        assert!(w.attach(Attachment::new(out_dir.path(), "files", "tag").unwrap()));
        w.save(&root.path().join("data")).unwrap();
        assert!(!file_path(&root.path().join("data")).with_extension("tsv.tmp").exists(), "the temporary file is renamed away");
        let back = Watcher::new(Registry::from_text(&std::fs::read_to_string(file_path(&root.path().join("data"))).unwrap()));
        assert_eq!(back.registry().to_text(), w.registry().to_text());
        assert_eq!(w.detach_folder(in_dir.path()), 2);
        assert_eq!(w.detach_folder(in_dir.path()), 0);
        w.save(&root.path().join("data")).unwrap();
        let after = Registry::from_text(&std::fs::read_to_string(file_path(&root.path().join("data"))).unwrap());
        assert_eq!(after.folders(), vec![out_dir.path().to_path_buf()]);
    }

    #[test]
    fn two_actions_on_one_folder_both_run() {
        let mut r = Registry::new();
        r.add(attach("/srv/in", "files", "tag"));
        r.add(attach("/srv/in", "notes", "import"));
        let dir = Path::new("/srv/in");
        r.on_listing(dir, &names(&[]));
        let runs = r.on_listing(dir, &names(&["n"]));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].app_id, "files");
        assert_eq!(runs[1].intent_id, "import");
    }
}
