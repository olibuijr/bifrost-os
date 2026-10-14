//! Process records from the procfs text of the kernel: the stat line (pid, command name, state, parent), the
//! NUL-separated command line, the parent chain of a process, and the start and exit events between two snapshots of
//! the process table.
//!
//! Pure parsing. The caller reads `/proc/<pid>/stat` and `/proc/<pid>/cmdline` and lists the table; this module
//! never opens a file. Snapshots show starts and exits, not the exec or fork calls that caused them: a fork appears as
//! a start whose parent is the forking process, and an exec keeps the pid, so it is not reported.

use std::collections::HashSet;

/// The fields of one `/proc/<pid>/stat` line that the model uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcStat {
    pub pid: u32,
    /// The command name, without the parentheses. The kernel truncates it to 15 bytes.
    pub comm: String,
    /// The one-letter state, such as `R` (running), `S` (sleeping) or `Z` (zombie).
    pub state: char,
    /// The parent process id; 0 for the init task of the namespace.
    pub ppid: u32,
}

impl ProcStat {
    /// A process that has exited but not yet been reaped by its parent.
    pub fn is_zombie(&self) -> bool {
        self.state == 'Z'
    }
}

/// Parses one stat line, such as `42 (my app) S 1 42 42 0 -1 ...`. The command name may contain spaces and
/// parentheses, so it runs from the first `(` to the last `)`. Returns None when a field is missing or not a number.
pub fn parse_stat(line: &str) -> Option<ProcStat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = line[..open].trim().parse().ok()?;
    let comm = line[open + 1..close].to_string();
    let mut fields = line[close + 1..].split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ppid = fields.next()?.parse().ok()?;
    Some(ProcStat { pid, comm, state, ppid })
}

/// Splits a `/proc/<pid>/cmdline` buffer into its arguments. Each argument ends with a NUL byte, and invalid UTF-8 is
/// replaced. A kernel thread has an empty command line, which gives no arguments.
pub fn parse_cmdline(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let trimmed = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    trimmed.split(|&b| b == 0).map(|arg| String::from_utf8_lossy(arg).into_owned()).collect()
}

/// The ancestors of `pid` in `procs`, nearest first: its parent, the parent's parent, and so on. The walk stops at
/// the init task, at a parent that is not in the table, or when a parent repeats, so a corrupt table cannot loop.
pub fn parent_chain(procs: &[ProcStat], pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut seen = HashSet::from([pid]);
    let mut current = procs.iter().find(|p| p.pid == pid);
    while let Some(proc_) = current {
        if proc_.ppid == 0 || !seen.insert(proc_.ppid) {
            break;
        }
        chain.push(proc_.ppid);
        current = procs.iter().find(|p| p.pid == proc_.ppid);
    }
    chain
}

/// A process that started or exited between two snapshots of the process table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Started { pid: u32, parent: u32, command: String },
    Exited { pid: u32, command: String },
}

/// The start and exit events from `previous` to `current`. A process is the pair of its pid and command name, so a
/// pid that is reused by another command reports an exit and then a start. Exits come first, each in the order of the
/// previous snapshot, then starts in the order of the current one.
pub fn snapshot_events(previous: &[ProcStat], current: &[ProcStat]) -> Vec<ProcessEvent> {
    let before: HashSet<(u32, &str)> = previous.iter().map(|p| (p.pid, p.comm.as_str())).collect();
    let after: HashSet<(u32, &str)> = current.iter().map(|p| (p.pid, p.comm.as_str())).collect();
    let mut events: Vec<ProcessEvent> = previous
        .iter()
        .filter(|p| !after.contains(&(p.pid, p.comm.as_str())))
        .map(|p| ProcessEvent::Exited { pid: p.pid, command: p.comm.clone() })
        .collect();
    events.extend(
        current
            .iter()
            .filter(|p| !before.contains(&(p.pid, p.comm.as_str())))
            .map(|p| ProcessEvent::Started { pid: p.pid, parent: p.ppid, command: p.comm.clone() }),
    );
    events
}
