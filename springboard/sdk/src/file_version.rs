//! `FileVersion` parity (Foundation).
//!
//! A `FileVersionStore` is the per-app version store: a directory holding one folder per file, keyed by its canonical
//! path (a safe save replaces the inode, so the inode cannot be the identity; a renamed file starts a new history). Each version is an atomic copy named `<millis>-<serial>.<d|k>`
//! (`d` discardable, `k` kept) with an optional `.name` sidecar for the localized name.
//!
//! Mirrors `currentVersionOfItemAtURL:`, `otherVersionsOfItemAtURL:` (newest first),
//! `addVersionOfItemAtURL:withContentsOfURL:options:`, `removeOtherVersionsOfItemAtURL:`,
//! `replaceItemAtURL:options:` and `remove`. Discardable versions are pruned by count and age on every add.
//! Gaps: no conflict versions (`isConflict` is always false, `isResolved` always true), no cloud versions.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Options for adding a version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AddingOptions {
    /// The system may delete the version to reclaim space (autosave snapshots).
    pub discardable: bool,
}

/// Options for replacing an item with a version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplacingOptions {
    /// Consume the version's file after restoring it.
    pub by_moving: bool,
}

/// One saved version of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    url: PathBuf,
    modified_ms: u64,
    discardable: bool,
    localized_name: Option<String>,
    current: bool,
}

impl FileVersion {
    /// `URL`: the file holding this version's contents.
    pub fn url(&self) -> &Path {
        &self.url
    }
    /// `modificationDate`.
    pub fn modification_date(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.modified_ms)
    }
    /// `localizedName`.
    pub fn localized_name(&self) -> Option<&str> {
        self.localized_name.as_deref()
    }
    /// `isDiscardable`.
    pub fn is_discardable(&self) -> bool {
        self.discardable
    }
    /// `isResolved`: no conflict versions exist, so every version is resolved.
    pub fn is_resolved(&self) -> bool {
        true
    }
    /// `isConflict`.
    pub fn is_conflict(&self) -> bool {
        false
    }
    /// Whether this is the item's current version rather than a stored one.
    pub fn is_current(&self) -> bool {
        self.current
    }
    /// Sets the localized name of a stored version.
    pub fn set_localized_name(&mut self, name: &str) -> io::Result<()> {
        if self.current {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "the current version has no stored name"));
        }
        fs::write(sidecar(&self.url), name)?;
        self.localized_name = Some(name.to_string());
        Ok(())
    }
    /// `remove`: deletes the stored version. The current version cannot be removed.
    pub fn remove(self) -> io::Result<()> {
        if self.current {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "cannot remove the current version"));
        }
        let _ = fs::remove_file(sidecar(&self.url));
        fs::remove_file(&self.url)
    }
}

fn sidecar(version: &Path) -> PathBuf {
    let mut s = version.as_os_str().to_owned();
    s.push(".name");
    PathBuf::from(s)
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The per-app version store.
pub struct FileVersionStore {
    root: PathBuf,
    /// Most discardable versions kept per item.
    pub max_discardable: usize,
    /// Discardable versions older than this are pruned.
    pub max_discardable_age: Duration,
}

impl FileVersionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FileVersionStore { root: root.into(), max_discardable: 10, max_discardable_age: Duration::from_secs(30 * 24 * 3600) }
    }

    /// The item's folder, keyed by its canonical path. Not by inode: a safe save (`write_atomic`) renames a new file
    /// into place, so the inode changes on every save and the history would be lost exactly when an app saves.
    fn dir_for(&self, item: &Path) -> io::Result<PathBuf> {
        let path = fs::canonicalize(item)?;
        // FNV-1a of the path bytes: a stable, short folder name
        let key = path.as_os_str().as_encoded_bytes().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
        Ok(self.root.join(format!("{key:016x}")))
    }

    /// `currentVersionOfItemAtURL:`.
    pub fn current_version_of_item(&self, item: &Path) -> Option<FileVersion> {
        let m = fs::metadata(item).ok()?;
        Some(FileVersion {
            url: item.to_path_buf(),
            modified_ms: m.modified().map(ms).unwrap_or(0),
            discardable: false,
            localized_name: None,
            current: true,
        })
    }

    /// `otherVersionsOfItemAtURL:`, newest first.
    pub fn other_versions_of_item(&self, item: &Path) -> Vec<FileVersion> {
        let Ok(dir) = self.dir_for(item) else { return Vec::new() };
        let Ok(rd) = fs::read_dir(&dir) else { return Vec::new() };
        let mut out: Vec<(u64, u64, FileVersion)> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let (stem, kind) = name.rsplit_once('.')?;
                if kind != "d" && kind != "k" {
                    return None;
                }
                let (t, s) = stem.split_once('-')?;
                let (t, s) = (t.parse::<u64>().ok()?, s.parse::<u64>().ok()?);
                let url = e.path();
                let localized_name = fs::read_to_string(sidecar(&url)).ok();
                Some((t, s, FileVersion { url, modified_ms: t, discardable: kind == "d", localized_name, current: false }))
            })
            .collect();
        out.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
        out.into_iter().map(|x| x.2).collect()
    }

    /// `addVersionOfItemAtURL:withContentsOfURL:options:`.
    pub fn add_of_item(&self, item: &Path, contents: &Path, options: AddingOptions) -> io::Result<FileVersion> {
        self.add_of_item_at(item, contents, options, SystemTime::now())
    }

    /// `add_of_item` with an explicit clock (for age pruning).
    pub fn add_of_item_at(&self, item: &Path, contents: &Path, options: AddingOptions, now: SystemTime) -> io::Result<FileVersion> {
        let dir = self.dir_for(item)?;
        let bytes = fs::read(contents)?;
        let t = ms(now);
        let kind = if options.discardable { "d" } else { "k" };
        let url = dir.join(format!("{:016}-{:06}.{}", t, SERIAL.fetch_add(1, Ordering::Relaxed) % 1_000_000, kind));
        bifrost_core::fsutil::write_atomic(&url, bytes)?;
        self.prune(item, now);
        Ok(FileVersion { url, modified_ms: t, discardable: options.discardable, localized_name: None, current: false })
    }

    fn prune(&self, item: &Path, now: SystemTime) {
        let cutoff = ms(now).saturating_sub(self.max_discardable_age.as_millis() as u64);
        let mut kept = 0;
        for v in self.other_versions_of_item(item) {
            if !v.discardable {
                continue;
            }
            kept += 1;
            if kept > self.max_discardable || v.modified_ms < cutoff {
                let _ = v.remove();
            }
        }
    }

    /// `removeOtherVersionsOfItemAtURL:`.
    pub fn remove_other_versions_of_item(&self, item: &Path) -> io::Result<()> {
        for v in self.other_versions_of_item(item) {
            v.remove()?;
        }
        Ok(())
    }

    /// `replaceItemAtURL:options:`: the item's current contents are kept as a version, then replaced by `version`.
    pub fn replace_item(&self, version: &FileVersion, item: &Path, options: ReplacingOptions) -> io::Result<PathBuf> {
        if version.current {
            return Ok(item.to_path_buf());
        }
        let bytes = fs::read(&version.url)?;
        self.add_of_item(item, item, AddingOptions::default())?;
        bifrost_core::fsutil::write_atomic(item, bytes)?;
        if options.by_moving {
            version.clone().remove()?;
        }
        Ok(item.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, FileVersionStore, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let item = d.path().join("doc.txt");
        fs::write(&item, "v1").unwrap();
        let store = FileVersionStore::new(d.path().join("store"));
        (d, store, item)
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn saving_a_version_stores_a_copy_and_lists_newest_first() {
        let (_d, store, item) = setup();
        store.add_of_item_at(&item, &item, AddingOptions::default(), at(10)).unwrap();
        fs::write(&item, "v2").unwrap();
        store.add_of_item_at(&item, &item, AddingOptions::default(), at(20)).unwrap();
        let v = store.other_versions_of_item(&item);
        assert_eq!(v.len(), 2);
        assert_eq!(fs::read_to_string(v[0].url()).unwrap(), "v2");
        assert_eq!(fs::read_to_string(v[1].url()).unwrap(), "v1");
        assert_eq!(v[0].modification_date(), at(20));
        assert!(v[0].is_resolved() && !v[0].is_discardable());
    }

    #[test]
    fn current_version_is_the_item() {
        let (_d, store, item) = setup();
        let c = store.current_version_of_item(&item).unwrap();
        assert_eq!(c.url(), item);
        assert!(c.is_current());
        assert!(c.remove().is_err());
        assert!(store.current_version_of_item(&item.with_extension("nope")).is_none());
    }

    #[test]
    fn replace_restores_content_and_keeps_current_as_version() {
        let (_d, store, item) = setup();
        let old = store.add_of_item(&item, &item, AddingOptions::default()).unwrap();
        fs::write(&item, "v2").unwrap();
        store.replace_item(&old, &item, ReplacingOptions::default()).unwrap();
        assert_eq!(fs::read_to_string(&item).unwrap(), "v1");
        let v = store.other_versions_of_item(&item);
        assert_eq!(v.len(), 2);
        assert!(v.iter().any(|x| fs::read_to_string(x.url()).unwrap() == "v2"));
    }

    #[test]
    fn remove_other_versions_purges() {
        let (_d, store, item) = setup();
        store.add_of_item(&item, &item, AddingOptions::default()).unwrap();
        store.add_of_item(&item, &item, AddingOptions { discardable: true }).unwrap();
        store.remove_other_versions_of_item(&item).unwrap();
        assert!(store.other_versions_of_item(&item).is_empty());
    }

    #[test]
    fn discardable_versions_are_pruned_by_count_and_age_but_kept_ones_stay() {
        let (_d, mut store, item) = setup();
        store.max_discardable = 2;
        store.max_discardable_age = Duration::from_secs(100);
        store.add_of_item_at(&item, &item, AddingOptions::default(), at(1)).unwrap();
        for t in [1000, 1010, 1020] {
            store.add_of_item_at(&item, &item, AddingOptions { discardable: true }, at(t)).unwrap();
        }
        let v = store.other_versions_of_item(&item);
        let disc: Vec<_> = v.iter().filter(|x| x.is_discardable()).map(|x| x.modification_date()).collect();
        assert_eq!(disc, vec![at(1020), at(1010)]);
        assert_eq!(v.iter().filter(|x| !x.is_discardable()).count(), 1);
        store.add_of_item_at(&item, &item, AddingOptions { discardable: true }, at(1125)).unwrap();
        let disc = store.other_versions_of_item(&item).iter().filter(|x| x.is_discardable()).count();
        assert_eq!(disc, 1, "1010 and 1020 are older than 100 s");
    }

    #[test]
    fn a_safe_save_keeps_the_history_and_a_rename_starts_a_new_one() {
        let (d, store, item) = setup();
        store.add_of_item(&item, &item, AddingOptions::default()).unwrap();
        // a safe save renames a new file into place: a new inode, the same document
        bifrost_core::fsutil::write_atomic(&item, b"v2".to_vec()).unwrap();
        assert_eq!(store.other_versions_of_item(&item).len(), 1);
        let renamed = d.path().join("renamed.txt");
        fs::rename(&item, &renamed).unwrap();
        assert!(store.other_versions_of_item(&renamed).is_empty());
    }

    #[test]
    fn localized_name_persists_and_remove_deletes() {
        let (_d, store, item) = setup();
        let mut v = store.add_of_item(&item, &item, AddingOptions::default()).unwrap();
        v.set_localized_name("Before edit").unwrap();
        let listed = store.other_versions_of_item(&item);
        assert_eq!(listed[0].localized_name(), Some("Before edit"));
        listed.into_iter().next().unwrap().remove().unwrap();
        assert!(store.other_versions_of_item(&item).is_empty());
    }
}
