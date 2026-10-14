//! Process > Sample Process: where each thread of a process waits, taken many times over a few seconds and summed up
//! as a text report. The kernel names the place a thread sleeps in (`task/<tid>/stack` where the account may read it,
//! otherwise `wchan`), so the report shows kernel waits, not user code. A sample runs on its own thread and reports
//! its progress; the window polls it.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// The reference app samples for ten seconds, once every ten milliseconds; a reading here walks every thread's
/// `/proc` files, so the default interval is twenty milliseconds.
pub const DEFAULT_SECONDS: u32 = 10;
pub const DEFAULT_INTERVAL_MS: u32 = 20;

/// One thread at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadSample {
    pub tid: u32,
    pub name: String,
    pub state: char,
    /// The kernel function the thread waits in (`wchan`), empty when it is running.
    pub wchan: String,
    /// The kernel stack, innermost frame first, when the account may read it.
    pub stack: Vec<String>,
}

impl ThreadSample {
    /// The frames the report lists: the stack, or the wait channel alone, or nothing for a running thread.
    pub fn frames(&self) -> Vec<String> {
        if !self.stack.is_empty() {
            self.stack.clone()
        } else if !self.wchan.is_empty() {
            vec![self.wchan.clone()]
        } else {
            Vec::new()
        }
    }
}

/// A kernel stack line (`[<0>] futex_wait_queue+0x6d/0x90`) as the function name.
pub fn parse_stack_line(line: &str) -> Option<String> {
    let rest = line.split_once("] ").map_or(line, |(_, r)| r).trim();
    let name = rest.split('+').next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The kernel stack text as frames.
pub fn parse_stack(text: &str) -> Vec<String> {
    text.lines().filter_map(parse_stack_line).collect()
}

/// `wchan` holds `0` (or nothing) when the thread is not waiting in the kernel.
pub fn parse_wchan(text: &str) -> String {
    let t = text.trim();
    if t == "0" {
        String::new()
    } else {
        t.to_string()
    }
}

/// Reads every thread of `pid` once under `proc` (`/proc`, or a test directory).
pub fn collect_once(proc: &Path, pid: u32) -> Vec<ThreadSample> {
    let tasks = proc.join(pid.to_string()).join("task");
    let Ok(entries) = fs::read_dir(&tasks) else { return Vec::new() };
    let mut tids: Vec<u32> = entries.flatten().filter_map(|e| e.file_name().to_string_lossy().parse().ok()).collect();
    tids.sort_unstable();
    tids.into_iter()
        .map(|tid| {
            let dir = tasks.join(tid.to_string());
            let stat = fs::read_to_string(dir.join("stat")).unwrap_or_default();
            let (name, state) = match (stat.find('('), stat.rfind(')')) {
                (Some(open), Some(close)) if close > open => (stat[open + 1..close].to_string(), stat[close + 1..].trim_start().chars().next().unwrap_or('?')),
                _ => (String::new(), '?'),
            };
            ThreadSample {
                tid,
                name,
                state,
                wchan: fs::read_to_string(dir.join("wchan")).map(|t| parse_wchan(&t)).unwrap_or_default(),
                stack: fs::read_to_string(dir.join("stack")).map(|t| parse_stack(&t)).unwrap_or_default(),
            }
        })
        .collect()
}

/// What a sample was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub seconds: u32,
    pub interval_ms: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options { seconds: DEFAULT_SECONDS, interval_ms: DEFAULT_INTERVAL_MS }
    }
}

/// Sums the samples into the report: per thread, how often it was in each distinct stack, most frequent first.
pub fn build_report(pid: u32, name: &str, options: Options, samples: &[Vec<ThreadSample>]) -> String {
    let mut out = format!(
        "Sampling process {pid} ({name}) for {} seconds with {} millisecond intervals\nSamples taken: {}\n\n",
        options.seconds,
        options.interval_ms,
        samples.len()
    );
    // tid -> (name, stack -> (state, count))
    let mut threads: BTreeMap<u32, (String, BTreeMap<Vec<String>, (char, usize)>)> = BTreeMap::new();
    for round in samples {
        for t in round {
            let entry = threads.entry(t.tid).or_insert_with(|| (t.name.clone(), BTreeMap::new()));
            let slot = entry.1.entry(t.frames()).or_insert((t.state, 0));
            slot.1 += 1;
        }
    }
    if threads.is_empty() {
        out.push_str("The process exited before it could be sampled.\n");
        return out;
    }
    for (tid, (tname, stacks)) in threads {
        let total: usize = stacks.values().map(|v| v.1).sum();
        out.push_str(&format!("Thread {tid} \"{tname}\"  ({total} samples)\n"));
        let mut list: Vec<(&Vec<String>, &(char, usize))> = stacks.iter().collect();
        list.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(b.0)));
        for (frames, (state, count)) in list {
            out.push_str(&format!("  {count}  {}\n", crate::model::state_name(*state)));
            if frames.is_empty() {
                out.push_str("      (running)\n");
            }
            for frame in frames {
                out.push_str(&format!("      {frame}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// A sample in progress on its own thread.
pub struct Job {
    pub pid: u32,
    pub name: String,
    pub options: Options,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<String>,
    /// The finished report; set by `poll`.
    pub report: Option<String>,
}

impl Job {
    /// Starts sampling `pid` under `proc`.
    pub fn start(proc: &Path, pid: u32, name: &str, options: Options) -> Job {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (p, c, root, label) = (progress.clone(), cancel.clone(), proc.to_path_buf(), name.to_string());
        thread::spawn(move || {
            let started = Instant::now();
            let total = Duration::from_secs(options.seconds as u64);
            let step = Duration::from_millis(options.interval_ms.max(1) as u64);
            let mut samples = Vec::new();
            while started.elapsed() < total && !c.load(Ordering::Relaxed) {
                let round = collect_once(&root, pid);
                if round.is_empty() {
                    break;
                }
                samples.push(round);
                p.store((started.elapsed().as_secs_f32() / total.as_secs_f32().max(0.001) * 1000.0) as u32, Ordering::Relaxed);
                thread::sleep(step);
            }
            let _ = tx.send(build_report(pid, &label, options, &samples));
        });
        Job { pid, name: name.to_string(), options, progress, cancel, rx, report: None }
    }

    /// Progress from 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.report.is_some() {
            return 1.0;
        }
        (self.progress.load(Ordering::Relaxed) as f32 / 1000.0).clamp(0.0, 1.0)
    }

    /// Stops sampling early; the report covers what was taken.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Takes the report if the sample has finished. True once it has.
    pub fn poll(&mut self) -> bool {
        if self.report.is_some() {
            return true;
        }
        match self.rx.try_recv() {
            Ok(text) => {
                self.report = Some(text);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.report = Some("Sampling stopped unexpectedly.\n".into());
                true
            }
        }
    }
}

/// The file name the report is saved under.
pub fn file_name(name: &str, pid: u32) -> String {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("Sample of {safe} ({pid}).txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(tid: u32, state: char, stack: &[&str]) -> ThreadSample {
        ThreadSample { tid, name: format!("t{tid}"), state, wchan: String::new(), stack: stack.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn stack_lines_become_function_names() {
        assert_eq!(parse_stack_line("[<0>] futex_wait_queue+0x6d/0x90").as_deref(), Some("futex_wait_queue"));
        assert_eq!(parse_stack_line("do_syscall_64+0x5b/0xa0").as_deref(), Some("do_syscall_64"));
        assert_eq!(parse_stack_line("   ").as_deref(), None);
        assert_eq!(parse_stack("[<0>] a+0x1/0x2\n[<0>] b+0x3/0x4\n"), vec!["a", "b"]);
        assert_eq!(parse_wchan("0"), "");
        assert_eq!(parse_wchan("do_epoll_wait\n"), "do_epoll_wait");
    }

    #[test]
    fn a_thread_without_a_stack_shows_its_wait_channel() {
        let mut t = thread(1, 'S', &[]);
        assert!(t.frames().is_empty());
        t.wchan = "poll_schedule_timeout".into();
        assert_eq!(t.frames(), vec!["poll_schedule_timeout"]);
        t.stack = vec!["a".into()];
        assert_eq!(t.frames(), vec!["a"]);
    }

    #[test]
    fn the_report_counts_identical_stacks_and_lists_the_commonest_first() {
        let samples = vec![
            vec![thread(1, 'S', &["futex_wait"]), thread(2, 'R', &[])],
            vec![thread(1, 'S', &["futex_wait"]), thread(2, 'S', &["poll"])],
            vec![thread(1, 'S', &["futex_wait"]), thread(2, 'S', &["poll"])],
        ];
        let report = build_report(7, "demo", Options { seconds: 3, interval_ms: 1000 }, &samples);
        assert!(report.starts_with("Sampling process 7 (demo) for 3 seconds with 1000 millisecond intervals\nSamples taken: 3\n"));
        assert!(report.contains("Thread 1 \"t1\"  (3 samples)\n  3  Sleeping\n      futex_wait\n"));
        let t2 = report.split("Thread 2").nth(1).unwrap();
        assert!(t2.find("  2  Sleeping").unwrap() < t2.find("  1  Running").unwrap());
        assert!(t2.contains("(running)"));
    }

    #[test]
    fn an_empty_sample_says_the_process_exited() {
        assert!(build_report(1, "x", Options::default(), &[]).contains("exited"));
    }

    #[test]
    fn this_process_can_be_sampled() {
        let threads = collect_once(Path::new("/proc"), std::process::id());
        assert!(!threads.is_empty());
        assert!(threads.iter().all(|t| t.tid > 0 && !t.name.is_empty()));
        assert!(collect_once(Path::new("/proc"), u32::MAX).is_empty());
    }

    #[test]
    fn a_job_finishes_and_reports() {
        let mut job = Job::start(Path::new("/proc"), std::process::id(), "self", Options { seconds: 1, interval_ms: 10 });
        for _ in 0..300 {
            if job.poll() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let report = job.report.clone().expect("finished");
        assert!(report.contains("Sampling process"));
        assert_eq!(job.fraction(), 1.0);
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Web Content/1", 12), "Sample of Web_Content_1 (12).txt");
    }
}
