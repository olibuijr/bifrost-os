//! Process > Sample Process: looks at every thread of one process many times a second for a few seconds and reports
//! where each one spent its time. A thread's place is the kernel stack it waits in (`/proc/<pid>/task/<tid>/stack`,
//! readable for the user's own processes on kernels that allow it), else the kernel function it sleeps in (`wchan`),
//! else just its state. The reading is `observe_thread`; counting and the report text are pure, so they are tested
//! without a live process.

use crate::inspect;
use crate::job::{Outcome, Progress};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::time::{Duration, Instant};

/// Time between two looks at the process, in milliseconds.
pub const INTERVAL_MS: u64 = 20;
/// How long a sample lasts, in seconds, by default and the choices Process > Sample Duration offers.
pub const DURATION_SECS: u32 = 10;
pub const DURATIONS: [u32; 3] = [3, 10, 30];
/// The system-wide sample in the diagnostics: how many looks at every thread, and the time between them.
pub const SYSTEM_ROUNDS: u32 = 5;
pub const SYSTEM_INTERVAL_MS: u64 = 200;
/// Most kernel frames kept per place, innermost first.
const MAX_FRAMES: usize = 6;
/// Frames every system call passes through, which say nothing about where the thread waits.
const NOISE: [&str; 2] = ["entry_SYSCALL_64_after_hwframe", "do_syscall_64"];

/// One thing a thread was seen doing: its state letter and where it waited.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Wait {
    pub state: char,
    pub point: String,
}

/// Function names in the text of `/proc/<pid>/task/<tid>/stack`, innermost first, with the offsets and the
/// system-call entry noise removed. Lines that are not frames (the kernel prints addresses when it hides symbols) are
/// skipped.
pub fn parse_stack(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let after = line.trim().split_once("] ")?.1;
            let name = after.split('+').next()?.trim();
            let valid = !name.is_empty() && !name.starts_with("0x") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
            (valid && !NOISE.contains(&name)).then(|| name.to_string())
        })
        .collect()
}

/// The place to count a sighting under: the kernel stack when it is known, else the `wchan` function, else what the
/// state alone says.
pub fn wait_point(stack: &[String], wchan: Option<&str>, state: char) -> String {
    if !stack.is_empty() {
        return stack.iter().take(MAX_FRAMES).map(String::as_str).collect::<Vec<_>>().join(" < ");
    }
    match wchan.map(str::trim) {
        Some(w) if !w.is_empty() && w != "0" => w.to_string(),
        _ if state == 'R' => "running".to_string(),
        _ => "no wait point readable".to_string(),
    }
}

/// What one thread did over the sample.
#[derive(Debug, Default)]
struct ThreadProfile {
    name: String,
    /// The process the thread belongs to (set for a system-wide sample).
    owner: u32,
    seen: u32,
    waits: HashMap<Wait, u32>,
}

/// The counts of a sample, by thread.
#[derive(Debug, Default)]
pub struct Profile {
    threads: BTreeMap<u32, ThreadProfile>,
    /// How many times the process (or every process, for a system-wide sample) was looked at.
    pub rounds: u32,
    /// Name each thread's process in the report (a system-wide sample covers many).
    pub show_owners: bool,
}

/// What the report says about the process before the counts.
#[derive(Clone, Debug, Default)]
pub struct Header {
    pub name: String,
    pub pid: u32,
    pub command: String,
    pub exe: String,
    pub seconds: u32,
    pub interval_ms: u64,
    /// The process ended before the time was up.
    pub ended_early: bool,
}

impl Profile {
    /// Counts one sighting of a thread.
    pub fn add(&mut self, tid: u32, name: &str, wait: Wait) {
        let thread = self.threads.entry(tid).or_default();
        thread.name = name.to_string();
        thread.seen += 1;
        *thread.waits.entry(wait).or_insert(0) += 1;
    }

    /// Counts one sighting of a thread and records the process it belongs to.
    pub fn add_for(&mut self, owner: u32, tid: u32, name: &str, wait: Wait) {
        self.add(tid, name, wait);
        if let Some(thread) = self.threads.get_mut(&tid) {
            thread.owner = owner;
        }
    }

    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// The report of a single process: a header, then each thread with its places.
    pub fn render(&self, head: &Header) -> String {
        let mut out = format!("Sampling {} ({}) for {} seconds with {} millisecond intervals\n", head.name, head.pid, head.seconds, head.interval_ms);
        if !head.command.is_empty() {
            out.push_str(&format!("Command: {}\n", head.command));
        }
        if !head.exe.is_empty() {
            out.push_str(&format!("Path: {}\n", head.exe));
        }
        out.push_str(&format!("Samples taken: {}   Threads: {}\n", self.rounds, self.threads.len()));
        if head.ended_early {
            out.push_str("The process ended before the sampling time was up.\n");
        }
        self.push_threads(&mut out);
        out
    }

    /// The report of a system-wide sample: how many looks were taken, then every thread with its process and places.
    pub fn render_system(&self, interval_ms: u64) -> String {
        let mut out = format!("System sample of every process: {} samples at {} millisecond intervals, {} threads\n", self.rounds, interval_ms, self.threads.len());
        if self.threads.is_empty() {
            out.push_str("No thread could be read. Threads of other users are not readable unless the system allows it.\n");
        }
        self.push_threads(&mut out);
        out
    }

    /// Each thread in id order with its places, the most frequent first. Indentation is leading spaces, which the viewer
    /// turns into an offset.
    fn push_threads(&self, out: &mut String) {
        for (tid, thread) in &self.threads {
            let owner = if self.show_owners && thread.owner != 0 { format!(" of pid {}", thread.owner) } else { String::new() };
            out.push_str(&format!("\nThread {tid} {}{owner}   seen in {} samples\n", thread.name, thread.seen));
            let mut waits: Vec<(&Wait, &u32)> = thread.waits.iter().collect();
            // most frequent first; equal counts in a fixed order so the report does not shuffle
            waits.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.point.cmp(&b.0.point)).then_with(|| a.0.state.cmp(&b.0.state)));
            for (wait, count) in waits {
                let share = *count as f64 / thread.seen.max(1) as f64 * 100.0;
                out.push_str(&format!("    {count} ({share:.0}%)  state {}  {}\n", wait.state, wait.point));
            }
        }
    }
}

/// The thread ids of a process, or None when it is gone.
fn threads_of(pid: u32) -> Option<Vec<u32>> {
    let mut tids: Vec<u32> = fs::read_dir(format!("/proc/{pid}/task")).ok()?.flatten().filter_map(|e| e.file_name().to_string_lossy().parse().ok()).collect();
    tids.sort_unstable();
    Some(tids)
}

/// A thread's name and what it is doing now; None when it has just gone.
fn observe_thread(pid: u32, tid: u32) -> Option<(String, Wait)> {
    let base = format!("/proc/{pid}/task/{tid}");
    let thread = inspect::parse_thread(tid, &fs::read_to_string(format!("{base}/stat")).ok()?)?;
    let stack = fs::read_to_string(format!("{base}/stack")).map(|t| parse_stack(&t)).unwrap_or_default();
    let wchan = fs::read_to_string(format!("{base}/wchan")).ok();
    let point = wait_point(&stack, wchan.as_deref(), thread.state);
    Some((thread.name, Wait { state: thread.state, point }))
}

/// Looks at every thread of every process in `pids`, `rounds` times with `interval_ms` between the looks: the system-wide
/// sample the diagnostics include. Processes that have gone are skipped. A stop request ends it early, with the looks
/// taken so far. The caller owns the progress count, so this only reads the stop request.
pub fn sample_system(pids: &[u32], rounds: u32, interval_ms: u64, progress: &Progress) -> Profile {
    let mut profile = Profile { show_owners: true, ..Profile::default() };
    for round in 0..rounds {
        if progress.cancelled() {
            break;
        }
        for &pid in pids {
            let Some(tids) = threads_of(pid) else { continue };
            for tid in tids {
                if let Some((name, wait)) = observe_thread(pid, tid) {
                    profile.add_for(pid, tid, &name, wait);
                }
            }
        }
        profile.rounds += 1;
        if round + 1 < rounds {
            std::thread::sleep(Duration::from_millis(interval_ms));
        }
    }
    profile
}

/// Samples a process for `seconds`, looking every `interval_ms`. Reports progress in milliseconds and stops early,
/// with `Cancelled`, when asked.
pub fn run(pid: u32, name: &str, seconds: u32, interval_ms: u64, progress: &Progress) -> Outcome {
    let info = inspect::inspect(pid);
    let total_ms = u64::from(seconds) * 1000;
    let mut profile = Profile::default();
    let mut ended_early = false;
    let started = Instant::now();
    progress.set(0, total_ms as u32);
    loop {
        if progress.cancelled() {
            return Outcome::Cancelled;
        }
        let elapsed = started.elapsed().as_millis() as u64;
        if elapsed >= total_ms {
            break;
        }
        let Some(tids) = threads_of(pid) else {
            ended_early = true;
            break;
        };
        for tid in tids {
            if let Some((thread_name, wait)) = observe_thread(pid, tid) {
                profile.add(tid, &thread_name, wait);
            }
        }
        profile.rounds += 1;
        progress.set(elapsed as u32, total_ms as u32);
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    if profile.rounds == 0 {
        return Outcome::Failed(format!("{name} ended before it could be sampled"));
    }
    let head = Header { name: name.to_string(), pid, command: info.command, exe: info.exe, seconds, interval_ms, ended_early };
    Outcome::Report { title: format!("Sample of {name} ({pid})"), text: profile.render(&head) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_frames_lose_offsets_and_system_call_noise() {
        let text = "[<0>] do_epoll_wait+0x4d0/0x8a0\n[<0>] __x64_sys_epoll_wait+0x5e/0x90\n[<0>] do_syscall_64+0x5c/0x90\n[<0>] entry_SYSCALL_64_after_hwframe+0x76/0x7e\n";
        assert_eq!(parse_stack(text), vec!["do_epoll_wait", "__x64_sys_epoll_wait"]);
    }

    #[test]
    fn hidden_symbols_and_blank_text_give_no_frames() {
        assert!(parse_stack("[<0>] 0xffffffffc0123456\n").is_empty());
        assert!(parse_stack("").is_empty());
        assert!(parse_stack("not a frame").is_empty());
    }

    #[test]
    fn the_wait_point_prefers_the_stack_then_wchan_then_the_state() {
        let stack: Vec<String> = ["a", "b", "c", "d", "e", "f", "g", "h"].iter().map(|s| s.to_string()).collect();
        assert_eq!(wait_point(&stack, Some("ep_poll"), 'S'), "a < b < c < d < e < f");
        assert_eq!(wait_point(&[], Some("ep_poll\n"), 'S'), "ep_poll");
        assert_eq!(wait_point(&[], Some("0"), 'R'), "running");
        assert_eq!(wait_point(&[], None, 'S'), "no wait point readable");
        assert_eq!(wait_point(&[], Some(""), 'D'), "no wait point readable");
    }

    fn head() -> Header {
        Header { name: "demo".into(), pid: 42, command: "demo --flag".into(), exe: "/usr/bin/demo".into(), seconds: 10, interval_ms: 20, ended_early: false }
    }

    #[test]
    fn the_report_lists_threads_with_the_most_frequent_place_first() {
        let mut p = Profile::default();
        let sleeping = Wait { state: 'S', point: "ep_poll".into() };
        let running = Wait { state: 'R', point: "running".into() };
        for _ in 0..3 {
            p.add(7, "worker", sleeping.clone());
        }
        p.add(7, "worker", running.clone());
        p.add(5, "main", running);
        p.rounds = 4;
        assert_eq!(p.thread_count(), 2);
        let text = p.render(&head());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Sampling demo (42) for 10 seconds with 20 millisecond intervals");
        assert_eq!(lines[1], "Command: demo --flag");
        assert_eq!(lines[2], "Path: /usr/bin/demo");
        assert_eq!(lines[3], "Samples taken: 4   Threads: 2");
        // threads in id order, so main (5) comes before worker (7)
        let main_at = text.find("Thread 5 main").unwrap();
        let worker_at = text.find("Thread 7 worker").unwrap();
        assert!(main_at < worker_at);
        assert!(text.contains("    3 (75%)  state S  ep_poll\n"));
        assert!(text.find("3 (75%)").unwrap() < text.find("1 (25%)").unwrap());
        assert!(!text.contains("ended before"));
    }

    #[test]
    fn the_report_says_when_the_process_ended_early() {
        let mut p = Profile::default();
        p.add(1, "x", Wait { state: 'S', point: "p".into() });
        p.rounds = 1;
        let mut h = head();
        h.ended_early = true;
        assert!(p.render(&h).contains("The process ended before the sampling time was up."));
    }

    #[test]
    fn the_system_report_names_each_threads_process() {
        let mut p = Profile { show_owners: true, ..Profile::default() };
        p.add_for(7, 70, "worker", Wait { state: 'S', point: "ep_poll".into() });
        p.add_for(8, 80, "other", Wait { state: 'R', point: "running".into() });
        p.rounds = 2;
        let text = p.render_system(20);
        assert!(text.starts_with("System sample of every process: 2 samples at 20 millisecond intervals, 2 threads\n"));
        assert!(text.contains("Thread 70 worker of pid 7   seen in 1 samples\n"));
        assert!(text.contains("Thread 80 other of pid 8"));
        // a single process's report keeps its plain thread lines
        let single = Profile::default().render(&head());
        assert!(!single.contains(" of pid"));
    }

    #[test]
    fn a_system_sample_looks_round_by_round_and_stops_when_asked() {
        let progress = Progress::default();
        let profile = sample_system(&[std::process::id()], 2, 1, &progress);
        assert_eq!(profile.rounds, 2);
        assert!(profile.thread_count() >= 1);
        assert!(profile.render_system(1).contains("Thread "));
        progress.request_stop();
        assert_eq!(sample_system(&[std::process::id()], 3, 1, &progress).rounds, 0);
    }

    #[test]
    fn this_process_can_be_sampled_for_a_moment() {
        let progress = Progress::default();
        match run(std::process::id(), "test", 1, 10, &progress) {
            Outcome::Report { title, text } => {
                assert!(title.starts_with("Sample of test"));
                assert!(text.contains("Thread "));
                assert!(text.contains("Samples taken: "));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(progress.fraction() > 0.0);
    }

    #[test]
    fn a_process_that_does_not_exist_fails() {
        let progress = Progress::default();
        // pid 0 is never a process directory under /proc
        assert!(matches!(run(0, "ghost", 1, 10, &progress), Outcome::Failed(_)));
    }

    #[test]
    fn a_stop_request_cancels_the_sample() {
        let progress = Progress::default();
        progress.request_stop();
        assert_eq!(run(std::process::id(), "test", 5, 10, &progress), Outcome::Cancelled);
    }
}
