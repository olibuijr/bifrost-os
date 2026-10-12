//! Sync-status state a file provider keeps for the Files app: the directories it manages, the directories the user
//! is looking at, badges on items under managed directories, the current selection and the target folder.
//! Pure model: the host reports what the user is doing (`begin_observing`, `set_selection`) and the Files app reads
//! the badge of each item (`badge_for`). Nothing is drawn here and nothing is shared across processes.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A badge a provider registered under an identifier: the name of its image and an optional label for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub image: String,
    pub label: Option<String>,
}

/// The sync controller state. A directory counts as managed when it is in the set or lies beneath one of them.
#[derive(Clone, Debug, Default)]
pub struct SyncController {
    directories: BTreeSet<PathBuf>,
    observing: BTreeSet<PathBuf>,
    badges: BTreeMap<String, Badge>,
    item_badges: BTreeMap<PathBuf, String>,
    selected: Vec<PathBuf>,
    target: Option<PathBuf>,
}

impl SyncController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the managed directories. Observations and item badges outside the new set are dropped.
    pub fn set_directories(&mut self, dirs: Vec<PathBuf>) {
        self.directories = dirs.into_iter().collect();
        let managed = |p: &Path| self.directories.iter().any(|d| p.starts_with(d));
        self.observing.retain(|p| managed(p.as_path()));
        self.item_badges.retain(|p, _| managed(p.as_path()));
    }

    /// The managed directories, in path order.
    pub fn directories(&self) -> Vec<&Path> {
        self.directories.iter().map(|p| p.as_path()).collect()
    }

    /// True when `path` is a managed directory or an item beneath one. Component-wise, so `/a/bc` is not under `/a/b`.
    pub fn is_managed(&self, path: &Path) -> bool {
        self.directories.iter().any(|d| path.starts_with(d))
    }

    /// The user opened `path`. Returns false, and records nothing, when `path` is not managed.
    pub fn begin_observing(&mut self, path: &Path) -> bool {
        if !self.is_managed(path) {
            return false;
        }
        self.observing.insert(path.to_path_buf());
        true
    }

    /// The user left `path`. Returns false when it was not being observed.
    pub fn end_observing(&mut self, path: &Path) -> bool {
        self.observing.remove(path)
    }

    /// True when `path` is an observed directory or lies beneath one.
    pub fn is_observing(&self, path: &Path) -> bool {
        self.observing.iter().any(|d| path.starts_with(d))
    }

    /// Registers the image and label for a badge identifier. Returns false when either name is empty.
    pub fn set_badge_image(&mut self, identifier: &str, image: &str, label: Option<&str>) -> bool {
        if identifier.is_empty() || image.is_empty() {
            return false;
        }
        let label = label.filter(|l| !l.is_empty()).map(str::to_string);
        self.badges.insert(identifier.to_string(), Badge { image: image.to_string(), label });
        true
    }

    /// Sets the badge identifier of an item, or clears it with `None` or an empty identifier. Returns false, and
    /// changes nothing, when the item is not under a managed directory.
    pub fn set_badge_identifier(&mut self, path: &Path, identifier: Option<&str>) -> bool {
        if !self.is_managed(path) {
            return false;
        }
        match identifier.filter(|i| !i.is_empty()) {
            Some(id) => {
                self.item_badges.insert(path.to_path_buf(), id.to_string());
            }
            None => {
                self.item_badges.remove(path);
            }
        }
        true
    }

    /// The badge identifier set on an item, if any.
    pub fn badge_identifier(&self, path: &Path) -> Option<&str> {
        self.item_badges.get(path).map(String::as_str)
    }

    /// The registered badge an item shows. `None` when the item has no identifier or the identifier has no image.
    pub fn badge_for(&self, path: &Path) -> Option<&Badge> {
        self.badge_identifier(path).and_then(|id| self.badges.get(id))
    }

    /// Records the items the user selected in the file manager and the folder they are in (the target).
    pub fn set_selection(&mut self, selected: Vec<PathBuf>, target: Option<PathBuf>) {
        self.selected = selected;
        self.target = target;
    }

    /// The selected items, in the order they were reported.
    pub fn selected_items(&self) -> &[PathBuf] {
        &self.selected
    }

    /// The folder the file manager is showing, if one is reported.
    pub fn targeted(&self) -> Option<&Path> {
        self.target.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sync_root() -> SyncController {
        let mut c = SyncController::new();
        c.set_directories(vec![PathBuf::from("/home/u/Sync")]);
        c
    }

    #[test]
    fn managed_is_component_wise() {
        let c = sync_root();
        assert!(c.is_managed(Path::new("/home/u/Sync")));
        assert!(c.is_managed(Path::new("/home/u/Sync/docs/a.txt")));
        assert!(!c.is_managed(Path::new("/home/u/Syncx/a.txt")));
        assert!(!c.is_managed(Path::new("/home/u/Other")));
    }

    #[test]
    fn observing_needs_a_managed_directory() {
        let mut c = sync_root();
        assert!(c.begin_observing(Path::new("/home/u/Sync/docs")));
        assert!(!c.begin_observing(Path::new("/home/u/Other")));
        assert!(c.is_observing(Path::new("/home/u/Sync/docs/deep/x.txt")));
        assert!(!c.is_observing(Path::new("/home/u/Sync/music")));
        assert!(c.end_observing(Path::new("/home/u/Sync/docs")));
        assert!(!c.end_observing(Path::new("/home/u/Sync/docs")));
        assert!(!c.is_observing(Path::new("/home/u/Sync/docs/deep/x.txt")));
    }

    #[test]
    fn replacing_directories_drops_stale_state() {
        let mut c = sync_root();
        c.begin_observing(Path::new("/home/u/Sync/docs"));
        c.set_badge_image("synced", "ok", None);
        c.set_badge_identifier(Path::new("/home/u/Sync/a.txt"), Some("synced"));
        c.set_directories(vec![PathBuf::from("/home/u/Music")]);
        assert_eq!(c.directories(), vec![Path::new("/home/u/Music")]);
        assert!(!c.is_observing(Path::new("/home/u/Sync/docs")));
        assert_eq!(c.badge_identifier(Path::new("/home/u/Sync/a.txt")), None);
    }

    #[test]
    fn badge_image_and_identifier_resolve_together() {
        let mut c = sync_root();
        assert!(!c.set_badge_image("", "ok", None));
        assert!(!c.set_badge_image("synced", "", None));
        assert!(c.set_badge_image("synced", "ok", Some("Synced")));
        assert!(c.set_badge_identifier(Path::new("/home/u/Sync/a.txt"), Some("synced")));
        let badge = c.badge_for(Path::new("/home/u/Sync/a.txt")).unwrap();
        assert_eq!(badge.image, "ok");
        assert_eq!(badge.label.as_deref(), Some("Synced"));
        // an identifier with no registered image shows no badge
        c.set_badge_identifier(Path::new("/home/u/Sync/b.txt"), Some("busy"));
        assert_eq!(c.badge_identifier(Path::new("/home/u/Sync/b.txt")), Some("busy"));
        assert!(c.badge_for(Path::new("/home/u/Sync/b.txt")).is_none());
    }

    #[test]
    fn badges_only_go_on_managed_items_and_clear() {
        let mut c = sync_root();
        c.set_badge_image("synced", "ok", None);
        assert!(!c.set_badge_identifier(Path::new("/home/u/Other/a.txt"), Some("synced")));
        assert_eq!(c.badge_identifier(Path::new("/home/u/Other/a.txt")), None);
        assert!(c.set_badge_identifier(Path::new("/home/u/Sync/a.txt"), Some("synced")));
        assert!(c.set_badge_identifier(Path::new("/home/u/Sync/a.txt"), None));
        assert!(c.badge_for(Path::new("/home/u/Sync/a.txt")).is_none());
    }

    #[test]
    fn selection_and_target_are_reported_back() {
        let mut c = sync_root();
        assert!(c.targeted().is_none());
        c.set_selection(
            vec![PathBuf::from("/home/u/Sync/a.txt"), PathBuf::from("/home/u/Sync/b.txt")],
            Some(PathBuf::from("/home/u/Sync")),
        );
        assert_eq!(c.selected_items().len(), 2);
        assert_eq!(c.selected_items()[1], PathBuf::from("/home/u/Sync/b.txt"));
        assert_eq!(c.targeted(), Some(Path::new("/home/u/Sync")));
        c.set_selection(Vec::new(), None);
        assert!(c.selected_items().is_empty());
        assert!(c.targeted().is_none());
    }
}
