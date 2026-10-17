//! Asset packs for apps (the BackgroundAssets surface): the manifest of downloadable packs, their local state, install
//! and removal on disk, and on-demand availability. The models are `bifrost_ui::assetpack`, `downloadqueue` and
//! `managed_download`; this module re-exports them and adds [`AssetPackStore`], the part an app needs to use them:
//! where installed packs live, which versions are installed across runs, and `ensure_local_availability` that fetches
//! what is missing through a function the app supplies (a `URLSession` download, a mock source in tests).
//!
//! ```ignore
//! let mut store = AssetPackStore::open(&data_dir.join("asset-packs"), manifest)?;
//! let result = store.ensure_local_availability(&["level2"], |pack| download(&pack.id, pack.download_bytes));
//! if let Some(dir) = store.pack_dir("level2").filter(|_| store.is_local("level2")) { /* read the files */ }
//! ```
//!
//! Layout under the root: `installed.txt` (one `id<TAB>version` line per installed pack) and `packs/<id>/pack.bin`.
//! Pack ids become directory names, so an id with a path separator or dot segment is refused.
//!
//! Downloading while the app is closed is the shell's job through the background download queue
//! (`bifrost_ui::managed_download`); this store is what the app reads once a pack has arrived.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use bifrost_ui::assetmock::{MockError, MockPackSource};
pub use bifrost_ui::assetpack::{
    AssetPack, AssetPackManager, AssetPackManifest, LocalAvailabilityError, PackError, PackStatus, SizeCalculationMethod,
};
pub use bifrost_ui::downloadqueue::{Budget, Download, DownloadQueue, Priority, QueueError};
pub use bifrost_ui::managed_download::{Conditions, ManagedDownloads, PackState, RequestError};

/// What an install, removal or open of the store can fail with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Pack(PackError),
    /// The id cannot be a directory name (empty, a path separator, or a dot segment).
    BadId(String),
    /// The bytes are not as long as the pack's download size.
    SizeMismatch { id: String, expected: u64, actual: u64 },
    Io(String),
}

impl From<PackError> for StoreError {
    fn from(e: PackError) -> Self {
        StoreError::Pack(e)
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e.to_string())
    }
}

const STATE_FILE: &str = "installed.txt";
const PACK_FILE: &str = "pack.bin";

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0'])
}

/// The local state of an app's asset packs, kept on disk.
pub struct AssetPackStore {
    manager: AssetPackManager,
    root: PathBuf,
}

impl AssetPackStore {
    /// Opens (and creates) the store at `root` for `manifest`. Packs recorded as installed whose files are gone, or
    /// that the manifest no longer lists, are dropped from the state.
    pub fn open(root: &Path, manifest: AssetPackManifest) -> Result<Self, StoreError> {
        fs::create_dir_all(root.join("packs"))?;
        let mut manager = AssetPackManager::new(manifest);
        if let Ok(text) = fs::read_to_string(root.join(STATE_FILE)) {
            for line in text.lines() {
                let Some((id, version)) = line.split_once('\t') else { continue };
                let Ok(version) = version.trim().parse::<u32>() else { continue };
                if id_ok(id) && root.join("packs").join(id).join(PACK_FILE).is_file() {
                    let _ = manager.set_installed(id, version);
                }
            }
        }
        Ok(AssetPackStore { manager, root: root.to_path_buf() })
    }

    pub fn manager(&self) -> &AssetPackManager {
        &self.manager
    }

    /// `status(ofAssetPackWithID:)`.
    pub fn status(&self, id: &str) -> Result<PackStatus, PackError> {
        self.manager.status(id)
    }

    /// True when the pack is on disk at the manifest's current version.
    pub fn is_local(&self, id: &str) -> bool {
        self.manager.status(id) == Ok(PackStatus::Available)
    }

    /// The directory a pack is installed in (it exists only once the pack is installed), `None` for an unusable id.
    pub fn pack_dir(&self, id: &str) -> Option<PathBuf> {
        id_ok(id).then(|| self.root.join("packs").join(id))
    }

    fn save(&self) -> Result<(), StoreError> {
        let mut text = String::new();
        for pack in self.manager.manifest().all_asset_packs() {
            if let Some(version) = self.manager.installed_version(&pack.id) {
                text.push_str(&format!("{}\t{}\n", pack.id, version));
            }
        }
        let tmp = self.root.join(format!("{STATE_FILE}.part"));
        fs::write(&tmp, text)?;
        fs::rename(&tmp, self.root.join(STATE_FILE))?;
        Ok(())
    }

    /// Installs a downloaded pack: checks the id and the size, writes the bytes, records the manifest's version.
    /// Returns the pack's directory.
    pub fn install(&mut self, id: &str, bytes: &[u8]) -> Result<PathBuf, StoreError> {
        let dir = self.pack_dir(id).ok_or_else(|| StoreError::BadId(id.to_string()))?;
        let pack = self.manager.manifest().asset_pack(id).ok_or_else(|| PackError::UnknownPack(id.to_string()))?;
        if bytes.len() as u64 != pack.download_bytes {
            return Err(StoreError::SizeMismatch { id: id.to_string(), expected: pack.download_bytes, actual: bytes.len() as u64 });
        }
        fs::create_dir_all(&dir)?;
        let part = dir.join(format!("{PACK_FILE}.part"));
        fs::write(&part, bytes)?;
        fs::rename(&part, dir.join(PACK_FILE))?;
        self.manager.mark_downloaded(id)?;
        self.save()?;
        Ok(dir)
    }

    /// `remove(assetPackWithID:)`: deletes the files and returns the logical bytes freed (0 for a pack that was not
    /// installed).
    pub fn remove(&mut self, id: &str) -> Result<u64, StoreError> {
        let dir = self.pack_dir(id).ok_or_else(|| StoreError::BadId(id.to_string()))?;
        let freed = self.manager.remove(id)?;
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.save()?;
        Ok(freed)
    }

    /// `ensureLocalAvailability(of:)`: for each id, a pack that is installed at the current version is a success; a
    /// missing or outdated one is fetched with `fetch` (the bytes of the pack, or an error text) and installed. A
    /// fetch or install that fails, or an unknown id, is a failure for that id; the others still go on.
    pub fn ensure_local_availability<F>(&mut self, ids: &[&str], mut fetch: F) -> LocalAvailabilityError
    where
        F: FnMut(&AssetPack) -> Result<Vec<u8>, String>,
    {
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        for &id in ids {
            match self.manager.status(id) {
                Ok(PackStatus::Available) => successes.push(id.to_string()),
                Ok(_) => {
                    let pack = self.manager.manifest().asset_pack(id).cloned();
                    let installed = pack.and_then(|p| fetch(&p).ok()).map(|bytes| self.install(id, &bytes));
                    match installed {
                        Some(Ok(_)) => successes.push(id.to_string()),
                        _ => failures.push((id.to_string(), PackError::NotAvailable(id.to_string()))),
                    }
                }
                Err(e) => failures.push((id.to_string(), e)),
            }
        }
        LocalAvailabilityError { successes, failures }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(id: &str, version: u32, bytes: u64) -> AssetPack {
        AssetPack { id: id.into(), version, download_bytes: bytes, file_sizes: vec![bytes * 2], languages: Vec::new() }
    }

    fn manifest(version: u32) -> AssetPackManifest {
        AssetPackManifest::new(vec![pack("level1", version, 4), pack("skin", 1, 3)]).unwrap()
    }

    #[test]
    fn an_installed_pack_is_local_and_survives_reopening() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        assert!(!store.is_local("level1"));
        let dir = store.install("level1", b"abcd").unwrap();
        assert_eq!(fs::read(dir.join("pack.bin")).unwrap(), b"abcd");
        assert!(store.is_local("level1"));
        let again = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        assert!(again.is_local("level1"));
        assert!(!again.is_local("skin"));
    }

    #[test]
    fn a_newer_manifest_marks_the_installed_pack_as_updatable() {
        let tmp = tempfile::tempdir().unwrap();
        AssetPackStore::open(tmp.path(), manifest(1)).unwrap().install("level1", b"abcd").unwrap();
        let store = AssetPackStore::open(tmp.path(), manifest(2)).unwrap();
        assert_eq!(store.status("level1"), Ok(PackStatus::UpdateAvailable));
        assert!(!store.is_local("level1"));
    }

    #[test]
    fn install_checks_the_id_the_pack_and_the_size() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        assert_eq!(store.install("../x", b"abcd"), Err(StoreError::BadId("../x".into())));
        assert_eq!(store.install("nope", b"abcd"), Err(StoreError::Pack(PackError::UnknownPack("nope".into()))));
        assert_eq!(store.install("level1", b"ab"), Err(StoreError::SizeMismatch { id: "level1".into(), expected: 4, actual: 2 }));
        assert!(!store.is_local("level1"), "a refused install leaves nothing behind");
    }

    #[test]
    fn remove_deletes_the_files_and_reports_the_bytes_freed() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        let dir = store.install("level1", b"abcd").unwrap();
        assert_eq!(store.remove("level1"), Ok(8));
        assert!(!dir.exists());
        assert!(!store.is_local("level1"));
        assert_eq!(store.remove("level1"), Ok(0), "a pack that is not installed frees nothing");
    }

    #[test]
    fn availability_fetches_what_is_missing_and_reports_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        store.install("skin", b"xyz").unwrap();
        let mut fetched = Vec::new();
        let result = store.ensure_local_availability(&["skin", "level1", "ghost"], |pack| {
            fetched.push(pack.id.clone());
            Ok(b"abcd".to_vec())
        });
        assert_eq!(fetched, vec!["level1".to_string()], "the installed pack is not fetched again");
        assert_eq!(result.successes, vec!["skin".to_string(), "level1".to_string()]);
        assert_eq!(result.failures, vec![("ghost".to_string(), PackError::UnknownPack("ghost".into()))]);
        assert!(store.is_local("level1"));
    }

    #[test]
    fn a_failed_fetch_leaves_the_pack_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        let result = store.ensure_local_availability(&["level1"], |_| Err("offline".into()));
        assert!(result.successes.is_empty());
        assert_eq!(result.failures, vec![("level1".to_string(), PackError::NotAvailable("level1".into()))]);
        assert!(!store.is_local("level1"));
    }

    #[test]
    fn the_mock_source_serves_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = MockPackSource::new(manifest(1));
        source.add_fixture("level1", b"abcd".to_vec()).unwrap();
        let mut store = AssetPackStore::open(tmp.path(), manifest(1)).unwrap();
        let result = store.ensure_local_availability(&["level1"], |pack| source.fetch(&pack.id).map(|b| b.to_vec()).map_err(|e| format!("{e:?}")));
        assert_eq!(result.successes, vec!["level1".to_string()]);
    }
}
