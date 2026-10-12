//! The log model: entries, levels, the search tokens and the bounded buffer. The parsers for `journalctl -o json`
//! and `coredumpctl list --json=short` are here too, so they are tested without a system log. No I/O in this file.

use bifrost_core::json::{self, Json};
use std::collections::VecDeque;

/// Entries kept in memory; the oldest go first.
pub const MAX_ENTRIES: usize = 50_000;

/// Message levels, as Console shows them. `Notice` is "Default".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    #[default]
    Notice,
    Error,
    Fault,
}

impl Level {
    /// From journald's `PRIORITY` (0 emerg .. 7 debug). A missing value reads as `Notice`, default.
    pub fn from_priority(priority: Option<u32>) -> Level {
        match priority {
            None => Level::Notice,
            Some(0..=2) => Level::Fault,
            Some(3) => Level::Error,
            Some(4 | 5) => Level::Notice,
            Some(6) => Level::Info,
            Some(_) => Level::Debug,
        }
    }

    /// The label in the Type column and in the detail pane.
    pub fn label(self) -> &'static str {
        match self {
            Level::Debug => "Debug",
            Level::Info => "Info",
            Level::Notice => "Default",
            Level::Error => "Error",
            Level::Fault => "Fault",
        }
    }

    /// The word after `type:` in a search (`type:error`).
    pub fn parse_word(word: &str) -> Option<Level> {
        Some(match word.to_ascii_lowercase().as_str() {
            "debug" => Level::Debug,
            "info" => Level::Info,
            "default" | "notice" => Level::Notice,
            "error" => Level::Error,
            "fault" => Level::Fault,
            _ => return None,
        })
    }
}

/// One message: what `log show` gives (date, process, pid, subsystem, category, level, message).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogEntry {
    /// Unix time in microseconds, 0 when unknown.
    pub date: i64,
    pub process: String,
    pub pid: u32,
    pub subsystem: String,
    pub category: String,
    pub level: Level,
    pub message: String,
}

/// The text of a journal field: a JSON string, or the bytes journald writes for a value that is not UTF-8.
fn field_text(v: &Json) -> Option<String> {
    match v {
        Json::Str(s) => Some(s.clone()),
        Json::Num(n) => Some(n.to_string()),
        Json::Arr(bytes) => {
            let raw: Vec<u8> = bytes.iter().filter_map(|b| b.number()).map(|n| n as u8).collect();
            Some(String::from_utf8_lossy(&raw).into_owned())
        }
        _ => None,
    }
}

/// One line of `journalctl -o json`. Returns None for a line that is not a JSON object.
pub fn parse_journal_line(line: &str) -> Option<LogEntry> {
    static REGISTRY: std::sync::OnceLock<Vec<bifrost_core::log::Spec>> = std::sync::OnceLock::new();
    parse_journal_line_in(line, REGISTRY.get_or_init(bifrost_core::log::read_registry))
}

/// As `parse_journal_line`; an app that claims a subsystem it does not own (by the kernel-set `_EXE`) keeps the
/// entry but loses the subsystem and category (`bifrost_core::log::may_log_as`).
pub fn parse_journal_line_in(line: &str, registry: &[bifrost_core::log::Spec]) -> Option<LogEntry> {
    let obj = json::parse(line.trim()).ok()?;
    if !matches!(obj, Json::Obj(_)) {
        return None;
    }
    let text = |key: &str| obj.get(key).and_then(field_text);
    Some(LogEntry {
        date: text("__REALTIME_TIMESTAMP").and_then(|t| t.parse().ok()).unwrap_or(0),
        process: text("SYSLOG_IDENTIFIER").or_else(|| text("_COMM")).unwrap_or_default(),
        pid: text("_PID").and_then(|p| p.parse().ok()).unwrap_or(0),
        subsystem: text("BIFROST_SUBSYSTEM").unwrap_or_default(),
        category: text("BIFROST_CATEGORY").unwrap_or_default(),
        level: Level::from_priority(text("PRIORITY").and_then(|p| p.parse().ok())),
        message: text("MESSAGE").unwrap_or_default(),
    })
    .map(|mut e| {
        if !e.subsystem.is_empty() && !bifrost_core::log::may_log_as(registry, text("_EXE").as_deref(), &e.subsystem) {
            e.message = format!("[claimed subsystem {}] {}", e.subsystem, e.message);
            e.subsystem.clear();
            e.category.clear();
        }
        e
    })
}

/// The name of a Unix signal number, as the shell prints it.
pub fn signal_name(sig: u32) -> String {
    let name = match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        31 => "SIGSYS",
        _ => return format!("signal {sig}"),
    };
    name.to_string()
}

/// `coredumpctl list --json=short`: an array of objects with `time` (microseconds), `pid`, `sig`, `exe` and
/// `corefile`. A crash is a fault for the signals that mean a bug in the program, an error otherwise.
pub fn parse_coredump_list(text: &str) -> Option<Vec<LogEntry>> {
    let all = json::parse(text.trim()).ok()?;
    let rows = all.as_array()?;
    Some(
        rows.iter()
            .map(|row| {
                let num = |key: &str| row.get(key).and_then(|v| v.number().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
                let sig = num("sig").unwrap_or(0.0) as u32;
                let exe = row.get("exe").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let shown = if exe.is_empty() { "Unknown program".to_string() } else { exe.clone() };
                LogEntry {
                    date: num("time").unwrap_or(0.0) as i64,
                    process: exe.rsplit('/').next().unwrap_or("").to_string(),
                    pid: num("pid").unwrap_or(0.0) as u32,
                    subsystem: "systemd-coredump".into(),
                    category: row.get("corefile").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
                    level: if matches!(sig, 4 | 6 | 7 | 8 | 11 | 31) { Level::Fault } else { Level::Error },
                    message: format!("{shown} terminated by {}", signal_name(sig)),
                }
            })
            .collect(),
    )
}

/// A parsed search: `process:`, `subsystem:`, `category:` and `message:` take substrings (each one must match),
/// `type:` or `level:` a level, and anything else is free text that must appear in the message, process, subsystem
/// or category.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub process: Vec<String>,
    pub subsystem: Vec<String>,
    pub category: Vec<String>,
    /// `message:` tokens: each must appear in the message text only (the predicate's `message CONTAINS`).
    pub message: Vec<String>,
    pub level: Option<Level>,
    pub text: Vec<String>,
}

/// Splits on spaces outside double quotes; the quotes are dropped (`process:"Notes App"` is one token).
fn tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut open = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                open = true;
            }
            c if c.is_whitespace() && !quoted => {
                if open {
                    out.push(std::mem::take(&mut cur));
                    open = false;
                }
            }
            c => {
                cur.push(c);
                open = true;
            }
        }
    }
    if open {
        out.push(cur);
    }
    out
}

fn has(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(needle)
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        for token in tokens(input) {
            if let Some((key, value)) = token.split_once(':') {
                if !value.is_empty() {
                    let value = value.to_lowercase();
                    match key.to_ascii_lowercase().as_str() {
                        "process" => {
                            q.process.push(value);
                            continue;
                        }
                        "subsystem" => {
                            q.subsystem.push(value);
                            continue;
                        }
                        "category" => {
                            q.category.push(value);
                            continue;
                        }
                        "message" => {
                            q.message.push(value);
                            continue;
                        }
                        "type" | "level" => {
                            if let Some(level) = Level::parse_word(&value) {
                                q.level = Some(level);
                                continue;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if !token.is_empty() {
                q.text.push(token.to_lowercase());
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.process.is_empty()
            && self.subsystem.is_empty()
            && self.category.is_empty()
            && self.message.is_empty()
            && self.level.is_none()
            && self.text.is_empty()
    }

    pub fn matches(&self, e: &LogEntry) -> bool {
        if self.level.is_some_and(|l| l != e.level) {
            return false;
        }
        self.process.iter().all(|n| has(&e.process, n))
            && self.subsystem.iter().all(|n| has(&e.subsystem, n))
            && self.category.iter().all(|n| has(&e.category, n))
            && self.message.iter().all(|n| has(&e.message, n))
            && self.text.iter().all(|n| has(&e.message, n) || has(&e.process, n) || has(&e.subsystem, n) || has(&e.category, n))
    }
}

/// A search the user kept in the sidebar. Its text is parsed with `Query::parse` when the row is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSearch {
    pub text: String,
}

/// Keeps `text` as a saved search, unless it is blank or already saved. Returns true when the list changed.
pub fn save_search(saved: &mut Vec<SavedSearch>, text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || saved.iter().any(|s| s.text == text) {
        return false;
    }
    saved.push(SavedSearch { text: text.to_string() });
    true
}

/// Removes the saved search with exactly this text. Returns true when one was removed.
pub fn remove_search(saved: &mut Vec<SavedSearch>, text: &str) -> bool {
    let text = text.trim();
    let before = saved.len();
    saved.retain(|s| s.text != text);
    saved.len() != before
}

/// The toolbar toggles. Default, Error and Fault always show; Info and Debug only when their toggle is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    pub info: bool,
    pub debug: bool,
}

impl Levels {
    pub fn shows(self, level: Level) -> bool {
        match level {
            Level::Debug => self.debug,
            Level::Info => self.info,
            _ => true,
        }
    }
}

/// The entries in memory, oldest first, each with a sequence number that never repeats (so a selection survives
/// the oldest entries leaving).
pub struct Buffer {
    items: VecDeque<(u64, LogEntry)>,
    next: u64,
    cap: usize,
}

impl Buffer {
    pub fn new(cap: usize) -> Self {
        Buffer { items: VecDeque::new(), next: 0, cap: cap.max(1) }
    }

    /// Adds an entry and returns its sequence number. When the buffer is full the oldest entry goes.
    pub fn push(&mut self, e: LogEntry) -> u64 {
        if self.items.len() >= self.cap {
            self.items.pop_front();
        }
        let seq = self.next;
        self.next += 1;
        self.items.push_back((seq, e));
        seq
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `(sequence, entry)` pairs, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &LogEntry)> + '_ {
        self.items.iter().map(|(s, e)| (*s, e))
    }

    pub fn get(&self, seq: u64) -> Option<&LogEntry> {
        let i = self.items.binary_search_by_key(&seq, |(s, _)| *s).ok()?;
        Some(&self.items[i].1)
    }

    /// The oldest sequence number still held.
    pub fn first_seq(&self) -> Option<u64> {
        self.items.front().map(|(s, _)| *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"{"__CURSOR":"s=1","__REALTIME_TIMESTAMP":"1760100000123456","PRIORITY":"3","_PID":"4242","SYSLOG_IDENTIFIER":"bifrost-notes","BIFROST_SUBSYSTEM":"is.olibuijr.notes","BIFROST_CATEGORY":"sync","MESSAGE":"sync failed"}"#;

    fn entry(process: &str, subsystem: &str, category: &str, level: Level, message: &str) -> LogEntry {
        LogEntry { date: 1, process: process.into(), pid: 1, subsystem: subsystem.into(), category: category.into(), level, message: message.into() }
    }

    #[test]
    fn an_app_cannot_log_under_another_apps_subsystem() {
        let reg = bifrost_core::log::registry_from_manifests(&["id = \"is.o.notes.desktop\"\n[[log.subsystem]]\nid = \"is.o.notes\"\n"]);
        let line = |exe: &str| format!(r#"{{"__REALTIME_TIMESTAMP":"1","_EXE":"{exe}","BIFROST_SUBSYSTEM":"is.o.notes","BIFROST_CATEGORY":"sync","MESSAGE":"m"}}"#);
        assert_eq!(parse_journal_line_in(&line("/var/lib/springboard/apps/is.o.notes.desktop/1/n"), &reg).unwrap().subsystem, "is.o.notes");
        let spoof = parse_journal_line_in(&line("/var/lib/springboard/apps/is.o.evil/1/e"), &reg).unwrap();
        assert!(spoof.subsystem.is_empty() && spoof.message.starts_with("[claimed subsystem"));
    }

    #[test]
    fn a_journal_line_becomes_an_entry() {
        let e = parse_journal_line(LINE).unwrap();
        assert_eq!(e.date, 1_760_100_000_123_456);
        assert_eq!((e.process.as_str(), e.pid), ("bifrost-notes", 4242));
        assert_eq!((e.subsystem.as_str(), e.category.as_str()), ("is.olibuijr.notes", "sync"));
        assert_eq!(e.level, Level::Error);
        assert_eq!(e.message, "sync failed");
    }

    #[test]
    fn missing_fields_default_and_byte_messages_decode() {
        let e = parse_journal_line(r#"{"MESSAGE":[104,105],"_COMM":"sh"}"#).unwrap();
        assert_eq!(e.message, "hi");
        assert_eq!(e.process, "sh");
        assert_eq!((e.pid, e.date, e.level), (0, 0, Level::Notice));
        assert!(e.subsystem.is_empty() && e.category.is_empty());
    }

    #[test]
    fn journald_priorities_map_to_console_levels() {
        let level = |p: &str| parse_journal_line(&format!(r#"{{"PRIORITY":"{p}"}}"#)).unwrap().level;
        assert_eq!([level("0"), level("2"), level("3"), level("4"), level("6"), level("7")], [Level::Fault, Level::Fault, Level::Error, Level::Notice, Level::Info, Level::Debug]);
    }

    #[test]
    fn lines_that_are_not_objects_are_rejected() {
        assert!(parse_journal_line("not json").is_none());
        assert!(parse_journal_line("[1,2]").is_none());
        assert!(parse_journal_line("").is_none());
    }

    #[test]
    fn coredump_list_rows_become_crash_entries() {
        let text = r#"[{"time":1760100000000000,"pid":77,"sig":11,"corefile":"present","exe":"/usr/bin/bifrost-notes"},{"time":"1760100001000000","pid":"78","sig":"15","corefile":"missing","exe":""}]"#;
        let rows = parse_coredump_list(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].process.as_str(), rows[0].pid, rows[0].level), ("bifrost-notes", 77, Level::Fault));
        assert_eq!(rows[0].message, "/usr/bin/bifrost-notes terminated by SIGSEGV");
        assert_eq!((rows[1].pid, rows[1].level, rows[1].date), (78, Level::Error, 1_760_100_001_000_000));
        assert!(rows[1].message.starts_with("Unknown program terminated by SIGTERM"));
        assert!(parse_coredump_list("oops").is_none());
    }

    #[test]
    fn query_tokens_split_and_keep_quoted_values() {
        let q = Query::parse(r#"process:notes subsystem:is.olibuijr category:sync type:ERROR "two words" process:"Notes App""#);
        assert_eq!(q.process, vec!["notes".to_string(), "notes app".to_string()]);
        assert_eq!(q.subsystem, vec!["is.olibuijr".to_string()]);
        assert_eq!(q.category, vec!["sync".to_string()]);
        assert_eq!(q.level, Some(Level::Error));
        assert_eq!(q.text, vec!["two words".to_string()]);
    }

    #[test]
    fn an_unknown_type_or_an_empty_value_is_free_text() {
        let q = Query::parse("type:bogus process: Fault");
        assert_eq!(q.level, None);
        assert_eq!(q.text, vec!["type:bogus".to_string(), "process:".to_string(), "fault".to_string()]);
    }

    #[test]
    fn query_matches_tokens_and_free_text() {
        let e = entry("bifrost-notes", "is.olibuijr.notes", "sync", Level::Error, "sync failed: timeout");
        assert!(Query::parse("").is_empty());
        assert!(Query::parse("process:notes subsystem:olibuijr category:SYN").matches(&e));
        assert!(Query::parse("type:error timeout").matches(&e));
        assert!(Query::parse("NOTES").matches(&e), "free text also looks at the process");
        assert!(!Query::parse("type:info").matches(&e));
        assert!(!Query::parse("process:calendar").matches(&e));
        assert!(!Query::parse("timeout process:calendar").matches(&e), "every token must match");
    }

    #[test]
    fn level_and_message_tokens_filter_like_type_and_the_message_predicate() {
        let e = entry("bifrost-notes", "is.olibuijr.notes", "sync", Level::Error, "sync failed: timeout");
        assert_eq!(Query::parse("level:error").level, Some(Level::Error));
        assert!(Query::parse("level:error").matches(&e));
        assert!(!Query::parse("level:info").matches(&e));
        assert_eq!(Query::parse("message:timeout").message, vec!["timeout".to_string()]);
        assert!(Query::parse("message:timeout").matches(&e));
        assert!(Query::parse("message:failed process:notes").matches(&e));
        assert!(!Query::parse("message:notes").matches(&e), "message: looks only at the message text");
        assert!(!Query::parse("message:bogus").matches(&e));
        assert!(!Query::parse("message:timeout").is_empty());
        // a bare `message:` has no value, so it is free text, as before
        assert_eq!(Query::parse("message:").text, vec!["message:".to_string()]);
    }

    #[test]
    fn saved_searches_keep_trimmed_unique_text_and_can_be_removed() {
        let mut saved = Vec::new();
        assert!(save_search(&mut saved, "  type:error "));
        assert_eq!(saved, vec![SavedSearch { text: "type:error".into() }]);
        assert!(!save_search(&mut saved, "type:error"), "a second copy is not added");
        assert!(!save_search(&mut saved, "   "), "a blank search is not saved");
        assert_eq!(saved.len(), 1);
        assert!(remove_search(&mut saved, "type:error"));
        assert!(saved.is_empty());
        assert!(!remove_search(&mut saved, "type:error"));
    }

    #[test]
    fn toggles_hide_info_and_debug_only() {
        let off = Levels { info: false, debug: false };
        let on = Levels { info: true, debug: true };
        assert!(!off.shows(Level::Info) && !off.shows(Level::Debug));
        assert!(off.shows(Level::Notice) && off.shows(Level::Error) && off.shows(Level::Fault));
        assert!(on.shows(Level::Debug) && on.shows(Level::Info));
    }

    #[test]
    fn the_buffer_keeps_the_newest_entries_and_stable_sequence_numbers() {
        let mut b = Buffer::new(3);
        let seqs: Vec<u64> = (0..5).map(|i| b.push(entry("p", "", "", Level::Notice, &i.to_string()))).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
        assert_eq!(b.len(), 3);
        assert_eq!(b.first_seq(), Some(2));
        assert!(b.get(1).is_none());
        assert_eq!(b.get(4).unwrap().message, "4");
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.push(entry("p", "", "", Level::Notice, "")), 5);
    }
}
