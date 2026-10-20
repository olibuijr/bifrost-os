//! File > Capture Archive: saves this boot's journal in the journal export format to the file the user chose in the
//! save panel, and reads the same stream into entries so the archive opens at once. The work runs on a utility
//! queue; the window polls `Capture::take` from its tick and commits or aborts the save. The command's error output is
//! read on its own thread (a full pipe never stalls it), and `Capture::cancel` kills the command.

use crate::model::LogEntry;
use crate::opened::ExportParser;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::fs::File;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

/// The most of the command's error output that is kept for the message; the rest is read and dropped.
const STDERR_KEEP: usize = 16 << 10;

/// The name suggested in the save panel for a capture made at `unix_secs`.
pub fn file_name(unix_secs: i64) -> String {
    format!("Console-{unix_secs}.logarchive")
}

/// The command whose output the archive holds: this boot, in the export format.
pub const COMMAND: [&str; 5] = ["journalctl", "-b", "-o", "export", "--no-pager"];

/// What `Capture::cancel` reaches while the command runs: the cancel flag and the child to kill.
#[derive(Default)]
struct Control {
    cancelled: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl Control {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

/// A capture in flight, for the save request `request`.
pub struct Capture {
    pub request: u32,
    result: Arc<Mutex<Option<Result<Vec<LogEntry>, String>>>>,
    control: Arc<Control>,
}

impl Capture {
    /// Starts the capture into `file`.
    pub fn start(request: u32, file: File) -> Capture {
        let result = Arc::new(Mutex::new(None));
        let slot = result.clone();
        let control = Arc::new(Control::default());
        let running = control.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let done = run(&COMMAND, file, &running);
            if let Ok(mut s) = slot.lock() {
                *s = Some(done);
            }
            // the result is news: wake the window so its tick sees it
            DispatchQueue::main().async_(|| {});
        });
        Capture { request, result, control }
    }

    /// Stops the capture: the command is killed and the result is an error saying so. Harmless when it has finished.
    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// The finished capture's entries, or its error. None while it still runs.
    pub fn take(&self) -> Option<Result<Vec<LogEntry>, String>> {
        self.result.lock().ok()?.take()
    }
}

impl Drop for Capture {
    /// A capture that is dropped (the window closed, the save abandoned) does not leave the command running.
    fn drop(&mut self) {
        self.control.cancel();
    }
}

/// Reads the command's error output on its own thread: the first `STDERR_KEEP` bytes are kept, the rest is read and
/// dropped, so the pipe never fills and stops the command while the standard output is still being copied.
fn drain_stderr(pipe: Option<std::process::ChildStderr>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = channel();
    // compliance-allow: RES-01 a pool worker waiting on another pool job can starve at the width cap; the reader must always run
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let room = STDERR_KEEP.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
        let _ = tx.send(kept);
    });
    rx
}

/// Runs `command`, copying its output to `file` while parsing it as journal export records. `control.cancel` ends it.
fn run(command: &[&str], mut file: File, control: &Control) -> Result<Vec<LogEntry>, String> {
    let mut child = Command::new(command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", command[0]))?;
    let mut stdout = child.stdout.take().ok_or("journalctl: no output pipe")?;
    let stderr = drain_stderr(child.stderr.take());
    if let Ok(mut slot) = control.child.lock() {
        *slot = Some(child);
    }
    // a cancel that came before the child was stored still ends it
    if control.cancelled.load(Ordering::Relaxed) {
        control.cancel();
    }
    let mut parser = ExportParser::new();
    let mut chunk = vec![0u8; 64 << 10];
    let copied = loop {
        if control.cancelled.load(Ordering::Relaxed) {
            break Err("The capture was cancelled.".to_string());
        }
        match stdout.read(&mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                if let Err(e) = file.write_all(&chunk[..n]) {
                    break Err(format!("Could not write the archive: {e}"));
                }
                parser.feed(&chunk[..n]);
            }
            Err(e) => break Err(format!("{}: {e}", command[0])),
        }
    };
    let child = control.child.lock().ok().and_then(|mut slot| slot.take());
    let Some(mut child) = child else { return Err("The capture was cancelled.".to_string()) };
    if copied.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| format!("{}: {e}", command[0]))?;
    copied?;
    if control.cancelled.load(Ordering::Relaxed) {
        return Err("The capture was cancelled.".to_string());
    }
    if !status.success() {
        let kept = stderr.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or_default();
        let err = String::from_utf8_lossy(&kept).trim().to_string();
        return Err(if err.is_empty() { format!("{}: exit status {}", command[0], status) } else { err });
    }
    file.flush().map_err(|e| format!("Could not write the archive: {e}"))?;
    Ok(parser.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("console-capture-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_suggested_name_carries_the_capture_time() {
        assert_eq!(file_name(1_700_000_000), "Console-1700000000.logarchive");
    }

    #[test]
    fn the_capture_command_reads_this_boot_in_the_export_format() {
        assert_eq!(COMMAND, ["journalctl", "-b", "-o", "export", "--no-pager"]);
    }

    #[test]
    fn output_is_saved_to_the_file_and_read_as_entries() {
        let dir = scratch("run");
        let path = dir.join("out.logarchive");
        let file = File::create(&path).unwrap();
        let script = "printf '__CURSOR=s=1\\n__REALTIME_TIMESTAMP=7\\nMESSAGE=hello\\n\\n'";
        let entries = run(&["sh", "-c", script], file, &Control::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].message.as_str(), entries[0].date), ("hello", 7));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "__CURSOR=s=1\n__REALTIME_TIMESTAMP=7\nMESSAGE=hello\n\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_command_reports_its_message() {
        let dir = scratch("fail");
        let file = File::create(dir.join("out")).unwrap();
        let err = run(&["sh", "-c", "echo 'no journal access' >&2; exit 3"], file, &Control::default()).unwrap_err();
        assert_eq!(err, "no journal access");
        let missing = File::create(dir.join("out2")).unwrap();
        assert!(run(&["console-no-such-command"], missing, &Control::default()).unwrap_err().contains("console-no-such-command"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_that_fills_its_error_pipe_does_not_stall() {
        let dir = scratch("stderr");
        let path = dir.join("out");
        let file = File::create(&path).unwrap();
        // 300 KB on stderr (far more than a pipe holds) while the output is still being written
        let script = "head -c 300000 /dev/zero >&2; printf '__CURSOR=s=1\\n__REALTIME_TIMESTAMP=7\\nMESSAGE=hi\\n\\n'; head -c 300000 /dev/zero >&2";
        let entries = run(&["sh", "-c", script], file, &Control::default()).unwrap();
        assert_eq!(entries.len(), 1);
        let failing = File::create(dir.join("out2")).unwrap();
        let err = run(&["sh", "-c", "head -c 300000 /dev/zero | tr '\\0' 'e' >&2; exit 2"], failing, &Control::default()).unwrap_err();
        assert!(err.len() <= STDERR_KEEP, "only the start of the error output is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancel_kills_the_command_and_ends_the_capture() {
        let dir = scratch("cancel");
        let file = File::create(dir.join("out")).unwrap();
        let control = Arc::new(Control::default());
        let c = control.clone();
        // compliance-allow: RES-01 the test cancels from outside the running capture
        let killer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            c.cancel();
        });
        let started = std::time::Instant::now();
        let err = run(&["sh", "-c", "exec sleep 30"], file, &control).unwrap_err();
        killer.join().unwrap();
        assert_eq!(err, "The capture was cancelled.");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
