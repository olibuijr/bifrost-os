//! Bug reporting: the report model, its limits, the diagnostics an app may attach, the redaction that runs before
//! anything is attached, and the client for the BifrOSt Cloud bug API (`docs/bug-reporting.md`).
//!
//! * [`BugReport`] is the body of `POST /bugs`. [`BugReport::validate`] enforces the server's limits first, so a
//!   report that cannot be accepted is refused on the device.
//! * [`Item`] is one diagnostic the person may attach. Every item starts unchecked ([`Item::consent`] is false);
//!   [`consented`] returns only the items the person checked, which is the only way items reach a report.
//! * The collectors ([`sysinfo_item`], [`log_item`], [`crash_item`], [`config_item`], [`screenshot_item`]) are pure over
//!   the data they are given. [`read_logs`], [`read_crash_text`] and [`SystemFacts::this_machine`] read the real system.
//! * [`redact`] strips tokens, passwords, email addresses and home-folder user names. Every text item passes through it.
//! * [`BugClient`] builds the three requests and reads their answers. Its calls are blocking and take a transport
//!   function (the URL session in [`session_send`], a canned answer in tests): run them on a background thread.
//!
//! Nothing here logs report text or diagnostics.

use crate::http_auth::base64_encode;
use crate::url::URLRequest;
use bifrost_core::json::{self, Json};
use bifrost_core::log::{format_entry, LogEntry, LogStore, Query};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The cloud API's base URL.
pub const BASE_URL: &str = "https://bifrost.olibuijr.com/api/cloud";
/// Title length in characters, 1 to this.
pub const MAX_TITLE: usize = 200;
/// Description length in characters, 1 to this.
pub const MAX_DESCRIPTION: usize = 20_000;
/// Most diagnostics in one report.
pub const MAX_DIAGNOSTICS: usize = 20;
/// Most bytes in one diagnostic (decoded).
pub const MAX_DIAGNOSTIC_BYTES: usize = 10 << 20;
/// Most bytes in the request body.
pub const MAX_BODY_BYTES: usize = 25 << 20;
/// The text shown instead of a removed secret.
pub const REDACTED: &str = "[redacted]";

// ---- vocabulary ----

/// What kind of problem the report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Crash,
    Bug,
    Performance,
    Ui,
    Feature,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [Category::Crash, Category::Bug, Category::Performance, Category::Ui, Category::Feature, Category::Other];

    /// The wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Crash => "crash",
            Category::Bug => "bug",
            Category::Performance => "performance",
            Category::Ui => "ui",
            Category::Feature => "feature",
            Category::Other => "other",
        }
    }

    /// The name the form shows.
    pub fn label(self) -> &'static str {
        match self {
            Category::Crash => "App crashed or quit",
            Category::Bug => "Something does not work",
            Category::Performance => "Slow or uses too much power",
            Category::Ui => "Looks or feels wrong",
            Category::Feature => "Missing feature",
            Category::Other => "Something else",
        }
    }

    pub fn parse(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// How often the problem happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reproducible {
    Always,
    Sometimes,
    Once,
    Unknown,
}

impl Reproducible {
    pub const ALL: [Reproducible; 4] = [Reproducible::Always, Reproducible::Sometimes, Reproducible::Once, Reproducible::Unknown];

    pub fn as_str(self) -> &'static str {
        match self {
            Reproducible::Always => "always",
            Reproducible::Sometimes => "sometimes",
            Reproducible::Once => "once",
            Reproducible::Unknown => "unknown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Reproducible::Always => "Every time",
            Reproducible::Sometimes => "Sometimes",
            Reproducible::Once => "Happened once",
            Reproducible::Unknown => "Not sure",
        }
    }

    pub fn parse(s: &str) -> Option<Reproducible> {
        Reproducible::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// The kind of one diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sysinfo,
    Log,
    Crash,
    Screenshot,
    Config,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Sysinfo => "sysinfo",
            Kind::Log => "log",
            Kind::Crash => "crash",
            Kind::Screenshot => "screenshot",
            Kind::Config => "config",
            Kind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Sysinfo, Kind::Log, Kind::Crash, Kind::Screenshot, Kind::Config, Kind::Other].into_iter().find(|k| k.as_str() == s)
    }
}

/// Where a submitted report stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    New,
    Triaged,
    InProgress,
    Fixed,
    WontFix,
    Duplicate,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::New => "new",
            Status::Triaged => "triaged",
            Status::InProgress => "in-progress",
            Status::Fixed => "fixed",
            Status::WontFix => "wont-fix",
            Status::Duplicate => "duplicate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::New => "Received",
            Status::Triaged => "Reviewed",
            Status::InProgress => "In progress",
            Status::Fixed => "Fixed",
            Status::WontFix => "Will not fix",
            Status::Duplicate => "Duplicate",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        [Status::New, Status::Triaged, Status::InProgress, Status::Fixed, Status::WontFix, Status::Duplicate].into_iter().find(|v| v.as_str() == s)
    }
}

// ---- the report ----

/// The hardware block of a report. Every field is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub model: Option<String>,
    pub cpu: Option<String>,
    pub gpu: Option<String>,
    pub memory_mb: Option<u64>,
    pub fingerprint: Option<String>,
}

/// One attached diagnostic, as the server takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub kind: Kind,
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// The body of `POST /bugs`.
#[derive(Clone, Debug, PartialEq)]
pub struct BugReport {
    pub title: String,
    pub description: String,
    pub category: Category,
    pub reproducible: Reproducible,
    pub app_id: Option<String>,
    pub app_version: Option<String>,
    pub os_version: String,
    pub os_build: String,
    pub hardware: Hardware,
    pub diagnostics: Vec<Diagnostic>,
}

/// Why a report cannot be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    TitleEmpty,
    TitleTooLong,
    DescriptionEmpty,
    DescriptionTooLong,
    OsVersionMissing,
    TooManyDiagnostics,
    DiagnosticEmptyName,
    DiagnosticTooLarge(String),
    TooLarge,
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportError::TitleEmpty => f.write_str("Enter a title."),
            ReportError::TitleTooLong => write!(f, "The title can have at most {MAX_TITLE} characters."),
            ReportError::DescriptionEmpty => f.write_str("Describe the problem."),
            ReportError::DescriptionTooLong => write!(f, "The description can have at most {MAX_DESCRIPTION} characters."),
            ReportError::OsVersionMissing => f.write_str("The system version could not be read."),
            ReportError::TooManyDiagnostics => write!(f, "A report can have at most {MAX_DIAGNOSTICS} diagnostics."),
            ReportError::DiagnosticEmptyName => f.write_str("A diagnostic has no name."),
            ReportError::DiagnosticTooLarge(name) => write!(f, "\"{name}\" is larger than {} MB.", MAX_DIAGNOSTIC_BYTES >> 20),
            ReportError::TooLarge => write!(f, "The report is larger than {} MB. Uncheck some diagnostics.", MAX_BODY_BYTES >> 20),
        }
    }
}

impl BugReport {
    /// Checks the server's limits. The first problem found is returned.
    pub fn validate(&self) -> Result<(), ReportError> {
        let title = self.title.trim().chars().count();
        if title == 0 {
            return Err(ReportError::TitleEmpty);
        }
        if title > MAX_TITLE {
            return Err(ReportError::TitleTooLong);
        }
        let description = self.description.trim().chars().count();
        if description == 0 {
            return Err(ReportError::DescriptionEmpty);
        }
        if description > MAX_DESCRIPTION {
            return Err(ReportError::DescriptionTooLong);
        }
        if self.os_version.trim().is_empty() {
            return Err(ReportError::OsVersionMissing);
        }
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            return Err(ReportError::TooManyDiagnostics);
        }
        for d in &self.diagnostics {
            if d.name.trim().is_empty() {
                return Err(ReportError::DiagnosticEmptyName);
            }
            if d.data.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(ReportError::DiagnosticTooLarge(d.name.clone()));
            }
        }
        // the body is base64 (4 bytes for 3) plus the text fields: bound it before building it
        let payload: usize = self.diagnostics.iter().map(|d| d.data.len().div_ceil(3) * 4 + d.name.len() + d.mime.len() + 64).sum();
        if payload + self.title.len() + self.description.len() > MAX_BODY_BYTES {
            return Err(ReportError::TooLarge);
        }
        Ok(())
    }

    /// The JSON body. Call [`BugReport::validate`] first.
    pub fn to_json(&self) -> String {
        let text = |s: &str| Json::Str(s.to_string());
        let opt = |s: &Option<String>| s.as_deref().map_or(Json::Null, |v| Json::Str(v.to_string()));
        let mut hw = Vec::new();
        for (key, value) in [("model", &self.hardware.model), ("cpu", &self.hardware.cpu), ("gpu", &self.hardware.gpu), ("fingerprint", &self.hardware.fingerprint)] {
            if let Some(v) = value {
                hw.push((key.to_string(), Json::Str(v.clone())));
            }
        }
        if let Some(mb) = self.hardware.memory_mb {
            hw.push(("memory_mb".to_string(), Json::Num(mb as f64)));
        }
        let diagnostics = self
            .diagnostics
            .iter()
            .map(|d| {
                Json::Obj(vec![
                    ("kind".into(), text(d.kind.as_str())),
                    ("name".into(), text(&d.name)),
                    ("mime".into(), text(&d.mime)),
                    ("data_b64".into(), Json::Str(base64_encode(&d.data))),
                ])
            })
            .collect();
        let mut fields = vec![
            ("title".to_string(), text(self.title.trim())),
            ("description".to_string(), text(self.description.trim())),
            ("category".to_string(), text(self.category.as_str())),
            ("reproducible".to_string(), text(self.reproducible.as_str())),
        ];
        if self.app_id.is_some() {
            fields.push(("app_id".into(), opt(&self.app_id)));
        }
        if self.app_version.is_some() {
            fields.push(("app_version".into(), opt(&self.app_version)));
        }
        fields.push(("os_version".into(), text(&self.os_version)));
        fields.push(("os_build".into(), text(&self.os_build)));
        fields.push(("hardware".into(), Json::Obj(hw)));
        fields.push(("diagnostics".into(), Json::Arr(diagnostics)));
        Json::Obj(fields).encode()
    }
}

// ---- redaction ----

/// Words whose value (after `:` or `=`) is removed.
const SECRET_KEYS: [&str; 10] = ["password", "passwd", "passphrase", "secret", "token", "api_key", "apikey", "cookie", "bc_session", "session_id"];

/// Whether a settings key names a secret (`wifi_password`, `api_token`). Such keys are not attached at all.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|s| k.contains(s)) || k.contains("credential") || k.contains("private_key")
}

fn is_email_char(c: char, domain: bool) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-') || (!domain && matches!(c, '_' | '%' | '+'))
}

fn starts_with_ci(chars: &[char], at: usize, word: &str) -> bool {
    let w: Vec<char> = word.chars().collect();
    at + w.len() <= chars.len() && chars[at..at + w.len()].iter().zip(&w).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn is_value_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '&' | ';' | '}' | ')' | ']')
}

/// Removes `Bearer`/`Basic` credentials and `key=value`, `key: value` and `"key":"value"` secrets.
fn redact_secrets(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    'scan: while i < chars.len() {
        let at_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
        if at_boundary {
            for scheme in ["Bearer ", "Basic "] {
                if starts_with_ci(&chars, i, scheme) {
                    let start = i + scheme.chars().count();
                    let end = (start..chars.len()).find(|&j| chars[j].is_whitespace()).unwrap_or(chars.len());
                    if end > start {
                        out.extend(&chars[i..start]);
                        out.push_str(REDACTED);
                        i = end;
                        continue 'scan;
                    }
                }
            }
            for key in SECRET_KEYS {
                if !starts_with_ci(&chars, i, key) {
                    continue;
                }
                let mut j = i + key.chars().count();
                if j < chars.len() && matches!(chars[j], '"' | '\'') {
                    j += 1;
                }
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                if j >= chars.len() || !matches!(chars[j], ':' | '=') {
                    continue;
                }
                j += 1;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                if j < chars.len() && matches!(chars[j], '"' | '\'') {
                    j += 1;
                }
                let end = (j..chars.len()).find(|&k| is_value_end(chars[k])).unwrap_or(chars.len());
                if end > j {
                    out.extend(&chars[i..j]);
                    out.push_str(REDACTED);
                    i = end;
                    continue 'scan;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replaces email addresses with `[email]`.
fn redact_emails(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut left = out.chars().rev().take_while(|c| is_email_char(*c, false)).count();
            let right = chars[i + 1..].iter().take_while(|c| is_email_char(**c, true)).count();
            let domain: String = chars[i + 1..i + 1 + right].iter().collect();
            let domain = domain.trim_end_matches('.');
            if left > 0 && domain.contains('.') && !domain.starts_with('.') {
                while left > 0 {
                    out.pop();
                    left -= 1;
                }
                out.push_str("[email]");
                i += 1 + domain.chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replaces `/home/<name>` and `/Users/<name>` with `<name>` shown as `[user]`, and `user` itself wherever it stands
/// as a whole word.
fn redact_user(input: &str, user: Option<&str>) -> String {
    let mut text = input.to_string();
    for root in ["/home/", "/Users/"] {
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find(root) {
            out.push_str(&rest[..at + root.len()]);
            rest = &rest[at + root.len()..];
            let name_len = rest.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))).unwrap_or(rest.len());
            if name_len > 0 {
                out.push_str("[user]");
            }
            rest = &rest[name_len..];
        }
        out.push_str(rest);
        text = out;
    }
    if let Some(user) = user.filter(|u| u.chars().count() >= 3) {
        let chars: Vec<char> = text.chars().collect();
        let name: Vec<char> = user.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let whole = i + name.len() <= chars.len()
                && chars[i..i + name.len()] == name[..]
                && (i == 0 || !chars[i - 1].is_alphanumeric())
                && chars.get(i + name.len()).is_none_or(|c| !c.is_alphanumeric());
            if whole {
                out.push_str("[user]");
                i += name.len();
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        text = out;
    }
    text
}

/// Runs of 32 or more letters, digits, `-` and `_` that contain a digit: keys, hashes and session tokens.
fn redact_long_tokens(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphanumeric() {
            let end = (i..chars.len()).find(|&j| !(chars[j].is_ascii_alphanumeric() || matches!(chars[j], '-' | '_'))).unwrap_or(chars.len());
            if end - i >= 32 && chars[i..end].iter().any(|c| c.is_ascii_digit()) {
                out.push_str(REDACTED);
            } else {
                out.extend(&chars[i..end]);
            }
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Strips tokens, passwords, email addresses and home-folder user names from `text`. `user` is the signed-in user's
/// login name, removed wherever it appears as a word.
pub fn redact(text: &str, user: Option<&str>) -> String {
    redact_long_tokens(&redact_user(&redact_emails(&redact_secrets(text)), user))
}

// ---- diagnostics ----

/// One diagnostic the person may attach. It starts unchecked: nothing is sent unless `consent` is true.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// A stable id for the list row.
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
    pub consent: bool,
}

impl Item {
    pub fn new(id: &str, kind: Kind, name: &str, mime: &str, data: Vec<u8>) -> Item {
        Item { id: id.into(), kind, name: name.into(), mime: mime.into(), data, consent: false }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// "812 bytes", "3.4 KB", "1.2 MB".
    pub fn size_label(&self) -> String {
        size_label(self.data.len())
    }

    /// Whether the item is text the person can read before sending.
    pub fn is_text(&self) -> bool {
        self.mime.starts_with("text/") || self.mime == "application/json"
    }

    /// The text behind "Show": the first `max_chars` characters of a text item, or a line for binary data.
    pub fn preview(&self, max_chars: usize) -> String {
        if !self.is_text() {
            return format!("{} data, {}. It cannot be shown as text.", self.mime, self.size_label());
        }
        let text = String::from_utf8_lossy(&self.data);
        let mut shown: String = text.chars().take(max_chars).collect();
        if text.chars().count() > max_chars {
            shown.push_str("\n...");
        }
        shown
    }
}

/// "812 bytes", "3.4 KB", "1.2 MB".
pub fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1 << 20 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    }
}

/// The diagnostics the person checked, in list order. Unchecked and empty items never leave the device.
pub fn consented(items: &[Item]) -> Vec<Diagnostic> {
    items
        .iter()
        .filter(|i| i.consent && !i.data.is_empty())
        .map(|i| Diagnostic { kind: i.kind, name: i.name.clone(), mime: i.mime.clone(), data: i.data.clone() })
        .collect()
}

/// What the system facts and the hardware block are built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemFacts {
    pub os_name: String,
    pub os_version: String,
    /// The kernel release, which stands in for the build.
    pub os_build: String,
    pub model: String,
    pub cpu: String,
    pub gpus: Vec<String>,
    pub memory_bytes: u64,
}

impl SystemFacts {
    pub fn from_summary(s: &bifrost_core::sysinfo::Summary) -> SystemFacts {
        SystemFacts {
            os_name: s.os_name.clone(),
            os_version: s.os_version.clone(),
            os_build: s.kernel.clone(),
            model: s.model.clone(),
            cpu: s.processor.clone(),
            gpus: s.graphics.clone(),
            memory_bytes: s.memory_bytes,
        }
    }

    /// This machine, read once per process.
    pub fn this_machine() -> SystemFacts {
        SystemFacts::from_summary(bifrost_core::sysinfo::Summary::this_machine())
    }

    pub fn hardware(&self) -> Hardware {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        Hardware {
            model: some(&self.model),
            cpu: some(&self.cpu),
            gpu: some(&self.gpus.join(", ")),
            memory_mb: (self.memory_bytes > 0).then_some(self.memory_bytes >> 20),
            fingerprint: None,
        }
    }

    /// The text of the system information item.
    pub fn text(&self) -> String {
        let mut out = format!("System: {} {}\nBuild: {}\nModel: {}\nProcessor: {}\n", self.os_name, self.os_version, self.os_build, self.model, self.cpu);
        for g in &self.gpus {
            out.push_str(&format!("Graphics: {g}\n"));
        }
        out.push_str(&format!("Memory: {} MB\n", self.memory_bytes >> 20));
        out
    }
}

/// The system information item: operating system version and build, model, processor, graphics and memory.
pub fn sysinfo_item(facts: &SystemFacts) -> Item {
    Item::new("sysinfo", Kind::Sysinfo, "system-information.txt", "text/plain", facts.text().into_bytes())
}

/// Which log lines the item holds.
#[derive(Clone, Debug, PartialEq)]
pub struct LogFilter {
    /// Only lines of this app: its subsystem id, or its process name. None keeps every process.
    pub app: Option<String>,
    /// How far back to look.
    pub since: Duration,
    /// Most lines kept (the newest).
    pub limit: usize,
}

impl Default for LogFilter {
    fn default() -> Self {
        LogFilter { app: None, since: Duration::from_secs(15 * 60), limit: 500 }
    }
}

impl LogFilter {
    fn keeps(&self, e: &LogEntry, now: SystemTime) -> bool {
        let recent = now.duration_since(e.date).map_or(true, |age| age <= self.since);
        let app = self.app.as_deref().is_none_or(|a| e.subsystem == a || e.process == a);
        recent && app
    }
}

/// The log item: the recent lines the filter keeps, oldest first, redacted.
pub fn log_item(entries: &[LogEntry], filter: &LogFilter, now: SystemTime, user: Option<&str>) -> Item {
    let mut kept: Vec<&LogEntry> = entries.iter().filter(|e| filter.keeps(e, now)).collect();
    kept.sort_by_key(|e| e.date);
    if kept.len() > filter.limit {
        kept.drain(..kept.len() - filter.limit);
    }
    let text: String = kept.iter().map(|e| format!("{}\n", format_entry(e))).collect();
    Item::new("log", Kind::Log, "log.txt", "text/plain", redact(&text, user).into_bytes())
}

/// Reads the recent system log (needs the `log-store` entitlement in the manifest). Oldest first.
pub fn read_logs(filter: &LogFilter) -> std::io::Result<Vec<LogEntry>> {
    let query = Query { last: Some(filter.since), limit: Some(filter.limit.saturating_mul(4).max(1000)), ..Query::default() };
    LogStore::local()?.entries(&query)
}

/// The crash report item. `name_hint` is the app id or name, used in the file name.
pub fn crash_item(report: &str, name_hint: &str, user: Option<&str>) -> Item {
    let safe: String = name_hint.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' }).collect();
    Item::new("crash", Kind::Crash, &format!("crash-{safe}.txt"), "text/plain", redact(report, user).into_bytes())
}

/// The crash report text the shell saved for the last crash (`last_crash_report.txt` in its data directory), if it
/// is about `app_id`. `dirs` are the directories the shell may have written to.
pub fn read_crash_text(dirs: &[PathBuf], app_id: &str) -> Option<String> {
    dirs.iter().find_map(|d| std::fs::read_to_string(d.join("last_crash_report.txt")).ok()).filter(|t| t.contains(app_id))
}

/// The directories the shell keeps its data (and so its crash report) in: the system's, then the user's.
pub fn shell_data_dirs() -> Vec<PathBuf> {
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    let user = springboard::bare::platform::data_dir(false, xdg.as_deref(), home.as_deref());
    let system = springboard::bare::platform::data_dir(true, None, None);
    let mut dirs = vec![PathBuf::from(&system)];
    if user != system {
        dirs.push(PathBuf::from(user));
    }
    dirs
}

/// The configuration item: `key=value` lines, sorted. Keys that name a secret are left out and the values of the
/// others are redacted.
pub fn config_item(pairs: &[(String, String)], user: Option<&str>) -> Item {
    let mut lines: Vec<String> = pairs.iter().filter(|(k, _)| !is_secret_key(k)).map(|(k, v)| format!("{k}={}", redact(v, user))).collect();
    lines.sort();
    let mut text = lines.join("\n");
    text.push('\n');
    Item::new("config", Kind::Config, "settings.txt", "text/plain", text.into_bytes())
}

/// Parses a `key=value` settings file (the shell's `settings.ini`). Comments and blank lines are skipped.
pub fn parse_settings(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// The screenshot item, from PNG bytes the capture path produced.
pub fn screenshot_item(png: Vec<u8>) -> Item {
    Item::new("screenshot", Kind::Screenshot, "screenshot.png", "image/png", png)
}

// ---- cloud client ----

/// The BifrOSt Account session this device holds. The token is a device bearer token; it is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct CloudSession {
    token: String,
}

impl fmt::Debug for CloudSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloudSession(..)")
    }
}

impl CloudSession {
    /// The file name of the session, in the `bifrost` directory of the data directory (next to `accounts.ini`).
    pub const FILE: &'static str = "cloud_session";

    pub fn new(token: &str) -> Option<CloudSession> {
        let token = token.trim();
        (!token.is_empty() && !token.contains(char::is_whitespace)).then(|| CloudSession { token: token.to_string() })
    }

    /// Reads `token=<value>` from the file's text.
    pub fn parse(text: &str) -> Option<CloudSession> {
        text.lines().find_map(|l| l.trim().strip_prefix("token=")).and_then(CloudSession::new)
    }

    pub fn path() -> Option<PathBuf> {
        crate::accounts::shared_path().and_then(|p| p.parent().map(|d| d.join(CloudSession::FILE)))
    }

    pub fn load_from(path: &Path) -> Option<CloudSession> {
        CloudSession::parse(&std::fs::read_to_string(path).ok()?)
    }

    /// The session of this user, if they are signed in to BifrOSt Account on this device.
    pub fn load() -> Option<CloudSession> {
        CloudSession::path().and_then(|p| CloudSession::load_from(&p))
    }
}

/// Why a call to the bug API failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// 401 or 403: sign in again.
    NotSignedIn,
    /// 400 with the server's reason.
    Invalid(String),
    /// 413.
    TooLarge,
    /// 429: at most 10 reports per hour.
    RateLimited,
    /// 404.
    NotFound,
    /// Any other status.
    Server(u16),
    /// The request did not reach the server.
    Network(String),
    /// The answer was not what the contract describes.
    Malformed,
    /// The report fails the local checks.
    Report(ReportError),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotSignedIn => f.write_str("Sign in to BifrOSt Account in Settings to send reports."),
            ApiError::Invalid(why) => write!(f, "The server did not accept the report: {why}"),
            ApiError::TooLarge => f.write_str("The report is too large. Uncheck some diagnostics and try again."),
            ApiError::RateLimited => f.write_str("You have sent 10 reports in the last hour. Try again later."),
            ApiError::NotFound => f.write_str("That report was not found."),
            ApiError::Server(code) => write!(f, "The server answered {code}. Try again later."),
            ApiError::Network(why) => write!(f, "The report could not be sent: {why}"),
            ApiError::Malformed => f.write_str("The server's answer could not be read."),
            ApiError::Report(e) => write!(f, "{e}"),
        }
    }
}

/// The answer to `POST /bugs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub id: String,
    pub status: Status,
    pub created: u64,
}

/// One row of `GET /bugs/mine`.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub category: Category,
    pub status: Status,
    pub created: u64,
    pub updated: u64,
    pub resolution: Option<String>,
}

/// A diagnostic as listed by `GET /bugs/mine/{id}`: name, kind and size only.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticInfo {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
}

/// `GET /bugs/mine/{id}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub summary: Summary,
    pub description: String,
    pub app_id: Option<String>,
    pub diagnostics: Vec<DiagnosticInfo>,
}

/// Sends one request and returns the status and body text. [`session_send`] is the real one.
pub type Transport<'a> = dyn FnMut(&URLRequest) -> Result<(u16, String), String> + 'a;

/// The URL session transport (blocking).
pub fn session_send(req: &URLRequest) -> Result<(u16, String), String> {
    crate::dav_client::session_send(req)
}

fn error_for(status: u16, body: &str) -> ApiError {
    match status {
        400 => ApiError::Invalid(json::parse(body).ok().and_then(|j| j.str_field("error")).unwrap_or_else(|| "bad request".into())),
        401 | 403 => ApiError::NotSignedIn,
        404 => ApiError::NotFound,
        413 => ApiError::TooLarge,
        429 => ApiError::RateLimited,
        other => ApiError::Server(other),
    }
}

fn number(j: &Json, key: &str) -> Option<u64> {
    j.get(key).and_then(Json::as_u64)
}

fn summary_of(j: &Json) -> Option<Summary> {
    Some(Summary {
        id: j.str_field("id")?,
        title: j.str_field("title")?,
        category: Category::parse(&j.str_field("category")?).unwrap_or(Category::Other),
        status: Status::parse(&j.str_field("status")?)?,
        created: number(j, "created")?,
        updated: number(j, "updated").or_else(|| number(j, "created"))?,
        resolution: j.str_field("resolution"),
    })
}

/// The client of the three user endpoints.
#[derive(Clone, Debug)]
pub struct BugClient {
    base: String,
    session: CloudSession,
}

impl BugClient {
    pub fn new(session: CloudSession) -> BugClient {
        BugClient::with_base(BASE_URL, session)
    }

    pub fn with_base(base: &str, session: CloudSession) -> BugClient {
        BugClient { base: base.trim_end_matches('/').to_string(), session }
    }

    fn request(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> URLRequest {
        let mut req = URLRequest::new(&format!("{}{path}", self.base));
        req.method = method.into();
        req.body = body;
        req.set_header("Authorization", &format!("Bearer {}", self.session.token));
        req.set_header("Accept", "application/json");
        if req.body.is_some() {
            req.set_header("Content-Type", "application/json");
        }
        req.timeout = Duration::from_secs(120);
        req
    }

    /// `POST /bugs` for a validated report.
    pub fn submit_request(&self, report: &BugReport) -> Result<URLRequest, ReportError> {
        report.validate()?;
        Ok(self.request("POST", "/bugs", Some(report.to_json().into_bytes())))
    }

    pub fn mine_request(&self) -> URLRequest {
        self.request("GET", "/bugs/mine", None)
    }

    pub fn detail_request(&self, id: &str) -> URLRequest {
        let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        self.request("GET", &format!("/bugs/mine/{safe}"), None)
    }

    fn call(&self, req: &URLRequest, send: &mut Transport<'_>) -> Result<String, ApiError> {
        let (status, body) = send(req).map_err(ApiError::Network)?;
        if (200..300).contains(&status) {
            Ok(body)
        } else {
            Err(error_for(status, &body))
        }
    }

    /// Sends the report.
    pub fn submit(&self, report: &BugReport, send: &mut Transport<'_>) -> Result<Receipt, ApiError> {
        let req = self.submit_request(report).map_err(ApiError::Report)?;
        let body = self.call(&req, send)?;
        let j = json::parse(&body).map_err(|_| ApiError::Malformed)?;
        Ok(Receipt {
            id: j.str_field("id").ok_or(ApiError::Malformed)?,
            status: j.str_field("status").and_then(|s| Status::parse(&s)).ok_or(ApiError::Malformed)?,
            created: number(&j, "created").ok_or(ApiError::Malformed)?,
        })
    }

    /// The signed-in user's reports, newest first.
    pub fn mine(&self, send: &mut Transport<'_>) -> Result<Vec<Summary>, ApiError> {
        let body = self.call(&self.mine_request(), send)?;
        let j = json::parse(&body).map_err(|_| ApiError::Malformed)?;
        j.as_array().ok_or(ApiError::Malformed)?.iter().map(|row| summary_of(row).ok_or(ApiError::Malformed)).collect()
    }

    /// One of the user's reports.
    pub fn detail(&self, id: &str, send: &mut Transport<'_>) -> Result<Detail, ApiError> {
        let body = self.call(&self.detail_request(id), send)?;
        let j = json::parse(&body).map_err(|_| ApiError::Malformed)?;
        let diagnostics = j
            .get("diagnostics")
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|d| Some(DiagnosticInfo { name: d.str_field("name")?, kind: Kind::parse(&d.str_field("kind")?).unwrap_or(Kind::Other), size: number(d, "size").unwrap_or(0) }))
            .collect();
        Ok(Detail { summary: summary_of(&j).ok_or(ApiError::Malformed)?, description: j.str_field("description").unwrap_or_default(), app_id: j.str_field("app_id"), diagnostics })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::log::Level;
    use std::time::UNIX_EPOCH;

    fn report() -> BugReport {
        BugReport {
            title: "Notes quits when I paste".into(),
            description: "Paste a large image and the app closes.".into(),
            category: Category::Crash,
            reproducible: Reproducible::Always,
            app_id: Some("is.olibuijr.notes.desktop".into()),
            app_version: Some("1.2.0".into()),
            os_version: "0.7.0".into(),
            os_build: "7.2.5".into(),
            hardware: Hardware { model: Some("Midget".into()), cpu: None, gpu: None, memory_mb: Some(8192), fingerprint: None },
            diagnostics: vec![],
        }
    }

    fn entry(secs: u64, process: &str, subsystem: &str, message: &str) -> LogEntry {
        LogEntry { date: UNIX_EPOCH + Duration::from_secs(secs), process: process.into(), pid: 7, subsystem: subsystem.into(), category: "x".into(), level: Level::Error, message: message.into() }
    }

    fn client() -> BugClient {
        BugClient::with_base("https://example.test/api/cloud/", CloudSession::new("tok123").unwrap())
    }

    // model and limits

    #[test]
    fn a_complete_report_validates() {
        assert_eq!(report().validate(), Ok(()));
    }

    #[test]
    fn title_and_description_limits() {
        let mut r = report();
        r.title = "   ".into();
        assert_eq!(r.validate(), Err(ReportError::TitleEmpty));
        r.title = "x".repeat(MAX_TITLE);
        assert_eq!(r.validate(), Ok(()));
        r.title = "x".repeat(MAX_TITLE + 1);
        assert_eq!(r.validate(), Err(ReportError::TitleTooLong));
        let mut r = report();
        r.description = String::new();
        assert_eq!(r.validate(), Err(ReportError::DescriptionEmpty));
        r.description = "d".repeat(MAX_DESCRIPTION + 1);
        assert_eq!(r.validate(), Err(ReportError::DescriptionTooLong));
    }

    #[test]
    fn limits_count_characters_not_bytes() {
        let mut r = report();
        r.title = "é".repeat(MAX_TITLE);
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn diagnostic_limits() {
        let mut r = report();
        let d = |n: usize| Diagnostic { kind: Kind::Log, name: "l".into(), mime: "text/plain".into(), data: vec![0; n] };
        r.diagnostics = (0..=MAX_DIAGNOSTICS).map(|_| d(1)).collect();
        assert_eq!(r.validate(), Err(ReportError::TooManyDiagnostics));
        r.diagnostics = vec![d(MAX_DIAGNOSTIC_BYTES + 1)];
        assert_eq!(r.validate(), Err(ReportError::DiagnosticTooLarge("l".into())));
        r.diagnostics = vec![d(MAX_DIAGNOSTIC_BYTES); 3];
        assert_eq!(r.validate(), Err(ReportError::TooLarge), "three 10 MB items are about 40 MB of base64");
        r.diagnostics = vec![Diagnostic { name: " ".into(), ..d(1) }];
        assert_eq!(r.validate(), Err(ReportError::DiagnosticEmptyName));
    }

    #[test]
    fn missing_os_version_is_refused() {
        let mut r = report();
        r.os_version = String::new();
        assert_eq!(r.validate(), Err(ReportError::OsVersionMissing));
    }

    #[test]
    fn json_matches_the_contract() {
        let mut r = report();
        r.diagnostics = vec![Diagnostic { kind: Kind::Log, name: "log.txt".into(), mime: "text/plain".into(), data: b"hello".to_vec() }];
        let j = json::parse(&r.to_json()).unwrap();
        assert_eq!(j.str_field("category").as_deref(), Some("crash"));
        assert_eq!(j.str_field("reproducible").as_deref(), Some("always"));
        assert_eq!(j.str_field("app_id").as_deref(), Some("is.olibuijr.notes.desktop"));
        assert_eq!(j.get("hardware").unwrap().get("memory_mb").unwrap().as_u64(), Some(8192));
        assert!(j.get("hardware").unwrap().get("cpu").is_none(), "unset hardware fields are left out");
        let d = &j.get("diagnostics").unwrap().as_array().unwrap()[0];
        assert_eq!(d.str_field("kind").as_deref(), Some("log"));
        assert_eq!(d.str_field("data_b64").as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn json_leaves_out_an_unset_app() {
        let mut r = report();
        r.app_id = None;
        r.app_version = None;
        let j = json::parse(&r.to_json()).unwrap();
        assert!(j.get("app_id").is_none() && j.get("app_version").is_none());
    }

    #[test]
    fn vocabulary_round_trips() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
        }
        for r in Reproducible::ALL {
            assert_eq!(Reproducible::parse(r.as_str()), Some(r));
        }
        assert_eq!(Status::parse("in-progress"), Some(Status::InProgress));
        assert_eq!(Status::parse("wont-fix"), Some(Status::WontFix));
        assert_eq!(Kind::parse("screenshot"), Some(Kind::Screenshot));
        assert_eq!(Category::parse("nope"), None);
    }

    // redaction

    #[test]
    fn redacts_key_value_secrets() {
        let out = redact("login password=hunter2 ok, api_key: abc123 done", None);
        assert!(!out.contains("hunter2") && !out.contains("abc123"), "{out}");
        assert!(out.contains("password=[redacted]") && out.contains("ok,"), "{out}");
    }

    #[test]
    fn redacts_json_and_quoted_secrets() {
        let out = redact(r#"{"access_token":"abcdef","user":"x"} refresh_token='zzz'"#, None);
        assert!(!out.contains("abcdef") && !out.contains("zzz"), "{out}");
        assert!(out.contains(r#""user":"x""#), "other fields stay: {out}");
    }

    #[test]
    fn redacts_bearer_and_cookie() {
        let out = redact("Authorization: Bearer eyJhbGciOi.payload.sig\nCookie: bc_session=s3cr3t; theme=dark", None);
        assert!(!out.contains("eyJhbGciOi") && !out.contains("s3cr3t"), "{out}");
        assert!(out.contains("Bearer [redacted]"), "{out}");
    }

    #[test]
    fn leaves_ordinary_words_alone() {
        let text = "the tokenizer stopped; secretary on duty; passwords page opened";
        assert_eq!(redact(text, None), text);
    }

    #[test]
    fn redacts_emails() {
        let out = redact("sent to olibuijr@olibuijr.com and bob.smith+x@mail.example.org.", None);
        assert_eq!(out, "sent to [email] and [email].");
        assert_eq!(redact("at @home or a@b", None), "at @home or a@b");
    }

    #[test]
    fn redacts_home_folder_names() {
        let out = redact("open /home/alice/Documents/a.txt and /Users/bob/x", None);
        assert_eq!(out, "open /home/[user]/Documents/a.txt and /Users/[user]/x");
    }

    #[test]
    fn redacts_the_login_name_as_a_word() {
        let out = redact("user alice logged in; alicesmith unchanged; alice.", Some("alice"));
        assert_eq!(out, "user [user] logged in; alicesmith unchanged; [user].");
        assert_eq!(redact("al is here", Some("al")), "al is here", "names under 3 characters are not matched");
    }

    #[test]
    fn redacts_long_tokens() {
        let token = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8";
        assert_eq!(redact(&format!("key {token} end"), None), "key [redacted] end");
        assert_eq!(redact("abcdefghijklmnopqrstuvwxyzabcdefghij", None), "abcdefghijklmnopqrstuvwxyzabcdefghij", "no digit: a plain word");
    }

    #[test]
    fn redaction_keeps_non_ascii_text() {
        assert_eq!(redact("Hæ, þetta virkar ekki", None), "Hæ, þetta virkar ekki");
    }

    // diagnostics and consent

    #[test]
    fn items_start_unchecked_and_only_checked_items_are_sent() {
        let mut items = vec![sysinfo_item(&SystemFacts { os_version: "0.7".into(), ..Default::default() }), crash_item("report", "app", None), screenshot_item(vec![1, 2, 3])];
        assert!(items.iter().all(|i| !i.consent));
        assert!(consented(&items).is_empty(), "nothing is sent unchecked");
        items[1].consent = true;
        let sent = consented(&items);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, Kind::Crash);
    }

    #[test]
    fn a_checked_empty_item_is_not_sent() {
        let mut item = Item::new("log", Kind::Log, "log.txt", "text/plain", vec![]);
        item.consent = true;
        assert!(consented(&[item]).is_empty());
    }

    #[test]
    fn preview_and_size() {
        let item = Item::new("a", Kind::Log, "a.txt", "text/plain", b"abcdefghij".to_vec());
        assert_eq!(item.preview(4), "abcd\n...");
        assert_eq!(item.preview(100), "abcdefghij");
        assert_eq!(item.size_label(), "10 bytes");
        let shot = screenshot_item(vec![0; 2048]);
        assert_eq!(shot.size_label(), "2.0 KB");
        assert!(shot.preview(10).contains("image/png"));
        assert_eq!(size_label(3 << 20), "3.0 MB");
    }

    #[test]
    fn sysinfo_item_lists_the_facts_and_the_hardware_block() {
        let facts = SystemFacts {
            os_name: "BifrOSt".into(),
            os_version: "0.7.0".into(),
            os_build: "7.2.5-3".into(),
            model: "Midget".into(),
            cpu: "Intel Core i7".into(),
            gpus: vec!["Intel Display".into()],
            memory_bytes: 8 << 30,
        };
        let text = String::from_utf8(sysinfo_item(&facts).data).unwrap();
        for want in ["BifrOSt 0.7.0", "7.2.5-3", "Midget", "Intel Core i7", "Intel Display", "8192 MB"] {
            assert!(text.contains(want), "{want} in {text}");
        }
        let hw = facts.hardware();
        assert_eq!((hw.model.as_deref(), hw.memory_mb, hw.gpu.as_deref()), (Some("Midget"), Some(8192), Some("Intel Display")));
        assert_eq!(SystemFacts::default().hardware(), Hardware::default());
    }

    #[test]
    fn log_item_filters_by_app_and_time_and_redacts() {
        let now = UNIX_EPOCH + Duration::from_secs(10_000);
        let entries = vec![
            entry(9_990, "notes", "is.olibuijr.notes.desktop", "saved /home/alice/n.txt"),
            entry(9_995, "mail", "is.olibuijr.mail.desktop", "other app"),
            entry(100, "notes", "is.olibuijr.notes.desktop", "too old"),
            entry(9_999, "notes", "is.olibuijr.notes.desktop", "token=abc123 refused"),
        ];
        let filter = LogFilter { app: Some("is.olibuijr.notes.desktop".into()), since: Duration::from_secs(900), limit: 10 };
        let item = log_item(&entries, &filter, now, None);
        let text = String::from_utf8(item.data).unwrap();
        assert_eq!(item.kind, Kind::Log);
        assert!(text.contains("saved /home/[user]/n.txt"), "{text}");
        assert!(text.contains("token=[redacted]"), "{text}");
        assert!(!text.contains("other app") && !text.contains("too old") && !text.contains("abc123"), "{text}");
        assert!(text.find("saved").unwrap() < text.find("refused").unwrap(), "oldest first");
    }

    #[test]
    fn log_item_keeps_the_newest_lines_within_the_limit() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let entries: Vec<LogEntry> = (0..5).map(|i| entry(990 + i, "p", "s", &format!("line{i}"))).collect();
        let filter = LogFilter { app: None, since: Duration::from_secs(60), limit: 2 };
        let text = String::from_utf8(log_item(&entries, &filter, now, None).data).unwrap();
        assert!(text.contains("line3") && text.contains("line4") && !text.contains("line2"), "{text}");
    }

    #[test]
    fn crash_item_redacts_and_names_the_file_safely() {
        let item = crash_item("Process: Notes\nopened /home/alice/a", "is.olibuijr/notes", None);
        assert_eq!(item.kind, Kind::Crash);
        assert_eq!(item.name, "crash-is.olibuijr_notes.txt");
        assert!(String::from_utf8(item.data).unwrap().contains("/home/[user]/a"));
    }

    #[test]
    fn crash_text_is_read_only_for_the_app_that_crashed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("last_crash_report.txt"), "Process: Notes (is.olibuijr.notes.desktop)\nReason: signal 11\n").unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        assert!(read_crash_text(&dirs, "is.olibuijr.notes.desktop").is_some());
        assert!(read_crash_text(&dirs, "is.olibuijr.mail.desktop").is_none());
        assert!(read_crash_text(&[PathBuf::from("/nonexistent-dir")], "x").is_none());
    }

    #[test]
    fn config_item_drops_secret_keys_and_redacts_values() {
        let pairs = vec![
            ("appearance".to_string(), "dark".to_string()),
            ("wifi_password".to_string(), "hunter2".to_string()),
            ("api_token".to_string(), "t".to_string()),
            ("home".to_string(), "/home/alice/x".to_string()),
        ];
        let text = String::from_utf8(config_item(&pairs, None).data).unwrap();
        assert_eq!(text, "appearance=dark\nhome=/home/[user]/x\n");
        assert!(is_secret_key("Wifi_Password") && !is_secret_key("appearance"));
    }

    #[test]
    fn settings_file_parses() {
        let pairs = parse_settings("# c\n\nappearance = dark\nbad line\nlang=is\n");
        assert_eq!(pairs, vec![("appearance".to_string(), "dark".to_string()), ("lang".to_string(), "is".to_string())]);
    }

    // client

    #[test]
    fn session_is_read_from_its_file_and_hides_the_token() {
        assert!(CloudSession::parse("token=abc\n").is_some());
        assert!(CloudSession::parse("token=\n").is_none());
        assert!(CloudSession::parse("other=1\n").is_none());
        assert!(CloudSession::new("a b").is_none());
        assert!(!format!("{:?}", CloudSession::new("secret").unwrap()).contains("secret"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CloudSession::FILE);
        assert!(CloudSession::load_from(&path).is_none());
        std::fs::write(&path, "token=xyz\n").unwrap();
        assert!(CloudSession::load_from(&path).is_some());
    }

    #[test]
    fn requests_carry_the_bearer_token_and_the_contract_paths() {
        let c = client();
        let post = c.submit_request(&report()).unwrap();
        assert_eq!((post.method.as_str(), post.url.as_str()), ("POST", "https://example.test/api/cloud/bugs"));
        let auth = post.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.as_str());
        assert_eq!(auth, Some("Bearer tok123"));
        assert!(post.headers.iter().any(|(k, v)| k == "Content-Type" && v == "application/json"));
        assert_eq!(c.mine_request().url, "https://example.test/api/cloud/bugs/mine");
        assert_eq!(c.detail_request("bug_01/../x").url, "https://example.test/api/cloud/bugs/mine/bug_01x", "the id cannot change the path");
    }

    #[test]
    fn an_invalid_report_is_not_sent() {
        let mut bad = report();
        bad.title = String::new();
        let mut called = false;
        let r = client().submit(&bad, &mut |_| {
            called = true;
            Ok((201, String::new()))
        });
        assert_eq!(r, Err(ApiError::Report(ReportError::TitleEmpty)));
        assert!(!called);
    }

    #[test]
    fn submit_reads_the_receipt() {
        let r = client().submit(&report(), &mut |_| Ok((201, r#"{"id":"bug_01H","status":"new","created":1760000000}"#.into())));
        assert_eq!(r, Ok(Receipt { id: "bug_01H".into(), status: Status::New, created: 1_760_000_000 }));
    }

    #[test]
    fn errors_map_from_status() {
        let status = |code: u16, body: &str| client().submit(&report(), &mut |_| Ok((code, body.to_string()))).unwrap_err();
        assert_eq!(status(400, r#"{"error":"title too long"}"#), ApiError::Invalid("title too long".into()));
        assert_eq!(status(401, ""), ApiError::NotSignedIn);
        assert_eq!(status(413, ""), ApiError::TooLarge);
        assert_eq!(status(429, ""), ApiError::RateLimited);
        assert_eq!(status(500, ""), ApiError::Server(500));
        let net = client().submit(&report(), &mut |_| Err("offline".into())).unwrap_err();
        assert_eq!(net, ApiError::Network("offline".into()));
        assert_eq!(client().submit(&report(), &mut |_| Ok((201, "nope".into()))).unwrap_err(), ApiError::Malformed);
    }

    #[test]
    fn mine_lists_reports_newest_first() {
        let body = r#"[{"id":"bug_2","title":"B","category":"ui","status":"fixed","created":20,"updated":30,"resolution":"Fixed in 0.7.1"},
                       {"id":"bug_1","title":"A","category":"crash","status":"in-progress","created":10,"updated":10}]"#;
        let rows = client().mine(&mut |_| Ok((200, body.into()))).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].status, rows[0].category, rows[0].resolution.as_deref()), (Status::Fixed, Category::Ui, Some("Fixed in 0.7.1")));
        assert_eq!((rows[1].id.as_str(), rows[1].status, rows[1].resolution.clone()), ("bug_1", Status::InProgress, None));
        assert_eq!(client().mine(&mut |_| Ok((200, r#"{"x":1}"#.into()))).unwrap_err(), ApiError::Malformed);
    }

    #[test]
    fn detail_lists_diagnostics_by_name_kind_and_size() {
        let body = r#"{"id":"bug_1","title":"A","category":"crash","status":"new","created":10,"updated":12,"description":"d","app_id":"x.y",
                       "diagnostics":[{"name":"log.txt","kind":"log","size":120},{"name":"c","kind":"weird","size":5}]}"#;
        let d = client().detail("bug_1", &mut |_| Ok((200, body.into()))).unwrap();
        assert_eq!((d.description.as_str(), d.app_id.as_deref()), ("d", Some("x.y")));
        assert_eq!(d.diagnostics, vec![DiagnosticInfo { name: "log.txt".into(), kind: Kind::Log, size: 120 }, DiagnosticInfo { name: "c".into(), kind: Kind::Other, size: 5 }]);
        assert_eq!(client().detail("nope", &mut |_| Ok((404, String::new()))).unwrap_err(), ApiError::NotFound);
    }
}
