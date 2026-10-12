//! The workspace (`bifrost_app::workspace`) on the shell side: what an app may open, reveal and recycle, and the events it
//! hears. Apps name files by document grants they hold and apps by installed id; the shell never takes a path or a command
//! from an app, so an app cannot reach a file it was not granted or start a binary that is not an installed app.
//!
//! The decisions are plain functions (`route_file`, `route_url`, `launchable`, `trash_into`, `reveal_target`) and the
//! subscription bookkeeping is `State`, so they are tested without a running shell.

use super::apphost;
use bifrost_proto::{GrantId, ToApp, ToShell, WorkspaceEventKind, WorkspaceOp, WorkspaceStatus};
use bifrost_ui::launch::Registry;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The Files app, which Reveal opens.
pub const FILES_APP: &str = "is.olibuijr.files.desktop";
/// The file in Recently Deleted that records where each item came from (Files reads it for Put Back).
const ORIGINS: &str = ".origins";

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// the default browser
    Web,
    /// the app (registry key) that declares the scheme
    App(String),
    /// no app declares the scheme
    NoHandler,
    /// never opened for an app: control characters, no scheme, or a scheme that would read local data
    Refused,
}

/// Schemes an app may not open through the workspace: they reach local files or run code instead of naming a service.
const REFUSED_SCHEMES: &[&str] = &["file", "javascript", "data", "blob", "vbscript", "about"];

/// The route for `url`: web links go to the browser, any other scheme to the app the registry names for it.
pub fn route_url(registry: &Registry, url: &str) -> Route {
    if url.is_empty() || url.chars().any(char::is_control) {
        return Route::Refused;
    }
    let Some((scheme, rest)) = url.split_once(':') else { return Route::Refused };
    let scheme = scheme.to_ascii_lowercase();
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) || REFUSED_SCHEMES.contains(&scheme.as_str()) {
        return Route::Refused;
    }
    if (scheme == "http" || scheme == "https") && rest.starts_with("//") {
        return Route::Web;
    }
    match registry.default_for_scheme(&scheme) {
        Some(app) => Route::App(app.to_string()),
        None => Route::NoHandler,
    }
}

/// The app that opens a file called `name`: the user's choice for its type (Default Apps), else the best declaration.
pub fn route_file(registry: &Registry, name: &str) -> Option<String> {
    registry.default_for_file(name).map(str::to_string)
}

/// True when a package of this `kind` is an app a person launches (`app`, `system-app`); services and system packages
/// are never started on an app's say-so.
pub fn launchable(kind: &str) -> bool {
    super::kinds::Kind::of(kind).on_home_grid()
}

/// The folder Reveal opens for `path`: the folder itself, or the folder that holds the file.
pub fn reveal_target(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map_or_else(|| path.to_path_buf(), Path::to_path_buf)
    }
}

/// Moves each of `paths` into the Recently Deleted folder `bin`, as Files does (`trash::discard`, with the origin recorded
/// so Put Back works). The home folder itself and the trash are never moved. One result per path.
pub fn trash_into(home: &Path, paths: &[PathBuf], bin: &Path) -> Vec<std::io::Result<PathBuf>> {
    crate::files::trash::purge(bin, crate::files::trash::now_secs());
    paths
        .iter()
        .map(|p| {
            if p == home || p.starts_with(bin) || p.file_name().is_none() {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not movable"));
            }
            let stored = crate::files::trash::discard(p, bin, crate::files::trash::now_secs())?;
            if let (Some(name), Some(parent)) = (stored.file_name().and_then(|n| n.to_str()), p.parent().and_then(Path::to_str)) {
                if !parent.contains('\t') && !parent.contains('\n') {
                    record_origin(bin, name, parent);
                }
            }
            Ok(stored)
        })
        .collect()
}

fn record_origin(bin: &Path, stored: &str, origin: &str) {
    let mut lines: Vec<(String, String)> = std::fs::read_to_string(bin.join(ORIGINS))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string())))
        .collect();
    lines.retain(|(n, _)| n != stored);
    lines.push((stored.to_string(), origin.to_string()));
    let text: String = lines.iter().map(|(a, b)| format!("{a}\t{b}\n")).collect();
    let _ = super::fsutil::write_atomic(&bin.join(ORIGINS), text);
}

/// Who hears which events, and which apps are running (so a launch or an exit is reported once).
#[derive(Default)]
pub struct State {
    subs: HashMap<u8, u32>,
    live: BTreeSet<u8>,
}

impl State {
    /// The app in `slot` wants the events in `mask` (replaces its earlier mask; 0 unsubscribes).
    pub fn subscribe(&mut self, slot: u8, mask: u32) {
        let mask = mask & bifrost_proto::workspace::ALL_EVENTS;
        if mask == 0 {
            self.subs.remove(&slot);
        } else {
            self.subs.insert(slot, mask);
        }
    }

    /// The slots that subscribed to `kind`, lowest first.
    pub fn listeners(&self, kind: WorkspaceEventKind) -> Vec<u8> {
        let mut v: Vec<u8> = self.subs.iter().filter(|(_, m)| **m & kind.bit() != 0).map(|(s, _)| *s).collect();
        v.sort_unstable();
        v
    }

    /// The app in `slot` started; true the first time (a second report for a running app is not a launch).
    pub fn launched(&mut self, slot: u8) -> bool {
        self.live.insert(slot)
    }

    /// The app in `slot` ended; true when it was running. Its subscription ends with it.
    pub fn gone(&mut self, slot: u8) -> bool {
        self.subs.remove(&slot);
        self.live.remove(&slot)
    }

    pub fn is_live(&self, slot: u8) -> bool {
        self.live.contains(&slot)
    }
}

type Reply = (WorkspaceStatus, Vec<String>);

fn denied() -> Reply {
    (WorkspaceStatus::Denied, Vec::new())
}

/// Shell side.
impl super::Shell {
    /// A workspace message from the app in `slot` (`rich_msg` hands them over).
    pub(super) fn workspace_msg(&mut self, slot: u8, msg: ToShell, t: f64) {
        match msg {
            ToShell::WorkspaceSubscribe { mask } => self.app_workspace.subscribe(slot, mask),
            ToShell::WorkspaceRequest { request, op, grants, arg } => {
                let (status, items) = self.workspace_run(slot, op, &grants, &arg, t);
                self.ext_send(slot as usize, &ToApp::WorkspaceReply { request, status, items });
            }
            _ => {}
        }
    }

    fn workspace_run(&mut self, slot: u8, op: WorkspaceOp, grants: &[GrantId], arg: &str, t: f64) -> Reply {
        let installed = apphost::installed();
        let Some(app_id) = installed.get(slot as usize).map(|a| a.manifest.id.clone()) else { return denied() };
        match op {
            WorkspaceOp::Frontmost => {
                let front = self.ext_fg.and_then(|n| installed.get(n as usize)).map(|a| a.manifest.id.clone());
                (WorkspaceStatus::Ok, front.into_iter().collect())
            }
            WorkspaceOp::Running => {
                let ids = self.ext.iter().enumerate().filter(|(_, h)| h.as_ref().is_some_and(|h| h.running())).filter_map(|(i, _)| installed.get(i)).filter(|a| launchable(&a.manifest.kind)).map(|a| a.manifest.id.clone()).collect();
                (WorkspaceStatus::Ok, ids)
            }
            WorkspaceOp::UrlForApp => match installed.iter().find(|a| a.manifest.id == arg && launchable(&a.manifest.kind)) {
                Some(a) => (WorkspaceStatus::Ok, vec![a.manifest.id.clone()]),
                None => (WorkspaceStatus::NotFound, Vec::new()),
            },
            WorkspaceOp::OpenUrl => self.workspace_open_url(arg, t),
            WorkspaceOp::OpenFile | WorkspaceOp::OpenWith | WorkspaceOp::Reveal | WorkspaceOp::Recycle => {
                #[cfg(feature = "desktop-wayland")]
                {
                    self.workspace_files(slot, &app_id, op, grants, arg, t)
                }
                #[cfg(not(feature = "desktop-wayland"))]
                {
                    let _ = (app_id, grants, arg, t);
                    (WorkspaceStatus::Unsupported, Vec::new())
                }
            }
        }
    }

    pub(super) fn workspace_open_url(&mut self, url: &str, t: f64) -> Reply {
        self.ensure_defaults();
        let route = match &self.defaults {
            Some((_, r, _)) => route_url(r, url),
            None => Route::Refused,
        };
        match route {
            Route::Refused => denied(),
            Route::NoHandler => (WorkspaceStatus::NotFound, Vec::new()),
            Route::Web => match self.desktop_open_url(url, t) {
                Ok(()) => (WorkspaceStatus::Ok, Vec::new()),
                Err(_) => (WorkspaceStatus::Failed, Vec::new()),
            },
            Route::App(key) => {
                let Some(def) = super::apps::apps().iter().find(|a| a.icon == key).map(|a| a.id) else { return (WorkspaceStatus::NotFound, Vec::new()) };
                self.desk_open_or_start(def, t);
                if let super::AppId::External(slot) = def {
                    self.ext_send(slot as usize, &ToApp::Open { url: url.to_string() });
                }
                (WorkspaceStatus::Ok, Vec::new())
            }
        }
    }

    /// Brings `app` up the way the share sheet does: a window on the desktop, else the app process.
    pub(super) fn desk_open_or_start(&mut self, app: super::AppId, t: f64) {
        if self.desk_active() {
            self.desk_open(app, t);
        } else if let super::AppId::External(slot) = app {
            self.ext_open(slot as usize);
        }
        self.dirty = true;
    }

    #[cfg(feature = "desktop-wayland")]
    fn workspace_files(&mut self, slot: u8, app_id: &str, op: WorkspaceOp, grants: &[GrantId], arg: &str, t: f64) -> Reply {
        // the same permission that lets an app hold grants at all
        let permitted = apphost::installed().get(slot as usize).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "documents"));
        if !permitted || grants.is_empty() {
            return denied();
        }
        let Ok((home, files)) = self.grant_files(app_id, grants) else { return denied() };
        match op {
            WorkspaceOp::Recycle => {
                if files.iter().any(|(_, write)| !write) {
                    return denied();
                }
                let paths: Vec<PathBuf> = files.into_iter().map(|(p, _)| p).collect();
                let results = trash_into(&home, &paths, &crate::files::trash::dir(&home));
                let moved: Vec<String> = paths.iter().zip(&results).filter(|(_, r)| r.is_ok()).filter_map(|(p, _)| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                let status = if moved.len() == paths.len() { WorkspaceStatus::Ok } else { WorkspaceStatus::Failed };
                (status, moved)
            }
            WorkspaceOp::Reveal => {
                // one Files window: the folder of the first file (gap: a selection across folders)
                let folder = reveal_target(&files[0].0);
                match self.desktop_open_folder(FILES_APP, &folder.to_string_lossy(), t) {
                    Ok(()) => (WorkspaceStatus::Ok, Vec::new()),
                    Err(_) => (WorkspaceStatus::Failed, Vec::new()),
                }
            }
            WorkspaceOp::OpenFile | WorkspaceOp::OpenWith => {
                // (target app key, grant) for each file
                let mut plan: Vec<(String, GrantId)> = Vec::new();
                if op == WorkspaceOp::OpenWith {
                    let installed = apphost::installed();
                    if !installed.iter().any(|a| a.manifest.id == arg && launchable(&a.manifest.kind)) {
                        return (WorkspaceStatus::NotFound, Vec::new());
                    }
                    plan.extend(grants.iter().map(|g| (arg.to_string(), *g)));
                } else {
                    self.ensure_defaults();
                    let Some((_, registry, _)) = &self.defaults else { return (WorkspaceStatus::Failed, Vec::new()) };
                    for ((path, _), g) in files.iter().zip(grants) {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        match route_file(registry, &name) {
                            Some(app) => plan.push((app, *g)),
                            None => return (WorkspaceStatus::NotFound, Vec::new()),
                        }
                    }
                }
                let mut seen: Vec<String> = Vec::new();
                for (key, _) in &plan {
                    if !seen.contains(key) {
                        seen.push(key.clone());
                    }
                }
                for key in seen {
                    // an app key is the manifest id for installed apps (`AppDef::icon`)
                    let Some(def) = super::apps::apps().iter().find(|a| a.icon == key).map(|a| a.id) else { return (WorkspaceStatus::NotFound, Vec::new()) };
                    let batch: Vec<GrantId> = plan.iter().filter(|(k, _)| *k == key).map(|(_, g)| *g).collect();
                    self.desk_open_or_start(def, t);
                    if let super::AppId::External(to) = def {
                        let request = self.app_share.next_inbound();
                        self.inbound_files(app_id, &key, to, request, &batch);
                    }
                }
                (WorkspaceStatus::Ok, Vec::new())
            }
            _ => denied(),
        }
    }

    /// An app event (launch, exit, activation) for the app in `slot`, sent to everyone who subscribed.
    pub(super) fn workspace_app_event(&mut self, kind: WorkspaceEventKind, slot: u8) {
        let installed = apphost::installed();
        let Some(app) = installed.get(slot as usize) else { return };
        match kind {
            WorkspaceEventKind::DidLaunchApplication if !self.app_workspace.launched(slot) => return,
            WorkspaceEventKind::DidTerminateApplication if !self.app_workspace.gone(slot) => return,
            _ => {}
        }
        if !launchable(&app.manifest.kind) {
            return;
        }
        self.workspace_broadcast(kind, &app.manifest.id, &app.manifest.name);
    }

    /// A system event (sleep, wake, screens sleep or wake) for everyone who subscribed.
    pub fn workspace_system_event(&mut self, kind: WorkspaceEventKind) {
        self.workspace_broadcast(kind, "", "");
    }

    fn workspace_broadcast(&mut self, kind: WorkspaceEventKind, id: &str, name: &str) {
        for to in self.app_workspace.listeners(kind) {
            self.ext_send(to as usize, &ToApp::WorkspaceEvent { kind, app_id: id.to_string(), name: name.to_string() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::launch::{Handler, Rank};

    fn registry() -> Registry {
        let mut r = Registry::new();
        r.register(Handler::new("viewer").opens("public.png", Rank::Default));
        r.register(Handler::new("editor").opens("public.image", Rank::Alternate).opens("public.plain-text", Rank::Default));
        r.register(Handler::new("browser").scheme("https").scheme("http"));
        r.register(Handler::new("mail").scheme("mailto"));
        r
    }

    #[test]
    fn open_uses_the_default_app_for_the_type() {
        let mut r = registry();
        assert_eq!(route_file(&r, "a.png").as_deref(), Some("viewer"));
        // the user's choice in Default Apps wins
        r.set_default_for_type("public.png", "editor").unwrap();
        assert_eq!(route_file(&r, "a.png").as_deref(), Some("editor"));
        assert_eq!(route_file(&r, "notes.txt").as_deref(), Some("editor"));
        assert_eq!(route_file(&r, "a.unknownext"), None);
    }

    #[test]
    fn links_open_by_scheme_and_local_schemes_are_refused() {
        let r = registry();
        assert_eq!(route_url(&r, "https://example.org/x"), Route::Web);
        assert_eq!(route_url(&r, "mailto:a@b.c"), Route::App("mail".into()));
        assert_eq!(route_url(&r, "gopher://x"), Route::NoHandler);
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "data:text/plain,hi", "no scheme", "https://a\nb", "", "ht tp://x"] {
            assert_eq!(route_url(&r, bad), Route::Refused, "{bad:?}");
        }
    }

    #[test]
    fn an_app_can_only_launch_apps_not_services() {
        assert!(launchable("app"));
        assert!(launchable("system-app"));
        assert!(!launchable("service"));
        assert!(!launchable("system"));
        assert!(!launchable("unknown"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-workspace-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn recycle_moves_items_to_the_trash() {
        let home = scratch("recycle");
        let doc = home.join("report.txt");
        std::fs::write(&doc, "x").unwrap();
        let bin = crate::files::trash::dir(&home);
        let out = trash_into(&home, &[doc.clone()], &bin);
        let stored = out[0].as_ref().unwrap();
        assert!(!doc.exists());
        assert!(stored.starts_with(&bin) && stored.exists());
        // Files can put it back: the origin is recorded
        let origins = std::fs::read_to_string(bin.join(ORIGINS)).unwrap();
        assert!(origins.contains(stored.file_name().unwrap().to_str().unwrap()) && origins.contains(home.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn recycle_refuses_the_home_folder_and_the_trash_itself() {
        let home = scratch("refuse");
        let bin = crate::files::trash::dir(&home);
        std::fs::create_dir_all(&bin).unwrap();
        let out = trash_into(&home, &[home.clone(), bin.join("1__x")], &bin);
        assert!(out.iter().all(|r| r.is_err()));
        assert!(home.exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn reveal_opens_the_folder_of_a_file_and_a_folder_itself() {
        let home = scratch("reveal");
        let f = home.join("a.txt");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(reveal_target(&f), home);
        assert_eq!(reveal_target(&home), home);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn apps_hear_did_launch_and_did_terminate() {
        let mut s = State::default();
        s.subscribe(1, WorkspaceEventKind::DidLaunchApplication.bit() | WorkspaceEventKind::DidTerminateApplication.bit());
        s.subscribe(2, WorkspaceEventKind::DidWake.bit());
        assert_eq!(s.listeners(WorkspaceEventKind::DidLaunchApplication), [1]);
        assert_eq!(s.listeners(WorkspaceEventKind::DidWake), [2]);
        assert!(s.listeners(WorkspaceEventKind::DidActivateApplication).is_empty());
        // a launch and an exit are each reported once
        assert!(s.launched(3));
        assert!(!s.launched(3));
        assert!(s.gone(3));
        assert!(!s.gone(3));
        // an app that quits stops hearing events
        assert!(s.gone(1) == false);
        assert!(s.listeners(WorkspaceEventKind::DidLaunchApplication).is_empty());
        // a mask of unknown bits subscribes to nothing
        s.subscribe(4, 0x8000_0000);
        assert!(s.listeners(WorkspaceEventKind::WillSleep).is_empty());
    }
}
