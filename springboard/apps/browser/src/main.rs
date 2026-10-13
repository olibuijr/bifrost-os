//! BifrOSt Browser: a Golden Gate SDK app (full-size window, unified toolbar, Browser-style tabs) around the SDK's
//! web view (`bifrost_app::webview`). Pages render in the bundled Servo engine process; the app draws the chrome.
mod perf;
mod store;
mod suggest;

mod browser_ctl;
mod control;
mod devtools;
mod elements;
mod network;
mod pagetools;
mod resample;

mod devmenu;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_SUBHEAD_PT, ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, SPACE_S, SPACE_XXS, TINT};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::webview::{FindDirection, WebProcessPool, WebViewEvent, WebViewId};
use bifrost_core::json::Json;
use bifrost_app::control::RpcError;
mod inspector;
mod pageui;
mod prefs;
mod system;
use inspector::Inspector;
use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
use bifrost_app::network_path::{self, Status};
use bifrost_app::{glyphs, widgets, App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};

/// The engine archive bundled in assets/servo.tar.gz (version and the first 12 hex digits of its SHA-256).
const ENGINE: &str = "servo-0.7.0-bifrost.12-9c0bb55f87bb";
const START: &str = "about:blank";

/// Toolbar ids.
const BACK: u32 = 1;
const FORWARD: u32 = 2;
const ADDRESS: u32 = 3;
const RELOAD: u32 = 4;
const NEW_TAB: u32 = 5;
const DOWNLOADS: u32 = 6;
/// Accessibility ids of tabs start here (`TAB0 + index`).
const TAB0: u64 = 100;

/// Tab strip height (Browser's compact tab bar), points.
const TAB_STRIP_PT: f32 = 30.0;
/// The address field's widest width, points (Browser keeps it readable on wide windows).
const ADDRESS_MAX_PT: f32 = 560.0;
/// Below this movement (points) a touch on the chrome is a tap.
const TAP_SLOP_PT: f32 = 6.0;

/// evdev keys for the browser's shortcuts.
const KEY_L: u32 = 38;
const KEY_C: u32 = 46;
const KEY_K: u32 = 37;
const KEY_T: u32 = 20;
const KEY_W: u32 = 17;
const KEY_R: u32 = 19;
/// Main-menu item ids (`App::menu_action`): File > Share and Downloads, View > Show Reader, Zoom and Find.
const MENU_READER: u32 = 1;
const MENU_SHARE: u32 = 2;
const MENU_ZOOM_IN: u32 = 3;
const MENU_ZOOM_OUT: u32 = 4;
const MENU_ACTUAL_SIZE: u32 = 5;
const MENU_FIND: u32 = 6;
const MENU_FIND_NEXT: u32 = 7;
const MENU_FIND_PREV: u32 = 8;
const MENU_DOWNLOADS: u32 = 9;
const MENU_CLEAR_DOWNLOADS: u32 = 10;
const MENU_AUTOFILL: u32 = 11;
const MENU_OVERVIEW: u32 = 12;
const KEY_D: u32 = 32;
const KEY_Y: u32 = 21;
const KEY_B: u32 = 48;
const KEY_U: u32 = 22;
const KEY_LEFTBRACE: u32 = 26;
const KEY_RIGHTBRACE: u32 = 27;
const KEY_TAB: u32 = 15;
const KEY_COMMA: u32 = 51;
const KEY_N: u32 = 49;
/// Hold this long without moving on a page and the touch becomes a right-click (the context menu).
const LONG_PRESS_S: f64 = 0.5;
/// evdev KEY_1 ..= KEY_9 are 2 ..= 10.
const KEY_1: u32 = 2;
const KEY_9: u32 = 10;

mod downloads;
mod findbar;
mod tabdiscard;
mod library;
mod crashbanner;
mod reader;
mod favicon;
mod inbox;
mod tokens;
mod session;
mod autofill;
mod overview;

const KEY_F: u32 = 33;
const KEY_BACKSLASH: u32 = 43;
const KEY_G: u32 = 34;
const KEY_0: u32 = 11;
const KEY_MINUS: u32 = 12;
const KEY_EQUAL: u32 = 13;

struct Tab {
    view: WebViewId,
}

#[derive(Clone, Copy, PartialEq)]
enum Press {
    Bar(u32),
    Tab(usize),
    TabClose(usize),
    Sugg(usize),
    Web,
    BannerClose,
    BannerReload,
    Reader,
    /// The star at the address field's trailing edge: toggles the bookmark of the open page.
    Bookmark,
}

struct Browser {
    pool: Option<WebProcessPool>,
    error: Option<String>,
    tabs: Vec<Tab>,
    current: usize,
    address: TextField,
    editing: bool,
    /// Last pointer position in pixels (tab hover).
    hover: Option<(f32, f32)>,
    press: Option<(Press, f32, f32)>,
    bar_layout: Option<ToolbarLayout>,
    initial_url: Option<String>,
    dark: Option<bool>,
    storm: session::recover::StormGuard,
    crash: crashbanner::CrashBanner,
    reader: reader::Reader,
    watchdog: session::recover::Watchdog,
    discard: tabdiscard::Discarder,
    clock: std::time::Instant,
    crash_log: Vec<String>,
    restore: Vec<String>,
    restore_sel: usize,
    store: store::Store,
    sugg: Vec<suggest::Suggestion>,
    sel: Option<usize>,
    session: session::Session,
    find: findbar::FindBar,
    inspector: Inspector,
    dev: devmenu::DevMenu,
    ctl: browser_ctl::CtlState,
    /// Script dialogs, context menu and <select> popups.
    pageui: pageui::PageUi,
    /// View Source fetches running off the UI thread: (page URL, result).
    sources: Vec<(String, inbox::Slot<String>)>,
    prefs: prefs::Prefs,
    sheet: prefs::Sheet,
    /// Private window (`--private`): no history, no session, a throwaway profile deleted on exit.
    private: bool,
    /// A finger down on the page: (time, x, y, finger), for long-press.
    long: Option<(f64, f32, f32, u32)>,
    /// The long-press already fired: swallow the rest of that touch.
    swallow: bool,
    /// A tab is being dragged to a new place.
    drag: bool,
    /// System services (system.rs): the network state, the path request, a copy awaiting its answer, the pointer shape
    /// and the dock progress last sent.
    net: system::Network,
    net_request: Option<u32>,
    clip: Option<(u32, system::Clip)>,
    pointer: system::Pointer,
    dock: system::Dock,
    /// Downloads list and its popover.
    downloads: downloads::Downloads,
    /// Bookmark popover, Favorites bar and the Bookmarks / History panel.
    lib: library::Library,
    /// Site icons for tabs, the address field and suggestions (favicon.rs).
    favicons: favicon::Favicons,
    /// Password AutoFill: which pages have a password field, and the request in flight (autofill.rs).
    autofill: autofill::AutoFill,
    /// Tab overview grid (overview.rs).
    overview: overview::Overview,
}

/// The bookmark star at the trailing edge of the address field (a square the height of the field).
fn star_rect(field: Rect) -> Rect {
    Rect { x: field.x + field.w - field.h, y: field.y, w: field.h, h: field.h }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Unpacks the bundled engine once per version into the user's cache and returns its binary.
fn engine() -> io::Result<PathBuf> {
    let package = std::env::current_exe()?.canonicalize()?.parent().map(Path::to_path_buf).ok_or_else(|| io::Error::other("no package dir"))?;
    let cache = home().join(".cache/bifrost-browser");
    fs::create_dir_all(&cache)?;
    let dir = cache.join(ENGINE);
    let ready = |d: &Path| fs::read_to_string(d.join(".ready")).is_ok_and(|r| r.trim() == ENGINE) && d.join("servo/bifrost-webview").is_file();
    if !ready(&dir) {
        // One extraction at a time: concurrent launches wait on the lock, then find the engine ready.
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(cache.join(".extract.lock"))?;
        lock.lock()?;
        // Staging dirs left by crashed launches that are over a day old.
        let day = std::time::Duration::from_secs(86_400);
        for e in fs::read_dir(&cache)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > day);
            if name.starts_with(&format!("{ENGINE}.")) && old {
                let _ = fs::remove_dir_all(e.path());
            }
        }
        if !ready(&dir) {
            let _ = fs::remove_dir_all(&dir); // a partial or mismatched cache
            let staging = cache.join(format!("{ENGINE}.{}", std::process::id()));
            let _ = fs::remove_dir_all(&staging);
            fs::create_dir(&staging)?;
            let ok = Command::new("tar")
                .args(["--extract", "--gzip", "--no-same-owner", "--file"])
                .arg(package.join("assets/servo.tar.gz"))
                .arg("--directory")
                .arg(&staging)
                .status()?
                .success();
            if !ok || !staging.join("servo/bifrost-webview").is_file() || fs::write(staging.join(".ready"), format!("{ENGINE}\n")).is_err() {
                let _ = fs::remove_dir_all(&staging);
                return Err(io::Error::other("Engine extraction failed"));
            }
            if let Err(e) = fs::rename(&staging, &dir) {
                let _ = fs::remove_dir_all(&staging);
                if !ready(&dir) {
                    return Err(e);
                }
            }
        }
    }
    Ok(dir.join("servo/bifrost-webview"))
}

impl Browser {
    /// The engine profile: the persistent one, or in a private window a throwaway directory of this process.
    fn profile(&self) -> PathBuf {
        if self.private {
            private_profile()
        } else {
            home().join(".local/share/bifrost-browser/profile")
        }
    }

    fn pt(ui: &Ui) -> f32 {
        ui.pt(1.0)
    }

    fn view(&self) -> Option<WebViewId> {
        self.tabs.get(self.current).map(|t| t.view)
    }

    fn strip_shown(&self) -> bool {
        self.tabs.len() > 1
    }

    /// Top of the Favorites bar (below the tab strip), points.
    fn fav_top(&self) -> f32 {
        UNIFIED_BAR_PT + if self.strip_shown() { TAB_STRIP_PT } else { 0.0 }
    }

    /// Loads a bookmark or history entry in the current tab.
    fn lib_open(&mut self, url: &str) {
        self.editing = false;
        if let Some(v) = self.view() {
            self.load_chosen(v, url);
        }
        self.sync_address();
    }

    /// Cmd+D, Cmd+Y, Cmd+Option+B and Cmd+Shift+B. True when the chord was one of them.
    fn lib_chord(&mut self, ctx: &Ctx, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 {
            return false;
        }
        let (alt, shift) = (mods & CHORD_ALT != 0, mods & CHORD_SHIFT != 0);
        match (key, alt, shift) {
            (KEY_D, false, false) => {
                self.editing = false;
                self.downloads.open = false;
                if let Some(s) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
                    let (u, t) = (s.url.clone(), s.title.clone());
                    let r = self.address_rect(&ctx.ui, ctx.cfg.w);
                    self.lib.bookmark_page(&mut self.store, &u, &t, store::now(), r.x + r.w / 2.0);
                }
            }
            (KEY_Y, false, false) | (KEY_B, true, false) => {
                let mode = if key == KEY_Y { library::Mode::History } else { library::Mode::Bookmarks };
                if self.lib.open && self.lib.mode == mode {
                    self.lib.close_all(&mut self.store);
                } else {
                    self.editing = false;
                    self.downloads.open = false;
                    self.lib.show_panel(mode);
                }
            }
            (KEY_B, false, true) => {
                self.lib.favbar = !self.lib.favbar;
                self.lib.save(ctx);
                self.place_web(ctx);
            }
            _ => return false,
        }
        true
    }

    /// Top of the web content, points.
    fn content_top(&self) -> f32 {
        self.fav_top() + self.lib.bar_height() + self.find_height()
    }

    fn start(&mut self) {
        if self.pool.is_some() {
            return;
        }
        match engine().and_then(|e| WebProcessPool::spawn(&e, &self.profile())) {
            Ok(mut p) => {
                // the network log's collector runs at the start of every document (engine devtools module)
                p.dev_collector(&format!("{};{}", network::COLLECTOR_JS, pagetools::CONSOLE_JS));
                self.pool = Some(p);
                self.watchdog.reset(self.clock.elapsed().as_secs_f64());
                self.error = None;
                if self.restore.is_empty() {
                    match self.initial_url.take() {
                        Some(url) => self.new_tab(&url),
                        None if !self.private && self.prefs.restore && self.restore_session() => {}
                        None => {
                            let url = self.prefs.start_url(START);
                            self.new_tab(&url)
                        }
                    }
                } else {
                    for url in std::mem::take(&mut self.restore) {
                        self.new_tab(&url);
                    }
                    self.select(self.restore_sel.min(self.tabs.len().saturating_sub(1)));
                }
            }
            Err(e) => self.error = Some(format!("The web engine could not start: {e}")),
        }
    }

    fn new_tab(&mut self, url: &str) {
        let Some(pool) = &mut self.pool else { return };
        let view = pool.create(url);
        self.tabs.push(Tab { view });
        self.select(self.tabs.len() - 1);
        if url == START {
            self.begin_editing();
        }
    }

    fn select(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.current = i;
        self.editing = false;
        self.downloads.open = false;
        let Some(view) = self.tabs.get(i).map(|t| t.view) else { return };
        if let Some(p) = &mut self.pool {
            p.show(view);
        }
        self.discard_touch();
        self.sync_address();
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.remember_closed(i);
        let tab = self.tabs.remove(i);
        self.inspector.forget(tab.view);
        self.pageui.forget(tab.view);
        self.discard.forget(tab.view);
        self.crash.forget(tab.view);
        self.reader.forget(tab.view);
        if let Some(p) = &mut self.pool {
            p.close(tab.view);
        }
        if self.tabs.is_empty() {
            self.new_tab(START);
        } else {
            self.select(self.current.min(self.tabs.len() - 1).saturating_sub(usize::from(i < self.current)));
        }
    }

    /// Cmd+1 ... Cmd+8 pick that tab, Cmd+9 the last one (Browser).
    fn select_numbered(&mut self, n: usize) {
        let last = self.tabs.len().saturating_sub(1);
        self.select(if n >= 9 { last } else { (n - 1).min(last) });
    }

    /// Ctrl+Tab and Cmd+Shift+] go to the next tab, Ctrl+Shift+Tab and Cmd+Shift+[ to the previous one, wrapping.
    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 1 {
            self.select(if forward { (self.current + 1) % n } else { (self.current + n - 1) % n });
        }
    }

    /// Duplicates the current tab next to it.
    fn duplicate_tab(&mut self) {
        let Some(url) = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| s.url.clone()) else { return };
        let at = self.current + 1;
        self.new_tab(&url);
        let last = self.tabs.len() - 1;
        if at < last {
            let t = self.tabs.remove(last);
            self.tabs.insert(at, t);
            self.select(at);
        }
    }

    /// Moves tab `from` to index `to`, keeping the selection on the same page.
    fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.current = if self.current == from {
            to
        } else if from < self.current && self.current <= to {
            self.current - 1
        } else if to <= self.current && self.current < from {
            self.current + 1
        } else {
            self.current
        };
    }

    /// The tab slot under window pixel x (clamped to the strip).
    fn tab_at(&self, ui: &Ui, w: u32, x: f32) -> Option<usize> {
        let px = x / ui.pt(1.0);
        let rects = self.tab_rects(ui, w);
        rects.iter().position(|r| px < r.x + r.w + 2.0).or(rects.len().checked_sub(1))
    }

    /// Cmd+Shift+N: a private window is a second browser process started with `--private`.
    fn open_private_window(&self) {
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(mut child) = Command::new(exe)
                .arg("--private")
                // its own control socket: binding the default path would take over the main window's
                .env("BIFROST_CONTROL", home().join(format!(".cache/bifrost-browser/private-{}-{}.sock", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()))))
                .spawn() {
                bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                    let _ = child.wait();
                });
            }
        }
    }

    /// Clear Website Data: stops the engine, deletes the profile (cookies, storage, cache) and restarts it on the
    /// same pages.
    fn clear_website_data(&mut self) {
        let urls: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| self.pool.as_ref().and_then(|p| p.state(t.view)).map(|s| s.url.clone()))
            .filter(|u| !u.is_empty() && u != START)
            .collect();
        let sel = self.current;
        self.pool = None; // dropping the pool ends the engine
        self.pageui = pageui::PageUi::default();
        self.tabs.clear();
        // state keyed by the old view ids
        self.discard = Default::default();
        self.ctl = Default::default();
        let _ = fs::remove_dir_all(self.profile());
        self.restore = urls;
        self.restore_sel = sel;
        self.current = 0;
        self.start();
    }

    /// Settings > Import Bookmarks: the newest bookmark export in ~/Downloads is added (links already bookmarked stay).
    fn import_bookmarks(&mut self) {
        let msg = match library::newest_export(&home().join("Downloads")) {
            None => "No bookmark export found in Downloads.".to_string(),
            Some(path) => match fs::read_to_string(&path) {
                Ok(html) => format!("Imported {} bookmarks.", library::import_html(&mut self.store, &html, store::now())),
                Err(_) => "The bookmark export could not be read.".to_string(),
            },
        };
        self.sheet.set_status(&msg);
    }

    /// Applies what the Settings sheet reported.
    fn sheet_action(&mut self, ctx: &Ctx, a: prefs::Action) {
        match a {
            prefs::Action::None => {}
            prefs::Action::Changed => self.prefs.save(ctx),
            prefs::Action::ClearHistory => {
                self.store.clear_history();
                self.sheet.set_status("History cleared.");
            }
            prefs::Action::ImportBookmarks => self.import_bookmarks(),
            prefs::Action::ClearData => {
                self.clear_website_data();
                self.sheet.set_status("Website data cleared.");
                self.place_web(ctx);
            }
            prefs::Action::Close => {
                self.sheet.open = false;
                self.prefs.save(ctx);
            }
        }
    }

    /// A press on the page held still for `LONG_PRESS_S`: cancel the touch and open the context menu there.
    fn fire_long_press(&mut self) {
        let Some((_, x, y, id)) = self.long.take() else { return };
        self.swallow = true;
        self.pageui.last_point = (x, y);
        if let Some(p) = &mut self.pool {
            p.touch(TouchKind::Cancel, id, x, y);
            if p.contains(x, y) {
                p.secondary_click(x, y);
            }
        }
    }

    /// The address of the page in the current tab, when it is a web page (http or https).
    fn web_url(&self) -> Option<String> {
        let v = self.view()?;
        let url = self.pool.as_ref()?.state(v)?.url.clone();
        (url.starts_with("http://") || url.starts_with("https://")).then_some(url)
    }

    /// File > Share, View > Show Reader and the History menu (Back and Forward entries of the selected tab).
    fn menus(&self) -> Vec<AppMenu> {
        let reader_ok = self.view().is_some_and(|v| self.reader.button(v));
        let reader_on = self.view().is_some_and(|v| self.reader.is_active(v));
        let page = self.view().is_some();
        let finished = self.downloads.items.iter().any(|i| i.state != downloads::State::Active);
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_SHARE, "Share\u{2026}").enabled(self.web_url().is_some()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_AUTOFILL, "AutoFill Password").enabled(self.view().is_some_and(|v| self.autofill.has_form(v))),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_DOWNLOADS, "Show Downloads").enabled(!self.downloads.items.is_empty()),
                    AppMenuItem::new(MENU_CLEAR_DOWNLOADS, "Clear Finished Downloads").enabled(finished),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_READER, "Show Reader").key("cmd+shift+r").checked(reader_on).enabled(reader_ok),
                    AppMenuItem::new(MENU_OVERVIEW, "Show All Tabs").key("cmd+shift+backslash").checked(self.overview.open).enabled(!self.tabs.is_empty()),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_ZOOM_IN, "Zoom In").key("cmd+=").enabled(page),
                    AppMenuItem::new(MENU_ZOOM_OUT, "Zoom Out").key("cmd+-").enabled(page),
                    AppMenuItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(page),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIND, "Find in Page\u{2026}").key("cmd+f").enabled(page),
                    AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(self.find.open),
                    AppMenuItem::new(MENU_FIND_PREV, "Find Previous").key("cmd+shift+g").enabled(self.find.open),
                ],
            },
            AppMenu { title: "Develop".into(), items: self.dev.menu_items(page) },
            AppMenu { title: "History".into(), items: self.history_menu() },
        ]
    }

    /// Share sends the page address to the shell's share menu (Copy and the apps that take links). The shell
    /// answers with the chosen target; the app keeps no state for it.
    fn share_page(&self, ctx: &Ctx) -> bool {
        if let Some(url) = self.web_url() {
            let anchor = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            let _ = bifrost_app::share::show_picker(ctx, &[bifrost_app::share::Payload::Url(url)], anchor);
        }
        false
    }

    /// File > AutoFill Password for the selected tab (autofill.rs).
    fn autofill_page(&mut self, ctx: &Ctx) -> bool {
        let Some(v) = self.view() else { return false };
        let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
        self.autofill.start(ctx, v, &url)
    }

    /// View > Show All Tabs and Cmd+Shift+backslash (overview.rs). Opening it closes the downloads list.
    fn toggle_overview(&mut self) {
        self.overview.open = !self.overview.open && !self.tabs.is_empty();
        if self.overview.open {
            self.downloads.open = false;
        }
    }

    fn toggle_reader(&mut self) {
        let (Some(v), Some(p)) = (self.view(), self.pool.as_mut()) else { return };
        let url = p.state(v).map(|s| s.url.clone()).unwrap_or_default();
        self.reader.toggle(p, v, &url, self.dark.unwrap_or(false));
    }

    fn sync_address(&mut self) {
        if self.editing {
            return;
        }
        let mut url = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| s.url.clone()).unwrap_or_default();
        if let Some(o) = self.view().and_then(|v| self.reader.original(v)) {
            url = o.to_string();
        }
        self.address.clear();
        if url != START {
            self.address.push_str(&url);
        }
    }

    fn refresh_suggestions(&mut self) {
        self.sugg = if self.editing { suggest::suggestions(&self.store, &self.address.to_string(), store::now()) } else { Vec::new() };
        self.sel = None;
    }

    fn begin_editing(&mut self) {
        self.editing = true;
        let url = self.address.to_string();
        self.address.clear();
        self.address.push_str(&url);
        self.address.command(bifrost_app::controls::Command::SelectAll, "");
        self.refresh_suggestions();
    }

    fn commit_address(&mut self) {
        let text = match self.sel.and_then(|i| self.sugg.get(i)) {
            Some(s) => s.target.clone(),
            None => self.address.to_string(),
        };
        self.editing = false;
        self.downloads.open = false;
        self.sugg.clear();
        self.sel = None;
        if text.trim().is_empty() {
            self.sync_address();
            return;
        }
        let url = self.prefs.url_for_input(&text);
        if let Some(v) = self.view() {
            self.load_chosen(v, &url);
        }
        self.address.clear();
        self.address.push_str(&url);
    }

    fn toolbar(&self) -> Toolbar {
        let st = self.view().and_then(|v| self.pool.as_ref()?.state(v));
        let fwd = st.as_ref().is_some_and(|s| s.can_go_forward);
        let back = self.view().is_some_and(|v| self.can_back(v));
        let loading = st.as_ref().is_some_and(|s| s.loading);
        Toolbar::new(vec![
            ToolbarItem::icon(BACK, "Back", "chevron.left").in_group(1).enabled(back),
            ToolbarItem::icon(FORWARD, "Forward", "chevron.right").in_group(1).enabled(fwd),
            ToolbarItem::flexible_space(),
            ToolbarItem::custom(ADDRESS, "Address", 0.0),
            ToolbarItem::flexible_space(),
            if loading {
                ToolbarItem::icon(RELOAD, "Stop", "xmark").enabled(true)
            } else {
                ToolbarItem::icon(RELOAD, "Reload", "arrow.clockwise").enabled(self.view().is_some())
            },
        ]
        .into_iter()
        // Browser shows the Downloads button once something was downloaded
        .chain((!self.downloads.items.is_empty()).then(|| ToolbarItem::icon(DOWNLOADS, "Downloads", "arrow.down.circle")))
        .chain(std::iter::once(ToolbarItem::icon(NEW_TAB, "New Tab", "plus")))
        .collect())
    }

    fn bar_bounds(ui: &Ui, w: u32) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        Rect { x, y: 0.0, w: (w as f32 / ui.pt(1.0) - x - 12.0).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The address field takes the space between the navigation and the trailing buttons, centred, at most
    /// `ADDRESS_MAX_PT` wide.
    fn address_rect(&self, ui: &Ui, w: u32) -> Rect {
        let bar = Self::bar_bounds(ui, w);
        let lead = bar.x + 2.0 * widgets::CAPSULE_PT + widgets::TOOLBAR_GAP_PT * 2.0;
        let extra = if self.downloads.items.is_empty() { 0.0 } else { widgets::CAPSULE_PT + widgets::TOOLBAR_GAP_PT };
        let trail = bar.x + bar.w - extra - 2.0 * widgets::CAPSULE_PT - widgets::TOOLBAR_GAP_PT * 3.0;
        let room = (trail - lead).max(80.0);
        let width = room.min(ADDRESS_MAX_PT);
        let window_mid = w as f32 / ui.pt(1.0) / 2.0;
        let x = (window_mid - width / 2.0).clamp(lead, trail - width);
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: width, h }
    }

    fn tab_rects(&self, ui: &Ui, w: u32) -> Vec<Rect> {
        let total = w as f32 / ui.pt(1.0) - 16.0;
        let n = self.tabs.len().max(1) as f32;
        let tw = total / n;
        (0..self.tabs.len()).map(|i| Rect { x: 8.0 + i as f32 * tw, y: UNIFIED_BAR_PT + 3.0, w: tw - 4.0, h: TAB_STRIP_PT - 6.0 }).collect()
    }

    fn close_rect(r: Rect) -> Rect {
        Rect { x: r.x + 6.0, y: r.y + (r.h - 16.0) / 2.0, w: 16.0, h: 16.0 }
    }

    /// Develop menu shortcuts: Cmd+Option+U View Source, Cmd+Shift+R Reload From Origin, Cmd+Option+R Responsive Design Mode.
    fn dev_key(&mut self, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 {
            return false;
        }
        let Some(v) = self.view() else { return false };
        match (key, mods & CHORD_ALT != 0, mods & CHORD_SHIFT != 0) {
            (KEY_U, true, _) => {
                let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
                if url.starts_with("http") {
                    // curl runs on a thread; tick() opens the source tab when it finishes.
                    // the result lands in the slot and wakes the idle loop; tick() collects it
                    let u = url.clone();
                    let slot = inbox::spawn(move || devmenu::fetch_source(&u));
                    self.sources.push((url, slot));
                }
            }
            (KEY_R, true, true) => {
                if let Some(p) = &mut self.pool {
                    p.reload_ignoring_cache(v);
                }
            }
            (KEY_R, true, false) => self.dev.toggle_responsive(),
            _ => return false,
        }
        true
    }

    fn place_web(&mut self, ctx: &Ctx) {
        let top = ctx.ui.pt(self.content_top()) as i32;
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        let dark = ctx.cfg.dark;
        let reserved = self.inspector.reserved_px(&ctx.ui);
        if let Some(p) = &mut self.pool {
            let area = (0, top, w, h.saturating_sub(top as u32 + reserved));
            match &self.dev.responsive {
                Some(r) => {
                    p.set_viewport_override(area, r.w, r.h, r.dpr as f32);
                }
                None => p.set_frame(area.0, area.1, area.2, area.3, ctx.cfg.scale),
            }
            p.set_dark(dark);
        }
        self.dark = Some(dark);
    }

    fn handle(&mut self, events: Vec<WebViewEvent>) {
        for ev in events {
            match ev {
                WebViewEvent::LoadStarted(v) | WebViewEvent::UrlChanged(v) if self.pool.as_ref().and_then(|p| p.state(v)).is_some() => {
                    // a response the browser cannot display is saved instead (downloads.rs)
                    let url = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone()).unwrap_or_default();
                    self.downloads.ua = self
                        .ctl
                        .ua
                        .clone()
                        .or_else(|| self.dev.user_agent().map(String::from))
                        .or_else(|| std::env::var("BIFROST_BROWSER_UA").ok())
                        .unwrap_or_else(|| downloads::DEFAULT_UA.to_string());
                    self.downloads.consider(v, &url);
                    if Some(v) == self.view() {
                        self.sync_address();
                    }
                }
                WebViewEvent::EvalResult(v, text) if self.downloads.on_eval(v, &text) => {}
                WebViewEvent::EvalResult(v, text) if self.favicons.on_eval(v, &text) => {}
                WebViewEvent::EvalResult(v, text) if self.autofill.on_eval(v, &text) => {}
                WebViewEvent::LoadFinished(v) => {
                    // cookies for the next download probe from this page (downloads.rs); password field (autofill.rs)
                    if let Some(p) = self.pool.as_mut() {
                        p.eval(v, downloads::COOKIE_JS);
                        p.eval(v, favicon::FAV_JS);
                        p.eval(v, autofill::FORM_JS);
                    }
                    let loaded = self.pool.as_ref().and_then(|p| p.state(v)).map(|s| s.url.clone());
                    if let (Some(u), Some(p)) = (loaded, self.pool.as_mut()) {
                        self.reader.on_load(p, v, &u);
                    }
                    if let Some(s) = self.pool.as_ref().and_then(|p| p.state(v)) {
                        let (u, t) = (s.url.clone(), s.title.clone());
                        if !u.starts_with(reader::DATA_PREFIX) {
                            self.store.visit(&u, &t, store::now());
                        }
                    }
                }
                WebViewEvent::CreateWebView { id, .. } => {
                    self.tabs.push(Tab { view: id });
                    self.select(self.tabs.len() - 1);
                }
                WebViewEvent::Closed(v) => {
                    self.downloads.forget(v);
                    self.reader.forget(v);
                    self.favicons.forget(v);
                    self.autofill.forget(v);
                    if self.pool.as_ref().is_none_or(|p| p.state(v).is_none()) && !self.tabs.iter().any(|t| t.view == v) {
                        continue;
                    }
                    if let Some(i) = self.tabs.iter().position(|t| t.view == v) {
                        self.close_tab(i);
                    }
                }
                WebViewEvent::EngineExited => {
                    let urls: Vec<String> = self
                        .tabs
                        .iter()
                        .filter_map(|t| self.pool.as_ref().and_then(|p| p.state(t.view)).map(|s| s.url.clone()))
                        .filter(|u| !u.is_empty())
                        .collect();
                    let now_s = self.clock.elapsed().as_secs_f64();
                    let last = urls.first().cloned().unwrap_or_default();
                    self.pool = None;
                    self.pageui = pageui::PageUi::default();
                    self.tabs.clear();
                    self.restore = urls;
                    self.restore_sel = self.current;
                    self.current = 0;
                    if self.storm.allow_restart(now_s) {
                        let line = session::recover::crash_line(&last, "engine exited", self.storm.restarts_in_window());
                        self.crash_log.push(line);
                        self.start();
                    } else {
                        self.crash_log.push(session::recover::crash_line(&last, "restart storm: engine not restarted", self.storm.restarts_in_window()));
                        self.error = Some("The web engine stopped. Choose Reload or New Tab to restart it.".into());
                    }
                }
                WebViewEvent::ContentProcessTerminated(v, _) => {
                    self.pageui.forget(v);
                    // webViewWebContentProcessDidTerminate: reload only this tab and say so in a banner (crashbanner.rs);
                    // a tab that keeps crashing is left alone behind a Reload button
                    let now = self.clock.elapsed().as_secs_f64();
                    if self.crash.on_crash(v, now) {
                        if let Some(p) = &mut self.pool {
                            p.reload(v);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Applies a path status. Coming back online loads the tabs that were waiting for the network.
    fn set_path(&mut self, status: Status) -> bool {
        if !self.net.set_status(status) || self.net.offline() {
            return false;
        }
        for (view, url) in self.net.take_waiting() {
            if let Some(p) = self.pool.as_mut().filter(|p| p.state(view).is_some()) {
                p.load(view, &url);
            }
        }
        true
    }

    /// Loads an address the user chose. While the network is down the tab shows the offline page instead, and the
    /// address loads again when the network comes back.
    fn load_chosen(&mut self, view: WebViewId, url: &str) {
        let offline = self.net.offline();
        let Some(p) = self.pool.as_mut() else { return };
        if offline {
            self.net.hold(view, url);
            p.load(view, &system::offline_url(url));
        } else {
            self.net.forget(view);
            p.load(view, url);
        }
    }

    /// The pointer shape over the chrome: an I-beam over the address field, unless a sheet or a page menu is up.
    fn update_pointer(&mut self, ctx: &Ctx, x: f32, y: f32) {
        let ui = &ctx.ui;
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let over = !self.sheet.open && !self.pageui.active() && py < UNIFIED_BAR_PT && self.address_rect(ui, ctx.cfg.w).contains(px, py);
        if let Some(change) = self.pointer.update(over) {
            system::send_pointer(ctx, change);
        }
    }

    /// Cmd+Shift+C copies the page's link (URL and title); Cmd+Shift+K copies the newest downloaded image.
    fn system_chord(&mut self, ctx: &Ctx, key: u32, mods: u32) -> bool {
        if mods & (CHORD_CTRL | CHORD_META) == 0 || mods & CHORD_SHIFT == 0 || mods & CHORD_ALT != 0 {
            return false;
        }
        match key {
            KEY_C => {
                let page = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|s| (s.url.clone(), s.title.clone()));
                match page {
                    Some((url, title)) if url.starts_with("http") => {
                        self.clip = system::send_copy(ctx, system::Clip::Link { url, title });
                        true
                    }
                    _ => false,
                }
            }
            KEY_K => {
                let image = system::newest_image(&self.downloads.items).and_then(|i| system::read_image(&i.path));
                match image {
                    Some((uti, bytes)) => {
                        self.clip = system::send_copy(ctx, system::Clip::Image { uti, bytes });
                        true
                    }
                    None => false,
                }
            }
            KEY_BACKSLASH => {
                self.toggle_overview();
                true
            }
            _ => false,
        }
    }

    fn activate(&mut self, id: u32) {
        if id != DOWNLOADS {
            self.downloads.open = false;
        }
        let view = self.view();
        match (id, &mut self.pool, view) {
            (BACK, Some(p), Some(v)) => match self.discard.back_target(v).map(str::to_string).filter(|_| !p.state(v).is_some_and(|s| s.can_go_back)) {
                // a revived tab has no engine history behind it: load the app-side previous entry
                Some(url) => p.load(v, &url),
                None => p.go_back(v),
            },
            (FORWARD, Some(p), Some(v)) => p.go_forward(v),
            (RELOAD, Some(p), Some(v)) => {
                // the button is Stop while the page loads (Browser)
                if p.state(v).is_some_and(|s| s.loading) { p.stop_loading(v) } else { p.reload(v) }
            }
            (RELOAD, None, _) => {
                self.storm.clear();
                self.start()
            }
            (NEW_TAB, _, _) => {
                if self.pool.is_none() {
                    self.start();
                } else {
                    self.new_tab(START);
                }
            }
            (DOWNLOADS, _, _) => self.downloads.open = !self.downloads.open,
            (ADDRESS, _, _) => self.begin_editing(),
            _ => {}
        }
    }

    fn chrome_hit(&self, ui: &Ui, w: u32, x: f32, y: f32) -> Option<Press> {
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if self.editing {
            if let Some(i) = suggest::hit(self.address_rect(ui, w), self.sugg.len(), px, py) {
                return Some(Press::Sugg(i));
            }
        }
        if py < UNIFIED_BAR_PT {
            if !self.editing && self.view().is_some_and(|v| self.reader.button(v)) && reader::button_rect(self.address_rect(ui, w)).contains(px, py) {
                return Some(Press::Reader);
            }
            if !self.editing && self.view().is_some_and(|v| self.pool.as_ref().and_then(|p| p.state(v)).is_some()) && star_rect(self.address_rect(ui, w)).contains(px, py) {
                return Some(Press::Bookmark);
            }
            if self.address_rect(ui, w).contains(px, py) {
                return Some(Press::Bar(ADDRESS));
            }
            return self.bar_layout.as_ref().and_then(|l| l.hit(px, py)).map(Press::Bar);
        }
        if self.strip_shown() && py < UNIFIED_BAR_PT + TAB_STRIP_PT {
            for (i, r) in self.tab_rects(ui, w).iter().enumerate() {
                if Self::close_rect(*r).contains(px, py) {
                    return Some(Press::TabClose(i));
                }
                if r.contains(px, py) {
                    return Some(Press::Tab(i));
                }
            }
            return None;
        }
        match self.banner_hit(w as f32 / ui.pt(1.0), px, py) {
            Some(crashbanner::BannerHit::Close) => return Some(Press::BannerClose),
            Some(crashbanner::BannerHit::Reload) => return Some(Press::BannerReload),
            None => {}
        }
        Some(Press::Web)
    }

    /// Develop menu: Responsive Design Mode, its size and pixel ratio, and the user agent preset.
    fn dev_menu_action(&mut self, ctx: &Ctx, id: u32) {
        match devmenu::pick(id) {
            Some(devmenu::Pick::Responsive) => self.dev.toggle_responsive(),
            Some(devmenu::Pick::UserAgent(i)) => self.set_user_agent(i),
            Some(devmenu::Pick::Size(i)) => self.dev.preset(i),
            Some(devmenu::Pick::Dpr(d)) => self.dev.set_dpr(d),
            None => return,
        }
        self.place_web(ctx);
    }

    /// Develop > User Agent. The engine reads the user agent at start-up, so the choice takes effect on the next start.
    fn set_user_agent(&mut self, i: usize) {
        self.dev.set_ua(i);
        let ua = self.dev.user_agent();
        self.ctl.ua = ua.map(String::from);
        if let Some(p) = self.pool.as_mut() {
            p.set_user_agent(ua);
        }
    }

    /// Clicking the address field's star: bookmarks the open page, or removes its bookmark when it has one.
    fn toggle_bookmark(&mut self) {
        let Some((url, title)) = self.view().and_then(|v| self.pool.as_ref()?.state(v)).map(|st| (st.url.clone(), st.title.clone())) else { return };
        self.store.toggle_bookmark(&url, &title, store::now());
    }
}

impl App for Browser {
    fn id(&self) -> &str {
        "is.olibuijr.browser.desktop"
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menus()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_READER => {
                self.toggle_reader();
                true
            }
            MENU_SHARE => self.share_page(ctx),
            MENU_FIND => {
                self.editing = false;
                if !self.find.open {
                    self.show_find();
                }
                true
            }
            MENU_FIND_NEXT => {
                self.find_step(FindDirection::Forward);
                true
            }
            MENU_FIND_PREV => {
                self.find_step(FindDirection::Backward);
                true
            }
            MENU_ZOOM_IN => {
                self.zoom_by(1);
                true
            }
            MENU_ZOOM_OUT => {
                self.zoom_by(-1);
                true
            }
            MENU_ACTUAL_SIZE => {
                self.zoom_by(0);
                true
            }
            MENU_DOWNLOADS => {
                self.downloads.open = !self.downloads.items.is_empty();
                true
            }
            MENU_CLEAR_DOWNLOADS => {
                self.downloads.clear_finished();
                true
            }
            MENU_AUTOFILL => self.autofill_page(ctx),
            MENU_OVERVIEW => {
                self.toggle_overview();
                true
            }
            _ if devmenu::pick(id).is_some() => {
                self.dev_menu_action(ctx, id);
                true
            }
            _ => match tabdiscard::history_target(id) {
                Some(target) => {
                    self.jump_to(target);
                    true
                }
                None => false,
            },
        }
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.wants_keyboard(true);
        self.inspector.load(ctx);
        self.prefs = prefs::Prefs::load(ctx);
        self.lib.load(ctx);
        // No unsaved state: the shell may stop the browser at once on quit (docs/bifrost-ui.md "Sudden termination").
        ctx.enable_sudden_termination();
        self.start();
        self.place_web(ctx);
        self.net_request = Some(network_path::request_path(ctx));
    }

    fn wake_fd(&self) -> Option<i32> {
        self.pool.as_ref().filter(|p| p.alive()).map(|p| p.fd())
    }

    /// The only periodic wake while the window is shown is the watchdog ping (and the short scroll probe of a tab
    /// being discarded); an idle static page costs nothing else. A hidden window is never asked.
    fn next_wake(&self, t: f64) -> Option<f64> {
        self.next_timed_ms().map(|ms| t + ms)
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        if self.dark != Some(ctx.cfg.dark) {
            self.place_web(ctx);
        }
        let Some(p) = &mut self.pool else { return false };
        let (events, frame) = p.poll();
        // heartbeat: ping the engine, and kill it when it stops answering (poll reports the exit; handle restarts)
        let now_s = self.clock.elapsed().as_secs_f64();
        if p.alive() {
            if p.take_pong() {
                self.watchdog.pong(now_s);
            }
            match self.watchdog.check(now_s) {
                session::recover::Action::SendPing => p.send_ping(now_s as u64),
                session::recover::Action::KillWedged => {
                    self.crash_log.push(session::recover::crash_line("", "engine wedged: no pong", self.storm.restarts_in_window()));
                    p.kill_engine();
                }
                session::recover::Action::None => {}
            }
        }
        let long_fired = self.long.is_some_and(|(t0, ..)| now_s - t0 >= LONG_PRESS_S);
        if long_fired {
            self.fire_long_press();
        }
        let committed: Vec<WebViewId> = events.iter().filter_map(|e| if let WebViewEvent::LoadCommitted(v) = e { Some(*v) } else { None }).collect();
        // a tab that committed a page other than the offline page (a data: URL) no longer waits for the network
        for v in &committed {
            let offline_page = self.pool.as_ref().and_then(|p| p.state(*v)).is_some_and(|s| s.url.starts_with("data:"));
            if !offline_page {
                self.net.forget(*v);
            }
        }
        let mut changed = !events.is_empty() || long_fired;
        for ev in &events {
            changed |= self.inspector.on_event(ev);
            self.ctl_on_event(ev);
            self.discard_event(ev);
        }
        for line in self.inspector.take_log() {
            ctx.log(&line);
        }
        for line in std::mem::take(&mut self.crash_log) {
            ctx.log(&line);
        }
        let fr = self.pool.as_ref().map(|p| p.frame()).unwrap_or_default();
        for ev in &events {
            changed |= self.pageui.on_event(ev, &mut self.pool, fr, &ctx.ui);
        }
        self.handle(events);
        changed |= self.ctl_tick();
        self.inspector.flush_prefs(ctx);
        if self.ctl.take_relayout() | self.inspector.take_relayout() {
            self.place_web(ctx);
            changed = true;
        }
        for v in committed {
            if let Some(i) = self.tabs.iter().position(|t| t.view == v) {
                self.apply_zoom(i);
            }
        }
        self.save_session(t);
        let mut ready = Vec::new();
        self.sources.retain(|(url, slot)| match inbox::take(slot) {
            Some(src) => {
                ready.push(devmenu::source_document(url, &src));
                false
            }
            None => true,
        });
        let opened = !ready.is_empty();
        for doc in ready {
            self.new_tab(&doc);
        }
        let (dl_changed, stop) = self.downloads.tick();
        let dock = system::dock_progress(&self.downloads.items);
        if system::dock_changed(self.dock, dock) {
            self.dock = dock;
            system::send_dock(ctx, dock);
        }
        let dl_changed = self.favicons.tick() || dl_changed;
        if let Some(p) = &mut self.pool {
            // Servo 0.7.0 cannot cancel a load (the engine's `stop` is a no-op), so leave the download's page:
            // back to where the link was, else a blank page
            for v in stop {
                if p.state(v).is_some_and(|st| st.can_go_back) {
                    p.go_back(v);
                } else {
                    p.load(v, "about:blank");
                }
            }
        }
        perf::tick(&mut self.store);
        self.discard_tick();
        frame || changed || opened || dl_changed
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(p) = &mut self.pool {
            p.set_focused(false);
            p.set_suspended(true);
        }
        perf::on_exit(&mut self.store);
    }

    fn resume(&mut self, ctx: &Ctx) {
        self.net_request = Some(network_path::request_path(ctx));
        // no pings ran while suspended: do not read the gap as a wedged engine
        self.watchdog.reset(self.clock.elapsed().as_secs_f64());
        if let Some(p) = &mut self.pool {
            p.set_focused(true);
            p.set_suspended(false);
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        // AutoFill: the logins listing, then the chosen password, go into the page (autofill.rs)
        if let Some((view, js)) = self.autofill.on_reply(ctx, request, result) {
            if let Some(p) = self.pool.as_mut() {
                p.eval(view, &js);
            }
            return false;
        }
        if self.net_request == Some(request) {
            self.net_request = None;
            match network_path::decode_reply(result.clone()) {
                Ok(path) => return self.set_path(path.status),
                Err(e) => ctx.log(&format!("network path unavailable: {e}")),
            }
            return false;
        }
        // a copy the service refused (an older shell) goes to the synchronous pasteboard instead
        if let Some((id, clip)) = self.clip.take() {
            if id == request {
                if result.is_err() {
                    system::fall_back(ctx, &clip);
                }
            } else {
                self.clip = Some((id, clip));
            }
        }
        false
    }

    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match network_path::path_event(service, event, body) {
            Some(Ok(path)) => self.set_path(path.status),
            _ => false,
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let ui = &ctx.ui;
        if let Input::Touch { x, y, .. } = ev {
            self.update_pointer(ctx, x, y);
        }
        // The SDK has no pointer-leave or window-deactivate input (runtime.rs `Input`), so the tab hover is cleared on
        // anything that is not a pointer position: keys, commands, menus, activation and touch cancel.
        if !matches!(ev, Input::Touch { kind: TouchKind::Down | TouchKind::Move | TouchKind::Up, .. }) {
            self.hover = None;
        }
        match ev {
            Input::Open(url) => {
                if self.pool.is_some() {
                    self.new_tab(&url);
                } else {
                    self.initial_url = Some(url);
                    self.start();
                }
                self.place_web(ctx);
                true
            }
            Input::Activate(id) => {
                if id >= TAB0 {
                    self.select((id - TAB0) as usize);
                } else {
                    self.activate(id as u32);
                }
                self.place_web(ctx);
                true
            }
            Input::Context { x, y } => {
                if self.pageui.active() {
                    return false;
                }
                if let Some(p) = self.pool.as_mut().filter(|p| p.contains(x, y)) {
                    self.pageui.last_point = (x, y);
                    p.secondary_click(x, y);
                }
                true
            }
            Input::Touch { kind, x, y, .. } if self.sheet.open => {
                self.hover = None;
                if matches!(kind, TouchKind::Down | TouchKind::Move) {
                    self.sheet.touch_drag(ui, ctx.cfg.w, ctx.cfg.h, kind == TouchKind::Down, x, y);
                }
                if kind == TouchKind::Up {
                    let a = self.sheet.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.prefs);
                    self.sheet_action(ctx, a);
                }
                true
            }
            Input::Key(k) if self.sheet.open => {
                let a = match k.as_chord() {
                    Some((KEY_COMMA, mods)) if mods & (CHORD_CTRL | CHORD_META) != 0 => prefs::Action::Close,
                    Some(_) => prefs::Action::None,
                    None => self.sheet.key(&k, &mut self.prefs),
                };
                self.sheet_action(ctx, a);
                true
            }
            // the tab overview covers the content: a card selects its tab, anything else is ignored (overview.rs)
            Input::Touch { kind, x, y, .. } if self.overview.open => {
                self.hover = None;
                if kind == TouchKind::Down {
                    let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
                    let wpt = ctx.cfg.w as f32 / ui.pt(1.0);
                    if let Some(i) = overview::hit(self.tabs.len(), wpt, self.content_top(), px, py) {
                        self.overview.open = false;
                        self.select(i);
                    }
                }
                true
            }
            Input::Touch { kind, x, y, .. } if self.lib.wants_touch(ui, y, self.fav_top()) => {
                if kind == TouchKind::Up {
                    let (top, t) = (self.fav_top(), self.clock.elapsed().as_secs_f64());
                    let act = self.lib.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.store, top, t, store::now());
                    if let library::Act::Open(u) = act {
                        self.lib_open(&u);
                    }
                }
                true
            }
            Input::Key(k) if self.lib.captures_keys() && k.as_chord().is_none() => {
                let hpt = ctx.cfg.h as f32 / ui.pt(1.0);
                if let library::Act::Open(u) = self.lib.key(&k, &mut self.store, store::now(), hpt) {
                    self.lib_open(&u);
                }
                true
            }
            Input::Touch { kind, .. } if self.swallow && self.pageui.active() && matches!(kind, TouchKind::Up | TouchKind::Cancel) => {
                // the finger of a long press lifting must not tap the menu it just opened
                self.swallow = false;
                self.press = None;
                true
            }
            Input::Touch { kind, x, y, .. } if self.pageui.active() => {
                match kind {
                    TouchKind::Up => {
                        if let pageui::Outcome::Inspect = self.pageui.touch_up(ui, ctx.cfg.w, ctx.cfg.h, x, y, &mut self.pool) {
                            self.inspect_element();
                            self.place_web(ctx);
                        }
                    }
                    TouchKind::Move => self.pageui.touch_move(ui, ctx.cfg.w, ctx.cfg.h, x, y),
                    _ => {}
                }
                true
            }
            Input::Key(k) if self.pageui.active() => {
                if let pageui::Outcome::Inspect = self.pageui.key(&k, &mut self.pool) {
                    self.inspect_element();
                    self.place_web(ctx);
                }
                true
            }
            Input::Touch { kind, id, x, y, .. } => {
                self.hover = Some((x, y));
                let view = self.view();
                if self.inspector.touch(ctx, kind, x, y, view, self.pool.as_mut()) {
                    if self.inspector.take_relayout() {
                        self.place_web(ctx);
                    }
                    return true;
                }
                if kind == TouchKind::Down && self.downloads.open {
                    let wpt = ctx.cfg.w as f32 / ui.pt(1.0);
                    match self.downloads.hit(wpt, x / ui.pt(1.0), y / ui.pt(1.0)) {
                        Some(a) => {
                            match a {
                                downloads::Action::Cancel(i) => {
                                    self.downloads.cancel(i);
                                }
                                downloads::Action::Reveal(i) => self.downloads.reveal(i),
                                downloads::Action::Open(i) => self.downloads.open_file(i),
                                downloads::Action::Clear => self.downloads.clear_finished(),
                                downloads::Action::Inside => {}
                            }
                            return true;
                        }
                        None => {
                            if self.chrome_hit(ui, ctx.cfg.w, x, y) != Some(Press::Bar(DOWNLOADS)) {
                                self.downloads.open = false;
                            }
                        }
                    }
                }
                if kind == TouchKind::Down {
                    self.inspector.blur();
                    let hit = self.chrome_hit(ui, ctx.cfg.w, x, y);
                    self.press = hit.map(|h| (h, x, y));
                    self.drag = false;
                    self.swallow = false;
                    self.long = (hit == Some(Press::Web)).then(|| (self.clock.elapsed().as_secs_f64(), x, y, id));
                    if hit == Some(Press::Web) {
                        self.pageui.last_point = (x, y);
                        self.editing = false;
                        self.sync_address();
                    }
                }
                let Some((press, x0, y0)) = self.press else { return false };
                if press == Press::Web {
                    let up = matches!(kind, TouchKind::Up | TouchKind::Cancel);
                    if self.swallow {
                        if up {
                            self.swallow = false;
                            self.press = None;
                        }
                        return true;
                    }
                    let slop = ui.pt(TAP_SLOP_PT);
                    if (x - x0).abs() > slop || (y - y0).abs() > slop {
                        self.long = None;
                    }
                    // a held, unmoved finger lifting after the long-press time is also a long press
                    if kind == TouchKind::Up && self.long.is_some_and(|(t0, ..)| self.clock.elapsed().as_secs_f64() - t0 >= LONG_PRESS_S) {
                        self.fire_long_press();
                        self.swallow = false;
                        self.press = None;
                        return true;
                    }
                    if up {
                        self.long = None;
                    }
                    if let Some(p) = &mut self.pool {
                        p.touch(kind, id, x, y);
                    }
                    if up {
                        self.press = None;
                    }
                    return false;
                }
                if let (Press::Tab(i), TouchKind::Move) = (press, kind) {
                    // dragging a tab reorders the strip (Browser); the dragged tab becomes the selected one
                    if self.drag || (x - x0).abs() > ui.pt(TAP_SLOP_PT) {
                        if !self.drag {
                            self.drag = true;
                            self.select(i);
                        }
                        if let Some(to) = self.tab_at(ui, ctx.cfg.w, x).filter(|&to| to != i) {
                            self.move_tab(i, to);
                            self.press = Some((Press::Tab(to), x0, y0));
                        }
                        return true;
                    }
                }
                if matches!(kind, TouchKind::Up) {
                    self.press = None;
                    if std::mem::take(&mut self.drag) {
                        return true;
                    }
                    let slop = ui.pt(TAP_SLOP_PT);
                    if (x - x0).abs() > slop || (y - y0).abs() > slop {
                        return true;
                    }
                    let strip_before = self.strip_shown();
                    match press {
                        Press::Bar(id) => self.activate(id),
                        Press::Tab(i) => self.select(i),
                        Press::TabClose(i) => self.close_tab(i),
                        Press::Sugg(i) => {
                            self.sel = Some(i);
                            self.commit_address();
                        }
                        Press::BannerClose => self.banner_activate(crashbanner::BannerHit::Close),
                        Press::BannerReload => self.banner_activate(crashbanner::BannerHit::Reload),
                        Press::Reader => self.toggle_reader(),
                        Press::Bookmark => self.toggle_bookmark(),
                        Press::Web => {}
                    }
                    if strip_before != self.strip_shown() {
                        self.place_web(ctx);
                    }
                } else if kind == TouchKind::Cancel {
                    self.press = None;
                    self.drag = false;
                }
                true
            }
            Input::Key(k) => {
                let view = self.view();
                if let Some((KEY_TAB, mods)) = k.as_chord() {
                    if mods & CHORD_CTRL != 0 {
                        self.cycle_tab(mods & CHORD_SHIFT == 0);
                        self.place_web(ctx);
                        return true;
                    }
                }
                if let Some(redraw) = self.inspector.key(&k, self.pool.as_mut(), view) {
                    if self.inspector.take_relayout() {
                        self.place_web(ctx);
                    }
                    return redraw;
                }
                if let Some((key, mods)) = k.as_chord() {
                    if key == KEY_R && mods & (CHORD_CTRL | CHORD_META) != 0 && mods & CHORD_SHIFT != 0 && mods & CHORD_ALT == 0 {
                        self.toggle_reader();
                        return true;
                    }
                    if self.dev_key(key, mods) {
                        self.place_web(ctx);
                        return true;
                    }
                    if self.system_chord(ctx, key, mods) {
                        return true;
                    }
                    if self.lib_chord(ctx, key, mods) {
                        return true;
                    }
                    let cmd = mods & (CHORD_CTRL | CHORD_META) != 0 && mods & CHORD_SHIFT == 0;
                    let shift = mods & CHORD_SHIFT != 0;
                    let before = (self.strip_shown(), self.find.open);
                    if mods & (CHORD_CTRL | CHORD_META) != 0 {
                        let used = match (key, shift) {
                            (KEY_F, false) => {
                                self.editing = false;
                                self.show_find();
                                true
                            }
                            (KEY_G, s) if self.find.open => {
                                self.find_step(if s { FindDirection::Backward } else { FindDirection::Forward });
                                true
                            }
                            (KEY_EQUAL, _) => {
                                self.zoom_by(1);
                                true
                            }
                            (KEY_MINUS, false) => {
                                self.zoom_by(-1);
                                true
                            }
                            (KEY_0, false) => {
                                self.zoom_by(0);
                                true
                            }
                            (KEY_T, true) => {
                                self.reopen_closed();
                                true
                            }
                            (KEY_N, true) => {
                                self.open_private_window();
                                true
                            }
                            (KEY_LEFTBRACE, true) => {
                                self.cycle_tab(false);
                                true
                            }
                            (KEY_RIGHTBRACE, true) => {
                                self.cycle_tab(true);
                                true
                            }
                            (KEY_D, false) if mods & CHORD_ALT != 0 => {
                                self.duplicate_tab();
                                true
                            }
                            (KEY_1..=KEY_9, false) if mods & CHORD_ALT == 0 => {
                                self.select_numbered((key - KEY_1 + 1) as usize);
                                true
                            }
                            _ => false,
                        };
                        if used {
                            if before != (self.strip_shown(), self.find.open) {
                                self.place_web(ctx);
                            }
                            return true;
                        }
                    }
                    let handled = cmd && match key {
                        KEY_COMMA => {
                            self.editing = false;
                            self.sheet.show(&self.prefs);
                            true
                        }
                        KEY_L => {
                            self.begin_editing();
                            true
                        }
                        KEY_T => {
                            self.activate(NEW_TAB);
                            true
                        }
                        KEY_W => {
                            self.close_tab(self.current);
                            true
                        }
                        KEY_R => {
                            self.activate(RELOAD);
                            true
                        }
                        KEY_LEFTBRACE => {
                            self.activate(BACK);
                            true
                        }
                        KEY_RIGHTBRACE => {
                            self.activate(FORWARD);
                            true
                        }
                        _ => false,
                    };
                    if before != (self.strip_shown(), self.find.open) {
                        self.place_web(ctx);
                    }
                    if handled {
                        return true;
                    }
                }
                if self.find.open && !self.editing {
                    let was = self.find.open;
                    let used = self.find_key(&k);
                    if was != self.find.open {
                        self.place_web(ctx);
                    }
                    if used {
                        return true;
                    }
                }
                if !self.editing && matches!(k, KeyInput::Code(27)) {
                    if let (Some(v), Some(p)) = (self.view(), self.pool.as_mut()) {
                        if self.reader.exit(p, v) {
                            return true;
                        }
                    }
                }
                if self.downloads.open && matches!(k, KeyInput::Code(27)) {
                    self.downloads.open = false;
                    return true;
                }
                if self.overview.open && matches!(k, KeyInput::Code(27)) {
                    self.overview.open = false;
                    return true;
                }
                if self.editing {
                    match k {
                        KeyInput::Text(s) => {
                            self.address.push_str(&s);
                            self.refresh_suggestions();
                        }
                        KeyInput::Code(8) => {
                            self.address.pop();
                            self.refresh_suggestions();
                        }
                        KeyInput::Code(46) => {
                            self.address.delete();
                            self.refresh_suggestions();
                        }
                        KeyInput::Code(40) => {
                            let n = self.sugg.len();
                            self.sel = if n == 0 { None } else { Some(self.sel.map_or(0, |i| (i + 1) % n)) };
                        }
                        KeyInput::Code(38) => {
                            let n = self.sugg.len();
                            self.sel = if n == 0 { None } else { Some(self.sel.map_or(n - 1, |i| (i + n - 1) % n)) };
                        }
                        KeyInput::Code(13) => self.commit_address(),
                        KeyInput::Code(1) | KeyInput::Code(27) => {
                            self.editing = false;
                            self.sugg.clear();
                            self.sync_address();
                        }
                        KeyInput::Code(37) => self.address.move_caret(false),
                        KeyInput::Code(39) => self.address.move_caret(true),
                        _ => return false,
                    }
                    return true;
                }
                if let Some(p) = &mut self.pool {
                    p.key(&k);
                }
                false
            }
            _ => false,
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        self.ctl_methods()
    }

    fn rpc(&mut self, _ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        self.ctl_rpc(method, params)
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let ui = &ctx.ui;
        let mut nodes: Vec<Node> = Vec::new();
        if let Some(l) = &self.bar_layout {
            let bar = self.toolbar();
            for (id, r) in &l.visible {
                if let Some(item) = bar.items.iter().find(|i| i.id == *id && *id != ADDRESS) {
                    nodes.push(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled });
                }
            }
        }
        nodes.push(Node { id: ADDRESS as u64, role: Role::TextField, label: "Address".into(), value: self.address.to_string(), bounds: self.address_rect(ui, ctx.cfg.w), enabled: true });
        if self.strip_shown() {
            for (i, r) in self.tab_rects(ui, ctx.cfg.w).into_iter().enumerate() {
                let title = self.pool.as_ref().and_then(|p| p.state(self.tabs.get(i)?.view)).map(|s| s.title.clone()).unwrap_or_default();
                nodes.push(Node { id: TAB0 + i as u64, role: Role::Button, label: if title.is_empty() { "Untitled".into() } else { title }, value: if i == self.current { "selected".into() } else { String::new() }, bounds: r, enabled: true });
            }
        }
        nodes
    }

    fn full_redraw(&self) -> bool {
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let ui = &ctx.ui;
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        let wpt = w as f32 / Self::pt(ui);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        if let Some(p) = &mut self.pool {
            p.draw(s);
        }
        self.inspector.draw(ctx, s, self.view());
        if let (Some(r), Some(text)) = (&self.dev.responsive, self.dev.readout()) {
            let _ = r;
            let img = ui.text(&text, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, (w as i32 - img.w as i32) / 2, ui.pt(self.content_top() + 2.0) as i32, 255, 255);
        }
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if devmenu::not_secure(&st.url) {
                let img = ui.text("Not Secure", MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
                s.blit(&img, w as i32 - img.w as i32 - ui.pt(SPACE_S) as i32, ui.pt(UNIFIED_BAR_PT + 2.0) as i32, 255, 255);
            }
        }
        if let Some(msg) = &self.error {
            let img = ui.text(&ui.fit(msg, MAC_BODY_PT, w as f32 * 0.8), MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, (w as i32 - img.w as i32) / 2, (h as i32) / 2, 255, 255);
        }

        // the unified toolbar band
        widgets::toolbar_band(s, ui, Rect { x: 0.0, y: 0.0, w: wpt, h: UNIFIED_BAR_PT });
        if self.private {
            // a private window has a visibly dark toolbar (Browser)
            s.fill_rrect(0.0, 0.0, s.w as f32, ui.pt(UNIFIED_BAR_PT), 0.0, tokens::scrim(140));
        }
        let pressed = match self.press {
            Some((Press::Bar(id), _, _)) => Some(id),
            _ => None,
        };
        let bar = self.toolbar();
        self.bar_layout = Some(widgets::toolbar(s, ui, Self::bar_bounds(ui, w), &bar, pressed));
        let field = self.address_rect(ui, w);
        let placeholder = if self.private { "Private Browsing" } else { "Search or enter website name" };
        if self.editing || self.address.is_empty() {
            widgets::search_field(s, ui, &self.address, field, placeholder, self.editing);
        } else {
            // not editing: only the host, centred (Browser); the full URL returns on focus
            widgets::search_field(s, ui, &TextField::new(), field, "", false);
            let url = self.address.to_string();
            let host = host_of(&url);
            // site icon left of the host, the pair centred
            let icon_px = ui.pt(favicon::ICON_PT).round() as usize;
            let gap = ui.pt(favicon::GAP_PT) as i32;
            let has_icon = favicon::host_key(&url).is_some_and(|h| self.favicons.get(&h, icon_px).is_some());
            let lead = if has_icon { icon_px as i32 + gap } else { 0 };
            let room = (ui.pt(field.w - field.h * 2.0) - lead as f32).max(0.0);
            let shown = ui.fit(&host, MAC_BODY_PT, room);
            let img = ui.text(&shown, MAC_BODY_PT, Weight::Regular, LABEL.now());
            // measure with ui.measure: the rendered image carries side padding
            let tw = ui.measure(&shown, MAC_BODY_PT, Weight::Regular) as i32;
            let (cx, cy) = (ui.pt(field.x + field.w / 2.0) as i32, ui.pt(field.y + field.h / 2.0) as i32);
            let left = cx - (lead + tw) / 2;
            if let Some(ic) = favicon::host_key(&url).and_then(|h| self.favicons.get(&h, icon_px)) {
                s.blit(ic, left, cy - icon_px as i32 / 2, 255, 255);
            }
            s.blit(&img, left + lead + (tw - img.w as i32) / 2, cy - img.h as i32 / 2, 255, 255);
        }
        // Reader button at the leading edge when the page has an article (Browser)
        if let Some(v) = self.view().filter(|&v| !self.editing && self.reader.button(v)) {
            let r = reader::button_rect(field);
            glyphs::glyph(s, ui, "doc", Rect { x: r.x + r.w * 0.2, y: r.y + r.h * 0.2, w: r.w * 0.6, h: r.h * 0.6 }, if self.reader.is_active(v) { TINT } else { LABEL_2.now() });
        }
        // the star at the field's trailing edge: filled for a bookmarked page, an outline otherwise (Browser)
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if !self.editing {
                let marked = self.store.is_bookmarked(&st.url);
                let (name, color) = if marked { ("star.fill", TINT) } else { ("star", LABEL_2.now()) };
                glyphs::glyph(s, ui, name, star_rect(field), color);
            }
        }
        // load progress along the bottom of the address field (Browser)
        if let Some(st) = self.view().and_then(|v| self.pool.as_ref()?.state(v)) {
            if st.loading {
                let (x, y) = (ui.pt(field.x + field.h / 2.0), ui.pt(field.y + field.h) - ui.pt(SPACE_XXS));
                let len = (ui.pt(field.w - field.h) * st.progress.clamp(0.0, 1.0)) as usize;
                s.fill_rect(x as usize, y as usize, len, ui.pt(SPACE_XXS).max(1.0) as usize, TINT);
            }
        }

        self.draw_find(s, ui, wpt);
        self.draw_banner(s, ui, wpt);

        // tabs, only when there are two or more (Browser)
        if self.strip_shown() {
            let top = ui.pt(UNIFIED_BAR_PT) as usize;
            let sh = ui.pt(TAB_STRIP_PT).ceil() as usize;
            s.fill_rect(0, top, s.w, sh, CARD_BG.now());
            s.fill_rect(0, top + sh - 1, s.w, 1, SEPARATOR_C.now());
            for (i, r) in self.tab_rects(ui, w).into_iter().enumerate() {
                let selected = i == self.current;
                let pressed = matches!(self.press, Some((Press::Tab(j), _, _)) if j == i);
                if selected || pressed {
                    let fill = if selected { SEGMENT_ON.now() } else { ROW_PRESSED.now() };
                    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(m::SELECTION_RADIUS), fill);
                } else if i + 1 != self.current && i + 1 < self.tabs.len() {
                    s.fill_rect(ui.pt(r.x + r.w + 2.0) as usize, ui.pt(r.y + 5.0) as usize, 1, ui.pt(r.h - 10.0) as usize, SEPARATOR_C.now());
                }
                let hovered = self.hover.is_some_and(|(hx, hy)| r.contains(hx / ui.pt(1.0), hy / ui.pt(1.0)));
                let show_x = selected || hovered;
                if show_x {
                    glyphs::glyph(s, ui, "xmark", Self::close_rect(r), LABEL_2.now());
                }
                let st = self.pool.as_ref().and_then(|p| p.state(self.tabs.get(i)?.view));
                let title = st.map(|s| if s.title.is_empty() { s.url.clone() } else { s.title.clone() }).unwrap_or_default();
                let title = if title.is_empty() || title == START { "New Tab".to_string() } else { title };
                // site icon (16 pt) left of the title; the pair is centred (Browser)
                let icon_px = ui.pt(favicon::ICON_PT).round() as usize;
                let gap = ui.pt(favicon::GAP_PT) as i32;
                let host = st.and_then(|s| favicon::host_key(&s.url));
                let has_icon = host.as_deref().is_some_and(|h| self.favicons.get(h, icon_px).is_some());
                let lead = if has_icon { icon_px as i32 + gap } else { 0 };
                let room = ui.pt(r.w - if show_x { 44.0 } else { 16.0 }).max(0.0) - lead as f32;
                let weight = if selected { Weight::Semibold } else { Weight::Regular };
                let img = ui.text(&ui.fit(&title, MAC_SUBHEAD_PT, room.max(0.0)), MAC_SUBHEAD_PT, weight, if selected { LABEL.now() } else { LABEL_2.now() });
                let cx = ui.pt(r.x + r.w / 2.0) as i32;
                let cy = ui.pt(r.y + r.h / 2.0) as i32;
                // measure with ui.measure: the rendered image carries side padding
                let tw = ui.measure(&ui.fit(&title, MAC_SUBHEAD_PT, room.max(0.0)), MAC_SUBHEAD_PT, weight) as i32;
                let left = cx - (lead + tw) / 2;
                if let Some(ic) = host.as_deref().filter(|_| has_icon).and_then(|h| self.favicons.get(h, icon_px)) {
                    s.blit(ic, left, cy - icon_px as i32 / 2, 255, 255);
                }
                s.blit(&img, left + lead + (tw - img.w as i32) / 2, cy - img.h as i32 / 2, 255, 255);
            }
        }
        self.lib.draw_bar(s, ui, wpt, self.fav_top(), &self.store);
        if self.editing {
            suggest::draw(s, ui, field, &self.sugg, self.sel, &mut self.favicons);
        }
        if self.overview.open {
            let tabs: Vec<(String, String)> = self
                .tabs
                .iter()
                .map(|t| {
                    let st = self.pool.as_ref().and_then(|p| p.state(t.view));
                    (st.map(|s| s.title.clone()).unwrap_or_default(), st.map(|s| s.url.clone()).unwrap_or_default())
                })
                .collect();
            self.overview.draw(s, ui, wpt, self.content_top(), &tabs, self.current);
        }
        let anchor = self.bar_layout.as_ref().and_then(|l| l.rect_of(DOWNLOADS)).map(|r| r.x + r.w / 2.0);
        self.downloads.draw(s, ui, wpt, anchor);
        self.lib.draw_overlays(s, ui, w, h, self.fav_top(), &self.store, store::now());
        self.pageui.draw(s, ui, w, h);
        self.sheet.draw(s, ui, w, h, &self.prefs);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

/// The host of a URL without scheme, `www.` or path ("example.com"); non-URLs come back unchanged.
fn host_of(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else { return url.to_string() };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit('@').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// The throwaway profile of this private window.
fn private_profile() -> PathBuf {
    home().join(format!(".cache/bifrost-browser/private-{}", std::process::id()))
}

/// Private profiles of windows that are gone (crashed or killed) are deleted at the next private launch.
fn remove_stale_private_profiles() {
    let Ok(dir) = fs::read_dir(home().join(".cache/bifrost-browser")) else { return };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(pid) = name.strip_prefix("private-").and_then(|p| p.parse::<u32>().ok()) {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if self.private {
            self.pool = None;
            let _ = fs::remove_dir_all(private_profile());
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("BifrOSt Browser {} / Servo 0.7.0 (bifrost-webview)", env!("CARGO_PKG_VERSION"));
        return;
    }
    let private = std::env::args().any(|a| a == "--private");
    if private {
        remove_stale_private_profiles();
    }
    bifrost_app::run(Browser {
        pool: None,
        error: None,
        storm: Default::default(),
        watchdog: Default::default(),
        discard: Default::default(),
        clock: std::time::Instant::now(),
        crash_log: Vec::new(),
        crash: Default::default(),
        restore: Vec::new(),
        restore_sel: 0,
        tabs: Vec::new(),
        current: 0,
        address: TextField::new(),
        reader: reader::Reader::default(),
        hover: None,
        editing: false,
        press: None,
        bar_layout: None,
        initial_url: std::env::args().skip(1).find(|a| !a.starts_with('-')),
        dark: None,
        store: if private {
            let mut st = store::Store::in_memory();
            st.private = true;
            st
        } else {
            store::Store::open(store::data_dir())
        },
        sugg: Vec::new(),
        sel: None,
        session: Default::default(),
        find: Default::default(),
        inspector: Inspector::new(),
        dev: devmenu::DevMenu::default(),
        ctl: Default::default(),
        sources: Vec::new(),
        downloads: Default::default(),
        lib: Default::default(),
        favicons: {
            let mut f = favicon::Favicons::default();
            f.set_private(private);
            f
        },
        autofill: Default::default(),
        overview: Default::default(),
        pageui: pageui::PageUi::default(),
        prefs: prefs::Prefs::default(),
        sheet: prefs::Sheet::default(),
        private,
        long: None,
        swallow: false,
        drag: false,
        net: Default::default(),
        net_request: None,
        clip: None,
        pointer: Default::default(),
        dock: Default::default(),
    });
}

#[cfg(test)]
mod host_tests {
    use super::host_of;
    use super::{MENU_ACTUAL_SIZE, MENU_CLEAR_DOWNLOADS, MENU_DOWNLOADS, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREV, MENU_READER, MENU_SHARE, MENU_ZOOM_IN, MENU_ZOOM_OUT};
    use crate::tabdiscard::NAV_MENU_BASE;

    #[test]
    fn menu_ids_are_distinct_and_below_the_history_range() {
        let ids = [MENU_READER, MENU_SHARE, MENU_ZOOM_IN, MENU_ZOOM_OUT, MENU_ACTUAL_SIZE, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREV, MENU_DOWNLOADS, MENU_CLEAR_DOWNLOADS];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a > 0 && *a < NAV_MENU_BASE, "id {a} is outside 1..NAV_MENU_BASE");
            assert!(ids[i + 1..].iter().all(|b| b != a), "id {a} is used twice");
        }
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn address_host_drops_scheme_www_path_and_credentials() {
        assert_eq!(host_of("https://www.example.com/a?b#c"), "example.com");
        assert_eq!(host_of("http://user:pw@sub.example.com:8080/x"), "sub.example.com:8080");
        assert_eq!(host_of("https://example.com"), "example.com");
        assert_eq!(host_of("about:blank"), "about:blank");
        assert_eq!(host_of("hello world"), "hello world");
    }
}

