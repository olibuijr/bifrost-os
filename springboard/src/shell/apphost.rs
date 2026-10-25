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
use bifrost_proto::{Configure, KeyInput, ToApp, ToShell, TouchKind, WindowInput, CHORD_SHIFT, SDK_VERSION};
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
    /// `game = true`: a game even outside the Games category (Game Mode, see `shell::gamemode`).
    pub game: bool,
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
    /// `package_types = ["rtfd"]`: the folder-package extensions the app reads and writes (`sdk::document_package`).
    /// Files and the file panels treat a folder with a known extension as one document; declaring it here is what
    /// lets the app open one.
    pub package_types: Vec<String>,
    /// `edit_keys = true`: the app is a text editor that takes the navigation keys with their modifiers. The arrows, Home and
    /// End with Shift, Alt, Ctrl or Cmd (and Home and End alone) reach it as chords on the evdev key
    /// (`KeyInput::chord(105, CHORD_SHIFT)` for Shift+Left), so it can extend the selection and move by word and line. Any other
    /// app gets those keys as the plain arrow codes, as before (see `Shell::hw_nav`).
    pub edit_keys: bool,
    /// `records_audio = true`: a native `audio` app that records from the microphone by opening the capture PCM itself
    /// (the upstream SoundCraft). The shell shows the microphone Allow sheet at launch while the person has not answered,
    /// since the app's first take cannot ask (`app_policy::ask_microphone_at_launch`). SDK apps ask through the SDK.
    pub records_audio: bool,
    /// `index_extension = true`: the app answers the shell's reindex requests (`CSIndexExtensionRequestHandler`). The
    /// shell asks once per run, when the app has no items in the Search index (see `request_index_extensions`).
    pub index_extension: bool,
    /// `file_manager_folders = ["~/Cloud"]`: the app is a file-manager sync extension for these folders (`~/` is the
    /// home folder). Files runs the extension through the app's control socket and only ever asks it about items inside
    /// them (`bifrost_app::file_manager_sync`, [`Manifest::file_manager_dirs`]).
    pub file_manager_folders: Vec<String>,
    /// `"native"`: a native Wayland app owns its GPU surfaces instead of using SDK frames. `"full-size"`: an SDK app
    /// with a full-size-content window (no title row; the traffic lights sit over its sidebar). `"borderless"`: a
    /// full-size window without any shell chrome, covering the work area, transparent where the app draws nothing and
    /// letting the pointer through there (`desk::borderless`; Sticky Notes' notes).
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
    /// `[background] tasks = ["com.x.refresh"]`: the task identifiers the app may submit to Background App Refresh
    /// (`shell::services::background`). A request for any other identifier is refused.
    pub background_tasks: Vec<String>,
    /// What the app opens, `type` or `type:rank` (rank owner, default, alternate, none; default is `default`): `handles = ["public.png:owner", "public.text"]`.
    pub handles: Vec<String>,
    /// URL schemes the app opens: `schemes = ["https"]`.
    pub schemes: Vec<String>,
    /// App Intents declared as `[[intent]]` tables (`shell::intents`); Search lists the shortcuts.
    pub intents: Vec<super::intents::ManifestIntent>,
    /// entity types the app offers to Search (`[[entity]]` tables with `type = "Note"`); `entity_index` indexes them
    pub entity_types: Vec<String>,
    /// `[[control]]` tables: buttons and toggles the app adds to Control Center (`shell::app_controls`).
    pub controls: Vec<super::app_controls::ManifestControl>,
    /// `[[widget]]` tables: the widget kinds the app offers, with their names and sizes
    /// (`bifrost_ui::widget_kind::parse_manifest`). The shell registers them in the Edit Widgets gallery.
    pub widgets: Vec<bifrost_ui::widget_kind::WidgetKind>,
    /// the `[[focus_filter]] intent` ids: run when the Focus changes (`services::focus_filter::focus_calls`)
    pub focus_intents: Vec<String>,
    /// `[[service]]` tables: Services menu entries this app provides for a selection (`bifrost_ui::app_services`).
    pub services: Vec<bifrost_ui::app_services::ServiceDecl>,
    /// `[[quick_action]]` tables: the static Home Screen quick actions (`shell::quick_actions`).
    pub quick_actions: Vec<bifrost_ui::quick_actions::ShortcutItem>,
    /// `[[settings_pane]]` tables: the app's panes, listed under the app in Settings (`shell::settings_panes`).
    pub settings_panes: Vec<super::settings_panes::SettingsPane>,
    /// `[privacy] location = "..."`: the reason shown when the app asks for location (the purpose string). A
    /// location request without it is refused, so no prompt shows without a reason.
    pub location_purpose: String,
    /// `[[audio_unit]]` tables: the audio processors the app ships (`bifrost_ui::audio_extension::parse_manifest`). Tables
    /// that fail the checks are left out; the audio unit settings list the rest, disabled until the person turns them on.
    pub audio_units: Vec<bifrost_ui::audio_extension::Manifest>,
    /// `[[script.command]]` and `[[script.class]]` tables: the scripting dictionary (`bifrost_core::script`).
    pub scripting: bifrost_core::script::dictionary::Dictionary,
}

impl Manifest {
    /// The app draws to the top edge of its window (`NativeWindowStyleMaskFullSizeContentView`).
    pub fn full_size_content(&self) -> bool {
        self.window == "full-size" || self.borderless()
    }
    /// The app's window has no shell chrome and takes input only where it draws (`window = "borderless"`).
    pub fn borderless(&self) -> bool {
        self.window == "borderless"
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
    /// The folders of `file_manager_folders` as absolute paths: `~/x` is under `home`, a relative path, a path with `..`,
    /// the root and the home folder itself are left out (an extension watches folders, never everything).
    pub fn file_manager_dirs(&self, home: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for raw in &self.file_manager_folders {
            let path = match raw.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(raw),
            };
            let clean = path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
            if clean && path != Path::new("/") && path != home && !out.contains(&path) {
                out.push(path);
            }
        }
        out
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
        // inside a `[privacy]` table the keys are the purpose strings (`location`); any other header ends it
        let mut privacy = false;
        // inside a `[[service]]` table keys belong to the service
        let mut service = false;
        // inside an `[[entity]]` table the `type` key names an entity type Search indexes
        let mut entity = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && !line.contains('=') {
                table = Some(line == "[[intent]]");
                privacy = line == "[privacy]";
                if line == "[[intent]]" {
                    m.intents.push(Default::default());
                }
                service = line == "[[service]]";
                if service {
                    m.services.push(Default::default());
                }
                entity = line == "[[entity]]";
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if service {
                if let Some(d) = m.services.last_mut() {
                    d.set(k, v);
                }
                continue;
            }
            let strs = || -> Vec<String> { v.trim_start_matches('[').trim_end_matches(']').split(',').map(|s| unquote(s.trim())).filter(|s| !s.is_empty()).collect() };
            if entity {
                if k == "type" {
                    m.entity_types.push(unquote(v));
                }
                continue;
            }
            if privacy {
                if k == "location" {
                    m.location_purpose = unquote(v);
                }
                continue;
            }
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
                "game" => m.game = unquote(v) == "true",
                "index_extension" => m.index_extension = unquote(v) == "true",
                "edit_keys" => m.edit_keys = unquote(v) == "true",
                "records_audio" => m.records_audio = unquote(v) == "true",
                "file_manager_folders" => m.file_manager_folders = strs(),
                "permissions" => m.permissions = strs(),
                "keychain_groups" => m.keychain_groups = strs(),
                "entitlements" => m.entitlements = strs(),
                "provides" => m.provides = strs(),
                "package_types" => m.package_types = strs(),
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
        m.controls = super::app_controls::parse_controls(text);
        m.widgets = bifrost_ui::widget_kind::parse_manifest(text);
        m.focus_intents = super::services::focus_filter::parse_declarations(text).into_iter().filter_map(|d| d.intent).collect();
        m.quick_actions = super::quick_actions::parse_static(text);
        m.settings_panes = super::settings_panes::parse_static(text);
        m.background_tasks = super::services::background::parse_tasks(text);
        m.audio_units = bifrost_ui::audio_extension::parse_manifest(text).into_iter().filter_map(Result::ok).collect();
        // the `[[script.*]]` tables are skipped above (their header ends any open table) and read here
        m.scripting = bifrost_core::script::dictionary::Dictionary::parse(text);
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

impl SizeLimits {
    /// The limits with the app's own content limits (`ToShell::SetContentLimits`) over the manifest's `min_size` and
    /// `max_size`; a limit the app left `None` keeps the manifest's. A fixed-size window keeps its fixed size.
    pub fn with_content_limits(self, min: Option<(f32, f32)>, max: Option<(f32, f32)>) -> Self {
        if self.fixed {
            return self;
        }
        SizeLimits { min: min.or(self.min), max: max.or(self.max), ..self }
    }
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

/// The directory of an entry whose package is gone. It never exists, so a reader that joins a file name onto an app's
/// directory finds nothing, instead of a file of the same name in the shell's working directory.
const REMOVED_DIR: &str = "/nonexistent/bifrost-removed-app";

/// An app found under `<root>/apps/<id>/current/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub manifest: Manifest,
    /// the version directory (`current` resolved)
    pub dir: PathBuf,
}

impl Installed {
    /// Whether the package is gone: the entry only keeps its slot (see `merge_installed`).
    pub fn is_removed(&self) -> bool {
        self.dir == Path::new(REMOVED_DIR)
    }
    /// This entry after its package was uninstalled: the id, name and window class stay (a running instance and its
    /// windows still resolve), everything that grants anything goes.
    fn removed(&self) -> Installed {
        let m = &self.manifest;
        Installed {
            manifest: Manifest { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), wayland_app_id: m.wayland_app_id.clone(), ..Manifest::default() },
            dir: PathBuf::from(REMOVED_DIR),
        }
    }
    pub fn exec_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.exec)
    }
    /// The app's icon file: its `icon.json` (layers) when the package has one, else the manifest's flat icon.
    pub fn icon_path(&self) -> PathBuf {
        let doc = self.dir.join("icon.json");
        if doc.is_file() {
            return doc;
        }
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
    let apps = scan(root);
    sync_app_types(&apps);
    if let Ok(mut g) = INSTALLED.write() { *g = Some(apps.into()); }
}
/// Registers the document types every installed app declares in its manifest (`[[exported_types]]` and
/// `[[imported_types]]`), so `uttype` lookups, Open With and the share menu see them. An app whose declarations
/// conflict is left out and reported on stderr; the others still register (`register_manifests`).
fn sync_app_types(apps: &[Installed]) {
    let texts: Vec<(&str, String)> = apps.iter().filter(|a| !a.is_removed()).filter_map(|a| std::fs::read_to_string(a.dir.join("manifest.toml")).ok().map(|t| (a.manifest.id.as_str(), t))).collect();
    let refs: Vec<&str> = texts.iter().map(|(_, t)| t.as_str()).collect();
    for (i, e) in bifrost_ui::uttype_declarations::register_manifests(&refs) {
        eprintln!("springboard: app {} declares document types that were not registered: {e:?}", texts[i].0);
    }
}
/// How many installed apps the registry holds. A slot is a `u8` on the wire and in the window table, and the Dock and
/// the app grid list the first 200; a package beyond the limit is not listed or launchable, and the shell says so (`merge_installed`).
pub const MAX_INSTALLED: usize = 200;
static IGNORED_REPORTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Merges a scan into the registry, keeping every slot where it was (a slot is an app's identity for its windows and
/// processes). An app whose package is gone from disk and from the scan keeps its slot as a removed entry: no
/// permissions, entitlements, services or handlers, and `ext_open` refuses to start it, so the checks that look an
/// app up by id stop trusting a manifest that no longer exists. Installing it again fills the same slot.
pub(super) fn merge_installed(old: &[Installed], scanned: Vec<Installed>) -> Vec<Installed> {
    let mut all: Vec<Installed> = old
        .iter()
        .map(|a| {
            let gone = !a.is_removed() && !scanned.iter().any(|s| s.manifest.id == a.manifest.id) && !a.dir.join("manifest.toml").exists();
            if gone { a.removed() } else { a.clone() }
        })
        .collect();
    let mut ignored = 0usize;
    for app in scanned {
        if let Some(existing) = all.iter_mut().find(|a| a.manifest.id == app.manifest.id) { *existing = app; }
        else if all.len() < MAX_INSTALLED { all.push(app); }
        else { ignored += 1; }
    }
    if IGNORED_REPORTED.swap(ignored, std::sync::atomic::Ordering::Relaxed) != ignored && ignored > 0 {
        bifrost_core::log::shell_logger("apphost").error(format!("{ignored} installed app(s) are not listed: the shell holds at most {MAX_INSTALLED}"));
    }
    all
}
/// Replace existing metadata while preserving SDK slot identities.
pub fn refresh() -> bool {
    let scanned = scan(&root());
    let Ok(mut g) = INSTALLED.write() else { return false };
    let all = merge_installed(g.as_deref().unwrap_or(&[]), scanned);
    if g.as_deref() == Some(all.as_slice()) { return false; }
    sync_app_types(&all);
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

/// The apps asked for a reindex this run (see `request_index_extensions`).
static REINDEX_ASKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

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
pub(super) fn is_known_incompatible(id: &str, exec: &Path) -> bool {
    let exec = resolved(exec);
    INCOMPATIBLE.lock().map(|g| g.iter().any(|(i, e)| i == id && *e == exec)).unwrap_or(false)
}

/// A binary's path with the package's `current` symlink resolved to its version folder (`apps/<id>/1.2.0/bin`), so two
/// versions have two paths; a path that cannot be resolved (gone, or a test fixture) is kept as it is.
pub(super) fn resolved(exec: &Path) -> PathBuf {
    std::fs::canonicalize(exec).unwrap_or_else(|_| exec.to_path_buf())
}

/// Whether an app's request to open a web link goes through: the app must be in front, and the previous link it opened
/// must be at least a second old (`last` records `t`, the shell clock in ms, when it passes).
fn open_url_allowed(in_front: bool, last: &mut Option<f64>, t: f64) -> bool {
    if !in_front || last.is_some_and(|l| t >= l && t - l < 1000.) {
        return false;
    }
    *last = Some(t);
    true
}

/// Seals a buffer the app sent against shrinking, then checks it holds at least `bytes`. The order matters: the length
/// of a file the app can still truncate says nothing, because it can shrink between the check and the seal and leave the
/// shell with a mapping that runs past the end of the file (SIGBUS on the first blit). After the seal the length can only grow.
fn sealed_len_at_least(f: &sys::Owned, bytes: usize) -> bool {
    sys::seal_shrink(f).is_ok() && sys::shm_len(f).is_ok_and(|n| n >= bytes)
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

/// What an app sent for an extra window before the window's `WindowOpen`. An SDK built before the fix queued the open request
/// behind the window's buffers and first frame, so the pixels came first; they are kept and replayed when the window opens.
enum Early {
    Buffers { id: u32, w: u32, h: u32, generation: u32, fds: Vec<sys::Owned> },
    Frame { id: u32, buf: u32, damage: Vec<bifrost_proto::Rect> },
}

impl Early {
    fn window(&self) -> u32 {
        match self {
            Early::Buffers { id, .. } | Early::Frame { id, .. } => *id,
        }
    }
}

/// The most early messages an app may leave waiting; the oldest go first (their descriptors close with them).
const MAX_EARLY: usize = 8;

/// What `pump` saw that the shell must act on besides the frame.
#[derive(Debug, Default, PartialEq)]
pub struct Pumped {
    /// something the desk shows changed: a new frame is ready to blit, or the app changed what the shell draws for it
    pub frame: bool,
    pub requests: Vec<ToShell>,
    /// the app exited or its socket closed during this pump
    pub died: bool,
    /// `frame` was asked for by more than pixels: a title, menus, a window's size or level, commands. The shell draws
    /// those itself, so they cost a whole desktop frame; pixels alone only republish the windows.
    pub chrome: bool,
    /// A pasteboard blob was already read in this pump: the rest are dropped unread (one read costs up to 64 MiB).
    blob_read: bool,
}

impl Pumped {
    /// Something the shell draws changed (not just the app's pixels).
    fn chrome(&mut self) {
        self.frame = true;
        self.chrome = true;
    }
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
/// How long (ms) after an app left the front its clipboard writes are still taken (a copy followed by a quick switch).
const PB_WRITE_GRACE_MS: f64 = 1500.0;

/// How long after a wheel event the drag emulated for it can still arrive, in ms.
pub(super) const WHEEL_DRAG_MS: f64 = 250.0;

pub struct AppHost {
    pub id: String,
    /// The binary this process was started from (empty for a host on a test channel).
    exec: PathBuf,
    /// The appearance flags the app last received (`u32::MAX` until the first send).
    appearance_sent: u32,
    /// The input source the app last received (`ToApp::InputSource`); `None` until the first send.
    input_japanese: Option<bool>,
    /// The pasteboard change count the app last received (`ToApp::Pasteboard`); `None` until the first sync.
    pb_sent: Option<u32>,
    /// The pasteboard change count the app was told about without its contents (`ToApp::PasteboardChanged`, sent while the
    /// app is not in front); `None` until then.
    pb_noted: Option<u32>,
    /// The last time (monotonic ms) the app was the one in front; a clipboard write is taken from the app in front, and for
    /// `PB_WRITE_GRACE_MS` after it left (a copy and a quick switch).
    fg_seen_ms: f64,
    /// Whether a clipboard write from the app would be taken in this pass (set by the shell before each pump). A large write
    /// is read from its descriptor only when it would be: an app that may not write costs the shell nothing.
    pb_may_write: bool,
    /// The out-of-band copy of the board built for the change count (kept while the socket is full, so a retry does not copy
    /// the board again).
    pb_blob: Option<(u32, sys::Owned, bifrost_proto::pbblob::Layout)>,
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
    /// The app has shown a frame since it started: a new `Buffers` set (it reallocates on every resize) leaves `shown`
    /// empty until the next frame, but the window stays mapped meanwhile (`between_buffers`).
    presented: bool,
    pending: Damage,
    status: Status,
    pub wants_keyboard: bool,
    pub hello: bool,
    /// the app speaks another SDK protocol (or its packets do not decode): its window shows "This app needs an update"
    pub incompatible: bool,
    /// the shell has been told this app is gone (`take_exit`), so its windows are removed exactly once
    exit_reported: bool,
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
    /// Element notifications the app posted (`ToShell::AxNotify`), waiting for the bridge to emit their AT-SPI signals; at most 64.
    pub ax_notes: Vec<(u64, bifrost_ui::ax::AxNotification)>,
    /// The app's custom rotors (`ToShell::AxRotors`), cleaned with `bifrost_ui::ax_rotor::clean`; empty when it declares none.
    pub custom_rotors: Vec<bifrost_ui::ax::AxCustomRotor>,
    /// The named custom actions of the app's elements (`ToShell::AxActions`), cleaned with `bifrost_ui::ax_action::clean`.
    pub ax_actions: Vec<bifrost_ui::ax_action::AxActionSet>,
    /// Guided Access requests (`ToShell::GuidedAccessRequest`: start with restrictions, or end), oldest first, taken by the shell each tick.
    pub guided_requests: Vec<(bool, Vec<bifrost_proto::GuidedRestriction>)>,
    last_notification: Option<f64>,
    /// When the app last opened a web link (`ToShell::OpenUrl`), so a burst of requests opens one page.
    last_open_url: Option<f64>,
    /// `ToShell::SuddenTermination`: the app allows being killed with no Quit round trip.
    pub sudden: bool,
    /// `ToShell::Activity`: the union of the app's live `ActivityOptions`; a user-initiated or latency-critical
    /// activity keeps the app from being suspended when it leaves the foreground.
    pub activity: u32,
    /// `ToShell::WindowLevel`: the stacking level the app asked for its window (`Normal` until it asks).
    pub window_level: bifrost_proto::WindowLevel,
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
    /// The pointer events the app takes (`ToShell::PointerInterest`, `bifrost_ui::pointer_event::{WHEEL, HOVER, GESTURES}`).
    pub pointer_interest: u32,
    /// Plain touches still to drop after a wheel event went to the app: the drag emulated for the wheel, and until when.
    wheel_swallow: (u8, f64),
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
    /// The window with key focus: 0 is the main window, otherwise an extra window's id. Keys, commands, menu items and
    /// text edits go to it (`input`); the desk sets it (`set_key_window`).
    pub key_window: u32,
    /// Each extra window's buffers, by window id (`ToShell::WindowBuffers`).
    window_bufs: std::collections::BTreeMap<u32, WinBufs>,
    /// Buffers and frames that arrived for a window the table does not hold yet, oldest first (`Early`).
    early: std::collections::VecDeque<Early>,
}

/// The `window` value of the manifest beside `exec` (empty for a classic title-bar window), `None` when there is none.
fn manifest_window_style(exec: &Path) -> Option<String> {
    let text = std::fs::read_to_string(exec.parent()?.join("manifest.toml")).ok()?;
    Manifest::parse(&text).map(|m| m.window)
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
        // the app container: its Documents, Library, Caches and tmp folders exist from the first launch on
        if let Err(e) = bifrost_core::container_layout::ContainerLayout::new(data_dir).ensure() {
            eprintln!("springboard: app container for {id}: {e}");
        }
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
        // the window style, so the app knows whether the traffic lights sit over its content (`layout::traffic_lights_over_content`);
        // never inherited from the shell's own environment
        match manifest_window_style(exec) {
            Some(style) => cmd.env("BIFROST_WINDOW", style),
            None => cmd.env_remove("BIFROST_WINDOW"),
        };
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
        crate::shell::services::metric_reports::note_launch(id);
        crate::shell::services::metric_reports::note_foreground(id);
        // the version folder this process runs from: `current` moves on when the app is updated, this does not
        h.exec = resolved(exec);
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
            pb_sent: None,
            pb_noted: None,
            fg_seen_ms: f64::NEG_INFINITY,
            pb_may_write: false,
            pb_blob: None,
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
            presented: false,
            pending: Damage::new(cfg.w as usize, cfg.h as usize),
            status: Status::Running,
            wants_keyboard: false,
            hello: false,
            incompatible: false,
            exit_reported: false,
            commands: bifrost_proto::Commands::default(),
            undo_titles: Default::default(),
            controls: Vec::new(),
            app_menus: Vec::new(),
            controls_tree: Vec::new(),
            ax_text: Vec::new(),
            ax_meta: Vec::new(),
            announcements: Vec::new(),
            announce_banner: Vec::new(),
            ax_notes: Vec::new(),
            custom_rotors: Vec::new(),
            ax_actions: Vec::new(),
            guided_requests: Vec::new(),
            last_notification: None,
            last_open_url: None,
            badge: String::new(),
            tile: Default::default(),
            dock_menu: Vec::new(),
            quick_actions: Vec::new(),
            sudden: false,
            activity: 0,
            window_level: Default::default(),
            quit_at: None,
            doc_title: String::new(),
            doc_edited: false,
            doc_file: String::new(),
            materials: Vec::new(),
            text_state: bifrost_proto::TextState::default(),
            caret_rect: None,
            input_traits: bifrost_proto::InputTraits::default(),
            accessory: Vec::new(),
            pointer_interest: 0,
            wheel_swallow: (0, 0.0),
            route: Route::Shm,
            frames: bifrost_ui::touch::TouchTracker::with_scale(cfg.scale),
            frame_scale: cfg.scale,
            fallback: false,
            windows: WindowTable::new(),
            key_window: 0,
            window_bufs: std::collections::BTreeMap::new(),
            early: std::collections::VecDeque::new(),
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

    /// The caret rectangle the app reported (`ToShell::CaretRect`, window points) in px of its window: the points
    /// times the scale the app was configured at. The candidate window and Look Up's popover are placed from it.
    pub fn caret_px(&self) -> Option<(f32, f32, f32, f32)> {
        self.caret_rect.map(|r| caret_points_to_px(r, self.frame_scale))
    }

    /// True once per host when the app is gone but the shell was not told by `pump` (a failed send, a stall kill, a
    /// `kill`, a quit that finished elsewhere): the caller removes its windows. An incompatible app is stopped on
    /// purpose and keeps its window (it says why), so it never reports.
    pub fn take_exit(&mut self) -> bool {
        if self.running() || self.exit_reported || self.incompatible {
            return false;
        }
        self.exit_reported = true;
        self.windows.close_app(&self.id);
        self.window_bufs.clear();
        self.early.clear();
        true
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

    /// Gives the app the shell's pasteboard when its change count differs from what the app last received (and on first
    /// contact, unless the board is empty): the typed items as `ToApp::Pasteboard`, and the text as `ToApp::Clipboard`, which is
    /// what the app's text fields and `Ctx::pasteboard_string` read. An empty text clears the app's.
    ///
    /// Items that do not fit a packet go out of band (`ToApp::PasteboardBlob`, a memfd): the app's text is then the
    /// plain-text item, so no cut text is ever sent. A board that cannot be sent at all (a descriptor could not be made, the
    /// socket is full) is tried again on the next pass. Only the app in front is given the contents
    /// (`AppHost::notify_pasteboard` tells the others the count).
    pub fn sync_pasteboard(&mut self, count: u32, items: &[(String, Vec<u8>)]) {
        let first = self.pb_sent.is_none();
        if self.pb_sent == Some(count) {
            return;
        }
        if first && items.is_empty() {
            self.pb_sent = Some(count);
            return;
        }
        if bifrost_proto::pbblob::fits_inline(items) {
            self.pb_sent = Some(count);
            self.send(&ToApp::Pasteboard { change_count: count, items: items.to_vec() });
            self.send(&ToApp::Clipboard { text: super::pasteboard::text_of(items).unwrap_or_default() });
            return;
        }
        // the blob goes out behind everything queued (it bypasses the backlog, so it must not overtake an older board)
        self.flush();
        if !self.running() || !self.backlog.is_empty() {
            return;
        }
        if !matches!(&self.pb_blob, Some((c, _, _)) if *c == count) {
            self.pb_blob = None;
            match bifrost_proto::pbblob::write(items) {
                Ok((fd, types)) => self.pb_blob = Some((count, fd, types)),
                Err(e) => {
                    // too large to hand over: the app keeps what it had, and is not asked again for this change
                    eprintln!("springboard: app {} was not given the pasteboard: {e}", self.id);
                    self.pb_sent = Some(count);
                    return;
                }
            }
        }
        let Some((_, fd, types)) = &self.pb_blob else { return };
        let (raw, msg) = (fd.0, ToApp::PasteboardBlob { change_count: count, types: types.clone() });
        if self.send_fd(&msg, raw) {
            self.pb_sent = Some(count);
            self.pb_blob = None;
        }
    }

    /// Tells an app that is not in front that the pasteboard changed, without the contents (`ToApp::PasteboardChanged`).
    pub fn notify_pasteboard(&mut self, count: u32) {
        if self.pb_noted == Some(count) || self.pb_sent == Some(count) {
            return;
        }
        self.pb_noted = Some(count);
        self.send(&ToApp::PasteboardChanged { change_count: count });
    }

    /// The app asked for the pasteboard (`ToShell::PasteboardRead`): it is given the contents again on the next pass.
    fn pasteboard_wanted(&mut self) {
        self.pb_sent = None;
        self.pb_noted = None;
    }

    /// The change count the app last received (with or without the contents).
    fn pb_known(&self) -> Option<u32> {
        self.pb_sent
    }

    /// Whether the app has been told about the board at `count`, with or without the contents.
    fn pb_told(&self, count: u32) -> bool {
        self.pb_sent == Some(count) || self.pb_noted == Some(count)
    }

    /// Records that the app already has the board at `count` (it made the change itself), so it is not sent back.
    pub fn pasteboard_known(&mut self, count: u32) {
        self.pb_sent = Some(count);
        self.pb_noted = Some(count);
    }

    /// Whether a clipboard write from the app is taken at `now` (monotonic ms): only the app in front writes, or one that
    /// was in front a moment ago.
    fn may_write_clipboard(&self, in_front: bool, now: f64) -> bool {
        in_front || now - self.fg_seen_ms <= PB_WRITE_GRACE_MS
    }

    /// True when the installed binary is no longer the one this process runs (the app was updated while open), so
    /// opening it again must start the new version instead of focusing the old process.
    /// `installed` is the registry's path (`apps/<id>/current/<exec>`, the same before and after an update), so it is
    /// compared with `current` resolved: the process keeps the version folder it was started from. Only a path that
    /// resolves counts: while an update flips `current` the link can dangle for a moment (or name a half-installed
    /// folder without the binary yet), and that must not make the running app look outdated and get a missing or
    /// partial package launched.
    pub fn is_outdated(&self, installed: &Path) -> bool {
        !self.exec.as_os_str().is_empty() && std::fs::canonicalize(installed).is_ok_and(|now| self.exec != now)
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
            // the next change count supersedes this one
            ToApp::PasteboardChanged { .. } => true,
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
        self.send_fd(m, file.as_raw_fd());
    }

    /// Sends `m` with the descriptor `fd` (SCM_RIGHTS), without ever blocking the shell thread. True when it was sent; a full
    /// socket drops the message (the stall timer kills an app that never reads) and the caller may try again later.
    fn send_fd(&mut self, m: &ToApp, fd: sys::Fd) -> bool {
        if !self.running() {
            return false;
        }
        match self.chan.try_send(&m.encode(), &[fd]) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.blocked_since.get_or_insert_with(sys::now_ms);
                self.stalled();
                false
            }
            Err(_) => {
                self.reap();
                false
            }
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

    /// Sends a pointer event (wheel, hover, gesture) when the app asked for that kind. True when it was sent.
    pub fn pointer(&mut self, ev: bifrost_ui::pointer_event::PointerEvent) -> bool {
        if self.pointer_interest & ev.interest() == 0 {
            return false;
        }
        self.send(&ToApp::Pointer(ev));
        true
    }

    /// Sends a pointer event (wheel, hover, gesture) to one of the app's extra windows, in window points
    /// (`WindowInput::Pointer`), when the app asked for that kind and the window is open. True when it was sent.
    pub fn pointer_window(&mut self, id: u32, ev: bifrost_ui::pointer_event::PointerEvent) -> bool {
        if id == 0 {
            return self.pointer(ev);
        }
        if self.pointer_interest & ev.interest() == 0 || !self.window_open(id) {
            return false;
        }
        self.send(&WindowInput::Pointer(ev).to_app(id));
        true
    }

    /// A wheel event went to the app at `t` ms: the drag emulated for it (a down, a move and an up) is dropped.
    pub(super) fn swallow_wheel_drag(&mut self, t: f64) {
        self.wheel_swallow = (3, t + WHEEL_DRAG_MS);
    }

    /// True for each plain touch of the drag emulated for a wheel event the app took already.
    pub(super) fn swallows_touch(&mut self, t: f64) -> bool {
        if self.wheel_swallow.0 == 0 || t > self.wheel_swallow.1 {
            self.wheel_swallow = (0, 0.0);
            return false;
        }
        self.wheel_swallow.0 -= 1;
        true
    }

    /// A press, drag or release for the app's main window, with the modifier keys held (`NSEvent.modifierFlags`).
    pub fn touch(&mut self, kind: TouchKind, id: u32, x: f32, y: f32, t_ms: u32, mods: bifrost_proto::ModifierFlags) {
        self.send(&ToApp::Touch { kind, id, x, y, t_ms, mods });
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
        self.input(WindowInput::Key(k));
    }

    /// Input for the key window: the message carries the window id when it is an extra window (`WindowInput::to_app`).
    pub fn input(&mut self, input: WindowInput) {
        let m = input.to_app(self.key_window);
        self.send(&m);
    }

    /// Input for one window of the app, key or not: a dead key's mark is taken back from the window it was shown in after
    /// the focus has moved on (`shell::dead_key`).
    pub fn input_to(&mut self, window: u32, input: WindowInput) {
        self.send(&input.to_app(window));
    }

    /// The desk made `window` the key window (0 is the main window). The app hears that the old one lost focus and that
    /// the new one gained it, so an app with extra windows always knows which one takes the keys.
    pub fn set_key_window(&mut self, window: u32) {
        if self.key_window == window {
            return;
        }
        let old = std::mem::replace(&mut self.key_window, window);
        self.send(&ToApp::WindowFocus { window: old, focused: false });
        self.send(&ToApp::WindowFocus { window, focused: true });
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
                    let safe = fds.iter().all(|f| sealed_len_at_least(f, bytes));
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
                    self.presented = true;
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
            ToShell::AppMenus(menus) => {if self.app_menus!=menus {self.app_menus=menus;out.chrome();}},
            // the rest of a set that did not fit one packet (`proto::app_menu_packets`); never more than the bar takes
            ToShell::AppMenusMore(menus) => {
                let room=bifrost_proto::MAX_APP_MENUS.saturating_sub(self.app_menus.len());
                if menus.len()>room {eprintln!("springboard: {} published more than {} menus; {} left out",self.id,bifrost_proto::MAX_APP_MENUS,menus.len()-room);}
                if room>0&&!menus.is_empty() {self.app_menus.extend(menus.into_iter().take(room));out.chrome();}
            },
            ToShell::ControlsTree(tree) => self.controls_tree=tree,
            ToShell::AxText(list) => self.ax_text=list,
            ToShell::AxMeta(list) => self.ax_meta=list,
            ToShell::AxNotify(list) => {
                for note in list {
                    if self.ax_notes.len() >= 64 {
                        self.ax_notes.remove(0);
                    }
                    self.ax_notes.push(note);
                }
            }
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
            ToShell::AxActions(list) => self.ax_actions = bifrost_ui::ax_action::clean(list),
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
            ToShell::WindowLevel(level) => {
                if self.window_level != level {
                    self.window_level = level;
                    out.chrome();
                }
            }
            ToShell::WindowLevelOf { id, level } => {
                // the main window (id 0) has its own level; an unknown extra window is ignored
                if self.windows.set_floating(&self.id, id, level == bifrost_proto::WindowLevel::Floating) {
                    out.chrome();
                }
            }
            ToShell::InputAccessory(items) => self.accessory = items.into_iter().take(bifrost_proto::MAX_ACCESSORY_ITEMS).collect(),
            ToShell::WindowOpen { id, title, w, h, position, flags } => {
                // no room for the window, or its id is taken: the app is told to drop it, so the two sides agree
                if self.windows.open(&self.id, id, &title, w, h).is_err() {
                    self.early.retain(|e| e.window() != id);
                    self.send(&ToApp::WindowClosed { id });
                } else {
                    if let Some((x, y)) = position {
                        self.windows.request_position(&self.id, id, x, y);
                    }
                    self.windows.set_title_double_click(&self.id, id, flags & bifrost_proto::window_flags::TITLE_DOUBLE_CLICK != 0);
                    // a position the app asked for wins over the cascade
                    self.windows.set_cascade(&self.id, id, position.is_none() && flags & bifrost_proto::window_flags::CASCADE != 0);
                    self.replay_early(id, out);
                }
                out.chrome();
            }
            ToShell::WindowMove { window, x, y } => {
                // an unknown window is ignored; the desk applies the position on its next sync
                if self.windows.request_position(&self.id, window, x, y) {
                    out.chrome();
                }
            }
            ToShell::WindowCollapse { window, collapsed } => {
                if self.windows.set_collapsed(&self.id, window, collapsed) {
                    out.chrome();
                }
            }
            ToShell::WindowClose { id } => {
                self.windows.close(&self.id, id);
                self.window_bufs.remove(&id);
                self.early.retain(|e| e.window() != id);
                out.chrome();
            }
            // the same title or size again changes nothing the desk draws: no frame
            ToShell::WindowTitle { id, title } => {
                let same = self.windows.of_app(&self.id).iter().any(|x| x.id == id && x.title == title);
                if !same {
                    self.windows.retitle(&self.id, id, &title);
                    out.chrome();
                }
            }
            ToShell::WindowResize { id, w, h } => {
                let same = self.windows.of_app(&self.id).iter().any(|x| x.id == id && x.w == w && x.h == h);
                if !same {
                    let _ = self.windows.resize(&self.id, id, w, h);
                    out.chrome();
                }
            }
            ToShell::WindowBuffers { id, w, h, generation } if !self.window_open(id) => self.keep_early(Early::Buffers { id, w, h, generation, fds }),
            ToShell::WindowFrame { id, buf, damage } if !self.window_open(id) => self.keep_early(Early::Frame { id, buf, damage }),
            ToShell::WindowBuffers { id, w, h, generation } => self.window_buffers(id, w, h, generation, fds),
            ToShell::WindowFrame { id, buf, damage } => self.window_frame_arrived(id, buf, damage, out),
            ToShell::Materials(list) => {if self.materials!=list {self.materials=list;out.chrome();}},
            ToShell::Commands(commands) => {if self.commands!=commands {self.commands=commands;out.chrome();}},
            ToShell::UndoTitles { undo, redo } => {if self.undo_titles!=(undo.clone(),redo.clone()) {self.undo_titles=(undo,redo);out.chrome();}},
            ToShell::Log { text } => bifrost_core::log::shell_logger("apphost").info(format!("app {}: {text}", self.id)),
            ToShell::PointerInterest { mask } => self.pointer_interest = mask,
            // a pasteboard write too large for a packet: the descriptor holds the bytes. It becomes an ordinary write; one
            // that cannot be read (no descriptor, a layout over the limits, a file shorter than it claims) is dropped
            // The read allocates up to 64 MiB, so it happens only for an app whose write would be taken (`pb_may_write`, the same
            // gate as `ext_tick`'s) and once per pump; any other blob is dropped unread with its descriptor.
            ToShell::PasteboardBlob { .. } if !self.pb_may_write || out.blob_read => {}
            ToShell::PasteboardBlob { types } => {
                out.blob_read = true;
                match fds.first().map(|fd| bifrost_proto::pbblob::read(fd, &types)) {
                    Some(Ok(items)) => out.requests.push(ToShell::PasteboardSet { items }),
                    Some(Err(e)) => eprintln!("springboard: app {} sent a pasteboard write that could not be read: {e}", self.id),
                    None => eprintln!("springboard: app {} sent a pasteboard write without its descriptor", self.id),
                }
            }
            other => out.requests.push(other),
        }
    }

    /// The newest frame: pixels and size (the switcher card, the crash screen).
    pub fn frame(&self) -> Option<(&[u32], usize, usize)> {
        let b = self.bufs.get(self.shown?)?;
        Some((b.map.pixels(), self.buffer_size.0, self.buffer_size.1))
    }

    /// Keeps what arrived for a window that is not open (yet), within `MAX_EARLY` messages.
    fn keep_early(&mut self, e: Early) {
        while self.early.len() >= MAX_EARLY {
            self.early.pop_front();
        }
        self.early.push_back(e);
    }

    /// The window `id` was just opened: what the app sent for it before that is handled now, in the order it came.
    fn replay_early(&mut self, id: u32, out: &mut Pumped) {
        let (mine, rest): (Vec<Early>, Vec<Early>) = std::mem::take(&mut self.early).into_iter().partition(|e| e.window() == id);
        self.early = rest.into();
        for e in mine {
            match e {
                Early::Buffers { id, w, h, generation, fds } => self.window_buffers(id, w, h, generation, fds),
                Early::Frame { id, buf, damage } => self.window_frame_arrived(id, buf, damage, out),
            }
        }
    }

    /// `ToShell::WindowBuffers`: maps one extra window's two buffers, with the main window's size and seal checks. A window
    /// the table does not hold gets nothing.
    fn window_buffers(&mut self, id: u32, w: u32, h: u32, generation: u32, fds: Vec<sys::Owned>) {
        let (w, h) = (w as usize, h as usize);
        let bytes = w * h * 4;
        if fds.len() != 2 || w == 0 || h == 0 || w > 8192 || h > 8192 || !self.window_open(id) {
            return;
        }
        let safe = fds.iter().all(|f| sealed_len_at_least(f, bytes));
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

    /// True once the app has committed a frame to its extra window `id` (the desk maps a window on its first frame).
    pub fn window_mapped(&self, id: u32) -> bool {
        self.window_frame(id).is_some()
    }

    /// Copies an extra window's newest frame into `s` (top left, clipped to `s`), then gives the frame back to the app.
    /// False when the window has no frame yet.
    pub fn blit_window(&mut self, s: &mut Surface, id: u32) -> bool {
        let copied = match self.window_frame(id) {
            Some((px, w, h)) => {
                let n = w.min(s.w);
                for y in 0..h.min(s.h) {
                    let to = y * s.stride;
                    s.px[to..to + n].copy_from_slice(&px[y * w..y * w + n]);
                }
                true
            }
            None => false,
        };
        if copied {
            self.window_blitted(id);
        }
        copied
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
    pub fn touch_window(&mut self, id: u32, touch: u32, kind: TouchKind, x: f32, y: f32, t_ms: u32, mods: bifrost_proto::ModifierFlags) -> bool {
        if !self.window_open(id) {
            return false;
        }
        self.send(&ToApp::WindowTouch { id, touch, kind, x, y, t_ms, mods });
        true
    }

    /// The person dragged window `id` to (x, y) points (the shell's clamped position): the table keeps it and the app is
    /// told (`WindowMoved`). False when the window is not open.
    pub fn window_dragged(&mut self, id: u32, x: f32, y: f32) -> bool {
        if !self.windows.note_position(&self.id, id, x, y) {
            return false;
        }
        self.send(&ToApp::WindowMoved { window: id, x, y });
        true
    }

    /// The person double-clicked the title bar of window `id`. True when the app opted in and was told; otherwise the
    /// shell keeps its own handling (zoom).
    pub fn window_title_double_clicked(&mut self, id: u32) -> bool {
        let wanted = self.windows.of_app(&self.id).iter().any(|w| w.id == id && w.title_double_click);
        if !wanted {
            return false;
        }
        self.send(&ToApp::WindowTitleDoubleClick { window: id });
        true
    }

    /// Whether frames arrived that `blit` has not yet copied.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The app already showed frames and is drawing its first one into a new buffer set (a resize reallocates them):
    /// its window keeps its last pixels instead of being unmapped (dropping the toplevel mid-drag ended the pointer
    /// grab of a resize after a few pixels, E2E-11).
    pub fn between_buffers(&self) -> bool {
        self.running() && self.presented && self.shown.is_none()
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
        self.early.clear();
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

    /// Force Quit: ends the app's process at once with SIGKILL, with no `Quit` first and no time to save.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub fn force_kill(&mut self) {
        if self.child.is_some() && self.status == Status::Running {
            self.sigkill();
        } else if self.child.is_none() {
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
        crate::shell::services::metric_reports::note_close(&self.id);
    }
}

/// The built-in Calculator's package id (Search answers open it when installed).
pub const CALCULATOR: &str = "is.olibuijr.calculator";

/// An installed app's icon at `size` px, from its package (`None` for built-in ids).
pub fn icon_image(id: &str, size: usize) -> Option<crate::gfx::Image> {
    let registered = installed();
    let app = registered.iter().find(|a| a.manifest.id == id)?;
    // the one icon path: the package's icon.json (layers) or its flat icon.png, drawn at `size` px
    let (variant, _) = bifrost_render::icon_doc::current_style();
    let icon = bifrost_render::icon_doc::app_icon(id, &app.icon_path(), size as f32, 1.0, variant)?;
    Some((*icon).clone())
}

use super::{home_zone, keyboard::Out, AppId, Event, Screen, Shell};

/// The system accessibility settings an app needs to draw itself (`bifrost_proto::appearance` bits).
pub fn appearance_flags() -> u32 {
    use bifrost_proto::appearance::*;
    use super::accessibility as a11y;
    [(a11y::increase_contrast(), INCREASE_CONTRAST), (a11y::reduce_transparency(), REDUCE_TRANSPARENCY), (a11y::button_shapes(), SHOW_BORDERS), (a11y::bold_text(), BOLD_TEXT), (bifrost_render::appearance::target_is_light(), LIGHT), (vibrancy_available(), VIBRANCY), (vibrancy_available(), MATERIALS)]
        .into_iter()
        .fold(0, |f, (on, bit)| if on { f | bit } else { f })
        | (bifrost_render::appearance::accent().to_wire() << ACCENT_SHIFT)
        // Highlight Colour, Sidebar Icon Size and Show Scroll Bars: a change is pushed to every app (`sync_appearance`)
        | ((bifrost_render::ui::highlight_index() + 1) << HIGHLIGHT_SHIFT & HIGHLIGHT_MASK)
        | ((bifrost_render::ui::sidebar_icon_size() + 1) << SIDEBAR_ICON_SHIFT & SIDEBAR_ICON_MASK)
        | ((bifrost_render::ui::scroll_bars_mode() + 1) << SCROLL_BARS_SHIFT & SCROLL_BARS_MASK)
}

/// Behind-window vibrancy works only when the GPU presenter blurs the desktop under windows, and not with Reduce
/// Transparency; on the CPU/SHM path windows stay opaque.
pub fn vibrancy_available() -> bool {
    bifrost_render::glass::gpu() && !super::accessibility::reduce_transparency()
}

/// A standard editing shortcut: the command, and the key equivalent it has on the keyboard (evdev key, its letter and the
/// CHORD_* bits of Cmd with it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EditShortcut {
    command: bifrost_proto::Command,
    key: u32,
    letter: char,
    mods: u32,
}

/// The editing command a shortcut's `Out` names (`Shortcut::out`).
fn edit_command_of(name: &str) -> Option<EditShortcut> {
    use bifrost_proto::{Command as C, CHORD_META};
    let (command, key, letter, mods) = match name {
        "Copy" => (C::Copy, 46, 'c', CHORD_META),
        "Cut" => (C::Cut, 45, 'x', CHORD_META),
        "Paste" => (C::Paste, 47, 'v', CHORD_META),
        "SelectAll" => (C::SelectAll, 30, 'a', CHORD_META),
        "Undo" => (C::Undo, 44, 'z', CHORD_META),
        "Redo" => (C::Redo, 44, 'z', CHORD_META | CHORD_SHIFT),
        _ => return None,
    };
    Some(EditShortcut { command, key, letter, mods })
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
            ToShell::SelectionReply { request, text } => self.selection_reply(slot, request, &text, t),
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
        // an uninstalled app keeps its slot but has nothing to start
        if app.is_removed() {
            return;
        }
        // An app that speaks another SDK protocol is started once and then left alone until its binary changes
        // (a new installed version has a new path): no respawn loop, and the window keeps saying why.
        if is_known_incompatible(&app.manifest.id, &app.exec_path()) {
            return;
        }
        if let Some(h) = self.ext[slot].as_mut().filter(|h| h.running()) {
            // an app with unsaved changes keeps running at its old version until they are saved or closed: the restart
            // would lose them (the update relaunch waits the same way, `Shell::app_update_relaunch`)
            if !h.is_outdated(&app.exec_path()) || h.doc_edited {
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
                super::entity_index::start(&app.manifest.id, &app.manifest.entity_types);
                self.usernotif_started(slot);
                self.extras_started(slot);
                self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidLaunchApplication, slot as u8);
            }
            Err(e) => {
                bifrost_core::log::shell_logger("apphost").error(format!("cannot start {}: {e}", app.manifest.id));
                // the person is told which app and why (a package that asks for a permission this system does not offer)
                self.launch_failed_because(slot, launch_failure_reason(&e));
                // never leave an empty (black) window behind a launch that failed
                #[cfg(feature = "desktop-wayland")]
                self.desk_close_app(AppId::External(slot as u8));
            }
        }
    }

    /// An installed app did not start (the spawn failed, or it exited before saying hello): the user gets an alert
    /// from the app, and the app's Dock launch bounce stops instead of running out its timeout.
    pub(super) fn launch_failed(&mut self, slot: usize) {
        self.launch_failed_because(slot, None);
    }

    /// [`Shell::launch_failed`] with the reason, when the shell knows it (see [`launch_failure_reason`]).
    pub(super) fn launch_failed_because(&mut self, slot: usize, reason: Option<String>) {
        let registered = installed();
        let Some(app) = registered.get(slot) else { return };
        let (title, body) = launch_failure_text_for(&app.manifest.name, reason.as_deref());
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

    /// Force Quit: the app in `slot` is killed at once (`AppHost::force_kill`) and dropped, with no `Quit` and no grace
    /// period, so an app that stopped answering goes away. Its menu extras and the workspace observers hear of it as for a
    /// normal quit.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn ext_force_quit(&mut self, slot: usize) {
        if let Some(mut h) = self.ext.get_mut(slot).and_then(Option::take) {
            h.force_kill();
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

    /// Tells the running apps about the pasteboard they have not seen; `written` lists the apps that made the newest writes
    /// and the change count each write gave (those apps have the board already). The app in front is given the contents
    /// (`ToApp::Pasteboard` or the blob form, and the text); every other app only hears that the board changed
    /// (`ToApp::PasteboardChanged`), so an app in the background never reads what was copied. One copy of the contents is
    /// made, for the one app in front.
    pub(super) fn ext_sync_pasteboard(&mut self, written: &[(usize, u32)]) {
        for &(slot, count) in written {
            if let Some(h) = self.ext.get_mut(slot).and_then(Option::as_mut) {
                h.pasteboard_known(count);
            }
        }
        let count = super::pasteboard::count();
        let front = self.ext_fg.map(usize::from);
        let stale = |slot: usize, h: &AppHost| if Some(slot) == front { h.pb_known() != Some(count) } else { !h.pb_told(count) };
        if !self.ext.iter().enumerate().any(|(slot, h)| h.as_ref().is_some_and(|h| stale(slot, h))) {
            return;
        }
        let mut contents = None;
        for (slot, h) in self.ext.iter_mut().enumerate() {
            let Some(h) = h else { continue };
            if !stale(slot, h) {
                continue;
            }
            if Some(slot) == front {
                let (count, items) = contents.get_or_insert_with(super::pasteboard::snapshot);
                h.sync_pasteboard(*count, items);
            } else {
                h.notify_pasteboard(count);
            }
        }
    }

    /// Sends `m` to the app in `slot` if it is running.
    pub(super) fn ext_send(&mut self, slot: usize, m: &bifrost_proto::ToApp) {
        if let Some(h) = self.ext.get_mut(slot).and_then(Option::as_mut) {
            h.send(m);
        }
    }

    /// Asks each running app that declares `index_extension` and has no items in the Search index to index again. Each
    /// app is asked once per run, so an app that indexes nothing is not asked on every open of the Search panel.
    pub(super) fn request_index_extensions(&mut self) {
        let apps = installed();
        for (slot, a) in apps.iter().enumerate() {
            let running = self.ext.get(slot).and_then(Option::as_ref).is_some();
            if !running || !a.manifest.index_extension || super::spotindex::has_items(&a.manifest.id) {
                continue;
            }
            let Ok(mut asked) = REINDEX_ASKED.lock() else { return };
            if asked.iter().any(|id| *id == a.manifest.id) {
                continue;
            }
            asked.push(a.manifest.id.clone());
            drop(asked);
            self.ext_send(slot, &bifrost_proto::ToApp::ReindexAll);
        }
    }

    pub(super) fn ext_has_frame(&self, slot: usize) -> bool {
        self.ext.get(slot).and_then(Option::as_ref).is_some_and(|h| h.frame().is_some())
    }

    pub(super) fn ext_touch(&mut self, slot: usize, ev: Event, t: f64) {
        // the drag emulated for a wheel event the app took (`pointer`) is not a touch
        if let Some(Some(h)) = self.ext.get_mut(slot) {
            if h.swallows_touch(t) {
                return;
            }
        }
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
        h.touch(kind, 0, x, y, t as u32, self.key_mods);
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
        // two fingers also make the pinch and rotation events of an app that asked for them
        self.pinch_contact(n, id, phase, x, y);
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
        self.sync_key_windows();
        match self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            Some(Some(h)) => {
                // a key equivalent of the app's own menu is a menu action, not a key (`performKeyEquivalent:`)
                if let Some(id) = super::evkeys::chord_char_for(key as u16).and_then(|c| super::desktop_bar::app_menu_chord(&h.app_menus, c, mods)) {
                    h.input(WindowInput::MenuAction(id));
                    return true;
                }
                h.key(KeyInput::chord(key, mods));
                true
            }
            _ => false,
        }
    }

    pub(super) fn ext_type_out(&mut self, out: Out, t: f64) {
        self.sync_key_windows();
        if out == Out::Hide {
            self.ext_set_keyboard(false, t);
            return;
        }
        // an installed input method that is the selected source takes the key before the field does
        if self.ext_text_focused() {
            if let Some(slot) = self.ext_fg {
                if let Some(Some(app)) = self.ext.get_mut(slot as usize) {
                    if self.input_providers.app_key(slot, &out, app) == bifrost_ui::imk_host::KeyResult::Consumed {
                        self.dirty = true;
                        return;
                    }
                }
            }
        }
        // Cmd+A, C, X, V, Z (and the Ctrl forms): the standard editing command, as the Edit menu would send it
        if let Out::Named(name) = out {
            if let Some(edit) = edit_command_of(name) {
                self.ext_edit_command(edit);
                return;
            }
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
            // Back in an external app: the app closes what it has open
            Out::Named("Escape") => KeyInput::Code(1),
            _ => return,
        };
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.key(key);
        }
    }

    /// A standard editing shortcut for the foreground app. The app's own enabled menu item on the chord wins
    /// (`performKeyEquivalent:`); otherwise the app gets the standard command (`Input::Command`) when it reported that it
    /// takes it, and the chord itself when it did not, so an app that reads chords still hears it.
    fn ext_edit_command(&mut self, edit: EditShortcut) {
        self.sync_key_windows();
        let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) else { return };
        if let Some(id) = super::desktop_bar::app_menu_chord(&h.app_menus, edit.letter, edit.mods) {
            h.input(WindowInput::MenuAction(id));
        } else if h.commands.supports(edit.command) {
            h.input(WindowInput::Command(edit.command));
        } else {
            h.key(KeyInput::chord(edit.key, edit.mods));
        }
    }

    /// True when an SDK app is the screen in front (its window takes the keys).
    pub fn sdk_app_front(&self) -> bool {
        matches!(self.screen, Screen::App(AppId::External(_)))
    }

    /// The foreground app's field traits to the keyboard: the return key label (`ReturnKeyType::basic`). Secure and
    /// content kinds are kept for the keyboard app's layouts once it draws them (docs/bifrost-ui.md, gaps).
    fn ext_apply_traits(&mut self, t: bifrost_proto::InputTraits) {
        self.keyboard.set_return_key(super::textinput::return_key(t));
    }

    /// Sends input to the key window of the foreground external app (an extra window's carries its id), if there is one.
    pub(super) fn ext_fg_input(&mut self, input: WindowInput) {
        if let Some(Some(h)) = self.ext_fg.and_then(|n| self.ext.get_mut(n as usize)) {
            h.input(input);
        }
    }

    /// True when the SDK app in front edits text from the keyboard: it declares `edit_keys`, or has a text field focused
    /// (it reported the field's caret). Ctrl+A is then the line start and Ctrl+Y the yank; otherwise they are Select All and Redo.
    pub fn ext_takes_edit_keys(&self) -> bool {
        let Screen::App(AppId::External(slot)) = self.screen else { return false };
        installed().get(slot as usize).is_some_and(|a| a.manifest.edit_keys) || self.ext_text_focused()
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
            h.input(super::textinput::to_input(c));
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
        // the newest clipboard text an app asked for, and whether it must stay off the disk (a secret)
        let mut clip: Option<(String, bool)> = None;
        // the apps that wrote the pasteboard in this pass, and the apps that sent only text
        let mut pb_from: Vec<usize> = Vec::new();
        let mut clip_from: Vec<usize> = Vec::new();
        let mut pb_reads: Vec<usize> = Vec::new();
        let mut context=None;
        let mut open_url=None;
        let mut badge_changed=false;
        let mut full_screen: Vec<(usize, bool)> = Vec::new();
        // main-window content sizes the apps asked for (`ToShell::SetContentSize`), applied once the loop is done
        let mut content_sizes: Vec<(u8, (f32, f32))> = Vec::new();
        let mut notifications = Vec::new();
        let mut rich_msgs = Vec::new();
        #[cfg(feature = "desktop-wayland")]
        let mut docs = Vec::new();
        let mut died = Vec::new();
        let mut vanished: Vec<u8> = Vec::new();
        // apps that exited before saying hello never started: the user is told (`launch_failed`)
        let mut failed = Vec::new();
        // direct apps that fell back to the socket path: their window opens now (after the loop, the shell is free)
        let mut fell_back = Vec::new();
        for (slot, h) in self.ext.iter_mut().enumerate() {
            let Some(h) = h else { continue };
            if fg == Some(slot as u8) {
                h.fg_seen_ms = now;
            }
            h.pb_may_write = h.may_write_clipboard(fg == Some(slot as u8), now);
            h.sync_appearance(appearance);
            h.sync_input_source(japanese);
            if fg == Some(slot as u8) && h.running() && h.size() != (cfg.w as usize, cfg.h as usize) {
                h.configure(cfg);
            }
            let p = h.pump();
            if p.died {
                h.exit_reported = true;
                died.push(slot as u8);
                if !h.hello && !h.incompatible {
                    failed.push(slot);
                }
            } else if h.take_exit() {
                // gone without `pump` seeing it (a send failed, a stall or a kill ended it): its windows must still go
                vanished.push(slot as u8);
                self.dirty = true;
            }
            if h.take_fallback() {
                fell_back.push(slot as u8);
            }
            if fg == Some(slot as u8) {
                if p.died || p.chrome {
                    // the app went, or changed something the shell draws (a title, its menus, a window's size)
                    self.dirty = true;
                } else if p.frame {
                    // new pixels alone: the desktop republishes the windows, nothing else is drawn again
                    self.ext_frame = true;
                }
            }
            for r in p.requests {
                match r {
                    ToShell::ContextMenu{x,y,items} if fg==Some(slot as u8)=>{if x.is_finite()&&y.is_finite()&&!items.is_empty()&&items.len()<=32 {context=Some((slot as u8,x,y,items));}},
                    ToShell::WantsKeyboard { on } if fg == Some(slot as u8) => kb = Some(on),
                    ToShell::InputTraits(t) if fg == Some(slot as u8) => traits = Some(t),
                    // only the app in front (or one that was a moment ago) writes the clipboard: an app in the background cannot
                    // replace what the user copied
                    ToShell::SetClipboard { text } if h.may_write_clipboard(fg == Some(slot as u8), now) => {
                        clip = Some((text, false));
                        clip_from.push(slot);
                    }
                    ToShell::PasteboardSet { items } if h.may_write_clipboard(fg == Some(slot as u8), now) => {
                        // the marker types make the copy a secret: kept in memory only, and cleared on a timer
                        let (text, kind) = super::pasteboard::set_marked(items, now);
                        clip = Some((text.unwrap_or_default(), kind.is_private()));
                        pb_from.push(slot);
                    }
                    ToShell::PasteboardRead if fg == Some(slot as u8) => pb_reads.push(slot),
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
                    ToShell::SetFullScreen { on } => full_screen.push((slot, on)),
                    ToShell::SetContentSize { w, h } => content_sizes.push((slot as u8, (w, h))),
                    ToShell::SetContentLimits { min, max } => super::desk::set_content_limits(slot as u8, min, max),
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
                    // a link opens the Browser and takes focus: only the app in front (the one the person is using) may ask,
                    // and not more than once a second
                    ToShell::OpenUrl { url } if open_url_allowed(fg == Some(slot as u8), &mut h.last_open_url, t) => open_url=Some(url),
                    ToShell::PlaySystemSound { name } => queue_system_sound(name),
                    r @ ToShell::Share { .. } if fg == Some(slot as u8) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::DragBegin { .. } | ToShell::DragRegister { .. } | ToShell::DragReply { .. } | ToShell::DragPerformed { .. } | ToShell::DragCancel { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::LoginItem { .. } | ToShell::StatusItemSet { .. } | ToShell::StatusItemRemove { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ ToShell::DistPost { .. } => rich_msgs.push((slot as u8, r)),
                    r @ ToShell::ServiceCall { .. } => rich_msgs.push((slot as u8, r)),
                    // the answer to a SelectionRequest (Look Up, Services); `selection_reply` drops one it did not ask for
                    r @ ToShell::SelectionReply { .. } => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::Preview { .. } | ToShell::PreviewClose) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::WorkspaceRequest { .. } | ToShell::WorkspaceSubscribe { .. }) => rich_msgs.push((slot as u8, r)),
                    r @ (ToShell::NotifyCategories(_) | ToShell::NotifyRequest { .. } | ToShell::NotifyRemove { .. } | ToShell::NotifyQuery { .. } | ToShell::NotifySettings { .. } | ToShell::NotifyAuthorize { .. } | ToShell::NotifySchedule { .. }) => rich_msgs.push((slot as u8, r)),
                    #[cfg(feature = "desktop-wayland")]
                    r @ (ToShell::DocOpen { .. } | ToShell::DocOpenEx { .. } | ToShell::DocSave { .. } | ToShell::DocCommit { .. } | ToShell::DocAbort { .. } | ToShell::DocReopen { .. } | ToShell::DocRelease { .. } | ToShell::DocSaveGrant { .. } | ToShell::DocGrantPath { .. }) => docs.push((slot as u8, r)),
                    _ => {}
                }
            }
        }
        #[cfg(feature = "desktop-wayland")]
        {
            for &slot in died.iter().chain(&vanished) { self.doc_app_gone(slot); }
            for (slot, r) in docs { self.doc_request(slot, r); }
            self.doc_keep_shown();
        }
        for &slot in &died { self.dnd_app_gone(slot); self.extras_app_gone(slot); self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidTerminateApplication, slot); }
        // a failed exit gets the "quit unexpectedly" alert (read before the host is dropped); the shell keeps running
        for &slot in &died { self.app_crashed(slot as usize); }
        for (id, code) in late_crashes() { self.app_crashed_late(&id, code); }
        // a window whose app has exited must not keep its last frame on screen
        for &slot in &vanished { self.dnd_app_gone(slot); self.extras_app_gone(slot); self.workspace_app_event(bifrost_proto::WorkspaceEventKind::DidTerminateApplication, slot); }
        // an app's content limits last as long as it runs: the next launch starts from the manifest's again
        for slot in died.into_iter().chain(vanished) { super::desk::forget_content_limits(slot); self.desk_close_app(super::AppId::External(slot)); }
        for slot in failed { self.launch_failed(slot); }
        if self.desk.is_some() {
            for slot in fell_back { if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running()) { self.desk_open(super::AppId::External(slot), t); } }
        }
        // the extra windows follow each app's table: opened ones get a desk window, closed ones leave the desk
        self.desk_sync_extra_windows();
        self.sync_key_windows();
        if badge_changed { self.dirty = true; }
        for (slot, on) in full_screen { self.desk_set_full_screen(super::AppId::External(slot as u8), on); }
        for (slot, size) in content_sizes { self.desk_set_content_size(super::AppId::External(slot), size); }
        let indexed = rich_msgs.iter().any(|(_, r)| matches!(r, ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }));
        for (slot, r) in rich_msgs { self.rich_msg(slot, r, t); }
        self.services_tick(t);
        if indexed { super::spotindex::flush(); }
        self.share_tick();
        self.usernotif_tick(t);
        for notification in notifications { self.notify(notification); }
        if let Some(url)=open_url{if let Err(e)=self.desktop_open_url(&url,t){eprintln!("BifrOSt web link: {e}");}}
        if let Some((slot,x,y,items))=context {if let Some(d)=&self.desk {if let Some(i)=d.index_of(super::AppId::External(slot)){let b=d.wins[i].body(&self.desk_metrics());self.desktop_context=Some(super::desktop_bar::ContextMenu{slot,x:x+b.x as f32,y:y+b.y as f32,items});self.desktop_menu=Some(super::desktop_bar::Menu::Context);#[cfg(feature="desktop-wayland")] {self.desktop_menu_row=None;}self.dirty=true;}}}
        if let Some((text, private)) = clip {
            if private {
                super::clipboard::set_text_volatile(&text);
            } else {
                super::clipboard::set_text(&text);
            }
        }
        // an app that asked for the pasteboard (it is in front) is given it again on this pass
        for slot in pb_reads {
            if let Some(h) = self.ext.get_mut(slot).and_then(Option::as_mut) {
                h.pasteboard_wanted();
            }
        }
        // one clipboard for the shell's fields, the apps and the window server's selection (`pasteboard_sync`): the pass
        // folds a copy in a shell field, an app's text and another program's selection into the pasteboard and offers it;
        // the writer of the newest typed write has that board already
        let mut written: Vec<(usize, u32)> = pb_from.into_iter().map(|slot| (slot, super::pasteboard::count())).collect();
        super::pasteboard_sync::tick();
        // an app that sent only text has the board as it was after the pass (the text is the board's item)
        written.extend(clip_from.into_iter().map(|slot| (slot, super::pasteboard::count())));
        self.ext_sync_pasteboard(&written);
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
            for id in super::atspi::sync(&mut self.ext, t, chrome) {
                self.ax_chrome_press(id, t);
            }
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

    /// The desk copies an extra window's newest frame into its body (`s`).
    pub(super) fn ext_window_blit(&mut self, s: &mut Surface, slot: usize, id: u32) {
        if let Some(Some(h)) = self.ext.get_mut(slot) {
            h.blit_window(s, id);
        }
    }

    pub(super) fn ext_blit_damage(&mut self, s: &mut Surface, slot: usize, damage: &mut Damage) {
        if let Some(Some(h)) = self.ext.get_mut(slot) {
            let d = h.blit(s, false);
            damage.union(&d);
        }
    }

    pub(super) fn describe_ext(&self) -> String {
        #[allow(unused_mut)]
        let mut run: Vec<String> = self.ext.iter().flatten().filter(|h| h.running()).map(|h| h.id.clone()).collect();
        // a native app the shell launched has no app host: its toplevels are the window server's, and it runs while it
        // shows one (E2E N2: FilmCraft was missing from `apps_running` while it held the focus)
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_ref() {
            let installed = installed();
            let manifests: Vec<(&str, &str, &str)> = installed.iter().map(|a| (a.manifest.id.as_str(), a.manifest.window.as_str(), a.manifest.wayland_app_id.as_str())).collect();
            for id in native_running(&manifests, d.native_windows.iter().map(|w| w.app_id.as_str())) {
                if !run.contains(&id) { run.push(id); }
            }
        }
        let run: Vec<&str> = run.iter().map(String::as_str).collect();
        let controls: Vec<(&str, usize, u32)> = self.ext.iter().flatten().map(|h| (h.id.as_str(), h.controls.len(), h.commands.0)).collect();
        #[allow(unused_mut)]
        let mut text = ext_state_text(&run, &controls);
        // every other client's window the shell lists (native and Linux apps), so a test can see which one is active
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_ref() {
            text.push_str(&foreign_state_line(d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter()))));
        }
        text
    }
}

/// The installed native apps (`window = "native"`; `manifests` is id, window, Wayland app id) with a toplevel among
/// `window_app_ids`, by manifest id, once each in manifest order.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
fn native_running<'a>(manifests: &[(&str, &str, &str)], window_app_ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let shown: Vec<&str> = window_app_ids.collect();
    manifests.iter().filter(|(_, window, app_id)| *window == "native" && !app_id.is_empty() && shown.contains(app_id)).map(|(id, ..)| id.to_string()).collect()
}

/// The `foreign_windows` line of the shell state text: each other client's window the shell lists as
/// `app id:window id:active` (1 or 0), so a test sees which window holds the focus. Ends with a newline.
#[cfg(feature = "desktop-wayland")]
fn foreign_state_line<'a>(windows: impl Iterator<Item = &'a crate::bare::wayland::ForeignWindow>) -> String {
    let mut seen = std::collections::HashSet::new();
    let items: Vec<String> = windows.filter(|w| seen.insert(w.id)).map(|w| format!("{}:{}:{}", w.app_id, w.id, w.active as u8)).collect();
    format!("foreign_windows={}\n", items.join(","))
}

/// The `apps_running` and `app_controls` lines of the shell state text (`controls` is app id, control count, command
/// bits). Both lines end with a newline, so the section that follows (the `window=` lines) starts its own line.
fn ext_state_text(running: &[&str], controls: &[(&str, usize, u32)]) -> String {
    let controls: Vec<String> = controls.iter().map(|(id, n, commands)| format!("{id}:{n}:{commands:x}")).collect();
    format!("apps_running={}\napp_controls={}\n", running.join(","), controls.join(","))
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

/// A caret rectangle in window points as px at `scale` (points to pixels); a scale that is not positive counts as 1.
pub fn caret_points_to_px((x, y, w, h): (f32, f32, f32, f32), scale: f32) -> (f32, f32, f32, f32) {
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    (x * s, y * s, w * s, h * s)
}

#[cfg(test)]
mod tests {
    /// E2E10: an app reports its caret in window points; Look Up and the candidate window place themselves in px.
    #[test]
    fn a_caret_rect_in_points_becomes_px_at_the_apps_scale() {
        assert_eq!(super::caret_points_to_px((100.0, 40.0, 2.0, 16.0), 1.5625), (156.25, 62.5, 3.125, 25.0));
        assert_eq!(super::caret_points_to_px((10.0, 20.0, 1.0, 17.0), 1.0), (10.0, 20.0, 1.0, 17.0));
        assert_eq!(super::caret_points_to_px((10.0, 20.0, 1.0, 17.0), 0.0), (10.0, 20.0, 1.0, 17.0), "no scale yet");
    }

    #[test]
    fn audio_unit_tables_declare_the_processors_an_app_ships() {
        let m = Manifest::parse("id = \"a.synth\"\nname = \"Synth\"\nexec = \"x\"\n\n[[audio_unit]]\nname = \"Pad\"\nkind = \"aumu\"\nsub_kind = \"pads\"\nmanufacturer = \"Exmp\"\noutputs = [2]\n\n[[audio_unit]]\nname = \"Broken\"\nkind = \"aufx\"\n").unwrap();
        assert_eq!(m.audio_units.len(), 1, "a table that fails the checks is left out");
        assert_eq!(m.audio_units[0].name, "Pad");
        assert_eq!(m.name, "Synth", "an audio unit table does not change the app keys");
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n").unwrap().audio_units.is_empty());
    }
    #[test]
    fn package_types_are_declared_in_the_manifest() {
        let m = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\npackage_types = [\"rtfd\"]\n").unwrap();
        assert_eq!(m.package_types, vec!["rtfd".to_string()]);
        let none = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap();
        assert!(none.package_types.is_empty());
    }
    #[test]
    fn records_audio_is_off_unless_declared() {
        let m = Manifest::parse("id = \"is.a.rec\"\nname = \"Rec\"\nexec = \"x\"\nrecords_audio = true\n").unwrap();
        assert!(m.records_audio);
        let no = Manifest::parse("id = \"is.a.rec\"\nname = \"Rec\"\nexec = \"x\"\n").unwrap();
        assert!(!no.records_audio);
    }

    #[test]
    fn edit_keys_is_declared_by_text_editors_only() {
        let m = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\nedit_keys = true\n").unwrap();
        assert!(m.edit_keys);
        let no = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\nedit_keys = false\n").unwrap();
        assert!(!no.edit_keys);
        assert!(!Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap().edit_keys);
    }
    #[test]
    fn file_manager_extensions_declare_the_folders_they_watch() {
        let m = Manifest::parse("id = \"is.a.cloud\"\nname = \"Cloud\"\nexec = \"x\"\nfile_manager_folders = [\"~/Cloud\", \"/mnt/sync\", \"~/\", \"/\", \"docs\", \"~/../etc\", \"/mnt/sync\"]\n").unwrap();
        assert_eq!(m.file_manager_folders.len(), 7);
        let home = Path::new("/home/ann");
        assert_eq!(m.file_manager_dirs(home), vec![PathBuf::from("/home/ann/Cloud"), PathBuf::from("/mnt/sync")]);
        let none = Manifest::parse("id = \"is.a.edit\"\nname = \"Edit\"\nexec = \"x\"\n").unwrap();
        assert!(none.file_manager_dirs(home).is_empty());
    }
    #[test]
    fn quit_plan_kills_terminable_apps_and_gives_the_rest_a_deadline() {
        assert_eq!(super::quit_plan(true), super::QuitPlan::KillNow);
        assert_eq!(super::quit_plan(false), super::QuitPlan::QuitThenKill(super::QUIT_GRACE_MS));
    }
    #[test]
    fn entity_tables_declare_the_types_search_indexes() {
        let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[entity]]\ntype = \"Note\"\n\n[[entity]]\ntype = \"Folder\"\nname = \"ignored\"\n\n[[intent]]\nid = \"open\"\ntitle = \"Open\"\n").unwrap();
        assert_eq!(m.entity_types, vec!["Note", "Folder"]);
        assert_eq!(m.name, "Notes", "an entity table does not change the app keys");
        assert_eq!(m.intents.len(), 1);
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n").unwrap().entity_types.is_empty());
    }

    #[test]
    fn control_tables_reach_the_manifest_for_control_center() {
        let m = Manifest::parse("id = \"a.b\"\nname = \"Notes\"\nexec = \"x\"\n\n[[control]]\nkind = \"toggle\"\ntitle = \"Quiet Hours\"\nsymbol = \"moon\"\nintent = \"set-quiet\"\n").unwrap();
        assert_eq!(m.controls.len(), 1);
        assert_eq!((m.controls[0].title.as_str(), m.controls[0].intent.as_str()), ("Quiet Hours", "set-quiet"));
        assert_eq!(m.name, "Notes", "a control table does not change the app keys");
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
    fn the_pasteboard_reaches_an_app_once_per_change_and_not_back_to_its_writer() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let next = |peer: &Channel| match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
            _ => None,
        };
        let text = |s: &str| vec![("public.utf8-plain-text".to_string(), s.as_bytes().to_vec())];
        host.sync_pasteboard(0, &[]);
        assert_eq!(next(&peer), None, "an empty board at first contact sends nothing");
        host.sync_pasteboard(3, &text("from another app"));
        assert_eq!(next(&peer), Some(ToApp::Pasteboard { change_count: 3, items: text("from another app") }));
        assert_eq!(next(&peer), Some(ToApp::Clipboard { text: "from another app".into() }), "the text fields read the plain clipboard");
        host.sync_pasteboard(3, &text("from another app"));
        assert_eq!(next(&peer), None, "unchanged: nothing sent");
        host.pasteboard_known(4);
        host.sync_pasteboard(4, &text("what this app wrote"));
        assert_eq!(next(&peer), None, "the writer has its own board");
        host.sync_pasteboard(5, &[("public.png".to_string(), vec![1])]);
        assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 5, .. })));
        assert_eq!(next(&peer), Some(ToApp::Clipboard { text: String::new() }), "a board without text clears the app's text");
    }

    #[test]
    fn a_large_pasteboard_reaches_the_app_in_front_through_a_descriptor_and_is_never_cut() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let big = vec![("public.png".to_string(), vec![9u8; 200_000]), ("public.utf8-plain-text".to_string(), b"a picture".to_vec())];
        host.sync_pasteboard(5, &big);
        let (packet, mut fds) = match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, fds) => (p, fds),
            _ => panic!("nothing sent"),
        };
        let Ok(ToApp::PasteboardBlob { change_count, types }) = ToApp::decode(&packet) else { panic!("a blob message") };
        assert_eq!(change_count, 5);
        let fd = fds.pop().expect("the descriptor travels with the message");
        assert_eq!(bifrost_proto::pbblob::read(&fd, &types).unwrap(), big, "every byte of every item");
        assert!(matches!(peer.recv(false).unwrap(), sys::Recv::Empty), "no cut text follows");
        host.sync_pasteboard(5, &big);
        assert!(matches!(peer.recv(false).unwrap(), sys::Recv::Empty), "unchanged: nothing sent");
    }

    #[test]
    fn an_app_in_the_background_hears_the_count_and_never_the_contents() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let next = |peer: &Channel| match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, _) => Some(ToApp::decode(&p).unwrap()),
            _ => None,
        };
        host.notify_pasteboard(8);
        assert_eq!(next(&peer), Some(ToApp::PasteboardChanged { change_count: 8 }));
        host.notify_pasteboard(8);
        assert_eq!(next(&peer), None, "once per change");
        // coming to the front: the contents follow, and a board the app wrote itself needs no message at all
        host.sync_pasteboard(8, &[("public.utf8-plain-text".to_string(), b"copied elsewhere".to_vec())]);
        assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 8, .. })));
        assert!(matches!(next(&peer), Some(ToApp::Clipboard { .. })));
        host.pasteboard_known(9);
        host.notify_pasteboard(9);
        assert_eq!(next(&peer), None, "the writer is not told about its own copy");
        // asking for the board again resets what the app is known to have
        host.pasteboard_wanted();
        host.sync_pasteboard(9, &[("public.utf8-plain-text".to_string(), b"again".to_vec())]);
        assert!(matches!(next(&peer), Some(ToApp::Pasteboard { change_count: 9, .. })));
    }

    #[test]
    fn only_an_app_in_front_or_just_left_may_write_the_clipboard() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        assert!(host.may_write_clipboard(true, 1e6));
        assert!(!host.may_write_clipboard(false, 1e6), "an app that was never in front cannot");
        host.fg_seen_ms = 1e6;
        assert!(host.may_write_clipboard(false, 1e6 + PB_WRITE_GRACE_MS), "a copy and a quick switch still land");
        assert!(!host.may_write_clipboard(false, 1e6 + PB_WRITE_GRACE_MS + 1.0));
    }

    #[test]
    fn a_pasteboard_write_through_a_descriptor_becomes_an_ordinary_write_and_a_bad_one_is_dropped() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        host.pb_may_write = true;
        let items = vec![("public.png".to_string(), vec![3u8; 100_000]), ("org.nspasteboard.ConcealedType".to_string(), Vec::new())];
        let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
        let mut pumped = Pumped::default();
        host.handle(ToShell::PasteboardBlob { types: types.clone() }, vec![fd], &mut pumped);
        assert_eq!(pumped.requests, vec![ToShell::PasteboardSet { items }]);
        // no descriptor, or one shorter than the layout says: nothing is written
        let mut none = Pumped::default();
        host.handle(ToShell::PasteboardBlob { types }, vec![], &mut none);
        assert!(none.requests.is_empty());
        let (short, _) = bifrost_proto::pbblob::write(&[("t".to_string(), vec![1u8; 4])]).unwrap();
        host.handle(ToShell::PasteboardBlob { types: vec![("t".to_string(), 4000)] }, vec![short], &mut none);
        assert!(none.requests.is_empty());
    }

    #[test]
    fn a_background_apps_blob_is_not_read_and_only_one_blob_is_read_per_pump() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let items = vec![("public.png".to_string(), vec![3u8; 100_000])];
        // an app that may not write: the descriptor is dropped unread, no request, no allocation for its layout
        let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
        let mut pumped = Pumped::default();
        host.handle(ToShell::PasteboardBlob { types }, vec![fd], &mut pumped);
        assert!(pumped.requests.is_empty());
        assert!(!pumped.blob_read);
        // an app that may write: the first blob of a pump is read, the next ones are dropped
        host.pb_may_write = true;
        for _ in 0..3 {
            let (fd, types) = bifrost_proto::pbblob::write(&items).unwrap();
            host.handle(ToShell::PasteboardBlob { types }, vec![fd], &mut pumped);
        }
        assert_eq!(pumped.requests, vec![ToShell::PasteboardSet { items }]);
    }

    #[test]
    fn a_blob_waits_for_the_backlog_and_a_full_socket_does_not_copy_the_board_again() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        let board = vec![("public.png".to_string(), vec![5u8; 100_000])];
        // fill the socket until it refuses
        for _ in 0..100_000 {
            if host.chan.try_send(&[0u8; 4000], &[]).is_err() {
                break;
            }
        }
        host.sync_pasteboard(3, &board);
        assert_eq!(host.pb_known(), None, "the socket was full: nothing was sent");
        let first = host.pb_blob.as_ref().map(|(c, fd, _)| (*c, fd.0)).expect("the copy is kept for the retry");
        host.sync_pasteboard(3, &board);
        assert_eq!(host.pb_blob.as_ref().map(|(c, fd, _)| (*c, fd.0)), Some(first), "the same descriptor, not another copy");
        // a queued older message is ahead of the blob: nothing is built while the backlog is not empty
        host.pb_blob = None;
        host.backlog.push_back(vec![0u8; 8]);
        host.sync_pasteboard(4, &board);
        assert!(host.pb_blob.is_none());
        assert_eq!(host.pb_known(), None);
        drop(peer);
    }

    #[test]
    fn a_board_with_content_is_sent_on_first_contact() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        host.sync_pasteboard(7, &[("public.utf8-plain-text".to_string(), b"kept".to_vec())]);
        let first = match peer.recv(false).unwrap() {
            sys::Recv::Packet(p, _) => ToApp::decode(&p).unwrap(),
            _ => panic!("nothing sent"),
        };
        assert!(matches!(first, ToApp::Pasteboard { change_count: 7, .. }));
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
        let _lock = crate::test_support::global_lock();
        use bifrost_proto::appearance::LIGHT;
        bifrost_render::appearance::jump_to(true);
        assert_ne!(appearance_flags() & LIGHT, 0, "light scheme sends the LIGHT bit");
        bifrost_render::appearance::jump_to(false);
        assert_eq!(appearance_flags() & LIGHT, 0, "dark scheme clears it");
    }

    #[test]
    fn highlight_sidebar_icons_and_scroll_bars_ride_in_the_appearance_flags() {
        let _lock = crate::test_support::global_lock();
        use bifrost_proto::appearance::*;
        use bifrost_render::ui;
        let keep = (ui::highlight_index(), ui::sidebar_icon_size(), ui::scroll_bars_mode());
        ui::set_highlight(4);
        ui::set_sidebar_icon_size(2);
        ui::set_scroll_bars(2);
        let f = appearance_flags();
        assert_eq!(field(f, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), Some(4));
        assert_eq!(field(f, SIDEBAR_ICON_MASK, SIDEBAR_ICON_SHIFT), Some(2));
        assert_eq!(field(f, SCROLL_BARS_MASK, SCROLL_BARS_SHIFT), Some(2));
        ui::set_highlight(0);
        ui::set_sidebar_icon_size(0);
        ui::set_scroll_bars(0);
        let f = appearance_flags();
        assert_eq!(field(f, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), Some(0), "follow the accent is sent, not left unsaid");
        assert_eq!(field(f, SIDEBAR_ICON_MASK, SIDEBAR_ICON_SHIFT), Some(0));
        assert_eq!(field(f, SCROLL_BARS_MASK, SCROLL_BARS_SHIFT), Some(0));
        assert_eq!(field(0, HIGHLIGHT_MASK, HIGHLIGHT_SHIFT), None, "an older shell says nothing");
        ui::set_highlight(keep.0);
        ui::set_sidebar_icon_size(keep.1);
        ui::set_scroll_bars(keep.2);
    }

    #[test]
    fn the_accent_colour_rides_in_the_appearance_flags() {
        let _lock = crate::test_support::global_lock();
        use bifrost_proto::appearance::{ACCENT_MASK, ACCENT_SHIFT};
        use bifrost_render::appearance::{accent, set_accent, Accent};
        let keep = accent();
        for a in Accent::ALL {
            set_accent(a);
            assert_eq!(Accent::from_wire((appearance_flags() & ACCENT_MASK) >> ACCENT_SHIFT), Some(a));
        }
        set_accent(keep);
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
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("1.7.0")).unwrap();
        std::fs::write(dir.path().join("1.7.0/bin"), b"").unwrap();
        let new = std::fs::canonicalize(dir.path().join("1.7.0/bin")).unwrap();
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", channel, None, cfg);
        assert!(!host.is_outdated(&new), "a test host has no binary of record, so it is never outdated");
        host.exec = dir.path().join("1.6.0/bin");
        assert!(host.is_outdated(&new), "the installed version moved on");
        host.exec = new.clone();
        assert!(!host.is_outdated(&new), "same binary: just focus it");
    }

    #[test]
    fn a_dangling_current_during_the_flip_is_not_an_update() {
        // the update replaces `current` (remove, then link) or links it before the binary is in place: while the
        // installed path does not resolve, the running version stays the one of record and nothing is launched
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("apps/x");
        std::fs::create_dir_all(app.join("1.0.0")).unwrap();
        std::fs::write(app.join("1.0.0/bin"), b"").unwrap();
        std::os::unix::fs::symlink("1.0.0", app.join("current")).unwrap();
        let installed = app.join("current/bin");
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("x", channel, None, cfg);
        host.exec = resolved(&installed);
        std::fs::remove_file(app.join("current")).unwrap();
        assert!(!host.is_outdated(&installed), "no current link at all: not outdated");
        std::os::unix::fs::symlink("1.1.0", app.join("current")).unwrap();
        assert!(!host.is_outdated(&installed), "current names a folder not there yet: not outdated");
        std::fs::create_dir_all(app.join("1.1.0")).unwrap();
        assert!(!host.is_outdated(&installed), "the folder is there but the binary is not: still not outdated");
        std::fs::write(app.join("1.1.0/bin"), b"").unwrap();
        assert!(host.is_outdated(&installed), "the new binary is in place: now it is");
    }

    #[test]
    fn an_update_that_flips_current_makes_the_running_host_outdated() {
        // the registry names `current/<exec>` before and after an update; the host keeps the version folder it started from
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("apps/x");
        for v in ["1.0.0", "1.1.0"] {
            std::fs::create_dir_all(app.join(v)).unwrap();
            std::fs::write(app.join(v).join("bin"), b"").unwrap();
        }
        std::os::unix::fs::symlink("1.0.0", app.join("current")).unwrap();
        let installed = app.join("current/bin");
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 4, h: 3, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("x", channel, None, cfg);
        host.exec = resolved(&installed);
        assert!(!host.is_outdated(&installed), "still the installed version");
        std::fs::remove_file(app.join("current")).unwrap();
        std::os::unix::fs::symlink("1.1.0", app.join("current")).unwrap();
        assert!(host.is_outdated(&installed), "current moved to 1.1.0 while 1.0.0 runs");
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
    fn a_web_link_comes_only_from_the_app_in_front_once_a_second() {
        let mut last = None;
        assert!(!open_url_allowed(false, &mut last, 0.), "a background app cannot open a page");
        assert_eq!(last, None, "a refused request does not start the wait");
        assert!(open_url_allowed(true, &mut last, 5000.));
        assert!(!open_url_allowed(true, &mut last, 5500.), "a second request within a second is dropped");
        assert!(open_url_allowed(true, &mut last, 6000.));
    }

    #[test]
    fn a_buffer_is_sealed_before_its_size_is_trusted() {
        let fd = sys::shm_create("seal-order", 4096).unwrap();
        assert!(sealed_len_at_least(&fd, 4096));
        assert!(sys::shm_resize(&fd, 0).is_err(), "the check sealed it: the app can no longer shrink it under the mapping");
        let short = sys::shm_create("seal-short", 64).unwrap();
        assert!(!sealed_len_at_least(&short, 4096), "a file shorter than the claim is refused");
        assert!(sys::shm_resize(&short, 0).is_err(), "and it stays sealed");
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
    fn content_limits_from_the_app_replace_the_manifest_ones_except_on_a_fixed_window() {
        let base = "id = \"a.b\"\nname = \"A\"\nexec = \"a\"\n";
        let free = Manifest::parse(&format!("{base}window_size = \"800x600\"\nmin_size = \"400x300\"\nmax_size = \"1200x900\"\n")).unwrap().size_limits();
        let ours = free.with_content_limits(Some((560.0, 500.0)), None);
        assert_eq!(ours, SizeLimits { initial: Some((800.0, 600.0)), min: Some((560.0, 500.0)), max: Some((1200.0, 900.0)), fixed: false });
        assert_eq!(free.with_content_limits(None, None), free, "no limit of its own keeps the manifest's");
        let fixed = Manifest::parse(&format!("{base}window_size = \"fixed:320x520\"\n")).unwrap().size_limits();
        assert_eq!(fixed.with_content_limits(Some((10.0, 10.0)), Some((2000.0, 2000.0))), fixed);
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
    fn the_window_style_comes_from_the_manifest_beside_the_executable() {
        let root = std::env::temp_dir().join(format!("system-pkgs-window-style-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |dir: &str, extra: &str| {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("manifest.toml"), format!("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n{extra}")).unwrap();
            d.join("x")
        };
        assert_eq!(manifest_window_style(&write("full", "window = \"full-size\"\n")).as_deref(), Some("full-size"));
        assert_eq!(manifest_window_style(&write("classic", "")).as_deref(), Some(""), "a classic window is told apart from an unknown one");
        assert_eq!(manifest_window_style(&root.join("none/x")), None, "no manifest: the app keeps the full-size layout");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn privacy_location_purpose_is_read_from_its_own_table() {
        let text = "id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n[privacy]\nlocation = \"Finna næstu stöð\"\n[[intent]]\nid = \"i\"\nlocation = \"not this\"\n";
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.location_purpose, "Finna næstu stöð");
        assert!(Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"").unwrap().location_purpose.is_empty());
        let after = Manifest::parse("id = \"a.b\"\nname = \"x\"\nexec = \"x\"\n[privacy]\n[window]\nlocation = \"other table\"\n").unwrap();
        assert!(after.location_purpose.is_empty(), "a later table ends [privacy]");
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
    fn the_key_window_gets_the_input_and_the_app_hears_each_focus_change() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        host.set_key_window(4);
        host.input(WindowInput::Command(bifrost_proto::Command::SelectAll));
        host.key(KeyInput::Code(30));
        host.set_key_window(4);
        host.set_key_window(0);
        host.key(KeyInput::Code(9));
        assert_eq!(
            sent(&peer),
            vec![
                ToApp::WindowFocus { window: 0, focused: false },
                ToApp::WindowFocus { window: 4, focused: true },
                ToApp::WindowInput { window: 4, input: WindowInput::Command(bifrost_proto::Command::SelectAll) },
                ToApp::WindowInput { window: 4, input: WindowInput::Key(KeyInput::Code(30)) },
                ToApp::WindowFocus { window: 4, focused: false },
                ToApp::WindowFocus { window: 0, focused: true },
                ToApp::Key(KeyInput::Code(9)),
            ],
            "a repeated focus sends nothing; the main window's keys keep the legacy message"
        );
    }

    #[test]
    fn input_to_reaches_the_named_window_whichever_is_key() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        // a dead key's mark is taken back from the window it was shown in after the focus moved on
        host.input_to(3, WindowInput::UnmarkText { commit: false });
        host.input_to(0, WindowInput::UnmarkText { commit: false });
        let got = sent(&peer);
        assert!(got.ends_with(&[
            ToApp::WindowInput { window: 3, input: WindowInput::UnmarkText { commit: false } },
            ToApp::UnmarkText { commit: false },
        ]), "{got:?}");
    }

    #[test]
    fn marked_text_and_accent_holds_follow_the_key_window() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        host.set_key_window(2);
        host.input(super::super::textinput::to_input(super::super::keyboard::Composition::Preedit { text: "´".into(), sel_start: 2, sel_len: 0 }));
        host.input(WindowInput::AccentHold { letter: 'a' });
        host.set_key_window(0);
        host.input(WindowInput::UnmarkText { commit: true });
        host.input(WindowInput::AccentHold { letter: 'o' });
        let got = sent(&peer);
        assert_eq!(
            got[2..],
            [
                ToApp::WindowInput { window: 2, input: WindowInput::SetMarkedText { text: "´".into(), sel_start: 2, sel_len: 0 } },
                ToApp::WindowInput { window: 2, input: WindowInput::AccentHold { letter: 'a' } },
                ToApp::WindowFocus { window: 2, focused: false },
                ToApp::WindowFocus { window: 0, focused: true },
                ToApp::UnmarkText { commit: true },
                ToApp::AccentHold { letter: 'o' },
            ],
            "an extra key window's composition carries its id; the main window keeps the legacy messages"
        );
    }

    #[test]
    fn a_window_takes_a_position_collapses_and_opts_in_to_the_title_double_click() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        let flags = bifrost_proto::window_flags::TITLE_DOUBLE_CLICK;
        host.handle(ToShell::WindowOpen { id: 4, title: "Note".into(), w: 200.0, h: 150.0, position: Some((12.0, 40.0)), flags }, Vec::new(), &mut out);
        assert_eq!(host.windows.take_pending_move("fixture", 4), Some((12.0, 40.0)), "the requested position is applied once");
        host.handle(ToShell::WindowCollapse { window: 4, collapsed: true }, Vec::new(), &mut out);
        assert!(host.windows.of_app("fixture")[0].collapsed);
        assert!(host.window_title_double_clicked(4));
        assert_eq!(sent(&peer), vec![ToApp::WindowTitleDoubleClick { window: 4 }]);
        assert!(host.window_dragged(4, 100.0, 60.0));
        assert_eq!(sent(&peer), vec![ToApp::WindowMoved { window: 4, x: 100.0, y: 60.0 }]);
        host.handle(ToShell::WindowOpen { id: 5, title: "Plain".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
        assert!(!host.window_title_double_clicked(5), "an app that did not opt in gets no message");
        assert!(sent(&peer).is_empty());
        let cascade = bifrost_proto::window_flags::CASCADE;
        host.handle(ToShell::WindowOpen { id: 6, title: "Viewer".into(), w: 10.0, h: 10.0, position: None, flags: cascade }, Vec::new(), &mut out);
        host.handle(ToShell::WindowOpen { id: 7, title: "Placed".into(), w: 10.0, h: 10.0, position: Some((1.0, 2.0)), flags: cascade }, Vec::new(), &mut out);
        let cascades = |id: u32| host.windows.of_app("fixture").iter().any(|w| w.id == id && w.cascade);
        assert!(cascades(6), "a window opened with the flag and no position cascades");
        assert!(!cascades(7), "a requested position wins over the cascade");
        assert!(!cascades(5), "a plain window does not cascade");
    }

    #[test]
    fn the_window_table_follows_the_messages_the_app_sends() {
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut out);
        host.handle(ToShell::WindowOpen { id: 2, title: "Grace".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut out);
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

    /// The state an app sends again unchanged is free: it asks the shell for no frame at all. Pixels alone ask for a
    /// frame of windows (`frame`), not for the shell to draw anything itself (`chrome`).
    #[test]
    fn identical_state_asks_for_no_frame_and_pixels_alone_are_no_chrome_change() {
        let (mut host, _peer) = super::route_tests::fixture_host();
        let titles = || ToShell::UndoTitles { undo: "Undo Typing".into(), redo: String::new() };
        let mut first = Pumped::default();
        host.handle(titles(), Vec::new(), &mut first);
        assert!(first.frame && first.chrome, "new titles are something the shell draws");
        let mut again = Pumped::default();
        host.handle(titles(), Vec::new(), &mut again);
        assert_eq!(again, Pumped::default(), "the same titles again cost nothing");
        host.handle(ToShell::Materials(Vec::new()), Vec::new(), &mut again);
        host.handle(ToShell::Commands(host.commands), Vec::new(), &mut again);
        assert_eq!(again, Pumped::default(), "nor do the same materials and commands");
        host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 480.0, h: 560.0, position: None, flags: 0 }, Vec::new(), &mut again);
        assert!(again.chrome, "a window opening is drawn by the shell");
        let mut same = Pumped::default();
        host.handle(ToShell::WindowTitle { id: 1, title: "Ada".into() }, Vec::new(), &mut same);
        host.handle(ToShell::WindowResize { id: 1, w: 480.0, h: 560.0 }, Vec::new(), &mut same);
        assert_eq!(same, Pumped::default(), "the same title and size again cost nothing");
        host.handle(ToShell::WindowTitle { id: 1, title: "Ada L.".into() }, Vec::new(), &mut same);
        assert!(same.frame && same.chrome, "a new title is drawn");
        // pixels
        let set = |n: &str| vec![sys::shm_create(n, 48).unwrap(), sys::shm_create(n, 48).unwrap()];
        let mut pixels = Pumped::default();
        host.handle(ToShell::Buffers { w: 4, h: 3, generation: 1 }, set("quiet-pixels"), &mut pixels);
        host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 4, 3]] }, Vec::new(), &mut pixels);
        assert!(pixels.frame, "new pixels are a frame");
        assert!(!pixels.chrome, "and only a frame of windows");
    }

    #[test]
    fn a_window_the_table_refuses_is_closed_again_for_the_app() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        let refused = crate::shell::app_window_table::MAX_PER_APP as u32 + 1;
        for id in 1..=refused {
            host.handle(ToShell::WindowOpen { id, title: "w".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
        }
        assert_eq!(host.windows.of_app("fixture").len(), crate::shell::app_window_table::MAX_PER_APP);
        assert_eq!(sent(&peer), vec![ToApp::WindowClosed { id: refused }]);
    }

    #[test]
    fn a_window_gets_its_own_buffers_and_its_frame_is_given_back_once_blitted() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 1, title: "Ada".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
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

    /// MA6: an SDK built before the fix sent a window's buffers and first frame before its `WindowOpen` (the request waited in
    /// the app's queue until the end of the turn). The shell dropped them, the app never sent the first frame again, and the
    /// window stayed unmapped. They are kept and handled when the window opens.
    #[test]
    fn buffers_and_a_frame_that_come_before_the_window_open_are_kept_until_it_opens() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        let bytes = 2 * 2 * 4;
        let fds = vec![sys::shm_create("early-window-0", bytes).unwrap(), sys::shm_create("early-window-1", bytes).unwrap()];
        host.handle(ToShell::WindowBuffers { id: 1, w: 2, h: 2, generation: 3 }, fds, &mut out);
        host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 2, 2]] }, Vec::new(), &mut out);
        assert!(host.window_frame(1).is_none(), "the window is not open yet");
        host.handle(ToShell::WindowOpen { id: 1, title: "New Message".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
        assert_eq!(host.window_frame(1).map(|(_, w, h)| (w, h)), Some((2, 2)), "the first frame is there once the window is");
        assert!(host.window_mapped(1));
        assert!(sent(&peer).is_empty(), "the app is not told to drop the window");
        // pixels for a window that never opens do not pile up, and a window the table refuses loses its pixels
        for id in 10..40 {
            host.handle(ToShell::WindowFrame { id, buf: 0, damage: Vec::new() }, Vec::new(), &mut out);
        }
        assert!(host.early.len() <= MAX_EARLY, "{} kept", host.early.len());
        host.handle(ToShell::WindowOpen { id: 39, title: "Late".into(), w: 2.0, h: 2.0, position: None, flags: 0 }, Vec::new(), &mut out);
        assert!(host.early.iter().all(|e| e.window() != 39), "what was kept for a window is used up when it opens");
        host.handle(ToShell::WindowClose { id: 1 }, Vec::new(), &mut out);
        assert!(host.window_frame(1).is_none());
    }

    #[test]
    fn the_shell_closing_a_window_drops_it_and_tells_the_app_once() {
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 3, title: "Card".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
        assert!(host.configure_window(3, 20.0, 30.0));
        assert!(host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 5, bifrost_proto::ModifierFlags::EMPTY));
        assert!(host.close_window_from_shell(3));
        assert!(!host.close_window_from_shell(3), "a window closes once");
        assert!(!host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 6, bifrost_proto::ModifierFlags::EMPTY), "no touch reaches a closed window");
        assert!(host.windows.of_app("fixture").is_empty());
        assert_eq!(
            sent(&peer),
            vec![
                ToApp::WindowConfigure { id: 3, w: 20.0, h: 30.0 },
                ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 5, mods: bifrost_proto::ModifierFlags::EMPTY },
                ToApp::WindowClosed { id: 3 },
            ]
        );
    }

    /// E2E-3: a Shift-click or Cmd-click reaches the app with its modifiers, on the main window and on an extra window.
    #[test]
    fn a_press_reaches_the_app_with_the_modifier_keys_held() {
        use bifrost_proto::ModifierFlags as F;
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 3, title: "Card".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
        host.touch(TouchKind::Down, 0, 4.0, 5.0, 7, F::SHIFT);
        assert!(host.touch_window(3, 1, TouchKind::Down, 1.0, 2.0, 8, F::COMMAND));
        assert_eq!(
            sent(&peer),
            vec![
                ToApp::Touch { kind: TouchKind::Down, id: 0, x: 4.0, y: 5.0, t_ms: 7, mods: F::SHIFT },
                ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Down, x: 1.0, y: 2.0, t_ms: 8, mods: F::COMMAND },
            ]
        );
    }

    /// E2E: the wheel over an extra window reaches that window with its id, only when the app asked for the wheel and
    /// the window is open, and the drag emulated for it is then dropped.
    #[test]
    fn the_wheel_over_an_extra_window_carries_the_window_id() {
        use bifrost_ui::pointer_event::{PointerEvent, WHEEL};
        use bifrost_ui::wheel::Phase;
        let (channel, peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 3, title: "Editor".into(), w: 10.0, h: 10.0, position: None, flags: 0 }, Vec::new(), &mut out);
        let wheel = PointerEvent::Wheel { x: 4.0, y: 5.0, dx: 0.0, dy: -20.0, precise: false, phase: Phase::None, momentum: Phase::None };
        assert!(!host.pointer_window(3, wheel), "an app that did not ask for the wheel keeps the emulated drag");
        host.handle(ToShell::PointerInterest { mask: WHEEL }, Vec::new(), &mut out);
        assert!(host.pointer_window(3, wheel));
        assert!(!host.pointer_window(9, wheel), "no window 9 is open");
        assert!(host.pointer_window(0, wheel), "window 0 is the main window");
        host.swallow_wheel_drag(100.0);
        assert!(host.swallows_touch(100.0) && host.swallows_touch(116.0) && host.swallows_touch(132.0));
        assert!(!host.swallows_touch(140.0), "only the three emulated touches are dropped");
        assert_eq!(
            sent(&peer),
            vec![ToApp::WindowInput { window: 3, input: WindowInput::Pointer(wheel) }, ToApp::Pointer(wheel)]
        );
    }

    /// The custom actions an app names for its elements are kept cleaned, and a new list replaces the old one.
    #[test]
    fn custom_actions_are_kept_cleaned_and_replaced_by_the_next_list() {
        use bifrost_ui::ax_action::AxActionSet;
        let (channel, _peer) = sys::socketpair_pair().unwrap();
        let mut host = AppHost::with_channel("fixture", channel, None, cfg());
        let mut out = Pumped::default();
        host.handle(ToShell::AxActions(vec![AxActionSet::new(4, &["Archive", "archive", " "]), AxActionSet::new(5, &[])]), Vec::new(), &mut out);
        assert_eq!(host.ax_actions, vec![AxActionSet::new(4, &["Archive"])]);
        host.handle(ToShell::AxActions(Vec::new()), Vec::new(), &mut out);
        assert!(host.ax_actions.is_empty());
    }
}

/// MA6: an app's second window (Mail's compose window) is drawn above the app's main window with the app's pixels in it,
/// and closing it takes it off the desk.
#[cfg(all(test, feature = "desktop-wayland"))]
mod extra_window_frame_tests {
    use super::*;
    use crate::shell::{desk, AppId, Screen};

    /// The colour the app paints into every window.
    const FILL: u32 = 0xff33_6699;

    /// Two buffers of `w` by `h` filled with `FILL`, handed to the host as a window's (or, with no id, the main window's).
    fn buffers(host: &mut AppHost, window: Option<u32>, w: usize, h: usize) {
        let bytes = w * h * 4;
        let fds = vec![sys::shm_create("extra-window-a", bytes).unwrap(), sys::shm_create("extra-window-b", bytes).unwrap()];
        for fd in &fds {
            Map::new(fd, bytes).unwrap().pixels_mut().fill(FILL);
        }
        let (w, h) = (w as u32, h as u32);
        let msg = match window {
            Some(id) => ToShell::WindowBuffers { id, w, h, generation: 1 },
            None => ToShell::Buffers { w, h, generation: 1 },
        };
        host.handle(msg, fds, &mut Pumped::default());
    }

    #[test]
    fn a_second_window_is_drawn_with_pixels_above_its_app_and_closing_it_takes_it_off_the_desk() {
        let _lock = crate::test_support::global_lock();
        let saved = std::env::var_os("SPRINGBOARD_WAYLAND");
        std::env::set_var("SPRINGBOARD_WAYLAND", "1");
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let (mine, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", mine, None, cfg);
        let mut out = Pumped::default();
        // the order the app sends them in: its window, then the window's buffers and first frame
        buffers(&mut host, None, 64, 48);
        host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 64, 48]] }, Vec::new(), &mut out);
        host.handle(ToShell::WindowOpen { id: 1, title: "New Message".into(), w: 40.0, h: 30.0, position: None, flags: 0 }, Vec::new(), &mut out);
        buffers(&mut host, Some(1), 40, 30);
        host.handle(ToShell::WindowFrame { id: 1, buf: 0, damage: vec![[0, 0, 40, 30]] }, Vec::new(), &mut out);
        sh.ext.push(Some(host));
        let m = sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m, AppId::External(0), (600, 400));
        sh.desk_sync_extra_windows();
        let (main, extra) = (AppId::External(0), AppId::ExternalWindow(0, 1));
        assert!(sh.desk.as_ref().unwrap().index_of(extra).is_some(), "the app's window is on the desk");
        assert!(!sh.desk_window_unmapped(extra), "the app committed a frame to it");
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.dirty = true;
        sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
        let frames = sh.take_wayland_windows();
        // an app that is not installed has no catalogue entry, so its main window has no id of its own: the desk draws it as 0
        let id_of = |app: AppId| desk::win_id(app).unwrap_or(0);
        let at = |app: AppId| frames.iter().position(|f| f.id == id_of(app));
        let (parent, window) = (at(main).expect("the app's own window is drawn"), at(extra).expect("a frame is drawn for the second window"));
        assert!(window > parent, "the second window is above its app's window");
        assert!(frames[window].pixels.as_ref().is_some_and(|p| p.iter().any(|&c| c == FILL)), "with the app's pixels in it");
        // closed: it is off the desk, the app is told, and the next frame has no window for it
        sh.desk_close_extra_window(0, 1);
        assert!(sh.desk.as_ref().unwrap().index_of(extra).is_none());
        assert!(sh.ext[0].as_ref().unwrap().windows.of_app("fixture").is_empty(), "the app's window table dropped it");
        sh.dirty = true;
        sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 2000.0);
        let after = sh.take_wayland_windows();
        assert!(after.iter().all(|f| f.id != id_of(extra)), "no frame for the closed window");
        assert!(after.iter().any(|f| f.id == id_of(main)), "the app's own window stays");
        if let Some(v) = saved { std::env::set_var("SPRINGBOARD_WAYLAND", v) } else { std::env::remove_var("SPRINGBOARD_WAYLAND") };
    }

    /// E2E F4: a cascaded window is cascaded only when it first opens; one the person moved comes back where they left
    /// it when the desk shows it again, instead of jumping back to a cascade slot.
    #[test]
    fn a_moved_cascaded_window_keeps_its_position_when_shown_again() {
        let _lock = crate::test_support::global_lock();
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let (mine, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", mine, None, cfg);
        let flags = bifrost_proto::window_flags::CASCADE;
        host.handle(ToShell::WindowOpen { id: 1, title: "Viewer".into(), w: 40.0, h: 30.0, position: None, flags }, Vec::new(), &mut Pumped::default());
        sh.ext.push(Some(host));
        let m = sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m, AppId::External(0), (600, 400));
        sh.desk_sync_extra_windows();
        let extra = AppId::ExternalWindow(0, 1);
        let cascades = |sh: &crate::shell::Shell| sh.ext[0].as_ref().unwrap().windows.of_app("fixture").iter().any(|w| w.id == 1 && w.cascade);
        assert!(!cascades(&sh), "the cascade flag is cleared once the window is placed");
        // the person drags it to the work area's top left corner
        let a = desk::Desk::area(&m);
        let moved = (a.x + 5, a.y + 7);
        {
            let d = sh.desk.as_mut().unwrap();
            let i = d.index_of(extra).expect("the viewer is on the desk");
            assert_ne!((d.wins[i].r.x, d.wins[i].r.y), moved, "the cascade put it somewhere else");
            d.wins[i].r.x = moved.0;
            d.wins[i].r.y = moved.1;
            // hidden: the desk drops it while the app's table keeps it
            d.close(i);
        }
        sh.desk_sync_extra_windows();
        let d = sh.desk.as_ref().unwrap();
        let i = d.index_of(extra).expect("shown again");
        assert_eq!((d.wins[i].r.x, d.wins[i].r.y), moved, "it comes back where the person left it");
    }

    /// E2E-11/12: an app that reallocates its buffers (every step of a live resize) keeps its window on the desk until it
    /// draws into them: the window is not unmapped and its frame (last pixels, last size) is still sent, so the toplevel
    /// is not destroyed under the pointer (which ended the resize) and the window behind does not take the focus.
    #[test]
    fn an_app_between_buffer_sets_keeps_its_window() {
        let _lock = crate::test_support::global_lock();
        let saved = std::env::var_os("SPRINGBOARD_WAYLAND");
        std::env::set_var("SPRINGBOARD_WAYLAND", "1");
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let (mine, _peer) = sys::socketpair_pair().unwrap();
        let cfg = Configure { w: 64, h: 48, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = AppHost::with_channel("fixture", mine, None, cfg);
        assert!(!host.between_buffers(), "an app that never drew is not between buffers");
        buffers(&mut host, None, 64, 48);
        host.handle(ToShell::Frame { buf: 0, damage: vec![[0, 0, 64, 48]] }, Vec::new(), &mut Pumped::default());
        sh.ext.push(Some(host));
        let m = sh.desk_metrics();
        let main = AppId::External(0);
        sh.desk.as_mut().unwrap().open(&m, main, (600, 400));
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut px = vec![0u32; w * h];
        sh.dirty = true;
        sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1000.0);
        let id = desk::win_id(main).unwrap_or(0);
        let first = sh.take_wayland_windows();
        let drawn = first.iter().find(|f| f.id == id).expect("the window is drawn");
        let size = (drawn.width, drawn.height);
        // the window was resized: the app sends a new buffer set and has not drawn into it yet
        let r = sh.desk.as_ref().unwrap().wins[0].r;
        sh.desk.as_mut().unwrap().wins[0].r.w = r.w - 40;
        buffers(sh.ext[0].as_mut().unwrap(), None, 80, 60);
        assert!(sh.ext[0].as_ref().unwrap().between_buffers());
        assert!(!sh.desk_window_unmapped(main), "the window stays mapped between buffer sets");
        sh.dirty = true;
        sh.draw_desk_windows(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, 1100.0);
        let next = sh.take_wayland_windows();
        let kept = next.iter().find(|f| f.id == id).expect("a frame is still sent: the toplevel is kept");
        assert_eq!((kept.width, kept.height), size, "at the size of the pixels it last presented");
        assert!(kept.pixels.is_none(), "the window server keeps showing the last pixels");
        if let Some(v) = saved { std::env::set_var("SPRINGBOARD_WAYLAND", v) } else { std::env::remove_var("SPRINGBOARD_WAYLAND") };
    }
}

#[cfg(test)]
mod ext_state_tests {
    use super::ext_state_text;

    #[test]
    fn state_text_ends_every_line_so_the_next_section_starts_its_own() {
        let text = ext_state_text(&["is.a.calendar"], &[("is.a.calendar", 62, 0), ("is.a.browser", 15, 0xf)]);
        assert_eq!(text, "apps_running=is.a.calendar\napp_controls=is.a.calendar:62:0,is.a.browser:15:f\n");
        // the shell appends the window lines straight after: each is a line of its own
        let joined = format!("{text}window=Settings,965,306,909,846\n");
        assert!(joined.lines().any(|l| l.starts_with("window=Settings")));
        assert!(joined.lines().all(|l| l.matches('=').count() >= 1));
    }

    #[test]
    fn state_text_with_no_apps_is_still_two_whole_lines() {
        assert_eq!(ext_state_text(&[], &[]), "apps_running=\napp_controls=\n");
    }

    /// E2E N2: a native app the shell launched runs while it shows a toplevel, though it has no app host.
    #[test]
    fn a_native_app_with_a_toplevel_is_running() {
        use super::native_running;
        let manifests = [("ai.film", "native", "ai.film"), ("is.notes", "sdk", ""), ("x.idle", "native", "x.idle"), ("x.blank", "native", "")];
        assert_eq!(native_running(&manifests, ["ai.film", "ai.film", "org.linux.app"].into_iter()), vec!["ai.film".to_string()]);
        assert!(native_running(&manifests, std::iter::empty()).is_empty(), "no toplevel, not running");
        assert!(native_running(&manifests, [""].into_iter()).is_empty(), "an app without a Wayland id never matches");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_foreign_windows_line_names_each_window_once_and_the_active_one() {
        use crate::bare::wayland::ForeignWindow;
        let w = |id, app_id: &str, active| ForeignWindow { id, title: "t, with a comma".into(), app_id: app_id.into(), active, maximized: false, minimized: false };
        let windows = [w(7, "ai.film", true), w(9, "org.x", false), w(7, "ai.film", true)];
        assert_eq!(super::foreign_state_line(windows.iter()), "foreign_windows=ai.film:7:1,org.x:9:0\n");
        assert_eq!(super::foreign_state_line(std::iter::empty()), "foreign_windows=\n");
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

    #[test]
    fn an_uninstalled_app_keeps_its_slot_but_loses_its_permissions() {
        let app=|id:&str| Installed {manifest:Manifest{id:id.into(),name:id.into(),exec:"app".into(),permissions:vec!["accessibility".into(),"storage".into()],..Manifest::default()},dir:PathBuf::from(format!("/nowhere/{id}"))};
        let old=vec![app("a"),app("b"),app("c")];
        let now=merge_installed(&old,vec![app("a"),app("c")]);
        assert_eq!(now.iter().map(|a|a.manifest.id.as_str()).collect::<Vec<_>>(),vec!["a","b","c"],"the slots do not move");
        assert!(now[1].is_removed() && now[1].manifest.permissions.is_empty() && now[1].manifest.exec.is_empty());
        assert_eq!(now[1].manifest.name,"b");
        assert!(!now[0].is_removed() && !now[2].is_removed());
        // installing it again fills the same slot
        let back=merge_installed(&now,vec![app("a"),app("b"),app("c")]);
        assert!(!back[1].is_removed() && back[1].manifest.permissions.len()==2);
        // an entry that is already removed stays removed and is stable
        assert_eq!(merge_installed(&now,vec![app("a"),app("c")]),now);
    }

    #[test]
    fn packages_beyond_the_limit_are_not_added() {
        let app=|i:usize| Installed {manifest:Manifest{id:format!("app{i}"),exec:"app".into(),..Manifest::default()},dir:PathBuf::from(format!("/nowhere/{i}"))};
        let scanned:Vec<Installed>=(0..MAX_INSTALLED+3).map(app).collect();
        let all=merge_installed(&[],scanned);
        assert_eq!(all.len(),MAX_INSTALLED);
    }
}

/// The alert title and body for an app that did not start, pure: an empty name reads as "The app".
pub fn launch_failure_text(name: &str) -> (String, String) {
    launch_failure_text_for(name, None)
}

/// [`launch_failure_text`] with the reason the shell refused the start, when it has one: the body then says why
/// instead of asking for another try (which would fail the same way).
pub fn launch_failure_text_for(name: &str, reason: Option<&str>) -> (String, String) {
    let name = name.trim();
    let name = if name.is_empty() { "The app" } else { name };
    let body = match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(reason) => reason.to_string(),
        None => "It stopped before it finished starting. Try opening it again.".to_string(),
    };
    (format!("{name} could not be opened"), body)
}

/// Why the shell refused to start an app, in words for the person, for the one refusal that is the package's doing:
/// its manifest names a permission this system does not offer. Any other error gets the generic text.
#[cfg(feature = "desktop-wayland")]
fn launch_failure_reason(e: &std::io::Error) -> Option<String> {
    match crate::bare::app_policy::PolicyError::of(e) {
        Some(crate::bare::app_policy::PolicyError::UnsupportedPermission(name)) => Some(unsupported_permission_text(name)),
        _ => None,
    }
}

#[cfg(not(feature = "desktop-wayland"))]
fn launch_failure_reason(_e: &std::io::Error) -> Option<String> {
    None
}

/// The body of the alert for a package that asks for a permission this system does not offer.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
fn unsupported_permission_text(permission: &str) -> String {
    format!("It asks for a permission this system does not offer (\"{permission}\"). Update the app from the App Store.")
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

    #[test]
    fn a_refused_permission_is_named_in_the_alert() {
        let body = crate::shell::apphost::unsupported_permission_text("teleport");
        let (title, shown) = crate::shell::apphost::launch_failure_text_for("Notes", Some(&body));
        assert_eq!(title, "Notes could not be opened");
        assert!(shown.contains("\"teleport\"") && !shown.contains("Try opening it again"));
        assert!(crate::shell::apphost::launch_failure_text_for("Notes", Some("  ")).1.contains("Try opening it again"));
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

    pub(super) fn fixture_host() -> (AppHost, Channel) {
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
    fn a_window_level_of_an_extra_window_is_kept_in_the_table() {
        let (mut host, _peer) = fixture_host();
        let mut out = Pumped::default();
        host.handle(ToShell::WindowOpen { id: 2, title: "CPU".into(), w: 300.0, h: 200.0, position: None, flags: 0 }, Vec::new(), &mut out);
        host.handle(ToShell::WindowLevelOf { id: 2, level: bifrost_proto::WindowLevel::Floating }, Vec::new(), &mut out);
        assert_eq!(host.windows.floating_ids(&host.id), vec![2]);
        assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal, "the main window keeps its level");
        host.handle(ToShell::WindowLevelOf { id: 0, level: bifrost_proto::WindowLevel::Floating }, Vec::new(), &mut out);
        assert!(host.windows.floating_ids(&host.id).contains(&2));
        host.kill();
    }

    #[test]
    fn a_window_level_request_is_kept_for_the_app() {
        let (mut host, _peer) = fixture_host();
        assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal);
        let mut out = Pumped::default();
        host.handle(ToShell::WindowLevel(bifrost_proto::WindowLevel::Floating), Vec::new(), &mut out);
        assert_eq!(host.window_level, bifrost_proto::WindowLevel::Floating);
        assert!(out.requests.is_empty(), "the level is not passed on to the shell's requests");
        host.handle(ToShell::WindowLevel(bifrost_proto::WindowLevel::Normal), Vec::new(), &mut out);
        assert_eq!(host.window_level, bifrost_proto::WindowLevel::Normal);
        host.kill();
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

#[cfg(test)]
mod service_manifest_tests {
    use super::Manifest;

    #[test]
    fn service_tables_are_read_and_do_not_leak_into_the_app_keys() {
        let m = Manifest::parse(
            "id = \"is.olibuijr.preview.desktop\"\nname = \"Viewer\"\nversion = \"0.11.8\"\nexec = \"bifrost-viewer\"\n\n[[service]]\ntitle = \"Open Images in Viewer\"\nsend_types = [\"public.image\", \"public.pdf\"]\nreturn_types = []\nmethod = \"open_images\"\n\n[[service]]\ntitle = \"Copy Path\"\nsend_types = [\"bifrost.file-path-text\"]\nreturn_types = [\"public.utf8-plain-text\"]\nmethod = \"copy_path\"\n",
        )
        .unwrap();
        assert_eq!(m.id, "is.olibuijr.preview.desktop");
        assert_eq!(m.name, "Viewer");
        assert!(m.intents.is_empty());
        assert_eq!(m.services.len(), 2);
        assert_eq!(m.services[0].title, "Open Images in Viewer");
        assert_eq!(m.services[0].send_types, vec!["public.image".to_string(), "public.pdf".to_string()]);
        assert!(m.services[0].return_types.is_empty());
        assert_eq!(m.services[0].method, "open_images");
        assert_eq!(m.services[1].return_types, vec!["public.utf8-plain-text".to_string()]);
        assert_eq!(m.services[1].rpc_method(), "service.copy_path");
    }

    #[test]
    fn a_manifest_without_services_has_none() {
        let m = Manifest::parse("id = \"a.b\"\nname = \"X\"\nexec = \"x\"\n").unwrap();
        assert!(m.services.is_empty());
    }
}

/// The sounds apps asked for since the shell's audio worker last looked, in order. Each is an alert tone name.
static SYSTEM_SOUNDS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// At most this many sounds wait for one frame; the rest are dropped so an app cannot flood the audio worker.
const MAX_SYSTEM_SOUNDS: usize = 4;

/// Queues an app's system sound when the name is one of the shell's alert tones or interface cues.
fn queue_system_sound(name: String) {
    let known = super::options::ALERT_SOUNDS.contains(&name.as_str()) || super::ui_sounds::index_of(&name).is_some();
    if !known {
        return;
    }
    let mut queue = SYSTEM_SOUNDS.lock().unwrap_or_else(|e| e.into_inner());
    if queue.len() < MAX_SYSTEM_SOUNDS {
        queue.push(name);
    }
}

/// Queues the sound of a request the shell delivers for an app (`UNNotificationSound`); names outside the alert tones
/// are dropped, as for `PlaySystemSound`.
pub(super) fn queue_request_sound(name: &str) {
    queue_system_sound(name.to_string());
}

/// Takes the queued system sounds. The shell's frame loop plays each one on the sound-effects role.
pub fn take_system_sounds() -> Vec<String> {
    std::mem::take(&mut *SYSTEM_SOUNDS.lock().unwrap_or_else(|e| e.into_inner()))
}
