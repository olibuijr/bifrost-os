//! The process inspector: what one process has open and what it is doing, read from `/proc/<pid>`. Open files and
//! sockets come from the descriptor table, the threads from `task/`. Processes of other users deny most of this;
//! the inspector then says so instead of showing nothing.

use crate::model::{bytes, cpu_time, state_name};
use crate::sample::{parse_status, StatusExtras, TICKS_PER_SEC};
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

/// The three segments of the inspector, as in the reference app: memory figures, statistics, open files and ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Memory,
    Statistics,
    OpenFiles,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Memory, Section::Statistics, Section::OpenFiles];

    pub fn label(self) -> &'static str {
        match self {
            Section::Memory => "Memory",
            Section::Statistics => "Statistics",
            Section::OpenFiles => "Open Files and Ports",
        }
    }

    pub fn index(self) -> usize {
        Section::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Section {
        Section::ALL.get(i).copied().unwrap_or(Section::Memory)
    }
}

/// Fields of `/proc/<pid>/stat` the Statistics segment shows beyond the thread list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatFields {
    pub state: char,
    pub ppid: u32,
    pub minor_faults: u64,
    pub major_faults: u64,
    pub ticks: u64,
    pub priority: i64,
    pub nice: i64,
    /// Start time in ticks after boot.
    pub start_ticks: u64,
}

/// Reads the fields after the command name of a `stat` line (the name may hold spaces and parentheses).
pub fn parse_stat_fields(stat: &str) -> Option<StatFields> {
    let close = stat.rfind(')')?;
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    let num = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok());
    let int = |i: usize| rest.get(i).and_then(|v| v.parse::<i64>().ok());
    Some(StatFields {
        state: rest.first()?.chars().next()?,
        ppid: num(1)? as u32,
        minor_faults: num(7)?,
        major_faults: num(9)?,
        ticks: num(11)? + num(12)?,
        priority: int(15)?,
        nice: int(16)?,
        start_ticks: num(19)?,
    })
}

/// One socket of the process: the protocol, the local and remote address and the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub proto: &'static str,
    pub local: String,
    pub remote: String,
    pub state: &'static str,
}

impl Port {
    /// The line the inspector shows: `TCP 127.0.0.1:631 LISTEN`, with the remote address when there is one.
    pub fn line(&self) -> String {
        let remote = if self.remote.ends_with(":0") { String::new() } else { format!(" > {}", self.remote) };
        let state = if self.state.is_empty() { String::new() } else { format!(" {}", self.state) };
        format!("{} {}{remote}{state}", self.proto, self.local)
    }
}

/// An address column of `/proc/net/*` (hex, each 32-bit word in host order) as `a.b.c.d:port` or `[v6]:port`.
pub fn parse_addr(field: &str) -> Option<String> {
    let (addr, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    match addr.len() {
        8 => {
            let n = u32::from_str_radix(addr, 16).ok()?;
            let b = n.to_le_bytes();
            Some(format!("{}.{}.{}.{}:{port}", b[0], b[1], b[2], b[3]))
        }
        32 => {
            let mut groups = Vec::new();
            for word in 0..4 {
                let n = u32::from_str_radix(&addr[word * 8..word * 8 + 8], 16).ok()?;
                let b = n.to_le_bytes();
                groups.push(format!("{:x}", u16::from_be_bytes([b[0], b[1]])));
                groups.push(format!("{:x}", u16::from_be_bytes([b[2], b[3]])));
            }
            Some(format!("[{}]:{port}", groups.join(":")))
        }
        _ => None,
    }
}

/// The TCP state name of the `st` column.
pub fn tcp_state(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "",
    }
}

/// Parses one `/proc/net/{tcp,tcp6,udp,udp6}` table into the sockets whose inode is in `inodes`.
pub fn parse_net_table(text: &str, proto: &'static str, inodes: &[u64]) -> Vec<Port> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let inode: u64 = f.get(9)?.parse().ok()?;
            if !inodes.contains(&inode) {
                return None;
            }
            let state = if proto.starts_with("TCP") { tcp_state(f.get(3)?) } else { "" };
            Some(Port { proto, local: parse_addr(f.get(1)?)?, remote: parse_addr(f.get(2)?)?, state })
        })
        .collect()
}

/// The inode of a `socket:[123]` descriptor target.
pub fn socket_inode(target: &str) -> Option<u64> {
    target.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspection {
    pub command: String,
    pub exe: String,
    pub cwd: String,
    pub files: Vec<String>,
    pub files_total: usize,
    pub sockets: usize,
    /// The process's sockets with their addresses (up to `MAX_FILES`).
    pub ports: Vec<Port>,
    pub stat: StatFields,
    pub status: StatusExtras,
    /// The command name of the parent process.
    pub parent: String,
    pub threads: Vec<Thread>,
    pub threads_total: usize,
    /// Set when the descriptor table could not be read.
    pub denied: bool,
}

/// `cmdline` bytes (NUL separated) as one line.
pub fn command_line(raw: &[u8]) -> String {
    let parts: Vec<String> = raw.split(|&b| b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
    parts.join(" ")
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

/// The lines of the Memory or Statistics segment as label and value pairs.
pub fn rows(section: Section, d: &Inspection) -> Vec<(String, String)> {
    let row = |label: &str, value: String| (label.to_string(), value);
    match section {
        Section::Memory => vec![
            row("Real Memory", bytes(d.status.rss_anon + d.status.rss_file + d.status.rss_shmem)),
            row("Private Memory", bytes(d.status.rss_anon)),
            row("Shared Memory", bytes(d.status.rss_shmem)),
            row("File-backed Memory", bytes(d.status.rss_file)),
            row("Swapped Memory", bytes(d.status.swap)),
            row("Virtual Memory Size", bytes(d.status.virt)),
        ],
        Section::Statistics => vec![
            row("Parent Process", if d.parent.is_empty() { d.stat.ppid.to_string() } else { format!("{} ({})", d.parent, d.stat.ppid) }),
            row("State", state_name(d.stat.state).to_string()),
            row("CPU Time", cpu_time(d.stat.ticks)),
            row("Started After Boot", format!("{:.0} s", d.stat.start_ticks as f64 / TICKS_PER_SEC)),
            row("Threads", d.threads_total.to_string()),
            row("Priority", d.stat.priority.to_string()),
            row("Nice", d.stat.nice.to_string()),
            row("Page Faults", format!("{} minor, {} major", d.stat.minor_faults, d.stat.major_faults)),
            row("Context Switches", format!("{} voluntary, {} involuntary", d.status.voluntary, d.status.involuntary)),
        ],
        Section::OpenFiles => Vec::new(),
    }
}

/// The lines of the Open Files and Ports segment: the open files, then the sockets.
pub fn file_lines(d: &Inspection) -> Vec<String> {
    let mut lines: Vec<String> = d.files.clone();
    if d.denied {
        lines.push("Open files are not readable for this process".into());
    }
    lines.extend(d.ports.iter().map(Port::line));
    lines
}

/// The whole report of the inspector as text, for Copy and for saving.
pub fn report(pid: u32, d: &Inspection) -> String {
    let mut out = format!("Process {pid}\nCommand: {}\nExecutable: {}\nFolder: {}\n", d.command, d.exe, d.cwd);
    for section in [Section::Memory, Section::Statistics] {
        out.push_str(&format!("\n{}\n", section.label()));
        for (label, value) in rows(section, d) {
            out.push_str(&format!("  {label}: {value}\n"));
        }
    }
    out.push_str(&format!("\n{} ({} files, {} sockets)\n", Section::OpenFiles.label(), d.files_total, d.sockets));
    for line in file_lines(d) {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

pub fn inspect_in(proc: &Path, pid: u32) -> Inspection {
    let dir = proc.join(pid.to_string());
    let mut inodes: Vec<u64> = Vec::new();
    let mut out = Inspection {
        stat: fs::read_to_string(dir.join("stat")).ok().and_then(|t| parse_stat_fields(&t)).unwrap_or_default(),
        status: fs::read_to_string(dir.join("status")).map(|t| parse_status(&t)).unwrap_or_default(),
        command: fs::read(dir.join("cmdline")).map(|b| command_line(&b)).unwrap_or_default(),
        exe: fs::read_link(dir.join("exe")).map(|p| p.display().to_string()).unwrap_or_default(),
        cwd: fs::read_link(dir.join("cwd")).map(|p| p.display().to_string()).unwrap_or_default(),
        ..Inspection::default()
    };
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
                    None => {
                        out.sockets += 1;
                        inodes.extend(socket_inode(&target));
                    }
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
    out.parent = fs::read_to_string(proc.join(out.stat.ppid.to_string()).join("comm")).map(|c| c.trim().to_string()).unwrap_or_default();
    if !inodes.is_empty() {
        let mut ports: Vec<Port> = Vec::new();
        for (file, proto) in [("net/tcp", "TCP"), ("net/tcp6", "TCP6"), ("net/udp", "UDP"), ("net/udp6", "UDP6")] {
            if let Ok(text) = fs::read_to_string(dir.join(file)) {
                ports.extend(parse_net_table(&text, proto, &inodes));
            }
        }
        ports.sort_by(|a, b| (a.proto, &a.local).cmp(&(b.proto, &b.local)));
        ports.truncate(MAX_FILES);
        out.ports = ports;
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
    fn addresses_decode_for_both_families() {
        assert_eq!(parse_addr("0100007F:0277").as_deref(), Some("127.0.0.1:631"));
        assert_eq!(parse_addr("00000000000000000000000001000000:0016").as_deref(), Some("[0:0:0:0:0:0:0:1]:22"));
        assert_eq!(parse_addr("zz:1"), None);
    }

    #[test]
    fn net_tables_keep_only_the_sockets_of_the_process() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:0277 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4242 1 0\n   1: 0100007F:0278 0100007F:9999 01 00000000:00000000 00:00000000 00000000     0        0 4343 1 0\n";
        let ports = parse_net_table(text, "TCP", &[4242]);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].line(), "TCP 127.0.0.1:631 LISTEN");
        let est = parse_net_table(text, "TCP", &[4343]);
        assert_eq!(est[0].line(), "TCP 127.0.0.1:632 > 127.0.0.1:39321 ESTABLISHED");
        assert_eq!(socket_inode("socket:[4242]"), Some(4242));
        assert_eq!(socket_inode("/tmp/x"), None);
    }

    #[test]
    fn stat_fields_cover_faults_priority_and_start() {
        let stat = "42 (my (odd) name) S 7 42 42 0 -1 4194560 100 0 3 0 7 5 0 0 20 -5 1 0 1234 1000 10 18446744073709551615";
        let f = parse_stat_fields(stat).unwrap();
        assert_eq!((f.state, f.ppid, f.minor_faults, f.major_faults, f.ticks, f.priority, f.nice, f.start_ticks), ('S', 7, 100, 3, 12, 20, -5, 1234));
    }

    #[test]
    fn the_segments_list_their_figures() {
        let mut d = Inspection::default();
        d.status.rss_anon = 4096;
        d.status.voluntary = 5;
        d.stat.ppid = 1;
        d.parent = "init".into();
        d.stat.state = 'S';
        let mem = rows(Section::Memory, &d);
        assert_eq!(mem[1], ("Private Memory".to_string(), "4.0 KB".to_string()));
        let stats = rows(Section::Statistics, &d);
        assert_eq!(stats[0].1, "init (1)");
        assert_eq!(stats[1].1, "Sleeping");
        assert!(report(9, &d).contains("Process 9\nCommand:"));
        assert_eq!(Section::from_index(Section::OpenFiles.index()), Section::OpenFiles);
    }

    #[test]
    fn this_process_can_be_inspected() {
        let i = inspect(std::process::id());
        assert!(!i.exe.is_empty());
        assert!(i.threads_total >= 1);
        assert!(!i.denied);
        assert!(i.stat.start_ticks > 0);
        assert!(i.status.rss_anon > 0);
    }
}
