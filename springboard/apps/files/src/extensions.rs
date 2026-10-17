//! File-manager sync extensions in Files. An app whose manifest declares `file_manager_folders` is an extension for those
//! folders: Files runs it through the app's control socket (`bifrost_app::file_manager_sync`) and, for the items inside
//! the folders, draws the badges it returns, adds its menu items to the item and folder shortcut menus, and sends it
//! the choice back.
//!
//! Files never waits on an extension while it draws: a badge is asked for on a worker thread and shows on the next draw
//! after the answer (the answer wakes the loop, and `tick` hands it over through [`Extensions::poll`]). The shortcut
//! menu asks with a short timeout, and an extension that does not answer is skipped for a while. Nothing outside the
//! folders the manifest names is ever sent to an extension, and an extension app that is not running simply adds
//! nothing.
use crate::*;
use bifrost_app::file_manager_sync::{socket_transport, Badge, MenuKind, RemoteExtension, SyncController, SyncHost};
use springboard::shell::apphost;

/// The first id given to an extension's menu row; the rows of one menu follow, so they never meet the Files ids.
pub(crate) const MENU_EXT_BASE: u32 = 1000;

/// Side of a badge, in points.
const BADGE_PT: f32 = 14.;

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
    /// The extension apps installed under `root` (those that declare `file_manager_folders`), connected through their
    /// control sockets in `runtime` (`$XDG_RUNTIME_DIR`). Without a runtime directory there are none.
    pub(crate) fn discover(root: &Path, home: &Path, runtime: Option<&Path>, own_id: &str) -> Extensions {
        let mut ext = Extensions::default();
        let Some(runtime) = runtime else { return ext };
        for app in apphost::scan(root) {
            let m = &app.manifest;
            let dirs = m.file_manager_dirs(home);
            if m.id == own_id || dirs.is_empty() {
                continue;
            }
            let mut controller = SyncController::default();
            if controller.set_directory_urls(&dirs).is_err() {
                continue;
            }
            let socket = bifrost_app::script_target::socket_path(&m.id, None, Some(runtime), Path::new(""));
            ext.host.register(&m.id, controller, Box::new(RemoteExtension::connect(socket_transport(socket))));
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
    use bifrost_app::file_manager_sync::{serve, MenuItem, SyncExtension, Transport};
    use bifrost_core::json::Json;
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

    #[test]
    fn no_extension_means_no_work() {
        let mut none = Extensions::default();
        assert!(none.is_empty());
        assert!(none.badges(Path::new("/x/y")).is_empty());
        assert!(!none.poll());
        assert!(none.menu_items(true, Path::new("/x"), &[]).is_empty());
        assert!(!none.perform(MENU_EXT_BASE, Path::new("/x"), &[]));
        // a missing runtime directory or an empty app root finds nothing
        assert!(Extensions::discover(Path::new("/nonexistent-root"), Path::new("/home/a"), Some(Path::new("/run/user/1")), "is.olibuijr.files.desktop").is_empty());
        assert!(Extensions::discover(Path::new("/nonexistent-root"), Path::new("/home/a"), None, "is.olibuijr.files.desktop").is_empty());
    }

    #[test]
    fn installed_apps_that_declare_folders_become_extensions() {
        let root = std::env::temp_dir().join(format!("files-ext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |id: &str, manifest: &str| {
            let d = root.join("apps").join(id).join("current");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("manifest.toml"), manifest).unwrap();
            std::fs::write(d.join("x"), b"ELF").unwrap();
        };
        mk("is.o.cloud", "id = \"is.o.cloud\"\nname = \"Cloud\"\nexec = \"x\"\nfile_manager_folders = [\"~/Cloud\"]\n");
        mk("is.o.calc", "id = \"is.o.calc\"\nname = \"Calc\"\nexec = \"x\"\n");
        mk("is.o.wide", "id = \"is.o.wide\"\nname = \"Wide\"\nexec = \"x\"\nfile_manager_folders = [\"/\", \"~/\"]\n");
        let ext = Extensions::discover(&root, Path::new("/home/ann"), Some(Path::new("/run/user/1000")), "is.olibuijr.files.desktop");
        assert_eq!(ext.host.extension_ids(), ["is.o.cloud"], "Calc declares nothing and Wide names only folders that are refused");
        let _ = std::fs::remove_dir_all(&root);
    }
}
