//! Pull-only over-the-air updates, the bifrost way: a background check (shortly
//! after start-up, when Wi-Fi connects, then every 15 minutes; never on Low Power
//! Mode unless charging) looks at the OS manifest and at the store index for
//! newer versions of installed apps. A found update becomes one notification
//! banner (never repeated for the same version, remembered in the prefs) and
//! red badges: "1" on Settings, the number of app updates on the App Store.
//! No OS update is installed here; the user restarts from Settings > General >
//! Software Update. System packages (`kind = "system"`, e.g. the Keyboard
//! with its emoji font and word lists) are different: any that the store
//! lists and the bifrost lacks are installed, and newer ones updated, in the
//! background (always on, sha256 verified), silently like bifrost component
//! updates: no banner. The shell then hot-reloads what they provide
//! (`Shell::system_reload`). With Automatic Updates on (the default), App Store
//! apps update the same way, like Golden Gate "Install application updates from the
//! App Store": silently, and only while the app is not running; an open app
//! keeps its offer and updates on a later check, after it quits.
//! Network work runs on a worker thread only.

use super::notify::store::Notification;
use super::store::{api, install, net};
use super::update::{self, Manifest, Updater};
use super::{AppId, Shell};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// First check this long after start-up.
pub const STARTUP_DELAY_S: f64 = 30.0;
/// Then a check this often.
pub const INTERVAL_S: f64 = 15.0 * 60.0;
/// A Wi-Fi reconnect triggers a check only when the last one is older than this.
pub const MIN_GAP_S: f64 = 300.0;

/// The file in the data directory whose presence switches the background checks off
/// (`softwareupdate --schedule off`). Manual checks still run.
pub const SCHEDULE_OFF_FILE: &str = "update-schedule-off";

/// Whether the background checks run: on unless the schedule was switched off.
pub fn schedule_on(dir: &Path) -> bool {
    !dir.join(SCHEDULE_OFF_FILE).exists()
}

/// Switches the background checks on or off, in the data directory every updater shares.
pub fn set_schedule(dir: &Path, automatic: bool) -> std::io::Result<()> {
    let file = dir.join(SCHEDULE_OFF_FILE);
    if automatic {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&file, b"")
    }
}

pub const OS_KEY: &str = "ota_notified_os";
pub const APPS_KEY: &str = "ota_notified_apps";

/// When a background check is due. Times are seconds on any monotonic clock.
#[derive(Debug, Default)]
pub struct Schedule {
    last: Option<f64>,
    wifi_was: bool,
    want: bool,
}

impl Schedule {
    /// True when a check should start now; call it regularly.
    pub fn due(&mut self, now: f64, wifi: bool, low_power: bool, charging: bool) -> bool {
        if wifi && !self.wifi_was && self.last.is_none_or(|l| now - l >= MIN_GAP_S) {
            self.want = true;
        }
        self.wifi_was = wifi;
        if self.last.is_none() || self.last.is_some_and(|l| now - l >= INTERVAL_S) {
            self.want = true;
        }
        if !self.want || !wifi || now < STARTUP_DELAY_S || (low_power && !charging) {
            return false;
        }
        self.want = false;
        self.last = Some(now);
        true
    }
}

/// Whether an update identified by `key` still needs its banner.
pub fn should_notify(last: Option<&str>, key: &str) -> bool {
    !key.is_empty() && last != Some(key)
}

/// The identity of a set of app updates: sorted `id@version` pairs.
pub fn apps_key(apps: &[(String, String)]) -> String {
    let mut v: Vec<String> = apps.iter().map(|(id, ver)| format!("{id}@{ver}")).collect();
    v.sort();
    v.join(",")
}

/// A banner to post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub app: AppId,
    pub title: String,
    pub body: String,
    /// the value to remember under `OS_KEY` / `APPS_KEY` once posted
    pub key: String,
}

impl Notice {
    pub fn pref(&self) -> &'static str {
        if self.app == AppId::Settings { OS_KEY } else { APPS_KEY }
    }

    pub fn into_notification(self, time: i64) -> Notification {
        Notification::new(self.app, self.title, self.body, time)
    }
}

/// What one background check found; `None` means that half could not be checked.
#[derive(Debug, Default)]
pub struct Found {
    pub os: Option<Option<Manifest>>,
    /// (id, newer version) of installed apps
    pub apps: Option<Vec<(String, String)>>,
    /// ids among `apps` that are system packages (counted in the badge, never announced)
    pub system: Vec<String>,
    /// system packages installed or updated by this check, for the shell to reload
    pub installed: Vec<install::InstalledApp>,
}

/// Downloads and installs the planned system packages, skipping any that fail (retried next check).
/// Each failure goes to the install log in `log` (once per distinct message, see `install_log::record_once`).
fn install_system(root: &std::path::Path, log: &std::path::Path, plan: &[api::StoreApp]) -> Vec<install::InstalledApp> {
    plan.iter()
        .filter_map(|a| {
            let progress = std::sync::atomic::AtomicU64::new(0);
            let got = install::install_download(root, &a.id, &net::absolute(&a.package), a.size, &a.sha256, install::shell_version(), &progress, &std::sync::atomic::AtomicBool::new(false));
            let checked = got.and_then(|app| {
                if app.id != a.id || !app.is_preinstalled() {
                    let _ = install::uninstall(root, &app.id);
                    return Err("package id does not match the store entry".to_string());
                }
                Ok(app)
            });
            crate::install_log::logged(log, "system package", &a.id, checked).ok()
        })
        .collect()
}

/// App updates to install in the background: user apps (not system packages) with a newer store version that are not
/// running, so an open window is never restarted under the user.
fn app_plan<'a>(ups: &[&'a api::StoreApp], installed: &[install::InstalledApp], running: &[PathBuf]) -> Vec<&'a api::StoreApp> {
    ups.iter()
        .copied()
        .filter(|s| !s.is_preinstalled())
        .filter(|s| {
            let Some(app) = installed.iter().find(|i| i.id == s.id) else { return false };
            let home = app.dir.parent().unwrap_or(&app.dir);
            !running.iter().any(|exe| exe.starts_with(home))
        })
        .collect()
}

/// Executables of every running process, resolved (`/proc/<pid>/exe`).
fn running_exes() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return vec![] };
    dir.flatten().filter(|e| e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))).filter_map(|e| std::fs::read_link(e.path().join("exe")).ok()).collect()
}

/// Downloads and installs app updates, skipping any that fail (retried next check).
/// Each failure goes to the install log in `log`.
fn install_apps(root: &std::path::Path, log: &std::path::Path, plan: &[api::StoreApp]) -> Vec<install::InstalledApp> {
    plan.iter()
        .filter_map(|a| {
            let progress = std::sync::atomic::AtomicU64::new(0);
            let got = install::install_download(root, &a.id, &net::absolute(&a.package), a.size, &a.sha256, install::shell_version(), &progress, &std::sync::atomic::AtomicBool::new(false));
            crate::install_log::logged(log, "app update", &a.id, got).ok()
        })
        .collect()
}

/// Where the OS update is looked up: the OTA server, except on a desktop, where pacman answers and the server is not asked.
fn os_server(pacman: bool, url: &str) -> Option<&str> {
    (!pacman).then_some(url)
}

fn run_check(url: Option<&str>, version: &str, git: &str, auto_apps: bool, log: &std::path::Path) -> Found {
    let os = url.and_then(|u| update::fetch_manifest(u).ok()).map(|m| update::is_newer(&m, version, git).then_some(m));
    let root = PathBuf::from(install::ROOT);
    let cache = root.join("store-cache.json");
    let catalog = api::fetch(&cache).ok().or_else(|| api::load_cache(&cache));
    let mut installed = vec![];
    if let Some(catalog) = &catalog {
        let plan: Vec<api::StoreApp> = api::system_plan(catalog, &install::installed_apps(), install::shell_version(), &install::refused(&root)).into_iter().cloned().collect();
        installed = install_system(&root, log, &plan);
        if auto_apps {
            let have = install::installed_apps();
            let ups = api::updates(catalog, &have);
            let plan: Vec<api::StoreApp> = app_plan(&ups, &have, &running_exes()).into_iter().cloned().collect();
            installed.extend(install_apps(&root, log, &plan));
        }
    }
    let (apps, system) = match &catalog {
        Some(c) => {
            let ups = api::updates(c, &install::installed_apps());
            (Some(ups.iter().map(|s| (s.id.clone(), s.version.clone())).collect()), ups.iter().filter(|s| s.is_preinstalled()).map(|s| s.id.clone()).collect())
        }
        None => (None, vec![]),
    };
    Found { os, apps, system, installed }
}

/// Applies each system package's `provides` from its installed files.
pub fn reload_provided(apps: &[install::InstalledApp]) {
    for app in apps.iter().filter(|a| a.is_system()) {
        for what in &app.provides {
            if what == "keyboard" {
                crate::emoji::reload_from(&app.dir.join("assets/NotoColorEmoji.ttf"));
            }
        }
    }
}

pub struct Ota {
    /// the data directory, where the schedule switch lives
    dir: PathBuf,
    sched: Schedule,
    rx: Option<Receiver<Found>>,
    /// the OS update on offer
    pub os: Option<Manifest>,
    pub apps: Vec<(String, String)>,
    /// ids of `apps` that are system packages
    pub system: Vec<String>,
    /// system packages freshly installed, waiting for the shell to reload them
    reload: Vec<install::InstalledApp>,
    /// downloads an offered OS update in the background (Automatic Updates)
    pub updater: Updater,
    url: String,
    /// a banner is waiting to be posted
    fresh: bool,
    /// next time (ms) the shell should poll the checker
    next_poll: f64,
}

impl Ota {
    pub fn new(dir: PathBuf, url: Option<&str>) -> Self {
        let updater = Updater::new(dir.clone(), url);
        Ota { dir, sched: Schedule::default(), rx: None, os: None, apps: vec![], system: vec![], reload: vec![], url: updater.url().to_string(), updater, fresh: false, next_poll: 0.0 }
    }

    /// Once a second is plenty for the shell's 60 Hz tick.
    pub fn poll_due(&mut self, t_ms: f64) -> bool {
        if t_ms < self.next_poll {
            return false;
        }
        self.next_poll = t_ms + 1000.0;
        true
    }

    /// Advances the checker; true when badges or the offered updates changed.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(&mut self, now: f64, wifi: bool, low_power: bool, charging: bool, auto_download: bool, version: &str, git: &str) -> bool {
        let mut changed = self.updater.poll(version, git);
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(found) => {
                    self.rx = None;
                    changed |= self.absorb(found);
                    if auto_download && self.os.is_some() && !self.updater.busy() {
                        if let Some(m) = self.os.clone() {
                            self.updater.set_available(m);
                            self.updater.start_download();
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.rx = None,
            }
        } else if schedule_on(&self.dir) && self.sched.due(now, wifi, low_power, charging) {
            // a desktop's OS updates come from pacman; its store packages (the keyboard, other system packages) still come
            // from the store, so the store check runs on every platform
            let pacman = self.updater.source() == update::Source::Pacman;
            if pacman {
                self.updater.check();
            }
            let (tx, rx) = channel();
            self.rx = Some(rx);
            let (url, v, g, log) = (self.url.clone(), version.to_string(), git.to_string(), self.dir.clone());
            crate::bare::sys::spawn_named("ota-download", move || {
                crate::gfx::affinity::background_thread();
                let _ = tx.send(run_check(os_server(pacman, &url), &v, &g, auto_download, &log));
            });
        }
        changed
    }

    /// Takes a check's result; true when the badges changed.
    pub fn absorb(&mut self, found: Found) -> bool {
        let before = (self.os_badge(), self.apps.clone());
        if let Some(os) = found.os {
            self.os = os;
        }
        if let Some(apps) = found.apps {
            self.apps = apps;
            self.system = found.system;
        }
        self.reload.extend(found.installed);
        self.fresh = true;
        before != (self.os_badge(), self.apps.clone())
    }

    /// "1" on Settings while an OS update is on offer.
    pub fn os_badge(&self) -> usize {
        usize::from(self.os.is_some())
    }

    /// System packages the last check installed; take them to hot-reload what they provide.
    pub fn take_installed(&mut self) -> Vec<install::InstalledApp> {
        std::mem::take(&mut self.reload)
    }

    /// The App Store's badge: installed apps and system packages with a newer store version.
    pub fn app_badge(&self) -> usize {
        self.apps.len()
    }

    /// The user installs: the offer (and its badge) is gone.
    pub fn clear_os(&mut self) {
        self.os = None;
    }

    /// The banners to post for the latest check, given what was announced before.
    pub fn notices(&mut self, last_os: Option<&str>, last_apps: Option<&str>) -> Vec<Notice> {
        if !std::mem::take(&mut self.fresh) {
            return vec![];
        }
        let mut out = vec![];
        if let Some(m) = &self.os {
            if should_notify(last_os, &m.version) {
                out.push(Notice { app: AppId::Settings, title: "Software Update".into(), body: format!("BifrOSt {} is now available.", m.version), key: m.version.clone() });
            }
        }
        // system packages update silently: only user apps get a banner
        let user: Vec<(String, String)> = self.apps.iter().filter(|(id, _)| !self.system.contains(id)).cloned().collect();
        let key = apps_key(&user);
        if should_notify(last_apps, &key) {
            let n = user.len();
            out.push(Notice { app: AppId::AppStore, title: "App Store".into(), body: format!("{n} app update{} available", if n == 1 { "" } else { "s" }), key });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn manifest(v: &str) -> Manifest {
        Manifest { version: v.into(), build: None, sha256: SUM.into(), size: 1, url: String::new(), notes: None, changelog: vec![] }
    }

    fn ota() -> Ota {
        Ota::new(std::env::temp_dir().join("sb-ota-nonexistent"), None)
    }

    #[test]
    fn the_schedule_switch_is_a_file_in_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(schedule_on(dir.path()));
        set_schedule(dir.path(), false).unwrap();
        assert!(!schedule_on(dir.path()));
        set_schedule(dir.path(), false).unwrap();
        set_schedule(dir.path(), true).unwrap();
        assert!(schedule_on(dir.path()));
        set_schedule(dir.path(), true).unwrap();
        assert!(schedule_on(dir.path()), "turning on twice is fine");
    }

    #[test]
    fn a_switched_off_schedule_starts_no_background_check() {
        let dir = tempfile::tempdir().unwrap();
        let mut o = Ota::new(dir.path().to_path_buf(), None);
        set_schedule(dir.path(), false).unwrap();
        // wifi is up and the startup delay has passed: an enabled schedule would check now
        assert!(!o.tick(100.0, true, false, false, false, "0.1.0", "g"));
        assert!(o.rx.is_none(), "no background check while the schedule is off");
    }

    #[test]
    fn first_check_waits_for_the_startup_delay_and_wifi() {
        let mut s = Schedule::default();
        assert!(!s.due(5.0, true, false, false), "too early");
        assert!(!s.due(29.0, true, false, false));
        assert!(!s.due(31.0, false, false, false), "no Wi-Fi");
        assert!(s.due(32.0, true, false, false), "delay passed and online");
        assert!(!s.due(33.0, true, false, false), "once");
    }

    #[test]
    fn checks_repeat_every_fifteen_minutes() {
        assert_eq!(INTERVAL_S, 900.0);
        let mut s = Schedule::default();
        assert!(s.due(40.0, true, false, false));
        assert!(!s.due(40.0 + INTERVAL_S - 1.0, true, false, false));
        assert!(s.due(40.0 + INTERVAL_S, true, false, false));
    }

    #[test]
    fn a_wifi_connection_triggers_a_check_but_not_a_flapping_one() {
        let mut s = Schedule::default();
        assert!(s.due(40.0, true, false, false));
        assert!(!s.due(100.0, false, false, false));
        assert!(!s.due(110.0, true, false, false), "reconnect within the minimum gap");
        assert!(!s.due(120.0, false, false, false));
        let mut s = Schedule::default();
        assert!(s.due(40.0, true, false, false));
        assert!(!s.due(500.0, false, false, false));
        assert!(s.due(1000.0, true, false, false), "reconnect long after the last check");
    }

    #[test]
    fn a_connection_made_before_the_startup_delay_still_checks_after_it() {
        let mut s = Schedule::default();
        assert!(!s.due(2.0, true, false, false));
        assert!(s.due(30.0, true, false, false));
    }

    #[test]
    fn low_power_mode_skips_unless_charging_and_the_check_stays_pending() {
        let mut s = Schedule::default();
        assert!(!s.due(60.0, true, true, false));
        assert!(!s.due(600.0, true, true, false));
        assert!(s.due(601.0, true, true, true), "charging overrides");
        let mut s = Schedule::default();
        assert!(!s.due(60.0, true, true, false));
        assert!(s.due(61.0, true, false, false), "back to normal: the missed check runs");
    }

    #[test]
    fn a_banner_is_never_repeated_for_the_same_version() {
        assert!(should_notify(None, "0.2.0") && should_notify(Some("0.1.9"), "0.2.0"));
        assert!(!should_notify(Some("0.2.0"), "0.2.0") && !should_notify(None, ""));
        let mut o = ota();
        o.os = Some(manifest("0.2.0"));
        o.fresh = true;
        let n = o.notices(None, None);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].app, n[0].title.as_str(), n[0].body.as_str(), n[0].pref()), (AppId::Settings, "Software Update", "BifrOSt 0.2.0 is now available.", OS_KEY));
        o.fresh = true;
        assert!(o.notices(Some("0.2.0"), None).is_empty(), "announced already");
        assert!(o.notices(None, None).is_empty(), "no new check, no banner");
        o.fresh = true;
        o.os = Some(manifest("0.3.0"));
        assert_eq!(o.notices(Some("0.2.0"), None).len(), 1, "a newer version is announced again");
    }

    #[test]
    fn app_updates_are_keyed_by_the_set_of_versions() {
        let a = vec![("b".to_string(), "1.1.0".to_string()), ("a".to_string(), "2.0.0".to_string())];
        assert_eq!(apps_key(&a), "a@2.0.0,b@1.1.0");
        let mut o = ota();
        o.apps = a.clone();
        o.fresh = true;
        let n = o.notices(None, None);
        assert_eq!((n[0].app, n[0].title.as_str(), n[0].body.as_str()), (AppId::AppStore, "App Store", "2 app updates available"));
        o.fresh = true;
        assert!(o.notices(None, Some(&n[0].key)).is_empty());
        o.apps = vec![("a".into(), "2.0.0".into())];
        o.fresh = true;
        let n = o.notices(None, Some("a@2.0.0,b@1.1.0"));
        assert_eq!(n[0].body, "1 app update available");
    }

    #[test]
    fn badges_follow_the_offers_and_clear_on_install() {
        let mut o = ota();
        assert_eq!((o.os_badge(), o.app_badge()), (0, 0));
        assert!(o.absorb(Found { os: Some(Some(manifest("0.2.0"))), apps: Some(vec![("a".into(), "1.0.1".into()), ("b".into(), "2.0.0".into())]), ..Default::default() }));
        assert_eq!((o.os_badge(), o.app_badge()), (1, 2));
        assert!(!o.absorb(Found::default()), "a failed check changes nothing");
        assert_eq!((o.os_badge(), o.app_badge()), (1, 2));
        o.clear_os();
        assert_eq!(o.os_badge(), 0);
        assert!(o.absorb(Found { os: Some(None), apps: Some(vec![]), ..Default::default() }) || o.app_badge() == 0);
        assert_eq!(o.app_badge(), 0);
    }
}

#[cfg(test)]
mod system_tests {
    #[test]
    fn a_desktop_skips_the_os_server_but_still_checks_the_store() {
        // pacman answers for the OS; the store must still be asked, or system packages (the keyboard) never install
        assert_eq!(os_server(true, "http://x/update.json"), None);
        assert_eq!(os_server(false, "http://x/update.json"), Some("http://x/update.json"));
    }

    use super::*;

    fn ota() -> Ota {
        Ota::new(std::env::temp_dir().join("sb-ota-nonexistent"), None)
    }

    #[test]
    fn app_updates_install_in_the_background_only_while_the_app_is_closed() {
        let store = |id: &str, kind: &str| api::StoreApp { id: id.into(), version: "2.0.0".into(), kind: kind.into(), ..Default::default() };
        let have = |id: &str| install::InstalledApp { id: id.into(), version: "1.0.0".into(), dir: PathBuf::from(format!("/var/lib/springboard/apps/{id}/current")), ..Default::default() };
        let (notes, calc, kb) = (store("is.o.notes", "app"), store("is.o.calc", "app"), store("is.o.kb", "system"));
        let ups = vec![&notes, &calc, &kb];
        let installed = vec![have("is.o.notes"), have("is.o.calc"), have("is.o.kb")];
        // Notes runs from its versioned folder (the resolved /proc/<pid>/exe), so it waits; the system package is not ours
        let running = vec![PathBuf::from("/var/lib/springboard/apps/is.o.notes/1.0.0/bifrost-notes")];
        let plan: Vec<&str> = app_plan(&ups, &installed, &running).iter().map(|s| s.id.as_str()).collect();
        assert_eq!(plan, ["is.o.calc"]);
        assert_eq!(app_plan(&ups, &installed, &[]).len(), 2, "once Notes quits it updates too");
    }

    #[test]
    fn the_background_plan_ignores_updates_built_for_another_cpu() {
        let x86 = api::StoreApp { id: "is.o.calc".into(), version: "2.0.0".into(), architecture: "x86_64-linux".into(), ..Default::default() };
        let installed = vec![install::InstalledApp { id: "is.o.calc".into(), version: "1.0.0".into(), dir: PathBuf::from("/var/lib/springboard/apps/is.o.calc/current"), ..Default::default() }];
        let plan = |host: &str| {
            let catalog = [x86.clone()];
            let ups = api::updates_for(&catalog, &installed, 0, host);
            app_plan(&ups, &installed, &[]).len()
        };
        assert_eq!(plan("aarch64-linux"), 0, "a tablet never downloads the x86 package");
        assert_eq!(plan("x86_64-linux"), 1, "a PC still updates");
    }

    #[test]
    fn system_updates_count_in_the_badge_but_never_get_a_banner() {
        let mut o = ota();
        let found = Found { apps: Some(vec![("is.o.kb".into(), "1.1.0".into())]), system: vec!["is.o.kb".into()], ..Default::default() };
        assert!(o.absorb(found));
        assert_eq!(o.app_badge(), 1);
        assert!(o.notices(None, None).is_empty(), "silent, like bifrost component updates");
        let mixed = Found { apps: Some(vec![("is.o.kb".into(), "1.1.0".into()), ("is.o.calc".into(), "2.0.0".into())]), system: vec!["is.o.kb".into()], ..Default::default() };
        o.absorb(mixed);
        assert_eq!(o.app_badge(), 2);
        let n = o.notices(None, None);
        assert_eq!((n.len(), n[0].body.as_str(), n[0].key.as_str()), (1, "1 app update available", "is.o.calc@2.0.0"));
    }

    #[test]
    fn freshly_installed_system_packages_are_handed_over_once() {
        let mut o = ota();
        let app = install::InstalledApp { id: "is.o.kb".into(), kind: "system".into(), provides: vec!["keyboard".into()], ..Default::default() };
        o.absorb(Found { installed: vec![app.clone()], ..Default::default() });
        assert_eq!(o.take_installed(), vec![app]);
        assert!(o.take_installed().is_empty());
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn manifest(v: &str) -> Manifest {
        Manifest { version: v.into(), build: None, sha256: SUM.into(), size: 1, url: String::new(), notes: None, changelog: vec![] }
    }

    fn ota() -> (tempfile::TempDir, Ota) {
        let dir = tempfile::tempdir().unwrap();
        let o = Ota::new(dir.path().to_path_buf(), None);
        (dir, o)
    }

    #[test]
    fn an_empty_key_or_the_remembered_key_never_notifies() {
        assert!(!should_notify(None, ""));
        assert!(!should_notify(Some("1.0"), "1.0"));
        assert!(should_notify(Some("1.0"), "1.1"));
        assert!(should_notify(None, "1.0"));
    }

    #[test]
    fn apps_key_is_order_independent_and_empty_for_no_updates() {
        let a = vec![("b".to_string(), "1".to_string()), ("a".to_string(), "2".to_string())];
        let b = vec![a[1].clone(), a[0].clone()];
        assert_eq!(apps_key(&a), "a@2,b@1");
        assert_eq!(apps_key(&a), apps_key(&b));
        assert_eq!(apps_key(&[]), "");
    }

    #[test]
    fn notices_route_to_the_prefs_that_remember_them() {
        let n = |app| Notice { app, title: String::new(), body: String::new(), key: "k".into() };
        assert_eq!(n(AppId::Settings).pref(), OS_KEY);
        assert_eq!(n(AppId::AppStore).pref(), APPS_KEY);
    }

    #[test]
    fn schedule_does_not_fire_exactly_at_the_interval_boundary_early() {
        let mut s = Schedule::default();
        assert!(s.due(STARTUP_DELAY_S, true, false, false));
        assert!(!s.due(STARTUP_DELAY_S + INTERVAL_S - 1.0, true, false, false), "not yet due");
        assert!(s.due(STARTUP_DELAY_S + INTERVAL_S, true, false, false), "due at the interval");
    }

    #[test]
    fn a_check_without_wifi_stays_pending_until_wifi_returns() {
        let mut s = Schedule::default();
        assert!(!s.due(100.0, false, false, false));
        assert!(!s.due(101.0, false, false, false));
        assert!(s.due(102.0, true, false, false));
        assert!(!s.due(103.0, true, false, false), "fired once");
    }

    #[test]
    fn absorb_keeps_the_previous_offers_when_a_half_could_not_be_checked() {
        let (_d, mut o) = ota();
        o.absorb(Found { os: Some(Some(manifest("1.0.0"))), apps: Some(vec![("a".into(), "1".into())]), ..Default::default() });
        assert_eq!((o.os_badge(), o.app_badge()), (1, 1));
        // offline: both halves None, nothing changes
        assert!(!o.absorb(Found::default()));
        assert_eq!((o.os_badge(), o.app_badge()), (1, 1));
        // a successful check with no OS update clears the OS offer
        assert!(o.absorb(Found { os: Some(None), ..Default::default() }));
        assert_eq!((o.os_badge(), o.app_badge()), (0, 1));
    }

    #[test]
    fn absorb_reports_a_change_only_when_the_badges_change() {
        let (_d, mut o) = ota();
        let f = || Found { os: Some(Some(manifest("1.0.0"))), apps: Some(vec![("a".into(), "1".into())]), ..Default::default() };
        assert!(o.absorb(f()));
        assert!(!o.absorb(f()), "same offers again");
        assert!(o.absorb(Found { apps: Some(vec![("a".into(), "2".into())]), ..Default::default() }), "new version of the same app");
    }

    #[test]
    fn the_os_banner_names_the_version_and_is_posted_once() {
        let (_d, mut o) = ota();
        o.absorb(Found { os: Some(Some(manifest("0.6.0"))), ..Default::default() });
        let n = o.notices(None, None);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].app, n[0].body.as_str(), n[0].key.as_str()), (AppId::Settings, "BifrOSt 0.6.0 is now available.", "0.6.0"));
        assert!(o.notices(None, None).is_empty(), "no new check, no banner");
        o.absorb(Found { os: Some(Some(manifest("0.6.0"))), ..Default::default() });
        assert!(o.notices(Some("0.6.0"), None).is_empty(), "already announced");
        o.absorb(Found { os: Some(Some(manifest("0.6.1"))), ..Default::default() });
        assert_eq!(o.notices(Some("0.6.0"), None).len(), 1, "a newer version is announced again");
    }

    #[test]
    fn app_banner_pluralises() {
        let (_d, mut o) = ota();
        o.absorb(Found { apps: Some(vec![("a".into(), "1".into()), ("b".into(), "1".into())]), ..Default::default() });
        assert_eq!(o.notices(None, None)[0].body, "2 app updates available");
    }

    #[test]
    fn clear_os_drops_the_offer_and_poll_due_throttles_to_once_a_second() {
        let (_d, mut o) = ota();
        o.absorb(Found { os: Some(Some(manifest("1.0.0"))), ..Default::default() });
        o.clear_os();
        assert_eq!(o.os_badge(), 0);
        assert!(o.poll_due(0.0));
        assert!(!o.poll_due(999.0));
        assert!(o.poll_due(1000.0));
    }

    #[test]
    fn a_staged_binary_on_disk_is_reported_by_the_updater_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("springboard.new"), b"x").unwrap();
        let o = Ota::new(dir.path().to_path_buf(), None);
        assert!(matches!(o.updater.state, update::State::Staged(_)));
    }
}

impl Shell {
    /// Reloads what freshly installed system packages provide, without a restart:
    /// `keyboard` swaps the emoji font and word lists (the keyboard globe re-checks
    /// `emoji::available()` on every press, text caches follow `emoji::generation()`).
    pub(super) fn system_reload(&mut self, apps: &[install::InstalledApp]) {
        reload_provided(apps);
        // a system-app brings a home-screen icon: pick it up without a restart
        if apps.iter().any(|a| !a.exec_path.as_os_str().is_empty()) && super::apphost::refresh() {
            self.installed_changed();
        }
        self.dirty = true;
    }

    /// Starts the background checker once the data directory (and prefs) are loaded.
    pub(super) fn ota_enable(&mut self) {
        self.ota = Some(Ota::new(self.data_dir.clone(), self.prefs.get(update::URL_PREF)));
    }

    /// Drives the checker from the shell's tick: posts banners and keeps the badges current.
    pub(super) fn ota_tick(&mut self, t: f64) {
        // apps installed or updated by another process (`--install-store-app` over ssh)
        if t - self.disk_check_ms >= 1000.0 {
            self.disk_check_ms = t;
            if super::apphost::disk_changed() && super::apphost::refresh() {
                self.installed_changed();
                if let Some(st) = self.store.as_mut() {
                    st.refresh_installed();
                }
                self.dirty = true;
            }
        }
        // a system package updated from the App Store's UPDATE button
        let from_store = self.store.as_mut().map(|s| s.take_system_installed()).unwrap_or_default();
        if !from_store.is_empty() {
            self.system_reload(&from_store);
        }
        if !self.ota.as_mut().is_some_and(|o| o.poll_due(t)) {
            return;
        }
        let Some(mut ota) = self.ota.take() else { return };
        let o = self.options();
        let changed = ota.tick(t / 1000.0, self.info.online, o.low_power, self.info.charging, o.auto_update, super::pages::BUILD.version, super::pages::BUILD.git);
        let (last_os, last_apps) = (self.prefs.get(OS_KEY).map(str::to_string), self.prefs.get(APPS_KEY).map(str::to_string));
        let reload = ota.take_installed();
        if !reload.is_empty() {
            self.system_reload(&reload);
            if let Some(st) = self.store.as_mut() {
                st.refresh_installed();
            }
        }
        let notices = ota.notices(last_os.as_deref(), last_apps.as_deref());
        if changed || !notices.is_empty() {
            self.notify.extra = [(AppId::Settings, ota.os_badge()), (AppId::AppStore, ota.app_badge())].into_iter().filter(|e| e.1 > 0).collect();
            self.store_changed();
            let ui = &self.ui;
            if let Some(st) = self.settings.as_mut() {
                st.set_update_badge(ui, ota.os_badge() > 0);
            }
        }
        for n in notices {
            let (pref, key) = (n.pref(), n.key.clone());
            if let Err(e) = self.prefs.set(pref, &key) {
                bifrost_core::log::shell_logger("ota").error(format!("settings.ini: {e}"));
            }
            self.notify(n.into_notification(crate::bare::sys::wall_time().0));
        }
        let ui = &self.ui;
        if let Some(st) = self.settings.as_mut() {
            st.mirror_update(ui, &ota.updater);
        }
        self.ota = Some(ota);
    }
}
