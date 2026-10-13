//! File > Capture Archive: saves this boot's journal in the journal export format to the file the user chose in the
//! save panel, and reads the same stream into entries so the archive opens at once. The work runs on a utility
//! queue; the window polls `Capture::take` from its tick and commits or aborts the save.

use crate::model::LogEntry;
use crate::opened::ExportParser;
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::fs::File;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// The name suggested in the save panel for a capture made at `unix_secs`.
pub fn file_name(unix_secs: i64) -> String {
    format!("Console-{unix_secs}.logarchive")
}

/// The command whose output the archive holds: this boot, in the export format.
pub const COMMAND: [&str; 5] = ["journalctl", "-b", "-o", "export", "--no-pager"];

/// A capture in flight, for the save request `request`.
pub struct Capture {
    pub request: u32,
    result: Arc<Mutex<Option<Result<Vec<LogEntry>, String>>>>,
}

impl Capture {
    /// Starts the capture into `file`.
    pub fn start(request: u32, file: File) -> Capture {
        let result = Arc::new(Mutex::new(None));
        let slot = result.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let done = run(&COMMAND, file);
            if let Ok(mut s) = slot.lock() {
                *s = Some(done);
            }
            // the result is news: wake the window so its tick sees it
            DispatchQueue::main().async_(|| {});
        });
        Capture { request, result }
    }

    /// The finished capture's entries, or its error. None while it still runs.
    pub fn take(&self) -> Option<Result<Vec<LogEntry>, String>> {
        self.result.lock().ok()?.take()
    }
}

/// Runs `command`, copying its output to `file` while parsing it as journal export records.
fn run(command: &[&str], mut file: File) -> Result<Vec<LogEntry>, String> {
    let mut child = Command::new(command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", command[0]))?;
    let mut stdout = child.stdout.take().ok_or("journalctl: no output pipe")?;
    let mut parser = ExportParser::new();
    let mut chunk = vec![0u8; 64 << 10];
    let copied = loop {
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
    if copied.is_err() {
        let _ = child.kill();
    }
    let out = child.wait_with_output().map_err(|e| format!("{}: {e}", command[0]))?;
    copied?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { format!("{}: exit status {}", command[0], out.status) } else { err });
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
        let entries = run(&["sh", "-c", script], file).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].message.as_str(), entries[0].date), ("hello", 7));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "__CURSOR=s=1\n__REALTIME_TIMESTAMP=7\nMESSAGE=hello\n\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_command_reports_its_message() {
        let dir = scratch("fail");
        let file = File::create(dir.join("out")).unwrap();
        let err = run(&["sh", "-c", "echo 'no journal access' >&2; exit 3"], file).unwrap_err();
        assert_eq!(err, "no journal access");
        let missing = File::create(dir.join("out2")).unwrap();
        assert!(run(&["console-no-such-command"], missing).unwrap_err().contains("console-no-such-command"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
