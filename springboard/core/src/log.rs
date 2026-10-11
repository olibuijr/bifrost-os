//! Unified Logging (docs/milestone-unified-logging.md): `Logger`, the subsystem registry and `LogStore`.
//!
//! Lives in `bifrost-core` so the shell (which cannot depend on the SDK) and apps share one implementation;
//! `bifrost_app::log` re-exports it. Entries go to the journald native socket with the fields
//! `BIFROST_SUBSYSTEM`, `BIFROST_CATEGORY`, `PRIORITY` and `SYSLOG_IDENTIFIER`; stderr is the fallback.
//! No new daemon: journald is the store (logd).

use crate::json::{self, Json};
use std::collections::VecDeque;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const REGISTRY_PATH: &str = "/var/lib/bifrost/log/registry.toml";
pub const OVERRIDES_PATH: &str = "/etc/bifrost/log.toml";
pub const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";
/// Entitlement a `system-app` needs in its manifest for `LogStore::local()`.
pub const ENTITLEMENT: &str = "log-store";
pub const SHELL_SUBSYSTEM: &str = "is.olibuijr.springboard";
pub const SHELL_CATEGORIES: &[&str] = &["ota", "store", "apphost", "presenter"];

fn registry_path() -> String {
    std::env::var("BIFROST_LOG_REGISTRY").unwrap_or_else(|_| REGISTRY_PATH.into())
}
fn overrides_path() -> String {
    std::env::var("BIFROST_LOG_CONFIG").unwrap_or_else(|_| OVERRIDES_PATH.into())
}

// ---- levels ----

/// `OSLogType`: debug, info, notice (default), error, fault. Ordered by severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Notice = 2,
    Error = 3,
    Fault = 4,
}

impl Level {
    /// syslog priority: debug 7, info 6, notice 5, error 3, fault 2.
    pub fn priority(self) -> u8 {
        match self {
            Level::Debug => 7,
            Level::Info => 6,
            Level::Notice => 5,
            Level::Error => 3,
            Level::Fault => 2,
        }
    }
    pub fn from_priority(p: u8) -> Level {
        match p {
            0..=2 => Level::Fault,
            3 => Level::Error,
            4 | 5 => Level::Notice,
            6 => Level::Info,
            _ => Level::Debug,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Notice => "notice",
            Level::Error => "error",
            Level::Fault => "fault",
        }
    }
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "debug" => Level::Debug,
            "info" => Level::Info,
            "notice" | "default" => Level::Notice,
            "error" => Level::Error,
            "fault" => Level::Fault,
            _ => return None,
        })
    }
    fn from_u8(n: u8) -> Level {
        match n {
            0 => Level::Debug,
            1 => Level::Info,
            2 => Level::Notice,
            3 => Level::Error,
            _ => Level::Fault,
        }
    }
}

// ---- privacy ----

const PRIV_START: char = '\u{E000}';
const PRIV_END: char = '\u{E001}';

/// A value marked `<private>` (default for dynamic values). Format it into the message:
/// `log.info(format!("opened {}", private(path)))`. The subsystem's `private` mode decides whether it is stored.
pub fn private(v: impl std::fmt::Display) -> String {
    format!("{PRIV_START}{v}{PRIV_END}")
}

/// Replaces marked values with `<private>`, or keeps the value when `show` is set. Markers never survive.
pub fn redact(msg: &str, show: bool) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut depth = 0u32;
    for c in msg.chars() {
        match c {
            PRIV_START => {
                if depth == 0 && !show {
                    out.push_str("<private>");
                }
                depth += 1;
            }
            PRIV_END => depth = depth.saturating_sub(1),
            c if depth == 0 || show => out.push(c),
            _ => {}
        }
    }
    out
}

// ---- journal wire format ----

/// systemd native journal protocol: `KEY=value\n`; a value containing a newline uses
/// `KEY\n` + u64 little-endian length + value + `\n`.
pub fn encode_datagram(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in fields {
        out.extend_from_slice(k.as_bytes());
        if v.contains('\n') {
            out.push(b'\n');
            out.extend_from_slice(&(v.len() as u64).to_le_bytes());
            out.extend_from_slice(v.as_bytes());
        } else {
            out.push(b'=');
            out.extend_from_slice(v.as_bytes());
        }
        out.push(b'\n');
    }
    out
}

// ---- registry ----

/// One subsystem: declared in a manifest (`[[log.subsystem]]`), stored in the registry, adjusted by overrides.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Spec {
    pub id: String,
    pub categories: Vec<String>,
    /// `None` in an override means "leave the registry level".
    pub level: Option<Level>,
    pub private: Option<bool>,
    /// The app id whose manifest declared it (registry only); only that app's processes may log as it.
    pub owner: Option<String>,
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix('"').and_then(|s| s.rsplit_once('"').map(|p| p.0)).unwrap_or(v).to_string()
}
fn list(v: &str) -> Vec<String> {
    v.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(unquote).filter(|s| !s.is_empty()).collect()
}

/// Parses `[[header]]` tables of the small TOML subset manifests use. Other tables are skipped.
pub fn parse_specs(text: &str, header: &str) -> Vec<Spec> {
    let want = format!("[[{header}]]");
    let mut out: Vec<Spec> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            inside = line == want;
            if inside {
                out.push(Spec::default());
            }
            continue;
        }
        if !inside {
            continue;
        }
        let (Some(s), Some((k, v))) = (out.last_mut(), line.split_once('=')) else { continue };
        match k.trim() {
            "id" => s.id = unquote(v),
            "categories" => s.categories = list(v),
            "level" => s.level = Level::parse(&unquote(v)),
            "private" => s.private = Some(v.trim() == "true"),
            "owner" => s.owner = Some(unquote(v)),
            _ => {}
        }
    }
    out.retain(|s| valid_subsystem(&s.id));
    out
}

pub fn valid_subsystem(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The registry from installed manifests: one entry per id (categories merged, first level wins), sorted by id.
/// The default level is `notice`.
pub fn registry_from_manifests(texts: &[&str]) -> Vec<Spec> {
    let mut all: Vec<Spec> = Vec::new();
    for t in texts {
        // the package id: the first top-level `id =`, before any table
        let owner = t.lines().map(str::trim).take_while(|l| !l.starts_with('[')).find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == "id").map(|(_, v)| unquote(v)));
        for s in parse_specs(t, "log.subsystem") {
            let s = Spec { owner: owner.clone(), ..s };
            match all.iter_mut().find(|x| x.id == s.id) {
                // a second package cannot take over a subsystem another one declared
                Some(x) if x.owner != s.owner => {}
                Some(x) => {
                    for c in s.categories {
                        if !x.categories.contains(&c) {
                            x.categories.push(c);
                        }
                    }
                }
                None => all.push(Spec { level: Some(s.level.unwrap_or(Level::Notice)), private: Some(s.private.unwrap_or(false)), ..s }),
            }
        }
    }
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

/// Overrides win for level and `private`; categories are unioned; unknown subsystems are added.
pub fn merge(registry: &[Spec], overrides: &[Spec]) -> Vec<Spec> {
    let mut out = registry.to_vec();
    for o in overrides {
        match out.iter_mut().find(|x| x.id == o.id) {
            Some(x) => {
                x.level = o.level.or(x.level);
                x.private = o.private.or(x.private);
                for c in &o.categories {
                    if !x.categories.contains(c) {
                        x.categories.push(c.clone());
                    }
                }
            }
            None => out.push(o.clone()),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Sets (or adds) one override entry; used by `bifrost-log config`.
pub fn set_override(overrides: &mut Vec<Spec>, id: &str, level: Option<Level>, private: Option<bool>) {
    match overrides.iter_mut().find(|x| x.id == id) {
        Some(x) => {
            x.level = level.or(x.level);
            x.private = private.or(x.private);
        }
        None => overrides.push(Spec { id: id.into(), categories: vec![], level, private, owner: None }),
    }
}

/// `registry.toml` / `log.toml` text. Unset fields are omitted.
pub fn render_specs(specs: &[Spec]) -> String {
    let mut s = String::from("# generated by BifrOSt; see docs/bifrost-ui.md (Unified Logging)\n");
    for x in specs {
        s.push_str(&format!("\n[[subsystem]]\nid = \"{}\"\n", x.id));
        if !x.categories.is_empty() {
            let c: Vec<String> = x.categories.iter().map(|c| format!("\"{c}\"")).collect();
            s.push_str(&format!("categories = [{}]\n", c.join(", ")));
        }
        if let Some(l) = x.level {
            s.push_str(&format!("level = \"{}\"\n", l.name()));
        }
        if let Some(p) = x.private {
            s.push_str(&format!("private = {p}\n"));
        }
        if let Some(o) = &x.owner {
            s.push_str(&format!("owner = \"{o}\"\n"));
        }
    }
    s
}

/// Effective (level, private) for a subsystem; unknown subsystems log at `notice`, private values redacted.
pub fn effective(specs: &[Spec], subsystem: &str) -> (Level, bool) {
    specs.iter().find(|s| s.id == subsystem).map(|s| (s.level.unwrap_or(Level::Notice), s.private.unwrap_or(false))).unwrap_or((Level::Notice, false))
}

pub fn read_registry() -> Vec<Spec> {
    std::fs::read_to_string(registry_path()).map(|t| parse_specs(&t, "subsystem")).unwrap_or_default()
}
pub fn read_overrides() -> Vec<Spec> {
    std::fs::read_to_string(overrides_path()).map(|t| parse_specs(&t, "subsystem")).unwrap_or_default()
}
pub fn read_merged() -> Vec<Spec> {
    merge(&read_registry(), &read_overrides())
}
pub fn write_overrides(specs: &[Spec]) -> io::Result<()> {
    let p = overrides_path();
    if let Some(d) = std::path::Path::new(&p).parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, render_specs(specs))
}

/// Rebuilds `registry.toml` from the `manifest.toml` text of every installed package, plus the built-in
/// shell subsystem. Called by the installer; the caller ignores failures (logging must never block installs).
pub fn write_registry(manifest_texts: &[&str]) -> io::Result<()> {
    let builtin = Spec {
        id: SHELL_SUBSYSTEM.into(),
        categories: SHELL_CATEGORIES.iter().map(|s| s.to_string()).collect(),
        level: Some(Level::Notice),
        private: Some(false),
        owner: None,
    };
    let specs = merge(&[builtin], &registry_from_manifests(manifest_texts));
    let p = registry_path();
    if let Some(d) = std::path::Path::new(&p).parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = format!("{p}.new");
    std::fs::write(&tmp, render_specs(&specs))?;
    std::fs::rename(tmp, p)
}

// ---- entries ----

/// `OSLogEntryLog`.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub date: SystemTime,
    pub process: String,
    pub pid: u32,
    pub subsystem: String,
    pub category: String,
    pub level: Level,
    pub message: String,
}

const RING_CAP: usize = 2000;
static RING: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());

fn ring_push(e: LogEntry) {
    let mut r = RING.lock().unwrap_or_else(|e| e.into_inner());
    if r.len() >= RING_CAP {
        r.pop_front();
    }
    r.push_back(e);
}

// ---- logger ----

fn start() -> &'static Instant {
    static S: OnceLock<Instant> = OnceLock::new();
    S.get_or_init(Instant::now)
}

fn sock() -> &'static Option<std::os::unix::net::UnixDatagram> {
    static S: OnceLock<Option<std::os::unix::net::UnixDatagram>> = OnceLock::new();
    S.get_or_init(|| std::os::unix::net::UnixDatagram::unbound().ok())
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// `Logger(subsystem:category:)`. Cheap to create and to call: a disabled message costs one atomic compare, and the
/// registry and override files are re-checked by mtime at most once a second.
pub struct Logger {
    subsystem: String,
    category: String,
    ident: String,
    level: AtomicU8,
    private: AtomicBool,
    next_check: AtomicU64,
    mtimes: Mutex<[Option<SystemTime>; 2]>,
}

impl Logger {
    pub fn new(subsystem: &str, category: &str) -> Logger {
        let ident = std::env::var("BIFROST_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "unknown".into())
        });
        let l = Logger {
            subsystem: subsystem.into(),
            category: category.into(),
            ident,
            level: AtomicU8::new(Level::Notice as u8),
            private: AtomicBool::new(false),
            next_check: AtomicU64::new(0),
            mtimes: Mutex::new([None, None]),
        };
        l.reload();
        l
    }

    fn reload(&self) {
        let (lv, pr) = effective(&read_merged(), &self.subsystem);
        self.level.store(lv as u8, Relaxed);
        self.private.store(pr, Relaxed);
        *self.mtimes.lock().unwrap_or_else(|e| e.into_inner()) = [mtime(&registry_path()), mtime(&overrides_path())];
    }

    fn recheck(&self) {
        let now = start().elapsed().as_millis() as u64;
        if now < self.next_check.load(Relaxed) {
            return;
        }
        self.next_check.store(now + 1000, Relaxed);
        let cur = [mtime(&registry_path()), mtime(&overrides_path())];
        let changed = *self.mtimes.lock().unwrap_or_else(|e| e.into_inner()) != cur;
        if changed {
            self.reload();
        }
    }

    /// The effective level (registry plus overrides).
    pub fn level(&self) -> Level {
        self.recheck();
        Level::from_u8(self.level.load(Relaxed))
    }

    pub fn enabled(&self, level: Level) -> bool {
        self.recheck();
        level as u8 >= self.level.load(Relaxed)
    }

    pub fn debug(&self, m: impl AsRef<str>) {
        self.log(Level::Debug, m.as_ref())
    }
    pub fn info(&self, m: impl AsRef<str>) {
        self.log(Level::Info, m.as_ref())
    }
    pub fn notice(&self, m: impl AsRef<str>) {
        self.log(Level::Notice, m.as_ref())
    }
    pub fn error(&self, m: impl AsRef<str>) {
        self.log(Level::Error, m.as_ref())
    }
    pub fn fault(&self, m: impl AsRef<str>) {
        self.log(Level::Fault, m.as_ref())
    }

    pub fn log(&self, level: Level, msg: &str) {
        if !self.enabled(level) {
            return;
        }
        let mut text = redact(msg, self.private.load(Relaxed));
        if text.len() > 16 * 1024 {
            let mut n = 16 * 1024;
            while !text.is_char_boundary(n) {
                n -= 1;
            }
            text.truncate(n);
        }
        ring_push(LogEntry {
            date: SystemTime::now(),
            process: self.ident.clone(),
            pid: std::process::id(),
            subsystem: self.subsystem.clone(),
            category: self.category.clone(),
            level,
            message: text.clone(),
        });
        let prio = level.priority().to_string();
        let data = encode_datagram(&[
            ("MESSAGE", &text),
            ("PRIORITY", &prio),
            ("SYSLOG_IDENTIFIER", &self.ident),
            ("BIFROST_SUBSYSTEM", &self.subsystem),
            ("BIFROST_CATEGORY", &self.category),
        ]);
        let path = std::env::var("BIFROST_JOURNAL_SOCKET").unwrap_or_else(|_| JOURNAL_SOCKET.into());
        let sent = sock().as_ref().is_some_and(|s| s.send_to(&data, path).is_ok());
        if !sent {
            eprintln!("{}: [{}:{}] {} {}", self.ident, self.subsystem, self.category, level.name(), text);
        }
    }
}

/// The shell's loggers (`is.olibuijr.springboard`, categories `ota`, `store`, `apphost`, `presenter`), created once.
pub fn shell_logger(category: &'static str) -> &'static Logger {
    static LOGGERS: Mutex<Vec<(&'static str, &'static Logger)>> = Mutex::new(Vec::new());
    let mut all = LOGGERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, l)) = all.iter().find(|(c, _)| *c == category) {
        return l;
    }
    let l: &'static Logger = Box::leak(Box::new(Logger::new(SHELL_SUBSYSTEM, category)));
    all.push((category, l));
    l
}

// ---- queries ----

/// A predicate subset of `log show --predicate`: subsystem, category, process, level, message contains, last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub subsystem: Option<String>,
    pub category: Option<String>,
    pub process: Option<String>,
    /// `level >= X`
    pub min_level: Option<Level>,
    /// `level == X`
    pub exact_level: Option<Level>,
    /// case-insensitive substring of the message
    pub contains: Option<String>,
    /// `--last 1h`
    pub last: Option<Duration>,
    /// maximum entries returned by `entries` (default 1000, newest kept)
    pub limit: Option<usize>,
}

pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok()?;
    let mul = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return None,
    };
    Some(Duration::from_secs(n * mul))
}

fn find_ci(h: &str, needle: &str) -> Option<usize> {
    h.to_ascii_uppercase().find(&needle.to_ascii_uppercase())
}
fn unquote_any(v: &str) -> String {
    let v = v.trim();
    for q in ['"', '\''] {
        if let Some(s) = v.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return s.to_string();
        }
    }
    v.to_string()
}

impl Query {
    /// `subsystem == "x" AND category == "y" AND level >= error AND message CONTAINS "foo"`.
    pub fn parse_predicate(p: &str) -> Result<Query, String> {
        let mut q = Query::default();
        let mut rest = p.trim();
        while !rest.is_empty() {
            let (clause, tail) = match find_ci(rest, " AND ") {
                Some(i) => (&rest[..i], &rest[i + 5..]),
                None => (rest, ""),
            };
            rest = tail.trim();
            let clause = clause.trim();
            let (field, op, value) = if let Some(i) = find_ci(clause, " CONTAINS ") {
                (&clause[..i], "contains", &clause[i + 10..])
            } else if let Some(i) = clause.find(">=") {
                (&clause[..i], ">=", &clause[i + 2..])
            } else if let Some(i) = clause.find("==") {
                (&clause[..i], "==", &clause[i + 2..])
            } else {
                return Err(format!("cannot parse clause {clause:?}"));
            };
            let field = field.trim().to_ascii_lowercase();
            let value = unquote_any(value);
            let lvl = || Level::parse(&value).ok_or_else(|| format!("unknown level {value:?}"));
            match (field.as_str(), op) {
                ("subsystem", "==") => q.subsystem = Some(value),
                ("category", "==") => q.category = Some(value),
                ("process", "==") => q.process = Some(value),
                ("message" | "eventmessage", "contains") => q.contains = Some(value),
                ("level" | "type", ">=") => q.min_level = Some(lvl()?),
                ("level" | "type", "==") => q.exact_level = Some(lvl()?),
                _ => return Err(format!("unsupported predicate {field} {op}")),
            }
        }
        Ok(q)
    }

    pub fn matches(&self, e: &LogEntry) -> bool {
        self.subsystem.as_ref().is_none_or(|s| *s == e.subsystem)
            && self.category.as_ref().is_none_or(|s| *s == e.category)
            && self.process.as_ref().is_none_or(|s| *s == e.process)
            && self.min_level.is_none_or(|l| e.level >= l)
            && self.exact_level.is_none_or(|l| e.level == l)
            && self.contains.as_ref().is_none_or(|s| e.message.to_lowercase().contains(&s.to_lowercase()))
            && self.last.is_none_or(|d| e.date.elapsed().map(|a| a <= d).unwrap_or(true))
    }

    /// `journalctl` arguments that pre-filter in the journal; `matches` still runs on every entry.
    pub fn journal_args(&self, follow: bool) -> Vec<String> {
        let mut a: Vec<String> = ["-o", "json", "--no-pager", "-q"].iter().map(|s| s.to_string()).collect();
        if let Some(l) = self.min_level {
            a.push("-p".into());
            a.push(l.priority().to_string());
        }
        if let Some(d) = self.last {
            a.push(format!("--since=-{}s", d.as_secs()));
        }
        if follow {
            a.extend(["-f".into(), "-n".into(), "0".into()]);
        } else {
            a.push("-n".into());
            a.push(self.limit.unwrap_or(1000).to_string());
        }
        for (f, v) in [("BIFROST_SUBSYSTEM", &self.subsystem), ("BIFROST_CATEGORY", &self.category), ("SYSLOG_IDENTIFIER", &self.process)] {
            if let Some(v) = v {
                a.push(format!("{f}={v}"));
            }
        }
        a
    }
}

fn json_field(o: &Json, key: &str) -> Option<String> {
    match o.get(key)? {
        Json::Str(s) => Some(s.clone()),
        // journald emits non-UTF-8 or binary values as arrays of byte numbers
        Json::Arr(a) => Some(String::from_utf8_lossy(&a.iter().filter_map(|n| n.as_u64().map(|n| n as u8)).collect::<Vec<u8>>()).into_owned()),
        _ => None,
    }
}

/// The app id of a store app's executable (`<root>/apps/<id>/<version>/<binary>`), from journald's trusted `_EXE`.
pub fn app_of_exe(exe: &str) -> Option<&str> {
    let rest = exe.strip_prefix("/var/lib/springboard/apps/")?;
    rest.split_once('/').map(|(id, _)| id).filter(|id| !id.is_empty())
}

/// Whether a process may log as `subsystem`. System binaries may use any subsystem; a store app only the ones its
/// manifest declared (registry `owner`), or undeclared ones nobody owns. binds a subsystem to the process
/// that logs it; journald's `_EXE` is set by the kernel, so an app cannot fake it.
pub fn may_log_as(registry: &[Spec], exe: Option<&str>, subsystem: &str) -> bool {
    let Some(app) = exe.and_then(app_of_exe) else { return true };
    match registry.iter().find(|s| s.id == subsystem).and_then(|s| s.owner.as_deref()) {
        Some(owner) => owner == app,
        // the shell's own subsystem has no owner but is reserved for the system
        None => subsystem != SHELL_SUBSYSTEM,
    }
}

/// An entry that claims a subsystem its process may not use loses the subsystem and category, so it never shows
/// under (or is filtered as) another app's subsystem.
fn attributed(line: &str, registry: &[Spec]) -> Option<LogEntry> {
    let mut e = entry_from_json(line)?;
    let exe = json::parse(line).ok().and_then(|o| json_field(&o, "_EXE"));
    if !e.subsystem.is_empty() && !may_log_as(registry, exe.as_deref(), &e.subsystem) {
        e.message = format!("[claimed subsystem {}] {}", e.subsystem, e.message);
        e.subsystem.clear();
        e.category.clear();
    }
    Some(e)
}

/// One `journalctl -o json` line.
pub fn entry_from_json(line: &str) -> Option<LogEntry> {
    let o = json::parse(line).ok()?;
    let us: u64 = json_field(&o, "__REALTIME_TIMESTAMP")?.parse().ok()?;
    let prio: u8 = json_field(&o, "PRIORITY").and_then(|p| p.parse().ok()).unwrap_or(6);
    Some(LogEntry {
        date: UNIX_EPOCH + Duration::from_micros(us),
        process: json_field(&o, "SYSLOG_IDENTIFIER").or_else(|| json_field(&o, "_COMM")).unwrap_or_default(),
        pid: json_field(&o, "_PID").and_then(|p| p.parse().ok()).unwrap_or(0),
        subsystem: json_field(&o, "BIFROST_SUBSYSTEM").unwrap_or_default(),
        category: json_field(&o, "BIFROST_CATEGORY").unwrap_or_default(),
        level: Level::from_priority(prio),
        message: json_field(&o, "MESSAGE").unwrap_or_default(),
    })
}

// ---- store ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    CurrentProcess,
    Local,
}

/// `OSLogStore`. `current_process()` reads this process's own entries (kept in memory, works in the sandbox);
/// `local()` reads the system journal and is for `system-app` packages with the `log-store` entitlement.
pub struct LogStore {
    scope: Scope,
}

impl LogStore {
    pub fn current_process() -> LogStore {
        LogStore { scope: Scope::CurrentProcess }
    }

    /// `OSLogStore.local()`. The shell binds the journal directories into the sandbox and sets
    /// `BIFROST_ENTITLEMENTS=log-store` only for a `system-app` whose manifest lists the entitlement; without it this
    /// returns `PermissionDenied`. The mount is the real gate; the variable only gives a clear error.
    pub fn local() -> io::Result<LogStore> {
        let ok = std::env::var("BIFROST_ENTITLEMENTS").is_ok_and(|v| v.split(',').any(|e| e.trim() == ENTITLEMENT));
        if ok {
            Ok(LogStore { scope: Scope::Local })
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "reading the system log needs kind = \"system-app\" and entitlements = [\"log-store\"]"))
        }
    }

    /// For trusted, unsandboxed tools (`bifrost-log`, the shell). Apps use `local()`.
    pub fn system() -> LogStore {
        LogStore { scope: Scope::Local }
    }

    /// Oldest first, at most `q.limit` (default 1000) newest entries.
    pub fn entries(&self, q: &Query) -> io::Result<Vec<LogEntry>> {
        match self.scope {
            Scope::CurrentProcess => {
                let r = RING.lock().unwrap_or_else(|e| e.into_inner());
                let mut v: Vec<LogEntry> = r.iter().filter(|e| q.matches(e)).cloned().collect();
                let cap = q.limit.unwrap_or(1000);
                if v.len() > cap {
                    v.drain(..v.len() - cap);
                }
                Ok(v)
            }
            Scope::Local => {
                let reg = read_registry();
                let out = std::process::Command::new("journalctl").args(q.journal_args(false)).output()?;
                if !out.status.success() {
                    return Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()));
                }
                Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| attributed(l, &reg)).filter(|e| q.matches(e)).collect())
            }
        }
    }

    /// Blocks, calling `on_entry` for each new entry until it returns false. Run it on a thread.
    pub fn stream(&self, q: &Query, mut on_entry: impl FnMut(LogEntry) -> bool) -> io::Result<()> {
        if self.scope == Scope::CurrentProcess {
            let mut seen = RING.lock().unwrap_or_else(|e| e.into_inner()).back().map(|e| e.date);
            loop {
                let fresh: Vec<LogEntry> = RING.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| seen.is_none_or(|s| e.date > s)).cloned().collect();
                for e in fresh {
                    seen = Some(e.date);
                    if q.matches(&e) && !on_entry(e) {
                        return Ok(());
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let mut child = std::process::Command::new("journalctl").args(q.journal_args(true)).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()?;
        let out = child.stdout.take().ok_or_else(|| io::Error::other("no journalctl output"))?;
        let reg = read_registry();
        for line in io::BufReader::new(out).lines() {
            let Some(e) = attributed(&line?, &reg) else { continue };
            if q.matches(&e) && !on_entry(e) {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

/// `bifrost-log show` line: `2026-10-10 12:00:00.123 E  notes[42] (is.olibuijr.notes:sync) message` (UTC).
pub fn format_entry(e: &LogEntry) -> String {
    let d = e.date.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (secs, ms) = (d.as_secs(), d.subsec_millis());
    let rem = secs % 86400;
    // civil date from days since the epoch (Howard Hinnant)
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dd = doy - (153 * mp + 2) / 5 + 1;
    let mm = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = yoe + era * 400 + i64::from(mm <= 2);
    let t = match e.level {
        Level::Debug => 'D',
        Level::Info => 'I',
        Level::Notice => 'N',
        Level::Error => 'E',
        Level::Fault => 'F',
    };
    format!("{yy:04}-{mm:02}-{dd:02} {:02}:{:02}:{:02}.{ms:03} {t}  {}[{}] ({}:{}) {}", rem / 3600, rem % 3600 / 60, rem % 60, e.process, e.pid, e.subsystem, e.category, e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_order_and_priorities() {
        assert!(Level::Fault > Level::Error && Level::Error > Level::Notice && Level::Notice > Level::Info && Level::Info > Level::Debug);
        assert_eq!([Level::Debug, Level::Info, Level::Notice, Level::Error, Level::Fault].map(Level::priority), [7, 6, 5, 3, 2]);
        assert_eq!(Level::from_priority(4), Level::Notice);
        assert_eq!(Level::from_priority(0), Level::Fault);
        assert_eq!(Level::parse("Default"), Some(Level::Notice));
        assert_eq!(Level::parse("loud"), None);
    }

    #[test]
    fn private_values_are_redacted_unless_shown() {
        let m = format!("opened {} for {}", private("/home/a/secret.txt"), private(7));
        assert_eq!(redact(&m, false), "opened <private> for <private>");
        assert_eq!(redact(&m, true), "opened /home/a/secret.txt for 7");
        assert_eq!(redact("plain", false), "plain");
    }

    #[test]
    fn single_line_datagram() {
        assert_eq!(encode_datagram(&[("MESSAGE", "hi"), ("PRIORITY", "5")]), b"MESSAGE=hi\nPRIORITY=5\n");
    }

    #[test]
    fn multiline_value_uses_the_binary_length_form() {
        let d = encode_datagram(&[("MESSAGE", "a\nbc"), ("PRIORITY", "3")]);
        let mut want = b"MESSAGE\n".to_vec();
        want.extend_from_slice(&4u64.to_le_bytes());
        want.extend_from_slice(b"a\nbc\nPRIORITY=3\n");
        assert_eq!(d, want);
    }

    const MANIFEST: &str = "id = \"is.olibuijr.notes.desktop\"\nname = \"Notes\"\n\n[[intent]]\nid = \"new-note\"\n\n[[log.subsystem]]\nid = \"is.olibuijr.notes\"\ncategories = [\"sync\", \"save\"]\nlevel = \"info\"\n\n[[log.subsystem]]\nid = \"bad id!\"\n";

    #[test]
    fn manifest_subsystems_parse_and_skip_other_tables() {
        let s = parse_specs(MANIFEST, "log.subsystem");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].id, "is.olibuijr.notes");
        assert_eq!(s[0].categories, ["sync", "save"]);
        assert_eq!(s[0].level, Some(Level::Info));
    }

    #[test]
    fn a_subsystem_belongs_to_the_app_that_declared_it() {
        let notes = "id = \"is.o.notes.desktop\"\n[[log.subsystem]]\nid = \"is.o.notes\"\ncategories = [\"sync\"]\n";
        let thief = "id = \"is.o.evil\"\n[[log.subsystem]]\nid = \"is.o.notes\"\ncategories = [\"steal\"]\n";
        let reg = registry_from_manifests(&[notes, thief]);
        assert_eq!((reg[0].owner.as_deref(), reg[0].categories.clone()), (Some("is.o.notes.desktop"), vec!["sync".to_string()]));
        assert_eq!(parse_specs(&render_specs(&reg), "subsystem")[0].owner.as_deref(), Some("is.o.notes.desktop"), "the owner survives the registry file");
        let exe = |id: &str| format!("/var/lib/springboard/apps/{id}/1.0.0/bin");
        assert!(may_log_as(&reg, Some(&exe("is.o.notes.desktop")), "is.o.notes"));
        assert!(!may_log_as(&reg, Some(&exe("is.o.evil")), "is.o.notes"), "another app may not log as Notes");
        assert!(!may_log_as(&reg, Some(&exe("is.o.evil")), SHELL_SUBSYSTEM), "nor as the shell");
        assert!(may_log_as(&reg, Some("/usr/bin/springboard"), "is.o.notes") && may_log_as(&reg, None, SHELL_SUBSYSTEM));
        let line = r#"{"__REALTIME_TIMESTAMP":"1","_EXE":"/var/lib/springboard/apps/is.o.evil/1.0.0/bin","BIFROST_SUBSYSTEM":"is.o.notes","BIFROST_CATEGORY":"sync","MESSAGE":"hi"}"#;
        let e = attributed(line, &reg).unwrap();
        assert!(e.subsystem.is_empty() && e.category.is_empty() && e.message.starts_with("[claimed subsystem is.o.notes]"));
    }

    #[test]
    fn registry_merges_manifests_and_overrides() {
        let other = "id = \"is.olibuijr.notes.desktop\"\n[[log.subsystem]]\nid = \"is.olibuijr.notes\"\ncategories = [\"ui\", \"sync\"]\n[[log.subsystem]]\nid = \"is.olibuijr.mail\"\n";
        let reg = registry_from_manifests(&[MANIFEST, other]);
        assert_eq!(reg.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["is.olibuijr.mail", "is.olibuijr.notes"]);
        assert_eq!(reg[1].categories, ["sync", "save", "ui"]);
        assert_eq!(reg[1].level, Some(Level::Info));
        assert_eq!(reg[0].level, Some(Level::Notice));
        let mut ov = vec![];
        set_override(&mut ov, "is.olibuijr.notes", Some(Level::Debug), Some(true));
        set_override(&mut ov, "x.y", Some(Level::Error), None);
        let m = merge(&reg, &ov);
        assert_eq!(effective(&m, "is.olibuijr.notes"), (Level::Debug, true));
        assert_eq!(effective(&m, "x.y"), (Level::Error, false));
        assert_eq!(effective(&m, "is.olibuijr.mail"), (Level::Notice, false));
        assert_eq!(effective(&m, "nobody"), (Level::Notice, false));
        // round trip through the file format
        assert_eq!(parse_specs(&render_specs(&m), "subsystem"), m);
    }

    #[test]
    fn predicate_parsing() {
        let q = Query::parse_predicate("subsystem == \"is.olibuijr.notes\" AND category == 'sync' and level >= error AND message CONTAINS \"disk full\"").unwrap();
        assert_eq!(q.subsystem.as_deref(), Some("is.olibuijr.notes"));
        assert_eq!(q.category.as_deref(), Some("sync"));
        assert_eq!(q.min_level, Some(Level::Error));
        assert_eq!(q.contains.as_deref(), Some("disk full"));
        assert!(Query::parse_predicate("colour == red").is_err());
        assert!(Query::parse_predicate("level >= loud").is_err());
        assert_eq!(parse_duration("90m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("5"), None);
    }

    fn entry(level: Level, msg: &str) -> LogEntry {
        LogEntry { date: SystemTime::now(), process: "notes".into(), pid: 1, subsystem: "a.b".into(), category: "sync".into(), level, message: msg.into() }
    }

    #[test]
    fn query_matches_entries() {
        let q = Query { subsystem: Some("a.b".into()), min_level: Some(Level::Error), contains: Some("DISK".into()), ..Default::default() };
        assert!(q.matches(&entry(Level::Fault, "Disk full")));
        assert!(!q.matches(&entry(Level::Notice, "Disk full")));
        assert!(!q.matches(&entry(Level::Error, "ok")));
        let exact = Query { exact_level: Some(Level::Info), ..Default::default() };
        assert!(exact.matches(&entry(Level::Info, "")) && !exact.matches(&entry(Level::Error, "")));
    }

    #[test]
    fn journal_args_prefilter() {
        let q = Query { subsystem: Some("a.b".into()), min_level: Some(Level::Error), last: Some(Duration::from_secs(60)), ..Default::default() };
        let a = q.journal_args(false);
        assert!(a.windows(2).any(|w| w == ["-p", "3"]));
        assert!(a.contains(&"--since=-60s".to_string()) && a.contains(&"BIFROST_SUBSYSTEM=a.b".to_string()));
        assert!(q.journal_args(true).contains(&"-f".to_string()));
    }

    #[test]
    fn journal_json_line_becomes_an_entry() {
        let line = r#"{"__REALTIME_TIMESTAMP":"1760000000123456","PRIORITY":"3","_PID":"42","SYSLOG_IDENTIFIER":"notes","BIFROST_SUBSYSTEM":"is.olibuijr.notes","BIFROST_CATEGORY":"sync","MESSAGE":"failed"}"#;
        let e = entry_from_json(line).unwrap();
        assert_eq!((e.level, e.pid, e.process.as_str(), e.subsystem.as_str(), e.category.as_str(), e.message.as_str()), (Level::Error, 42, "notes", "is.olibuijr.notes", "sync", "failed"));
        assert_eq!(e.date, UNIX_EPOCH + Duration::from_micros(1760000000123456));
        // binary MESSAGE arrives as a byte array; entries from other services have no subsystem
        let e = entry_from_json(r#"{"__REALTIME_TIMESTAMP":"1","_COMM":"sshd","MESSAGE":[104,105]}"#).unwrap();
        assert_eq!((e.message.as_str(), e.process.as_str(), e.subsystem.as_str()), ("hi", "sshd", ""));
    }

    #[test]
    fn format_entry_prints_a_utc_date() {
        let mut e = entry(Level::Error, "x");
        e.date = UNIX_EPOCH + Duration::from_secs(86400 * 365);
        assert_eq!(format_entry(&e), "1971-01-01 00:00:00.000 E  notes[1] (a.b:sync) x");
    }
}
