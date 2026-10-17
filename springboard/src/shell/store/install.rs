//! Installing `.aap` packages (docs/apps.md): verify the SHA-256, read the
//! ustar archive in memory (rejecting unsafe paths), validate `manifest.toml`,
//! unpack to `apps/<id>/<version>/`, flip the `current` symlink, keep one
//! previous version, and record it in `apps/installed.json`.

use super::json::{self, Json};
use crate::shell::kinds::Kind;
use crate::shell::sha256::{hex, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

pub const ROOT: &str = "/var/lib/springboard";
/// Serialises the registry's read-modify-write (installs run on several worker threads).
static REGISTRY: Mutex<()> = Mutex::new(());
const MAX_PACKAGE_FILES: usize = 4096;
const MAX_UNPACKED: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024;

/// Version of this shell, compared with a package's `min_shell`.
pub fn shell_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What the app host needs to launch an installed app.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `apps/<id>/current/<exec>`
    pub exec_path: PathBuf,
    /// `apps/<id>/current/<icon>` (a 180x180 PNG)
    pub icon_path: PathBuf,
    /// `apps/<id>/current` (the package root; assets live below it)
    pub dir: PathBuf,
    /// `"app"` or `"system"` (empty reads as `"app"`)
    pub kind: String,
    /// what a system package supplies to the shell, e.g. `["keyboard"]`
    pub provides: Vec<String>,
    /// the kept previous version that `rollback` can restore, if any
    pub previous: Option<String>,
    /// the day the customer first installed the app (days since 1970-01-01); 0 when the registry does not say
    pub purchased: i64,
}

impl InstalledApp {
    /// A system package: preinstalled data or service, never spawned, no home-screen icon.
    pub fn is_system(&self) -> bool {
        Kind::of(&self.kind).hidden_from_grid()
    }

    /// Installed by the background checker, never removed by the user (`system`, `system-app`, `service`).
    pub fn is_preinstalled(&self) -> bool {
        Kind::of(&self.kind).preinstalled()
    }

    /// An `bifrostd` service: never shown in the App Store.
    pub fn is_service(&self) -> bool {
        Kind::of(&self.kind) == Kind::Service
    }
}

/// An `bifrostd` background service, installed and updated silently and never listed in the App Store.
pub const KIND_SERVICE: &str = crate::shell::kinds::SERVICE;
pub const KIND_SYSTEM: &str = crate::shell::kinds::SYSTEM;
/// A preinstalled app with an icon and an exec, installed silently and not removable.
pub const KIND_SYSTEM_APP: &str = crate::shell::kinds::SYSTEM_APP;

pub fn apps_dir(root: &Path) -> PathBuf {
    root.join("apps")
}

/// The installed apps, from `apps/installed.json` under the default root.
pub fn installed_apps() -> Vec<InstalledApp> {
    installed_in(Path::new(ROOT))
}

pub fn installed_in(root: &Path) -> Vec<InstalledApp> {
    read_registry(root).into_iter().map(|e| e.app(root)).collect()
}

/// Today as days since 1970-01-01: the day a first install is recorded on.
pub fn install_day() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86_400).unwrap_or(0)
}

// ---- semver ----

/// `major.minor.patch` (a `-pre` or `+build` suffix is ignored).
pub fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let mut n = || it.next()?.parse::<u64>().ok();
    let r = (n()?, n()?, n()?);
    it.next().is_none().then_some(r)
}

// ---- manifest ----

#[derive(Clone, Debug, PartialEq, Eq, Default)]
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
    /// `"app"` (default, also when empty) or `"system"`
    pub kind: String,
    pub provides: Vec<String>,
    /// `"full-size"`: a full-size-content window (`NativeWindowStyleMaskFullSizeContentView`). The shell draws no
    /// title row; the traffic lights sit over the app at the top left and the app's toolbar band drags the window.
    pub window: String,
    /// Relative directory of the app's help book (`help = "help"`): Markdown pages with front matter, read by the
    /// shell's help index (`shell::helpbook`). Empty when the app ships no help.
    pub help: String,
    /// `"still"` or `"day"`: the kind of wallpaper the app provides (`shell::wallpaper_types`); empty for none.
    pub wallpaper_kind: String,
    /// The app's wallpaper pictures, package-relative (`wallpapers = ["wallpapers/a.png"]`).
    pub wallpapers: Vec<String>,
}

impl Manifest {
    pub fn full_size_content(&self) -> bool {
        self.window == "full-size"
    }
    pub fn is_system(&self) -> bool {
        Kind::of(&self.kind) == Kind::System
    }
}

fn toml_string(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v.strip_prefix('"')?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(match it.next()? {
                'n' => '\n',
                't' => '\t',
                o => o,
            }),
            c => out.push(c),
        }
    }
    None
}

/// The flat subset of TOML a manifest uses: `key = "string"` and
/// `key = ["a", "b"]`, with `#` comments. Only top-level keys are read: everything from the first table header
/// on (`[[intent]]`, whose `id` and `shortcut = true` are not the app's) is left to its own parser.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut m = Manifest::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('[') {
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').ok_or_else(|| format!("manifest line {}: expected key = value", n + 1))?;
        let (k, v) = (k.trim(), v.trim());
        if let Some(list) = v.strip_prefix('[') {
            let body = list.rsplit_once(']').ok_or_else(|| format!("manifest line {}: unterminated list", n + 1))?.0;
            let items: Vec<String> = body.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| toml_string(s).ok_or_else(|| format!("manifest line {}: bad list item", n + 1))).collect::<Result<_, _>>()?;
            match k {
                "permissions" => m.permissions = items,
                "provides" => m.provides = items,
                "wallpapers" => m.wallpapers = items,
                _ => {}
            }
            continue;
        }
        if (k == "sdk_protocol" || v.parse::<u64>().is_ok() || v == "true" || v == "false")
            && !matches!(k, "id" | "name" | "version" | "exec" | "icon" | "min_shell" | "summary" | "category" | "kind" | "window" | "help") {
            continue;
        }
        let Some(s) = toml_string(v) else {
            return Err(format!("manifest line {}: {k} must be a string", n + 1));
        };
        match k {
            "id" => m.id = s,
            "name" => m.name = s,
            "version" => m.version = s,
            "exec" => m.exec = s,
            "icon" => m.icon = s,
            "min_shell" => m.min_shell = s,
            "summary" => m.summary = s,
            "category" => m.category = s,
            "kind" => m.kind = s,
            "window" => m.window = s,
            "help" => m.help = s,
            "wallpaper_kind" => m.wallpaper_kind = s,
            _ => {}
        }
    }
    Ok(m)
}

/// Reverse-DNS id: two or more dot-separated segments of `[a-z0-9_-]`, each
/// starting with a letter or digit.
pub fn valid_id(id: &str) -> bool {
    let segs: Vec<&str> = id.split('.').collect();
    id.len() <= 128
        && segs.len() >= 2
        && segs.iter().all(|s| s.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'))
}

/// Normalises an archive path: no absolute paths, no `..`, no empty parts.
pub fn safe_path(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.contains('\0') || raw.starts_with('/') || raw.contains('\\') {
        return Err(format!("unsafe path {raw:?}"));
    }
    let mut out = PathBuf::new();
    for c in Path::new(raw).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return Err(format!("unsafe path {raw:?}")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("unsafe path {raw:?}"));
    }
    Ok(out)
}

/// Checks a manifest against the files in the package and this shell.
pub fn validate(m: &Manifest, files: &[TarEntry], shell: &str) -> Result<(), String> {
    if !valid_id(&m.id) {
        return Err(format!("invalid app id {:?}", m.id));
    }
    if m.name.trim().is_empty() {
        return Err("manifest has no name".into());
    }
    if parse_semver(&m.version).is_none() {
        return Err(format!("invalid version {:?}", m.version));
    }
    let need = if m.min_shell.is_empty() { (0, 0, 0) } else { parse_semver(&m.min_shell).ok_or_else(|| format!("invalid min_shell {:?}", m.min_shell))? };
    let have = parse_semver(shell).ok_or("shell version is not semver")?;
    if need > have {
        return Err(format!("needs shell {} or newer (this is {shell})", m.min_shell));
    }
    let has = |p: &str| safe_path(p).ok().is_some_and(|p| files.iter().any(|f| !f.is_dir && f.path == p));
    if Kind::parse(&m.kind).is_none() {
        return Err(format!("unsupported package kind {:?}", m.kind));
    }
    // only a system package may be data-only; if it names an exec anyway, it must exist
    if m.exec.is_empty() && !Kind::of(&m.kind).exec_optional() || !m.exec.is_empty() && !has(&m.exec) {
        return Err(format!("exec {:?} is not inside the package", m.exec));
    }
    if !Kind::of(&m.kind).may_provide() && !m.provides.is_empty() {
        return Err("only a system package may declare `provides`".into());
    }
    if !m.icon.is_empty() && !has(&m.icon) {
        return Err(format!("icon {:?} is not inside the package", m.icon));
    }
    if let Some(wallpaper) = crate::shell::wallpaper_types::from_manifest(&m.id, &m.wallpaper_kind, &m.wallpapers)? {
        if let Some(missing) = wallpaper.images.iter().find(|p| !has(p)) {
            return Err(format!("wallpaper {missing:?} is not inside the package"));
        }
    }
    Ok(())
}

// ---- ustar ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TarEntry {
    pub path: PathBuf,
    /// the contents, when the archive was read with `read_tar`; empty from `index_tar`
    pub data: Vec<u8>,
    pub mode: u32,
    pub is_dir: bool,
    /// where the contents start in the archive and how long they are
    pub offset: u64,
    pub size: u64,
}

fn field(h: &[u8]) -> &[u8] {
    &h[..h.iter().position(|&b| b == 0).unwrap_or(h.len())]
}

fn octal(h: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(field(h)).ok()?.trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

/// Reads an uncompressed POSIX (ustar) tar held in memory. Only regular
/// files and directories are accepted; links, devices and paths that are
/// absolute or contain `..` make the whole package invalid.
pub fn read_tar(data: &[u8]) -> Result<Vec<TarEntry>, String> {
    let mut files = index_tar(&mut std::io::Cursor::new(data))?;
    for f in files.iter_mut().filter(|f| !f.is_dir) {
        f.data = data[f.offset as usize..][..f.size as usize].to_vec();
    }
    Ok(files)
}

/// Reads only the headers of a ustar tar (`r` at its start), seeking past the
/// contents: a package on disk is checked without being loaded. The same rules
/// as `read_tar`.
pub fn index_tar<R: Read + Seek>(r: &mut R) -> Result<Vec<TarEntry>, String> {
    let io = |e: std::io::Error| e.to_string();
    let len = r.seek(SeekFrom::End(0)).map_err(io)?;
    let mut out: Vec<TarEntry> = vec![];
    let (mut at, mut total) = (0u64, 0u64);
    loop {
        if at + 512 > len {
            return Err("tar ends without an end marker".into());
        }
        r.seek(SeekFrom::Start(at)).map_err(io)?;
        let mut h = [0u8; 512];
        r.read_exact(&mut h).map_err(io)?;
        if h.iter().all(|&b| b == 0) {
            return Ok(out);
        }
        let sum: u32 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u32 }).sum();
        if octal(&h[148..156]) != Some(sum as u64) {
            return Err("tar header checksum is wrong".into());
        }
        let size = octal(&h[124..136]).ok_or("bad tar size")?;
        let mode = octal(&h[100..108]).ok_or("bad tar mode")? as u32;
        let kind = h[156];
        let mut name = String::from_utf8_lossy(field(&h[..100])).into_owned();
        if &h[257..262] == b"ustar" {
            let prefix = String::from_utf8_lossy(field(&h[345..500])).into_owned();
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        at += 512;
        let padded = size.div_ceil(512) * 512;
        let end = at.checked_add(padded).filter(|&e| e <= len).ok_or("tar entry runs past the end")?;
        total += size;
        if total > MAX_UNPACKED || out.len() >= MAX_PACKAGE_FILES {
            return Err("package is too large".into());
        }
        match kind {
            b'0' | 0 | b'5' => {
                let is_dir = kind == b'5' || name.ends_with('/');
                let path = safe_path(name.trim_end_matches('/'))?;
                if out.iter().any(|e| e.path == path) {
                    return Err(format!("duplicate path {name:?}"));
                }
                out.push(TarEntry { path, data: vec![], mode, is_dir, offset: at, size: if is_dir { 0 } else { size } });
            }
            b'x' | b'g' => return Err("pax headers are not supported".into()),
            k => return Err(format!("unsupported tar entry type {:?}", k as char)),
        }
        at = end;
    }
}

/// Writes entries below `dir` (every path was checked by `read_tar`).
pub fn extract(files: &[TarEntry], dir: &Path) -> Result<(), String> {
    extract_with(files, dir, |f, out| out.write_all(&f.data))
}

/// Writes entries below `dir`, copying their contents from the archive `r` they were indexed in.
pub fn extract_from<R: Read + Seek>(r: &mut R, files: &[TarEntry], dir: &Path) -> Result<(), String> {
    extract_with(files, dir, |f, out| {
        r.seek(SeekFrom::Start(f.offset))?;
        std::io::copy(&mut r.by_ref().take(f.size), out).and_then(|n| if n == f.size { Ok(()) } else { Err(std::io::ErrorKind::UnexpectedEof.into()) })
    })
}

fn extract_with(files: &[TarEntry], dir: &Path, mut content: impl FnMut(&TarEntry, &mut std::fs::File) -> std::io::Result<()>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for f in files {
        let to = dir.join(&f.path);
        if f.is_dir {
            std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&to).map_err(|e| format!("{}: {e}", f.path.display()))?;
        content(f, &mut out).map_err(|e| format!("{}: {e}", f.path.display()))?;
        std::fs::set_permissions(&to, std::fs::Permissions::from_mode(if f.mode & 0o111 != 0 { 0o755 } else { 0o644 })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---- registry ----

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    name: String,
    version: String,
    previous: Option<String>,
    /// the first install day, kept across updates (`InstalledApp::purchased`)
    purchased: i64,
    exec: String,
    icon: String,
    kind: String,
    provides: Vec<String>,
}

impl Entry {
    fn app(&self, root: &Path) -> InstalledApp {
        let cur = apps_dir(root).join(&self.id).join("current");
        let exec_path = if self.exec.is_empty() { PathBuf::new() } else { cur.join(&self.exec) };
        InstalledApp { id: self.id.clone(), name: self.name.clone(), version: self.version.clone(), exec_path, icon_path: cur.join(&self.icon), dir: cur, kind: self.kind.clone(), provides: self.provides.clone(), previous: self.previous.clone(), purchased: self.purchased }
    }
}

fn registry_path(root: &Path) -> PathBuf {
    apps_dir(root).join("installed.json")
}

fn read_registry(root: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(registry_path(root)) else { return vec![] };
    let Ok(j) = json::parse(&text) else { return vec![] };
    j.get("apps")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|a| Some(Entry { id: a.str_field("id").filter(|i| valid_id(i))?, name: a.str_field("name")?, version: a.str_field("version")?, previous: a.str_field("previous"), purchased: a.get("purchased").and_then(Json::as_u64).unwrap_or(0) as i64, exec: a.str_field("exec").filter(|e| e.is_empty() || safe_path(e).is_ok()).unwrap_or_default(), icon: a.str_field("icon").filter(|e| e.is_empty() || safe_path(e).is_ok()).unwrap_or_default(), kind: a.str_field("kind").unwrap_or_default(), provides: a.get("provides").and_then(Json::as_array).unwrap_or_default().iter().filter_map(|p| p.as_str().map(str::to_string)).collect() }))
        .filter(|e| !e.exec.is_empty() || e.kind == KIND_SYSTEM || e.kind == KIND_SERVICE)
        .collect()
}

fn write_registry(root: &Path, entries: &[Entry]) -> Result<(), String> {
    let rows: Vec<String> = entries
        .iter()
        .map(|e| {
            let prev = e.previous.as_deref().map(|p| format!(",\"previous\":{}", json::quote(p))).unwrap_or_default();
            let bought = if e.purchased > 0 { format!(",\"purchased\":{}", e.purchased) } else { String::new() };
            let provides: Vec<String> = e.provides.iter().map(|p| json::quote(p)).collect();
            format!("{{\"id\":{},\"name\":{},\"version\":{}{prev}{bought},\"exec\":{},\"icon\":{},\"kind\":{},\"provides\":[{}]}}", json::quote(&e.id), json::quote(&e.name), json::quote(&e.version), json::quote(&e.exec), json::quote(&e.icon), json::quote(&e.kind), provides.join(","))
        })
        .collect();
    let path = registry_path(root);
    std::fs::create_dir_all(apps_dir(root)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format!("{{\"apps\":[{}]}}\n", rows.join(","))).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    rebuild_log_registry(root, entries);
    Ok(())
}

/// Unified Logging: rebuilds `/var/lib/bifrost/log/registry.toml` from the `[[log.subsystem]]` tables of the
/// installed manifests. Only for the real data root (or when `BIFROST_LOG_REGISTRY` redirects it), never for test
/// roots; failures are ignored so logging can never block an install.
fn rebuild_log_registry(root: &Path, entries: &[Entry]) {
    if root != Path::new("/var/lib/springboard") && std::env::var_os("BIFROST_LOG_REGISTRY").is_none() {
        return;
    }
    let texts: Vec<String> = entries.iter().filter_map(|e| std::fs::read_to_string(apps_dir(root).join(&e.id).join("current").join("manifest.toml")).ok()).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let _ = bifrost_core::log::write_registry(&refs);
}

// ---- install / uninstall ----

/// Verifies `bytes` against `sha256`, then installs the package under `root`.
pub fn install_package(root: &Path, bytes: &[u8], sha256: &str, shell: &str) -> Result<InstalledApp, String> {
    let mut h = Sha256::new();
    h.update(bytes);
    if hex(&h.finish()) != sha256.to_ascii_lowercase() {
        return Err("checksum mismatch; package discarded".into());
    }
    install_archive(root, &mut std::io::Cursor::new(bytes), shell)
}

/// Installs the (already verified) package `r` holds: the archive is indexed and
/// unpacked straight from where it is, never loaded whole.
fn install_archive<R: Read + Seek>(root: &Path, r: &mut R, shell: &str) -> Result<InstalledApp, String> {
    let files = index_tar(r)?;
    let mf = files.iter().find(|f| !f.is_dir && f.path == Path::new("manifest.toml")).ok_or("package has no manifest.toml")?;
    if mf.size > MAX_MANIFEST {
        return Err("manifest.toml is too large".into());
    }
    let mut text = vec![0u8; mf.size as usize];
    r.seek(SeekFrom::Start(mf.offset)).and_then(|_| r.read_exact(&mut text)).map_err(|e| e.to_string())?;
    let m = parse_manifest(&String::from_utf8_lossy(&text))?;
    validate(&m, &files, shell)?;

    let app_dir = apps_dir(root).join(&m.id);
    let stage = app_dir.join(format!(".{}.part", m.version));
    let target = app_dir.join(&m.version);
    let _ = std::fs::remove_dir_all(&stage);
    extract_from(r, &files, &stage)?;
    if !m.exec.is_empty() {
        use std::os::unix::fs::PermissionsExt;
        let exec = stage.join(safe_path(&m.exec)?);
        std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let _guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&stage, &target).map_err(|e| e.to_string())?;

    // flip `current` atomically: a new symlink renamed over the old one
    let old = std::fs::read_link(app_dir.join("current")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let link = app_dir.join(".current.new");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&m.version, &link).map_err(|e| e.to_string())?;
    std::fs::rename(&link, app_dir.join("current")).map_err(|e| e.to_string())?;

    let previous = old.filter(|o| *o != m.version && parse_semver(o).is_some());
    // keep only the new and the previous version
    if let Ok(rd) = std::fs::read_dir(&app_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if parse_semver(&n).is_some() && n != m.version && Some(&n) != previous.as_ref() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let mut reg = read_registry(root);
    // the first install day stays the purchase day across updates
    let purchased = reg.iter().find(|e| e.id == m.id).map(|e| e.purchased).filter(|&d| d > 0).unwrap_or_else(install_day);
    reg.retain(|e| e.id != m.id);
    let entry = Entry { id: m.id.clone(), name: m.name, version: m.version, previous, purchased, exec: m.exec, icon: m.icon, kind: m.kind, provides: m.provides };
    let app = entry.app(root);
    reg.push(entry);
    write_registry(root, &reg)?;
    bifrost_core::log::shell_logger("store").notice(format!("installed {} {}", app.id, app.version));
    Ok(app)
}

/// Downloads a package to a `.part` file under `root` while hashing it, checks
/// the SHA-256 and installs it from there: the package is never held in memory.
/// Error returned when a download is stopped by its cancel flag; the `.part` file is kept.
pub const CANCELLED: &str = "download cancelled";

pub fn install_download(root: &Path, id: &str, url: &str, size: u64, sha256: &str, shell: &str, progress: &std::sync::atomic::AtomicU64, cancel: &std::sync::atomic::AtomicBool) -> Result<InstalledApp, String> {
    let client = crate::privd_client::connect();
    if client.is_local() {
        return install_download_local(root, url, size, sha256, shell, progress, cancel);
    }
    // user session: privd fetches the catalogue and the package itself and checks the catalogue hash; the
    // session passes only the app id, so there is no user-staged file for root to open
    let got = client.store_install(id).map_err(|e| e.to_string())?;
    if got != id {
        return Err("Package identity mismatch".into());
    }
    let _ = (url, size, sha256, progress, cancel);
    installed_in(root).into_iter().find(|a| a.id == id).ok_or_else(|| "installed app not found in the registry".to_string())
}

/// The root-side install: downloads under `root` and installs from there.
pub fn install_download_local(root: &Path, url: &str, size: u64, sha256: &str, shell: &str, progress: &std::sync::atomic::AtomicU64, cancel: &std::sync::atomic::AtomicBool) -> Result<InstalledApp, String> {
    let part = apps_dir(root).join(format!(".download-{}.part", sha256.to_ascii_lowercase().chars().filter(char::is_ascii_alphanumeric).take(16).collect::<String>()));
    let result = download_to(url, size, &part, progress, cancel).and_then(|got| {
        if got != sha256.to_ascii_lowercase() {
            return Err("checksum mismatch; package discarded".to_string());
        }
        let mut file = std::fs::File::open(&part).map_err(|e| e.to_string())?;
        install_archive(root, &mut file, shell)
    });
    if result.is_ok() || result.as_ref().is_err_and(|e| e.contains("checksum mismatch")) {let _ = std::fs::remove_file(&part);}
    result
}

/// privd's `store.install` (the only form it takes: an app id): fetch the PC App Store
/// catalogue, find the app and install the catalogued package (checksum from the catalogue).
pub fn install_store_app(root: &Path, id: &str) -> Result<InstalledApp, String> {
    use super::{api, net};
    let bytes = net::get(&net::absolute("/api/v1/pc/apps.json"), 4 * 1024 * 1024).map_err(|e| e.to_string())?;
    let catalogue = api::parse_catalog(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let app = catalogue.iter().find(|a| a.id == id && Kind::parse(&a.kind).is_some()).ok_or("App not found in PC App Store")?;
    let done = std::sync::atomic::AtomicU64::new(0);
    let installed = install_download_local(root, &net::absolute(&app.package), app.size, &app.sha256, shell_version(), &done, &std::sync::atomic::AtomicBool::new(false))?;
    if installed.id != id {
        return Err("Package identity mismatch".into());
    }
    Ok(installed)
}

/// Roll back to the previous version, when one is kept.
pub fn rollback(root: &Path, id: &str) -> Result<(), String> {
    let _guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let mut reg = read_registry(root);
    let e = reg.iter_mut().find(|e| e.id == id).ok_or("not installed")?;
    let prev = e.previous.take().ok_or("no previous version")?;
    let app_dir = apps_dir(root).join(id);
    let link = app_dir.join(".current.new");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&prev, &link).map_err(|e| e.to_string())?;
    std::fs::rename(&link, app_dir.join("current")).map_err(|e| e.to_string())?;
    let bad = std::mem::replace(&mut e.version, prev);
    let _ = std::fs::remove_dir_all(app_dir.join(bad));
    write_registry(root, &reg)
}

/// Removes an app and its package files (not its data directory); a user session asks privd.
pub fn uninstall(root: &Path, id: &str) -> Result<(), String> {
    let client = crate::privd_client::connect();
    if client.is_local() {
        return uninstall_local(root, id);
    }
    client.store_uninstall(id).map_err(|e| e.to_string())
}

/// privd's side of `uninstall`, and the root shell's.
pub fn uninstall_local(root: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid app id".into());
    }
    let _guard = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let mut reg = read_registry(root);
    reg.retain(|e| e.id != id);
    write_registry(root, &reg)?;
    let _ = std::fs::remove_dir_all(apps_dir(root).join(id));
    Ok(())
}

/// Downloads a package to `dest` with progress, checking the announced size; returns the
/// SHA-256 (hex) of what was written, hashed as it arrived.
pub fn download_to(url: &str, size: u64, dest: &Path, progress: &std::sync::atomic::AtomicU64, cancel: &std::sync::atomic::AtomicBool) -> Result<String, String> {
    download_to_with(size, dest, progress, cancel, |start, sink| super::net::get_streamed_from(url, size, start, sink))
}

fn download_to_with(size: u64, dest: &Path, progress: &std::sync::atomic::AtomicU64, cancel: &std::sync::atomic::AtomicBool,
    mut fetch: impl FnMut(u64, &mut dyn FnMut(&[u8]) -> std::io::Result<()>) -> std::io::Result<()>) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    if size == 0 || size > 256 * 1024 * 1024 {
        return Err("package size is not acceptable".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    use std::io::{Read,Seek,SeekFrom};
    let mut file=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dest).map_err(|e|e.to_string())?;
    let mut written=file.metadata().map_err(|e|e.to_string())?.len();
    if written>size {file.set_len(0).map_err(|e|e.to_string())?;written=0;}
    // Hashing only covers bytes seen in this call. A resumed partial is not re-read here; it is
    // hashed once on completion so a failed attempt costs no extra I/O.
    let mut hash=Sha256::new();let mut buffer=[0;32*1024];
    let mut hash_covers_all=written==0;
    if !hash_covers_all {file.seek(SeekFrom::End(0)).map_err(|e|e.to_string())?;}
    progress.store(written,Ordering::Relaxed);
    for attempt in 0..3 {
        if cancel.load(Ordering::Relaxed) {return Err(CANCELLED.into());}
        if written==size {break;}
        let result=fetch(written, &mut |b| {
            if cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other(CANCELLED));}
            file.write_all(b)?;hash.update(b);written+=b.len() as u64;progress.store(written,Ordering::Relaxed);Ok(())
        });
        match result {
            Ok(())=>break,
            Err(_) if cancel.load(Ordering::Relaxed)=>return Err(CANCELLED.into()),
            Err(e) if e.kind()==std::io::ErrorKind::Unsupported=>{
                file.set_len(0).map_err(|e|e.to_string())?;file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;written=0;hash=Sha256::new();hash_covers_all=true;progress.store(0,Ordering::Relaxed);
            }
            Err(e)=>{if attempt==2{return Err(e.to_string());}std::thread::sleep(std::time::Duration::from_millis(250*(attempt+1)));}
        }
    }
    if written!=size {return Err(format!("download is {written} bytes, expected {size}"));}
    if !hash_covers_all {
        // Resumed download: hash the whole file, including the bytes from earlier attempts.
        file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
        hash=Sha256::new();
        loop {let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}hash.update(&buffer[..n]);}
    }
    Ok(hex(&hash.finish()))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A ustar archive of `(path, bytes, mode)` entries.
    pub fn tar(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut out = vec![];
        for (name, data, mode) in entries {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[156] = b'0';
            h[257..262].copy_from_slice(b"ustar");
            h[148..156].fill(b' ');
            let sum: u32 = h.iter().map(|&b| b as u32).sum();
            h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
            h[155] = b' ';
            out.extend_from_slice(&h);
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    const MANIFEST: &str = "id = \"is.olibuijr.calc\"\nname = \"Calc\"\nversion = \"1.0.0\"\nexec = \"bin/calc\"\nicon = \"icon.png\"\nmin_shell = \"0.1.0\"\nsummary = \"x\"\ncategory = \"Utilities\"\npermissions = [\"network\", \"audio\"]\n";

    fn pkg(manifest: &str) -> Vec<u8> {
        tar(&[("manifest.toml", manifest.as_bytes(), 0o644), ("bin/calc", b"ELF", 0o644), ("icon.png", b"PNG", 0o644)])
    }

    fn sum(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        hex(&h.finish())
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("apps-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_archive_on_disk_is_indexed_and_unpacked_without_loading_it() {
        let dir = scratch("indexed");
        let bytes = pkg(MANIFEST);
        let mut cur = std::io::Cursor::new(&bytes);
        let index = index_tar(&mut cur).unwrap();
        assert_eq!(index.iter().map(|f| f.path.to_string_lossy().into_owned()).collect::<Vec<_>>(), ["manifest.toml", "bin/calc", "icon.png"]);
        assert!(index.iter().all(|f| f.data.is_empty()), "headers only");
        assert_eq!(index, read_tar(&bytes).unwrap().into_iter().map(|mut f| {
            f.data.clear();
            f
        }).collect::<Vec<_>>());
        extract_from(&mut cur, &index, &dir.join("out")).unwrap();
        assert_eq!(std::fs::read(dir.join("out/bin/calc")).unwrap(), b"ELF");
        assert_eq!(std::fs::read(dir.join("out/icon.png")).unwrap(), b"PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn intent_tables_do_not_leak_into_the_app_manifest() {
        let m = parse_manifest("id = \"is.x.app\"\nname = \"X\"\nversion = \"1.0.0\"\nexec = \"x\"\n\n[[intent]]\nid = \"new-note\"\nshortcut = true\n").unwrap();
        assert_eq!(m.id, "is.x.app");
        assert_eq!(m.exec, "x");
    }

    #[test]
    fn a_manifest_can_ask_for_a_full_size_window() {
        let m = parse_manifest("id = \"a.b\"\nwindow = \"full-size\"\n").unwrap();
        assert!(m.full_size_content());
        assert!(!parse_manifest("id = \"a.b\"\n").unwrap().full_size_content(), "the default keeps the title row");
    }

    #[test]
    fn manifest_parses() {
        let m = parse_manifest(MANIFEST).unwrap();
        assert_eq!((m.id.as_str(), m.version.as_str(), m.exec.as_str()), ("is.olibuijr.calc", "1.0.0", "bin/calc"));
        assert_eq!(m.permissions, ["network", "audio"]);
        assert!(parse_manifest("id = 5").is_err() && parse_manifest("garbage").is_err());
    }

    #[test]
    fn id_and_semver_rules() {
        assert!(valid_id("is.olibuijr.calc") && valid_id("a.b-c_d") && !valid_id("calc") && !valid_id("Is.Foo") && !valid_id("a..b") && !valid_id("a/b.c") && !valid_id("../x.y"));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("1.2.3-rc1"), Some((1, 2, 3)));
        assert!(parse_semver("1.2").is_none() && parse_semver("1.2.3.4").is_none() && parse_semver("a.b.c").is_none());
    }

    #[test]
    fn validation_checks_exec_icon_and_min_shell() {
        let files = read_tar(&pkg(MANIFEST)).unwrap();
        let m = parse_manifest(MANIFEST).unwrap();
        assert!(validate(&m, &files, "0.1.0").is_ok());
        assert!(validate(&m, &files, "0.0.9").unwrap_err().contains("needs shell"));
        let mut bad = m.clone();
        bad.exec = "bin/missing".into();
        assert!(validate(&bad, &files, "0.1.0").is_err());
        bad = m.clone();
        bad.exec = "../calc".into();
        assert!(validate(&bad, &files, "0.1.0").is_err());
        bad = m.clone();
        bad.version = "x".into();
        assert!(validate(&bad, &files, "0.1.0").is_err());
        bad = m;
        bad.id = "nodots".into();
        assert!(validate(&bad, &files, "0.1.0").is_err());
    }

    #[test]
    fn an_app_provides_its_wallpapers_through_the_manifest() {
        let text = format!("{MANIFEST}wallpaper_kind = \"day\"\nwallpapers = [\"wallpapers/a.png\", \"wallpapers/b.png\"]\n");
        let m = parse_manifest(&text).unwrap();
        assert_eq!(m.wallpaper_kind, "day");
        assert_eq!(m.wallpapers, vec!["wallpapers/a.png".to_string(), "wallpapers/b.png".to_string()]);
        // the pictures must be in the package: without them the install is refused
        let files = read_tar(&pkg(&text)).unwrap();
        assert!(validate(&m, &files, "0.1.0").unwrap_err().contains("not inside the package"));
        let with_pictures = read_tar(&tar(&[
            ("manifest.toml", text.as_bytes(), 0o644),
            ("bin/calc", b"ELF", 0o644),
            ("icon.png", b"PNG", 0o644),
            ("wallpapers/a.png", b"PNG", 0o644),
            ("wallpapers/b.png", b"PNG", 0o644),
        ]))
        .unwrap();
        assert!(validate(&m, &with_pictures, "0.1.0").is_ok());
        // an app that declares no wallpaper is unaffected
        assert!(parse_manifest(MANIFEST).unwrap().wallpapers.is_empty());
    }

    #[test]
    fn tar_reads_files_and_rejects_unsafe_paths() {
        let files = read_tar(&tar(&[("./a.txt", b"hi", 0o644), ("d/b.sh", b"#!", 0o755)])).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].path.as_path(), files[0].data.as_slice()), (Path::new("a.txt"), &b"hi"[..]));
        for bad in ["../evil", "a/../../evil", "/etc/passwd", "a/./../../x"] {
            assert!(read_tar(&tar(&[(bad, b"x", 0o644)])).is_err(), "{bad}");
        }
        let mut broken = tar(&[("a", b"x", 0o644)]);
        broken[0] ^= 1;
        assert!(read_tar(&broken).is_err() && read_tar(&[0u8; 100]).is_err());
        let mut link = tar(&[("l", b"", 0o777)]);
        link[156] = b'2';
        let sum: u32 = link[..512].iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u32 }).sum();
        link[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        assert!(read_tar(&link).unwrap_err().contains("unsupported"));
    }

    #[test]
    fn install_unpacks_flips_current_and_registers() {
        let root = scratch("install");
        let bytes = pkg(MANIFEST);
        let app = install_package(&root, &bytes, &sum(&bytes), "0.1.0").unwrap();
        assert_eq!(app.id, "is.olibuijr.calc");
        assert_eq!(std::fs::read(&app.exec_path).unwrap(), b"ELF");
        use std::os::unix::fs::PermissionsExt;
        assert!(std::fs::metadata(&app.exec_path).unwrap().permissions().mode() & 0o111 != 0);
        assert_eq!(std::fs::read_link(apps_dir(&root).join("is.olibuijr.calc/current")).unwrap(), Path::new("1.0.0"));
        assert_eq!(installed_in(&root), vec![app.clone()]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn upgrade_keeps_one_previous_and_rollback_restores_it() {
        let root = scratch("upgrade");
        for v in ["1.0.0", "1.1.0", "1.2.0"] {
            let b = pkg(&MANIFEST.replace("1.0.0", v));
            install_package(&root, &b, &sum(&b), "0.1.0").unwrap();
        }
        let dir = apps_dir(&root).join("is.olibuijr.calc");
        assert!(dir.join("1.2.0").is_dir() && dir.join("1.1.0").is_dir() && !dir.join("1.0.0").exists());
        assert_eq!(installed_in(&root)[0].version, "1.2.0");
        rollback(&root, "is.olibuijr.calc").unwrap();
        assert_eq!(installed_in(&root)[0].version, "1.1.0");
        assert_eq!(std::fs::read_link(dir.join("current")).unwrap(), Path::new("1.1.0"));
        uninstall(&root, "is.olibuijr.calc").unwrap();
        assert!(installed_in(&root).is_empty() && !dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sha_mismatch_is_rejected_and_nothing_is_written() {
        let root = scratch("sha");
        let bytes = pkg(MANIFEST);
        let err = install_package(&root, &bytes, &"0".repeat(64), "0.1.0").unwrap_err();
        assert!(err.contains("checksum"));
        assert!(installed_in(&root).is_empty() && !apps_dir(&root).join("is.olibuijr.calc").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn too_new_package_is_rejected() {
        let root = scratch("minshell");
        let b = pkg(&MANIFEST.replace("0.1.0", "9.0.0"));
        assert!(install_package(&root, &b, &sum(&b), "0.1.0").unwrap_err().contains("needs shell"));
        let _ = std::fs::remove_dir_all(root);
    }

    const SYSTEM: &str = "id = \"is.olibuijr.keyboard\"\nname = \"Keyboard\"\nversion = \"1.0.0\"\nkind = \"system\"\nprovides = [\"keyboard\"]\nicon = \"icon.png\"\nsummary = \"x\"\ncategory = \"System\"\n";

    #[test]
    fn system_manifest_may_omit_exec_but_an_app_may_not() {
        let m = parse_manifest(SYSTEM).unwrap();
        assert!(m.is_system() && m.exec.is_empty());
        assert_eq!(m.provides, ["keyboard"]);
        let files = read_tar(&tar(&[("manifest.toml", SYSTEM.as_bytes(), 0o644), ("icon.png", b"PNG", 0o644)])).unwrap();
        assert!(validate(&m, &files, "0.1.0").is_ok());
        let mut app = m.clone();
        app.kind = "app".into();
        assert!(validate(&app, &files, "0.1.0").is_err(), "an app needs an exec");
        app.kind = String::new();
        assert!(validate(&app, &files, "0.1.0").is_err());
        let mut odd = m.clone();
        odd.kind = "widget".into();
        assert!(validate(&odd, &files, "0.1.0").unwrap_err().contains("kind"));
        let mut named = m.clone();
        named.exec = "bin/missing".into();
        assert!(validate(&named, &files, "0.1.0").is_err(), "a named exec must exist");
        let mut sneaky = parse_manifest(MANIFEST).unwrap();
        sneaky.provides = vec!["keyboard".into()];
        assert!(validate(&sneaky, &read_tar(&pkg(MANIFEST)).unwrap(), "0.1.0").is_err());
    }

    #[test]
    fn a_system_package_with_an_exec_installs_a_runnable_provider() {
        // the keyboard 1.1: a standalone process the shell spawns on demand
        let manifest = SYSTEM.replace("version = \"1.0.0\"", "version = \"1.1.0\"\nexec = \"bifrost-keyboard\"");
        let root = scratch("system-exec");
        let bytes = tar(&[("manifest.toml", manifest.as_bytes(), 0o644), ("icon.png", b"PNG", 0o644), ("bifrost-keyboard", b"ELF", 0o644)]);
        let app = install_package(&root, &bytes, &sum(&bytes), "0.1.0").unwrap();
        assert!(app.is_system() && app.exec_path.ends_with("current/bifrost-keyboard"));
        use std::os::unix::fs::PermissionsExt;
        assert!(std::fs::metadata(&app.exec_path).unwrap().permissions().mode() & 0o111 != 0, "made executable");
        assert_eq!(app.provides, ["keyboard"]);
        assert_eq!(installed_in(&root), vec![app]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_data_only_system_package_installs_and_registers() {
        let root = scratch("system");
        let bytes = tar(&[("manifest.toml", SYSTEM.as_bytes(), 0o644), ("icon.png", b"PNG", 0o644), ("assets/words/en.txt", b"the 5\n", 0o644)]);
        let app = install_package(&root, &bytes, &sum(&bytes), "0.1.0").unwrap();
        assert!(app.is_system() && app.exec_path.as_os_str().is_empty());
        assert_eq!(app.provides, ["keyboard"]);
        assert_eq!(std::fs::read(app.dir.join("assets/words/en.txt")).unwrap(), b"the 5\n");
        let back = installed_in(&root);
        assert_eq!(back, vec![app]);
        assert!(back[0].is_system());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod resumed_download_review_tests {
    use super::*;
    #[test]
    fn review_package_download_resumes_and_hashes_existing_bytes() {
        use std::{io::{Read,Write},net::TcpListener,sync::atomic::AtomicU64};
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("part");std::fs::write(&path,b"abc").unwrap();
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let server=std::thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request=Vec::new();let mut byte=[0];
            while !request.ends_with(b"\r\n\r\n") {stream.read_exact(&mut byte).unwrap();request.push(byte[0]);}
            assert!(String::from_utf8_lossy(&request).contains("Range: bytes=3-"));
            stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef").unwrap();
        });
        // A test transport speaks to the local fixture, while the real client
        // still checks an HTTPS URL and enforces range/framing/size semantics.
        struct Fixture(std::net::SocketAddr);
        impl crate::deildu::http::Transport for Fixture {
            fn connect(&self, _: &str, _: u16, tls: bool) -> std::io::Result<Box<dyn crate::deildu::http::Stream>> {
                assert!(tls);
                Ok(Box::new(std::net::TcpStream::connect(self.0)?))
            }
        }
        let client = crate::deildu::http::Client::new(std::sync::Arc::new(Fixture(address)), Default::default()).require_https();
        let progress=AtomicU64::new(0);
        let got=download_to_with(6,&path,&progress,&std::sync::atomic::AtomicBool::new(false),|start,sink| client.get_streamed("https://fixture.invalid/package",start,6-start,sink)).unwrap();
        let mut hash=Sha256::new();hash.update(b"abcdef");assert_eq!(got,hex(&hash.finish()));
        assert_eq!(std::fs::read(path).unwrap(),b"abcdef");server.join().unwrap();
    }
}
