//! File-manager sync extensions: the extension point that lets an app add badges, shortcut-menu items and a toolbar
//! button to the Files app, and watch the folders it cares about (the job of the reference OS's file-manager sync
//! extension and its private host framework).
//!
//! This is the in-process model. An extension implements [`SyncExtension`]; the file manager owns one [`SyncHost`] and
//! asks it three questions: which folders are being observed (so the extension is told when a window opens or closes
//! one), which badge an item shows, and which menu items an extension adds for a context. The extension answers
//! through a [`SyncController`], which holds its observed folders, its registered badges and the badge set on each
//! item, and which also carries the host's view of the current selection (`targeted`, `selected`).
//!
//! Rules kept from the reference behaviour: an extension only sees and badges paths inside the folders it asked to
//! observe; a badge must be registered (id, label, image name) before an item can carry it; setting the badge id to
//! `None` clears it; an extension that asks for a badge on a path outside its folders is refused. Nothing is queried
//! remotely and no extension code runs in the file manager's process: the extension host speaks this same API over a
//! socket.
//!
//! Out of process: an extension app declares its folders in `manifest.toml` (`file_manager_folders`) and answers
//! `ae.filemanager.*` events on its control socket with [`serve`]; the file manager keeps a [`RemoteExtension`] for it
//! (see the notes above that type). The Files app is the host.
//!
//! Gaps: no enable screen in Settings, no service-name list, badge images are names for the shell's icon set (there is
//! no image data path), and nothing starts an extension app that is not running.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Most badges one extension may register.
pub const MAX_BADGES: usize = 16;
/// Most items one menu may hold.
pub const MAX_MENU_ITEMS: usize = 32;

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncError {
    /// The path is not absolute, or holds `..`.
    BadPath,
    /// The path is outside every folder the extension observes.
    NotObserved,
    /// The badge id is blank, already registered, or the extension has [`MAX_BADGES`].
    BadBadge,
    /// The badge id was never registered.
    UnknownBadge,
    /// No extension with that id is registered.
    UnknownExtension,
}

/// A badge an extension registered: the id the extension uses, the label the person sees, the icon name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub id: String,
    pub label: String,
    pub image: String,
}

/// Where a menu is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// the shortcut menu of the selected items
    ContextualItems,
    /// the shortcut menu of the folder background
    ContextualContainer,
    /// the shortcut menu of a sidebar entry
    Sidebar,
    /// the menu behind the extension's toolbar button
    Toolbar,
}

/// One row of a menu an extension adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub id: u32,
    pub title: String,
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(id: u32, title: &str) -> MenuItem {
        MenuItem { id, title: title.to_string(), enabled: true }
    }
}

/// The toolbar button an extension adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarButton {
    pub name: String,
    pub image: String,
    pub tool_tip: String,
}

/// What an extension implements. All methods have do-nothing defaults.
pub trait SyncExtension {
    /// A folder the extension observes came into view.
    fn begin_observing(&mut self, _dir: &Path) {}
    /// A folder the extension observes left every window.
    fn end_observing(&mut self, _dir: &Path) {}
    /// The file manager needs the badge for `path`. The extension answers with `controller.set_badge`; the host also
    /// keeps any badge set earlier.
    fn request_badge(&mut self, _controller: &mut SyncController, _path: &Path) {}
    /// The items this extension adds to a menu.
    fn menu(&mut self, _kind: MenuKind, _controller: &SyncController) -> Vec<MenuItem> {
        Vec::new()
    }
    /// The toolbar button, if the extension has one.
    fn toolbar(&self) -> Option<ToolbarButton> {
        None
    }
    /// The person chose the item `item` of a menu of `kind` that this extension filled; the controller holds the folder
    /// and the selection the menu was for.
    fn perform(&mut self, _kind: MenuKind, _item: u32, _controller: &SyncController) {}
    /// Called on every tick of the file manager. An extension that answers from elsewhere (see [`RemoteExtension`])
    /// hands what arrived to the controller here and returns true when something to draw changed.
    fn poll(&mut self, _controller: &mut SyncController) -> bool {
        false
    }
}

fn clean(path: &Path) -> Result<PathBuf, SyncError> {
    if !path.is_absolute() {
        return Err(SyncError::BadPath);
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => return Err(SyncError::BadPath),
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// One extension's state: its folders, its badges, the badge on each item, and the host's selection.
#[derive(Clone, Debug, Default)]
pub struct SyncController {
    directories: Vec<PathBuf>,
    badges: Vec<Badge>,
    item_badges: BTreeMap<PathBuf, String>,
    targeted: Option<PathBuf>,
    selected: Vec<PathBuf>,
}

impl SyncController {
    /// The folders the extension observes.
    pub fn directory_urls(&self) -> &[PathBuf] {
        &self.directories
    }

    /// Replaces the folder set. Badges on items that fell outside the new set are dropped.
    pub fn set_directory_urls(&mut self, dirs: &[PathBuf]) -> Result<(), SyncError> {
        let mut cleaned = Vec::new();
        for d in dirs {
            let d = clean(d)?;
            if !cleaned.contains(&d) {
                cleaned.push(d);
            }
        }
        self.directories = cleaned;
        let dirs = self.directories.clone();
        self.item_badges.retain(|p, _| dirs.iter().any(|d| p.starts_with(d)));
        Ok(())
    }

    fn observes(&self, path: &Path) -> bool {
        self.directories.iter().any(|d| path.starts_with(d))
    }

    /// Registers a badge. The id is unique per extension.
    pub fn set_badge_image(&mut self, id: &str, label: &str, image: &str) -> Result<(), SyncError> {
        let id = id.trim();
        if id.is_empty() || self.badges.len() >= MAX_BADGES || self.badges.iter().any(|b| b.id == id) {
            return Err(SyncError::BadBadge);
        }
        self.badges.push(Badge { id: id.to_string(), label: label.trim().to_string(), image: image.to_string() });
        Ok(())
    }

    /// The registered badges, in registration order.
    pub fn badges(&self) -> &[Badge] {
        &self.badges
    }

    /// Puts the badge `id` on `path`, or clears it with `None`.
    pub fn set_badge(&mut self, path: &Path, id: Option<&str>) -> Result<(), SyncError> {
        let path = clean(path)?;
        if !self.observes(&path) {
            return Err(SyncError::NotObserved);
        }
        match id {
            None => {
                self.item_badges.remove(&path);
            }
            Some(id) => {
                if !self.badges.iter().any(|b| b.id == id) {
                    return Err(SyncError::UnknownBadge);
                }
                self.item_badges.insert(path, id.to_string());
            }
        }
        Ok(())
    }

    /// The badge on `path`.
    pub fn badge_for(&self, path: &Path) -> Option<&Badge> {
        let id = self.item_badges.get(path)?;
        self.badges.iter().find(|b| &b.id == id)
    }

    /// The folder the person is looking at, as the host last reported it.
    pub fn targeted_url(&self) -> Option<&Path> {
        self.targeted.as_deref()
    }

    /// The selected items, as the host last reported them. Empty outside the extension's folders.
    pub fn selected_item_urls(&self) -> &[PathBuf] {
        &self.selected
    }

    fn set_context(&mut self, targeted: Option<&Path>, selected: &[PathBuf]) {
        self.targeted = targeted.and_then(|t| clean(t).ok()).filter(|t| self.observes(t));
        let selected: Vec<PathBuf> = selected.iter().filter_map(|p| clean(p).ok()).filter(|p| self.observes(p)).collect();
        self.selected = selected;
    }
}

struct Registered {
    id: String,
    controller: SyncController,
    extension: Box<dyn SyncExtension>,
    /// folders currently reported as begun, so begin and end calls pair up
    open: Vec<PathBuf>,
}

/// The file manager's side: every enabled extension and what it asked for.
#[derive(Default)]
pub struct SyncHost {
    extensions: Vec<Registered>,
}

/// A menu item with the extension that added it, so a tap goes back to the right extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMenuItem {
    pub extension: String,
    pub item: MenuItem,
}

impl SyncHost {
    pub fn new() -> SyncHost {
        SyncHost::default()
    }

    /// Enables an extension. An id already in use replaces the older extension.
    pub fn register(&mut self, id: &str, controller: SyncController, extension: Box<dyn SyncExtension>) {
        self.extensions.retain(|e| e.id != id);
        self.extensions.push(Registered { id: id.to_string(), controller, extension, open: Vec::new() });
    }

    /// Disables an extension, ending the folders it still had open.
    pub fn unregister(&mut self, id: &str) -> bool {
        let Some(i) = self.extensions.iter().position(|e| e.id == id) else { return false };
        let mut gone = self.extensions.remove(i);
        for dir in std::mem::take(&mut gone.open) {
            gone.extension.end_observing(&dir);
        }
        true
    }

    /// The controller of one extension.
    pub fn controller(&self, id: &str) -> Option<&SyncController> {
        self.extensions.iter().find(|e| e.id == id).map(|e| &e.controller)
    }

    /// Ids of the enabled extensions, in the order they were enabled.
    pub fn extension_ids(&self) -> Vec<&str> {
        self.extensions.iter().map(|e| e.id.as_str()).collect()
    }

    /// A window now shows `dir`. Every extension whose folders contain it hears `begin_observing` once, until the
    /// matching [`SyncHost::folder_closed`].
    pub fn folder_opened(&mut self, dir: &Path) -> Result<usize, SyncError> {
        let dir = clean(dir)?;
        let mut told = 0;
        for e in &mut self.extensions {
            if e.controller.observes(&dir) && !e.open.contains(&dir) {
                e.open.push(dir.clone());
                e.extension.begin_observing(&dir);
                told += 1;
            }
        }
        Ok(told)
    }

    /// No window shows `dir` any more.
    pub fn folder_closed(&mut self, dir: &Path) -> Result<usize, SyncError> {
        let dir = clean(dir)?;
        let mut told = 0;
        for e in &mut self.extensions {
            if let Some(i) = e.open.iter().position(|d| *d == dir) {
                e.open.remove(i);
                e.extension.end_observing(&dir);
                told += 1;
            }
        }
        Ok(told)
    }

    /// The badges that show on `path`: one per extension that badged it, in the order the extensions were enabled.
    /// Each extension is asked first, so a badge it has not set yet appears on the next draw.
    pub fn badges_for(&mut self, path: &Path) -> Vec<Badge> {
        let Ok(path) = clean(path) else { return Vec::new() };
        let mut out = Vec::new();
        for e in &mut self.extensions {
            if !e.controller.observes(&path) {
                continue;
            }
            e.extension.request_badge(&mut e.controller, &path);
            if let Some(b) = e.controller.badge_for(&path) {
                out.push(b.clone());
            }
        }
        out
    }

    /// The items the extensions add to a menu. The host's selection is handed to each extension first, narrowed to its
    /// own folders; an extension with no folder covering the targeted folder adds nothing.
    pub fn menu(&mut self, kind: MenuKind, targeted: &Path, selected: &[PathBuf]) -> Vec<HostMenuItem> {
        let mut out = Vec::new();
        for e in &mut self.extensions {
            e.controller.set_context(Some(targeted), selected);
            if e.controller.targeted_url().is_none() {
                continue;
            }
            let mut items = e.extension.menu(kind, &e.controller);
            items.truncate(MAX_MENU_ITEMS);
            out.extend(items.into_iter().map(|item| HostMenuItem { extension: e.id.clone(), item }));
        }
        out
    }

    /// The toolbar buttons of the extensions that have one, with the extension that owns each.
    pub fn toolbar_buttons(&self) -> Vec<(String, ToolbarButton)> {
        self.extensions.iter().filter_map(|e| e.extension.toolbar().map(|b| (e.id.clone(), b))).collect()
    }

    /// Whether `extension` is enabled (for a tap that arrives after it was disabled).
    pub fn has(&self, extension: &str) -> Result<(), SyncError> {
        if self.extensions.iter().any(|e| e.id == extension) {
            Ok(())
        } else {
            Err(SyncError::UnknownExtension)
        }
    }
}

// --- out of process: the extension app behind its control socket --------------------------------------------------
//
// An extension app declares the folders it watches in its manifest (`file_manager_folders`). The file manager keeps a
// [`RemoteExtension`] for it in the [`SyncHost`]: the same questions the in-process model asks (begin and end
// observing, a badge for a path, the menu items, a tap on one) go to the app as `ae.filemanager.<id>` events on its
// control socket, and the app answers them with [`serve`] from its `rpc` handler. Nothing leaves the folders the
// manifest names: the host only asks about paths inside them, and the app's own controller refuses the rest.
//
// The file manager must not wait on another app while it draws. Badges are asked for on a worker thread, in batches,
// and the answers wait in a cache that the file manager reads on its next draw ([`SyncHost::poll`] reports when one
// changed); an answer is asked for again after [`STALE_AFTER`]. Menu items are asked for on the spot with a short
// timeout, and an app that does not answer is left alone for [`MUTE_FOR`].

use crate::app_events::{AppEvent, AppEventReply, ERR_TARGET_UNREACHABLE, ERR_TIMEOUT};
use crate::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::Json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The event class of every request to an extension app.
pub const EVENT_CLASS: &str = "filemanager";
/// Most paths in one badge request.
pub const MAX_PATHS: usize = 64;
/// Most selected paths a menu request carries.
pub const MAX_SELECTED: usize = 256;
/// A badge answer is asked for again after this long.
pub const STALE_AFTER: Duration = Duration::from_secs(5);
/// An app that did not answer a menu request is not asked again for this long.
pub const MUTE_FOR: Duration = Duration::from_secs(30);
const BADGE_TIMEOUT: Duration = Duration::from_secs(2);
const MENU_TIMEOUT: Duration = Duration::from_millis(200);
const PERFORM_TIMEOUT: Duration = Duration::from_secs(5);

impl MenuKind {
    /// The name the kind has on the wire.
    pub fn name(self) -> &'static str {
        match self {
            MenuKind::ContextualItems => "items",
            MenuKind::ContextualContainer => "container",
            MenuKind::Sidebar => "sidebar",
            MenuKind::Toolbar => "toolbar",
        }
    }

    pub fn parse(s: &str) -> Option<MenuKind> {
        [MenuKind::ContextualItems, MenuKind::ContextualContainer, MenuKind::Sidebar, MenuKind::Toolbar].into_iter().find(|k| k.name() == s)
    }
}

fn paths_json(paths: &[PathBuf]) -> Json {
    Json::Arr(paths.iter().filter_map(|p| p.to_str()).map(|s| Json::Str(s.to_string())).collect())
}

fn paths_of(v: Option<&Json>, limit: usize) -> Vec<PathBuf> {
    v.and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(Json::as_str).map(PathBuf::from).take(limit).collect()
}

fn invalid(what: &str) -> RpcError {
    RpcError::new(INVALID_PARAMS, what.to_string())
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The extension app's side: answers one `ae.filemanager.*` event with `ext` and its `controller`. Call it from the
/// app's `rpc` handler for the methods that start with `ae.filemanager.` (`AppEvent::from_method`).
pub fn serve(ev: &AppEvent, controller: &mut SyncController, ext: &mut dyn SyncExtension) -> Result<Json, RpcError> {
    if ev.class != EVENT_CLASS {
        return Err(RpcError::new(METHOD_NOT_FOUND, format!("no handler for {}", ev.method())));
    }
    let p = &ev.params;
    let path_of = |key: &str| -> Result<PathBuf, RpcError> {
        let raw = p.str_field(key).ok_or_else(|| invalid("a path is missing"))?;
        clean(Path::new(&raw)).map_err(|_| invalid("the path is not absolute or holds .."))
    };
    let context = |controller: &mut SyncController| -> Result<MenuKind, RpcError> {
        let kind = p.str_field("kind").and_then(|k| MenuKind::parse(&k)).ok_or_else(|| invalid("the menu kind is unknown"))?;
        let targeted = path_of("targeted")?;
        let selected = paths_of(p.get("selected"), MAX_SELECTED);
        controller.set_context(Some(&targeted), &selected);
        Ok(kind)
    };
    match ev.id.as_str() {
        "describe" => {
            let badges = controller.badges().iter().map(|b| obj(vec![("id", Json::Str(b.id.clone())), ("label", Json::Str(b.label.clone())), ("image", Json::Str(b.image.clone()))])).collect();
            let toolbar = match ext.toolbar() {
                Some(t) => obj(vec![("name", Json::Str(t.name)), ("image", Json::Str(t.image)), ("tool_tip", Json::Str(t.tool_tip))]),
                None => Json::Null,
            };
            Ok(obj(vec![("badges", Json::Arr(badges)), ("toolbar", toolbar)]))
        }
        "observe" => {
            let dir = path_of("folder")?;
            if !controller.observes(&dir) {
                return Err(invalid("the folder is not one the extension observes"));
            }
            if matches!(p.get("begin"), Some(Json::Bool(true))) {
                ext.begin_observing(&dir);
            } else {
                ext.end_observing(&dir);
            }
            Ok(obj(vec![]))
        }
        "badges" => {
            let mut items = Vec::new();
            for raw in paths_of(p.get("paths"), MAX_PATHS) {
                let badge = match clean(&raw) {
                    Ok(path) if controller.observes(&path) => {
                        ext.request_badge(controller, &path);
                        controller.badge_for(&path).map(|b| Json::Str(b.id.clone())).unwrap_or(Json::Null)
                    }
                    _ => Json::Null,
                };
                items.push(obj(vec![("path", Json::Str(raw.to_string_lossy().into_owned())), ("badge", badge)]));
            }
            Ok(obj(vec![("items", Json::Arr(items))]))
        }
        "menu" => {
            let kind = context(controller)?;
            if controller.targeted_url().is_none() {
                return Ok(obj(vec![("items", Json::Arr(vec![]))]));
            }
            let mut items = ext.menu(kind, controller);
            items.truncate(MAX_MENU_ITEMS);
            let rows = items.into_iter().map(|i| obj(vec![("id", Json::Num(f64::from(i.id))), ("title", Json::Str(i.title)), ("enabled", Json::Bool(i.enabled))])).collect();
            Ok(obj(vec![("items", Json::Arr(rows))]))
        }
        "perform" => {
            let kind = context(controller)?;
            if controller.targeted_url().is_none() {
                return Err(invalid("the folder is not one the extension observes"));
            }
            let item = p.get("item").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| invalid("the menu item is missing"))?;
            ext.perform(kind, item, controller);
            Ok(obj(vec![]))
        }
        _ => Err(RpcError::new(METHOD_NOT_FOUND, format!("no handler for {}", ev.method()))),
    }
}

/// How a request reaches the extension app: the event and how long to wait. The real one is [`socket_transport`].
pub type Transport = Arc<dyn Fn(&AppEvent, Duration) -> AppEventReply + Send + Sync>;

/// The transport over the control socket of the extension app.
pub fn socket_transport(socket: PathBuf) -> Transport {
    Arc::new(move |ev: &AppEvent, timeout: Duration| crate::app_events::send_message(&socket, ev, timeout))
}

enum Job {
    Describe,
    Observe { dir: PathBuf, begin: bool },
    Badges(Vec<PathBuf>),
    Perform { kind: MenuKind, item: u32, targeted: PathBuf, selected: Vec<PathBuf> },
}

enum Answer {
    Described { badges: Vec<Badge>, toolbar: Option<ToolbarButton> },
    Badges(Vec<(PathBuf, Option<String>)>),
}

fn event(id: &str, fields: Vec<(&str, Json)>) -> AppEvent {
    AppEvent::new(EVENT_CLASS, id, obj(fields))
}

fn context_event(id: &str, kind: MenuKind, item: Option<u32>, targeted: &Path, selected: &[PathBuf]) -> AppEvent {
    let mut fields = vec![("kind", Json::Str(kind.name().into())), ("targeted", Json::Str(targeted.to_string_lossy().into_owned())), ("selected", paths_json(selected))];
    if let Some(item) = item {
        fields.push(("item", Json::Num(f64::from(item))));
    }
    event(id, fields)
}

/// The worker: one request at a time, badge requests that wait together are sent as one.
fn work(transport: Transport, jobs: Receiver<Job>, answers: Sender<Answer>) {
    let mut backlog: VecDeque<Job> = VecDeque::new();
    loop {
        let Some(job) = backlog.pop_front().or_else(|| jobs.recv().ok()) else { break };
        match job {
            Job::Describe => {
                let reply = transport(&event("describe", vec![]), BADGE_TIMEOUT);
                if reply.ok() {
                    let badges = reply
                        .result
                        .get("badges")
                        .and_then(Json::as_array)
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|b| Some(Badge { id: b.str_field("id")?, label: b.str_field("label").unwrap_or_default(), image: b.str_field("image").unwrap_or_default() }))
                        .collect();
                    let toolbar = reply.result.get("toolbar").filter(|t| t.get("name").is_some()).and_then(|t| {
                        Some(ToolbarButton { name: t.str_field("name")?, image: t.str_field("image").unwrap_or_default(), tool_tip: t.str_field("tool_tip").unwrap_or_default() })
                    });
                    let _ = answers.send(Answer::Described { badges, toolbar });
                    wake();
                }
            }
            Job::Observe { dir, begin } => {
                let _ = transport(&event("observe", vec![("folder", Json::Str(dir.to_string_lossy().into_owned())), ("begin", Json::Bool(begin))]), BADGE_TIMEOUT);
            }
            Job::Badges(mut paths) => {
                while paths.len() < MAX_PATHS {
                    match jobs.try_recv() {
                        Ok(Job::Badges(more)) => paths.extend(more),
                        Ok(other) => {
                            backlog.push_back(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                paths.truncate(MAX_PATHS);
                let reply = transport(&event("badges", vec![("paths", paths_json(&paths))]), BADGE_TIMEOUT);
                let mut got: HashMap<PathBuf, Option<String>> = HashMap::new();
                if reply.ok() {
                    for item in reply.result.get("items").and_then(Json::as_array).unwrap_or(&[]) {
                        if let Some(path) = item.str_field("path") {
                            got.insert(PathBuf::from(path), item.str_field("badge"));
                        }
                    }
                }
                // a path the app did not answer for (or a failed call) is cached as "no badge" until it goes stale
                let rows = paths.into_iter().map(|p| {
                    let badge = got.remove(&p).flatten();
                    (p, badge)
                });
                let _ = answers.send(Answer::Badges(rows.collect()));
                wake();
            }
            Job::Perform { kind, item, targeted, selected } => {
                let _ = transport(&context_event("perform", kind, Some(item), &targeted, &selected), PERFORM_TIMEOUT);
            }
        }
    }
}

/// Wakes the app loop (the main queue) so it reads the answer without waiting for input.
fn wake() {
    crate::dispatch::DispatchQueue::main().async_(|| {});
}

/// An extension app seen from the file manager. See the module notes above.
pub struct RemoteExtension {
    jobs: Sender<Job>,
    answers: Receiver<Answer>,
    transport: Transport,
    cache: HashMap<PathBuf, (Option<String>, Instant)>,
    asked: HashSet<PathBuf>,
    toolbar: Option<ToolbarButton>,
    muted_until: Option<Instant>,
    stale_after: Duration,
}

impl RemoteExtension {
    /// Connects through `transport` and asks the app to describe its badges and toolbar button.
    pub fn connect(transport: Transport) -> RemoteExtension {
        let (jobs, job_rx) = channel();
        let (answer_tx, answers) = channel();
        let worker = transport.clone();
        std::thread::spawn(move || work(worker, job_rx, answer_tx));
        let _ = jobs.send(Job::Describe);
        RemoteExtension { jobs, answers, transport, cache: HashMap::new(), asked: HashSet::new(), toolbar: None, muted_until: None, stale_after: STALE_AFTER }
    }

    /// How long a badge answer is trusted before it is asked for again.
    pub fn set_stale_after(&mut self, after: Duration) {
        self.stale_after = after;
    }

    /// Whether menu requests are being skipped because the app did not answer the last one.
    pub fn muted(&self) -> bool {
        self.muted_until.is_some_and(|t| Instant::now() < t)
    }

    fn apply(controller: &mut SyncController, path: &Path, badge: Option<&str>) {
        // a badge the app never registered is refused by the controller; the draw then shows none
        let _ = controller.set_badge(path, badge);
    }
}

impl SyncExtension for RemoteExtension {
    fn begin_observing(&mut self, dir: &Path) {
        let _ = self.jobs.send(Job::Observe { dir: dir.to_path_buf(), begin: true });
    }

    fn end_observing(&mut self, dir: &Path) {
        let _ = self.jobs.send(Job::Observe { dir: dir.to_path_buf(), begin: false });
    }

    fn request_badge(&mut self, controller: &mut SyncController, path: &Path) {
        if let Some((badge, at)) = self.cache.get(path) {
            Self::apply(controller, path, badge.as_deref());
            if at.elapsed() < self.stale_after {
                return;
            }
        }
        if self.asked.len() < 4 * MAX_PATHS && self.asked.insert(path.to_path_buf()) {
            let _ = self.jobs.send(Job::Badges(vec![path.to_path_buf()]));
        }
    }

    fn menu(&mut self, kind: MenuKind, controller: &SyncController) -> Vec<MenuItem> {
        let Some(targeted) = controller.targeted_url() else { return Vec::new() };
        if self.muted() {
            return Vec::new();
        }
        let reply = (self.transport)(&context_event("menu", kind, None, targeted, controller.selected_item_urls()), MENU_TIMEOUT);
        if !reply.ok() {
            if matches!(reply.error_number, ERR_TIMEOUT | ERR_TARGET_UNREACHABLE) {
                self.muted_until = Some(Instant::now() + MUTE_FOR);
            }
            return Vec::new();
        }
        reply
            .result
            .get("items")
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|i| {
                Some(MenuItem { id: u32::try_from(i.get("id")?.as_u64()?).ok()?, title: i.str_field("title")?, enabled: !matches!(i.get("enabled"), Some(Json::Bool(false))) })
            })
            .take(MAX_MENU_ITEMS)
            .collect()
    }

    fn perform(&mut self, kind: MenuKind, item: u32, controller: &SyncController) {
        if let Some(targeted) = controller.targeted_url() {
            let _ = self.jobs.send(Job::Perform { kind, item, targeted: targeted.to_path_buf(), selected: controller.selected_item_urls().to_vec() });
        }
    }

    fn toolbar(&self) -> Option<ToolbarButton> {
        self.toolbar.clone()
    }

    fn poll(&mut self, controller: &mut SyncController) -> bool {
        let mut changed = false;
        while let Ok(answer) = self.answers.try_recv() {
            match answer {
                Answer::Described { badges, toolbar } => {
                    for b in badges {
                        let _ = controller.set_badge_image(&b.id, &b.label, &b.image);
                    }
                    self.toolbar = toolbar;
                    changed = true;
                }
                Answer::Badges(rows) => {
                    for (path, badge) in rows {
                        self.asked.remove(&path);
                        let before = self.cache.insert(path.clone(), (badge.clone(), Instant::now())).map(|(b, _)| b);
                        if before.as_ref() != Some(&badge) {
                            changed = true;
                        }
                        Self::apply(controller, &path, badge.as_deref());
                    }
                }
            }
        }
        changed
    }
}

impl SyncHost {
    /// Hands the answers that came from the extension apps to their controllers. True when something to draw changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for e in &mut self.extensions {
            changed |= e.extension.poll(&mut e.controller);
        }
        changed
    }

    /// The person chose `item` of a menu `extension` filled, for the folder `targeted` and the `selected` items.
    pub fn perform(&mut self, extension: &str, kind: MenuKind, item: u32, targeted: &Path, selected: &[PathBuf]) -> Result<(), SyncError> {
        let e = self.extensions.iter_mut().find(|e| e.id == extension).ok_or(SyncError::UnknownExtension)?;
        e.controller.set_context(Some(targeted), selected);
        if e.controller.targeted_url().is_none() {
            return Err(SyncError::NotObserved);
        }
        e.extension.perform(kind, item, &e.controller);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Log(Vec<String>);

    struct Cloud {
        log: Rc<RefCell<Log>>,
    }

    impl SyncExtension for Cloud {
        fn begin_observing(&mut self, dir: &Path) {
            self.log.borrow_mut().0.push(format!("begin {}", dir.display()));
        }
        fn end_observing(&mut self, dir: &Path) {
            self.log.borrow_mut().0.push(format!("end {}", dir.display()));
        }
        fn request_badge(&mut self, c: &mut SyncController, path: &Path) {
            if path.extension().is_some_and(|e| e == "tmp") {
                c.set_badge(path, Some("pending")).unwrap();
            }
        }
        fn menu(&mut self, kind: MenuKind, c: &SyncController) -> Vec<MenuItem> {
            match kind {
                MenuKind::ContextualItems if !c.selected_item_urls().is_empty() => vec![MenuItem::new(1, "Share Link")],
                MenuKind::ContextualContainer => vec![MenuItem::new(2, "Sync This Folder")],
                MenuKind::Toolbar => vec![MenuItem { id: 3, title: "Pause Sync".into(), enabled: false }],
                _ => Vec::new(),
            }
        }
        fn toolbar(&self) -> Option<ToolbarButton> {
            Some(ToolbarButton { name: "Cloud".into(), image: "cloud".into(), tool_tip: "Sync status".into() })
        }
    }

    fn controller(dirs: &[&str]) -> SyncController {
        let mut c = SyncController::default();
        c.set_directory_urls(&dirs.iter().map(PathBuf::from).collect::<Vec<_>>()).unwrap();
        c.set_badge_image("pending", "Waiting to sync", "arrow.clockwise").unwrap();
        c.set_badge_image("done", "Up to date", "checkmark").unwrap();
        c
    }

    fn host(dirs: &[&str]) -> (SyncHost, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut h = SyncHost::new();
        h.register("cloud", controller(dirs), Box::new(Cloud { log: log.clone() }));
        (h, log)
    }

    #[test]
    fn badges_are_registered_once_and_must_exist_before_use() {
        let mut c = controller(&["/home/a/Sync"]);
        assert_eq!(c.set_badge_image("done", "x", "y"), Err(SyncError::BadBadge));
        assert_eq!(c.set_badge_image("  ", "x", "y"), Err(SyncError::BadBadge));
        let item = Path::new("/home/a/Sync/report.txt");
        assert_eq!(c.set_badge(item, Some("missing")), Err(SyncError::UnknownBadge));
        c.set_badge(item, Some("done")).unwrap();
        assert_eq!(c.badge_for(item).map(|b| b.label.as_str()), Some("Up to date"));
        c.set_badge(item, None).unwrap();
        assert!(c.badge_for(item).is_none());
    }

    #[test]
    fn at_most_sixteen_badges_per_extension() {
        let mut c = SyncController::default();
        for i in 0..MAX_BADGES {
            c.set_badge_image(&format!("b{i}"), "l", "i").unwrap();
        }
        assert_eq!(c.set_badge_image("extra", "l", "i"), Err(SyncError::BadBadge));
    }

    #[test]
    fn an_extension_only_badges_paths_inside_its_folders() {
        let mut c = controller(&["/home/a/Sync"]);
        assert_eq!(c.set_badge(Path::new("/home/a/Other/x"), Some("done")), Err(SyncError::NotObserved));
        assert_eq!(c.set_badge(Path::new("/home/a/Sync/../Other/x"), Some("done")), Err(SyncError::BadPath));
        assert_eq!(c.set_badge(Path::new("Sync/x"), Some("done")), Err(SyncError::BadPath));
        // a sibling that only shares a name prefix is outside the folder
        assert_eq!(c.set_badge(Path::new("/home/a/SyncX/x"), Some("done")), Err(SyncError::NotObserved));
    }

    #[test]
    fn shrinking_the_folder_set_drops_badges_that_fell_outside() {
        let mut c = controller(&["/home/a/One", "/home/a/Two"]);
        c.set_badge(Path::new("/home/a/One/f"), Some("done")).unwrap();
        c.set_badge(Path::new("/home/a/Two/f"), Some("done")).unwrap();
        c.set_directory_urls(&[PathBuf::from("/home/a/Two")]).unwrap();
        assert!(c.badge_for(Path::new("/home/a/One/f")).is_none());
        assert!(c.badge_for(Path::new("/home/a/Two/f")).is_some());
        assert_eq!(c.set_directory_urls(&[PathBuf::from("rel")]), Err(SyncError::BadPath));
        assert_eq!(c.directory_urls().len(), 1, "a refused set leaves the folders alone");
    }

    #[test]
    fn begin_and_end_pair_up_per_folder() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync/docs")), Ok(1));
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync/docs")), Ok(0), "a second window on the same folder is not told again");
        assert_eq!(h.folder_opened(Path::new("/home/a/Elsewhere")), Ok(0));
        assert_eq!(h.folder_closed(Path::new("/home/a/Sync/docs")), Ok(1));
        assert_eq!(h.folder_closed(Path::new("/home/a/Sync/docs")), Ok(0));
        assert_eq!(log.borrow().0, ["begin /home/a/Sync/docs", "end /home/a/Sync/docs"]);
    }

    #[test]
    fn disabling_an_extension_ends_its_open_folders() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        h.folder_opened(Path::new("/home/a/Sync")).unwrap();
        assert!(h.unregister("cloud"));
        assert!(!h.unregister("cloud"));
        assert_eq!(log.borrow().0.last().map(String::as_str), Some("end /home/a/Sync"));
        assert_eq!(h.has("cloud"), Err(SyncError::UnknownExtension));
    }

    #[test]
    fn the_host_asks_for_a_badge_and_returns_it() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        let got = h.badges_for(Path::new("/home/a/Sync/draft.tmp"));
        assert_eq!(got.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["pending"]);
        assert!(h.badges_for(Path::new("/home/a/Sync/final.txt")).is_empty());
        assert!(h.badges_for(Path::new("/home/a/Elsewhere/draft.tmp")).is_empty(), "outside the extension's folders");
        assert_eq!(h.controller("cloud").unwrap().badges().len(), 2);
    }

    #[test]
    fn menus_follow_the_kind_and_the_selection_is_narrowed_to_the_extension() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        let dir = Path::new("/home/a/Sync");
        let none = h.menu(MenuKind::ContextualItems, dir, &[]);
        assert!(none.is_empty(), "no selection, no item menu");
        let outside = h.menu(MenuKind::ContextualItems, dir, &[PathBuf::from("/home/a/Elsewhere/x")]);
        assert!(outside.is_empty(), "selected items outside the folders are not shown to the extension");
        let some = h.menu(MenuKind::ContextualItems, dir, &[PathBuf::from("/home/a/Sync/x")]);
        assert_eq!(some, vec![HostMenuItem { extension: "cloud".into(), item: MenuItem::new(1, "Share Link") }]);
        assert_eq!(h.controller("cloud").unwrap().selected_item_urls(), [PathBuf::from("/home/a/Sync/x")]);
        let bg = h.menu(MenuKind::ContextualContainer, dir, &[]);
        assert_eq!(bg[0].item.title, "Sync This Folder");
        let tool = h.menu(MenuKind::Toolbar, dir, &[]);
        assert!(!tool[0].item.enabled);
    }

    #[test]
    fn a_folder_outside_the_extension_gets_no_menu_and_the_toolbar_button_is_listed() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        assert!(h.menu(MenuKind::ContextualContainer, Path::new("/home/a/Elsewhere"), &[]).is_empty());
        let buttons = h.toolbar_buttons();
        assert_eq!(buttons.len(), 1);
        assert_eq!((buttons[0].0.as_str(), buttons[0].1.name.as_str()), ("cloud", "Cloud"));
    }

    #[test]
    fn registering_an_id_again_replaces_the_extension() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        h.register("cloud", controller(&["/home/a/Other"]), Box::new(Cloud { log: log.clone() }));
        assert_eq!(h.extension_ids(), ["cloud"]);
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync")), Ok(0));
        assert_eq!(h.folder_opened(Path::new("/home/a/Other")), Ok(1));
    }

    #[test]
    fn menus_are_capped() {
        struct Many;
        impl SyncExtension for Many {
            fn menu(&mut self, _: MenuKind, _: &SyncController) -> Vec<MenuItem> {
                (0..100).map(|i| MenuItem::new(i, "x")).collect()
            }
        }
        let mut h = SyncHost::new();
        h.register("many", controller(&["/d"]), Box::new(Many));
        assert_eq!(h.menu(MenuKind::Sidebar, Path::new("/d"), &[]).len(), MAX_MENU_ITEMS);
    }

    // --- out of process ---

    use std::sync::Mutex;

    /// An extension app in a test: the served side, plus a log of what it was asked.
    struct Served {
        controller: SyncController,
        ext: Box<dyn SyncExtension + Send>,
        asked: Vec<String>,
    }

    struct App {
        performed: Arc<Mutex<Vec<String>>>,
    }

    impl SyncExtension for App {
        fn request_badge(&mut self, c: &mut SyncController, path: &Path) {
            if path.extension().is_some_and(|e| e == "tmp") {
                c.set_badge(path, Some("pending")).unwrap();
            }
        }
        fn menu(&mut self, kind: MenuKind, c: &SyncController) -> Vec<MenuItem> {
            match kind {
                MenuKind::ContextualItems if !c.selected_item_urls().is_empty() => vec![MenuItem::new(7, "Share Link")],
                MenuKind::ContextualContainer => vec![MenuItem { id: 8, title: "Sync This Folder".into(), enabled: false }],
                _ => Vec::new(),
            }
        }
        fn perform(&mut self, kind: MenuKind, item: u32, c: &SyncController) {
            let names: Vec<String> = c.selected_item_urls().iter().map(|p| p.display().to_string()).collect();
            self.performed.lock().unwrap().push(format!("{} {item} {}", kind.name(), names.join(",")));
        }
        fn toolbar(&self) -> Option<ToolbarButton> {
            Some(ToolbarButton { name: "Cloud".into(), image: "cloud".into(), tool_tip: "Sync status".into() })
        }
    }

    /// The transport of the test: the app's `serve` behind a lock, recording the event methods.
    fn served(dirs: &[&str]) -> (Transport, Arc<Mutex<Served>>, Arc<Mutex<Vec<String>>>) {
        let performed = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(Served { controller: controller(dirs), ext: Box::new(App { performed: performed.clone() }), asked: Vec::new() }));
        let shared = state.clone();
        let transport: Transport = Arc::new(move |ev: &AppEvent, _: Duration| {
            let mut s = shared.lock().unwrap();
            s.asked.push(format!("{} {}", ev.id, ev.params.encode()));
            let Served { controller, ext, .. } = &mut *s;
            match serve(ev, controller, ext.as_mut()) {
                Ok(result) => AppEventReply { error_number: 0, error_string: None, result },
                Err(e) => AppEventReply { error_number: e.code, error_string: Some(e.message), result: Json::Null },
            }
        });
        (transport, state, performed)
    }

    fn until(host: &mut SyncHost, what: &str, mut done: impl FnMut(&mut SyncHost) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            host.poll();
            if done(host) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}");
    }

    fn remote_host(dirs: &[&str]) -> (SyncHost, Arc<Mutex<Served>>, Arc<Mutex<Vec<String>>>) {
        let (transport, state, performed) = served(dirs);
        let mut c = SyncController::default();
        c.set_directory_urls(&dirs.iter().map(PathBuf::from).collect::<Vec<_>>()).unwrap();
        let mut ext = RemoteExtension::connect(transport);
        ext.set_stale_after(Duration::from_secs(3600));
        let mut host = SyncHost::new();
        host.register("cloud", c, Box::new(ext));
        (host, state, performed)
    }

    #[test]
    fn the_app_describes_its_badges_and_a_badge_arrives_for_the_next_draw() {
        let (mut host, _state, _) = remote_host(&["/home/a/Sync"]);
        until(&mut host, "the badge descriptions", |h| h.controller("cloud").is_some_and(|c| c.badges().len() == 2));
        assert_eq!(host.toolbar_buttons().first().map(|(id, b)| (id.as_str(), b.name.as_str())), Some(("cloud", "Cloud")));
        let tmp = Path::new("/home/a/Sync/draft.tmp");
        // the first draw has no answer yet; the app is asked on the side and the answer lands in the cache
        assert!(host.badges_for(tmp).is_empty());
        until(&mut host, "the badge answer", |h| !h.badges_for(tmp).is_empty());
        assert_eq!(host.badges_for(tmp)[0].id, "pending");
        // a path the app has no badge for stays clear
        let plain = Path::new("/home/a/Sync/report.txt");
        host.badges_for(plain);
        until(&mut host, "the clear answer", |h| h.controller("cloud").is_some_and(|c| c.badge_for(plain).is_none()));
        assert!(host.badges_for(plain).is_empty());
    }

    #[test]
    fn only_paths_inside_the_observed_folders_are_ever_sent() {
        let (mut host, state, _) = remote_host(&["/home/a/Sync"]);
        host.folder_opened(Path::new("/home/a/Sync")).unwrap();
        assert_eq!(host.folder_opened(Path::new("/home/a/Documents")), Ok(0));
        for outside in ["/home/a/Documents/x.tmp", "/etc/passwd"] {
            assert!(host.badges_for(Path::new(outside)).is_empty());
        }
        assert!(host.menu(MenuKind::ContextualItems, Path::new("/home/a/Documents"), &[PathBuf::from("/home/a/Documents/x")]).is_empty());
        host.badges_for(Path::new("/home/a/Sync/in.tmp"));
        until(&mut host, "the answer", |h| !h.badges_for(Path::new("/home/a/Sync/in.tmp")).is_empty());
        let asked = state.lock().unwrap().asked.join("\n");
        assert!(asked.contains("/home/a/Sync/in.tmp"), "{asked}");
        assert!(!asked.contains("Documents") && !asked.contains("passwd"), "{asked}");
    }

    #[test]
    fn menu_items_come_from_the_app_and_a_tap_goes_back_to_it() {
        let (mut host, _state, performed) = remote_host(&["/home/a/Sync"]);
        let dir = Path::new("/home/a/Sync");
        let picked = [PathBuf::from("/home/a/Sync/a.txt"), PathBuf::from("/home/b/outside.txt")];
        let items = host.menu(MenuKind::ContextualItems, dir, &picked);
        assert_eq!(items, vec![HostMenuItem { extension: "cloud".into(), item: MenuItem::new(7, "Share Link") }]);
        let folder = host.menu(MenuKind::ContextualContainer, dir, &[]);
        assert_eq!(folder.len(), 1);
        assert!(!folder[0].item.enabled, "a disabled row stays disabled");
        host.perform("cloud", MenuKind::ContextualItems, 7, dir, &picked).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while performed.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // the selection the app saw was narrowed to its folder
        assert_eq!(*performed.lock().unwrap(), vec!["items 7 /home/a/Sync/a.txt".to_string()]);
        assert_eq!(host.perform("gone", MenuKind::Toolbar, 1, dir, &[]), Err(SyncError::UnknownExtension));
        assert_eq!(host.perform("cloud", MenuKind::Toolbar, 1, Path::new("/elsewhere"), &[]), Err(SyncError::NotObserved));
    }

    #[test]
    fn an_app_that_does_not_answer_is_left_alone_for_a_while() {
        let calls = Arc::new(Mutex::new(0usize));
        let seen = calls.clone();
        let dead: Transport = Arc::new(move |_: &AppEvent, _: Duration| {
            *seen.lock().unwrap() += 1;
            AppEventReply { error_number: ERR_TARGET_UNREACHABLE, error_string: Some("down".into()), result: Json::Null }
        });
        let mut c = SyncController::default();
        c.set_directory_urls(&[PathBuf::from("/home/a/Sync")]).unwrap();
        let mut host = SyncHost::new();
        host.register("cloud", c, Box::new(RemoteExtension::connect(dead)));
        let dir = Path::new("/home/a/Sync");
        let before = *calls.lock().unwrap();
        assert!(host.menu(MenuKind::ContextualContainer, dir, &[]).is_empty());
        let after_first = *calls.lock().unwrap();
        assert!(after_first > before);
        assert!(host.menu(MenuKind::ContextualContainer, dir, &[]).is_empty());
        // the second menu was not even asked: only the describe/observe traffic on the worker may add calls
        std::thread::sleep(Duration::from_millis(50));
        let menus_asked = *calls.lock().unwrap() - after_first;
        assert!(menus_asked <= 1, "the muted app is not asked for a menu again ({menus_asked})");
    }

    #[test]
    fn serve_refuses_what_it_cannot_answer() {
        let mut c = controller(&["/home/a/Sync"]);
        let mut app = App { performed: Arc::new(Mutex::new(Vec::new())) };
        let ask = |id: &str, params: Json| serve(&AppEvent::new(EVENT_CLASS, id, params), &mut controller(&["/home/a/Sync"]), &mut App { performed: Arc::new(Mutex::new(Vec::new())) });
        assert_eq!(ask("nope", Json::Null).unwrap_err().code, METHOD_NOT_FOUND);
        let relative = obj(vec![("folder", Json::Str("Sync".into())), ("begin", Json::Bool(true))]);
        assert_eq!(ask("observe", relative).unwrap_err().code, INVALID_PARAMS);
        let outside = obj(vec![("folder", Json::Str("/etc".into())), ("begin", Json::Bool(true))]);
        assert_eq!(ask("observe", outside).unwrap_err().code, INVALID_PARAMS);
        let other_class = AppEvent::new("notes", "make", Json::Null);
        assert_eq!(serve(&other_class, &mut c, &mut app).unwrap_err().code, METHOD_NOT_FOUND);
        let traversal = obj(vec![("paths", Json::Arr(vec![Json::Str("/home/a/Sync/../../etc/passwd".into())]))]);
        let reply = ask("badges", traversal).unwrap();
        assert_eq!(reply.get("items").and_then(Json::as_array).map(|i| i[0].get("badge").cloned()), Some(Some(Json::Null)));
    }
}
