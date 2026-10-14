//! Folder Actions: an app intent attached to a folder runs once for each file
//! that appears in that folder.
//!
//! Pure model. The shell keeps a `Registry` of attachments, and after each
//! watch event on an attached folder (see `bifrost_app::fswatch`) it passes the
//! folder's current listing to `Registry::on_listing`. The answer lists the
//! runs to start: each one calls the attached intent (`shell::intents`) with the
//! new file's name as the `file` parameter. The first listing of a folder only
//! records what is there, so attaching an action never runs it on old files.
//! Hidden entries (names starting with a dot) are ignored.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Parameter name that carries the new file's name to the intent.
pub const FILE_PARAM: &str = "file";

/// One intent attached to one folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub folder: PathBuf,
    pub app_id: String,
    pub intent_id: String,
}

impl Attachment {
    /// `None` when the folder is not absolute or either id is empty or uses a
    /// character outside ASCII letters, digits, `-` and `_`.
    pub fn new(folder: &Path, app_id: &str, intent_id: &str) -> Option<Self> {
        if !folder.is_absolute() || !valid_id(app_id) || !valid_id(intent_id) {
            return None;
        }
        Some(Self { folder: folder.to_path_buf(), app_id: app_id.to_string(), intent_id: intent_id.to_string() })
    }
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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

    fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.watches.iter().filter_map(|w| w.attachment.as_ref())
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
