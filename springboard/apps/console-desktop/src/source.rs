//! Where the entries come from: the journal (`journalctl`), the crash reports (`coredumpctl`) and the files in
//! `/var/log`. The window sees only `Source`; it never starts a process itself.

use crate::model::{self, Level, LogEntry};
use bifrost_app::dispatch::{DispatchGroup, DispatchQueue, Qos};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long dropping a follow waits for its reader to see the end of the pipe (killed, so this is immediate).
const READER_JOIN: Duration = Duration::from_secs(2);

/// Receives the entries that arrive while a source is followed. It runs on the utility queue.
pub type Sink = Box<dyn FnMut(LogEntry) + Send>;

pub trait Source {
    /// The newest `limit` entries, oldest first.
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String>;
    /// Sends new entries to `sink` until the returned `Follow` is dropped. None when the source is not live.
    fn follow(&mut self, _sink: Sink) -> Result<Option<Follow>, String> {
        Ok(None)
    }
    /// Extra text for the detail pane, such as a crash's `coredumpctl info`.
    fn detail(&mut self, _e: &LogEntry) -> Option<String> {
        None
    }
}

/// A running `journalctl --follow`. Dropping it stops the process; `finished` is true when it ended on its own.
pub struct Follow {
    child: Child,
    reader: DispatchGroup,
    done: Arc<AtomicBool>,
}

impl Follow {
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.reader.wait(Some(READER_JOIN));
    }
}

fn capture(args: &[&str]) -> Result<Output, String> {
    Command::new(args[0]).args(&args[1..]).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", args[0]))
}

fn stderr_text(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if text.is_empty() { format!("exit status {}", out.status) } else { text }
}

/// The fixed journal the testing kit shows (`BIFROST_TEST_PIN=1`): same rows and times on every machine.
fn pinned_entries() -> Vec<LogEntry> {
    let base = bifrost_app::testing::unix_secs() * 1_000_000;
    let row = |i: i64, process: &str, level: Level, message: &str| LogEntry {
        date: base + i * 1_250_000,
        process: process.into(),
        pid: 100 + i as u32,
        subsystem: "is.olibuijr.test".into(),
        category: "general".into(),
        level,
        message: message.into(),
    };
    vec![
        row(0, "springboard", Level::Notice, "shell ready"),
        row(1, "bifrost-notes", Level::Error, "sync failed"),
        row(2, "bifrost-books", Level::Notice, "library scanned"),
    ]
}

/// The system journal. `load` reads the newest entries; `follow` streams new ones.
pub struct Journal;

impl Source for Journal {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        if bifrost_app::testing::pinned() {
            return Ok(pinned_entries());
        }
        let n = limit.to_string();
        let out = capture(&["journalctl", "-o", "json", "--no-pager", "-n", n.as_str()])?;
        if !out.status.success() {
            return Err(stderr_text(&out));
        }
        Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(model::parse_journal_line).collect())
    }

    fn follow(&mut self, mut sink: Sink) -> Result<Option<Follow>, String> {
        if bifrost_app::testing::pinned() {
            return Ok(None);
        }
        let mut child = Command::new("journalctl")
            .args(["-o", "json", "--no-pager", "--follow", "-n", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("journalctl: {e}"))?;
        let stdout = child.stdout.take().ok_or("journalctl: no output pipe")?;
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let reader = DispatchGroup::new();
        DispatchQueue::global(Qos::Utility).async_group(&reader, move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(e) = model::parse_journal_line(&line) {
                    sink(e);
                }
            }
            flag.store(true, Ordering::Relaxed);
            // the end of the journal is news: wake the window so it notices `finished`
            DispatchQueue::main().async_(|| {});
        });
        Ok(Some(Follow { child, reader, done }))
    }
}

/// The crash reports systemd-coredump kept, from `coredumpctl`.
pub struct Coredumps;

impl Source for Coredumps {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        let out = capture(&["coredumpctl", "list", "--json=short", "--no-pager"])?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // with no cores coredumpctl prints nothing (and exits non-zero): that is an empty list, not an error
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = model::parse_coredump_list(&text).ok_or_else(|| "coredumpctl: unexpected output".to_string())?;
        if rows.len() > limit {
            rows.drain(..rows.len() - limit);
        }
        Ok(rows)
    }

    fn detail(&mut self, e: &LogEntry) -> Option<String> {
        if e.pid == 0 {
            return None;
        }
        let pid = e.pid.to_string();
        let out = capture(&["coredumpctl", "info", pid.as_str(), "--no-pager"]).ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// One plain file, read from its end: each line is a message from that file.
pub struct LogFile {
    pub path: PathBuf,
}

impl Source for LogFile {
    fn load(&mut self, limit: usize) -> Result<Vec<LogEntry>, String> {
        const TAIL: u64 = 4 << 20;
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut file = File::open(&self.path).map_err(|e| format!("{}: {e}", self.path.display()))?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let cut = len > TAIL;
        if cut {
            file.seek(SeekFrom::Start(len - TAIL)).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut lines: Vec<&str> = text.lines().collect();
        if cut && !lines.is_empty() {
            // the first line may start in the middle
            lines.remove(0);
        }
        let skip = lines.len().saturating_sub(limit);
        Ok(lines[skip..]
            .iter()
            .map(|line| LogEntry { process: name.clone(), level: Level::Notice, message: line.to_string(), ..LogEntry::default() })
            .collect())
    }
}

/// The readable regular files directly in `/var/log`, by name.
pub fn log_files() -> Vec<PathBuf> {
    if bifrost_app::testing::pinned() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir("/var/log")
        .map(|dir| dir.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && File::open(p).is_ok()).collect())
        .unwrap_or_default();
    files.sort();
    files
}
