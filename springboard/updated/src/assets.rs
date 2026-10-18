//! Background asset pack downloads for the update agent (the asset daemon). Each app has one managed download queue
//! (`bifrost_ui::managed_download`), which owns the queue rules: budget, metering, priority and three attempts. This
//! module keeps one queue per app, starts a new budget day, and decides when a background pass may run at all.

use bifrost_core::asset_pack_file::{self, id_ok};
use bifrost_core::container_layout::ContainerLayout;
use bifrost_ui::managed_download::{Conditions, ManagedDownloads, RequestError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Bytes an app may fetch in the background each day.
pub const DAILY_BUDGET: u64 = 512 * 1024 * 1024;

/// The asset daemon: the managed download queues of all apps, run while the machine is idle.
#[derive(Default)]
pub struct AssetDaemon {
    queues: BTreeMap<String, ManagedDownloads>,
    day: u64,
}

impl AssetDaemon {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `pack` for `app`. Refused when that pack is already queued, downloading or done.
    pub fn request(&mut self, app: &str, pack: &str, bytes: u64, priority: u8) -> Result<(), RequestError> {
        self.queues.entry(app.to_string()).or_insert_with(|| ManagedDownloads::new(DAILY_BUDGET)).request(pack, bytes, priority)
    }

    /// The queue of `app`, if it has ever requested a pack.
    pub fn queue(&self, app: &str) -> Option<&ManagedDownloads> {
        self.queues.get(app)
    }

    /// Runs every app's queue under `conditions` until nothing more may start. `day` is the current day number (days
    /// since the Unix epoch): when it changes, every queue gets its budget back. `transfer` is called with the app and
    /// pack id and returns true when the pack arrived. Returns how many packs finished.
    pub fn pass<F: FnMut(&str, &str) -> bool>(&mut self, day: u64, conditions: Conditions, mut transfer: F) -> usize {
        if day != self.day {
            self.day = day;
            for q in self.queues.values_mut() {
                q.reset_day();
            }
        }
        let mut done = 0;
        for (app, q) in self.queues.iter_mut() {
            done += q.run_all(conditions, |pack| transfer(app.as_str(), pack));
        }
        done
    }
}

/// The conditions of a background pass from the update agent's state: the machine must be on an unmetered (Wi-Fi or
/// wired) connection, and a pass never runs in low-power mode. `None` means no pass may run.
pub fn background_conditions(wifi: bool, low_power: bool) -> Option<Conditions> {
    if low_power || !wifi {
        return None;
    }
    Some(Conditions { network_available: true, unmetered: true, app_running: false })
}

/// Where packs come from: a folder on this machine, `<root>/<app id>/<pack id>.bfpk` (the system's own pack folder or
/// one an administrator or the user filled, for example from a disk). Nothing is fetched from a third party; a pack that
/// is not in the folder simply fails its attempts and stays visible as failed. Installed packs land in the app's
/// container under `<data>/appdata/<app id>`.
pub struct LocalPackSource {
    root: PathBuf,
    data: PathBuf,
}

/// The system pack folder.
pub const DEFAULT_SOURCE: &str = "/usr/share/bifrost/asset-packs";

impl LocalPackSource {
    pub fn new(root: impl Into<PathBuf>, data_dir: &Path) -> Self {
        LocalPackSource { root: root.into(), data: data_dir.join("appdata") }
    }

    /// The pack file for `pack` of `app`, or `None` for an id that is not a plain identifier.
    pub fn path_of(&self, app: &str, pack: &str) -> Option<PathBuf> {
        (id_ok(app) && id_ok(pack)).then(|| self.root.join(app).join(format!("{pack}.bfpk")))
    }

    /// The pack ids available for `app`, sorted.
    pub fn available(&self, app: &str) -> Vec<String> {
        if !id_ok(app) {
            return Vec::new();
        }
        let mut ids: Vec<String> = std::fs::read_dir(self.root.join(app))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| asset_pack_file::pack_id_of_file(&e.path()))
            .collect();
        ids.sort();
        ids
    }

    /// The transfer the asset daemon calls: reads the pack file, verifies it (digest, safe names, the id matches the
    /// request) and installs it into the app's container. True when the pack is installed; a missing, damaged or
    /// mismatched pack is false so the queue counts the attempt. A pack already installed at the same version counts as
    /// done without being read again.
    pub fn transfer(&self, app: &str, pack: &str) -> bool {
        let Some(path) = self.path_of(app, pack) else { return false };
        let Ok(bytes) = std::fs::read(&path) else { return false };
        let Ok(file) = asset_pack_file::decode(&bytes) else { return false };
        if file.id != pack {
            return false;
        }
        let layout = ContainerLayout::new(self.data.join(app));
        if asset_pack_file::installed_version(&layout, pack) == Some(file.version) {
            return true;
        }
        asset_pack_file::install_versioned(&layout, &file).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::asset_pack_file::{encode, PackFile};
    use bifrost_ui::managed_download::PackState;

    const WIRED: Conditions = Conditions { network_available: true, unmetered: true, app_running: false };
    const METERED: Conditions = Conditions { network_available: true, unmetered: false, app_running: false };

    #[test]
    fn a_wired_pass_fetches_the_queued_pack() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let done = d.pass(1, WIRED, |app, pack| {
            assert_eq!((app, pack), ("levels", "maps-eu"));
            true
        });
        assert_eq!(done, 1);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Done));
    }

    #[test]
    fn a_metered_pass_fetches_nothing() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let mut called = false;
        assert_eq!(d.pass(1, METERED, |_, _| {
            called = true;
            true
        }), 0);
        assert!(!called);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Queued));
    }

    #[test]
    fn a_duplicate_request_is_refused() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        assert_eq!(d.request("levels", "maps-eu", 10, 1), Err(RequestError::Duplicate));
    }

    #[test]
    fn a_failing_pack_gets_three_attempts_then_fails() {
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        let mut attempts = 0;
        assert_eq!(d.pass(1, WIRED, |_, _| {
            attempts += 1;
            false
        }), 0);
        assert_eq!(attempts, 3);
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Failed));
    }

    #[test]
    fn the_budget_returns_on_a_new_day() {
        let mut d = AssetDaemon::new();
        d.request("levels", "big", DAILY_BUDGET, 1).unwrap();
        d.request("levels", "next", 1, 1).unwrap();
        assert_eq!(d.pass(1, WIRED, |_, _| true), 1);
        assert_eq!(d.queue("levels").unwrap().state("next"), Some(PackState::Queued));
        assert_eq!(d.pass(2, WIRED, |_, _| true), 1);
        assert_eq!(d.queue("levels").unwrap().state("next"), Some(PackState::Done));
    }

    #[test]
    fn background_passes_stop_in_low_power_and_on_metered_links() {
        assert_eq!(background_conditions(true, false), Some(WIRED));
        assert_eq!(background_conditions(true, true), None);
        assert_eq!(background_conditions(false, false), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("assets-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put_pack(root: &Path, app: &str, id: &str, version: u32) {
        let pack = PackFile { id: id.into(), version, files: vec![("a.txt".into(), b"data".to_vec())] };
        std::fs::create_dir_all(root.join(app)).unwrap();
        std::fs::write(root.join(app).join(format!("{id}.bfpk")), encode(&pack).unwrap()).unwrap();
    }

    #[test]
    fn a_pack_from_the_local_source_is_installed_in_the_app_container() {
        let base = scratch("install");
        let (src, data) = (base.join("src"), base.join("data"));
        put_pack(&src, "levels", "maps-eu", 2);
        let source = LocalPackSource::new(&src, &data);
        assert_eq!(source.available("levels"), vec!["maps-eu".to_string()]);
        let mut d = AssetDaemon::new();
        d.request("levels", "maps-eu", 10, 1).unwrap();
        assert_eq!(d.pass(1, WIRED, |app, pack| source.transfer(app, pack)), 1);
        let installed = data.join("appdata/levels/Library/Caches/AssetPacks/maps-eu/a.txt");
        assert_eq!(std::fs::read(installed).unwrap(), b"data");
        assert_eq!(d.queue("levels").unwrap().state("maps-eu"), Some(PackState::Done));
    }

    #[test]
    fn a_missing_damaged_or_misnamed_pack_fails_its_attempts() {
        let base = scratch("fail");
        let (src, data) = (base.join("src"), base.join("data"));
        let source = LocalPackSource::new(&src, &data);
        assert!(!source.transfer("levels", "absent"));
        put_pack(&src, "levels", "maps-eu", 1);
        let path = source.path_of("levels", "maps-eu").unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[8] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();
        assert!(!source.transfer("levels", "maps-eu"));
        // a valid pack stored under another pack's name is not accepted for it
        put_pack(&src, "levels", "other", 1);
        std::fs::rename(source.path_of("levels", "other").unwrap(), source.path_of("levels", "maps-eu").unwrap()).unwrap();
        assert!(!source.transfer("levels", "maps-eu"));
    }

    #[test]
    fn ids_that_could_leave_the_source_folder_are_refused() {
        let source = LocalPackSource::new("/srv/packs", Path::new("/data"));
        assert!(source.path_of("../etc", "x").is_none());
        assert!(source.path_of("app", "a/b").is_none());
        assert!(!source.transfer("app", ".."));
        assert!(source.available("..").is_empty());
    }

    #[test]
    fn a_pack_at_the_installed_version_is_accepted_and_a_newer_one_replaces_it() {
        let base = scratch("current");
        let (src, data) = (base.join("src"), base.join("data"));
        put_pack(&src, "levels", "maps-eu", 5);
        let source = LocalPackSource::new(&src, &data);
        assert!(source.transfer("levels", "maps-eu"));
        assert!(source.transfer("levels", "maps-eu"));
        let layout = ContainerLayout::new(data.join("appdata/levels"));
        assert_eq!(asset_pack_file::installed_version(&layout, "maps-eu"), Some(5));
        put_pack(&src, "levels", "maps-eu", 6);
        assert!(source.transfer("levels", "maps-eu"));
        assert_eq!(asset_pack_file::installed_version(&layout, "maps-eu"), Some(6));
    }
}
