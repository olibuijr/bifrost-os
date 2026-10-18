//! File-manager sync extensions in Files. An app whose manifest declares `file_manager_folders` is an extension of the
//! system point `is.bifrost.file-manager-sync` for those folders (`bifrost_ui::extension_manifest`): Files reaches it
//! through the shared extension host (`springboard::shell::extension_host`, the one file providers use), so the app is
//! started in extension mode when Files first needs it and stopped when it has been idle. For the items inside the
//! folders Files draws the badges it returns, adds its menu items to the item and folder shortcut menus, and sends it
//! the choice back (`bifrost_app::file_manager_sync`).
//!
//! Files never waits on an extension while it draws: a badge is asked for on a worker thread and shows on the next draw
//! after the answer (the answer wakes the loop, and `tick` hands it over through [`Extensions::poll`]). Showing one of
//! the extension's folders is what starts it. The shortcut menu is asked on the UI thread with a short timeout, so it
//! never waits for a launch: when the extension is not running the menu starts it in the background and shows no rows
//! this time. An extension that does not answer is skipped for a while. Nothing outside the folders the manifest names
//! is ever sent to an extension.
use crate::provider::locked;
use crate::*;
use bifrost_app::app_events::{AppEvent, AppEventReply, ERR_EVENT_NOT_HANDLED, ERR_TARGET_UNREACHABLE, ERR_TIMEOUT};
use bifrost_app::extension::FILE_MANAGER_POINT;
use bifrost_app::file_manager_sync::{Badge, MenuKind, RemoteExtension, SyncController, SyncHost, Transport, ERR_STARTING};
use bifrost_core::json::Json;
use springboard::shell::apphost;
use springboard::shell::extension_host::{Host, HostError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, TryLockError};

/// The first id given to an extension's menu row; the rows of one menu follow, so they never meet the Files ids.
pub(crate) const MENU_EXT_BASE: u32 = 1000;

/// A call with less time than this is made on the UI thread (the shortcut menu): it never waits for a launch.
const LAUNCH_BUDGET: Duration = Duration::from_millis(1000);

/// Side of a badge, in points.
const BADGE_PT: f32 = 14.;

fn failed(code: i64, text: &str) -> AppEventReply {
    AppEventReply { error_number: code, error_string: Some(text.into()), result: Json::Null }
}

/// The event reply of a host call. A reply that did not come in time is a timeout, the others that the app was not
/// reachable, and an error answer from the extension itself is "not handled".
fn reply_of(result: Result<Json, HostError>) -> AppEventReply {
    match result {
        Ok(result) => AppEventReply { error_number: 0, error_string: None, result },
        Err(HostError::Remote(message)) => failed(ERR_EVENT_NOT_HANDLED, &message),
        Err(HostError::Unreachable(message)) if message.contains("temporarily unavailable") || message.contains("timed out") => failed(ERR_TIMEOUT, &message),
        Err(e) => failed(ERR_TARGET_UNREACHABLE, &e.to_string()),
    }
}

/// The host, when it is free and `bundle_id` runs; otherwise the extension is started on a thread of its own (once at
/// a time) and None is returned.
fn running_host<'a>(host: &'a Arc<Mutex<Host>>, bundle_id: &str, warming: &Arc<AtomicBool>) -> Option<std::sync::MutexGuard<'a, Host>> {
    let guard = match host.try_lock() {
        Ok(g) => Some(g),
        Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),
        Err(TryLockError::WouldBlock) => None, // a launch or another call holds the host: ask again next time
    };
    if let Some(mut g) = guard {
        if g.is_ready(bundle_id, FILE_MANAGER_POINT) {
            return Some(g);
        }
    }
    if !warming.swap(true, Ordering::SeqCst) {
        let (host, bundle_id, warming) = (Arc::clone(host), bundle_id.to_string(), Arc::clone(warming));
        std::thread::spawn(move || {
            let started = locked(&host).begin(&bundle_id, FILE_MANAGER_POINT, Instant::now());
            if let Ok(lease) = started {
                locked(&host).end(lease, Instant::now());
            }
            warming.store(false, Ordering::SeqCst);
        });
    }
    None
}

/// The transport of the extension `bundle_id`: each request is one call through the extension host, which starts the
/// extension if it is not running. A request with less than [`LAUNCH_BUDGET`] to wait never waits for a launch: it
/// starts the extension in the background and answers [`ERR_STARTING`].
pub(crate) fn host_transport(host: Arc<Mutex<Host>>, bundle_id: String) -> Transport {
    let warming = Arc::new(AtomicBool::new(false));
    Arc::new(move |ev: &AppEvent, timeout: Duration| {
        let begun = if timeout < LAUNCH_BUDGET {
            match running_host(&host, &bundle_id, &warming) {
                Some(mut g) => g.begin(&bundle_id, FILE_MANAGER_POINT, Instant::now()),
                None => return failed(ERR_STARTING, "the extension is starting"),
            }
        } else {
            locked(&host).begin(&bundle_id, FILE_MANAGER_POINT, Instant::now())
        };
        let lease = match begun {
            Ok(lease) => lease,
            Err(e) => return reply_of(Err(e)),
        };
        let result = lease.exchange(&ev.method(), ev.params.clone(), timeout);
        locked(&host).end(lease, Instant::now());
        reply_of(result)
    })
}

/// The extensions Files hosts and the menu rows they added to the menu that is open.
#[derive(Default)]
pub(crate) struct Extensions {
    host: SyncHost,
    /// The folder the window shows, for the begin and end calls.
    shown: Option<PathBuf>,
    /// Menu id, extension, menu kind and the extension's own item id of each row of the open menu.
    rows: Vec<(u32, String, MenuKind, u32)>,
}

impl Extensions {
    /// The extension apps installed under `root` that `host` offers as file-manager sync extensions and that declare
    /// `file_manager_folders`. Nothing is started here: the host starts an extension on the first request for one of its
    /// folders.
    pub(crate) fn discover(root: &Path, home: &Path, host: &Arc<Mutex<Host>>, own_id: &str) -> Extensions {
        let mut ext = Extensions::default();
        let offered: Vec<String> = locked(host).discover(FILE_MANAGER_POINT).into_iter().map(|e| e.bundle_id).collect();
        if offered.is_empty() {
            return ext;
        }
        for app in apphost::scan(root) {
            let m = &app.manifest;
            let dirs = m.file_manager_dirs(home);
            if m.id == own_id || dirs.is_empty() || !offered.contains(&m.id) {
                continue;
            }
            let mut controller = SyncController::default();
            if controller.set_directory_urls(&dirs).is_err() {
                continue;
            }
            let transport = host_transport(Arc::clone(host), m.id.clone());
            ext.host.register(&m.id, controller, Box::new(RemoteExtension::connect_lazy(transport)));
        }
        ext
    }

    /// Extensions over an existing host (the tests register their own).
    #[cfg(test)]
    pub(crate) fn with_host(host: SyncHost) -> Extensions {
        Extensions { host, ..Extensions::default() }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.host.extension_ids().is_empty()
    }

    /// The window now shows `dir`: the extensions that watch it hear of it, and those that watched the folder before
    /// hear it is gone.
    pub(crate) fn show(&mut self, dir: &Path) {
        if self.is_empty() || self.shown.as_deref() == Some(dir) {
            return;
        }
        if let Some(old) = self.shown.take() {
            let _ = self.host.folder_closed(&old);
        }
        let _ = self.host.folder_opened(dir);
        self.shown = Some(dir.to_path_buf());
    }

    /// The badges on `path`, one per extension that badged it.
    pub(crate) fn badges(&mut self, path: &Path) -> Vec<Badge> {
        if self.is_empty() {
            return Vec::new();
        }
        self.host.badges_for(path)
    }

    /// Takes in what the extensions answered since the last tick. True when a badge changed and the window must draw.
    pub(crate) fn poll(&mut self) -> bool {
        !self.is_empty() && self.host.poll()
    }

    /// The rows the extensions add to a shortcut menu for the folder `targeted` and the `selected` items, with the ids
    /// Files gives them. `items` is the menu of the selected items, else the menu of the folder background.
    pub(crate) fn menu_items(&mut self, items: bool, targeted: &Path, selected: &[PathBuf]) -> Vec<ContextItem> {
        self.rows.clear();
        if self.is_empty() {
            return Vec::new();
        }
        let kind = if items { MenuKind::ContextualItems } else { MenuKind::ContextualContainer };
        let mut out = Vec::new();
        for (n, row) in self.host.menu(kind, targeted, selected).into_iter().enumerate() {
            let id = MENU_EXT_BASE + n as u32;
            self.rows.push((id, row.extension, kind, row.item.id));
            out.push(ContextItem { id, label: row.item.title, enabled: row.item.enabled });
        }
        out
    }

    /// The choice of an extension's row: sent to the extension it came from. False when `id` is not one of the open
    /// menu's extension rows.
    pub(crate) fn perform(&mut self, id: u32, targeted: &Path, selected: &[PathBuf]) -> bool {
        let Some((_, extension, kind, item)) = self.rows.iter().find(|r| r.0 == id).cloned() else { return false };
        let _ = self.host.perform(&extension, kind, item, targeted, selected);
        true
    }
}

impl Files {
    /// The extension rows to add to the shortcut menu that is opening (`on_item` for the menu of the selection).
    pub(crate) fn extension_menu_items(&self, on_item: bool) -> Vec<ContextItem> {
        let selected = if on_item { self.selected_paths() } else { Vec::new() };
        self.ext.borrow_mut().menu_items(on_item, &self.path, &selected)
    }

    /// Runs a choice from an extension's row. False when the id is not one.
    pub(crate) fn extension_action(&mut self, id: u32) -> bool {
        if id < MENU_EXT_BASE {
            return false;
        }
        let selected = self.selected_paths();
        self.ext.borrow_mut().perform(id, &self.path, &selected)
    }

    /// Draws the badges of the extensions on the item `path`, right to left from the right edge `x`, top at `y`, in points.
    pub(crate) fn paint_badges(&self, ui: &Ui, s: &mut Surface, path: &Path, x: f32, y: f32) {
        let badges = self.ext.borrow_mut().badges(path);
        for (n, badge) in badges.iter().enumerate() {
            let bx = x - BADGE_PT - n as f32 * (BADGE_PT + SPACE_XXS);
            // a plate under the glyph so it reads on any icon and in both appearances
            s.fill_rrect(ui.pt(bx), ui.pt(y), ui.pt(BADGE_PT), ui.pt(BADGE_PT), ui.pt(BADGE_PT / 2.), CONTENT_BG.now());
            let pad = SPACE_XXS / 2.;
            bifrost_app::glyphs::glyph(s, ui, &badge.image, Rect { x: bx + pad, y: y + pad, w: BADGE_PT - 2. * pad, h: BADGE_PT - 2. * pad }, LINK);
        }
    }

    /// Hands the extensions' answers to the window (this one and the New Window ones). True when a draw is due.
    pub(crate) fn poll_extensions(&mut self) -> bool {
        let mut changed = self.ext.borrow_mut().poll();
        for e in &mut self.windows {
            changed |= e.files.ext.borrow_mut().poll();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::app_events::{AppEvent, AppEventReply};
    use bifrost_app::extension::scan_manifests;
    use bifrost_app::extension_service::serve_at;
    use bifrost_app::file_manager_sync::{serve, MenuItem, SyncExtension, SyncService, Transport};
    use bifrost_core::json::Json;
    use springboard::shell::extension_host::{Entry as HostEntry, Instance, Launcher, Package, Sockets};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    /// An extension app: badges `.tmp` files, adds a row to each menu, and records what it was told to do.
    struct Cloud {
        done: Arc<Mutex<Vec<String>>>,
    }

    impl SyncExtension for Cloud {
        fn request_badge(&mut self, c: &mut SyncController, path: &Path) {
            if path.extension().is_some_and(|e| e == "tmp") {
                c.set_badge(path, Some("pending")).unwrap();
            }
        }
        fn menu(&mut self, kind: MenuKind, c: &SyncController) -> Vec<MenuItem> {
            match kind {
                MenuKind::ContextualItems if !c.selected_item_urls().is_empty() => vec![MenuItem::new(7, "Share Link")],
                MenuKind::ContextualContainer => vec![MenuItem::new(8, "Sync This Folder")],
                _ => Vec::new(),
            }
        }
        fn perform(&mut self, kind: MenuKind, item: u32, c: &SyncController) {
            let names: Vec<String> = c.selected_item_urls().iter().map(|p| p.display().to_string()).collect();
            self.done.lock().unwrap().push(format!("{} {item} [{}] in {}", kind.name(), names.join(","), c.targeted_url().map(|p| p.display().to_string()).unwrap_or_default()));
        }
    }

    /// Extensions backed by an app in this process that watches `dir`, with the log of what it was told to do.
    fn cloud(dir: &str) -> (Extensions, Arc<Mutex<Vec<String>>>) {
        let done = Arc::new(Mutex::new(Vec::new()));
        let mut served = SyncController::default();
        served.set_directory_urls(&[PathBuf::from(dir)]).unwrap();
        served.set_badge_image("pending", "Waiting to sync", "arrow.clockwise").unwrap();
        let app = Arc::new(Mutex::new((served, Cloud { done: done.clone() })));
        let transport: Transport = Arc::new(move |ev: &AppEvent, _: Duration| {
            let mut a = app.lock().unwrap();
            let (controller, ext) = &mut *a;
            match serve(ev, controller, ext) {
                Ok(result) => AppEventReply { error_number: 0, error_string: None, result },
                Err(e) => AppEventReply { error_number: e.code, error_string: Some(e.message), result: Json::Null },
            }
        });
        let mut mine = SyncController::default();
        mine.set_directory_urls(&[PathBuf::from(dir)]).unwrap();
        let mut host = SyncHost::new();
        host.register("is.example.cloud", mine, Box::new(RemoteExtension::connect(transport)));
        (Extensions::with_host(host), done)
    }

    fn wait(files: &Files, what: &str, mut done: impl FnMut(&Files) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            files.ext.borrow_mut().poll();
            if done(files) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}");
    }

    fn font_ui() -> Ui {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        Ui::new(font, 1.25)
    }

    fn files_with(ext: Extensions, rows: &[&str]) -> Files {
        let mut files = crate::tests::fixture(rows.iter().map(|n| Entry { name: (*n).into(), path: format!("/fixture/{n}").into(), is_dir: false, size: 1, mtime: 0, kind: Kind::Other, items: None }).collect());
        files.ext = std::cell::RefCell::new(ext);
        files
    }

    #[test]
    fn a_badge_the_extension_returns_is_drawn_on_the_item_after_the_answer() {
        let ui = font_ui();
        let (ext, _) = cloud("/fixture");
        let files = files_with(ext, &["draft.tmp", "report.txt"]);
        let (w, h) = (200usize, 60usize);
        let paint = |path: &str| {
            let mut px = vec![0u32; w * h];
            files.paint_badges(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, Path::new(path), 100., 20.);
            px
        };
        // before the answer there is nothing to draw
        assert!(paint("/fixture/draft.tmp").iter().all(|&p| p == 0));
        wait(&files, "the badge", |f| f.ext.borrow_mut().badges(Path::new("/fixture/draft.tmp")).len() == 1);
        assert!(paint("/fixture/draft.tmp").iter().any(|&p| p != 0), "the plate and the glyph are drawn");
        assert!(paint("/fixture/report.txt").iter().all(|&p| p == 0), "an item the extension did not badge stays clean");
        assert!(paint("/elsewhere/draft.tmp").iter().all(|&p| p == 0), "outside the watched folder nothing is asked");
    }

    #[test]
    fn extension_rows_join_the_menus_and_a_choice_goes_back_to_the_extension() {
        let (ext, done) = cloud("/fixture");
        let mut files = files_with(ext, &["a.txt", "b.txt"]);
        // the folder menu
        let folder_rows = files.extension_menu_items(false);
        assert_eq!(folder_rows.iter().map(|r| (r.id, r.label.as_str(), r.enabled)).collect::<Vec<_>>(), vec![(MENU_EXT_BASE, "Sync This Folder", true)]);
        // the menu of the selection
        files.selection = vec![1];
        let item_rows = files.extension_menu_items(true);
        assert_eq!(item_rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), vec!["Share Link"]);
        assert!(!files.extension_action(MENU_OPEN), "a Files id is not an extension row");
        assert!(files.extension_action(item_rows[0].id));
        let deadline = Instant::now() + Duration::from_secs(5);
        while done.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*done.lock().unwrap(), vec!["items 7 [/fixture/b.txt] in /fixture".to_string()]);
        // a row of a menu that was replaced no longer works
        files.selection.clear();
        files.extension_menu_items(false);
        assert!(!files.extension_action(MENU_EXT_BASE + 5));
    }

    #[test]
    fn a_folder_the_extension_does_not_watch_gets_no_rows_and_no_calls() {
        let (ext, done) = cloud("/fixture/Sync");
        let mut files = files_with(ext, &["a.txt"]);
        files.selection = vec![0];
        assert!(files.extension_menu_items(true).is_empty());
        assert!(files.extension_menu_items(false).is_empty());
        files.ext.borrow_mut().show(Path::new("/fixture"));
        assert!(done.lock().unwrap().is_empty());
    }

    // --- through the extension host: launch on demand, reap when idle ---

    const CLOUD_ID: &str = "is.example.cloud";
    const CLOUD_MANIFEST: &str = "id = \"is.example.cloud\"\nname = \"Cloud\"\nexec = \"x\"\nfile_manager_folders = [\"/fixture\"]\n";

    /// The extension process of the tests: the real SDK service in a thread on the socket the host names.
    struct InProcess {
        socket: PathBuf,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Instance for InProcess {
        fn alive(&mut self) -> bool {
            self.thread.as_ref().is_some_and(|t| !t.is_finished())
        }
        fn stop(&mut self) {
            if let Ok(mut s) = UnixStream::connect(&self.socket) {
                let _ = s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"extension.shutdown\",\"params\":{}}\n");
                let _ = s.shutdown(std::net::Shutdown::Write);
                let _ = std::io::Read::read_to_end(&mut s, &mut Vec::new());
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Starts the cloud extension as a `SyncService` (what an extension app runs in extension mode) and counts launches.
    struct CloudLauncher {
        launches: Arc<AtomicUsize>,
        done: Arc<Mutex<Vec<String>>>,
    }

    impl Launcher for CloudLauncher {
        fn launch(&self, _entry: &HostEntry, socket: &Path) -> std::io::Result<Box<dyn Instance>> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            let (path, done) = (socket.to_path_buf(), self.done.clone());
            let thread = std::thread::spawn(move || {
                let mut served = SyncController::default();
                served.set_directory_urls(&[PathBuf::from("/fixture")]).unwrap();
                served.set_badge_image("pending", "Waiting to sync", "arrow.clockwise").unwrap();
                let _ = serve_at(&path, &mut SyncService::new(served, Cloud { done }), false);
            });
            Ok(Box::new(InProcess { socket: socket.to_path_buf(), thread: Some(thread) }))
        }
    }

    struct Hosted {
        ext: Extensions,
        host: Arc<Mutex<Host>>,
        launches: Arc<AtomicUsize>,
        done: Arc<Mutex<Vec<String>>>,
        dir: PathBuf,
    }

    /// A host over the packages `manifests`, plus the same manifests written under an app root.
    fn host_over(dir: &Path, manifests: &[(&str, &str)], launcher: CloudLauncher) -> (Arc<Mutex<Host>>, PathBuf) {
        let root = dir.join("root");
        for (id, text) in manifests {
            let d = root.join("apps").join(id).join("current");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("manifest.toml"), text).unwrap();
            std::fs::write(d.join("x"), b"ELF").unwrap();
        }
        let scan = scan_manifests(manifests.iter().map(|(id, text)| (*id, *text)));
        let packages = manifests.iter().map(|(id, _)| Package { app_id: (*id).into(), exec: "/bin/true".into(), data_dir: dir.join("data") }).collect();
        let mut host = Host::new(scan, packages, Box::new(launcher), Arc::new(Sockets), dir.join("sock"));
        host.set_start_wait(Duration::from_secs(3));
        (Arc::new(Mutex::new(host)), root)
    }

    fn hosted(tag: &str) -> Hosted {
        let dir = std::env::temp_dir().join(format!("files-fm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (launches, done) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
        let launcher = CloudLauncher { launches: launches.clone(), done: done.clone() };
        let (host, root) = host_over(&dir, &[(CLOUD_ID, CLOUD_MANIFEST)], launcher);
        let ext = Extensions::discover(&root, Path::new("/home/ann"), &host, "is.olibuijr.files.desktop");
        Hosted { ext, host, launches, done, dir }
    }

    impl Hosted {
        fn running(&self) -> bool {
            locked(&self.host).is_running(CLOUD_ID, FILE_MANAGER_POINT)
        }

        fn finish(self) {
            locked(&self.host).stop_all();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn until(ext: &mut Extensions, what: &str, mut done: impl FnMut(&mut Extensions) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            ext.poll();
            if done(ext) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn nothing_starts_until_a_watched_folder_is_shown_and_then_the_host_launches_it() {
        let mut h = hosted("lazy");
        assert!(!h.ext.is_empty(), "the declared folders make the app a file-manager extension");
        assert_eq!(locked(&h.host).live(), 0, "discovery starts nothing");
        // a folder the extension does not watch, and an item outside its folders: still nothing
        h.ext.show(Path::new("/elsewhere"));
        assert!(h.ext.badges(Path::new("/elsewhere/a.tmp")).is_empty());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(h.launches.load(Ordering::SeqCst), 0);
        assert!(!h.running());
        // showing a watched folder and asking for a badge starts it through the host
        h.ext.show(Path::new("/fixture"));
        let tmp = Path::new("/fixture/draft.tmp");
        h.ext.badges(tmp);
        until(&mut h.ext, "the badge", |e| e.badges(tmp).len() == 1);
        assert_eq!(h.launches.load(Ordering::SeqCst), 1);
        assert!(h.running());
        assert_eq!(locked(&h.host).live(), 1);
        h.finish();
    }

    #[test]
    fn an_idle_extension_is_reaped_and_the_next_request_starts_it_again() {
        let mut h = hosted("reap");
        h.ext.show(Path::new("/fixture"));
        let first = Path::new("/fixture/draft.tmp");
        h.ext.badges(first);
        until(&mut h.ext, "the first badge", |e| e.badges(first).len() == 1);
        assert_eq!(locked(&h.host).reap(Instant::now() + Duration::from_secs(1)), Vec::<String>::new(), "not idle long enough");
        assert_eq!(locked(&h.host).reap(Instant::now() + Duration::from_secs(120)), vec![CLOUD_ID.to_string()]);
        assert!(!h.running());
        assert_eq!(locked(&h.host).live(), 0);
        let second = Path::new("/fixture/other.tmp");
        h.ext.badges(second);
        until(&mut h.ext, "the badge after the restart", |e| e.badges(second).len() == 1);
        assert_eq!(h.launches.load(Ordering::SeqCst), 2, "the host started it again");
        assert!(h.running());
        h.finish();
    }

    #[test]
    fn the_menu_never_waits_for_a_launch_and_shows_rows_once_the_extension_runs() {
        let mut h = hosted("menu");
        let started = Instant::now();
        assert!(h.ext.menu_items(false, Path::new("/fixture"), &[]).is_empty(), "no rows while the extension is not running");
        assert!(started.elapsed() < Duration::from_millis(900), "the menu did not wait for the launch");
        // the menu started the extension in the background
        let deadline = Instant::now() + Duration::from_secs(8);
        while !h.running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(h.running());
        assert_eq!(h.launches.load(Ordering::SeqCst), 1);
        // "starting" does not mute the extension: the next menu has its rows
        let rows = h.ext.menu_items(false, Path::new("/fixture"), &[]);
        assert_eq!(rows.iter().map(|r| (r.id, r.label.as_str())).collect::<Vec<_>>(), vec![(MENU_EXT_BASE, "Sync This Folder")]);
        assert!(h.ext.perform(rows[0].id, Path::new("/fixture"), &[]));
        let deadline = Instant::now() + Duration::from_secs(8);
        while h.done.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*h.done.lock().unwrap(), vec!["container 8 [] in /fixture".to_string()]);
        assert_eq!(h.launches.load(Ordering::SeqCst), 1, "one process served the menu and the choice");
        h.finish();
    }

    #[test]
    fn host_errors_become_event_errors_and_an_unknown_extension_is_unreachable() {
        assert!(reply_of(Ok(Json::Bool(true))).ok());
        assert_eq!(reply_of(Err(HostError::Unreachable("Resource temporarily unavailable (os error 11)".into()))).error_number, ERR_TIMEOUT);
        assert_eq!(reply_of(Err(HostError::Unreachable("/x.sock: No such file".into()))).error_number, ERR_TARGET_UNREACHABLE);
        assert_eq!(reply_of(Err(HostError::NotAvailable)).error_number, ERR_TARGET_UNREACHABLE);
        assert_eq!(reply_of(Err(HostError::Remote("bad".into()))).error_number, ERR_EVENT_NOT_HANDLED);
        let h = hosted("err");
        let transport = host_transport(h.host.clone(), "com.nobody.x".into());
        let reply = transport(&AppEvent::new("filemanager", "describe", Json::Obj(vec![])), Duration::from_secs(2));
        assert_eq!(reply.error_number, ERR_TARGET_UNREACHABLE);
        assert_eq!(locked(&h.host).live(), 0);
        h.finish();
    }

    #[test]
    fn no_extension_means_no_work() {
        let mut none = Extensions::default();
        assert!(none.is_empty());
        assert!(none.badges(Path::new("/x/y")).is_empty());
        assert!(!none.poll());
        assert!(none.menu_items(true, Path::new("/x"), &[]).is_empty());
        assert!(!none.perform(MENU_EXT_BASE, Path::new("/x"), &[]));
        // a host that offers no file-manager extension, or an empty app root, finds nothing
        let dir = std::env::temp_dir().join(format!("files-fm-none-{}", std::process::id()));
        let launcher = CloudLauncher { launches: Arc::default(), done: Arc::default() };
        let (host, _) = host_over(&dir, &[], launcher);
        assert!(Extensions::discover(Path::new("/nonexistent-root"), Path::new("/home/a"), &host, "is.olibuijr.files.desktop").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_apps_that_declare_folders_become_extensions() {
        let dir = std::env::temp_dir().join(format!("files-fm-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cloud = "id = \"is.o.cloud\"\nname = \"Cloud\"\nexec = \"x\"\nfile_manager_folders = [\"~/Cloud\"]\n";
        let calc = "id = \"is.o.calc\"\nname = \"Calc\"\nexec = \"x\"\n";
        let wide = "id = \"is.o.wide\"\nname = \"Wide\"\nexec = \"x\"\nfile_manager_folders = [\"/\", \"~/\"]\n";
        let launcher = CloudLauncher { launches: Arc::default(), done: Arc::default() };
        // the host knows the three; "is.o.late" is on disk but the host was built before it was installed
        let (host, root) = host_over(&dir, &[("is.o.cloud", cloud), ("is.o.calc", calc), ("is.o.wide", wide)], launcher);
        let late = root.join("apps/is.o.late/current");
        std::fs::create_dir_all(&late).unwrap();
        std::fs::write(late.join("manifest.toml"), "id = \"is.o.late\"\nname = \"Late\"\nexec = \"x\"\nfile_manager_folders = [\"~/Late\"]\n").unwrap();
        std::fs::write(late.join("x"), b"ELF").unwrap();
        let ext = Extensions::discover(&root, Path::new("/home/ann"), &host, "is.olibuijr.files.desktop");
        assert_eq!(ext.host.extension_ids(), ["is.o.cloud"], "Calc declares nothing, Wide names only folders that are refused, Late is not offered by the host");
        assert_eq!(locked(&host).live(), 0);
        // Files itself is never its own extension
        let own = Extensions::discover(&root, Path::new("/home/ann"), &host, "is.o.cloud");
        assert!(own.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
