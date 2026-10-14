//! View > Run System Diagnostics: collects the machine's state into a folder of text files (kernel counters and
//! pressure from `/proc`, the process list, the disks, the network, the inhibitor locks, the kernel log and the tail of
//! the journal) and zips it. It runs on its own thread, reports progress, and hands the zip back as bytes for the save
//! panel, so the app never needs a path of its own to write to.

use bifrost_app::archive;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;

/// Kernel files copied as they are: the file under `/proc` and the name it gets in the archive.
pub const PROC_FILES: [(&str, &str); 11] = [
    ("loadavg", "loadavg.txt"),
    ("uptime", "uptime.txt"),
    ("version", "version.txt"),
    ("meminfo", "meminfo.txt"),
    ("vmstat", "vmstat.txt"),
    ("stat", "stat.txt"),
    ("diskstats", "diskstats.txt"),
    ("net/dev", "net-dev.txt"),
    ("pressure/cpu", "pressure-cpu.txt"),
    ("pressure/memory", "pressure-memory.txt"),
    ("pressure/io", "pressure-io.txt"),
];

/// Commands whose output is kept: the archive name, the program and its arguments.
pub const COMMANDS: [(&str, &str, &[&str]); 8] = [
    ("processes.txt", "ps", &["auxww"]),
    ("disks.txt", "df", &["-h"]),
    ("block-devices.txt", "lsblk", &[]),
    ("inhibitors.txt", "systemd-inhibit", &["--list", "--no-pager"]),
    ("sockets.txt", "ss", &["-tunap"]),
    ("kernel-log.txt", "dmesg", &[]),
    ("journal-tail.txt", "journalctl", &["-n", "500", "--no-pager"]),
    ("failed-units.txt", "systemctl", &["--failed", "--no-pager"]),
];

/// The output of a command as text. A command that cannot start or fails still gives a line that says why, so the
/// archive shows what was missing.
pub fn capture(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.status.success() {
                text.push_str(&format!("\n[{program} exited with {}: {}]\n", out.status, String::from_utf8_lossy(&out.stderr).trim()));
            }
            text
        }
        Err(e) => format!("[{program} could not run: {e}]\n"),
    }
}

/// Writes the diagnostic files into `dir`. `proc` is `/proc` (or a test directory); `run_commands` false skips the
/// commands. `step(done, total)` is called after each file. Returns the names written.
pub fn gather(dir: &Path, proc: &Path, run_commands: bool, step: &mut dyn FnMut(usize, usize)) -> io::Result<Vec<String>> {
    fs::create_dir_all(dir)?;
    let total = PROC_FILES.len() + if run_commands { COMMANDS.len() } else { 0 } + 1;
    let mut written = Vec::new();
    let mut done = 0;
    for (source, name) in PROC_FILES {
        let text = fs::read_to_string(proc.join(source)).unwrap_or_else(|e| format!("[{source} could not be read: {e}]\n"));
        fs::write(dir.join(name), text)?;
        written.push(name.to_string());
        done += 1;
        step(done, total);
    }
    if run_commands {
        for (name, program, args) in COMMANDS {
            fs::write(dir.join(name), capture(program, args))?;
            written.push(name.to_string());
            done += 1;
            step(done, total);
        }
    }
    let summary = format!("Activity Monitor system diagnostics\nFiles: {}\n", written.join(", "));
    fs::write(dir.join("summary.txt"), summary)?;
    written.push("summary.txt".into());
    step(total, total);
    Ok(written)
}

/// The suggested name of the saved archive.
pub const FILE_NAME: &str = "System Diagnostics.zip";

/// The diagnostics running on their own thread.
pub struct Job {
    progress: Arc<AtomicU32>,
    rx: Receiver<Result<Vec<u8>, String>>,
    /// The finished archive (or the reason it failed); set by `poll`.
    pub result: Option<Result<Vec<u8>, String>>,
}

impl Job {
    /// Starts gathering. The working folder is made under `scratch` and removed when the zip is in memory.
    pub fn start(scratch: PathBuf) -> Job {
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = mpsc::channel();
        let p = progress.clone();
        thread::spawn(move || {
            let _ = tx.send(run(&scratch, &p));
        });
        Job { progress, rx, result: None }
    }

    pub fn fraction(&self) -> f32 {
        if self.result.is_some() {
            return 1.0;
        }
        (self.progress.load(Ordering::Relaxed) as f32 / 1000.0).clamp(0.0, 1.0)
    }

    /// True once the job has finished.
    pub fn poll(&mut self) -> bool {
        if self.result.is_some() {
            return true;
        }
        match self.rx.try_recv() {
            Ok(r) => {
                self.result = Some(r);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.result = Some(Err("The diagnostics stopped unexpectedly".into()));
                true
            }
        }
    }
}

fn run(scratch: &Path, progress: &AtomicU32) -> Result<Vec<u8>, String> {
    let work = scratch.join("diagnostics");
    let out = scratch.join("zip");
    let _ = fs::remove_dir_all(scratch);
    let result = (|| -> io::Result<Vec<u8>> {
        fs::create_dir_all(&out)?;
        gather(&work, Path::new("/proc"), true, &mut |done, total| progress.store((done as f32 / total as f32 * 900.0) as u32, Ordering::Relaxed))?;
        let zip = archive::compress_directory(&work, &out, &mut |_, _| true)?;
        fs::read(zip)
    })();
    let _ = fs::remove_dir_all(scratch);
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("activity-diag-test-{}-{tag}", std::process::id()))
    }

    #[test]
    fn missing_kernel_files_leave_a_note_instead_of_failing() {
        let proc = scratch("proc");
        fs::create_dir_all(&proc).unwrap();
        fs::write(proc.join("loadavg"), "0.10 0.20 0.30 1/100 42\n").unwrap();
        let dir = scratch("out");
        let mut steps = Vec::new();
        let written = gather(&dir, &proc, false, &mut |d, t| steps.push((d, t))).unwrap();
        assert_eq!(written.len(), PROC_FILES.len() + 1);
        assert_eq!(fs::read_to_string(dir.join("loadavg.txt")).unwrap(), "0.10 0.20 0.30 1/100 42\n");
        assert!(fs::read_to_string(dir.join("meminfo.txt")).unwrap().contains("could not be read"));
        assert!(dir.join("summary.txt").exists());
        assert_eq!(steps.last(), Some(&(PROC_FILES.len() + 1, PROC_FILES.len() + 1)));
        let _ = fs::remove_dir_all(&proc);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_that_does_not_exist_is_reported_in_its_file() {
        assert!(capture("definitely-not-a-program-xyz", &[]).contains("could not run"));
        assert_eq!(capture("echo", &["hi"]), "hi\n");
        assert!(capture("false", &[]).contains("exited with"));
    }

    #[test]
    fn the_job_ends_with_a_zip() {
        let mut job = Job::start(scratch("job"));
        for _ in 0..600 {
            if job.poll() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(50));
        }
        match job.result.as_ref().expect("finished") {
            Ok(bytes) => assert_eq!(&bytes[..2], b"PK"),
            Err(e) => panic!("{e}"),
        }
        assert_eq!(job.fraction(), 1.0);
    }
}
