//! Software Update: checks a manifest (`latest.json`) on the LAN server,
//! downloads the binary to `springboard.new` (checking size and SHA-256),
//! and leaves the rest to `tools/boot.sh`, which promotes a staged binary at
//! the next shell start under probation and rolls back if it fails. Nothing
//! here changes that contract: this only writes `springboard.new`, atomically,
//! and only once the hash matches. HTTPS only (the shared rustls client), on
//! worker threads; the UI polls `Updater::poll`. TLS authenticates the server;
//! there is no client-side manifest signature.

use super::sha256::{hex, Sha256};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

pub const DEFAULT_URL: &str = "https://bifrost.olibuijr.com/ota/springboard/latest.json";
/// The pref that overrides `DEFAULT_URL`.
pub const URL_PREF: &str = "update_url";
const MAX_MANIFEST: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    /// git build stamp, when the server gives one
    pub build: Option<String>,
    pub sha256: String,
    pub size: u64,
    /// the binary's URL; by default `springboard` next to the manifest
    pub url: String,
    /// release notes shown on the Software Update page (`notes` in the manifest)
    pub notes: Option<String>,
    /// per-release notes, newest first (`changelog` in the manifest)
    pub changelog: Vec<super::store::api::Release>,
}

pub fn parse_manifest(text: &str, manifest_url: &str) -> Result<Manifest, String> {
    use super::store::json::Json;
    let j = super::store::json::parse(text).map_err(|e| format!("manifest is not JSON: {e}"))?;
    // a string, or a bare number kept as written (`"version": 2`)
    let field = |key: &str| match j.get(key) {
        Some(Json::Str(s)) => Some(s.clone()),
        Some(Json::Num(n)) => Some(n.to_string()),
        _ => None,
    };
    let version = field("version").ok_or("manifest has no version")?;
    let sha256 = field("sha256").ok_or("manifest has no sha256")?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("manifest sha256 is not 64 hex digits".into());
    }
    let size = j.get("size").and_then(Json::as_u64).ok_or("manifest has no size")?;
    let url = field("url").map(|u| resolve(manifest_url, &u)).unwrap_or_else(|| resolve(manifest_url, "springboard"));
    if !url.starts_with("https://") {
        return Err("update URLs must be https://".into());
    }
    let changelog = super::store::api::parse_changelog(j.get("changelog"));
    Ok(Manifest { version, build: field("build"), sha256, size, url, notes: field("notes").filter(|n| !n.is_empty()), changelog })
}

/// `rel` against the directory of `base` (absolute URLs pass through).
fn resolve(base: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    let dir = base.rfind('/').map_or(base, |i| &base[..=i]);
    format!("{dir}{}", rel.trim_start_matches('/'))
}

/// Whether `m` is an update: a higher semver (a version without one falls back to the build stamp).
pub fn is_newer(m: &Manifest, version: &str, git: &str) -> bool {
    // a semver that is older than the running one is never an update, whatever the stamp says
    if let (Some(o), Some(r)) = (super::store::install::parse_semver(&m.version), super::store::install::parse_semver(version)) {
        // the same version is never an update either: a dev push (`tab deploy`) runs newer code
        // under the released number, and the release would roll it back; releases bump the version
        return o > r;
    }
    match &m.build {
        Some(b) => b != git,
        None => m.version != version,
    }
}

/// GET `url`, handing the body to `sink` in pieces. HTTPS only, through the
/// shared rustls client (in byte ranges when the size is known).
fn get(url: &str, size: u64, sink: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
    if !url.starts_with("https://") {
        return Err(io::Error::other("updates require https://"));
    }
    if size > 0 {
        super::store::net::get_chunked(url, size, sink)
    } else {
        let mut sink = sink;
        sink(&super::store::net::get(url, MAX_MANIFEST + 1)?)
    }
}

/// Fetches the manifest at `url` (HTTPS only).
pub fn fetch_manifest(url: &str) -> Result<Manifest, String> {
    if !url.starts_with("https://") {
        return Err("update URL must be https://".into());
    }
    let body = fetch_body(url)?;
    parse_manifest(&String::from_utf8_lossy(&body), url)
}

fn fetch_body(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    get(url, 0, |b| {
        body.extend_from_slice(b);
        if body.len() > MAX_MANIFEST {
            return Err(io::Error::other("manifest too large"));
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(body)
}

/// Downloads the binary into `<dir>/springboard.part`, verifies size and
/// hash, then renames it to `springboard.new` (what boot.sh promotes).
pub fn download(dir: &Path, m: &Manifest, progress: &AtomicU64) -> Result<(), String> {
    stage(dir, m, progress, |sink| get(&m.url, m.size, sink))
}

/// Writes what `fetch` delivers to `springboard.part`, checks it against `m`, and renames it to
/// `springboard.new`. The source is a parameter so the check is testable without a network.
fn stage(dir: &Path, m: &Manifest, progress: &AtomicU64, fetch: impl FnOnce(&mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()>) -> Result<(), String> {
    let part = dir.join("springboard.part");
    let result = (|| -> Result<(), String> {
        let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut h = Sha256::new();
        let mut total = 0u64;
        let mut sink = |b: &[u8]| -> io::Result<()> {
            total += b.len() as u64;
            if total > m.size {
                return Err(io::Error::other("download is larger than announced"));
            }
            h.update(b);
            f.write_all(b)?;
            progress.store(total, Ordering::Relaxed);
            Ok(())
        };
        fetch(&mut sink).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        if total != m.size {
            return Err(format!("download is {total} bytes, expected {}", m.size));
        }
        if hex(&h.finish()) != m.sha256 {
            return Err("checksum mismatch; update discarded".into());
        }
        std::fs::rename(&part, dir.join("springboard.new")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Manifest),
    Downloading,
    /// `springboard.new` is in place; restart to install
    Staged(Manifest),
    Failed(String),
}

/// Where updates come from: pacman on an Arch desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Pacman,
}

impl Source {
    pub fn detect() -> Self {
        Source::Pacman
    }
}

enum Msg {
    Checked(Result<Manifest, String>),
    Packaged(Result<super::pacman_update::Found, String>),
    Downloaded(Manifest, Result<(), String>),
}

pub struct Updater {
    pub state: State,
    pub done: Arc<AtomicU64>,
    total: u64,
    rx: Option<Receiver<Msg>>,
    url: String,
    dir: PathBuf,
    source: Source,
    /// the installed package version (pacman source), known after the first check
    installed: Option<String>,
}

impl Updater {
    pub fn new(dir: PathBuf, url: Option<&str>) -> Self {
        Self::with_source(dir, url, Source::detect())
    }

    pub fn with_source(dir: PathBuf, url: Option<&str>, source: Source) -> Self {
        // a binary staged earlier (and not yet promoted) still counts as staged
        let state = if dir.join("springboard.new").exists() { State::Staged(Manifest { version: "staged".into(), build: None, sha256: String::new(), size: 0, url: String::new(), notes: None, changelog: vec![] }) } else { State::Idle };
        Updater { state, done: Arc::new(AtomicU64::new(0)), total: 0, rx: None, url: url.filter(|u| !u.is_empty()).unwrap_or(DEFAULT_URL).to_string(), dir, source, installed: None }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    /// The installed package version, when pacman has told.
    pub fn installed(&self) -> Option<&str> {
        self.installed.as_deref()
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Copies another updater's state (the background one) without a worker of its own:
    /// the progress counter is shared, so a running download shows its percentage.
    pub fn mirror(&mut self, other: &Updater) {
        if self.busy() {
            return;
        }
        self.state = other.state.clone();
        self.total = other.total;
        self.done = other.done.clone();
        self.installed = other.installed.clone();
    }

    /// Download progress 0..=1.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f32 / self.total as f32).min(1.0)
        }
    }

    pub fn check(&mut self) {
        if self.busy() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.state = State::Checking;
        if self.source == Source::Pacman {
            crate::bare::sys::spawn_named("update-pacman-check", move || {
                let _ = tx.send(Msg::Packaged(super::pacman_update::check()));
            });
            return;
        }
        let url = self.url.clone();
        crate::bare::sys::spawn_named("update-check", move || {
            let _ = tx.send(Msg::Checked(fetch_manifest(&url)));
        });
    }

    /// Marks `m` as the available update (a background check found it).
    pub fn set_available(&mut self, m: Manifest) {
        if !self.busy() && !matches!(self.state, State::Staged(_) | State::Downloading) {
            self.state = State::Available(m);
        }
    }

    pub fn start_download(&mut self) {
        let State::Available(m) = self.state.clone() else { return };
        if self.busy() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.state = State::Downloading;
        self.total = m.size;
        self.done.store(0, Ordering::Relaxed);
        if self.source == Source::Pacman {
            // the user confirmed on the page: pacman -Syu springboard-shell
            crate::bare::sys::spawn_named("update-pacman", move || {
                let r = super::pacman_update::install();
                let _ = tx.send(Msg::Downloaded(m, r));
            });
            return;
        }
        let (dir, done) = (self.dir.clone(), self.done.clone());
        crate::bare::sys::spawn_named("update-download", move || {
            let r = download(&dir, &m, &done);
            let _ = tx.send(Msg::Downloaded(m, r));
        });
    }

    /// Takes a finished result; true when the state changed.
    pub fn poll(&mut self, version: &str, git: &str) -> bool {
        let Some(rx) = &self.rx else { return false };
        let msg = match rx.try_recv() {
            Ok(m) => m,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                self.state = State::Failed("update worker stopped".into());
                return true;
            }
        };
        self.rx = None;
        self.state = match msg {
            Msg::Packaged(Ok(found)) => {
                self.installed = found.installed;
                found.update.map_or(State::UpToDate, State::Available)
            }
            Msg::Packaged(Err(e)) => State::Failed(e),
            Msg::Checked(Ok(m)) if is_newer(&m, version, git) => State::Available(m),
            Msg::Checked(Ok(_)) => State::UpToDate,
            Msg::Checked(Err(e)) | Msg::Downloaded(_, Err(e)) => State::Failed(e),
            Msg::Downloaded(m, Ok(())) => State::Staged(m),
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn manifest_parses_and_resolves_the_binary_url() {
        let text = format!("{{\"version\": \"0.2.0\", \"build\":\"abc123\", \"sha256\": \"{SUM}\", \"size\": 1234}}");
        let m = parse_manifest(&text, "https://updates.example/springboard/latest.json").unwrap();
        assert_eq!((m.version.as_str(), m.size, m.build.as_deref()), ("0.2.0", 1234, Some("abc123")));
        assert_eq!(m.url, "https://updates.example/springboard/springboard");
        let abs = parse_manifest(&format!("{{\"version\":\"1\",\"sha256\":\"{SUM}\",\"size\":1,\"url\":\"https://h/x\"}}"), "https://a/b.json").unwrap();
        assert_eq!(abs.url, "https://h/x");
        // plain http is refused, whether the manifest or the binary URL names it
        assert!(parse_manifest(&text, "http://192.168.1.12:8190/springboard/latest.json").is_err());
        assert!(parse_manifest(&format!("{{\"version\":\"1\",\"sha256\":\"{SUM}\",\"size\":1,\"url\":\"http://h/x\"}}"), "https://a/b.json").is_err());
        assert!(fetch_manifest("http://h/latest.json").is_err() && get("http://h/x", 0, |_| Ok(())).is_err());
    }

    #[test]
    fn manifest_fields_survive_order_escapes_and_nesting() {
        let text = format!(
            "{{\"changelog\":[{{\"version\":\"9.9.9\",\"notes\":\"x\"}}],\"notes\":\"line1\\nline2 \\u00e1 \\\"q\\\"\",\"size\":7,\"sha256\":\"{SUM}\",\"version\":\"1.2.3\"}}"
        );
        let m = parse_manifest(&text, "https://h/latest.json").unwrap();
        assert_eq!((m.version.as_str(), m.size), ("1.2.3", 7), "top-level version, not the changelog's");
        assert_eq!(m.notes.as_deref(), Some("line1\nline2 \u{e1} \"q\""));
    }

    #[test]
    fn bad_manifests_are_rejected() {
        let u = "https://h/latest.json";
        assert!(parse_manifest("{}", u).is_err());
        assert!(parse_manifest("{\"version\":\"1\",\"sha256\":\"zz\",\"size\":1}", u).is_err());
        assert!(parse_manifest(&format!("{{\"version\":\"1\",\"sha256\":\"{SUM}\"}}"), u).is_err());
    }

    #[test]
    fn newer_compares_build_when_given_else_version() {
        let mut m = Manifest { version: "0.1.0".into(), build: Some("aaa".into()), sha256: SUM.into(), size: 1, url: String::new(), notes: None, changelog: vec![] };
        assert!(!is_newer(&m, "0.1.0", "bbb") && !is_newer(&m, "0.1.0", "aaa"), "same version: a dev build is not rolled back");
        assert!(is_newer(&m, "0.0.9", "aaa"));
        m.version = "nightly".into();
        assert!(is_newer(&m, "0.1.0", "bbb") && !is_newer(&m, "0.1.0", "aaa"), "no semver: the build stamp decides");
    }

    #[test]
    fn semver_wins_over_the_build_stamp_and_notes_parse() {
        let m = |v: &str, b: Option<&str>| Manifest { version: v.into(), build: b.map(Into::into), sha256: SUM.into(), size: 1, url: String::new(), notes: None, changelog: vec![] };
        assert!(is_newer(&m("0.10.0", Some("aaa")), "0.9.0", "aaa"), "numeric, not string, order");
        assert!(!is_newer(&m("0.8.0", Some("zzz")), "0.9.0", "aaa"), "an older semver is no update");
        assert!(!is_newer(&m("0.9.0", Some("zzz")), "0.9.0", "aaa") && !is_newer(&m("0.9.0", Some("aaa")), "0.9.0", "aaa"));
        assert!(is_newer(&m("1.0.0", None), "0.9.9", "x") && !is_newer(&m("0.9.9", None), "0.9.9", "x"));
        let text = format!("{{\"version\":\"1\",\"sha256\":\"{SUM}\",\"size\":1,\"notes\":\"Faster Wi-Fi.\"}}");
        assert_eq!(parse_manifest(&text, DEFAULT_URL).unwrap().notes.as_deref(), Some("Faster Wi-Fi."));
    }

    #[test]
    fn https_manifest_urls_resolve_and_default_is_https() {
        assert!(DEFAULT_URL.starts_with("https://bifrost.olibuijr.com/ota/springboard/"));
        let text = format!("{{\"version\": \"0.2.0\", \"sha256\": \"{SUM}\", \"size\": 9}}");
        assert_eq!(parse_manifest(&text, DEFAULT_URL).unwrap().url, "https://bifrost.olibuijr.com/ota/springboard/springboard");
        assert_eq!(resolve(DEFAULT_URL, "https://x/y"), "https://x/y");
    }

    #[test]
    fn staging_verifies_the_hash_and_leaves_no_partial_file() {
        let dir = std::env::temp_dir().join(format!("sb-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest { version: "2".into(), build: None, sha256: SUM.into(), size: 3, url: "https://x/springboard".into(), notes: None, changelog: vec![] };
        let p = AtomicU64::new(0);
        stage(&dir, &m, &p, |sink| sink(b"abc")).unwrap();
        assert_eq!(std::fs::read(dir.join("springboard.new")).unwrap(), b"abc");
        assert_eq!(p.load(Ordering::Relaxed), 3);
        std::fs::remove_file(dir.join("springboard.new")).unwrap();
        m.sha256 = "0".repeat(64);
        assert!(stage(&dir, &m, &p, |sink| sink(b"abc")).is_err());
        assert!(!dir.join("springboard.new").exists() && !dir.join("springboard.part").exists());
        m.sha256 = SUM.into();
        m.size = 2;
        assert!(stage(&dir, &m, &p, |sink| sink(b"abc")).is_err(), "a body longer than announced is discarded");
        assert!(!dir.join("springboard.part").exists());
    }

    fn mf(v: &str, b: Option<&str>) -> Manifest {
        Manifest { version: v.into(), build: b.map(Into::into), sha256: SUM.into(), size: 3, url: "https://x/springboard".into(), notes: None, changelog: vec![] }
    }

    #[test]
    fn malformed_versions_fall_back_to_build_or_string_inequality() {
        // neither side parses as semver: the build stamp decides, else plain inequality
        assert!(is_newer(&mf("", None), "0.1.0", "x"), "an empty version differs from the running one");
        assert!(!is_newer(&mf("garbage", None), "garbage", "x"));
        assert!(!is_newer(&mf("1.x", Some("g")), "0.1.0", "g"));
        assert!(is_newer(&mf("1.x", Some("h")), "0.1.0", "g"));
    }

    #[test]
    fn manifest_validation_rejects_each_bad_field() {
        let u = "https://h/latest.json";
        let ok = |sum: &str, size: &str| format!("{{\"version\":\"1\",\"sha256\":\"{sum}\",\"size\":{size}}}");
        assert!(parse_manifest(&ok(SUM, "5"), u).is_ok());
        assert!(parse_manifest(&ok(&SUM[..63], "5"), u).is_err(), "63 hex digits");
        assert!(parse_manifest(&ok(&format!("{SUM}0"), "5"), u).is_err(), "65 hex digits");
        assert!(parse_manifest(&ok(&format!("g{}", &SUM[1..]), "5"), u).is_err(), "non-hex digit");
        assert!(parse_manifest(&ok(SUM, "\"5\""), u).is_err(), "size must be a number");
        assert!(parse_manifest(&ok(SUM, "-1"), u).is_err(), "negative size");
        assert!(parse_manifest(&format!("{{\"sha256\":\"{SUM}\",\"size\":1}}"), u).is_err(), "no version");
        assert!(parse_manifest("not json", u).is_err());
        assert!(parse_manifest("", u).is_err());
        // upper-case hashes are normalised so they compare equal to hex()
        let m = parse_manifest(&ok(&SUM.to_uppercase(), "5"), u).unwrap();
        assert_eq!(m.sha256, SUM);
        // a bare numeric version is kept as written
        let n = parse_manifest(&format!("{{\"version\":2,\"sha256\":\"{SUM}\",\"size\":1}}"), u).unwrap();
        assert_eq!(n.version, "2");
    }

    #[test]
    fn relative_urls_resolve_against_the_manifest_directory() {
        assert_eq!(resolve("https://h/a/b/latest.json", "springboard"), "https://h/a/b/springboard");
        assert_eq!(resolve("https://h/a/b/latest.json", "/springboard"), "https://h/a/b/springboard");
        assert_eq!(resolve("https://h/a/b/latest.json", "http://o/x"), "http://o/x");
    }

    #[test]
    fn staging_is_idempotent_and_replaces_an_older_staged_binary() {
        let dir = tempfile::tempdir().unwrap();
        let p = AtomicU64::new(0);
        let m = mf("2", None);
        std::fs::write(dir.path().join("springboard.new"), b"old").unwrap();
        stage(dir.path(), &m, &p, |sink| sink(b"abc")).unwrap();
        assert_eq!(std::fs::read(dir.path().join("springboard.new")).unwrap(), b"abc");
        stage(dir.path(), &m, &p, |sink| sink(b"abc")).unwrap();
        assert_eq!(std::fs::read(dir.path().join("springboard.new")).unwrap(), b"abc");
        assert!(!dir.path().join("springboard.part").exists());
    }

    #[test]
    fn a_short_download_or_a_failing_fetch_keeps_the_previous_staged_binary() {
        let dir = tempfile::tempdir().unwrap();
        let p = AtomicU64::new(0);
        let m = mf("2", None);
        std::fs::write(dir.path().join("springboard.new"), b"good").unwrap();
        assert!(stage(dir.path(), &m, &p, |sink| sink(b"ab")).is_err(), "short body");
        assert!(stage(dir.path(), &m, &p, |sink| {
            sink(b"abc")?;
            Err(io::Error::other("connection reset"))
        })
        .is_err());
        assert_eq!(std::fs::read(dir.path().join("springboard.new")).unwrap(), b"good");
        assert!(!dir.path().join("springboard.part").exists());
    }

    #[test]
    fn a_binary_staged_earlier_starts_as_staged_otherwise_idle() {
        let dir = tempfile::tempdir().unwrap();
        let u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        assert_eq!(u.state, State::Idle);
        assert_eq!(u.url(), DEFAULT_URL);
        std::fs::write(dir.path().join("springboard.new"), b"x").unwrap();
        let u = Updater::with_source(dir.path().to_path_buf(), Some(""), Source::Pacman);
        assert!(matches!(u.state, State::Staged(_)));
        assert_eq!(u.url(), DEFAULT_URL, "an empty override means the default");
        let c = Updater::with_source(dir.path().to_path_buf(), Some("https://c/x.json"), Source::Pacman);
        assert_eq!(c.url(), "https://c/x.json");
    }

    fn updater_with(msg: Msg) -> Updater {
        let dir = tempfile::tempdir().unwrap();
        let mut u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        let (tx, rx) = channel();
        tx.send(msg).unwrap();
        u.rx = Some(rx);
        u
    }

    #[test]
    fn poll_maps_worker_results_to_states() {
        let mut u = updater_with(Msg::Checked(Ok(mf("0.2.0", None))));
        assert!(u.poll("0.1.0", "g") && matches!(u.state, State::Available(_)) && !u.busy());
        let mut u = updater_with(Msg::Checked(Ok(mf("0.1.0", None))));
        assert!(u.poll("0.1.0", "g") && u.state == State::UpToDate, "equal version is up to date");
        let mut u = updater_with(Msg::Checked(Ok(mf("0.0.9", None))));
        assert!(u.poll("0.1.0", "g") && u.state == State::UpToDate, "older is up to date");
        let mut u = updater_with(Msg::Checked(Err("offline".into())));
        assert!(u.poll("0.1.0", "g") && u.state == State::Failed("offline".into()));
        let mut u = updater_with(Msg::Downloaded(mf("0.2.0", None), Ok(())));
        assert!(u.poll("0.1.0", "g") && matches!(u.state, State::Staged(_)));
        let mut u = updater_with(Msg::Downloaded(mf("0.2.0", None), Err("checksum mismatch".into())));
        assert!(u.poll("0.1.0", "g") && matches!(u.state, State::Failed(_)));
    }

    #[test]
    fn poll_is_quiet_without_a_result_and_reports_a_dead_worker() {
        let dir = tempfile::tempdir().unwrap();
        let mut u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        assert!(!u.poll("0.1.0", "g"), "no worker");
        let (tx, rx) = channel::<Msg>();
        u.rx = Some(rx);
        assert!(!u.poll("0.1.0", "g") && u.busy(), "worker still running");
        drop(tx);
        assert!(u.poll("0.1.0", "g") && matches!(u.state, State::Failed(_)) && !u.busy());
    }

    #[test]
    fn set_available_never_overrides_staged_downloading_or_a_running_worker() {
        let dir = tempfile::tempdir().unwrap();
        let mut u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        u.set_available(mf("1.0.0", None));
        assert!(matches!(u.state, State::Available(_)));
        u.state = State::Staged(mf("1.0.0", None));
        u.set_available(mf("2.0.0", None));
        assert!(matches!(&u.state, State::Staged(m) if m.version == "1.0.0"));
        u.state = State::Downloading;
        u.set_available(mf("2.0.0", None));
        assert_eq!(u.state, State::Downloading);
        u.state = State::Idle;
        let (_tx, rx) = channel::<Msg>();
        u.rx = Some(rx);
        u.set_available(mf("2.0.0", None));
        assert_eq!(u.state, State::Idle, "busy");
    }

    #[test]
    fn fraction_is_clamped_and_mirror_copies_state_unless_busy() {
        let dir = tempfile::tempdir().unwrap();
        let mut u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        assert_eq!(u.fraction(), 0.0, "unknown total");
        u.total = 10;
        u.done.store(5, Ordering::Relaxed);
        assert_eq!(u.fraction(), 0.5);
        u.done.store(50, Ordering::Relaxed);
        assert_eq!(u.fraction(), 1.0);
        let mut other = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        other.state = State::UpToDate;
        u.mirror(&other);
        assert_eq!(u.state, State::UpToDate);
        let (_tx, rx) = channel::<Msg>();
        u.rx = Some(rx);
        other.state = State::Idle;
        u.mirror(&other);
        assert_eq!(u.state, State::UpToDate, "a busy updater keeps its own state");
    }

    #[test]
    fn start_download_needs_an_available_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut u = Updater::with_source(dir.path().to_path_buf(), None, Source::Pacman);
        u.start_download();
        assert_eq!(u.state, State::Idle);
        assert!(!u.busy());
    }
}
