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

/// Counts one plain touch at `t` ms against the drag emulated for a wheel event (`left` touches, until `until`): true
/// while the down, the move and the up of that drag are still due.
fn wheel_drag_touch((left, until): &mut (u8, f64), t: f64) -> bool {
    if *left == 0 || t > *until {
        (*left, *until) = (0, 0.0);
        return false;
    }
    *left -= 1;
    true
}

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
    /// `ToShell::CloseRequests`: the app answers the close control of its main window itself (a save sheet for an edited
    /// document), so the shell sends `ToApp::CloseRequest` for it (`Shell::desk_close_requested`) instead of closing at once.
    pub close_requests: bool,
    /// A `ToApp::CloseRequest` was sent and the app has not ordered its main window out since. On the direct route the
    /// app's toplevel cannot be unmapped (`desk_order_main_out`), so the order-out that answers the request quits the app, as
    /// the close control did before apps could answer it (`order_out_quits`).
    pub close_pending: bool,
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
    /// Plain touches still to come after a wheel event the app did not take: the drag emulated for the wheel, and until
    /// when. They are a scroll, so the app gets them as a finger's and they pan on a desktop too (`Shell::ext_touch`).
    wheel_pan: (u8, f64),
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
    /// Where the app asked its main window to go (`ToShell::WindowMove` with window 0, points relative to the visible
    /// desktop area): a document app moving a document into its main window keeps that document's frame. The desk
    /// applies it once (`take_main_move`).
    main_move: Option<(f32, f32)>,
    /// The window the app asked to bring to the front (`ToShell::WindowRaise`, 0 is the main window), applied once by the
    /// desk (`take_raise`).
    raise: Option<u32>,
    /// The app ordered its main window out (`WindowClose` id 0, its last document closed) or asked for it back
    /// (`WindowOpen` id 0): `Some(false)` and `Some(true)`, applied once by the desk (`take_main_order`). The app keeps
    /// running either way; only activating it reopens a main window it ordered out (`ToApp::Reopen`).
    main_order: Option<bool>,
    /// The main window is out because the app itself ordered it out (`WindowClose` id 0) and has not ordered it back in
    /// since. Only then does activating the app send `ToApp::Reopen`: the first activation of an app started in the
    /// background, or of a direct app that fell back to shared memory, is a plain first show. The desk keeps it.
    pub main_ordered_out: bool,
    /// The window with key focus: 0 is the main window, otherwise an extra window's id. Keys, commands, menu items and
    /// text edits go to it (`input`); the desk sets it (`set_key_window`).
    pub key_window: u32,
    /// Each extra window's buffers, by window id (`ToShell::WindowBuffers`).
    window_bufs: std::collections::BTreeMap<u32, WinBufs>,
    /// Buffers and frames that arrived for a window the table does not hold yet, oldest first (`Early`).
    early: std::collections::VecDeque<Early>,
    /// Windows the shell closed (the red light, an app that does not ask) or refused: what the app still had in flight
    /// for them when it heard (`WindowClosed`) is dropped, not kept as early pixels. Kept, it would show as the first frame
    /// of the next window to open under that id. An id is forgotten when its window opens again.
    shell_closed: std::collections::HashSet<u32>,
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
            close_requests: false,
            close_pending: false,
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
            wheel_pan: (0, 0.0),
            route: Route::Shm,
            frames: bifrost_ui::touch::TouchTracker::with_scale(cfg.scale),
            frame_scale: cfg.scale,
            fallback: false,
            windows: WindowTable::new(),
            main_move: None,
            raise: None,
            main_order: None,
            main_ordered_out: false,
            key_window: 0,
            window_bufs: std::collections::BTreeMap::new(),
            early: std::collections::VecDeque::new(),
            shell_closed: std::collections::HashSet::new(),
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
        self.shell_closed.clear();
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
        wheel_drag_touch(&mut self.wheel_swallow, t)
    }

    /// A wheel event the app did not ask for happened at `t` ms: the drag emulated for it still reaches the app, as a
    /// finger's, since a wheel notch is a scroll and not a mouse drag (`pans_touch`).
    pub(super) fn keep_wheel_drag(&mut self, t: f64) {
        self.wheel_pan = (3, t + WHEEL_DRAG_MS);
    }

    /// True for each plain touch of the drag emulated for a wheel event the app did not take: it pans like a finger.
    pub(super) fn pans_touch(&mut self, t: f64) -> bool {
        wheel_drag_touch(&mut self.wheel_pan, t)
    }

    /// A press, drag or release for the app's main window, with the modifier keys held (the reference event modifier flags).
    pub fn touch(&mut self, kind: TouchKind, id: u32, x: f32, y: f32, t_ms: u32, source: bifrost_proto::TouchSource, mods: bifrost_proto::ModifierFlags) {
        self.send(&ToApp::Touch { kind, id, x, y, t_ms, source, mods });
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

    pub(super) fn handle(&mut self, msg: ToShell, fds: Vec<sys::Owned>, out: &mut Pumped) {
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
            ToShell::CloseRequests { on } => self.close_requests = on,
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
            // window 0 is the main window: the desk orders it back in on its next sync
            ToShell::WindowOpen { id: 0, .. } => {
                self.main_order = Some(true);
                out.chrome();
            }
            ToShell::WindowOpen { id, title, w, h, position, flags } => {
                // no room for the window, or its id is taken: the app is told to drop it, so the two sides agree
                if self.windows.open(&self.id, id, &title, w, h).is_err() {
                    self.early.retain(|e| e.window() != id);
                    self.note_shell_closed(id);
                    self.send(&ToApp::WindowClosed { id });
                } else {
                    self.shell_closed.remove(&id);
                    if let Some((x, y)) = position {
                        self.windows.request_position(&self.id, id, x, y);
                    }
                    self.windows.set_title_double_click(&self.id, id, flags & bifrost_proto::window_flags::TITLE_DOUBLE_CLICK != 0);
                    // a position the app asked for wins over the cascade
                    self.windows.set_cascade(&self.id, id, position.is_none() && flags & bifrost_proto::window_flags::CASCADE != 0);
                    self.windows.set_full_size(&self.id, id, flags & bifrost_proto::window_flags::FULL_SIZE != 0);
                    self.replay_early(id, out);
                }
                out.chrome();
            }
            // window 0 is the main window: the desk moves it on its next sync, as it does an extra window
            ToShell::WindowMove { window: 0, x, y } => {
                if x.is_finite() && y.is_finite() {
                    self.main_move = Some((x, y));
                    out.chrome();
                }
            }
            ToShell::WindowMove { window, x, y } => {
                // an unknown window is ignored; the desk applies the position on its next sync
                if self.windows.request_position(&self.id, window, x, y) {
                    out.chrome();
                }
            }
            ToShell::WindowRaise { window } => {
                self.raise = Some(window);
                out.chrome();
            }
            ToShell::WindowCollapse { window, collapsed } => {
                if self.windows.set_collapsed(&self.id, window, collapsed) {
                    out.chrome();
                }
            }
            // window 0 is the main window: the desk orders it out on its next sync and the app keeps running
            ToShell::WindowClose { id: 0 } => {
                self.main_order = Some(false);
                out.chrome();
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
            // sent before the app heard that the shell closed the window: nothing is left to show them in (the descriptors close
            // as `fds` drops)
            ToShell::WindowBuffers { id, .. } | ToShell::WindowFrame { id, .. } if self.shell_closed.contains(&id) && !self.window_open(id) => {}
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

    pub fn window_open(&self, id: u32) -> bool {
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
        self.note_shell_closed(id);
        self.send(&ToApp::WindowClosed { id });
        true
    }

    /// Remembers that the shell closed (or refused) window `id`, so pixels the app still sends for it are dropped.
    fn note_shell_closed(&mut self, id: u32) {
        // the ids are the app's own and few; a runaway app cannot grow the set
        if self.shell_closed.len() >= 256 {
            self.shell_closed.clear();
        }
        self.shell_closed.insert(id);
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
    pub fn touch_window(&mut self, id: u32, touch: u32, kind: TouchKind, x: f32, y: f32, t_ms: u32, source: bifrost_proto::TouchSource, mods: bifrost_proto::ModifierFlags) -> bool {
        if !self.window_open(id) {
            return false;
        }
        self.send(&ToApp::WindowTouch { id, touch, kind, x, y, t_ms, source, mods });
        true
    }

    /// The person dragged window `id` to (x, y) points (the shell's clamped position): the table keeps it and the app is
    /// told (`WindowMoved`). False when the window is not open.
    /// The position the app asked its main window to move to (`ToShell::WindowMove` with window 0), once.
    pub fn take_main_move(&mut self) -> Option<(f32, f32)> {
        self.main_move.take()
    }

    /// The window the app asked to bring to the front (`ToShell::WindowRaise`), once.
    pub fn take_raise(&mut self) -> Option<u32> {
        self.raise.take()
    }

    /// Whether the app asked for its main window to be ordered out (`Some(false)`) or back in (`Some(true)`), once.
    pub fn take_main_order(&mut self) -> Option<bool> {
        self.main_order.take()
    }

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
        self.shell_closed.clear();
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

/// Whether an app's order-out of its main window quits it: only the answer to the close control (`close_pending`) of an app on
/// the direct route, whose window the shell cannot take down any other way. Everywhere else the app keeps running without
/// a window (`desk_order_main_out`).
pub fn order_out_quits(route: Route, close_pending: bool) -> bool {
    route == Route::Direct && close_pending
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

/// An app's icon at `size` px from the package icon file at `icon_path` (`App::icon_path`), for a process that scanned the
/// store itself (the Files app's Applications place) and so has no registry to look `id` up in. Cached like `icon_image`.
pub fn icon_image_at(id: &str, icon_path: &Path, size: usize) -> Option<std::sync::Arc<crate::gfx::Image>> {
    let (variant, _) = bifrost_render::icon_doc::current_style();
    bifrost_render::icon_doc::app_icon(id, icon_path, size as f32, 1.0, variant)
}

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

/// The Configure a background app gets when its window on the desk has a body of `size` px and the app was last
/// configured to `current`: the foreground's settings at that size, with no keyboard inset (the keyboard is over the app
/// in front only). `None` when nothing changed, the app is not running or the size is empty.
pub fn back_configure(fg: Configure, size: (u32, u32), running: bool, current: (usize, usize)) -> Option<Configure> {
    let (w, h) = size;
    (running && w > 0 && h > 0 && current != (w as usize, h as usize)).then_some(Configure { w, h, safe_bottom: 0.0, ..fg })
}

impl Shell {
    /// The body size of the window on the desk of the background app in `slot` (`fg` is the app in front): none with no
    /// desk, for the app in front, for an app with no main window on the desk, or for a window collapsed to its title bar
    /// (which keeps the size its app had, as `desk_layout` does).
    fn ext_back_size(&self, slot: usize, fg: Option<u8>) -> Option<(u32, u32)> {
        let slot = u8::try_from(slot).ok().filter(|s| fg != Some(*s))?;
        let app = super::AppId::External(slot);
        if self.desk.as_ref()?.is_collapsed(app) {
            return None;
        }
        let (w, h) = self.desk_app_size(app)?;
        Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))
    }

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
        // the app learns whether a mouse or a finger made the touch: only a finger pans a list on a desktop. The drag
        // emulated for a wheel notch the app did not take is a scroll, so it counts as a finger's.
        let source = if h.pans_touch(t) { bifrost_proto::TouchSource::Finger } else { super::touch_frames::plain_source(kind, &self.touch_contacts, self.touch_source) };
        self.touch_source = source;
        // the pointer as a touch frame, before the plain stream; a finger on the touchscreen is reported by its contact
        if let Some((raw, s)) = super::touch_frames::pointer_sample(kind, x, y, &self.touch_contacts) {
            h.feed_touch(raw, s, t);
        }
        h.touch(kind, 0, x, y, t as u32, source, self.key_mods);
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
                if let Some(id) = super::desktop_bar::chord_menu_id(&h.app_menus, key as u16, mods) {
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
            // Shift+Tab is the chord on Tab (evdev 15), as the SDK's views and sheets read it
            Out::Named("BackTab") => KeyInput::chord(15, CHORD_SHIFT),
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
        // the body size of each background app's window on the desk: a window in the back is still on screen, so its app
        // is configured to the frame the shell gives it (a restored session, an update relaunch) and draws at it now, not
        // only once it comes to the front (0.10.39: Mail restored behind other windows drew at its launch size, black past it)
        let back_sizes: Vec<Option<(u32, u32)>> = (0..self.ext.len()).map(|slot| self.ext_back_size(slot, fg)).collect();
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
            } else if let Some(cfg) = back_sizes.get(slot).copied().flatten().and_then(|size| back_configure(cfg, size, h.running(), h.size())) {
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


#[cfg(test)]
mod tests;

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
mod window_table_tests;

/// MA6: an app's second window (Mail's compose window) is drawn above the app's main window with the app's pixels in it,
/// and closing it takes it off the desk.
#[cfg(all(test, feature = "desktop-wayland"))]
mod extra_window_frame_tests;

#[cfg(test)]
mod ext_state_tests;

#[cfg(test)]
mod migrate_tests;

#[cfg(test)]
mod registry_update_tests;

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
mod badge_tests;

#[cfg(test)]
mod route_tests;

#[cfg(test)]
mod service_manifest_tests;

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
