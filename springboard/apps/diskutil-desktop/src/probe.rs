//! The reads that need a process or the mount table: `/proc/mounts`, `df` for usage, and `smartctl -H` for SMART
//! health. Each runs a fixed program with fixed arguments; device and mount point names are passed as separate
//! arguments, never through a shell.
//!
//! The tools can stall (a mounted disk whose `statfs` hangs, a USB bridge where `smartctl` takes a long time), so each
//! runs with a time limit and is killed when it passes it, and the window runs them on a worker thread (`spawn`) and
//! takes the answers as `Done` values.

use crate::model::{parse_df, parse_lsblk_pairs, parse_smart, Usage};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

/// How long `df`, `smartctl` and `lsblk` may run before they are killed and the answer is "not available".
pub const DF_LIMIT: Duration = Duration::from_secs(8);
pub const SMART_LIMIT: Duration = Duration::from_secs(15);
pub const LSBLK_LIMIT: Duration = Duration::from_secs(5);
/// The most output read from one of the tools.
const MAX_OUTPUT: u64 = 1 << 20;

/// One finished probe, sent from the worker to the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    /// Usage by mount point; empty when `df` failed or ran out of time.
    Usage(Vec<(String, Usage)>),
    /// The SMART health sentence of a disk.
    Smart(String, String),
    /// The `lsblk` pairs of a device (empty when the tool failed).
    Lsblk(String, Vec<(String, String)>),
}

/// Runs `work` on a utility queue and sends what it returns, then wakes the app so the window takes it.
pub fn spawn(tx: Sender<Done>, work: impl FnOnce() -> Done + Send + 'static) {
    DispatchQueue::global(Qos::Utility).async_(move || {
        let _ = tx.send(work());
        DispatchQueue::main().async_(|| {});
    });
}

/// Runs `command` with no input and returns its standard output as text, or None when it could not start, or ran past
/// `limit` (it is then killed). The output is read on its own thread, so a process that never exits cannot hold the
/// caller; the child is reaped on another thread.
pub fn run_limited(command: &mut Command, limit: Duration) -> Option<String> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let Some(out) = child.stdout.take() else {
        let _ = child.kill();
        // compliance-allow: RES-01 a pool worker waiting on another pool job can starve at the width cap; a stalled tool must not hold one
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return None;
    };
    let (tx, rx) = channel();
    // compliance-allow: RES-01 a pool worker waiting on another pool job can starve at the width cap; a stalled tool must not hold one
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.take(MAX_OUTPUT).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let got = rx.recv_timeout(limit).ok();
    if got.is_none() {
        let _ = child.kill();
    }
    // compliance-allow: RES-01 a pool worker waiting on another pool job can starve at the width cap; a stalled tool must not hold one
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    got.map(|buf| String::from_utf8_lossy(&buf).into_owned())
}

/// The mount table as text. Empty when it cannot be read.
pub fn read_mounts() -> String {
    std::fs::read_to_string("/proc/mounts").unwrap_or_default()
}

/// Usage for the given mount points, from `df`. Paths that do not start with `/` are dropped, so an option can never
/// be passed where a path is expected.
pub fn read_df(points: &[String]) -> Vec<(String, Usage)> {
    let paths: Vec<&String> = points.iter().filter(|p| p.starts_with('/')).collect();
    if paths.is_empty() {
        return Vec::new();
    }
    match run_limited(Command::new("df").args(["-B1", "--output=target,size,used,avail", "--"]).args(&paths), DF_LIMIT) {
        Some(text) => parse_df(&text),
        None => Vec::new(),
    }
}

/// Every field `lsblk` knows about one device, as `KEY="value"` pairs (empty when the tool is missing or the name is
/// not a plain device name). Get Info shows the ones the scan of `/sys` does not read.
pub fn read_lsblk(name: &str) -> Vec<(String, String)> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Vec::new();
    }
    match run_limited(Command::new("lsblk").args(["-d", "-P", "-O"]).arg(format!("/dev/{name}")), LSBLK_LIMIT) {
        Some(text) => parse_lsblk_pairs(&text),
        None => Vec::new(),
    }
}

/// The SMART overall health of a disk, or `Not available` when the tool is missing or the drive reports nothing.
pub fn read_smart(name: &str) -> String {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return "Not available".to_string();
    }
    match run_limited(Command::new("smartctl").arg("-H").arg(format!("/dev/{name}")), SMART_LIMIT) {
        Some(text) => parse_smart(&text).unwrap_or_else(|| "Not available".to_string()),
        None => "Not available".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_that_finishes_gives_its_output() {
        let text = run_limited(Command::new("echo").arg("hello"), Duration::from_secs(5));
        assert_eq!(text.as_deref(), Some("hello\n"));
    }

    #[test]
    fn a_tool_that_runs_past_the_limit_is_given_up_on() {
        let started = std::time::Instant::now();
        let text = run_limited(Command::new("sleep").arg("30"), Duration::from_millis(200));
        assert!(text.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_tool_is_not_available() {
        assert!(run_limited(&mut Command::new("bifrost-no-such-tool"), Duration::from_secs(1)).is_none());
        assert_eq!(read_smart("not a name"), "Not available");
    }
}
