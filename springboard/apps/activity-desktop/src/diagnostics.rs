//! View > Run System Diagnostics: gathers what a support request needs (kernel counters from `/proc`, the process
//! list as the monitor last saw it, the tail of the system journal, the kernel log and the disk usage) into a folder,
//! zips it in the app's storage and hands the archive to the window, which asks where to save it. Every source that is
//! not readable is named in `summary.txt` instead of failing the run. The gathering is pure file and process work, so
//! it runs on the job thread.

use crate::job::{Outcome, Progress};
use crate::sample::ProcSample;
use crate::sample_proc;
use bifrost_app::archive;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Most bytes kept of one file or one command's output.
const MAX_BYTES: usize = 512 * 1024;
/// How long one command may run before it is stopped.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// The `/proc` files copied as they are, by path under `/proc`.
pub const PROC_FILES: [&str; 13] =
    ["stat", "meminfo", "loadavg", "uptime", "vmstat", "diskstats", "swaps", "version", "cmdline", "net/dev", "pressure/cpu", "pressure/memory", "pressure/io"];

/// One command whose output goes into a file. The alternatives are tried in order; the first that works is used.
pub struct Capture {
    pub file: &'static str,
    pub commands: &'static [(&'static str, &'static [&'static str])],
}

pub const CAPTURES: [Capture; 4] = [
    Capture { file: "journal.txt", commands: &[("journalctl", &["-b", "-n", "500", "--no-pager"])] },
    Capture { file: "kernel-log.txt", commands: &[("dmesg", &[]), ("journalctl", &["-k", "-b", "-n", "300", "--no-pager"])] },
    Capture { file: "disk-usage.txt", commands: &[("df", &["-h"])] },
    Capture { file: "failed-units.txt", commands: &[("systemctl", &["--failed", "--no-pager"])] },
];

/// The file name a `/proc` path gets in the bundle: "proc-net-dev.txt".
pub fn proc_file_name(path: &str) -> String {
    format!("proc-{}.txt", path.replace('/', "-"))
}

/// Cuts text to `MAX_BYTES` at a character boundary and says so.
pub fn cap(mut text: String) -> String {
    if text.len() > MAX_BYTES {
        let mut cut = MAX_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[cut at 512 KiB]\n");
    }
    text
}

/// The process list as the monitor last read it, one line per process in pid order, tab separated.
pub fn format_processes(procs: &[ProcSample]) -> String {
    let mut rows: Vec<&ProcSample> = procs.iter().collect();
    rows.sort_by_key(|p| p.pid);
    let mut out = String::from("PID\tPPID\tUSER\t%CPU\tRSS KB\tTHREADS\tNAME\n");
    for p in rows {
        out.push_str(&format!("{}\t{}\t{}\t{:.1}\t{}\t{}\t{}\n", p.pid, p.ppid, p.user, p.cpu, p.rss / 1024, p.threads, p.name));
    }
    out
}

/// The summary file: when it was made, what is in the bundle and what could not be read.
pub fn summary(stamp: u64, included: &[String], missing: &[String]) -> String {
    let mut out = format!("System diagnostics\nGathered at Unix time {stamp}\n\nIncluded:\n");
    for name in included {
        out.push_str(&format!("  {name}\n"));
    }
    if missing.is_empty() {
        out.push_str("\nEverything asked for was readable.\n");
    } else {
        out.push_str("\nNot available:\n");
        for note in missing {
            out.push_str(&format!("  {note}\n"));
        }
    }
    out
}

/// Runs a command and returns what it printed, giving up after `timeout`. Output stops being read at `MAX_BYTES`. A
/// command that fails but printed something still counts; one that printed nothing is an error.
pub fn run_capture(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{program}: no output"));
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    let status: Result<std::process::ExitStatus, String> = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("{program} timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(format!("{program}: {e}")),
        }
    };
    let text = cap(String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned());
    match status {
        Ok(s) if s.success() || !text.is_empty() => Ok(text),
        Ok(s) => Err(format!("{program} exited with {s}")),
        Err(e) => Err(e),
    }
}

/// Gathers the bundle inside `dir` and returns the finished zip. `procs` is the monitor's latest process list.
/// Progress counts the sources plus the final zip.
pub fn gather(dir: &Path, procs: &[ProcSample], progress: &Progress) -> Outcome {
    if dir.as_os_str().is_empty() {
        return Outcome::Failed("The app has no storage folder to prepare the diagnostics in".into());
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stage = dir.join(format!("System Diagnostics {stamp}"));
    let _ = fs::remove_dir_all(&stage);
    if let Err(e) = fs::create_dir_all(&stage) {
        return Outcome::Failed(format!("Could not prepare the diagnostics folder: {e}"));
    }
    // the /proc files, the process list, the system sample, the captures, and the zip
    let total = (PROC_FILES.len() + CAPTURES.len() + 3) as u32;
    let mut done = 0u32;
    let mut included = Vec::new();
    let mut missing = Vec::new();
    let store = |name: &str, text: String, included: &mut Vec<String>, missing: &mut Vec<String>| match fs::write(stage.join(name), text) {
        Ok(()) => included.push(name.to_string()),
        Err(e) => missing.push(format!("{name}: could not be written ({e})")),
    };
    let abandon = |stage: &Path| {
        let _ = fs::remove_dir_all(stage);
        Outcome::Cancelled
    };

    for path in PROC_FILES {
        if progress.cancelled() {
            return abandon(&stage);
        }
        match fs::read_to_string(format!("/proc/{path}")) {
            Ok(text) => store(&proc_file_name(path), cap(text), &mut included, &mut missing),
            Err(e) => missing.push(format!("/proc/{path}: {e}")),
        }
        done += 1;
        progress.set(done, total);
    }
    store("processes.txt", format_processes(procs), &mut included, &mut missing);
    done += 1;
    progress.set(done, total);

    // a short look at every thread of every process, what Run Spindump gives the reference app
    let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
    let sample = sample_proc::sample_system(&pids, sample_proc::SYSTEM_ROUNDS, sample_proc::SYSTEM_INTERVAL_MS, progress);
    if progress.cancelled() {
        return abandon(&stage);
    }
    store("system-sample.txt", cap(sample.render_system(sample_proc::SYSTEM_INTERVAL_MS)), &mut included, &mut missing);
    done += 1;
    progress.set(done, total);

    for capture in CAPTURES.iter() {
        if progress.cancelled() {
            return abandon(&stage);
        }
        let mut failures = Vec::new();
        let mut output = None;
        for (program, args) in capture.commands {
            match run_capture(program, args, COMMAND_TIMEOUT) {
                Ok(text) => {
                    output = Some(text);
                    break;
                }
                Err(e) => failures.push(e),
            }
        }
        match output {
            Some(text) => store(capture.file, text, &mut included, &mut missing),
            None => missing.push(format!("{}: {}", capture.file, failures.join("; "))),
        }
        done += 1;
        progress.set(done, total);
    }

    store("summary.txt", summary(stamp, &included, &missing), &mut included, &mut missing);
    let zipped = archive::compress_directory(&stage, dir, &mut |_, _| !progress.cancelled());
    let _ = fs::remove_dir_all(&stage);
    if progress.cancelled() {
        return Outcome::Cancelled;
    }
    match zipped {
        Ok(path) => {
            progress.set(total, total);
            Outcome::Bundle(path)
        }
        Err(e) => Outcome::Failed(format!("Could not create the diagnostics archive: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_paths_become_flat_file_names() {
        assert_eq!(proc_file_name("meminfo"), "proc-meminfo.txt");
        assert_eq!(proc_file_name("net/dev"), "proc-net-dev.txt");
        assert_eq!(proc_file_name("pressure/cpu"), "proc-pressure-cpu.txt");
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary_and_marked() {
        assert_eq!(cap("short".into()), "short");
        let long = "é".repeat(MAX_BYTES);
        let cut = cap(long);
        assert!(cut.ends_with("[cut at 512 KiB]\n"));
        assert!(cut.len() <= MAX_BYTES + 32);
    }

    #[test]
    fn the_process_list_is_in_pid_order_with_a_header() {
        let mk = |pid: u32, name: &str| ProcSample { pid, ppid: 1, name: name.into(), user: "me".into(), cpu: 2.5, rss: 4096 * 3, threads: 4, ..ProcSample::default() };
        let text = format_processes(&[mk(30, "b"), mk(10, "a")]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "PID\tPPID\tUSER\t%CPU\tRSS KB\tTHREADS\tNAME");
        assert_eq!(lines[1], "10\t1\tme\t2.5\t12\t4\ta");
        assert_eq!(lines[2], "30\t1\tme\t2.5\t12\t4\tb");
    }

    #[test]
    fn the_summary_names_what_was_missed() {
        let ok = summary(5, &["a.txt".into()], &[]);
        assert!(ok.contains("Gathered at Unix time 5"));
        assert!(ok.contains("  a.txt\n"));
        assert!(ok.contains("Everything asked for was readable."));
        let partial = summary(5, &["a.txt".into()], &["dmesg: denied".into()]);
        assert!(partial.contains("Not available:\n  dmesg: denied\n"));
    }

    #[test]
    fn commands_return_their_output() {
        assert_eq!(run_capture("sh", &["-c", "echo hi"], Duration::from_secs(5)).unwrap(), "hi\n");
    }

    #[test]
    fn a_failing_command_with_output_still_counts_and_one_without_does_not() {
        assert_eq!(run_capture("sh", &["-c", "echo partial; exit 3"], Duration::from_secs(5)).unwrap(), "partial\n");
        assert!(run_capture("sh", &["-c", "exit 3"], Duration::from_secs(5)).is_err());
    }

    #[test]
    fn a_slow_command_is_stopped() {
        let err = run_capture("sleep", &["5"], Duration::from_millis(100)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }

    #[test]
    fn a_missing_program_is_an_error() {
        assert!(run_capture("no-such-program-for-the-monitor", &[], Duration::from_secs(1)).is_err());
    }

    #[test]
    fn without_a_storage_folder_the_run_fails_at_once() {
        let out = gather(Path::new(""), &[], &Progress::default());
        assert!(matches!(out, Outcome::Failed(_)));
    }

    #[test]
    fn a_stop_request_ends_the_run_without_a_bundle() {
        let dir = std::env::current_dir().unwrap().join("target").join("activity-diagnostics-test");
        fs::create_dir_all(&dir).unwrap();
        let progress = Progress::default();
        progress.request_stop();
        assert_eq!(gather(&dir, &[], &progress), Outcome::Cancelled);
        let _ = fs::remove_dir_all(&dir);
    }
}
