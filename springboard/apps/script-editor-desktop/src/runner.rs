//! Runs a script through the system shell (`/bin/sh -c`). The shell runs on a worker thread, so the window keeps
//! drawing while a script works; the result comes back over a channel that the app polls each tick.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use bifrost_app::dispatch::{DispatchQueue, Qos};

/// The shell every script runs in.
pub const SHELL: &str = "/bin/sh";

/// Output kept from each stream. Longer output is cut and marked, so a runaway script cannot fill the memory.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

const TRUNCATED: &str = "[output truncated]";

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The shell exited with this status.
    Exited(i32),
    /// The shell was ended by a signal (for example after Stop).
    Stopped,
    /// The shell could not be started or waited for.
    Failed(String),
}

/// What a finished run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub stdout: String,
    pub stderr: String,
    pub outcome: Outcome,
}

impl Report {
    fn failed(message: String) -> Report {
        Report { stdout: String::new(), stderr: String::new(), outcome: Outcome::Failed(message) }
    }
}

/// A run in progress. Dropping it does not stop the script; call `stop` first to end it.
pub struct Run {
    rx: Receiver<Report>,
    pid: Arc<AtomicU32>,
}

impl Run {
    /// Starts `script` in the shell with `cwd` as its working directory.
    pub fn start(script: &str, cwd: PathBuf) -> Run {
        let (tx, rx) = mpsc::channel();
        let pid = Arc::new(AtomicU32::new(0));
        let worker_pid = Arc::clone(&pid);
        let script = script.to_string();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let report = work(&script, &cwd, &worker_pid);
            DispatchQueue::main().async_(move || {
                let _ = tx.send(report);
            });
        });
        Run { rx, pid }
    }

    /// The report, once the run has finished. Returns `None` while it is still going.
    pub fn poll(&self) -> Option<Report> {
        self.rx.try_recv().ok()
    }

    /// Asks the running shell to end (SIGTERM). False when no process is running yet or it could not be signalled.
    pub fn stop(&self) -> bool {
        let pid = self.pid.load(Ordering::SeqCst);
        if pid == 0 {
            return false;
        }
        Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn work(script: &str, cwd: &PathBuf, pid: &AtomicU32) -> Report {
    let spawned = Command::new(SHELL)
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let report = match spawned {
        Ok(child) => {
            pid.store(child.id(), Ordering::SeqCst);
            let waited = child.wait_with_output();
            pid.store(0, Ordering::SeqCst);
            match waited {
                Ok(out) => Report {
                    stdout: clip(&out.stdout),
                    stderr: clip(&out.stderr),
                    outcome: match out.status.code() {
                        Some(code) => Outcome::Exited(code),
                        None => Outcome::Stopped,
                    },
                },
                Err(e) => Report::failed(format!("Could not read the output: {e}")),
            }
        }
        Err(e) => Report::failed(format!("Could not start {SHELL}: {e}")),
    };
    report
}

fn clip(bytes: &[u8]) -> String {
    truncate_output(&String::from_utf8_lossy(bytes), MAX_OUTPUT_BYTES)
}

/// `text` cut to at most `max` bytes at a character boundary, with a marker line when anything was cut.
pub fn truncate_output(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n{TRUNCATED}", &text[..end])
}

/// The one-line description of how a run ended, as the status bar shows it.
pub fn status_line(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Exited(code) => format!("Exited with status {code}"),
        Outcome::Stopped => "Stopped".to_string(),
        Outcome::Failed(message) => message.clone(),
    }
}

/// The lines of a report in order: standard output first, then standard error. Each line says whether it is an
/// error line, so the output pane can colour it.
pub fn output_lines(report: &Report) -> Vec<(bool, String)> {
    let mut lines: Vec<(bool, String)> = report.stdout.lines().map(|l| (false, l.to_string())).collect();
    lines.extend(report.stderr.lines().map(|l| (true, l.to_string())));
    lines
}

/// The directory a script starts in: the home directory, or the root when there is none.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(stdout: &str, stderr: &str, outcome: Outcome) -> Report {
        Report { stdout: stdout.into(), stderr: stderr.into(), outcome }
    }

    #[test]
    fn truncation_keeps_short_output_and_marks_long_output() {
        assert_eq!(truncate_output("short", 10), "short");
        assert_eq!(truncate_output("abcdef", 3), format!("abc\n{TRUNCATED}"));
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // "é" is two bytes: cutting at byte 1 must back off to the start of the character
        assert_eq!(truncate_output("é", 1), format!("\n{TRUNCATED}"));
        assert_eq!(truncate_output("aé", 2), format!("a\n{TRUNCATED}"));
    }

    #[test]
    fn status_lines_describe_each_outcome() {
        assert_eq!(status_line(&Outcome::Exited(0)), "Exited with status 0");
        assert_eq!(status_line(&Outcome::Exited(3)), "Exited with status 3");
        assert_eq!(status_line(&Outcome::Stopped), "Stopped");
        assert_eq!(status_line(&Outcome::Failed("no shell".into())), "no shell");
    }

    #[test]
    fn output_lines_list_standard_output_before_errors() {
        let r = report("one\ntwo\n", "bad\n", Outcome::Exited(1));
        assert_eq!(
            output_lines(&r),
            vec![(false, "one".to_string()), (false, "two".to_string()), (true, "bad".to_string())]
        );
        assert!(output_lines(&report("", "", Outcome::Exited(0))).is_empty());
    }

    #[test]
    fn home_dir_is_always_a_directory() {
        assert!(home_dir().is_dir());
    }
}
