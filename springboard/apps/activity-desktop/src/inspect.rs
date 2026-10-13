//! The process inspector: what one process has open and what it is doing, read from `/proc/<pid>`. Open files and
//! sockets come from the descriptor table, the threads from `task/`. Processes of other users deny most of this;
//! the inspector then says so instead of showing nothing.

use std::fs;
use std::path::Path;

/// Most descriptors and threads listed (a browser has thousands).
pub const MAX_FILES: usize = 60;
pub const MAX_THREADS: usize = 40;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thread {
    pub tid: u32,
    pub name: String,
    pub state: char,
    /// user + system clock ticks used so far
    pub ticks: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspection {
    pub command: String,
    pub exe: String,
    pub cwd: String,
    pub files: Vec<String>,
    pub files_total: usize,
    pub sockets: usize,
    pub threads: Vec<Thread>,
    pub threads_total: usize,
    /// Set when the descriptor table could not be read.
    pub denied: bool,
    pub ppid: u32,
    /// The parent's short name (empty when it has gone or is not readable).
    pub parent: String,
    /// Page faults served from memory and from disk so far.
    pub minor_faults: u64,
    pub major_faults: u64,
    /// Context switches so far: the process gave up the CPU, or the kernel took it away.
    pub voluntary_switches: u64,
    pub involuntary_switches: u64,
}

/// `cmdline` bytes (NUL separated) as one line.
pub fn command_line(raw: &[u8]) -> String {
    let parts: Vec<String> = raw.split(|&b| b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
    parts.join(" ")
}

/// The parent pid and the minor and major fault counts of a `stat` line (fields 4, 10 and 12, counted from the last
/// `)` because the name may hold spaces).
pub fn parse_stat_extras(stat: &str) -> Option<(u32, u64, u64)> {
    let close = stat.rfind(')')?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    Some((rest.get(1)?.parse().ok()?, rest.get(7)?.parse().ok()?, rest.get(9)?.parse().ok()?))
}

/// The voluntary and involuntary context switches of `/proc/<pid>/status` (zero for a missing line).
pub fn parse_switches(status: &str) -> (u64, u64) {
    let value = |key: &str| status.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    (value("voluntary_ctxt_switches:"), value("nonvoluntary_ctxt_switches:"))
}

/// Reads the state, name and ticks from one `stat` line (the name may hold spaces and parentheses).
pub fn parse_thread(tid: u32, stat: &str) -> Option<Thread> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    let state = rest.first()?.chars().next()?;
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some(Thread { tid, name: stat[open + 1..close].to_string(), state, ticks: utime + stime })
}

/// Whether a descriptor target is a socket, and the line to show for the others.
pub fn describe_fd(target: &str) -> Option<String> {
    if target.starts_with("socket:") {
        return None;
    }
    if let Some(rest) = target.strip_prefix("anon_inode:") {
        return Some(format!("[{rest}]"));
    }
    Some(target.to_string())
}

pub fn inspect(pid: u32) -> Inspection {
    inspect_in(Path::new("/proc"), pid)
}

pub fn inspect_in(proc: &Path, pid: u32) -> Inspection {
    let dir = proc.join(pid.to_string());
    let mut out = Inspection {
        command: fs::read(dir.join("cmdline")).map(|b| command_line(&b)).unwrap_or_default(),
        exe: fs::read_link(dir.join("exe")).map(|p| p.display().to_string()).unwrap_or_default(),
        cwd: fs::read_link(dir.join("cwd")).map(|p| p.display().to_string()).unwrap_or_default(),
        ..Inspection::default()
    };
    if let Some((ppid, minor, major)) = fs::read_to_string(dir.join("stat")).ok().and_then(|t| parse_stat_extras(&t)) {
        out.ppid = ppid;
        out.minor_faults = minor;
        out.major_faults = major;
        out.parent = fs::read_to_string(proc.join(ppid.to_string()).join("comm")).map(|c| c.trim().to_string()).unwrap_or_default();
    }
    if let Ok(status) = fs::read_to_string(dir.join("status")) {
        (out.voluntary_switches, out.involuntary_switches) = parse_switches(&status);
    }
    match fs::read_dir(dir.join("fd")) {
        Ok(entries) => {
            let mut fds: Vec<(u32, String)> = entries
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().parse::<u32>().ok()?;
                    let target = fs::read_link(e.path()).ok()?.display().to_string();
                    Some((n, target))
                })
                .collect();
            fds.sort_by_key(|f| f.0);
            for (n, target) in fds {
                match describe_fd(&target) {
                    None => out.sockets += 1,
                    Some(line) => {
                        out.files_total += 1;
                        if out.files.len() < MAX_FILES {
                            out.files.push(format!("{n}  {line}"));
                        }
                    }
                }
            }
        }
        Err(_) => out.denied = true,
    }
    if let Ok(tasks) = fs::read_dir(dir.join("task")) {
        let mut tids: Vec<u32> = tasks.flatten().filter_map(|e| e.file_name().to_string_lossy().parse().ok()).collect();
        tids.sort_unstable();
        out.threads_total = tids.len();
        for tid in tids.into_iter().take(MAX_THREADS) {
            if let Some(t) = fs::read_to_string(dir.join("task").join(tid.to_string()).join("stat")).ok().and_then(|s| parse_thread(tid, &s)) {
                out.threads.push(t);
            }
        }
    }
    out
}

/// The `some avg10` figure of `/proc/pressure/memory` (percent of the last ten seconds some task waited for memory).
pub fn parse_pressure(text: &str) -> Option<f32> {
    let line = text.lines().find(|l| l.starts_with("some"))?;
    line.split_whitespace().find_map(|f| f.strip_prefix("avg10=")).and_then(|v| v.parse().ok())
}

pub fn read_pressure() -> Option<f32> {
    parse_pressure(&fs::read_to_string("/proc/pressure/memory").ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_join_the_arguments() {
        assert_eq!(command_line(b"sh\0-c\0echo hi\0"), "sh -c echo hi");
        assert_eq!(command_line(b""), "");
    }

    #[test]
    fn thread_stats_survive_odd_names() {
        let stat = "42 (my (odd) name) S 1 42 42 0 -1 4194560 100 0 0 0 7 5 0 0 20 0 1 0 100 1000 10 18446744073709551615";
        let t = parse_thread(42, stat).unwrap();
        assert_eq!((t.name.as_str(), t.state, t.ticks), ("my (odd) name", 'S', 12));
    }

    #[test]
    fn descriptors_split_into_files_and_sockets() {
        assert_eq!(describe_fd("socket:[123]"), None);
        assert_eq!(describe_fd("anon_inode:eventfd").as_deref(), Some("[eventfd]"));
        assert_eq!(describe_fd("/tmp/a").as_deref(), Some("/tmp/a"));
    }

    #[test]
    fn pressure_reads_the_some_line() {
        let text = "some avg10=1.50 avg60=0.40 avg300=0.10 total=12345\nfull avg10=0.20 avg60=0.00 avg300=0.00 total=99\n";
        assert_eq!(parse_pressure(text), Some(1.5));
        assert_eq!(parse_pressure(""), None);
    }

    #[test]
    fn stat_extras_read_the_parent_and_the_faults() {
        let stat = "42 (my (odd) name) S 7 42 42 0 -1 4194560 100 0 9 0 7 5 0 0 20 0 1 0 100 1000 10";
        assert_eq!(parse_stat_extras(stat), Some((7, 100, 9)));
        assert_eq!(parse_stat_extras("42 (x) S"), None);
    }

    #[test]
    fn context_switches_come_from_the_status_file() {
        let status = "Name:\tsh\nvoluntary_ctxt_switches:\t12\nnonvoluntary_ctxt_switches:\t3\n";
        assert_eq!(parse_switches(status), (12, 3));
        assert_eq!(parse_switches("Name:\tsh\n"), (0, 0));
    }

    #[test]
    fn this_process_can_be_inspected() {
        let i = inspect(std::process::id());
        assert!(!i.exe.is_empty());
        assert!(i.threads_total >= 1);
        assert!(!i.denied);
        assert!(i.ppid > 0);
        assert!(i.minor_faults > 0);
    }
}
