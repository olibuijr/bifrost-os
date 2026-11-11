//! Desktop Files is independently delivered through the App Store.
#![allow(unused_imports)]
use bifrost_app::kit::ui::{SPACE_S, SPACE_XXS};
use bifrost_app::kit::ui::{CELL_RADIUS_PT, MAC_CALLOUT_PT, MAC_HEADLINE_PT, MAC_TITLE3_PT};
/// Grouped cards and the View Options panel: `prefs_layout::m::CARD_RADIUS`, 13 pt.
const CARD_R: f32 = bifrost_app::controls::prefs_layout::m::CARD_RADIUS;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::controls::{Controls, Node, Role, Rect, TextField};
use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT, KEY_ESCAPE, KEY_ESCAPE_ASCII};
use bifrost_app::{Command, Commands, App, Ctx, Input, ContextItem, KeyInput, TouchKind};
use bifrost_app::app_windows::WindowId;
use bifrost_app::kit::pack;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::pathbar::Segment;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use springboard::files::{
    batch::{self, Answer, Op, Progress, Report, Resolution},
    find::{FileManager, TypeFilter},
    fs::{self, Entry, Kind, Sort},
    handoff, icons,
    tags::{sidecar_path, Tag, TagIndex},
    trash, archive,
};
use springboard::shell::clock::Wall;

use springboard::shell::ui::{
    Ui, Weight, CARD_BG, DETAIL_BG, LABEL, LABEL_2, LINK, SELECTION, SEPARATOR_C, WHITE, accent_now,
    CONTENT_BG,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use bifrost_app::kit::ui::{SPACE_L, SPACE_XL, SPACE_XS};

mod applications;
mod barmenu;
mod control;
mod dragout;
mod dragpreview;
mod coordinate;
mod go;
mod info;
mod keys;
mod menus;
mod openwith;
mod model;
mod ops;
mod pathbar;
mod perf;
mod perms;
mod peek;
mod provider;
mod search;
mod smart;
mod server;
mod sidebar;
mod statusbar;
mod tabs;
mod empty_trash;
mod extensions;
mod system;
mod toolbar;
mod undo;
mod views;
mod window;
mod windows;
mod info_window;
use barmenu::*;
use extensions::*;
use go::*;
use info::*;
use openwith::*;
use keys::*;
use menus::*;
use model::*;
use ops::*;
use pathbar::*;
use perf::*;
use perms::*;
use peek::*;
use search::*;
use sidebar::*;
use statusbar::*;
use system::*;
use toolbar::*;
use undo::*;
use tabs::TabHit;
use window::*;
use views::{*, column::*, icon::*, list::*, options::*};

struct Files {
    home: PathBuf,
    path: PathBuf,
    history: Vec<PathBuf>,
    at: usize,
    /// The rows on screen: the folder's listing, or the search hits.
    rows: Vec<Entry>,
    /// The open folder's entries, sorted.
    listing: Vec<Entry>,
    /// The search hits under the folder, when a search has returned any.
    hits: Option<Vec<Entry>>,
    search: Option<Search>,
    /// The scope bar under the toolbar while a query searches: This Computer or the open folder, and its plates.
    scope_bar: ScopeBar,
    /// Selected rows, in the order they were picked; the last one is the focus.
    selection: Vec<usize>,
    /// While on, taps add or remove rows from the selection (the Select button).
    select_mode: bool,
    /// View > Hide Sidebar (⌃⌘S or the context menu).
    sidebar_hidden: bool,
    /// View > Show Status Bar (⌘/ or the context menu); off by default, as in Files.
    status_bar: bool,
    /// Width of the toolbar title, in points, as measured at the last paint.
    title_w: f32,
    /// The footer path bar's segments at the last paint, for clicks.
    path_segs: Vec<Segment>,
    loading: bool,
    error: Option<String>,
    job: Option<Receiver<Listing>>,
    grid: bool,
    sort: Sort,
    query: TextField,
    controls: Controls,
    searching: bool,
    scroll: f32,
    /// How far the sidebar list is scrolled (points): a drag that starts in the sidebar moves it.
    side_scroll: f32,
    /// Where the press landed (x, y in points) and when. Never moves: a drag starts from it and a click ends on it.
    down: Option<(f32, f32, f64)>,
    /// The y the last pointer move left the finger at: the list scrolls by the difference to the next one.
    pan_y: f32,
    last_click: Option<(usize, f64)>,
    preview: Option<(String, String)>,
    w: f32,
    h: f32,
    painted_state: String,
    painted_selection: Vec<usize>,
    revision: u64,
    /// Show dot files (Cmd or Ctrl+Shift+. toggles).
    show_hidden: bool,
    /// The copied or cut items, and whether they were cut.
    clip: Option<(Vec<PathBuf>, bool)>,
    /// The item being renamed in place, with the name being typed.
    naming: Option<(PathBuf, TextField)>,
    /// The running copy, move, duplicate or trash, if any.
    work: Option<Work>,
    /// The last failed action, shown in the footer until the next navigation.
    notice: Option<String>,
    /// The Get Info panel for the focused item, when open.
    info: Option<Info>,
    /// Colour tags for every path, kept in the sidecar file at home.
    tag_index: TagIndex,
    /// The Tags panel, which toggles colours on the selection.
    tag_panel: bool,
    /// The open window's tabs; the open one's navigation is kept here too (see `tabs.rs`).
    tabs: tabs::TabStrip<tabs::TabState>,
    /// Saved searches shown in the sidebar's Smart Folders section, from `.smart-folders` at home.
    smart: Vec<smart::SmartFolder>,
    /// The Open With panel for the focused file: the apps the launch service names for it.
    open_with: Option<OpenWith>,
    /// A press on an item has moved far enough to drag the selection.
    dragging: bool,
    /// The columns view is on (with `grid` false).
    columns: bool,
    /// View settings: gallery, sort direction, Group By, sizes, column width, expanded folders.
    vo: ViewOpts,
    /// The folders above the open one, for the columns view.
    ancestors: Vec<(PathBuf, Vec<Entry>)>,
    /// WatchEvents stream on the data folder: it wakes the loop when the Show in Files request arrives.
    watch: Option<bifrost_app::fswatch::WatchStream>,
    /// Watch events kept so the open folder can catch up on the ones it missed while it was loading.
    journal: bifrost_app::fswatch::EventJournal,
    /// The journal id the open folder has applied up to.
    journal_seen: u64,
    /// The app's data folder, once configured; the folder stream watches it beside the open folder.
    data_dir: Option<PathBuf>,
    /// Rendered names, bounded and keyed by path and mtime.
    labels: Labels,
    /// rendered list-row text (names, dates, sizes, kinds), so a repaint does not rasterise every visible row again
    row_text: std::cell::RefCell<bifrost_app::kit::TextCache>,
    /// The folder or sidebar place a drag would drop onto.
    drop_target: Option<PathBuf>,
    /// Undoable file operations, newest last (Cmd+Z).
    undo: Vec<Undo>,
    /// Go to Folder, spring-loaded folders and the copy-drag flag.
    fx: FileManagerState,
    /// Drag and drop with other windows and apps (`dragout`).
    dnd: dragout::DragOut,
    /// Service calls (preview panel, pasteboard, search index, Dock progress) and the replies they wait for.
    sys: SystemState,
    /// The windows opened with New Window (⌘N), in the order they were opened. The main window is `self`.
    windows: Vec<windows::ExtraWindow>,
    /// This `Files` is a Get Info window (`info_window.rs`): it shows only its `info`, filling the window.
    info_window: bool,
    /// Get Info asked for its own window; `open_info_window` opens it once a `Ctx` is at hand.
    info_wanted: bool,
    /// File-manager sync extensions: apps that badge items and add menu items for the folders they declare. A cell,
    /// because drawing asks for the badges.
    ext: std::cell::RefCell<Extensions>,
}
fn home() -> PathBuf {
    let account = std::fs::read_to_string("/var/lib/springboard/account").unwrap_or_default();
    let user = account
        .lines()
        .find_map(|l| l.strip_prefix("username="))
        .unwrap_or("");
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let f: Vec<_> = l.split(':').collect();
            (f.len() > 5 && f[0] == user).then(|| PathBuf::from(f[5]))
        })
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or("/".into())))
}
impl Files {
    pub(crate) fn new() -> Self {
        let home = home();
        for folder in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Videos",
        ] {
            let _ = std::fs::create_dir_all(home.join(folder));
        }
        let tag_index = TagIndex::load(&sidecar_path(&home));
        let smart = smart::load(&home);
        let ext = std::cell::RefCell::new(Extensions::discover(&springboard::shell::apphost::root(), &home, &provider::shared_host(), "is.olibuijr.files.desktop"));
        let mut f = Self {
            path: home.clone(),
            tabs: tabs::TabStrip::new(tabs::TabState { path: home.clone(), history: vec![], at: 0 }),
            home,
            history: vec![],
            at: 0,
            rows: vec![],
            listing: vec![],
            hits: None,
            search: None,
            scope_bar: ScopeBar::default(),
            selection: vec![],
            select_mode: false,
            sidebar_hidden: false,
            status_bar: false,
            title_w: 120.,
            path_segs: Vec::new(),
            loading: false,
            error: None,
            job: None,
            grid: true,
            sort: Sort::Name,
            query: TextField::new(),
            controls: Controls::default(),
            searching: false,
            scroll: 0.,
            side_scroll: 0.,
            down: None,
            pan_y: 0.,
            last_click: None,
            preview: None,
            w: 960.,
            h: 630.,
            painted_state: String::new(),
            painted_selection: vec![],
            revision: 0,
            show_hidden: false,
            clip: None,
            naming: None,
            work: None,
            notice: None,
            info: None,
            tag_index,
            smart,
            tag_panel: false,
            open_with: None,
            dragging: false,
            drop_target: None,
            undo: vec![],
            columns: false,
            ancestors: vec![],
            watch: None,
            journal: bifrost_app::fswatch::EventJournal::new(JOURNAL_CAP),
            journal_seen: 0,
            fx: FileManagerState::default(),
            dnd: Default::default(),
            vo: ViewOpts::default(),
            data_dir: None,
            labels: new_labels(),
            row_text: Default::default(),
            sys: SystemState::default(),
            windows: Vec::new(),
            info_window: false,
            info_wanted: false,
            ext,
        };
        // a folder given on the command line opens there (Show in Files)
        // (a folder that is gone opens the nearest one above it, in `navigate`)
        let start = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| f.home.clone());
        f.navigate(start, true);
        f
    }
}
impl Files {
    /// The items Files adds to the File menu (the shell merges them into its own).
    pub(crate) fn file_menu_items(&self) -> Vec<bifrost_app::AppMenuItem> {
        // File > Preview (the same panel as the context menu's Preview): the shell merges an app menu titled File
        // into its own File menu, and the item needs a selection
        // File > Open (Cmd+O) and Preview (Space), with their key equivalents as the reference's File menu shows them
        let open = bifrost_app::AppMenuItem::new(MENU_OPEN, "Open").key("cmd+o").enabled(self.selection.len() == 1 && self.naming.is_none());
        let quick_look = bifrost_app::AppMenuItem::new(MENU_QUICK_LOOK, "Preview").key("space").enabled(!self.selected_paths().is_empty());
        // File > New Folder (Shift+Cmd+N): the toolbar has no New Folder button; the folder menus have it too
        let new_folder = bifrost_app::AppMenuItem::new(MENU_NEW_FOLDER, "New Folder").key("cmd+shift+n").enabled(!self.in_recents() && !self.in_bin());
        // the item actions of the context menu, with the same keys (the reference file manager's File menu lists them too); not while a name
        // is being typed, where the chords are the field's
        let (has, single, typing) = (!self.selection.is_empty(), self.selection.len() == 1, self.naming.is_some());
        let (in_bin, in_recents) = (self.in_bin(), self.in_recents());
        let files_ok = has && !typing && !in_bin && !in_recents;
        let trash = if in_bin {
            bifrost_app::AppMenuItem::new(MENU_PUT_BACK, "Put Back").enabled(has && !typing)
        } else {
            bifrost_app::AppMenuItem::new(MENU_TRASH, "Move to Trash").key("cmd+backspace").enabled(has && !typing)
        };
        // File > New Window (⌘N) and New Tab (⌘T) come first, as in the reference file manager's File menu: both had
        // their keys and no menu item
        let mut items = vec![
            bifrost_app::AppMenuItem::new(MENU_NEW_WINDOW, "New Window").key("cmd+n").enabled(!typing && !self.searching),
            bifrost_app::AppMenuItem::new(MENU_NEW_TAB, "New Tab").key("cmd+t").enabled(!typing),
            new_folder,
            bifrost_app::AppMenuItem::separator(),
            open,
            quick_look,
            bifrost_app::AppMenuItem::new(MENU_INFO, "Get Info").key("cmd+i").enabled(single && !typing),
            bifrost_app::AppMenuItem::new(MENU_RENAME, "Rename").enabled(single && !typing && !in_bin),
            bifrost_app::AppMenuItem::new(MENU_DUPLICATE, "Duplicate").key("cmd+d").enabled(files_ok),
            bifrost_app::AppMenuItem::new(MENU_ALIAS, "Make Alias").key("ctrl+cmd+a").enabled(files_ok),
            bifrost_app::AppMenuItem::separator(),
            trash,
        ];
        // a Get Info window shows no folder: none of the items has anything to act on there
        if self.info_window {
            for item in &mut items {
                item.enabled = false;
            }
        }
        items
    }
    /// The Edit commands this window takes now: the name field's or the search field's while one is typed in,
    /// otherwise the file commands for its selection and its folder. A Get Info window takes none.
    pub(crate) fn edit_commands(&self) -> Commands {
        if self.info_window {
            return Commands::default();
        }
        let clipboard = bifrost_app::kit::clipboard::text().is_some_and(|s| !s.is_empty());
        if let Some((_, field)) = &self.naming {
            return field.commands(clipboard);
        }
        if self.searching {
            return self.query.commands(clipboard);
        }
        Commands::default()
            .with(Command::Undo, !self.undo.is_empty())
            .with(Command::Copy, !self.selection.is_empty())
            .with(Command::Cut, !self.selection.is_empty())
            .with(Command::Paste, !self.in_recents())
            .with(Command::SelectAll, true)
    }
    /// The Edit menu's Undo and Redo titles for this window.
    pub(crate) fn edit_undo_titles(&self) -> (String, String) {
        if self.naming.is_some() || self.searching || self.info_window { (String::new(), String::new()) } else { self.file_undo_titles() }
    }
    /// One input for this window: its handler, then the window Get Info asked for and the service calls queued.
    pub(crate) fn window_input_event(&mut self, ctx: &Ctx, input: Input) -> bool {
        let handled = self.route_input(ctx, input);
        self.open_info_window(ctx);
        self.flush_services(ctx);
        handled
    }
    /// A File menu item chosen while this window is the key window.
    pub(crate) fn file_menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if self.info_window {
            return false;
        }
        match id {
            // the window is opened by the window that has the keys; an extra window's new window joins the main
            // window's list (`adopt_opened`)
            MENU_NEW_WINDOW => return self.new_window(ctx),
            MENU_NEW_TAB if self.naming.is_none() => self.new_tab(),
            MENU_OPEN => if let Some(i) = self.focus() { self.open(i) },
            MENU_QUICK_LOOK => self.quick_look(),
            MENU_NEW_FOLDER if !self.in_recents() && !self.in_bin() => self.new_folder(),
            MENU_INFO | MENU_RENAME | MENU_DUPLICATE | MENU_ALIAS | MENU_TRASH | MENU_PUT_BACK => {
                let changed = self.context_action(id);
                // Get Info from the menu bar (cmd+i) opens its own window here; only `event` did, so the panel
                // stayed in the browser window until the next click
                self.open_info_window(ctx);
                self.flush_services(ctx);
                return changed;
            }
            _ => return false,
        }
        self.update_controls();
        self.revision += 1;
        self.flush_services(ctx);
        true
    }
}
impl App for Files {
    fn id(&self) -> &str {
        "is.olibuijr.files.desktop"
    }
    fn main_menu(&self, _ctx: &Ctx) -> Vec<bifrost_app::AppMenu> {
        vec![bifrost_app::AppMenu { title: "File".into(), items: self.file_menu_items() }]
    }
    /// The File menu follows the key window: its items are enabled for that window's selection and folder.
    fn menus_for(&self, _ctx: &Ctx, window: WindowId) -> Vec<bifrost_app::AppMenu> {
        vec![bifrost_app::AppMenu { title: "File".into(), items: self.window_files(window).file_menu_items() }]
    }
    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.file_menu_action(ctx, id)
    }
    /// The key window's input. A menu item chosen while a New Window window is key acts on that window: the SDK's
    /// default drops it for an extra window. Everything else takes the default way (`window_event`).
    fn window_input(&mut self, ctx: &Ctx, window: WindowId, input: bifrost_app::proto::WindowInput) -> bool {
        match input {
            bifrost_app::proto::WindowInput::MenuAction(id) if !window.is_main() => {
                let Some(at) = self.windows.iter().position(|e| e.id == window) else { return false };
                let changed = self.windows[at].files.file_menu_action(ctx, id);
                self.adopt_opened(at);
                changed
            }
            other => bifrost_app::app_windows::default_window_input(self, ctx, window, other),
        }
    }
    fn configure(&mut self, ctx: &Ctx) {
        self.painted_state.clear();
        // the appearance or the date conventions changed: the other windows are painted again too (only the main
        // window was, so a New Window window kept the old scheme until something changed in it)
        for e in &mut self.windows {
            e.files.painted_state.clear();
        }
        // files from any app can be dropped on the folder listing (`dragout`)
        bifrost_app::dnd::register_types(ctx, &["public.data"]);
        if self.data_dir.is_none() {
            let _ = std::fs::create_dir_all(&ctx.data_dir);
            self.data_dir = Some(ctx.data_dir.clone());
            self.rewatch();
        }
        self.w = ctx.cfg.w as f32 / ctx.ui.s;
        self.h = ctx.cfg.h as f32 / ctx.ui.s;
        self.update_controls();
    }
    fn rpc_methods(&self) -> Vec<&'static str> {
        self.ctl_methods()
    }
    fn rpc(&mut self, _ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &bifrost_core::json::Json) -> Result<bifrost_core::json::Json, bifrost_app::control::RpcError> {
        let reply = self.ctl_rpc(method, params);
        // what a control call copied or cut is what a paste in any window takes
        self.share_clips();
        reply
    }
    fn window_controls(&self, _: &Ctx, window: WindowId) -> Vec<Node> {
        self.window_nodes(window)
    }
    fn controls(&self, _: &Ctx)->Vec<Node> {let mut nodes=self.controls.nodes().to_vec();for n in &mut nodes {if n.id==4{n.value=self.query.to_string();}}nodes}
    /// The Edit menu follows the key window: Copy and Cut need a selection in the window that has the keys. The
    /// shell sends a command only when the app reported it as taken, so with the main window's state alone a
    /// selection in a New Window window could not be copied at all.
    fn commands(&self, ctx: &Ctx) -> Commands {
        self.window_files(ctx.key_window()).edit_commands()
    }
    fn undo_titles(&self, ctx: &Ctx) -> (String, String) {
        self.window_files(ctx.key_window()).edit_undo_titles()
    }
    fn event(&mut self, ctx: &Ctx, input: Input) -> bool {
        let handled = self.window_input_event(ctx, input);
        // what this window copied or cut is what a paste in any window takes
        self.share_clips();
        handled
    }
    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        // the reply is for the window that made the call: a New Window window's paste, eject or preview is its own
        self.service_answer(request, result) || self.windows.iter_mut().any(|e| e.files.service_answer(request, result))
    }
    fn service_event(&mut self, _ctx: &Ctx, service: &str, event: &str, body: &bifrost_core::json::Json) -> bool {
        if service == bifrost_core::disk_wire::SERVICE {
            // (`image_event` shows an opened image's volume in the window that asked for it)
            return self.image_event(event, body);
        }
        if !bifrost_app::quick_look::is_quick_look(service) {
            return false;
        }
        let Some(e) = bifrost_app::quick_look::event(event, body) else { return false };
        // the panel shows the items of the window that opened it
        if self.sys.preview.is_none() {
            if let Some(w) = self.windows.iter_mut().find(|w| w.files.sys.preview.is_some()) {
                return w.files.preview_event(e);
            }
        }
        self.preview_event(e)
    }
    fn drag(&mut self, ctx: &Ctx, ev: bifrost_app::dnd::Event) -> bool {
        // the end of a session, and the events of a shell that does not name the window under the pointer
        self.drag_event(ctx, None, ev)
    }
    fn window_drag(&mut self, ctx: &Ctx, window: Option<WindowId>, ev: bifrost_app::dnd::Event) -> bool {
        // the window under the pointer resolves the drop: a drag from one window released over another is the
        // other window's drop, at a point in its own coordinates
        self.drag_event(ctx, window, ev)
    }
    fn document(&mut self, ctx: &Ctx, ev: bifrost_app::DocumentEvent) -> bool {
        // a grant for an Open goes to the workspace; the rest belong to drag and drop
        match system::open_answer(ctx, ev) {
            Ok(changed) => changed,
            // the grant a drag asked for goes to the window whose drag it is: answered to the main window, a New
            // Window window's grants were handed back and its drag never became a session
            Err(ev) => match self.document_window(&ev) {
                Some(i) => self.windows[i].files.drag_out_document(ctx, ev),
                None => self.drag_out_document(ctx, ev),
            },
        }
    }
    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        // a New Window window takes everything the shell sends its way: its touches, and while it is the key window
        // the keys and the Edit commands. (They were dropped here, so ⌘C, ⌘V and every other key did nothing in
        // a second window.)
        let Some(at) = self.windows.iter().position(|e| e.id == window) else { return false };
        // ⌘W closes a Get Info window, and a browser window that has no other tab to close
        if let Input::Key(key) = &ev {
            let files = &self.windows[at].files;
            if bifrost_app::app_windows::is_close_window_key(key) && windows::closes_window(files.info_window, files.tabs.len(), files.naming.is_some()) {
                let _ = ctx.close_window(window);
                return self.close_extra(window);
            }
        }
        let files = &mut self.windows[at].files;
        let touch = matches!(ev, Input::Touch { .. });
        // a Get Info window has its permission grid to click and nothing to type into
        if files.info_window && !touch {
            return false;
        }
        // an extra window's touches and its secondary click arrive in window points (`WindowTouch`, `Context` with a
        // window); the handlers take px, as the main window's. The menu a secondary click or a toolbar pull-down opens
        // is this window's, and its choice (`ContextAction`) comes back here, for this window's selection and folder.
        let ev = windows::window_input_in_px(ev, ctx.ui.pt(1.0));
        let changed = files.window_input_event(ctx, ev);
        self.share_clips();
        // a window that window opened (Get Info, New Window) is drawn and closed from here, with the others
        self.adopt_opened(at);
        changed
    }
    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        match self.windows.iter_mut().find(|e| e.id == window) {
            Some(e) => e.draw(ctx, s, t),
            None => Damage::new(s.w, s.h),
        }
    }
    fn window_closed(&mut self, _ctx: &Ctx, window: WindowId) -> bool {
        self.close_extra(window)
    }
    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        // the one watch stream: the data folder (Show in Files), every window's open folder and the media folders
        self.sync_watch();
        let now = Instant::now();
        let events = self.poll_watch(now);
        self.spread_events(events);
        // a volume mounted or unmounted while the app runs: every sidebar follows the mount table
        let mut changed = self.follow_mounts(now, false);
        // this window, then the New Window ones: each has its own listing, search, job and folder events
        let cut = self.clip.is_some();
        changed |= self.tick_view(ctx, t);
        let mut opened = Vec::new();
        for e in &mut self.windows {
            changed |= e.files.tick_view(ctx, t);
            // a window that window opened (Get Info) is drawn and closed from here, with the others
            opened.append(&mut e.files.windows);
        }
        self.windows.extend(opened);
        // a cut that was pasted (its move is done) is over in every window, not only in the one that pasted it
        if cut && (self.clip.is_none() || self.windows.iter().any(|e| !e.files.info_window && e.files.clip.is_none())) {
            self.clip = None;
            for e in &mut self.windows {
                e.files.clip = None;
            }
        }
        // a file the shell would not open (no app declares its type, or the grant was refused)
        if let Some(name) = system::take_open_failure() {
            self.notice = Some(format!("The document \u{201C}{name}\u{201D} could not be opened."));
            self.revision += 1;
            changed = true;
        }
        // a provider download to open
        for work in system::take_from_workers() {
            match work {
                system::FromWorker::Open(path) => self.sys.effects.push(Effect::OpenFiles(vec![path], None)),
            }
            changed = true;
        }
        // Show in Files: the shell or the helper leaves a request (a folder, perhaps an item to select) in our data folder
        if let Some(request) = handoff::take_request(&ctx.data_dir) {
            self.show_request(request);
            changed = true;
        }
        // a provider thread left a footer message (a download failed, or the file is on its way to its app)
        if self.take_provider_notice() {
            changed = true;
        }
        self.ctl_tick();
        // a copy or a cut made on a way that is not a window's own input (a menu item, a control call, a service
        // reply) reaches the other windows here at the latest
        self.share_clips();
        // a badge an extension answered with is drawn on this frame
        if self.poll_extensions() {
            changed = true;
        }
        self.flush_services(ctx);
        if changed {
            self.revision += 1;
        }
        changed
    }
    /// No timer: jobs and searches wake the loop through their `Feed`, and Show in Files through the data-folder watch.
    fn next_wake(&self, t: f64) -> Option<f64> {
        // a drag hovering over a folder opens it; a batch of folder events matures; otherwise the loop sleeps
        let events = self.watch_deadline(Instant::now());
        let hovering = self.fx.spring.is_some() || self.windows.iter().any(|e| e.files.fx.spring.is_some());
        let wake = wake_at(t, hovering, events);
        // a disk or a media folder just changed: its mount follows, and the mount table is read until it has
        let mounts = self.fx.mounts_until.map(|_| t + MOUNT_POLL_MS);
        let wake = match (wake, mounts) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // a finger resting on an item lifts it when its hold time is up
        let hold = self.hold_due().into_iter().chain(self.windows.iter().filter_map(|e| e.files.hold_due())).reduce(f64::min);
        let wake = match (wake, hold) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // a thumbnail being drawn is taken by the next tick
        if self.vo.thumbs_pending() || self.windows.iter().any(|e| e.files.vo.thumbs_pending()) {
            let soon = t + THUMB_POLL_MS;
            return Some(wake.map_or(soon, |w| w.min(soon)));
        }
        wake
    }
    fn wake_fd(&self) -> Option<i32> {
        self.watch.as_ref().and_then(|w| w.fd())
    }
    /// A background Files window stays on screen: it follows the open folder (files added, removed or renamed by
    /// another app or a shell) instead of showing the listing it had when it lost focus.
    fn ticks_in_background(&self) -> bool {
        true
    }
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _: f64) -> Damage {
        self.render(&ctx.ui,s)
    }
}
/// How often a drag hovering over a folder is checked for its spring-loaded open (ms).
const SPRING_POLL_MS: f64 = 100.0;
/// How often the mount table is read for a few seconds after a disk or a media folder changed (ms).
const MOUNT_POLL_MS: f64 = 1000.0;
/// How soon the loop wakes to take a thumbnail the workers are drawing.
const THUMB_POLL_MS: f64 = 50.0;

/// When the loop wakes, in ms like `t`: soon while a drag hovers (spring-loading), when the next batch of folder events
/// matures (`events` from now), else never.
fn wake_at(t: f64, spring: bool, events: Option<Duration>) -> Option<f64> {
    let spring = spring.then_some(t + SPRING_POLL_MS);
    let events = events.map(|d| t + d.as_secs_f64() * 1000.0);
    match (spring, events) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
impl Files {
    /// One window's share of `tick`: what its own workers and its folder's events brought. The main window runs it
    /// for itself and for every New Window window, so each window's listing lands, its search streams in, its job
    /// makes progress and its folder follows changes made elsewhere. True when the window needs a frame.
    pub(crate) fn tick_view(&mut self, ctx: &Ctx, t: f64) -> bool {
        // the pictures the thumbnail workers finished
        let mut changed = self.vo.poll_thumbs();
        // a finger that has rested on an item long enough lifts it
        changed |= self.hold_tick(ctx, t);
        changed |= self.poll_search();
        // the shell would not open an app from the Applications place (not installed any more, or not an app)
        if let Some(name) = self.sys.take_app_open_failure() {
            self.notice = Some(format!("\u{201C}{name}\u{201D} can\u{2019}t be opened."));
            changed = true;
        }
        if self.poll_server() {
            changed = true;
        }
        if self.spring_tick() {
            changed = true;
        }
        // the folder events the journal took since the last look
        if self.catch_up() {
            changed = true;
        }
        // a folder's batches land as the worker reads them; the final listing replaces them and ends the load
        while let Some(done) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            if done.partial {
                if done.path == self.path {
                    if let Ok(batch) = done.result {
                        self.append_batch(batch);
                    }
                }
                self.revision += 1;
                changed = true;
                continue;
            }
            let mut vanished = None;
            if done.path == self.path {
                self.loading = false;
                self.ancestors = done.ancestors;
                match done.result {
                    Ok(rows) => {
                        // the worker already ordered the rows (and renamed the Trash's). An item clicked while the
                        // first batches were on screen stays selected: the final listing used to clear it, so the first
                        // click on a freshly opened folder's icon selected nothing (W5-15)
                        let keep = self.selected_paths();
                        self.listing = rows;
                        self.refresh_rows();
                        self.reselect(&keep);
                        // a folder read again in place is where it was: its scroll and its selection
                        self.restore_place();
                        // a renamed item stays selected, and a new folder opens its name for editing
                        self.apply_pending();
                    }
                    // the folder was removed while it was listed: it opens the nearest folder that is still there
                    Err(_) if !self.is_virtual_folder(&done.path) && !done.path.is_dir() => vanished = Some(done.path.clone()),
                    Err(e) => {
                        self.listing.clear();
                        self.refresh_rows();
                        self.error = Some(e);
                        self.fx.pending = None;
                        self.fx.relist = None;
                    }
                }
            }
            self.revision += 1;
            self.job = None;
            if let Some(gone) = vanished {
                self.navigate(gone, false);
            }
            self.update_controls();
            self.catch_up();
            changed = true;
            break;
        }
        let mut finished = None;
        if let Some(work) = self.work.as_mut() {
            while let Ok(event) = work.rx.try_recv() {
                match event {
                    Event::Progress(p) => {
                        work.progress = p;
                        changed = true;
                    }
                    Event::Conflict(name) => {
                        work.conflict = Some(name);
                        changed = true;
                    }
                    Event::Done(report) => {
                        finished = Some(report);
                        break;
                    }
                }
            }
        }
        self.sys.track_dock(self.work.as_ref());
        if let Some(report) = finished {
            self.sys.end_dock();
            if let Some(work) = self.work.take() {
                self.ctl_job_done(&report);
                self.finish_job(work, report);
            }
            changed = true;
        }
        // the service calls this window's handlers queued; the main window's `tick` sends its own at its end
        if self.fx.extra_window {
            self.flush_services(ctx);
            if changed {
                self.revision += 1;
            }
        }
        changed
    }
    /// Routes one input to its handler. `event` sends the service calls the handler queued.
    fn route_input(&mut self, ctx: &Ctx, input: Input) -> bool {
        if self.goto_input(&input) {
            self.update_controls();
            return true;
        }
        if self.server_input(&input) {
            self.update_controls();
            return true;
        }
        if self.empty_trash_input(ctx, &input) {
            self.update_controls();
            return true;
        }
        match input {
            Input::Context{x,y}=>self.context_menu_at(ctx,x,y),
            // New Window (⌘N) and the empty-area menu's New Window need the ctx to open the window
            Input::ContextAction(menus::MENU_NEW_WINDOW) => return self.new_window(ctx),
            Input::ContextAction(id) => {
                if !self.context_action(id) {
                    return false;
                }
            }
            Input::Activate(id) => {self.update_controls();if let Some(n)=self.controls.nodes().iter().find(|n|n.id==id&&n.enabled){let r=n.bounds;self.click(r.x+r.w/2.,r.y+r.h/2.,0.);}else{return false;}}
            Input::Command(command) if self.naming.is_some() => {
                let clipboard = bifrost_app::kit::clipboard::text().unwrap_or_default();
                if let Some((_, field)) = self.naming.as_mut() {
                    if let Some(text) = field.command(command, &clipboard) {
                        bifrost_app::kit::clipboard::set_text(&text);
                    }
                }
            }
            Input::Command(command) if self.searching => {
                let clipboard=bifrost_app::kit::clipboard::text().unwrap_or_default();
                if let Some(text)=self.query.command(command,&clipboard){bifrost_app::kit::clipboard::set_text(&text);}
                if matches!(command,Command::Cut|Command::Paste|Command::Undo|Command::Redo){self.query_changed();}
            }
            Input::Command(command) => match command {
                Command::Copy => self.copy_selected(false),
                Command::Cut => self.copy_selected(true),
                Command::Paste => self.paste(),
                Command::SelectAll => self.select_all(),
                // Undo and Redo act on the text field while naming or searching; otherwise Undo reverses the last
                // move, rename, trash or new folder
                Command::Undo if self.naming.is_none() && !self.searching => self.undo_file_op(),
                Command::Undo | Command::Redo => {}
            },
            Input::Touch { kind: TouchKind::Down, x, y, .. } if self.column_edge(x / ctx.ui.s, y / ctx.ui.s).is_some() => {
                self.vo.resize_k = self.column_edge(x / ctx.ui.s, y / ctx.ui.s).unwrap_or(0);
            }
            Input::Touch { kind: TouchKind::Move, x, .. } if self.vo.resize_k > 0 => self.resize_to(x / ctx.ui.s),
            Input::Touch { kind: TouchKind::Up, .. } if self.vo.resize_k > 0 => self.vo.resize_k = 0,
            Input::Touch {
                kind: TouchKind::Down,
                x,
                y,
                t,
                source,
                ..
            } => {
                self.notice = None;
                // a press while a drag is still held means the release of that drag never came (the shell owned the
                // pointer and ended the session elsewhere): the old drag is over, it must not drop on this press
                self.cancel_drag();
                self.down = Some((x / ctx.ui.s, y / ctx.ui.s, t));
                self.pan_y = y / ctx.ui.s;
                // a finger that rests on an item lifts it after the hold time (`hold_tick`, or its first move after it)
                let on_item = x / ctx.ui.s >= self.side() && self.item_at(x / ctx.ui.s, y / ctx.ui.s).is_some();
                self.fx.hold = (source == bifrost_app::TouchSource::Finger && on_item).then_some(t);
            }
            Input::Touch {
                kind: TouchKind::Move,
                x,
                y,
                t,
                mods,
                source,
                ..
            } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                if let Some((sx, sy, _)) = self.down {
                    let moved = (x - sx).abs() > DRAG_SLOP || (y - sy).abs() > DRAG_SLOP;
                    // a mouse press drags an item out as soon as it moves. A finger's drag, and the drag the shell emulates
                    // for a wheel notch, is a scroll even when it starts on an item: in a full icon view every press lands
                    // on one, and the lines below the window could never be reached. A finger lifts the item it has rested
                    // on for the hold time; one that moved before that scrolls until it is raised
                    let held = self.fx.hold.is_some_and(|t0| t - t0 >= dragout::hold_ms());
                    if moved && !held {
                        self.fx.hold = None;
                    }
                    if held || (source == bifrost_app::TouchSource::Pointer && moved) {
                        self.lift_item(ctx, sx, sy);
                    }
                }
                if self.dragging {
                    // Option held while dragging inside the window copies instead of moving
                    if mods.option() {
                        self.fx.copy_drag = true;
                    }
                    let target = self.drop_target_at(x, y);
                    if target != self.drop_target {
                        self.spring_hover(target.as_ref());
                        self.drop_target = target;
                        self.revision += 1;
                    }
                } else if self.down.is_some() && bifrost_app::pans_now() {
                    let (old, new) = (self.pan_y, y);
                    if self.down.is_some_and(|(sx, _, _)| sx < self.side()) {
                        // a drag that began in the sidebar scrolls the source list, not the listing
                        self.side_scroll = (self.side_scroll + old - new).clamp(0., self.side_scroll_max());
                    } else {
                        // every view's own lines: the columns view scrolls its open folder's column by its row pitch
                        self.scroll = (self.scroll + old - new).clamp(0., self.scroll_max());
                    }
                    // the press point stays where it was: a drag that has not begun yet is measured from it, and the
                    // release is a click only if it ends near it
                    self.pan_y = new;
                }
            }
            Input::Touch {
                kind: TouchKind::Up,
                x,
                y,
                t,
                ..
            } => {
                self.fx.hold = None;
                if self.dragging {
                    // drop the selection on the target under the finger, if any
                    self.dragging = false;
                    self.down = None;
                    self.revision += 1;
                    self.fx.spring = None;
                    // the target of the last move, else what is under the release: after a folder sprang open under
                    // a finger that then did not move, that is the folder's own background
                    let dest = self.drop_target.take().or_else(|| self.drop_target_at(x / ctx.ui.s, y / ctx.ui.s));
                    let paths = std::mem::take(&mut self.fx.drag_paths);
                    let copy = std::mem::take(&mut self.fx.copy_drag);
                    match dest {
                        Some(dest) => {
                            self.spring_keep();
                            // Option held during the drag copies, as in Files; so does a folder on another volume
                            self.start_drop(copy, false, paths, dest);
                        }
                        // released over nothing that takes the items: a folder the drag sprang open is left again
                        None => {
                            self.spring_return();
                        }
                    }
                } else if let Some((sx, sy, _)) = self.down.take() {
                    let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                    if (x - sx).abs() < 8. && (y - sy).abs() < 8. {
                        self.click(x, y, t)
                    }
                }
            }
            Input::Key(KeyInput::Text(s)) if self.work.as_ref().is_some_and(|w| w.conflict.is_some()) => {
                // k, r, s choose Keep Both, Replace, Skip; a capital applies it to the rest
                let choice = match s.as_str() {
                    "k" | "K" => Resolution::KeepBoth,
                    "r" | "R" => Resolution::Replace,
                    "s" | "S" => Resolution::Skip,
                    _ => return false,
                };
                if s.chars().all(|c| c.is_uppercase()) {
                    if let Some(work) = self.work.as_mut() {
                        work.apply_all = true;
                    }
                }
                self.answer_conflict(choice);
            }
            Input::Key(KeyInput::Text(s)) => {
                if let Some((_, field)) = self.naming.as_mut() {
                    field.push_str(&s);
                } else if self.searching {
                    self.query.push_str(&s);
                    self.query_changed();
                } else if s == " " {
                    self.quick_look();
                }
            }
            Input::Key(k) if k.as_chord().is_some() => {
                let (key, mods) = k.as_chord().unwrap_or((0, 0));
                if self.dragging && mods & CHORD_ALT != 0 {
                    self.fx.copy_drag = true; // Option during a drag: drop copies
                    self.revision += 1;
                }
                if key == 49 && mods & CHORD_META != 0 && mods & (CHORD_SHIFT | CHORD_ALT) == 0 && self.naming.is_none() && !self.searching {
                    return self.new_window(ctx); // ⌘N: File > New Window
                }
                if !self.chord(key, mods) {
                    return false;
                }
            }
            Input::Key(KeyInput::Code(code)) => {
                if self.naming.is_some() {
                    if !self.naming_key(code) {
                        return false;
                    }
                } else {
                    match code {
                        9 => {self.update_controls();let focus=self.controls.advance(false);self.searching=focus==Some(4);}
                        13 if self.controls.focused().is_some_and(|n|n.role==Role::Button)=>{let (id,bounds)={let n=self.controls.focused().unwrap();(n.id,n.bounds)};self.click(bounds.x+bounds.w/2.,bounds.y+bounds.h/2.,0.);self.update_controls();self.controls.focus(Some(id));}
                        37 | 39 if self.searching => self.query.move_caret(code==39),
                        36 if self.searching => self.query.home(),
                        35 if self.searching => self.query.end(),
                        46 if self.searching => {
                            self.query.delete();
                            self.query_changed();
                        }
                        // Escape gives a drag up: the shell ends the session it owns, and a folder the drag sprang
                        // open is left again
                        KEY_ESCAPE | KEY_ESCAPE_ASCII if self.dragging && self.dnd.foreign.is_none() => {
                            match self.dnd.session {
                                Some(session) => bifrost_app::dnd::cancel(ctx, session),
                                None => self.cancel_drag(),
                            }
                        }
                        KEY_ESCAPE | KEY_ESCAPE_ASCII => {
                            if self.work.is_some() {
                                self.cancel_job();
                            } else {
                                self.preview = None;
                                self.info = None;
                                self.tag_panel = false;
                                self.open_with = None;
                                self.revision += 1;
                                self.searching = false;
                                self.select_mode = false;
                                self.query.clear();
                                self.query_changed();
                            }
                        }
                        8 if self.searching => {
                            self.query.pop();
                            self.query_changed();
                        }
                        8 => self.go_back(),
                        // Return in the search field opens the hit; on an item it renames (Cmd+O and Cmd+Down open)
                        13 if self.searching => self.open_selected(),
                        13 => self.start_rename(),
                        32 => self.quick_look(),
                        37..=40 => self.arrow(code),
                        _ => {}
                    }
                }
            }
            _ => return false,
        }
        self.update_controls();
        true
    }
}
fn local_account() {
    unsafe extern "C" {
        fn geteuid() -> u32;
        fn setgroups(n: usize, groups: *const u32) -> i32;
        fn setgid(gid: u32) -> i32;
        fn setuid(uid: u32) -> i32;
    }
    if unsafe { geteuid() } != 0 {
        return;
    }
    let account = std::fs::read_to_string("/var/lib/springboard/account").expect("local account");
    let user = account
        .lines()
        .find_map(|l| l.strip_prefix("username="))
        .expect("local username");
    let passwd = std::fs::read_to_string("/etc/passwd").expect("accounts");
    let fields: Vec<_> = passwd
        .lines()
        .find(|l| l.split(':').next() == Some(user))
        .expect("local account exists")
        .split(':')
        .collect();
    let uid: u32 = fields[2].parse().expect("uid");
    let gid: u32 = fields[3].parse().expect("gid");
    assert!(uid != 0, "Files requires an unprivileged account");
    assert!(
        unsafe { setgroups(0, std::ptr::null()) } == 0
            && unsafe { setgid(gid) } == 0
            && unsafe { setuid(uid) } == 0,
        "switch Files to local account"
    );
    std::env::set_var("HOME", fields[5]);
}
fn main() {
    local_account();
    bifrost_app::run(Files::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_window_wakes_in_milliseconds_not_every_millisecond() {
        // `t` is in ms: a batch maturing in 250 ms wakes 250 ms on, and a hovering drag polls every 100 ms
        assert_eq!(wake_at(1000.0, false, Some(Duration::from_millis(250))), Some(1250.0));
        assert_eq!(wake_at(1000.0, true, None), Some(1100.0));
        assert_eq!(wake_at(1000.0, true, Some(Duration::from_millis(40))), Some(1040.0));
        assert_eq!(wake_at(1000.0, false, None), None);
    }
    pub(crate) fn fixture(rows: Vec<Entry>) -> Files {
        Files {
            scope_bar: ScopeBar::default(),
            dnd: Default::default(), windows: Vec::new(), info_window: false, info_wanted: false, home:"/fixture".into(), path:"/fixture".into(), history:vec![], at:0,
            tabs: tabs::TabStrip::new(tabs::TabState { path: "/fixture".into(), history: vec![], at: 0 }),
            listing: rows.clone(), rows, hits: None, search: None, selection:vec![], select_mode:false, sidebar_hidden:false, status_bar:false, title_w:120., path_segs:vec![], loading:false, error:None, job:None,grid:true,sort:Sort::Name,
            query:TextField::new(),controls:Controls::default(),searching:false,scroll:0.,side_scroll:0.,down:None,pan_y:0.,last_click:None,preview:None,
            w:960.,h:630.,painted_state:String::new(),painted_selection:vec![],revision:0,
            show_hidden:false,clip:None,naming:None,work:None,notice:None,info:None,tag_index:TagIndex::default(),tag_panel:false,smart:vec![],open_with:None,dragging:false,drop_target:None,undo:vec![],columns:false,ancestors:vec![],watch:None,journal:bifrost_app::fswatch::EventJournal::new(JOURNAL_CAP),journal_seen:0,fx:FileManagerState::default(),vo:ViewOpts::default(),data_dir:None,labels:new_labels(),row_text:Default::default(),sys:SystemState::default(),ext:Default::default(),
        }
    }
    #[test]
    fn a_selection_made_before_the_final_listing_survives_it() {
        let mut f = fixture(vec![folder("a"), folder("b")]);
        f.selection = vec![1];
        let keep = f.selected_paths();
        // the final listing has another item in front and the clicked one moved
        f.listing = vec![folder("0"), folder("a"), folder("b")];
        f.refresh_rows();
        f.reselect(&keep);
        assert_eq!(f.selected_paths(), vec![PathBuf::from("/fixture/b")]);
        f.listing = vec![folder("a")];
        f.refresh_rows();
        f.reselect(&keep);
        assert!(f.selection.is_empty(), "an item gone from the folder leaves the selection");
    }

    pub(crate) fn folder(name: &str) -> Entry {
        Entry {name:name.into(),path:format!("/fixture/{name}").into(),is_dir:true,size:0,mtime:0,kind:Kind::Folder,items:None}
    }
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    #[test]
    fn the_conflict_prompt_shades_the_whole_folder_view_and_leaves_the_sidebar() {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let ui = Ui::new(font, 1.0);
        let mut files = fixture(vec![]);
        let (w, h) = (960usize, 630usize);
        let paint = |files: &mut Files| {
            let mut px = vec![0u32; w * h];
            files.paint(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
            px
        };
        let before = paint(&mut files);
        // a paste met a taken name: the job waits for the prompt's answer
        let (_events, rx) = mpsc::channel::<Event>();
        let (answers, _asked) = mpsc::channel::<Answer>();
        files.work = Some(Work {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            title: "Copying".into(),
            kind: JobKind::Transfer(Op::Copy),
            dest: "/fixture".into(),
            sources: vec![],
            progress: Progress::default(),
            conflict: Some("d4.txt".into()),
            apply_all: false,
            answers,
        });
        let after = paint(&mut files);
        let (x, y, card_w, card_h) = prompt_frame(files.w, files.side());
        let at = |px: &[u32], x: f32, y: f32| px[y as usize * w + x as usize];
        let channels = |p: u32| [p & 255, (p >> 8) & 255, (p >> 16) & 255];
        // under the card, where the names of the first line of icons are, the view is shaded: it used to be untouched
        let below = (x + 40., y + card_h + 20.);
        assert!(below.1 < files.bottom(), "the point is in the folder view");
        let (was, is) = (at(&before, below.0, below.1), at(&after, below.0, below.1));
        assert_ne!(was, is, "the view under the prompt is shaded");
        assert!(channels(is).iter().zip(channels(was)).all(|(a, b)| *a <= b), "and darker, never lighter: {is:08x} from {was:08x}");
        assert_eq!(is >> 24, 255, "the window stays opaque");
        // far below, the same shade: one sheet over the whole view
        assert_eq!(at(&after, below.0, files.bottom() - 8.), is);
        // the card itself is the plain card colour, and the sidebar is not shaded
        assert_eq!(at(&after, x + card_w - 6., y + 6.), CARD_BG.now());
        assert_eq!(at(&after, 8., 400.), at(&before, 8., 400.));
    }
    /// A change of the clock style or the zone repaints a window whatever kind it is: the conventions are part of
    /// what a paint depends on (the main window alone hears `configure`).
    #[test]
    fn the_painted_state_follows_the_date_conventions() {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let ui = Ui::new(font, 1.0);
        let mut files = fixture(vec![folder("a")]);
        files.fx.extra_window = true;
        let (w, h) = (960, 630);
        let mut pixels = vec![0; w * h];
        assert!(files.render(&ui, &mut Surface { w, h, stride: w, px: &mut pixels, y0: 0 }).is_all());
        assert!(files.painted_state.ends_with(&format!(":{}", bifrost_app::time_zone::conventions_revision())));
        // painted under other conventions: everything is painted again
        let stale = format!(":{}", bifrost_app::time_zone::conventions_revision().wrapping_add(1));
        let at = files.painted_state.rfind(':').unwrap();
        files.painted_state.replace_range(at.., &stale);
        assert!(files.render(&ui, &mut Surface { w, h, stride: w, px: &mut pixels, y0: 0 }).is_all());
    }
    #[test]
    fn the_file_menu_starts_with_new_window_and_new_tab() {
        let mut files = fixture(vec![folder("a")]);
        let items = files.file_menu_items();
        let head: Vec<(&str, &str, bool)> = items.iter().take(3).map(|i| (i.title.as_str(), i.key.as_str(), i.enabled)).collect();
        assert_eq!(head, vec![("New Window", "cmd+n", true), ("New Tab", "cmd+t", true), ("New Folder", "cmd+shift+n", true)]);
        // Preview's key equivalent is the bare space bar, a name the shell's key parser knows
        let preview = items.iter().find(|i| i.id == MENU_QUICK_LOOK).unwrap();
        assert_eq!(preview.key, "space");
        assert_eq!(bifrost_app::controls::keys::KeyCombo::parse(&preview.key).map(|c| c.to_string()).as_deref(), Some("Space"));
        assert_eq!((items[0].id, items[1].id), (MENU_NEW_WINDOW, MENU_NEW_TAB));
        // while a name is typed the chords are the field's
        files.naming = Some((PathBuf::from("/fixture/a"), TextField::new()));
        let typing = files.file_menu_items();
        assert!(typing.iter().filter(|i| i.id == MENU_NEW_WINDOW || i.id == MENU_NEW_TAB).all(|i| !i.enabled));
        // a Get Info window offers neither
        files.naming = None;
        files.info_window = true;
        assert!(files.file_menu_items().iter().all(|i| !i.enabled));
    }
    #[test]
    fn partial_selection_matches_full_repaint_in_both_views() {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let ui = Ui::new(font,1.25);
        let mut files = fixture((0..12).map(|i| folder(&format!("Folder {i}"))).collect());
        let (w,h)=(1200,788);
        let mut pixels=vec![0;w*h];
        for grid in [true,false] {
            files.grid=grid;
            files.painted_state.clear();
            assert!(files.render(&ui,&mut Surface{w,h,stride:w,px:&mut pixels,y0:0}).is_all());
            for selection in [vec![0],vec![7],vec![1],vec![],vec![2,9],vec![9,2]] {
                files.selection=selection.clone();
                let damage=files.render(&ui,&mut Surface{w,h,stride:w,px:&mut pixels,y0:0});
                assert!(!damage.is_all());
                let mut expected=vec![0;w*h];
                files.paint(&ui,&mut Surface{w,h,stride:w,px:&mut expected,y0:0});
                let differences: Vec<_> = pixels.iter().zip(&expected).enumerate().filter(|(_, (a,b))|a!=b).take(12).map(|(i,(a,b))|(i%w,i/w,*a,*b)).collect();
                assert!(differences.is_empty(),"selection {selection:?}, grid {grid}: {differences:?}");
                assert!(files.render(&ui,&mut Surface{w,h,stride:w,px:&mut pixels,y0:0}).is_empty());
            }
        }
    }
    #[test]
    fn streamed_batches_fill_the_rows_before_the_listing_ends() {
        let mut files = fixture(vec![]);
        files.listing.clear();
        files.rows.clear();
        files.loading = true;
        files.append_batch(vec![folder("a"), folder("b")]);
        files.append_batch(vec![folder("c")]);
        assert_eq!(files.rows.len(), 3, "the first rows show before the folder is read");
        assert_eq!(files.listing.len(), 3);
        assert_eq!(files.vo.depth, vec![0, 0, 0]);
        // with a folder expanded the rows are rebuilt from the listing; the folder does not exist, so nothing splices in
        files.vo.expanded.insert(PathBuf::from("/fixture/a"));
        files.append_batch(vec![folder("d")]);
        assert_eq!(files.rows.len(), 4);
        assert_eq!(files.listing.len(), 4);
    }

    #[test]
    fn a_missing_folder_at_launch_opens_the_nearest_folder_that_is_there() {
        // the launch navigates to the folder it was given (or Home); a folder that is gone opens its parent and says so
        let home = temp("gone-launch");
        std::fs::create_dir_all(home.join("Projects")).unwrap();
        let mut files = fixture(vec![]);
        files.home = home.clone();
        files.navigate(home.join("Projects/drag-check"), true);
        assert_eq!(files.path, home.join("Projects"));
        assert_eq!(files.history, vec![home.join("Projects")], "the history names the folder that is shown");
        assert!(files.error.is_none(), "no OS error is shown");
        assert_eq!(files.notice.as_deref(), Some("The folder \u{201C}drag-check\u{201D} can\u{2019}t be found."));
        // a folder gone from the top of Home opens Home itself
        files.navigate(home.join("gone/deeper"), true);
        assert_eq!(files.path, home);
        assert_eq!(existing_folder(&home.join("gone/deeper"), &home), home);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_removed_while_it_is_shown_opens_the_folder_above_it() {
        let ctx = test_ctx();
        let home = temp("gone-shown");
        let child = home.join("drag-check");
        std::fs::create_dir_all(&child).unwrap();
        let mut files = fixture(vec![]);
        files.home = home.clone();
        files.navigate(child.clone(), true);
        // the folder goes away while its listing is still in flight, so that listing fails
        std::fs::remove_dir_all(&child).unwrap();
        let (tx, rx) = feed();
        tx.send(Listing { path: child.clone(), result: Err("No such file or directory (os error 2)".into()), ancestors: vec![], partial: false }).unwrap();
        files.job = Some(rx);
        files.loading = true;
        files.tick(&ctx, 0.);
        assert_eq!(files.path, home, "the window shows the folder above the one that went away");
        assert!(files.error.is_none(), "the raw OS error is never shown");
        assert_eq!(files.notice.as_deref(), Some("The folder \u{201C}drag-check\u{201D} can\u{2019}t be found."));
        assert_eq!(files.history.last(), Some(&home), "the history no longer names the removed folder");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_folder_that_cannot_be_listed_says_so_in_a_sentence() {
        assert_eq!(cant_open_message(Path::new("/x/drag-check")), "The folder \u{201C}drag-check\u{201D} can\u{2019}t be opened.");
    }

    #[test]
    fn idle_files_sleeps_without_a_polling_deadline() {
        let files = fixture(vec![]);
        assert_eq!(files.next_wake(1.0), None, "an idle Files window has no timer");
    }

    #[test]
    fn worker_feed_wakes_the_dispatch_main_queue() {
        let (tx, rx) = feed::<u8>();
        tx.send(7).unwrap();
        assert_eq!(rx.try_recv(), Ok(7));
        assert!(bifrost_app::dispatch::drain_main(), "Feed::send must enqueue an event-driven main-queue wake");
    }

    #[test]
    fn expanded_folder_children_are_read_again_only_after_the_folder_changes() {
        let dir = temp("kids");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a"), "x").unwrap();
        let vo = ViewOpts::default();
        assert_eq!(vo.kids_of(&dir.join("sub")).map(|k| k.len()), Some(1));
        assert_eq!(vo.kids_of(&dir.join("sub")).map(|k| k.len()), Some(1), "a second read is served from the cache");
        // the folder's modification time is coarse, so wait before the change
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(dir.join("sub/b"), "x").unwrap();
        assert_eq!(vo.kids_of(&dir.join("sub")).map(|k| k.len()), Some(2));
        assert_eq!(vo.kids_of(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_finds_names_in_subfolders_and_clearing_restores_the_folder() {
        let dir = temp("search");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/needle notes.txt"), "x").unwrap();
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        let mut files = fixture(vec![]);
        files.home = dir.clone();
        files.path = dir.clone();
        files.listing = fs::list(&dir).unwrap();
        files.rows = files.listing.clone();
        files.query.push_str("needle");
        // the Folder scope: the default, This Computer, would walk the whole disk of the machine running the test
        files.scope_bar.scope = Scope::Folder;
        files.restart_search();
        // the search thread streams hits; wait for it to finish
        for _ in 0..500 {
            files.poll_search();
            if files.search.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let hits = files.hits.clone().expect("search returned");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "needle notes.txt");
        assert_eq!(files.visible().len(), 1, "only the hit shows");
        // clearing the query shows the folder again
        files.query.clear();
        files.query_changed();
        assert!(files.hits.is_none());
        assert_eq!(files.visible().len(), files.listing.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn the_file_menu_lists_the_item_actions_with_their_keys() {
        let dir = temp("filemenu");
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let mut files = fixture(fs::list(&dir).unwrap());
        files.select_only(0);
        let row = |files: &Files, title: &str| files.file_menu_items().into_iter().find(|i| i.title == title).map(|i| (i.key, i.enabled));
        assert_eq!(row(&files, "Open"), Some(("cmd+o".into(), true)));
        assert_eq!(row(&files, "Preview"), Some(("space".into(), true)));
        assert_eq!(row(&files, "Move to Trash"), Some(("cmd+backspace".into(), true)));
        assert_eq!(row(&files, "Get Info"), Some(("cmd+i".into(), true)));
        assert_eq!(row(&files, "Duplicate"), Some(("cmd+d".into(), true)));
        assert_eq!(row(&files, "Make Alias"), Some(("ctrl+cmd+a".into(), true)));
        assert_eq!(row(&files, "Rename"), Some((String::new(), true)));
        files.selection.clear();
        assert_eq!(row(&files, "Move to Trash").map(|r| r.1), Some(false), "nothing selected, nothing to trash");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn get_info_describes_the_focused_file() {
        let dir = temp("info");
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let mut files = fixture(fs::list(&dir).unwrap());
        files.select_only(0);
        files.get_info();
        let info = files.info.as_ref().expect("panel open");
        assert_eq!(info.name, "notes.txt");
        assert!(info.rows.contains(&("Kind", "Plain Text".to_string())));
        assert!(info.rows.iter().any(|(label, value)| *label == "Size" && value.contains("5 bytes")));
        assert!(files.info_wanted, "Get Info asks for its own window; the entry points open it with a Ctx");
        // the chord opens it too, and a click closes it
        files.info = None;
        assert!(files.chord(23, CHORD_META));
        assert!(files.info.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn compress_then_extract_round_trips_a_file() {
        let dir = temp("zip");
        std::fs::write(dir.join("a.txt"), "hello zip").unwrap();
        let (tx, _rx) = feed();
        let cancel = AtomicBool::new(false);
        let made = compress_now(&[dir.join("a.txt")], &dir, archive::Format::Zip, &cancel, &tx);
        let archive = made.done.first().expect("archive written").clone();
        assert!(archive.ends_with("a.txt.zip"));
        let out = extract_now(&archive, &dir, &cancel, &tx);
        assert!(out.failed.is_empty(), "{out:?}");
        let folder = out.done.first().expect("folder written");
        assert_eq!(std::fs::read_to_string(folder.join("a.txt")).unwrap(), "hello zip");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn duplicate_keeps_both_without_asking() {
        let dir = temp("dup");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let (tx, _rx) = feed();
        let cancel = AtomicBool::new(false);
        // no answer channel: the job must not wait for one
        let report = transfer_now(Op::Copy, &[dir.join("a.txt")], &dir, &cancel, &tx, None);
        assert_eq!(report.done.len(), 1);
        assert!(dir.join("a 2.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn tags_are_saved_and_follow_a_rename() {
        let dir = temp("tags");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let mut files = fixture(vec![]);
        files.home = dir.clone();
        files.path = dir.clone();
        files.rows = fs::list(&dir).unwrap();
        files.listing = files.rows.clone();
        files.select_only(0);
        files.toggle_tag(Tag::Red);
        assert!(files.tag_index.has(&dir.join("a.txt"), Tag::Red));
        // saved to the sidecar, so a fresh load sees it
        assert!(TagIndex::load(&sidecar_path(&dir)).has(&dir.join("a.txt"), Tag::Red));
        // a second toggle clears it
        files.toggle_tag(Tag::Red);
        assert!(!files.tag_index.has(&dir.join("a.txt"), Tag::Red));
        files.toggle_tag(Tag::Blue);
        // a rename carries the colour to the new name
        files.select_only(0);
        files.start_rename();
        if let Some((_, f)) = files.naming.as_mut() { f.clear(); f.push_str("b.txt"); }
        files.commit_rename();
        assert!(files.tag_index.has(&dir.join("b.txt"), Tag::Blue), "followed the rename");
        assert!(!files.tag_index.has(&dir.join("a.txt"), Tag::Blue));
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn undo_restores_the_tags_an_edit_changed() {
        let dir = temp("tagundo");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let mut files = fixture(vec![]);
        files.home = dir.clone();
        files.path = dir.clone();
        files.rows = fs::list(&dir).unwrap();
        files.listing = files.rows.clone();
        files.select_only(0);
        let a = dir.join("a.txt");
        files.toggle_tag(Tag::Red);
        files.toggle_tag(Tag::Blue);
        assert_eq!(files.file_undo_titles().0, "Undo Tags");
        // Cmd+Z takes Blue off again, then Red; the sidecar follows
        files.undo_file_op();
        assert_eq!(files.tag_index.tags(&a), vec![Tag::Red]);
        assert_eq!(TagIndex::load(&sidecar_path(&dir)).tags(&a), vec![Tag::Red]);
        files.undo_file_op();
        assert!(files.tag_index.tags(&a).is_empty());
        assert!(files.undo.is_empty());
        assert!(files.notice.is_none());
        // a change that changes nothing records nothing
        files.edit_tags(&[a.clone()], |_| {});
        assert!(files.undo.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn tagged_listing_and_drop_targets() {
        let dir = temp("drop");
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.join("Music")).unwrap();
        std::fs::write(dir.join("kept.txt"), "k").unwrap();
        let mut index = TagIndex::default();
        index.toggle(&dir.join("kept.txt"), Tag::Green);
        index.toggle(&dir.join("gone.txt"), Tag::Green);
        // the Tags listing shows only tagged items that still exist
        let tagged = tagged_entries(&index, None);
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].name, "kept.txt");
        // dropping: a folder row is a target, a selected row is not, nor is the bin
        let mut files = fixture(vec![folder("A"), folder("B")]);
        files.home = dir.clone();
        files.path = dir.clone();
        files.select_only(0);
        // the first line of the icon grid: A in the first cell, B in the second (cell centres follow the icon size)
        let (cell_a, cell_b) = (files.side() + 16. + 0.5 * files.grid_w(), files.side() + 16. + 1.5 * files.grid_w());
        let first_line = files.rows_top() + 16. + 0.5 * files.grid_h();
        assert_eq!(files.drop_target_at(cell_b, first_line), Some(PathBuf::from("/fixture/B")));
        assert_eq!(files.drop_target_at(cell_a, first_line), None, "the dragged row is not a target");
        // sidebar: Recents (no header), then each section's header and rows (the Applications row is there only when
        // the machine has an Applications folder); the Trash is not a drop target
        let row = |path: &Path| {
            let mut y = -SIDEBAR_SCROLL;
            for (header, rows) in files.sections() {
                y += if header.is_some() { 30. } else { 0. };
                for place in rows {
                    if place.path == path {
                        return y + 5.;
                    }
                    y += 32.;
                }
            }
            panic!("no sidebar row for {}", path.display())
        };
        assert_eq!(files.drop_target_at(80., row(&dir.join("Documents"))), Some(dir.join("Documents")));
        assert_eq!(files.drop_target_at(80., row(&files.bin())), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
    fn pointer(kind: TouchKind, x: f32, y: f32, t: f64) -> Input {
        Input::Touch { kind, id: 0, x, y, t, source: bifrost_app::TouchSource::Pointer, mods: bifrost_app::proto::ModifierFlags::EMPTY }
    }
    fn test_ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = bifrost_app::proto::Configure { w: 960, h: 630, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: false, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(springboard::font::Font::parse(data).unwrap(), 1.0), cfg)
    }
    fn finger(kind: TouchKind, x: f32, y: f32, t: f64) -> Input {
        Input::Touch { kind, id: 0, x, y, t, source: bifrost_app::TouchSource::Finger, mods: bifrost_app::proto::ModifierFlags::EMPTY }
    }
    #[test]
    fn a_scroll_that_starts_on_an_icon_reaches_the_short_last_line() {
        // nine items, four a line: two full lines and a third with one item, which starts below the window
        let ctx = test_ctx();
        let mut files = fixture((0..9).map(|i| folder(&format!("item{i}"))).collect());
        assert_eq!(files.grid_cols(), 4, "the fixture window holds four icons a line");
        assert_eq!(files.visible().len().div_ceil(files.grid_cols()), 3, "the ninth item has a line of its own");
        let (bottom, cell_h) = (files.bottom(), files.grid_h() - 6.);
        let line_top = |files: &Files, line: f32| files.rows_top() + 16. + line * files.grid_h() - files.scroll;
        assert!(line_top(&files, 2.) + cell_h > bottom, "the third line does not fit at scroll 0: it is drawn cut at the bottom");
        // the wheel over the first icon: the shell sends the notch as a finger's press, one move up and a release
        let x = files.side() + 16. + 0.5 * files.grid_w();
        let y = line_top(&files, 0.) + 0.5 * files.grid_h();
        files.route_input(&ctx, finger(TouchKind::Down, x, y, 0.));
        files.route_input(&ctx, finger(TouchKind::Move, x, y - 40., 16.));
        assert!(!files.dragging, "a scroll over an icon does not drag the item out");
        assert_eq!(files.scroll, 40., "it scrolls the view");
        files.route_input(&ctx, finger(TouchKind::Move, x, y - 400., 32.));
        files.route_input(&ctx, finger(TouchKind::Up, x, y - 400., 48.));
        assert!(!files.dragging && files.selection.is_empty());
        // the scroll range ends with the whole third line inside the view (what `paint_icon_cell` draws)
        let top = line_top(&files, 2.);
        assert!(top >= files.body_top() + 4. && top + cell_h <= bottom, "the third line is in view: {top}..{} of {bottom}", top + cell_h);
        // a mouse press on an icon still drags it out, and leaves the view where it is
        let seen = files.scroll;
        let y = line_top(&files, 2.) + 0.5 * files.grid_h();
        files.route_input(&ctx, pointer(TouchKind::Down, x, y, 100.));
        files.route_input(&ctx, pointer(TouchKind::Move, x, y - 40., 116.));
        assert!(files.dragging, "a mouse drag lifts the item");
        assert_eq!(files.scroll, seen);
    }
    /// Nine icons, four a line, and the centre of the first one.
    fn nine_icons() -> (Files, f32, f32) {
        let files = fixture((0..9).map(|i| folder(&format!("item{i}"))).collect());
        let x = files.side() + 16. + 0.5 * files.grid_w();
        let y = files.rows_top() + 16. + 0.5 * files.grid_h();
        (files, x, y)
    }
    #[test]
    fn a_finger_that_rests_on_an_icon_then_moves_drags_the_item() {
        let ctx = test_ctx();
        let hold = dragout::hold_ms();
        // the first move comes after the hold time: it lifts the item and the view stays where it is
        let (mut files, x, y) = nine_icons();
        files.route_input(&ctx, finger(TouchKind::Down, x, y, 1000.));
        assert_eq!(files.hold_due(), Some(1000. + hold), "the loop wakes when the hold time is up");
        // a finger is never quite still: a small move during the rest does not end the hold
        files.route_input(&ctx, finger(TouchKind::Move, x + 2., y, 1100.));
        assert!(!files.dragging && files.hold_due().is_some());
        files.route_input(&ctx, finger(TouchKind::Move, x + 2., y - 40., 1000. + hold + 50.));
        assert!(files.dragging, "a move after the hold drags the item");
        assert_eq!(files.selected_paths(), vec![PathBuf::from("/fixture/item0")]);
        assert_eq!(files.fx.drag_paths, files.selected_paths());
        assert_eq!(files.scroll, 0., "and does not scroll");
        files.route_input(&ctx, finger(TouchKind::Up, x, y - 40., 1000. + hold + 100.));
        assert!(!files.dragging && files.hold_due().is_none());
        // the loop's tick lifts the item at the hold time, before the finger moves
        let (mut files, x, y) = nine_icons();
        files.route_input(&ctx, finger(TouchKind::Down, x, y, 1000.));
        assert!(!files.hold_tick(&ctx, 1000. + hold - 1.) && !files.dragging, "a shorter rest is not a hold");
        assert!(files.hold_tick(&ctx, 1000. + hold) && files.dragging);
        assert_eq!(files.selection, vec![0], "the lifted item is selected");
        assert!(files.hold_due().is_none() && !files.hold_tick(&ctx, 1000. + hold + 16.));
        // a rest on the empty part of the view lifts nothing
        let (mut files, x, _) = nine_icons();
        files.route_input(&ctx, finger(TouchKind::Down, x + 3. * files.grid_w(), files.bottom() - 4., 0.));
        assert!(files.hold_due().is_none());
    }
    #[test]
    fn a_finger_that_moves_at_once_scrolls_and_never_drags() {
        let ctx = test_ctx();
        let hold = dragout::hold_ms();
        let (mut files, x, y) = nine_icons();
        files.route_input(&ctx, finger(TouchKind::Down, x, y, 0.));
        files.route_input(&ctx, finger(TouchKind::Move, x, y - 40., 16.));
        assert!(!files.dragging);
        assert_eq!(files.scroll, 40.);
        assert!(files.hold_due().is_none() && !files.hold_tick(&ctx, hold + 1.), "the move ended the hold");
        // still down long after the hold time: it goes on scrolling (to the end of this folder, 44 pt down)
        files.route_input(&ctx, finger(TouchKind::Move, x, y - 44., hold + 100.));
        assert!(!files.dragging && files.selection.is_empty());
        assert_eq!(files.scroll, 44.);
        files.route_input(&ctx, finger(TouchKind::Up, x, y - 44., hold + 120.));
        assert!(!files.dragging && files.selection.is_empty());
    }
    #[test]
    fn a_mouse_drag_lifts_the_item_at_once_and_a_mouse_press_never_holds() {
        let ctx = test_ctx();
        let hold = dragout::hold_ms();
        let (mut files, x, y) = nine_icons();
        files.route_input(&ctx, pointer(TouchKind::Down, x, y, 0.));
        assert!(files.hold_due().is_none(), "a mouse press arms no hold");
        // held still past the hold time, then a move inside the slop: nothing is lifted
        assert!(!files.hold_tick(&ctx, hold + 1.));
        files.route_input(&ctx, pointer(TouchKind::Move, x + 3., y, hold + 10.));
        assert!(!files.dragging);
        files.route_input(&ctx, pointer(TouchKind::Up, x + 3., y, hold + 20.));
        // a drag past the slop lifts it without a wait
        files.route_input(&ctx, pointer(TouchKind::Down, x, y, 2000.));
        files.route_input(&ctx, pointer(TouchKind::Move, x, y - 40., 2016.));
        assert!(files.dragging, "a mouse drag lifts the item");
        assert_eq!(files.scroll, 0.);
    }
    /// A one-column icon grid with the folder A above the folder B: the x of the column and the y of each cell's centre.
    fn stacked_folders() -> (Files, f32, f32, f32) {
        let mut files = fixture(vec![folder("A"), folder("B")]);
        files.w = files.side() + 16. + 1.5 * files.grid_w();
        let x = files.side() + 16. + 0.5 * files.grid_w();
        let a = files.rows_top() + 16. + 0.5 * files.grid_h();
        let b = files.rows_top() + 16. + 1.5 * files.grid_h();
        (files, x, a, b)
    }
    #[test]
    fn a_slow_vertical_drag_onto_the_folder_below_moves_the_item() {
        // the real stream: a press on A, many small moves straight down, a release over B. A relative mouse reports a point
        // or two per move, an absolute pointer a few more; none is ever 12 points (the drag slop) from the one before. The
        // pan of an unmoved list must not take the press point along, or the drag never begins and the release is read as
        // a click.
        let ctx = test_ctx();
        for step in [1., 3., 7.] {
            let (mut files, x, a, b) = stacked_folders();
            files.route_input(&ctx, pointer(TouchKind::Down, x, a, 0.));
            let mut y = a;
            let mut t = 0.;
            while y < b {
                y = (y + step).min(b);
                t += 16.;
                files.route_input(&ctx, pointer(TouchKind::Move, x, y, t));
            }
            assert!(files.dragging, "step {step}: the drag began from the press point");
            assert_eq!(files.fx.drag_paths, vec![PathBuf::from("/fixture/A")]);
            assert_eq!(files.drop_target, Some(PathBuf::from("/fixture/B")), "step {step}: B is lit under the pointer");
            files.route_input(&ctx, pointer(TouchKind::Up, x, b, t + 16.));
            assert!(!files.dragging);
            assert!(matches!(files.work.as_ref().map(|w| w.kind), Some(JobKind::Transfer(Op::Move))), "step {step}: the release moves A into B");
            assert_eq!(files.work.as_ref().map(|w| w.dest.clone()), Some(PathBuf::from("/fixture/B")));
        }
    }
    #[test]
    fn a_press_ends_a_drag_whose_release_never_came() {
        // the shell owned the pointer for the session and ended it without a drop here: the next press must not drop
        let ctx = test_ctx();
        let (mut files, x, a, _) = stacked_folders();
        files.dragging = true;
        files.fx.drag_paths = vec![PathBuf::from("/fixture/A")];
        files.drop_target = Some(PathBuf::from("/fixture/B"));
        files.route_input(&ctx, pointer(TouchKind::Down, x, a, 0.));
        assert!(!files.dragging);
        assert!(files.drop_target.is_none());
        assert!(files.fx.drag_paths.is_empty());
        files.route_input(&ctx, pointer(TouchKind::Up, x, a, 40.));
        assert!(files.work.is_none(), "the click moved nothing");
    }
    #[test]
    fn columns_click_the_folders_above_and_the_open_one() {
        let dir = temp("columns");
        std::fs::create_dir_all(dir.join("Projects/app")).unwrap();
        std::fs::write(dir.join("Projects/notes.txt"), "n").unwrap();
        let mut files = fixture(vec![]);
        files.home = dir.clone();
        files.path = dir.join("Projects/app");
        files.columns = true;
        files.grid = false;
        files.w = 960.;
        files.ancestors = vec![
            (dir.clone(), fs::list(&dir).unwrap()),
            (dir.join("Projects"), fs::list(&dir.join("Projects")).unwrap()),
        ];
        files.rows = vec![folder("inside")];
        files.listing = files.rows.clone();
        // the open folder's column sits after the two shown ancestors
        assert_eq!(files.ancestor_shown(), 2);
        assert_eq!(files.item_at(176. + 2. * COLUMN_W + 10., 100.), Some(0));
        assert_eq!(files.item_at(182., 100.), None, "an ancestor column is not an item");
        // the first column lists the root's folders: a click on Projects opens it
        assert_eq!(files.ancestor_hit(182., files.rows_top() + 0.5 * ROW_H), Some(dir.join("Projects")));
        // the second column lists Projects: "app" (a folder, sorted first) opens; notes.txt does not
        assert_eq!(files.ancestor_hit(182. + COLUMN_W, files.rows_top() + 0.5 * ROW_H), Some(dir.join("Projects/app")));
        assert_eq!(files.ancestor_hit(182. + COLUMN_W, files.rows_top() + 1.5 * ROW_H), None, "a file is not opened as a folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn the_item_count_is_singular_for_one() {
        assert_eq!(items_text(0), "0 items");
        assert_eq!(items_text(1), "1 item");
        assert_eq!(items_text(2), "2 items");
    }
    #[test]
    fn the_path_bar_names_the_stand_in_folders_and_walks_down_from_home() {
        let mut files = fixture(vec![]);
        let names = |f: &Files| f.path_items().into_iter().map(|(n, g, _)| format!("{n}:{g}")).collect::<Vec<_>>();
        files.path = PathBuf::from(fs::RECENTS);
        assert_eq!(names(&files), ["Recents:clock"]);
        files.path = PathBuf::from(TAGGED);
        assert_eq!(names(&files), ["Tags:tag"]);
        files.path = tag_path(Tag::Red);
        assert_eq!(names(&files), ["Red:tag"]);
        files.path = PathBuf::from("/fixture/Music/Live");
        assert_eq!(names(&files), ["fixture:house", "Music:folder", "Live:folder"]);
        assert_eq!(files.path_items()[1].2, PathBuf::from("/fixture/Music"), "each segment opens its folder");
        files.path = PathBuf::from("/etc/ssh");
        assert_eq!(names(&files), ["Computer:desktopcomputer", "etc:folder", "ssh:folder"]);
    }
    #[test]
    fn the_sidebar_hides_and_the_toolbar_moves_beside_the_traffic_lights() {
        let mut files = fixture(vec![]);
        let shown = files.bar_bounds().x;
        assert!(files.chord(31, CHORD_CTRL | CHORD_META), "⌃⌘S");
        assert!(files.sidebar_hidden && files.side() == 0.);
        assert!(files.bar_bounds().x < shown && files.bar_bounds().x > traffic_lights_trailing());
        assert!(files.chord(31, CHORD_CTRL | CHORD_META), "⌃⌘S again");
        assert!(!files.sidebar_hidden, "the chord shows it again");
    }
    #[test]
    fn emptying_the_trash_deletes_every_item_for_good() {
        let dir = temp("empty-trash");
        let bin = dir.join(".RecentlyDeleted");
        std::fs::create_dir_all(bin.join("sub")).unwrap();
        std::fs::write(bin.join("1__a.txt"), "x").unwrap();
        std::fs::write(bin.join("2__b.txt"), "y").unwrap();
        let items = vec![bin.join("1__a.txt"), bin.join("2__b.txt"), bin.join("sub")];
        let (tx, _rx) = feed();
        let report = empty_now(&items, &bin, &AtomicBool::new(false), &tx);
        assert_eq!(report.done.len(), 3);
        assert!(report.failed.is_empty() && !report.cancelled);
        let left: Vec<_> = std::fs::read_dir(&bin).unwrap().flatten().filter(|e| e.file_name().to_str() != Some(ORIGINS)).collect();
        assert!(left.is_empty(), "only the origins file remains");
    }
    #[test]
    fn empty_trash_says_so_when_the_bin_has_nothing_to_delete() {
        let mut files = fixture(vec![]);
        files.empty_trash();
        assert_eq!(files.notice.as_deref(), Some("The Trash is already empty"));
    }
    #[test]
    fn the_tab_strip_shows_with_two_tabs_and_moves_the_content_under_it() {
        let mut files = fixture(vec![]);
        assert_eq!(files.body_top(), HEADER_H, "one tab: no strip");
        assert_eq!(files.tab_hit(300., HEADER_H + 5.), None);
        files.tabs.insert_after_active(tabs::TabState { path: "/fixture/Music".into(), history: vec![], at: 0 });
        assert_eq!(files.body_top(), HEADER_H + tabs::TAB_STRIP_H);
        assert_eq!(files.rows_top(), HEADER_H + tabs::TAB_STRIP_H + 34.);
        let (x0, _) = files.tab_cells()[0];
        assert_eq!(files.tab_hit(x0 + 4., HEADER_H + 4.), Some(tabs::TabHit::Select(0)));
        assert_eq!(files.tab_hit(x0 + 4., HEADER_H - 1.), None, "the toolbar is not the strip");
        assert_eq!(files.tab_hit(x0 + 4., files.body_top() + 1.), None, "the content is not the strip");
        assert_eq!(files.tab_label(0), "fixture", "a parked tab is named by its folder");
    }
    #[test]
    fn toolbar_buttons_are_published_as_controls_where_they_are_drawn() {
        let mut files = fixture(vec![]);
        files.update_controls();
        let ids: Vec<u64> = files.controls.nodes().iter().map(|n| n.id).collect();
        for id in [BAR_BACK, BAR_FORWARD, BAR_ICONS, BAR_LIST, BAR_COLUMNS, BAR_GALLERY, BAR_SORT, BAR_SHARE, BAR_TAGS, BAR_MORE, BAR_SEARCH] {
            assert!(ids.contains(&(id as u64)), "{id} published");
        }
        let list = files.controls.nodes().iter().find(|n| n.id == BAR_LIST as u64).unwrap().bounds;
        files.click(list.x + list.w / 2., list.y + list.h / 2., 0.);
        assert!(!files.grid && !files.columns, "the list segment switches to the list view");
        assert!(files.controls.focused().is_none(), "a click leaves no focus ring on the button");
        assert!(files.controls.nodes().iter().all(|n| n.bounds.y < UNIFIED_BAR_PT), "every control is in the unified bar");
        files.w = 768.;
        assert!(files.bar_layout().overflow.is_empty() && files.bar_layout().rect_of(BAR_SEARCH).is_some(), "a 768 pt window fits the whole bar");
        // while a query is active the four view buttons are one pop-up control, and the bar still fits
        files.query.push_str("aqua");
        files.update_controls();
        let ids: Vec<u64> = files.controls.nodes().iter().map(|n| n.id).collect();
        assert!(ids.contains(&(BAR_VIEW as u64)) && ids.contains(&(BAR_SEARCH as u64)), "the pop-up and the field are published");
        assert!(![BAR_ICONS, BAR_LIST, BAR_COLUMNS, BAR_GALLERY].iter().any(|id| ids.contains(&(*id as u64))), "the four buttons are gone while searching");
        assert!(files.bar_layout().overflow.is_empty(), "a 768 pt window fits the bar while searching too");
    }
    #[test]
    fn hidden_files_are_shown_only_when_asked() {
        let mut files = fixture(vec![folder(".cache"), folder("Music")]);
        assert_eq!(files.visible().len(), 1);
        files.toggle_hidden();
        assert_eq!(files.visible().len(), 2);
        assert!(files.show_hidden);
    }
    #[test]
    fn chords_map_to_file_actions_and_plain_keys_are_left_alone() {
        use bifrost_app::proto::{CHORD_META, CHORD_SHIFT};
        let mut files = fixture(vec![folder("A"), folder("B")]);
        files.select_only(0);
        // Cmd+F focuses the search field; Cmd+2 switches to list view
        assert!(files.chord(33, CHORD_META));
        assert!(files.searching);
        assert!(files.chord(3, CHORD_META));
        assert!(!files.grid);
        // Cmd+A selects every visible row
        assert!(files.chord(30, CHORD_META));
        assert_eq!(files.selection, vec![0, 1]);
        // Cmd+Shift+. toggles hidden files
        assert!(files.chord(52, CHORD_META | CHORD_SHIFT));
        assert!(files.show_hidden);
        // no Ctrl or Cmd: not ours
        assert!(!files.chord(33, 0));
        // an unknown chord is not handled
        assert!(!files.chord(19, CHORD_META | CHORD_SHIFT));
    }
    /// A UI stand-in: answers every conflict with `choice`. Returns the event sender for the
    /// job, the answer receiver it reads, and the thread to join after the job.
    fn answering(choice: Answer) -> (Feed<Event>, Receiver<Answer>, std::thread::JoinHandle<()>) {
        let (tx, rx) = feed::<Event>();
        let (answers, answer_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            for event in rx {
                if matches!(event, Event::Conflict(_)) && answers.send(choice).is_err() {
                    break;
                }
            }
        });
        (tx, answer_rx, handle)
    }
    #[test]
    fn copy_paste_moves_and_duplicates_in_a_real_folder() {
        let dir = temp("copy");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("dst")).unwrap();
        std::fs::write(dir.join("src/note.txt"), "hello").unwrap();
        let cancel = AtomicBool::new(false);
        let (tx, answers, helper) = answering(Some((Resolution::KeepBoth, false)));
        // a copy keeps the original; the pasted name is numbered when taken
        let report = transfer_now(Op::Copy, &[dir.join("src/note.txt")], &dir.join("dst"), &cancel, &tx, Some(&answers));
        assert_eq!(report.done.len(), 1);
        assert!(dir.join("src/note.txt").exists() && dir.join("dst/note.txt").exists());
        let report = transfer_now(Op::Copy, &[dir.join("src/note.txt")], &dir.join("dst"), &cancel, &tx, Some(&answers));
        assert!(dir.join("dst/note 2.txt").exists(), "keep both numbers it: {report:?}");
        // a move leaves nothing behind
        std::fs::write(dir.join("src/moved.txt"), "x").unwrap();
        let report = transfer_now(Op::Move, &[dir.join("src/moved.txt")], &dir.join("dst"), &cancel, &tx, Some(&answers));
        assert!(report.failed.is_empty());
        assert!(!dir.join("src/moved.txt").exists() && dir.join("dst/moved.txt").exists());
        drop(tx);
        helper.join().unwrap();
        // a cancelled job stops before it starts
        let (_stop_tx, stop_answers) = mpsc::channel();
        let (tx, _rx) = feed();
        let stop = AtomicBool::new(true);
        let report = transfer_now(Op::Copy, &[dir.join("src/note.txt")], &dir.join("dst"), &stop, &tx, Some(&stop_answers));
        assert!(report.cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn a_conflict_waits_for_the_answer_replace_or_skip() {
        let dir = temp("conflict");
        std::fs::create_dir_all(dir.join("dst")).unwrap();
        std::fs::write(dir.join("a.txt"), "new").unwrap();
        std::fs::write(dir.join("dst/a.txt"), "old").unwrap();
        let cancel = AtomicBool::new(false);
        let (tx, answers, helper) = answering(Some((Resolution::Replace, false)));
        let report = transfer_now(Op::Copy, &[dir.join("a.txt")], &dir.join("dst"), &cancel, &tx, Some(&answers));
        assert!(report.failed.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("dst/a.txt")).unwrap(), "new", "replaced");
        drop(tx);
        helper.join().unwrap();
        let (tx, answers, helper) = answering(Some((Resolution::Skip, false)));
        std::fs::write(dir.join("dst/a.txt"), "kept").unwrap();
        let report = transfer_now(Op::Copy, &[dir.join("a.txt")], &dir.join("dst"), &cancel, &tx, Some(&answers));
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("dst/a.txt")).unwrap(), "kept", "skipped");
        drop(tx);
        helper.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn rename_in_place_refuses_a_taken_name() {
        let dir = temp("rename");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let mut files = fixture(fs::list(&dir).unwrap());
        files.path = dir.clone();
        files.select_only(files.rows.iter().position(|e| e.name == "a.txt").unwrap());
        files.start_rename();
        if let Some((_, f)) = files.naming.as_mut() { f.clear(); f.push_str("b.txt"); }
        files.commit_rename();
        assert!(files.notice.as_deref().unwrap_or("").starts_with("Could not rename"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn trashed_items_go_back_to_their_folder() {
        let dir = temp("bin");
        let bin = dir.join(trash::DIR);
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::write(dir.join("Documents/report.txt"), "r").unwrap();
        let (tx, _rx) = feed();
        let cancel = AtomicBool::new(false);
        let report = trash_now(&[dir.join("Documents/report.txt")], &bin, &cancel, &tx);
        assert_eq!(report.done.len(), 1);
        assert!(!dir.join("Documents/report.txt").exists());
        // the origin is recorded beside the item
        let stored = read_origins(&bin);
        assert_eq!(stored.len(), 1);
        let name = stored[0].0.clone();
        let origin = take_origin(&bin, &name).expect("origin kept");
        assert_eq!(origin, dir.join("Documents"));
        assert!(take_origin(&bin, &name).is_none(), "taken once");
        // put back to the origin
        record_origin(&bin, &name, &dir.join("Documents"));
        trash::restore(&bin.join(&name), &origin).unwrap();
        assert!(dir.join("Documents/report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

