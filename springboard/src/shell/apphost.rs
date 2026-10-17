//! The shell side of the app protocol (docs/apps.md): installed-app registry,
//! spawning an app over a `SOCK_SEQPACKET` socketpair, receiving its shared
//! frame buffers and presenting them as the app surface.
//!
//! The shell keeps the newest presented buffer mapped and held (it is the
//! switcher card and the crash screenshot) and releases the older one when a
//! newer frame arrives, so the app always has exactly one free buffer.

use super::app_window_table::WindowTable;
use crate::gfx::{Damage, Surface};
use bifrost_proto::sys::{self, Channel, Map, Recv};
use bifrost_proto::{Configure, KeyInput, ToApp, ToShell, TouchKind, CHORD_CTRL, CHORD_SHIFT, SDK_VERSION};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::collections::VecDeque;
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
    /// `keychain_groups = ["is.example.group"]`: keychain access groups the app shares items through (the
    /// `keychain-groups` service). Honoured only with the `keychain` permission; see `services::keychain_groups`.
    pub keychain_groups: Vec<String>,
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
    /// `animation_behavior = "default" | "document" | "utility" | "alert" | "none"`: which window animations play.
    pub animation_behavior: String,
    pub wayland_app_id: String,
    /// What the app opens, `type` or `type:rank` (rank owner, default, alternate, none; default is `default`): `handles = ["public.png:owner", "public.text"]`.
    pub handles: Vec<String>,
    /// URL schemes the app opens: `schemes = ["https"]`.
    pub schemes: Vec<String>,
    /// App Intents declared as `[[intent]]` tables (`shell::intents`); Search lists the shortcuts.
    pub intents: Vec<super::intents::ManifestIntent>,
    /// `[[quick_action]]` tables: the static Home Screen quick actions (`shell::quick_actions`).
    pub quick_actions: Vec<bifrost_ui::quick_actions::ShortcutItem>,
}

impl Manifest {
    /// The app draws to the top edge of its window (`NativeWindowStyleMaskFullSizeContentView`).
    pub fn full_size_content(&self) -> bool {
        self.window == "full-size"
    }
    pub fn animation_behavior(&self) -> super::window_fx::AnimationBehavior {
        super::window_fx::AnimationBehavior::parse(self.animation_behavior.trim())
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
                "keychain_groups" => m.keychain_groups = strs(),
                "entitlements" => m.entitlements = strs(),
                "provides" => m.provides = strs(),
                "handles" => m.handles = strs(),
                "schemes" => m.schemes = strs(),
                "kind" => m.kind = unquote(v),
                "window" => m.window = unquote(v),
                "window_size" => m.window_size = unquote(v),
                "min_size" => m.min_size = unquote(v),
                "max_size" => m.max_size = unquote(v),
                "animation_behavior" => m.animation_behavior = unquote(v),
                "wayland_app_id" => m.wayland_app_id = unquote(v),
                _ => {}
            }
        }
        m.quick_actions = super::quick_actions::parse_static(text);
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

pub(super) fn unquote(s: &str) -> String {
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

/// How long an app whose socket closed may take to exit before it is SIGKILLed (a core dump runs in that time).
const LATE_REAP_MS: u64 = 3000;
/// (app id, exit status) of apps that crashed after their socket had closed; the shell posts them (`late_crashes`).
static LATE_CRASHES: std::sync::Mutex<Vec<(String, i32)>> = std::sync::Mutex::new(Vec::new());

fn late_reap(mut c: Child, id: String) {
    let _ = std::thread::Builder::new().name("app-reap".into()).spawn(move || {
        let status = |st: std::process::ExitStatus| st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0));
        let mut code = None;
        for _ in 0..LATE_REAP_MS / 20 {
            if let Ok(Some(st)) = c.try_wait() {
                code = Some(status(st));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let code = code.unwrap_or_else(|| {
            let _ = c.kill();
            c.wait().ok().map_or(137, status)
        });
        bifrost_core::log::shell_logger("apphost").notice(format!("app {id} exited {code} after its socket closed"));
        if super::crash_alert::is_crash(code) {
            LATE_CRASHES.lock().unwrap_or_else(|e| e.into_inner()).push((id, code));
        }
    });
}

/// Crashes `late_reap` saw since the last call.
pub fn late_crashes() -> Vec<(String, i32)> {
    std::mem::take(&mut *LATE_CRASHES.lock().unwrap_or_else(|e| e.into_inner()))
}
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

static INCOMPATIBLE: std::sync::Mutex<Vec<(String, PathBuf)>> = std::sync::Mutex::new(Vec::new());

/// Records that the binary at `exec` for app `id` speaks another SDK protocol.
fn remember_incompatible(id: &str, exec: &Path) {
    if exec.as_os_str().is_empty() {
        return;
    }
    if let Ok(mut g) = INCOMPATIBLE.lock() {
        if !g.iter().any(|(i, e)| i == id && e == exec) {
            g.push((id.to_string(), exec.to_path_buf()));
        }
    }
}

/// True when this exact binary was already found incompatible with the shell's SDK protocol.
fn is_known_incompatible(id: &str, exec: &Path) -> bool {
    INCOMPATIBLE.lock().map(|g| g.iter().any(|(i, e)| i == id && e == exec)).unwrap_or(false)
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

/// One extra window's shared buffers and what the shell holds of its frames (the same roles as the main window's
/// `bufs`, `shown`, `release_after_blit` and `pending`).
struct WinBufs {
    generation: u32,
    bufs: Vec<Buf>,
    size: (usize, usize),
    shown: Option<usize>,
    release_after_blit: Option<usize>,
    pending: Damage,
}

/// Packets queued for an app whose socket is full; past this the app is killed.
const MAX_BACKLOG: usize = 256;
/// Messages read from one app per shell pass: a flood past this waits for the next pass instead of starving the rest.
const MAX_PUMP: usize = 256;
/// How long (ms) an app may leave its socket full before it is killed.
const STALL_MS: f64 = 5000.0;

pub struct AppHost {
    pub id: String,
    /// The binary this process was started from (empty for a host on a test channel).
    exec: PathBuf,
    /// The appearance flags the app last received (`u32::MAX` until the first send).
    appearance_sent: u32,
    /// The input source the app last received (`ToApp::InputSource`); `None` until the first send.
    input_japanese: Option<bool>,
    child: Option<Child>,
    chan: Channel,
    bufs: Vec<Buf>,
    /// Generation of the current `Buffers` set (the app's counter); `Release` carries it so the app can ignore a late one.
    generation: u32,
    /// Packets the app's full socket would not take yet, oldest first (bounded; see `queue`).
    backlog: VecDeque<Vec<u8>>,
    /// Since when (monotonic ms) the socket has been full; a long stall kills the app.
    blocked_since: Option<f64>,
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
    /// the app speaks another SDK protocol (or its packets do not decode): its window shows "This app needs an update"
    pub incompatible: bool,
    pub commands: bifrost_proto::Commands,
    /// the app's Edit menu titles for Undo and Redo (empty: the plain words)
    pub undo_titles: (String, String),
    pub controls: Vec<bifrost_proto::Node>,
    /// The app's own main-menu items (`ToShell::AppMenus`), shown by the desktop bar while the app is in front.
    pub app_menus: Vec<bifrost_proto::AppMenu>,
    /// Hierarchy and state beside `controls` (`ToShell::ControlsTree`); empty means a flat list.
    pub controls_tree: Vec<bifrost_proto::NodeExt>,
    /// Caret, selection and visible range of text elements (`ToShell::AxText`); the AT-SPI `Text` interface reads it.
    pub ax_text: Vec<bifrost_proto::AxText>,
    /// `accessibilityTitle` / `accessibilityHelp` per element (`ToShell::AxMeta`).
    pub ax_meta: Vec<bifrost_proto::AxMeta>,
    /// Announcements waiting for the accessibility bridge (`ToShell::Announce`), oldest first, at most 8.
    pub announcements: Vec<(String, bifrost_proto::AnnouncePriority)>,
    /// The same announcements, waiting for the on-screen banner (`accessibility::banner`); the shell takes them each tick.
    pub announce_banner: Vec<(String, bifrost_proto::AnnouncePriority)>,
    /// The app's custom rotors (`ToShell::AxRotors`), cleaned with `bifrost_ui::ax_rotor::clean`; empty when it declares none.
    pub custom_rotors: Vec<bifrost_ui::ax::AxCustomRotor>,
    /// Guided Access requests (`ToShell::GuidedAccessRequest`: start with restrictions, or end), oldest first, taken by the shell each tick.
    pub guided_requests: Vec<(bool, Vec<bifrost_proto::GuidedRestriction>)>,
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
    /// Progress and attention bounce the app set on its Dock tile (the `dock` service).
    pub tile: super::desktop_dock::TileState,
    /// The items the app set on its Dock menu (the `dock` service's `menu`), shown under the app's own rows.
    pub dock_menu: Vec<super::services::dock_tile::MenuEntry>,
    /// The dynamic Home Screen quick actions the app set (the `quick_actions` service's `set`).
    pub quick_actions: Vec<bifrost_ui::quick_actions::ShortcutItem>,
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
    /// How the shell shows this app (`launch_route` at spawn; a `Buffers` message from a direct app demotes it).
    route: Route,
    /// The touch frames the app gets (`ToApp::Touches`): one tracker per surface, at the app's scale.
    frames: bifrost_ui::touch::TouchTracker,
    /// The scale `frames` was built for (points to pixels), so a new `Configure` at another scale rebuilds it.
    frame_scale: f32,
    /// Set when a direct app demoted to `Route::Shm`; `take_fallback` reads it once so the shell opens its window.
    fallback: bool,
    /// The extra windows the app has open, front to back (`ToShell::WindowOpen` and friends). The desk draws each one.
    pub windows: WindowTable,
    /// Each extra window's buffers, by window id (`ToShell::WindowBuffers`).
    window_bufs: std::collections::BTreeMap<u32, WinBufs>,
}

impl AppHost {
    /// Starts `exec` with the app end of a fresh socketpair and sends `cfg`.
    pub fn spawn(id: &str, exec: &Path, data_dir: &Path, shell_dir: &Path, cfg: Configure) -> std::io::Result<AppHost> {
        Self::spawn_with(id, exec, data_dir, shell_dir, cfg, false)
    }

    /// `spawn`, telling the app whether to restore its saved window state (`BIFROST_RESTORE`, see `bifrost_app::restoration`).
    pub fn spawn_with(id: &str, exec: &Path, data_dir: &Path, shell_dir: &Path, cfg: Configure, restore: bool) -> std::io::Result<AppHost> {
        let route = launch_route();
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
            .env("BIFROST_RESTORE", if restore { "1" } else { "0" })
            // explicit, so the app never picks up a stale BIFROST_DIRECT from the shell's environment
            .env("BIFROST_DIRECT", if route == Route::Direct { "1" } else { "0" })
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
        h.route = route;
        h.send(&ToApp::Configure(cfg));
        Ok(h)
    }

    /// A host on an existing channel (tests drive the app side themselves).
    pub fn with_channel(id: &str, chan: Channel, child: Option<Child>, cfg: Configure) -> AppHost {
        AppHost {
            id: id.to_string(),
            exec: PathBuf::new(),
            appearance_sent: u32::MAX,
            input_japanese: None,
            child,
            chan,
            bufs: Vec::new(),
            generation: 0,
            backlog: VecDeque::new(),
            blocked_since: None,
            size: (cfg.w as usize, cfg.h as usize),
            buffer_size: (cfg.w as usize, cfg.h as usize),
            shown: None,
            release_after_blit: None,
            pending: Damage::new(cfg.w as usize, cfg.h as usize),
            status: Status::Running,
            wants_keyboard: false,
            hello: false,
            incompatible: false,
            commands: bifrost_proto::Commands::default(),
            undo_titles: Default::default(),
            controls: Vec::new(),
            app_menus: Vec::new(),
            controls_tree: Vec::new(),
            ax_text: Vec::new(),
            ax_meta: Vec::new(),
            announcements: Vec::new(),
            announce_banner: Vec::new(),
            custom_rotors: Vec::new(),
            guided_requests: Vec::new(),
            last_notification: None,
            badge: String::new(),
            tile: Default::default(),
            dock_menu: Vec::new(),
            quick_actions: Vec::new(),
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
            route: Route::Shm,
            frames: bifrost_ui::touch::TouchTracker::with_scale(cfg.scale),
            frame_scale: cfg.scale,
            fallback: false,
            windows: WindowTable::new(),
            window_bufs: std::collections::BTreeMap::new(),
        }
    }

    /// The route the shell gave this app: `Direct` apps draw in the window server and expect no frames.
    pub fn route(&self) -> Route {
        self.route
    }

    /// True once after a direct app fell back to the shared-memory path (`Buffers`): the shell must open its window.
    pub fn take_fallback(&mut self) -> bool {
        std::mem::take(&mut self.fallback)
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// The process id of the app, when it runs as a child of the shell (none on a test channel).
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    pub fn running(&self) -> bool {
        self.status == Status::Running
    }

    /// The exit status once the app has gone (`128 + signal` when a signal ended it); None while it runs.
    pub fn exit_code(&self) -> Option<i32> {
        match self.status {
            Status::Exited(code) => Some(code),
            Status::Running => None,
        }
    }

    /// Tells the app the system appearance settings when they differ from what it last got (and on first contact).
    pub fn sync_appearance(&mut self, flags: u32) {
        if self.appearance_sent != flags {
            self.appearance_sent = flags;
            self.send(&ToApp::Appearance { flags });
        }
    }

    /// Tells the app whether the kana (Romaji) input source is active, on first contact and whenever it changes.
    pub fn sync_input_source(&mut self, japanese: bool) {
        if self.input_japanese != Some(japanese) {
            self.input_japanese = Some(japanese);
            self.send(&ToApp::InputSource { japanese });
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
        if !self.running() {
            return;
        }
        // a Move is superseded by the next one, so it is the one thing that may be dropped on a full socket
        let droppable = match m {
            ToApp::Touch { kind: TouchKind::Move, .. } => true,
            // a frame of only moved and stationary touches is superseded by the next one
            ToApp::Touches(ev) => ev.droppable(),
            _ => false,
        };
        self.queue(m.encode(), droppable);
    }

    /// Sends without ever blocking the shell thread (IPC-3): a full socket queues the packet (bounded) and a
    /// stalled app is killed after `STALL_MS`.
    fn queue(&mut self, pkt: Vec<u8>, droppable: bool) {
        self.flush();
        if !self.running() {
            return;
        }
        if self.backlog.is_empty() {
            match self.chan.try_send(&pkt, &[]) {
                Ok(()) => {
                    self.blocked_since = None;
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.blocked_since.get_or_insert_with(sys::now_ms);
                    if self.stalled() {
                        return;
                    }
                }
                Err(_) => {
                    self.reap();
                    return;
                }
            }
        }
        if droppable {
            return;
        }
        if self.backlog.len() >= MAX_BACKLOG {
            self.overloaded();
            return;
        }
        self.backlog.push_back(pkt);
    }

    /// Sends what the socket now takes of the backlog.
    fn flush(&mut self) {
        while let Some(front) = self.backlog.front() {
            match self.chan.try_send(front, &[]) {
                Ok(()) => {
                    self.backlog.pop_front();
                    self.blocked_since = None;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.stalled();
                    return;
                }
                Err(_) => {
                    self.backlog.clear();
                    self.reap();
                    return;
                }
            }
        }
    }

    /// True (and the app killed) once the socket has been full for `STALL_MS`.
    fn stalled(&mut self) -> bool {
        if self.blocked_since.is_some_and(|t| sys::now_ms() - t > STALL_MS) {
            self.overloaded();
            return true;
        }
        false
    }

    fn overloaded(&mut self) {
        eprintln!("springboard: app {} stopped reading its socket; killing", self.id);
        self.backlog.clear();
        self.sigkill();
        self.reap();
    }

    /// Sends `m` with one descriptor (SCM_RIGHTS): the file a document panel granted. The app gets its own copy of the
    /// descriptor; the shell's copy closes when `file` is dropped.
    pub fn send_file(&mut self, m: &ToApp, file: &std::fs::File) {
        use std::os::fd::AsRawFd;
        if !self.running() {
            return;
        }
        match self.chan.try_send(&m.encode(), &[file.as_raw_fd()]) {
            Ok(()) => {}
            // the app is not reading: this grant is dropped rather than blocking the shell (the stall timer kills it)
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.blocked_since.get_or_insert_with(sys::now_ms);
                self.stalled();
            }
            Err(_) => self.reap(),
        }
    }

    pub fn configure(&mut self, cfg: Configure) {
        self.size = (cfg.w as usize, cfg.h as usize);
        if cfg.scale != self.frame_scale {
            self.frames = bifrost_ui::touch::TouchTracker::with_scale(cfg.scale);
            self.frame_scale = cfg.scale;
        }
        self.send(&ToApp::Configure(cfg));
    }

    pub fn touch(&mut self, kind: TouchKind, id: u32, x: f32, y: f32, t_ms: u32) {
        self.send(&ToApp::Touch { kind, id, x, y, t_ms });
    }

    /// Sends one frame of touches (`ToApp::Touches`). Call it before the matching `touch` so the app can drop the
    /// plain stream once it sees frames; locations are in the app's surface pixels, like `touch`.
    pub fn touches(&mut self, ev: bifrost_ui::touch::TouchEvent) {
        self.send(&ToApp::Touches(ev));
    }

    /// Feeds one contact to the app's touch tracker and sends the frame it makes (nothing when the sample changes
    /// nothing). Call it before the matching `touch`, so the app sees the frame first.
    pub fn feed_touch(&mut self, raw: bifrost_ui::touch::Raw, sample: bifrost_ui::touch::Sample, t_ms: f64) {
        if let Some(ev) = self.frames.frame(&[(raw, sample)], t_ms) {
            self.touches(ev);
        }
    }

    /// Takes every touch the app holds away (focus lost, window closed, a system gesture took over): one frame of
    /// `Cancelled` touches. A no-op when no touch is down.
    pub fn cancel_touches(&mut self, t_ms: f64) {
        if let Some(ev) = self.frames.cancel_all(t_ms) {
            self.touches(ev);
        }
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

    /// Reads every queued message without blocking, at most `MAX_PUMP` of them; the rest stay in the socket and
    /// wake the shell's loop again (the socket is in its poll set).
    pub fn pump(&mut self) -> Pumped {
        self.pump_up_to(MAX_PUMP)
    }

    fn pump_up_to(&mut self, max: usize) -> Pumped {
        let mut out = Pumped::default();
        if !self.running() {
            return out;
        }
        self.flush();
        for _ in 0..max {
            match self.chan.recv(false) {
                Ok(Recv::Packet(p, fds)) => {
                    let Ok(msg) = ToShell::decode(&p) else {
                        // an app built for another protocol: say so once, stop it, and keep its window up with a message
                        if !self.incompatible {
                            eprintln!("springboard: app {} sent a packet this shell cannot decode (SDK protocol {SDK_VERSION}); needs an update", self.id);
                            self.mark_incompatible();
                        }
                        continue;
                    };
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
                if sdk_version != SDK_VERSION {
                    eprintln!("springboard: app {} speaks SDK protocol {sdk_version}, this shell {SDK_VERSION}; needs an update", self.id);
                    self.mark_incompatible();
                } else if !self.id.is_empty() && app_id != self.id {
                    eprintln!("springboard: app {} sent Hello sdk {sdk_version} id {app_id}; killing", self.id);
                    self.kill();
                    out.died = true;
                } else {
                    self.hello = true;
                }
            }
            ToShell::Buffers { w, h, generation } => {
                // a direct app that sends shared-memory buffers fell back (no Vulkan device, no layer global)
                let next = route_after_message(self.route, true);
                if next != self.route {
                    self.route = next;
                    self.fallback = true;
                }
                let (w, h) = (w as usize, h as usize);
                let bytes = w * h * 4;
                if fds.len() == 2 && w > 0 && h > 0 && w <= 8192 && h <= 8192 {
                    // IPC-1: a memfd shorter than the claim would SIGBUS the shell on the first blit, and one the app can
                    // still shrink could do the same later: require the size and seal against shrinking first.
                    let safe = fds.iter().all(|f| sys::shm_len(f).is_ok_and(|n| n >= bytes) && sys::seal_shrink(f).is_ok());
                    let maps: Vec<Buf> = if safe { fds.iter().filter_map(|f| Map::new(f, bytes).ok()).map(|map| Buf { map }).collect() } else { Vec::new() };
                    if maps.len() == 2 {
                        self.bufs = maps;
                        self.generation = generation;
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
                            let generation = self.generation;
                            self.send(&ToApp::Release { buf: prev as u32, generation });
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
            ToShell::AxText(list) => self.ax_text=list,
            ToShell::AxMeta(list) => self.ax_meta=list,
            ToShell::Announce { text, priority } => {
                if self.announcements.len() >= 8 {
                    self.announcements.remove(0);
                }
                if self.announce_banner.len() >= 8 {
                    self.announce_banner.remove(0);
                }
                self.announce_banner.push((text.clone(), priority));
                self.announcements.push((text, priority));
            }
            ToShell::AxRotors(list) => self.custom_rotors = bifrost_ui::ax_rotor::clean(list),
            ToShell::GuidedAccessRequest { on, restrictions } => {
                if self.guided_requests.len() >= 4 {
                    self.guided_requests.remove(0);
                }
                self.guided_requests.push((on, restrictions));
            }
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
            ToShell::Activity { options } => {
                self.activity = options;
                crate::bare::sleep::note_app_activity(&self.id, options);
            }
            ToShell::InputAccessory(items) => self.accessory = items.into_iter().take(bifrost_proto::MAX_ACCESSORY_ITEMS).collect(),
            ToShell::WindowOpen { id, title, w, h } => {
                // no room for the window, or its id is taken: the app is told to drop it, so the two sides agree
                if self.windows.open(&self.id, id, &title, w, h).is_err() {
                    self.send(&ToApp::WindowClosed { id });
                }
                out.frame = true;
            }
            ToShell::WindowClose { id } => {
                self.windows.close(&self.id, id);
                self.window_bufs.remove(&id);
                out.frame = true;
            }
            ToShell::WindowTitle { id, title } => {
                self.windows.retitle(&self.id, id, &title);
                out.frame = true;
            }
            ToShell::WindowResize { id, w, h } => {
                let _ = self.windows.resize(&self.id, id, w, h);
                out.frame = true;
            }
            ToShell::WindowBuffers { id, w, h, generation } => self.window_buffers(id, w, h, generation, fds),
            ToShell::WindowFrame { id, buf, damage } => self.window_frame_arrived(id, buf, damage, out),
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

    /// `ToShell::WindowBuffers`: maps one extra window's two buffers, with the main window's size and seal checks. A window
    /// the table does not hold gets nothing.
    fn window_buffers(&mut self, id: u32, w: u32, h: u32, generation: u32, fds: Vec<sys::Owned>) {
        let (w, h) = (w as usize, h as usize);
        let bytes = w * h * 4;
        if fds.len() != 2 || w == 0 || h == 0 || w > 8192 || h > 8192 || !self.window_open(id) {
            return;
        }
        let safe = fds.iter().all(|f| sys::shm_len(f).is_ok_and(|n| n >= bytes) && sys::seal_shrink(f).is_ok());
        if !safe {
            return;
        }
        let bufs: Vec<Buf> = fds.iter().filter_map(|f| Map::new(f, bytes).ok()).map(|map| Buf { map }).collect();
        if bufs.len() == 2 {
            self.window_bufs.insert(id, WinBufs { generation, bufs, size: (w, h), shown: None, release_after_blit: None, pending: Damage::new(w, h) });
        }
    }

    /// `ToShell::WindowFrame`: the window's newest frame. The buffer it replaced is given back once that one is blitted.
    fn window_frame_arrived(&mut self, id: u32, buf: u32, damage: Vec<bifrost_proto::Rect>, out: &mut Pumped) {
        let buf = buf as usize;
        let Some(wb) = self.window_bufs.get_mut(&id) else { return };
        if buf >= wb.bufs.len() {
            return;
        }
        let mut give_back = None;
        if let Some(old) = wb.shown.filter(|&o| o != buf) {
            give_back = wb.release_after_blit.replace(old).filter(|&p| p != old && p != buf);
        }
        wb.shown = Some(buf);
        for r in damage {
            wb.pending.add(r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
        }
        let generation = wb.generation;
        out.frame = true;
        if let Some(prev) = give_back {
            self.send(&ToApp::WindowRelease { id, buf: prev as u32, generation });
        }
    }

    fn window_open(&self, id: u32) -> bool {
        self.windows.of_app(&self.id).iter().any(|w| w.id == id)
    }

    /// The newest frame of one extra window: its pixels, width and height (the desk draws it like the main window).
    #[allow(dead_code)] // the desk draws the windows once it reads the table (docs/bifrost-ui.md, "Several windows per app")
    pub fn window_frame(&self, id: u32) -> Option<(&[u32], usize, usize)> {
        let wb = self.window_bufs.get(&id)?;
        let b = wb.bufs.get(wb.shown?)?;
        Some((b.map.pixels(), wb.size.0, wb.size.1))
    }

    /// The desk copied an extra window's newest frame: the buffer that frame replaced goes back to the app.
    #[allow(dead_code)]
    pub fn window_blitted(&mut self, id: u32) {
        let Some(wb) = self.window_bufs.get_mut(&id) else { return };
        wb.pending.clear();
        let Some(old) = wb.release_after_blit.take() else { return };
        let generation = wb.generation;
        self.send(&ToApp::WindowRelease { id, buf: old as u32, generation });
    }

    /// The shell's close control on one of the app's extra windows: the table drops it and the app is told. False when
    /// the window is not open.
    #[allow(dead_code)]
    pub fn close_window_from_shell(&mut self, id: u32) -> bool {
        if self.windows.close(&self.id, id).is_none() {
            return false;
        }
        self.window_bufs.remove(&id);
        self.send(&ToApp::WindowClosed { id });
        true
    }

    /// The shell resized one of the app's extra windows (points). False when the window is not open.
    #[allow(dead_code)]
    pub fn configure_window(&mut self, id: u32, w: f32, h: f32) -> bool {
        if self.windows.resize(&self.id, id, w, h).is_err() {
            return false;
        }
        self.send(&ToApp::WindowConfigure { id, w, h });
        true
    }

    /// A touch inside one of the app's extra windows, with window-local coordinates in points. False when it is not open.
    #[allow(dead_code)]
    pub fn touch_window(&mut self, id: u32, touch: u32, kind: TouchKind, x: f32, y: f32, t_ms: u32) -> bool {
        if !self.window_open(id) {
            return false;
        }
        self.send(&ToApp::WindowTouch { id, touch, kind, x, y, t_ms });
        true
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
            let generation = self.generation;
            self.send(&ToApp::Release { buf: old as u32, generation });
        }
        done
    }

    fn finish(&mut self, code: i32) {
        self.status = Status::Exited(code);
        // an app that exits takes its extra windows with it
        self.windows.close_app(&self.id);
        self.window_bufs.clear();
        bifrost_core::log::shell_logger("apphost").notice(format!("app {} exited {code}", self.id));
    }

    fn reap(&mut self) {
        if !self.running() {
            return;
        }
        let code = match self.child.as_mut() {
            None => 0,
            Some(c) => match exit_status_soon(c) {
                Ok(Some(st)) => st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)),
                // the socket is gone but the process still runs: it may be finishing a crash (a core dump takes a
                // moment) or a slow quit. A short-lived thread waits for its real status (SIGKILL after LATE_REAP_MS)
                // so a crash still reaches `late_crashes`; the shell goes on at once and counts this exit as clean.
                _ => {
                    if let Some(c) = self.child.take() {
                        late_reap(c, self.id.clone());
                    }
                    0
                }
            },
        };
        self.finish(code);
    }

    /// Stops the app at once: SIGKILL, with a Quit first (and a polite moment) only when it is not suddenly terminable.
    /// Stops an app whose protocol does not match and keeps its host, so the window says why instead of staying black.
    fn mark_incompatible(&mut self) {
        self.incompatible = true;
        remember_incompatible(&self.id, &self.exec);
        self.kill();
    }

    pub fn kill(&mut self) {
        if let Some(c) = &mut self.child {
            if self.status == Status::Running {
                if !self.sudden {
                    // best effort: a full socket (an app that stopped reading) must not block the shell thread
                    let _ = self.chan.try_send(&ToApp::Quit.encode(), &[]);
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
        self.cancel_touches(now);
        match quit_plan(self.sudden) {
            QuitPlan::KillNow => self.sigkill(),
            QuitPlan::QuitThenKill(ms) => {
                // best effort, as in `kill`: the quit deadline kills the app when Quit cannot be queued
                let _ = self.chan.try_send(&ToApp::Quit.encode(), &[]);
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

/// How the shell shows an app it launched (docs/graphics-architecture.md, "Switch-over").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The shared-memory path: the app sends frames over the socket and the shell draws its window.
    Shm,
    /// The direct path: the app draws in the window server. The shell makes no window for it, expects no frames,
    /// and lists it, focuses it and captures its thumbnails through the window server's protocols.
    Direct,
}

impl Route {
    /// Whether the shell expects frames from the app over the socket.
    pub fn expects_frames(self) -> bool {
        self == Route::Shm
    }
}

/// The route for an app the shell is about to launch, pure. Direct only when the shell itself runs with
/// `BIFROST_DIRECT=1` under `bifrost-wm` (its environment and `/etc/bifrost/compositor`); anything else is the
/// shared-memory path, so any other session and missing switches keep the shared-memory path.
pub fn route_for(direct_env: Option<&str>, compositor: Option<&str>) -> Route {
    match (direct_env.map(str::trim), compositor.map(str::trim)) {
        (Some("1"), Some("bifrost-wm")) => Route::Direct,
        _ => Route::Shm,
    }
}

/// `route_for` with the shell's real environment and session file.
pub fn launch_route() -> Route {
    let env = std::env::var("BIFROST_DIRECT").ok();
    let compositor = std::fs::read_to_string("/etc/bifrost/compositor").ok();
    route_for(env.as_deref(), compositor.as_deref())
}

/// The route after a message from the app. A direct app that sends shared-memory `Buffers` fell back to the
/// socket path (its SDK found no Vulkan device or no `bifrost_layer_manager_v1`); every other message keeps the route.
pub fn route_after_message(route: Route, is_buffers: bool) -> Route {
    match route {
        Route::Direct if is_buffers => Route::Shm,
        other => other,
    }
}

/// The window server's toplevel for an app: the first foreign toplevel whose app id is the app's id (the SDK's
/// direct path sets its xdg app id to the manifest id). `windows` are (toplevel id, app id) pairs in list order.
pub fn foreign_window_of<'a>(windows: impl IntoIterator<Item = (u32, &'a str)>, app_id: &str) -> Option<u32> {
    windows.into_iter().find(|(_, id)| *id == app_id).map(|(toplevel, _)| toplevel)
}

/// What a switcher card or a crash screen shows for an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thumb {
    /// the app's last frame, kept by the shell
    Frame,
    /// a screencopy of the app's toplevel from the window server
    Capture,
    /// the app's icon: nothing to show yet
    Icon,
}

/// The thumbnail source, pure. A shared-memory app shows its last frame; a direct app has no frame in the shell, so
/// its card is a screencopy once the capture is ready and its icon until then.
pub fn thumbnail(route: Route, has_frame: bool, capture_ready: bool) -> Thumb {
    match route {
        Route::Shm if has_frame => Thumb::Frame,
        Route::Shm => Thumb::Icon,
        Route::Direct if capture_ready => Thumb::Capture,
        Route::Direct => Thumb::Icon,
    }
}

impl Drop for AppHost {
    fn drop(&mut self) {
        self.kill();
        crate::bare::sleep::forget_app(&self.id);
    }
}

/// The built-in Calculator's package id (Search answers open it when installed).
pub const CALCULATOR: &str = "is.olibuijr.calculator";

/// An installed app's icon at `size` px, from its package (`None` for built-in ids).
pub fn icon_image(id: &str, size: usize) -> Option<crate::gfx::Image> {
    let registered = installed();
    let app = registered.iter().find(|a| a.manifest.id == id)?;
    let img = crate::cg::imageio::decode(&std::fs::read(app.icon_path()).ok()?)?;
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
            ToShell::Preview { items, index } => {
                let app_id = installed().get(slot as usize).map(|a| a.manifest.id.clone()).unwrap_or_default();
                self.quick_look_show(&app_id, items, index as usize);
            }
            ToShell::PreviewClose => {
                let app_id = installed().get(slot as usize).map(|a| a.manifest.id.clone()).unwrap_or_default();
                if self.quick_look_owner() == Some(app_id.as_str()) {
                    self.quick_look_hide();
                }
            }
            ToShell::ServiceCall { request, service, method, body } => {
                let app_id = installed().get(slot as usize).map(|a| a.manifest.id.clone()).unwrap_or_default();
                let (ok, body) = match self.service_call(&app_id, &service, &method, &body, t) {
                    Ok(j) => (true, j.encode()),
                    Err(e) => (false, e),
                };
                self.ext_send(slot as usize, &bifrost_proto::ToApp::ServiceReply { request, ok, body });
            }
            ToShell::DistPost { name, user_info } => {
                let m = bifrost_proto::ToApp::DistNotify { name, user_info };
                for other in 0..self.ext.len() {
                    if other != slot as usize {
                        self.ext_send(other, &m);
                    }
                }
            }
            m @ (ToShell::WorkspaceRequest { .. } | ToShell::WorkspaceSubscribe { .. }) => self.workspace_msg(slot, m, t),
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
        // An app that speaks another SDK protocol is started once and then left alone until its binary changes
        // (a new installed version has a new path): no respawn loop, and the window keeps saying why.
        if is_known_incompatible(&app.manifest.id, &app.exec_path()) {
            return;
        }
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
        match AppHost::spawn_with(&app.manifest.id, &app.exec_path(), &data, &root(), self.ext_cfg(), self.session_restore_flag()) {
            Ok(h) => {
                self.ext[slot] = Some(h);
                self.usernotif_started(slot);
                self.extras_started(slot);
                self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidLaunchApplication, slot as u8);
            }
            Err(e) => {
                bifrost_core::log::shell_logger("apphost").error(format!("cannot start {}: {e}", app.manifest.id));
                self.launch_failed(slot);
                // never leave an empty (black) window behind a launch that failed
                #[cfg(feature = "desktop-wayland")]
                self.desk_close_app(AppId::External(slot as u8));
            }
        }
    }

    /// An installed app did not start (the spawn failed, or it exited before saying hello): the user gets an alert
    /// from the app, and the app's Dock launch bounce stops instead of running out its timeout.
    pub(super) fn launch_failed(&mut self, slot: usize) {
        let registered = installed();
        let Some(app) = registered.get(slot) else { return };
        let (title, body) = launch_failure_text(&app.manifest.name);
        let time = self.notify.now();
        self.notify(super::notify::Notification::new(AppId::External(slot as u8), title, body, time));
        if let Some(k) = super::apps().iter().position(|a| a.id == AppId::External(slot as u8)) {
            if let Some(d) = self.desk.as_mut() {
                d.dock_bounce.retain(|b| b.0 != k);
            }
        }
        self.dirty = true;
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
        self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidTerminateApplication, slot as u8);
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
        // the pointer as a touch frame, before the plain stream; a finger on the touchscreen is reported by its contact
        if let Some((raw, s)) = super::touch_frames::pointer_sample(kind, x, y, &self.touch_contacts) {
            h.feed_touch(raw, s, t);
        }
        h.touch(kind, 0, x, y, t as u32);
    }

    /// A touchscreen contact (`Event::Contact`) at (`x`, `y`) in screen pixels. The foreground external app gets it as a
    /// touch frame, in the same coordinates as its plain touches. A finger that went down on the on-screen keyboard stays
    /// with the keyboard. The finger is recorded either way, so the pointer stream stands aside while it is down.
    /// `deliver` is false when a system gesture holds the finger: its contact is still recorded, but the app gets nothing.
    pub fn ext_contact(&mut self, id: i32, phase: crate::bare::input::TouchPhase, x: f32, y: f32, t: f64, deliver: bool) {
        use crate::bare::input::TouchPhase as Phase;
        match phase {
            Phase::Down => self.touch_contacts.down(id),
            Phase::Up | Phase::Cancel => self.touch_contacts.up(id),
            Phase::Move => {}
        }
        let Screen::App(AppId::External(n)) = self.screen else { return };
        if !deliver {
            return;
        }
        let (x, y) = if self.letterboxed(AppId::External(n)) {
            match self.letterbox_event(Event::Move(x, y)) {
                Event::Move(x, y) => (x, y),
                _ => (x, y),
            }
        } else {
            (x, y)
        };
        if phase == Phase::Down && self.keyboard.shown() && y >= self.kb_top() {
            return;
        }
        let Some(Some(h)) = self.ext.get_mut(n as usize) else { return };
        h.feed_touch(super::touch_frames::raw_of_phase(phase), super::touch_frames::contact_sample(id, x, y), t);
    }

    /// The foreground app lost the touches (focus moved, the window closed, a system gesture took them): every touch it
    /// holds is cancelled in one frame, before the app is suspended.
    pub fn ext_cancel_touches(&mut self, t: f64) {
        self.touch_contacts.clear();
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.cancel_touches(t);
        }
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
            Out::Named(name) if super::kb_input::function_key_code(name).is_some() => KeyInput::Code(u32::from(super::kb_input::function_key_code(name).unwrap_or(0))),
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

    /// Sends a message to the foreground external app, if there is one.
    pub(super) fn ext_fg_send(&mut self, m: &ToApp) {
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.send(m);
        }
    }

    /// True when the external app in front has a text field focused (it reported the field's caret).
    pub(super) fn ext_text_focused(&self) -> bool {
        matches!(self.screen, Screen::App(AppId::External(_)))
            && self.ext_fg.and_then(|n| self.ext.get(n as usize)).and_then(Option::as_ref).is_some_and(|h| h.caret_rect.is_some())
    }

    /// Candidate text the keyboard app produced goes to the foreground field as marked text (`SetMarkedText`,
    /// `UnmarkText`). With no external app in front it is dropped, so it cannot arrive in the next one.
    fn ext_forward_composition(&mut self) {
        let comp = super::keyboard::take_composition();
        if comp.is_empty() {
            return;
        }
        // a Wayland client's field has focus: the composition goes through the window server's input method
        if self.im.active {
            for c in &comp {
                let requests = super::textinput::composition(&mut self.im, c);
                self.im_out.extend(requests);
            }
            return;
        }
        let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) else { return };
        for c in comp {
            h.send(&super::textinput::to_app(c));
        }
    }

    /// Shows or hides the keyboard over the foreground app and tells it the new safe area.
    pub(super) fn ext_set_keyboard(&mut self, on: bool, t: f64) {
        // a desktop has a hardware keyboard: physical keys reach the app, no docked one (set_keyboard applies the policy)
        self.set_keyboard(on, t);
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
        // the first frame goes first: login items and the restored windows start once it is on screen (or the ceiling)
        if self.desktop_gate.is_open(t, crate::bootsplash::first_desktop_frame_presented()) {
            self.login_items_launch();
            self.session_restore(t);
        }
        self.session_tick(t);
        let now = bifrost_proto::sys::now_ms();
        self.dying.retain_mut(|h| !h.quit_poll(now));
        let fg = match self.screen {
            Screen::App(AppId::External(n)) => Some(n),
            _ => None,
        };
        if fg != self.ext_fg {
            self.ext_cancel_touches(t);
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
            if let Some(n) = fg {
                self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidActivateApplication, n);
            }
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
        let japanese = super::kb_layouts::active() == super::kb_layouts::Layout::Japanese;
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
        // apps that exited before saying hello never started: the user is told (`launch_failed`)
        let mut failed = Vec::new();
        // direct apps that fell back to the socket path: their window opens now (after the loop, the shell is free)
        let mut fell_back = Vec::new();
        for (slot, h) in self.ext.iter_mut().enumerate() {
            let Some(h) = h else { continue };
            h.sync_appearance(appearance);
            h.sync_input_source(japanese);
            if fg == Some(slot as u8) && h.running() && h.size() != (cfg.w as usize, cfg.h as usize) {
                h.configure(cfg);
            }
            let p = h.pump();
            if p.died {
                died.push(slot as u8);
                if !h.hello && !h.incompatible {
                    failed.push(slot);
                }
            }
            if h.take_fallback() {
                fell_back.push(slot as u8);
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
                    r @ ToShell::ServiceCall { .. } => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::Preview { .. } | ToShell::PreviewClose) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::WorkspaceRequest { .. } | ToShell::WorkspaceSubscribe { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::NotifyCategories(_) | ToShell::NotifyRequest { .. } | ToShell::NotifyRemove { .. } | ToShell::NotifyQuery { .. } | ToShell::NotifySettings { .. }) => rich_msgs.push((slot as u8, r)),
                    #[cfg(feature = "desktop-wayland")]
                    r @ (ToShell::DocOpen { .. } | ToShell::DocOpenEx { .. } | ToShell::DocSave { .. } | ToShell::DocCommit { .. } | ToShell::DocAbort { .. } | ToShell::DocReopen { .. } | ToShell::DocRelease { .. } | ToShell::DocSaveGrant { .. } | ToShell::DocGrantPath { .. }) => docs.push((slot as u8, r)),
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
        for &slot in &died { self.dnd_app_gone(slot); self.extras_app_gone(slot); self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidTerminateApplication, slot); }
        // a failed exit gets the "quit unexpectedly" alert (read before the host is dropped); the shell keeps running
        for &slot in &died { self.app_crashed(slot as usize); }
        for (id, code) in late_crashes() { self.app_crashed_late(&id, code); }
        // a window whose app has exited must not keep its last frame on screen
        for slot in died { self.desk_close_app(super::AppId::External(slot)); }
        for slot in failed { self.launch_failed(slot); }
        if self.desk.is_some() {
            for slot in fell_back { self.desk_open(super::AppId::External(slot), t); }
        }
        if badge_changed { self.dirty = true; }
        let indexed = rich_msgs.iter().any(|(_, r)| matches!(r, ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }));
        for (slot, r) in rich_msgs { self.rich_msg(slot, r, t); }
        self.services_tick(t);
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
        {
            let chrome = super::atspi::due(t).then(|| self.ax_chrome_snap());
            super::atspi::sync(&mut self.ext, t, chrome);
        }
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
            let outdated = matches!(self.ext.get(slot), Some(Some(h)) if h.incompatible);
            if outdated {
                let (w, h) = (self.w as f32, self.h as f32);
                s.fill_rrect(0.0, 0.0, w, h, 0.0, super::ui::BLACK);
                self.label("This app needs an update", super::ui::MAC_BODY_PT, super::ui::Weight::Semibold, super::ui::WHITE, |im| {
                    s.blit(im, ((w - im.w as f32) / 2.0) as i32, ((h - im.h as f32) / 2.0) as i32, 255, 255);
                });
            } else {
                s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, super::ui::BLACK);
            }
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

/// The child's exit status once its socket closed: a process that exits (or crashes) closes the socket a moment before
/// the kernel reports it, so this polls briefly (at most ~20 ms) before the caller falls back to a late reap.
fn exit_status_soon(c: &mut std::process::Child) -> std::io::Result<Option<std::process::ExitStatus>> {
    for _ in 0..10 {
        if let Some(st) = c.try_wait()? {
            return Ok(Some(st));
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    c.try_wait()
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
    fn the_input_source_is_sent_on_first_contact_and_on_each_change() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let next = |peer: &Channel| match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
            _ => None,
        };
        host.sync_input_source(false);
        assert_eq!(next(&peer), Some(ToApp::InputSource { japanese: false }), "first contact sends the source");
        host.sync_input_source(false);
        assert_eq!(next(&peer), None, "unchanged: nothing sent");
        host.sync_input_source(true);
        assert_eq!(next(&peer), Some(ToApp::InputSource { japanese: true }));
    }

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
    fn a_hello_from_another_protocol_marks_the_host_incompatible_instead_of_closing_it() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let mut pumped = Pumped::default();
        host.handle(ToShell::Hello { sdk_version: SDK_VERSION + 1, app_id: "fixture".into() }, vec![], &mut pumped);
        assert!(host.incompatible && !host.hello);
        assert!(!pumped.died, "the window stays up to show the message");
    }

    #[test]
    fn an_incompatible_binary_is_remembered_so_the_shell_never_respawns_it() {
        let exec = PathBuf::from("/var/lib/springboard/apps/loop-test/1.4.1/bin");
        assert!(!is_known_incompatible("loop-test", &exec));
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("loop-test", channel, None, cfg);
        host.exec = exec.clone();
        host.handle(ToShell::Hello { sdk_version: SDK_VERSION + 1, app_id: "loop-test".into() }, vec![], &mut Pumped::default());
        assert!(is_known_incompatible("loop-test", &exec));
        assert!(!is_known_incompatible("loop-test", Path::new("/var/lib/springboard/apps/loop-test/1.5.0/bin")), "a new version gets a fresh try");
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
        host.handle(ToShell::Buffers{w:4,h:3,generation:1},buffers(),&mut pumped);
        host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,4,3]]},vec![],&mut pumped);
        let mut resized=cfg; resized.w=8; resized.h=6;
        host.configure(resized);
        assert_eq!(host.size(),(8,6));
        assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((4,3)));
        let mut pixels=vec![0;8*6];
        host.blit(&mut Surface{w:8,h:6,stride:8,px:&mut pixels,y0:0},true);
        host.handle(ToShell::Buffers{w:8,h:6,generation:2},vec![sys::shm_create("resize-c",192).unwrap(),sys::shm_create("resize-d",192).unwrap()],&mut pumped);
        host.handle(ToShell::Frame{buf:0,damage:vec![[0,0,8,6]]},vec![],&mut pumped);
        assert_eq!(host.frame().map(|(_,w,h)|(w,h)),Some((8,6)));
    }

    #[test]
    fn manifest_animation_behavior_parses_and_defaults() {
        let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
        use super::super::window_fx::AnimationBehavior as B;
        assert_eq!(Manifest::parse(base).unwrap().animation_behavior(), B::Default);
        assert_eq!(Manifest::parse(&format!("{base}animation_behavior = \"none\"\n")).unwrap().animation_behavior(), B::None);
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
mod window_table_tests {
    use super::*;

    fn cfg() -> Configure {
        Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false }
    }

    /// Every message the shell has queued for the app so far.
    fn sent(peer: &Channel) -> Vec<ToApp> {
        let mut out = Vec::new();
        while let Recv::Packet(p, _) = peer.recv(false).unwrap() {
            out.push(ToApp::decode(&p).unwrap());
        }
        out
    }

    #[test]
    fn the_window_table_follows_the_messages_the_app_sends() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0 }, Vec::new(), &mut out);
        host.handle(ToShell::WindowOpen { id: 2, title: "Grace".into(), w: 480.0, h: 560.0 }, Vec::new(), &mut out);
        let ids = |h: &AppHost| h.windows.of_app("fixture").iter().map(|w| w.id).collect::<Vec<u32>>();
        assert_eq!(ids(&host), vec![2, 1], "the newest window is in front");
        assert!(out.frame, "a window change asks the desk for a frame");
        host.handle(ToShell::WindowTitle { id: 1, title: "Ada L.".into() }, Vec::new(), &mut out);
        host.handle(ToShell::WindowResize { id: 1, w: 320.0, h: 240.0 }, Vec::new(), &mut out);
        let one = host.windows.of_app("fixture")[1];
        assert_eq!((one.title.as_str(), one.w, one.h), ("Ada L.", 320.0, 240.0));
        host.handle(ToShell::WindowClose { id: 2 }, Vec::new(), &mut out);
        assert_eq!(ids(&host), vec![1]);
    }

    #[test]
    fn a_window_the_table_refuses_is_closed_again_for_the_app() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        let refused = crate::shell::app_window_table::MAX_PER_APP as u32 + 1;
        for id in 1..=refused {
            host.handle(ToShell::WindowOpen { id, title: "w".into(), w: 10.0, h: 10.0 }, Vec::new(), &mut out);
        }
        assert_eq!(host.windows.of_app("fixture").len(), crate::shell::app_window_table::MAX_PER_APP);
        assert_eq!(sent(&peer), vec![ToApp::WindowClosed { id: refused }]);
    }

    #[test]
    fn a_window_gets_its_own_buffers_and_its_frame_is_given_back_once_blitted() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 2.0, h: 2.0 }, Vec::new(), &mut out);
        let bytes = 2 * 2 * 4;
        let fds = vec![sys::shm_create("test-window-0", bytes).unwrap(), sys::shm_create("test-window-1", bytes).unwrap()];
        host.handle(ToShell::WindowBuffers { id: 1, w: 2, h: 2, generation: 7 }, fds, &mut out);
        assert!(host.window_frame(1).is_none(), "no frame before the first one");
        host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
        assert_eq!(host.window_frame(1).map(|(_, w, h)| (w, h)), Some((2, 2)));
        host.handle(ToShell::WindowFrame { id: 1, buf: 1, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
        assert!(sent(&peer).is_empty(), "the replaced buffer waits until its frame is blitted");
        host.window_blitted(1);
        assert_eq!(sent(&peer), vec![ToApp::WindowRelease { id: 1, buf: 0, generation: 7 }]);
        host.handle(ToShell::WindowFrame { id: 9, buf: 0, damage: vec![] }, Vec::new(), &mut out);
        assert!(host.window_frame(9).is_none(), "a frame for a window that is not open is dropped");
    }

    #[test]
    fn the_shell_closing_a_window_drops_it_and_tells_the_app_once() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 3, title: "Card".into(), w: 10.0, h: 10.0 }, Vec::new(), &mut out);
        assert!(host.configure_window(3, 20.0, 30.0));
        assert!(host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 5));
        assert!(host.close_window_from_shell(3));
        assert!(!host.close_window_from_shell(3), "a window closes once");
        assert!(!host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 6), "no touch reaches a closed window");
        assert!(host.windows.of_app("fixture").is_empty());
        assert_eq!(
            sent(&peer),
            vec![
                ToApp::WindowConfigure { id: 3, w: 20.0, h: 30.0 },
                ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 5 },
                ToApp::WindowClosed { id: 3 },
            ]
        );
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

/// The alert title and body for an app that did not start, pure: an empty name reads as "The app".
pub fn launch_failure_text(name: &str) -> (String, String) {
    let name = name.trim();
    let name = if name.is_empty() { "The app" } else { name };
    (format!("{name} could not be opened"), "It stopped before it finished starting. Try opening it again.".to_string())
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

    #[test]
    fn a_failed_launch_names_the_app_and_says_to_try_again() {
        let (title, body) = crate::shell::apphost::launch_failure_text("  Notes ");
        assert_eq!(title, "Notes could not be opened");
        assert!(body.contains("Try opening it again"));
        assert_eq!(crate::shell::apphost::launch_failure_text("").0, "The app could not be opened");
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    fn cfg() -> Configure {
        Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false }
    }

    #[test]
    fn the_route_is_direct_only_under_bifrost_wm_with_the_switch_on() {
        assert_eq!(route_for(Some("1"), Some("bifrost-wm")), Route::Direct);
        assert_eq!(route_for(Some(" 1\n"), Some("bifrost-wm\n")), Route::Direct);
        assert_eq!(route_for(Some("1"), Some("other-wm")), Route::Shm, "any other compositor keeps the shared-memory path");
        assert_eq!(route_for(None, Some("bifrost-wm")), Route::Shm, "no switch, no direct path");
        assert_eq!(route_for(Some("0"), Some("bifrost-wm")), Route::Shm);
        assert_eq!(route_for(Some("1"), None), Route::Shm, "no /etc/bifrost/compositor, no direct path");
        assert_eq!(route_for(Some("true"), Some("bifrost-wm")), Route::Shm, "only the literal 1 switches it on");
    }

    #[test]
    fn a_direct_app_that_sends_shared_memory_buffers_falls_back() {
        assert_eq!(route_after_message(Route::Direct, true), Route::Shm);
        assert_eq!(route_after_message(Route::Direct, false), Route::Direct, "a frame or any other message keeps the route");
        assert_eq!(route_after_message(Route::Shm, true), Route::Shm);
        assert_eq!(route_after_message(Route::Shm, false), Route::Shm);
    }

    #[test]
    fn only_the_shared_memory_route_expects_frames() {
        assert!(Route::Shm.expects_frames());
        assert!(!Route::Direct.expects_frames());
    }

    #[test]
    fn the_window_of_an_app_is_its_first_toplevel_with_that_app_id() {
        let windows = [(3, "is.a"), (7, "is.b"), (9, "is.b")];
        assert_eq!(foreign_window_of(windows, "is.b"), Some(7), "the first listed toplevel wins");
        assert_eq!(foreign_window_of(windows, "is.a"), Some(3));
        assert_eq!(foreign_window_of(windows, "is.c"), None, "no window while the app is still starting");
        assert_eq!(foreign_window_of([(1, "is.bb")], "is.b"), None, "app ids match whole, not by prefix");
        assert_eq!(foreign_window_of(Vec::<(u32, &str)>::new(), "is.a"), None);
    }

    #[test]
    fn a_direct_app_thumbnail_is_a_capture_once_ready_and_its_icon_before() {
        assert_eq!(thumbnail(Route::Shm, true, false), Thumb::Frame);
        assert_eq!(thumbnail(Route::Shm, true, true), Thumb::Frame, "the shared-memory frame is kept by the shell");
        assert_eq!(thumbnail(Route::Shm, false, true), Thumb::Icon, "no frame yet");
        assert_eq!(thumbnail(Route::Direct, true, true), Thumb::Capture);
        assert_eq!(thumbnail(Route::Direct, true, false), Thumb::Icon, "the shell has no frame for a direct app");
        assert_eq!(thumbnail(Route::Direct, false, false), Thumb::Icon);
    }

    #[test]
    fn a_direct_host_is_demoted_by_buffers_once_and_ignores_its_frames() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        host.route = Route::Direct;
        let mut out = Pumped::default();
        host.handle(ToShell::Frame { buf: 0, damage: vec![] }, Vec::new(), &mut out);
        assert_eq!(host.route(), Route::Direct, "a frame from a direct app does not demote it");
        assert!(!host.take_fallback());
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, Vec::new(), &mut out);
        assert_eq!(host.route(), Route::Shm);
        assert!(host.take_fallback(), "the fallback is reported once");
        assert!(!host.take_fallback());
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, Vec::new(), &mut out);
        assert!(!host.take_fallback(), "a shared-memory app is never demoted again");
    }

    fn fixture_host() -> (AppHost, Channel) {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        (AppHost::with_channel("fixture", channel, None, cfg), peer)
    }

    #[test]
    fn short_memfds_are_refused_and_good_ones_sealed() {
        let (mut host, _peer) = fixture_host();
        let mut out = Pumped::default();
        host.handle(ToShell::Buffers { w: 8192, h: 8192, generation: 1 }, vec![sys::shm_create("s1", 4096).unwrap(), sys::shm_create("s2", 4096).unwrap()], &mut out);
        assert!(host.bufs.is_empty(), "4 KiB files cannot back 8192 x 8192");
        let a = sys::shm_create("g1", 48).unwrap();
        let b = sys::shm_create("g2", 48).unwrap();
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 7 }, vec![a, b], &mut out);
        assert_eq!(host.bufs.len(), 2);
        assert_eq!(host.generation, 7);
    }

    #[test]
    fn a_stale_release_never_reaches_the_new_set() {
        let (mut host, peer) = fixture_host();
        let mut out = Pumped::default();
        let set = |n: &str| vec![sys::shm_create(n, 48).unwrap(), sys::shm_create(n, 48).unwrap()];
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, set("a"), &mut out);
        host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
        host.handle(ToShell::Frame { buf: 1, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 2 }, set("b"), &mut out);
        host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, vec![], &mut out);
        let mut px = vec![0; 12];
        host.blit(&mut Surface { w: 4, h: 3, stride: 4, px: &mut px, y0: 0 }, true);
        while let Ok(Recv::Packet(p, _)) = peer.recv(false) {
            if let Ok(ToApp::Release { generation, .. }) = ToApp::decode(&p) {
                assert_eq!(generation, 2, "only releases of the current set go out");
            }
        }
    }

    #[test]
    fn an_app_that_never_reads_does_not_block_the_shell() {
        let (mut host, _peer) = fixture_host();
        for _ in 0..100_000 {
            host.send(&ToApp::Resume);
            if !host.running() {
                break;
            }
        }
        assert!(host.backlog.len() <= MAX_BACKLOG);
    }

    #[test]
    fn a_flooding_app_is_read_in_bounded_passes() {
        let (mut host, peer) = fixture_host();
        for i in 0..10u32 {
            peer.send(&ToShell::Notify { title: i.to_string(), body: String::new() }.encode(), &[]).unwrap();
        }
        let first = host.pump_up_to(3);
        assert_eq!(first.requests.len(), 3, "one pass reads at most its budget");
        let second = host.pump_up_to(3);
        assert_eq!(second.requests.len(), 3);
        let rest = host.pump_up_to(usize::MAX);
        assert_eq!(rest.requests.len(), 4, "the rest of the flood is read on the next passes, in order");
        assert_eq!(rest.requests.first(), Some(&ToShell::Notify { title: "6".into(), body: String::new() }));
        assert!(matches!(peer.recv(false), Ok(Recv::Empty)));
    }

    #[test]
    fn quitting_never_blocks_on_an_app_that_stopped_reading() {
        let (mut host, _peer) = fixture_host();
        // fill the socket: nothing reads the peer end
        while host.chan.try_send(&[0u8; 64], &[]).is_ok() {}
        host.sudden = false;
        let _ = host.begin_quit(0.0);
        host.kill();
    }
}
