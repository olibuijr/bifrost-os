//! The shell side of the app protocol (docs/apps.md): installed-app registry,
//! spawning an app over a `SOCK_SEQPACKET` socketpair, receiving its shared
//! frame buffers and presenting them as the app surface.
//!
//! The shell keeps the newest presented buffer mapped and held (it is the
//! switcher card and the crash screenshot) and releases the older one when a
//! newer frame arrives, so the app always has exactly one free buffer.

use crate::gfx::{Damage, Surface};
use bifrost_proto::sys::{self, Channel, Map, Recv};
use bifrost_proto::{Configure, KeyInput, ToApp, ToShell, TouchKind, CHORD_CTRL, CHORD_SHIFT, SDK_VERSION};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

/// `manifest.toml` of an installed package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub exec: String,
    pub icon: String,
    pub min_shell: String,
    pub summary: String,
    pub category: String,
    pub permissions: Vec<String>,
    /// `entitlements = ["log-store"]`: privileged capabilities, honoured only for the kinds that may hold them
    /// (`log-store`: `system-app`; see `app_policy::Policy::with_entitlements`).
    pub entitlements: Vec<String>,
    /// `"app"` (default), `"system-app"`, `"system"` or `"service"`; see `shell::kinds`. Unknown kinds fail to parse.
    pub kind: String,
    pub provides: Vec<String>,
    /// `"native"`: a native Wayland app owns its GPU surfaces instead of using SDK frames. `"full-size"`: an SDK app
    /// with a full-size-content window (no title row; the traffic lights sit over its sidebar).
    pub window: String,
    /// Window size in points (`NativeWindow` content size): `window_size = "WxH"` is the initial size and
    /// `"fixed:WxH"` a fixed-size window (no resizing, no zoom), like Calculator.
    pub window_size: String,
    /// `min_size = "WxH"` / `max_size = "WxH"` in points (`NativeWindow.contentMinSize` / `contentMaxSize`).
    pub min_size: String,
    pub max_size: String,
    pub wayland_app_id: String,
    /// What the app opens, `type` or `type:rank` (rank owner, default, alternate, none; default is `default`): `handles = ["public.png:owner", "public.text"]`.
    pub handles: Vec<String>,
    /// URL schemes the app opens: `schemes = ["https"]`.
    pub schemes: Vec<String>,
    /// App Intents declared as `[[intent]]` tables (`shell::intents`); Search lists the shortcuts.
    pub intents: Vec<super::intents::ManifestIntent>,
}

impl Manifest {
    /// The app draws to the top edge of its window (`NativeWindowStyleMaskFullSizeContentView`).
    pub fn full_size_content(&self) -> bool {
        self.window == "full-size"
    }
    /// The window size rules from `window_size`, `min_size` and `max_size`, in points.
    pub fn size_limits(&self) -> SizeLimits {
        let fixed = self.window_size.trim().strip_prefix("fixed:").and_then(parse_size);
        let initial = fixed.or_else(|| parse_size(&self.window_size));
        let min = fixed.or_else(|| parse_size(&self.min_size));
        let max = fixed.or_else(|| parse_size(&self.max_size));
        SizeLimits { initial, min, max, fixed: fixed.is_some() }
    }
    /// Not an app: a system package or service (never on the home grid, never spawned by the shell).
    pub fn is_system(&self) -> bool {
        super::kinds::Kind::of(&self.kind).hidden_from_grid()
    }

    /// Parses the small TOML subset manifests use: `key = "string"` and
    /// `permissions = ["a", "b"]`; `#` comments and unknown keys are ignored.
    pub fn parse(text: &str) -> Option<Manifest> {
        let mut m = Manifest::default();
        // inside an `[[intent]]` table keys belong to the intent; any other table header ends it (its keys are ignored)
        let mut table: Option<bool> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && !line.contains('=') {
                table = Some(line == "[[intent]]");
                if line == "[[intent]]" {
                    m.intents.push(Default::default());
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let strs = || -> Vec<String> { v.trim_start_matches('[').trim_end_matches(']').split(',').map(|s| unquote(s.trim())).filter(|s| !s.is_empty()).collect() };
            if let Some(is_intent) = table {
                if let (true, Some(i)) = (is_intent, m.intents.last_mut()) {
                    match k {
                        "id" => i.id = unquote(v),
                        "title" => i.title = unquote(v),
                        "description" => i.description = unquote(v),
                        "shortcut" => i.shortcut = unquote(v) == "true",
                        "params" => i.params = strs(),
                        _ => {}
                    }
                }
                continue;
            }
            match k {
                "id" => m.id = unquote(v),
                "name" => m.name = unquote(v),
                "version" => m.version = unquote(v),
                "exec" => m.exec = unquote(v),
                "icon" => m.icon = unquote(v),
                "min_shell" => m.min_shell = unquote(v),
                "summary" => m.summary = unquote(v),
                "category" => m.category = unquote(v),
                "permissions" => m.permissions = strs(),
                "entitlements" => m.entitlements = strs(),
                "provides" => m.provides = strs(),
                "handles" => m.handles = strs(),
                "schemes" => m.schemes = strs(),
                "kind" => m.kind = unquote(v),
                "window" => m.window = unquote(v),
                "window_size" => m.window_size = unquote(v),
                "min_size" => m.min_size = unquote(v),
                "max_size" => m.max_size = unquote(v),
                "wayland_app_id" => m.wayland_app_id = unquote(v),
                _ => {}
            }
        }
        // ids become directory names: keep them to a safe alphabet
        let safe = !m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
        if super::kinds::Kind::parse(&m.kind).is_none() {
            return None;
        }
        let exec_ok = if super::kinds::Kind::of(&m.kind).exec_optional() && m.exec.is_empty() { true } else { !m.exec.is_empty() && !m.exec.starts_with('/') && !m.exec.split('/').any(|p| p == "..") };
        (safe && exec_ok && !m.name.is_empty()).then_some(m)
    }
}

/// An app window's size rules in points (`NativeWindow.contentMinSize`, `contentMaxSize`, a non-resizable style mask).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeLimits {
    pub initial: Option<(f32, f32)>,
    pub min: Option<(f32, f32)>,
    pub max: Option<(f32, f32)>,
    /// Not resizable and not zoomable; `initial`, `min` and `max` are all the fixed size.
    pub fixed: bool,
}

/// `"320x520"` as points; anything else (zero, negative, malformed) is `None`.
fn parse_size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    (w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()).then_some((w, h))
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_string()
}

/// An app found under `<root>/apps/<id>/current/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub manifest: Manifest,
    /// the version directory (`current` resolved)
    pub dir: PathBuf,
}

impl Installed {
    pub fn exec_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.exec)
    }
    pub fn icon_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.icon)
    }
}

/// Lists the installed apps under `<root>/apps`, in install order (`apps/installed.json`, newest last).
pub fn scan(root: &Path) -> Vec<Installed> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(root.join("apps")) else { return v };
    for e in rd.flatten() {
        let dir = e.path().join("current");
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.toml")) else { continue };
        let Some(manifest) = Manifest::parse(&text) else { continue };
        // system packages are data for the shell, not apps: no slot, no icon, never spawned
        if manifest.is_system() {
            continue;
        }
        if manifest.id != e.file_name().to_string_lossy() || !dir.join(&manifest.exec).is_file() {
            continue;
        }
        v.push(Installed { manifest, dir });
    }
    // install order (apps/installed.json, newest last) keeps home-grid positions stable;
    // apps missing from the registry go last, by name
    let order: Vec<String> = super::store::install::installed_in(root).into_iter().map(|a| a.id).collect();
    let pos = |id: &str| order.iter().position(|o| o == id).unwrap_or(usize::MAX);
    v.sort_by(|a, b| pos(&a.manifest.id).cmp(&pos(&b.manifest.id)).then_with(|| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase())));
    v
}

/// Shared snapshots keep running launches valid while package metadata changes.
static INSTALLED: std::sync::RwLock<Option<std::sync::Arc<[Installed]>>> = std::sync::RwLock::new(None);
static ROOT: OnceLock<PathBuf> = OnceLock::new();
pub fn register(root: &Path) {
    let _ = ROOT.set(root.to_path_buf());
    if let Ok(mut g) = INSTALLED.write() { *g = Some(scan(root).into()); }
}
fn merge_installed(old: &[Installed], scanned: Vec<Installed>) -> Vec<Installed> {
    let mut all = old.to_vec();
    for app in scanned {
        if let Some(existing) = all.iter_mut().find(|a| a.manifest.id == app.manifest.id) { *existing = app; }
        else if all.len() < 200 { all.push(app); }
    }
    all
}
/// Replace existing metadata while preserving SDK slot identities.
pub fn refresh() -> bool {
    let scanned = scan(&root());
    let Ok(mut g) = INSTALLED.write() else { return false };
    let all = merge_installed(g.as_deref().unwrap_or(&[]), scanned);
    if g.as_deref() == Some(all.as_slice()) { return false; }
    *g = Some(all.into());
    true
}
/// Signature of the on-disk registry: mtimes of `apps/` and `apps/installed.json`.
fn disk_stamp(root: &Path) -> (Option<std::time::SystemTime>, Option<std::time::SystemTime>) {
    let m = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    (m(root.join("apps")), m(root.join("apps/installed.json")))
}
/// True when another process (`--install-store-app`, `--install-package`) changed the apps
/// registry since the last call. Cheap (two stats); the caller throttles it to about 1 Hz.
pub fn disk_changed() -> bool {
    static LAST: std::sync::Mutex<Option<(Option<std::time::SystemTime>, Option<std::time::SystemTime>)>> = std::sync::Mutex::new(None);
    let now = disk_stamp(&root());
    let Ok(mut g) = LAST.lock() else { return false };
    let first = g.is_none();
    let changed = g.is_some_and(|old| old != now);
    *g = Some(now);
    !first && changed
}
pub fn installed() -> std::sync::Arc<[Installed]> {
    INSTALLED.read().ok().and_then(|g| g.clone()).unwrap_or_else(||std::sync::Arc::from([]))
}

/// The shell data dir (`/var/lib/springboard`), also the apps' asset dir.
pub fn root() -> PathBuf {
    ROOT.get().cloned().unwrap_or_else(|| PathBuf::from("/var/lib/springboard"))
}

/// The slot of an installed app by manifest id.
pub fn slot_of(id: &str) -> Option<usize> {
    installed().iter().position(|a| a.manifest.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    /// exited with this code (`128 + signal` when killed by a signal)
    Exited(i32),
}

/// The manifest id of the on-screen keyboard system app (`provides = ["keyboard"]`).
pub const KEYBOARD_APP_ID: &str = "is.olibuijr.keyboard";

/// What `pump` saw that the shell must act on besides the frame.
#[derive(Debug, Default, PartialEq)]
pub struct Pumped {
    /// a new frame is ready to blit
    pub frame: bool,
    pub requests: Vec<ToShell>,
    /// the app exited or its socket closed during this pump
    pub died: bool,
}

struct Buf {
    map: Map,
}

pub struct AppHost {
    pub id: String,
    /// The binary this process was started from (empty for a host on a test channel).
    exec: PathBuf,
    /// The appearance flags the app last received (`u32::MAX` until the first send).
    appearance_sent: u32,
    child: Option<Child>,
    chan: Channel,
    bufs: Vec<Buf>,
    size: (usize, usize),
    buffer_size: (usize, usize),
    /// the buffer holding the newest frame
    shown: Option<usize>,
    /// the buffer the newest frame replaced: given back after that frame is blitted, so the app
    /// draws at most one frame per composite (paced by the display, not free-running)
    release_after_blit: Option<usize>,
    pending: Damage,
    status: Status,
    pub wants_keyboard: bool,
    pub hello: bool,
    pub commands: bifrost_proto::Commands,
    /// the app's Edit menu titles for Undo and Redo (empty: the plain words)
    pub undo_titles: (String, String),
    pub controls: Vec<bifrost_proto::Node>,
    /// The app's own main-menu items (`ToShell::AppMenus`), shown by the desktop bar while the app is in front.
    pub app_menus: Vec<bifrost_proto::AppMenu>,
    /// Hierarchy and state beside `controls` (`ToShell::ControlsTree`); empty means a flat list.
    pub controls_tree: Vec<bifrost_proto::NodeExt>,
    last_notification: Option<f64>,
    /// `ToShell::SuddenTermination`: the app allows being killed with no Quit round trip.
    pub sudden: bool,
    /// `ToShell::Activity`: the union of the app's live `ActivityOptions`; a user-initiated or latency-critical
    /// activity keeps the app from being suspended when it leaves the foreground.
    pub activity: u32,
    /// Set by `begin_quit`: when (monotonic ms) a Quit that was not answered turns into SIGKILL.
    quit_at: Option<f64>,
    /// The dock badge the app set (`ToShell::SetBadge`), already cleaned by `clean_badge`.
    pub badge: String,
    /// `ToShell::DocumentState`: the document's title, unsaved-changes flag and file name (cleaned by `clean_badge`-style trimming).
    pub doc_title: String,
    pub doc_edited: bool,
    pub doc_file: String,
    /// The window's vibrancy regions in window points (`ToShell::Materials`); the desk turns them into window px.
    pub materials: Vec<bifrost_proto::Material>,
    /// The focused field's text around the caret, caret rectangle (window points) and traits, as the app last
    /// reported them (`ToShell::TextState`, `CaretRect`, `InputTraits`); `inputAccessoryView` buttons.
    pub text_state: bifrost_proto::TextState,
    pub caret_rect: Option<(f32, f32, f32, f32)>,
    pub input_traits: bifrost_proto::InputTraits,
    pub accessory: Vec<bifrost_proto::AccessoryItem>,
}

impl AppHost {
    /// Starts `exec` with the app end of a fresh socketpair and sends `cfg`.
    pub fn spawn(id: &str, exec: &Path, data_dir: &Path, shell_dir: &Path, cfg: Configure) -> std::io::Result<AppHost> {
        let (mine, theirs) = sys::socketpair_pair()?;
        let _ = std::fs::create_dir_all(data_dir);
        let fd = theirs.fd();
        #[cfg(feature="desktop-wayland")]
        let mut cmd = if std::env::var_os("SPRINGBOARD_WAYLAND").is_some() {
            let manifest=Manifest::parse(&std::fs::read_to_string(exec.parent().unwrap().join("manifest.toml"))?).ok_or_else(||std::io::Error::other("Invalid app manifest"))?;
            if manifest.id!=id {return Err(std::io::Error::other("App identity mismatch"));}
            let policy=crate::bare::app_policy::Policy::from_permissions(id,&manifest.permissions)?.with_entitlements(id,&manifest.kind,&manifest.entitlements);
            crate::bare::app_policy::command(id,exec,data_dir,policy,false)?
        } else {Command::new(exec)};
        #[cfg(not(feature="desktop-wayland"))]
        let mut cmd = Command::new(exec);
        cmd.env("BIFROST_SOCK", fd.to_string())
            .env("BIFROST_DATA", data_dir)
            .stdin(Stdio::null())
            .current_dir(data_dir);
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_none() {cmd.env("BIFROST_SHELL_DIR",shell_dir);}
        // SAFETY: only async-signal-safe fcntl between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                sys::make_inheritable(fd);
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        drop(theirs);
        let mut h = AppHost::with_channel(id, mine, Some(child), cfg);
        h.exec = exec.to_path_buf();
        h.send(&ToApp::Configure(cfg));
        Ok(h)
    }

    /// A host on an existing channel (tests drive the app side themselves).
    pub fn with_channel(id: &str, chan: Channel, child: Option<Child>, cfg: Configure) -> AppHost {
        AppHost {
            id: id.to_string(),
            exec: PathBuf::new(),
            appearance_sent: u32::MAX,
            child,
            chan,
            bufs: Vec::new(),
            size: (cfg.w as usize, cfg.h as usize),
            buffer_size: (cfg.w as usize, cfg.h as usize),
            shown: None,
            release_after_blit: None,
            pending: Damage::new(cfg.w as usize, cfg.h as usize),
            status: Status::Running,
            wants_keyboard: false,
            hello: false,
            commands: bifrost_proto::Commands::default(),
            undo_titles: Default::default(),
            controls: Vec::new(),
            app_menus: Vec::new(),
            controls_tree: Vec::new(),
            last_notification: None,
            badge: String::new(),
            sudden: false,
            activity: 0,
            quit_at: None,
            doc_title: String::new(),
            doc_edited: false,
            doc_file: String::new(),
            materials: Vec::new(),
            text_state: bifrost_proto::TextState::default(),
            caret_rect: None,
            input_traits: bifrost_proto::InputTraits::default(),
            accessory: Vec::new(),
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn running(&self) -> bool {
        self.status == Status::Running
    }

    /// Tells the app the system appearance settings when they differ from what it last got (and on first contact).
    pub fn sync_appearance(&mut self, flags: u32) {
        if self.appearance_sent != flags {
            self.appearance_sent = flags;
            self.send(&ToApp::Appearance { flags });
        }
    }

    /// True when the installed binary is no longer the one this process runs (the app was updated while open), so
    /// opening it again must start the new version instead of focusing the old process.
    pub fn is_outdated(&self, installed: &Path) -> bool {
        !self.exec.as_os_str().is_empty() && self.exec != installed
    }

    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// The raw socket, for the shell's poll loop.
    pub fn fd(&self) -> sys::Fd {
        self.chan.fd()
    }

    pub fn send(&mut self, m: &ToApp) {
        if self.running() && self.chan.send(&m.encode(), &[]).is_err() {
            self.reap();
        }
    }

    /// Sends `m` with one descriptor (SCM_RIGHTS): the file a document panel granted. The app gets its own copy of the
    /// descriptor; the shell's copy closes when `file` is dropped.
    pub fn send_file(&mut self, m: &ToApp, file: &std::fs::File) {
        use std::os::fd::AsRawFd;
        if self.running() && self.chan.send(&m.encode(), &[file.as_raw_fd()]).is_err() {
            self.reap();
        }
    }

    pub fn configure(&mut self, cfg: Configure) {
        self.size = (cfg.w as usize, cfg.h as usize);
        self.send(&ToApp::Configure(cfg));
    }

    pub fn touch(&mut self, kind: TouchKind, id: u32, x: f32, y: f32, t_ms: u32) {
        self.send(&ToApp::Touch { kind, id, x, y, t_ms });
    }

    pub fn key(&mut self, k: KeyInput) {
        self.send(&ToApp::Key(k));
    }

    pub fn suspend(&mut self) {
        if self.activity & (bifrost_proto::activity::USER_INITIATED | bifrost_proto::activity::LATENCY_CRITICAL) != 0 {
            return; // `ProcessInfo.beginActivity`: not throttled
        }
        #[cfg(feature="desktop-wayland")]
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_some(){crate::bare::app_manager::focus(&self.id,false);}
        self.send(&ToApp::Suspend);
    }

    pub fn resume(&mut self) {
        #[cfg(feature="desktop-wayland")]
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_some(){crate::bare::app_manager::focus(&self.id,true);}
        self.send(&ToApp::Resume);
        // the app redraws on Resume; until then keep showing the last frame
    }

    /// Reads every queued message without blocking.
    pub fn pump(&mut self) -> Pumped {
        let mut out = Pumped::default();
        if !self.running() {
            return out;
        }
        loop {
            match self.chan.recv(false) {
                Ok(Recv::Packet(p, fds)) => {
                    let Ok(msg) = ToShell::decode(&p) else { continue };
                    self.handle(msg, fds, &mut out);
                }
                Ok(Recv::Empty) => break,
                Ok(Recv::Closed) | Err(_) => {
                    self.reap();
                    out.died = true;
                    break;
                }
            }
        }
        if self.running() {
            if let Some(c) = &mut self.child {
                if let Ok(Some(st)) = c.try_wait() {
                    self.finish(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)));
                    out.died = true;
                }
            }
        }
        out
    }

    fn handle(&mut self, msg: ToShell, fds: Vec<sys::Owned>, out: &mut Pumped) {
        match msg {
            ToShell::Hello { sdk_version, app_id } => {
                if sdk_version != SDK_VERSION || (!self.id.is_empty() && app_id != self.id) {
                    eprintln!("springboard: app {} sent Hello sdk {sdk_version} id {app_id}; killing", self.id);
                    self.kill();
                    out.died = true;
                } else {
                    self.hello = true;
                }
            }
            ToShell::Buffers { w, h } => {
                let (w, h) = (w as usize, h as usize);
                let bytes = w * h * 4;
                if fds.len() == 2 && w > 0 && h > 0 && w <= 8192 && h <= 8192 {
                    let maps: Vec<Buf> = fds.iter().filter_map(|f| Map::new(f, bytes).ok()).map(|map| Buf { map }).collect();
                    if maps.len() == 2 {
                        self.bufs = maps;
                        self.buffer_size = (w, h);
                        self.shown = None;
                        self.release_after_blit = None;
                        self.pending = Damage::new(w, h);
                    }
                }
            }
            ToShell::Frame { buf, damage } => {
                let buf = buf as usize;
                if buf < self.bufs.len() {
                    if let Some(old) = self.shown.filter(|&o| o != buf) {
                        if let Some(prev) = self.release_after_blit.replace(old).filter(|&p| p != old && p != buf) {
                            self.send(&ToApp::Release { buf: prev as u32 });
                        }
                    }
                    self.shown = Some(buf);
                    for r in damage {
                        self.pending.add(r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
                    }
                    out.frame = true;
                }
            }
            ToShell::WantsKeyboard { on } => {
                self.wants_keyboard = on;
                out.requests.push(ToShell::WantsKeyboard { on });
            }
            // Only the keyboard system app may type into the focused field or size the keyboard; from any other app it is dropped.
            ToShell::KeyboardOut(o) => {
                if self.id == KEYBOARD_APP_ID {
                    out.requests.push(ToShell::KeyboardOut(o));
                }
            }
            ToShell::Controls(nodes) => self.controls=nodes,
            ToShell::AppMenus(menus) => {if self.app_menus!=menus {self.app_menus=menus;out.frame=true;}},
            ToShell::ControlsTree(tree) => self.controls_tree=tree,
            ToShell::TextState(t) => {
                // an empty state is the app saying no text field has focus any more
                if t == bifrost_proto::TextState::default() {
                    self.caret_rect = None;
                    self.input_traits = bifrost_proto::InputTraits::default();
                    self.accessory.clear();
                }
                self.text_state = t;
            }
            ToShell::CaretRect { x, y, w, h } => self.caret_rect = Some((x, y, w, h)),
            ToShell::InputTraits(t) => {
                if self.input_traits != t {
                    self.input_traits = t;
                    out.requests.push(ToShell::InputTraits(t));
                }
            }
            ToShell::SuddenTermination { enabled } => self.sudden = enabled,
            ToShell::Activity { options } => self.activity = options,
            ToShell::InputAccessory(items) => self.accessory = items.into_iter().take(bifrost_proto::MAX_ACCESSORY_ITEMS).collect(),
            ToShell::Materials(list) => {if self.materials!=list {self.materials=list;out.frame=true;}},
            ToShell::Commands(commands) => {if self.commands!=commands {self.commands=commands;out.frame=true;}},
            ToShell::UndoTitles { undo, redo } => {if self.undo_titles!=(undo.clone(),redo.clone()) {self.undo_titles=(undo,redo);out.frame=true;}},
            ToShell::Log { text } => bifrost_core::log::shell_logger("apphost").info(format!("app {}: {text}", self.id)),
            other => out.requests.push(other),
        }
    }

    /// The newest frame: pixels and size (the switcher card, the crash screen).
    pub fn frame(&self) -> Option<(&[u32], usize, usize)> {
        let b = self.bufs.get(self.shown?)?;
        Some((b.map.pixels(), self.buffer_size.0, self.buffer_size.1))
    }

    /// Whether frames arrived that `blit` has not yet copied.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Copies the newest frame into `dst` at (0, 0) and returns the rectangles
    /// that changed since the last `blit` (all of it when `full`), clipped to
    /// `dst`. Drawing only the damage relies on `dst` keeping the last frame.
    pub fn blit(&mut self, dst: &mut Surface, full: bool) -> Damage {
        let mut done = Damage::new(dst.w, dst.h);
        let Some((px, w, h)) = self.frame() else { return done };
        let src_all = [crate::gfx::Area { x0: 0, y0: 0, x1: w, y1: h }];
        let areas: Vec<crate::gfx::Area> = if full { src_all.to_vec() } else { self.pending.areas().to_vec() };
        for a in areas {
            let (x0, x1) = (a.x0.min(dst.w), a.x1.min(dst.w).min(w));
            for y in a.y0..a.y1.min(dst.h).min(h) {
                if y < dst.y0 || x0 >= x1 {
                    continue;
                }
                let d = (y - dst.y0) * dst.stride;
                dst.px[d + x0..d + x1].copy_from_slice(&px[y * w + x0..y * w + x1]);
            }
            done.add(x0 as i32, a.y0 as i32, (x1 - x0) as i32, (a.y1 - a.y0) as i32);
        }
        self.pending.clear();
        if let Some(old) = self.release_after_blit.take() {
            self.send(&ToApp::Release { buf: old as u32 });
        }
        done
    }

    fn finish(&mut self, code: i32) {
        self.status = Status::Exited(code);
        bifrost_core::log::shell_logger("apphost").notice(format!("app {} exited {code}", self.id));
    }

    fn reap(&mut self) {
        if !self.running() {
            return;
        }
        let code = match self.child.as_mut().map(|c| c.wait()) {
            Some(Ok(st)) => st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)),
            _ => 0,
        };
        self.finish(code);
    }

    /// Stops the app at once: SIGKILL, with a Quit first (and a polite moment) only when it is not suddenly terminable.
    pub fn kill(&mut self) {
        if let Some(c) = &mut self.child {
            if self.status == Status::Running {
                if !self.sudden {
                    let _ = self.chan.send(&ToApp::Quit.encode(), &[]);
                    for _ in 0..5 {
                        if matches!(c.try_wait(), Ok(Some(_))) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(4));
                    }
                }
                self.sigkill();
            }
        } else {
            self.status = Status::Exited(0);
        }
    }

    fn sigkill(&mut self) {
        #[cfg(feature="desktop-wayland")]
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_some(){crate::bare::app_manager::terminate(&self.id);}
        self.kill_child();
    }

    /// Kills this host's own process, leaving any other instance of the app alone.
    fn kill_child(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let st = c.wait().ok();
            self.status = Status::Exited(st.and_then(|s| s.code()).unwrap_or(137));
        }
    }

    /// Starts quitting without blocking: a suddenly terminable app is killed now; any other gets `Quit` and
    /// `QUIT_GRACE_MS` to save and exit (`quit_poll` finishes it). True when the app is already gone.
    pub fn begin_quit(&mut self, now: f64) -> bool {
        if !self.running() {
            return true;
        }
        match quit_plan(self.sudden) {
            QuitPlan::KillNow => self.sigkill(),
            QuitPlan::QuitThenKill(ms) => {
                let _ = self.chan.send(&ToApp::Quit.encode(), &[]);
                self.quit_at = Some(now + ms);
            }
        }
        !self.running()
    }

    /// Advances a pending quit: true once the app has exited or the deadline killed it.
    pub fn quit_poll(&mut self, now: f64) -> bool {
        if !self.running() {
            return true;
        }
        let _ = self.pump(); // keep the socket drained so the app is never stuck writing while it saves
        if !self.running() {
            return true;
        }
        if self.quit_at.is_some_and(|at| now >= at) {
            // Only this instance: the app may have been reopened since it was asked to quit, and the whole-app
            // cgroup kill would take the new instance with it (a black window). bwrap runs the app in its own pid
            // namespace with --die-with-parent, so killing our child ends the whole tree.
            self.kill_child();
        }
        !self.running()
    }

    /// The monotonic ms at which a pending quit turns into a kill.
    pub fn quit_deadline(&self) -> Option<f64> {
        self.quit_at.filter(|_| self.running())
    }
}

/// How long an app that is not suddenly terminable gets to save and exit after `Quit`.
pub const QUIT_GRACE_MS: f64 = 1500.0;

/// What quitting an app does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QuitPlan {
    /// SIGKILL at once, no `Quit` round trip.
    KillNow,
    /// Send `Quit`, kill after this many ms if it is still there.
    QuitThenKill(f64),
}

/// The decision, pure: terminable apps die now, the rest are asked and given the grace period.
pub fn quit_plan(terminable: bool) -> QuitPlan {
    if terminable { QuitPlan::KillNow } else { QuitPlan::QuitThenKill(QUIT_GRACE_MS) }
}

impl Drop for AppHost {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The built-in Calculator's package id (Search answers open it when installed).
pub const CALCULATOR: &str = "is.olibuijr.calculator";

/// An installed app's icon at `size` px, from its package (`None` for built-in ids).
pub fn icon_image(id: &str, size: usize) -> Option<crate::gfx::Image> {
    let registered = installed();
    let app = registered.iter().find(|a| a.manifest.id == id)?;
    let img = crate::imgdec::decode(&std::fs::read(app.icon_path()).ok()?)?;
    Some(if img.w == size { img } else { img.resized(size, size) })
}

use super::{home_zone, keyboard::Out, AppId, Event, Screen, Shell};

/// The system accessibility settings an app needs to draw itself (`bifrost_proto::appearance` bits).
pub fn appearance_flags() -> u32 {
    use bifrost_proto::appearance::*;
    use super::accessibility as a11y;
    [(a11y::increase_contrast(), INCREASE_CONTRAST), (a11y::reduce_transparency(), REDUCE_TRANSPARENCY), (a11y::button_shapes(), SHOW_BORDERS), (a11y::bold_text(), BOLD_TEXT), (bifrost_render::appearance::target_is_light(), LIGHT), (vibrancy_available(), VIBRANCY), (vibrancy_available(), MATERIALS)]
        .into_iter()
        .fold(0, |f, (on, bit)| if on { f | bit } else { f })
}

/// Behind-window vibrancy works only when the GPU presenter blurs the desktop under windows, and not with Reduce
/// Transparency; on the CPU/SHM path windows stay opaque.
pub fn vibrancy_available() -> bool {
    bifrost_render::glass::gpu() && !super::accessibility::reduce_transparency()
}

impl Shell {
    pub(super) fn ext_cfg(&self) -> Configure {
        let (w, h) = self.ext_fg.and_then(|i| self.desk_app_size(super::AppId::External(i))).unwrap_or((self.w, self.h));
        Configure {
            w: w as u32,
            h: h as u32,
            scale: self.layout.s,
            safe_top: if self.desk_active() { 0.0 } else { self.layout.status_h as f32 },
            safe_bottom: if self.ext_fg.is_some() && self.keyboard.shown() { home_zone(&self.ui) + self.keyboard.h() } else { 0.0 },
            dark: !bifrost_render::appearance::target_is_light(),
            text_scale: self.ui.text_scale,
            reduce_motion: super::accessibility::reduce_motion(),
        }
    }

    /// A notification, share or drag message from the app in `slot` (`ext_tick` collects them).
    fn rich_msg(&mut self, slot: u8, msg: ToShell, t: f64) {
        match msg {
            ToShell::Share { request, items, anchor } => self.share_begin(slot, request, items, anchor),
            m @ (ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }) => {
                if let Some(a) = installed().get(slot as usize) {
                    super::spotindex::apply(&a.manifest.id, m);
                }
            }
            ToShell::DistPost { name, user_info } => {
                let m = bifrost_proto::ToApp::DistNotify { name, user_info };
                for other in 0..self.ext.len() {
                    if other != slot as usize {
                        self.ext_send(other, &m);
                    }
                }
            }
            m @ (ToShell::DragBegin { .. } | ToShell::DragRegister { .. } | ToShell::DragReply { .. } | ToShell::DragPerformed { .. } | ToShell::DragCancel { .. }) => self.dnd_msg(slot, m),
            m @ (ToShell::LoginItem { .. } | ToShell::StatusItemSet { .. } | ToShell::StatusItemRemove { .. }) => self.extras_msg(slot, m),
            other => self.usernotif_msg(slot, other, t),
        }
    }

    /// Starts the installed app in `slot` unless it is already running.
    pub(super) fn ext_open(&mut self, slot: usize) {
        if self.ext.len() <= slot {
            self.ext.resize_with(slot + 1, || None);
        }
        let registered = installed();
        let Some(app) = registered.get(slot) else { return };
        if let Some(h) = self.ext[slot].as_mut().filter(|h| h.running()) {
            if !h.is_outdated(&app.exec_path()) {
                return;
            }
            // The package was updated while the app was open: quit the old process and start the new version.
            h.kill();
            self.ext[slot] = None;
        }
        // An instance still quitting must be gone before the new one starts (one process per app, like Golden Gate).
        for h in self.dying.iter_mut().filter(|h| h.id == app.manifest.id) {
            h.kill_child();
        }
        self.dying.retain(|h| h.running());
        let data = root().join("appdata").join(&app.manifest.id);
        migrate_legacy(&root(), &app.manifest.id, &data);
        match AppHost::spawn(&app.manifest.id, &app.exec_path(), &data, &root(), self.ext_cfg()) {
            Ok(h) => {
                self.ext[slot] = Some(h);
                self.usernotif_started(slot);
                self.extras_started(slot);
            }
            Err(e) => {
                bifrost_core::log::shell_logger("apphost").error(format!("cannot start {}: {e}", app.manifest.id));
                // never leave an empty (black) window behind a launch that failed
                #[cfg(feature = "desktop-wayland")]
                self.desk_close_app(AppId::External(slot as u8));
            }
        }
    }

    pub(super) fn ext_close(&mut self, slot: usize) {
        if let Some(mut h) = self.ext.get_mut(slot).and_then(Option::take) {
            // Quit now, reaped by `ext_tick` (kill after the grace period); the UI thread never waits
            if !h.begin_quit(bifrost_proto::sys::now_ms()) {
                self.dying.push(h);
            }
        }
        if self.ext_fg == Some(slot as u8) {
            self.ext_fg = None;
        }
        self.extras_app_gone(slot as u8);
    }

    /// Shutdown, logout, restart: terminable apps are killed at once, the rest get `Quit` together and one
    /// shared deadline. Blocks (bounded by `QUIT_GRACE_MS`) because the machine is going down anyway.
    pub fn stop_apps(&mut self) {
        let start = bifrost_proto::sys::now_ms();
        let mut all: Vec<AppHost> = self.dying.drain(..).collect();
        all.extend(self.ext.iter_mut().filter_map(Option::take));
        all.retain_mut(|h| !h.begin_quit(start));
        while !all.is_empty() {
            let now = bifrost_proto::sys::now_ms();
            all.retain_mut(|h| !h.quit_poll(now));
            if now >= start + QUIT_GRACE_MS {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Sends `m` to the app in `slot` if it is running.
    pub(super) fn ext_send(&mut self, slot: usize, m: &bifrost_proto::ToApp) {
        if let Some(h) = self.ext.get_mut(slot).and_then(Option::as_mut) {
            h.send(m);
        }
    }

    pub(super) fn ext_has_frame(&self, slot: usize) -> bool {
        self.ext.get(slot).and_then(Option::as_ref).is_some_and(|h| h.frame().is_some())
    }

    pub(super) fn ext_touch(&mut self, slot: usize, ev: Event, t: f64) {
        let top = self.kb_top();
        match ev {
            Event::Down(x, y) if self.keyboard.shown() && y >= top => {
                self.kb_touch = true;
                self.keyboard.down(x, y - top, t);
                return;
            }
            Event::Move(x, y) if self.kb_touch => {
                self.keyboard.moved(x, y - top);
                return;
            }
            Event::Up(x, y) if self.kb_touch => {
                self.kb_touch = false;
                if let Some(out) = self.keyboard.up(x, y - top, t) {
                    self.ext_type_out(out, t);
                }
                self.dirty = true;
                return;
            }
            _ => {}
        }
        let Some(Some(h)) = self.ext.get_mut(slot) else { return };
        let (kind, x, y) = match ev {
            Event::Down(x, y) => (TouchKind::Down, x, y),
            Event::Move(x, y) => (TouchKind::Move, x, y),
            Event::Up(x, y) => (TouchKind::Up, x, y),
            _ => return,
        };
        h.touch(kind, 0, x, y, t as u32);
    }

    /// Routes the shell keyboard's output to the foreground external app.
    /// Sends a Ctrl/Cmd chord to the external app in front. False when no external app is in front.
    pub(super) fn ext_chord(&mut self, key: u32, mods: u32) -> bool {
        match self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            Some(Some(h)) => {
                // a key equivalent of the app's own menu is a menu action, not a key (`performKeyEquivalent:`)
                if let Some(id) = super::evkeys::char_for(key as u16, false).and_then(|c| super::desktop_bar::app_menu_chord(&h.app_menus, c, mods)) {
                    h.send(&bifrost_proto::ToApp::MenuAction(id));
                    return true;
                }
                h.key(KeyInput::chord(key, mods));
                true
            }
            _ => false,
        }
    }

    pub(super) fn ext_type_out(&mut self, out: Out, t: f64) {
        if out == Out::Hide {
            self.ext_set_keyboard(false, t);
            return;
        }
        let key = match out {
            Out::Text(s) => KeyInput::Text(s),
            Out::Backspace => KeyInput::Code(8),
            Out::Enter => KeyInput::Code(13),
            Out::Named("Tab") => KeyInput::Code(9),
            Out::Named("Left") => KeyInput::Code(37),
            Out::Named("Up") => KeyInput::Code(38),
            Out::Named("Right") => KeyInput::Code(39),
            Out::Named("Down") => KeyInput::Code(40),
            // Ctrl+Z and Ctrl+Shift+Z, as the keyboard sends them to an external app
            Out::Named("Undo") => KeyInput::chord(44, CHORD_CTRL),
            Out::Named("Redo") => KeyInput::chord(44, CHORD_CTRL | CHORD_SHIFT),
            // Back in an external app: the app closes what it has open
            Out::Named("Escape") => KeyInput::Code(1),
            _ => return,
        };
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.key(key);
        }
    }

    /// The foreground app's field traits to the keyboard: the return key label (`ReturnKeyType::basic`). Secure and
    /// content kinds are kept for the keyboard app's layouts once it draws them (docs/bifrost-ui.md, gaps).
    fn ext_apply_traits(&mut self, t: bifrost_proto::InputTraits) {
        self.keyboard.set_return_key(super::textinput::return_key(t));
    }

    /// Candidate text the keyboard app produced goes to the foreground field as marked text (`SetMarkedText`,
    /// `UnmarkText`). With no external app in front it is dropped, so it cannot arrive in the next one.
    fn ext_forward_composition(&mut self) {
        let comp = super::keyboard::take_composition();
        if comp.is_empty() {
            return;
        }
        let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) else { return };
        for c in comp {
            h.send(&super::textinput::to_app(c));
        }
    }

    /// The caret of the foreground app's field in screen px `(x, y, w, h)`, from its `CaretRect` (window points) and
    /// the window body; for candidate and keyboard popups.
    pub(super) fn ext_caret_screen(&self) -> Option<(f32, f32, f32, f32)> {
        let n = self.ext_fg?;
        let h = self.ext.get(n as usize)?.as_ref()?;
        let (cx, cy, cw, ch) = h.caret_rect?;
        let m = self.desk_metrics();
        let (ox, oy) = match self.desk.as_ref().and_then(|d| d.index_of(super::AppId::External(n)).map(|i| d.wins[i].body(&m))) {
            Some(b) => (b.x as f32, b.y as f32),
            None => (0.0, 0.0),
        };
        let s = m.pt;
        Some((ox + cx * s, oy + cy * s, cw * s, ch * s))
    }

    /// Shows or hides the keyboard over the foreground app and tells it the new safe area.
    fn ext_set_keyboard(&mut self, on: bool, t: f64) {
        // a desktop has a hardware keyboard: physical keys reach the app, no docked one
        self.set_keyboard(on && !self.desk_active(), t);
        let cfg = self.ext_cfg();
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.configure(cfg);
        }
        self.dirty = true;
    }

    /// Whether the keyboard is on screen over an external app (the damage-only blit must not run).
    pub(super) fn ext_kb_visible(&self) -> bool {
        matches!(self.screen, Screen::App(AppId::External(_))) && (self.keyboard.shown() || self.keyboard.animating())
    }

    /// The running apps' sockets: the main loop wakes the moment a frame, release or request arrives
    /// (`ext_tick` drains them every pass), instead of polling every 8 ms.
    pub(super) fn ext_fds(&self) -> Vec<std::ffi::c_int> {
        self.ext.iter().flatten().filter(|h| h.running()).map(|h| h.fd()).collect()
    }

    /// Drains every running app's socket; sends Suspend/Resume when the foreground changes
    /// and Configure when the screen size changed.
    pub(super) fn ext_tick(&mut self, t: f64) {
        self.login_items_launch();
        let now = bifrost_proto::sys::now_ms();
        self.dying.retain_mut(|h| !h.quit_poll(now));
        let fg = match self.screen {
            Screen::App(AppId::External(n)) => Some(n),
            _ => None,
        };
        if fg != self.ext_fg {
            if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
                h.suspend();
            }
            let wants = fg.and_then(|n| self.ext.get(n as usize)).and_then(Option::as_ref).is_some_and(|h| h.wants_keyboard);
            // leaving an app, or opening one that has no text field focused (the Terminal's
            // keyboard must not stay up over News)
            if (self.ext_fg.is_some() || (fg.is_some() && !wants)) && self.keyboard.shown() {
                self.set_keyboard(false, t);
            }
            self.ext_fg = fg;
            if let Some(t) = fg.and_then(|n| self.ext.get(n as usize)).and_then(Option::as_ref).map(|h| h.input_traits) {
                self.ext_apply_traits(t);
            }
            if wants {
                self.set_keyboard(true, t);
            }
            let cfg = self.ext_cfg();
            if let Some(Some(h)) = fg.and_then(|n| self.ext.get_mut(n as usize)) {
                h.resume();
                if wants {
                    h.configure(cfg);
                }
            }
        }
        let mut kb = None;
        let mut traits = None;
        let cfg = self.ext_cfg();
        let appearance = appearance_flags();
        let mut clip = None;
        let mut pb_from: Option<usize> = None;
        let mut context=None;
        let mut open_url=None;
        let mut badge_changed=false;
        let mut notifications = Vec::new();
        let mut rich_msgs = Vec::new();
        #[cfg(feature = "desktop-wayland")]
        let mut docs = Vec::new();
        let mut died = Vec::new();
        for (slot, h) in self.ext.iter_mut().enumerate() {
            let Some(h) = h else { continue };
            h.sync_appearance(appearance);
            if fg == Some(slot as u8) && h.running() && h.size() != (cfg.w as usize, cfg.h as usize) {
                h.configure(cfg);
            }
            let p = h.pump();
            if p.died {
                died.push(slot as u8);
            }
            if (p.frame || p.died) && fg == Some(slot as u8) {
                self.dirty = true;
            }
            for r in p.requests {
                match r {
                    ToShell::ContextMenu{x,y,items} if fg==Some(slot as u8)=>{if x.is_finite()&&y.is_finite()&&!items.is_empty()&&items.len()<=32 {context=Some((slot as u8,x,y,items));}},
                    ToShell::WantsKeyboard { on } if fg == Some(slot as u8) => kb = Some(on),
                    ToShell::InputTraits(t) if fg == Some(slot as u8) => traits = Some(t),
                    ToShell::SetClipboard { text } => clip = Some(text),
                    ToShell::PasteboardSet { items } => { if let Some(t) = super::pasteboard::set(items) { clip = Some(t); } pb_from = Some(slot); }
                    #[cfg(feature="desktop-wayland")]
                    ToShell::RemoteSharing { on } => {
                        static BUSY: std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(false);
                        if BUSY.swap(true,std::sync::atomic::Ordering::Acquire){continue;}
                        let id=h.id.clone();let registered=installed();
                        if let Some(package)=registered.iter().find(|a|a.manifest.id==id) {
                            let permission=crate::bare::app_policy::Policy::from_permissions(&id,&package.manifest.permissions);
                            let dir=package.dir.clone();crate::bare::sys::spawn_named("app-sharing", move||{
                                let result=permission.and_then(|policy|crate::bare::app_policy::remote_sharing(&id,policy,on,&dir));
                                if let Err(e)=result{eprintln!("BifrOSt broker denied/failed {} Remote request: {e}",id);}
                                BUSY.store(false,std::sync::atomic::Ordering::Release);
                            });
                        } else {BUSY.store(false,std::sync::atomic::Ordering::Release);}
                    }
                    ToShell::Notify { title, body } => {
                        let permitted = installed().get(slot).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "notifications"));
                        if permitted && h.last_notification.is_none_or(|last| t - last >= 1000.) {
                            h.last_notification = Some(t);
                            notifications.push(super::notify::Notification::new(super::AppId::External(slot as u8), title, body, self.wall.now(t)));
                        }
                    }
                    ToShell::DocumentState { title, edited, represented } => {
                        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
                        let (title, represented) = (clean(&title), clean(&represented));
                        if h.doc_title != title || h.doc_edited != edited || h.doc_file != represented {
                            h.doc_title = title;
                            h.doc_edited = edited;
                            h.doc_file = represented;
                            badge_changed = true;
                        }
                    }
                    ToShell::SetBadge { label } => {
                        let permitted = installed().get(slot).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "notifications"));
                        let label = clean_badge(&label);
                        if permitted && h.badge != label {
                            h.badge = label;
                            badge_changed = true;
                        }
                    }
                    ToShell::Keychain { request, op } => {
                        // identity is the app the shell launched (`h.id`), never anything the app sent
                        let permitted = installed().get(slot).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == "keychain"));
                        let (status, value) = if !permitted {
                            (bifrost_proto::KeychainStatus::NotPermitted, Vec::new())
                        } else {
                            match crate::keychain::Store::open(&root(), &h.id).and_then(|s| s.apply(&op)) {
                                Ok(v) => (bifrost_proto::KeychainStatus::Ok, v),
                                Err(e) => (e.status(), Vec::new()),
                            }
                        };
                        h.send(&ToApp::KeychainReply { request, status, value });
                    }
                    ToShell::OpenUrl { url } => open_url=Some(url),
                    r @ ToShell::Share { .. } if fg == Some(slot as u8) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::DragBegin { .. } | ToShell::DragRegister { .. } | ToShell::DragReply { .. } | ToShell::DragPerformed { .. } | ToShell::DragCancel { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::LoginItem { .. } | ToShell::StatusItemSet { .. } | ToShell::StatusItemRemove { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ ToShell::DistPost { .. } => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::NotifyCategories(_) | ToShell::NotifyRequest { .. } | ToShell::NotifyRemove { .. } | ToShell::NotifyQuery { .. }) => rich_msgs.push((slot as u8, r)),
                    #[cfg(feature = "desktop-wayland")]
                    r @ (ToShell::DocOpen { .. } | ToShell::DocOpenEx { .. } | ToShell::DocSave { .. } | ToShell::DocCommit { .. } | ToShell::DocAbort { .. } | ToShell::DocReopen { .. } | ToShell::DocRelease { .. } | ToShell::DocSaveGrant { .. }) => docs.push((slot as u8, r)),
                    _ => {}
                }
            }
        }
        #[cfg(feature = "desktop-wayland")]
        {
            for &slot in &died { self.doc_app_gone(slot); }
            for (slot, r) in docs { self.doc_request(slot, r); }
            self.doc_keep_shown();
        }
        for &slot in &died { self.dnd_app_gone(slot); self.extras_app_gone(slot); }
        // a window whose app has exited must not keep its last frame on screen
        for slot in died { self.desk_close_app(super::AppId::External(slot)); }
        if badge_changed { self.dirty = true; }
        let indexed = rich_msgs.iter().any(|(_, r)| matches!(r, ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }));
        for (slot, r) in rich_msgs { self.rich_msg(slot, r, t); }
        if indexed { super::spotindex::flush(); }
        self.share_tick();
        self.usernotif_tick(t);
        for notification in notifications { self.notify(notification); }
        if let Some(url)=open_url{if let Err(e)=self.desktop_open_url(&url,t){eprintln!("BifrOSt web link: {e}");}}
        if let Some((slot,x,y,items))=context {if let Some(d)=&self.desk {if let Some(i)=d.index_of(super::AppId::External(slot)){let b=d.wins[i].body(&self.desk_metrics());self.desktop_context=Some(super::desktop_bar::ContextMenu{slot,x:x+b.x as f32,y:y+b.y as f32,items});self.desktop_menu=Some(super::desktop_bar::Menu::Context);self.dirty=true;}}}
        if let Some(text) = clip {
            super::clipboard::set_text(&text);
        }
        if let Some(from) = pb_from {
            // typed items go to every other connected app, and to external Wayland clients
            let (change_count, items) = super::pasteboard::snapshot();
            let m = bifrost_proto::ToApp::Pasteboard { change_count, items: items.clone() };
            for slot in 0..self.ext.len() {
                if slot != from { self.ext_send(slot, &m); }
            }
            super::pasteboard::offer_wayland(&items);
        }
        if let Some(on) = kb {
            self.ext_set_keyboard(on, t);
        }
        if let Some(tr) = traits {
            self.ext_apply_traits(tr);
        }
        self.ext_forward_composition();
        #[cfg(feature = "atspi")]
        super::atspi::sync(&mut self.ext, t);
    }

    pub(super) fn ext_draw(&mut self, s: &mut Surface, slot: usize, t: f64) {
        let full = match self.ext.get_mut(slot) {
            Some(Some(h)) if h.frame().is_some() => {
                h.blit(s, true);
                true
            }
            _ => false,
        };
        if !full {
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, super::ui::BLACK);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            let kb_top = self.kb_top();
            self.keyboard.draw(s, &self.ui, kb_top, t);
            let below = (kb_top + self.keyboard.h() + self.keyboard.offset(t)) as usize;
            s.fill_rect(0, below, self.w as usize, self.h as usize, super::keyboard::BG);
        }
    }

    pub(super) fn ext_blit_damage(&mut self, s: &mut Surface, slot: usize, damage: &mut Damage) {
        if let Some(Some(h)) = self.ext.get_mut(slot) {
            let d = h.blit(s, false);
            damage.union(&d);
        }
    }

    pub(super) fn describe_ext(&self) -> String {
        let run: Vec<&str> = self.ext.iter().flatten().filter(|h| h.running()).map(|h| h.id.as_str()).collect();
        format!("apps_running={}\napp_controls={}", run.join(","),self.ext.iter().flatten().map(|h|format!("{}:{}:{:x}",h.id,h.controls.len(),h.commands.0)).collect::<Vec<_>>().join(","))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quit_plan_kills_terminable_apps_and_gives_the_rest_a_deadline() {
        assert_eq!(super::quit_plan(true), super::QuitPlan::KillNow);
        assert_eq!(super::quit_plan(false), super::QuitPlan::QuitThenKill(super::QUIT_GRACE_MS));
    }
    #[test]
    fn intent_tables_do_not_leak_into_the_app_keys() {
        let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\ntitle = \"New Note\"\nshortcut = true\nparams = []\n\n[[intent]]\nid = \"find\"\ntitle = \"Find\"\nparams = [\"text:string\", \"due:date?\"]\n\n[other]\nname = \"ignored\"\n").unwrap();
        assert_eq!((m.id.as_str(), m.name.as_str()), ("a.b", "Notes"));
        assert_eq!(m.intents.len(), 2);
        assert!(m.intents[0].runnable());
        assert!(!m.intents[1].runnable());
        assert_eq!(m.intents[1].params, vec!["text:string", "due:date?"]);
    }

    use super::*;

    #[test]
    fn appearance_is_sent_on_first_contact_and_only_when_it_changes() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let next = |peer: &Channel| match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
            _ => None,
        };
        host.sync_appearance(0);
        assert_eq!(next(&peer), Some(ToApp::Appearance { flags: 0 }), "first contact sends even all-off");
        host.sync_appearance(0);
        assert_eq!(next(&peer), None, "unchanged: nothing sent");
        host.sync_appearance(bifrost_proto::appearance::SHOW_BORDERS);
        assert_eq!(next(&peer), Some(ToApp::Appearance { flags: bifrost_proto::appearance::SHOW_BORDERS }));
    }

    #[test]
    fn the_light_scheme_is_a_flag_and_the_config_dark_is_its_inverse() {
        use bifrost_proto::appearance::LIGHT;
        bifrost_render::appearance::jump_to(true);
        assert_ne!(appearance_flags() & LIGHT, 0, "light scheme sends the LIGHT bit");
        bifrost_render::appearance::jump_to(false);
        assert_eq!(appearance_flags() & LIGHT, 0, "dark scheme clears it");
    }

    #[test]
    fn only_the_keyboard_system_app_may_type_into_the_focused_field() {
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let typed = ToShell::KeyboardOut(bifrost_proto::KeyboardOut::Text("x".into()));
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut other = AppHost::with_channel("is.olibuijr.calculator", channel, None, cfg.clone());
        let mut pumped = Pumped::default();
        other.handle(typed.clone(), vec![], &mut pumped);
        assert!(pumped.requests.is_empty(), "an ordinary app cannot inject keystrokes");
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut keyboard = AppHost::with_channel(KEYBOARD_APP_ID, channel, None, cfg);
        keyboard.handle(typed.clone(), vec![], &mut pumped);
        assert_eq!(pumped.requests, vec![typed]);
    }

    #[test]
    fn a_host_is_outdated_only_when_it_runs_a_different_binary_than_the_installed_one() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let new = Path::new("/var/lib/springboard/apps/x/1.7.0/bin");
        assert!(!host.is_outdated(new), "a test host has no binary of record, so it is never outdated");
        host.exec = PathBuf::from("/var/lib/springboard/apps/x/1.6.0/bin");
        assert!(host.is_outdated(new), "the installed version moved on");
        host.exec = new.to_path_buf();
        assert!(!host.is_outdated(new), "same binary: just focus it");
    }

    #[test]
    fn resize_keeps_old_buffer_dimensions_until_replacement_arrives() {
        let (channel,_peer)=sys::socketpair_pair().unwrap();
        let cfg=Configure {w:4,h:3,scale:1.,safe_top:0.,safe_bottom:0.,dark:true,text_scale:1.,reduce_motion:false};
        let mut host=AppHost::with_channel("fixture",channel,None,cfg.clone());
        let mut pumped=Pumped::default();
        let buffers=|| vec![sys::shm_create("resize-a",48).unwrap(),sys::shm_create("resize-b",48).unwrap()];
        host.handle(ToShell::Buffers{w:4,h:3},buffers(),&mut pumped);
        host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,4,3]]},vec![],&mut pumped);
        let mut resized=cfg; resized.w=8; resized.h=6;
        host.configure(resized);
        assert_eq!(host.size(),(8,6));
        assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((4,3)));
        let mut pixels=vec![0;8*6];
        host.blit(&mut Surface{w:8,h:6,stride:8,px:&mut pixels,y0:0},true);
        host.handle(ToShell::Buffers{w:8,h:6},vec![sys::shm_create("resize-c",192).unwrap(),sys::shm_create("resize-d",192).unwrap()],&mut pumped);
        host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,8,6]]},vec![],&mut pumped);
        assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((8,6)));
    }

    #[test]
    fn manifest_window_size_keys_give_fixed_initial_min_and_max_sizes() {
        let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
        let fixed = Manifest::parse(&format!("{base}window_size = \"fixed:320x520\"\n")).unwrap().size_limits();
        assert_eq!(fixed, SizeLimits { initial: Some((320.0, 520.0)), min: Some((320.0, 520.0)), max: Some((320.0, 520.0)), fixed: true });
        let free = Manifest::parse(&format!("{base}window_size = \"800x600\"\nmin_size = \"400x300\"\nmax_size = \"1200 x 900\"\n")).unwrap().size_limits();
        assert_eq!(free, SizeLimits { initial: Some((800.0, 600.0)), min: Some((400.0, 300.0)), max: Some((1200.0, 900.0)), fixed: false });
        let bad = Manifest::parse(&format!("{base}window_size = \"fixed:0x10\"\nmin_size = \"wide\"\n")).unwrap().size_limits();
        assert_eq!(bad, SizeLimits::default(), "malformed sizes are ignored");
    }

    #[test]
    fn manifest_parses_and_rejects_unsafe_paths() {
        let m = Manifest::parse("# c\nid = \"is.olibuijr.calculator\"\nname = \"Calculator\"\nversion = \"1.0.0\"\nexec = \"calculator\"\nicon = \"icon.png\"\npermissions = [\"storage\", \"audio\"]\n").unwrap();
        assert_eq!((m.id.as_str(), m.exec.as_str(), m.permissions.len()), ("is.olibuijr.calculator", "calculator", 2));
        assert!(Manifest::parse("id = \"a/b\"\nname = \"x\"\nexec = \"x\"").is_none());
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"../x\"").is_none());
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"/bin/sh\"").is_none());
        assert!(Manifest::parse("id = \"a.b\"\nexec = \"x\"").is_none());
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"").is_none(), "an app needs an exec");
        let sys = Manifest::parse("id = \"a.b\"\nname = \"x\"\nkind = \"system\"\nprovides = [\"keyboard\"]").unwrap();
        assert!(sys.is_system() && sys.exec.is_empty() && sys.provides == ["keyboard"]);
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nkind = \"system\"\nexec = \"../x\"").is_none());
    }

    #[test]
    fn scan_hides_system_packages_from_the_home_grid() {
        let root = std::env::temp_dir().join(format!("system-pkgs-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |id: &str, manifest: &str, exec: bool| {
            let d = root.join("apps").join(id).join("current");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("manifest.toml"), manifest).unwrap();
            if exec {
                std::fs::write(d.join("x"), b"ELF").unwrap();
            }
        };
        mk("is.o.calc", "id = \"is.o.calc\"\nname = \"Calc\"\nexec = \"x\"\n", true);
        mk("is.o.kb", "id = \"is.o.kb\"\nname = \"Keyboard\"\nkind = \"system\"\nprovides = [\"keyboard\"]\n", false);
        mk("is.o.kb2", "id = \"is.o.kb2\"\nname = \"Kb2\"\nkind = \"system\"\nexec = \"x\"\n", true);
        let ids: Vec<String> = scan(&root).into_iter().map(|a| a.manifest.id).collect();
        assert_eq!(ids, ["is.o.calc"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}

/// Where a stock app kept its data while it was built into the shell, relative to the shell's
/// data dir, and where the packaged app reads it, relative to its own data dir.
fn legacy_data(id: &str) -> &'static [(&'static str, &'static str)] {
    match id {
        "is.olibuijr.notes" => &[("Notes", "")],
        "is.olibuijr.games" => &[("Games", "")],
        "is.olibuijr.news" => &[("News", "")],
        "is.olibuijr.music" => &[("Music", "")],
        "is.olibuijr.weather" => &[("Weather", "")],
        "is.olibuijr.books" => &[("Books", "")],
        "is.olibuijr.podcasts" => &[("Podcasts", "")],
        "is.olibuijr.voicememos" => &[("VoiceMemos", "")],
        "is.olibuijr.calendar" => &[("Calendar", "")],
        "is.olibuijr.contacts" => &[("Contacts", "Contacts")],
        "is.olibuijr.reminders" => &[("reminders.tsv", "reminders.tsv")],
        "is.olibuijr.clock" => &[("clock.ini", "clock.ini"), ("clock-timers.ini", "clock-timers.ini"), ("clock-stopwatch.ini", "clock-stopwatch.ini"), ("world-cities.ini", "world-cities.ini")],
        "is.olibuijr.shortcuts" => &[("shortcuts.tsv", "shortcuts.tsv")],
        _ => &[],
    }
}

/// Moves a stock app's old data into its package data dir once, before its first launch, so
/// nothing is lost when an app leaves the shell. A data dir that already exists is left alone.
pub fn migrate_legacy(root: &Path, id: &str, data: &Path) {
    let moves = legacy_data(id);
    if data.exists() || moves.is_empty() {
        return;
    }
    if std::fs::create_dir_all(data).is_err() {
        return;
    }
    for (old, new) in moves {
        let from = root.join(old);
        if !from.exists() {
            continue;
        }
        let ok = if new.is_empty() {
            // the directory's contents become the app's data dir
            std::fs::read_dir(&from).map(|rd| rd.flatten().all(|e| std::fs::rename(e.path(), data.join(e.file_name())).is_ok())).unwrap_or(false)
        } else {
            std::fs::rename(&from, data.join(new)).is_ok()
        };
        eprintln!("springboard: moved {} into {} for {id}: {}", from.display(), data.display(), if ok { "ok" } else { "FAILED" });
    }
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    #[test]
    fn stock_app_data_moves_once_into_the_package_dir() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Notes")).unwrap();
        std::fs::write(root.join("Notes/a.txt"), "hello").unwrap();
        std::fs::create_dir_all(root.join("Contacts")).unwrap();
        std::fs::write(root.join("Contacts/contacts.tsv"), "x").unwrap();
        std::fs::write(root.join("reminders.tsv"), "r").unwrap();
        std::fs::create_dir_all(root.join("VoiceMemos")).unwrap();
        std::fs::write(root.join("VoiceMemos/Voice Memo 1.wav"), "w").unwrap();
        for id in ["is.olibuijr.notes", "is.olibuijr.contacts", "is.olibuijr.reminders", "is.olibuijr.voicememos"] {
            migrate_legacy(&root, id, &root.join("appdata").join(id));
        }
        assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.notes/a.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.contacts/Contacts/contacts.tsv")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.reminders/reminders.tsv")).unwrap(), "r");
        assert_eq!(std::fs::read_to_string(root.join("appdata/is.olibuijr.voicememos/Voice Memo 1.wav")).unwrap(), "w");
        // a second run with fresh old data does not overwrite what the app already owns
        std::fs::write(root.join("Notes/b.txt"), "later").unwrap();
        migrate_legacy(&root, "is.olibuijr.notes", &root.join("appdata/is.olibuijr.notes"));
        assert!(!root.join("appdata/is.olibuijr.notes/b.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod registry_update_tests {
    use super::*;
    #[test]
    fn review_registry_update_preserves_slots_and_old_snapshot_ownership() {
        let app=|id:&str,version:&str| Installed {manifest:Manifest{id:id.into(),version:version.into(),exec:"app".into(),..Manifest::default()},dir:PathBuf::from(format!("/apps/{id}/{version}"))};
        let old:std::sync::Arc<[Installed]>=vec![app("a","1"),app("b","1")].into();
        let current=merge_installed(&old,vec![app("b","2"),app("c","1"),app("a","2")]);
        assert_eq!(current.iter().map(|a|a.manifest.id.as_str()).collect::<Vec<_>>(),vec!["a","b","c"]);
        assert_eq!(current[0].exec_path(),PathBuf::from("/apps/a/2/app"));
        assert_eq!(current[1].manifest.version,"2");
        assert_eq!(old[0].manifest.version,"1");
        assert_eq!(merge_installed(&current,current.clone()),current);
    }
}

/// A dock badge as shown: control characters removed, trimmed, at most four characters
/// ("99+" fits, a long text is cut).
pub(super) fn clean_badge(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(4).collect()
}

#[cfg(test)]
mod badge_tests {
    use super::clean_badge;

    #[test]
    fn badges_are_short_single_line_labels() {
        assert_eq!(clean_badge("12"), "12");
        assert_eq!(clean_badge("  7 \n"), "7");
        assert_eq!(clean_badge("99+"), "99+");
        assert_eq!(clean_badge("123456"), "1234");
        assert_eq!(clean_badge("a\u{0}b\tc"), "abc");
        assert_eq!(clean_badge(""), "");
        assert_eq!(clean_badge("éééééé"), "éééé", "cut by characters, not bytes");
    }
}
