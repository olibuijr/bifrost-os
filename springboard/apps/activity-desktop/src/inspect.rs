//! The process inspector: what one process has open and what it is doing, read from `/proc/<pid>`. Open files and
//! sockets come from the descriptor table, the threads from `task/`. Processes of other users deny most of this;
//! the inspector then says so instead of showing nothing.

use crate::model::{self, cpu_time};
use crate::sample::ProcSample;
use std::collections::HashMap;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

/// Most descriptors and threads listed (a browser has thousands).
pub const MAX_FILES: usize = 60;
pub const MAX_THREADS: usize = 40;

/// One TCP or UDP socket of a process: its endpoints and state, from the kernel's socket tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Port {
    /// `tcp`, `tcp6`, `udp` or `udp6`.
    pub protocol: &'static str,
    pub local: String,
    /// `*` for a socket with no peer (a listener or an unconnected datagram socket).
    pub remote: String,
    pub state: String,
    pub inode: u64,
}

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
    /// The TCP and UDP sockets of the process with their endpoints (other socket families are only counted).
    pub ports: Vec<Port>,
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

/// The inode of a socket descriptor's target (`socket:[1234]`).
pub fn socket_inode(target: &str) -> Option<u64> {
    target.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
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

/// The name of a TCP or UDP state code from a socket table (`st` column). UDP has no connection states: its
/// unconnected sockets read as UNCONN and a connected one as ESTABLISHED.
pub fn socket_state(code: &str, udp: bool) -> String {
    let n = u8::from_str_radix(code, 16).unwrap_or(0);
    let name = match n {
        1 => "ESTABLISHED",
        2 => "SYN_SENT",
        3 => "SYN_RECV",
        4 => "FIN_WAIT1",
        5 => "FIN_WAIT2",
        6 => "TIME_WAIT",
        7 if udp => "UNCONN",
        7 => "CLOSE",
        8 => "CLOSE_WAIT",
        9 => "LAST_ACK",
        10 => "LISTEN",
        11 => "CLOSING",
        12 => "NEW_SYN_RECV",
        _ if udp => "ESTABLISHED",
        _ => "",
    };
    if name.is_empty() {
        return format!("state {code}");
    }
    name.to_string()
}

/// One endpoint as the socket tables write it, `0100007F:0035`. Addresses are 32-bit words in host order (little-endian
/// on the machines BifrOSt runs on); IPv6 has four of them. An all-zero address with port 0 reads as `*`.
pub fn endpoint(hex: &str, ipv6: bool) -> Option<String> {
    let (addr, port) = hex.split_once(':')?;
    if !addr.is_ascii() {
        return None;
    }
    let port = u16::from_str_radix(port, 16).ok()?;
    if port == 0 && addr.bytes().all(|c| c == b'0') {
        return Some("*".to_string());
    }
    if ipv6 {
        if addr.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(4).enumerate() {
            let word = u32::from_str_radix(&addr[i * 8..i * 8 + 8], 16).ok()?;
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Some(format!("[{}]:{port}", Ipv6Addr::from(bytes)))
    } else {
        let word = u32::from_str_radix(addr, 16).ok()?;
        Some(format!("{}:{port}", Ipv4Addr::from(word.to_le_bytes())))
    }
}

/// The sockets listed in one of `/proc/<pid>/net/{tcp,tcp6,udp,udp6}`. The first line is the header.
pub fn parse_socket_table(text: &str, protocol: &'static str) -> Vec<Port> {
    let ipv6 = protocol.ends_with('6');
    let udp = protocol.starts_with("udp");
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                return None;
            }
            Some(Port {
                protocol,
                local: endpoint(f[1], ipv6)?,
                remote: endpoint(f[2], ipv6)?,
                state: socket_state(f[3], udp),
                inode: f[9].parse().ok()?,
            })
        })
        .collect()
}

/// The socket tables of a process's network namespace, keyed by inode.
fn read_sockets(proc: &Path, pid: u32) -> HashMap<u64, Port> {
    let mut found = HashMap::new();
    for protocol in ["tcp", "tcp6", "udp", "udp6"] {
        if let Ok(text) = fs::read_to_string(proc.join(pid.to_string()).join("net").join(protocol)) {
            for port in parse_socket_table(&text, protocol) {
                found.insert(port.inode, port);
            }
        }
    }
    found
}

/// The rows of the inspector's detail views: Memory (segment 0) and Statistics (segment 1), as label and value. The
/// process's reading in the last snapshot, when it has one, supplies the figures the inspection does not read.
pub fn detail_rows(segment: usize, data: &Inspection, proc: Option<&ProcSample>) -> Vec<(&'static str, String)> {
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    if segment == 0 {
        if let Some(p) = proc {
            rows.push(("Real Memory", model::bytes(p.rss)));
            rows.push(("Virtual Memory", model::bytes(p.vsize)));
            rows.push(("Bytes Read", model::bytes(p.read_total)));
            rows.push(("Bytes Written", model::bytes(p.write_total)));
        }
        rows.push(("Page Faults", format!("{} minor, {} major", data.minor_faults, data.major_faults)));
        return rows;
    }
    if data.ppid > 0 {
        let parent = if data.parent.is_empty() { data.ppid.to_string() } else { format!("{} ({})", data.parent, data.ppid) };
        rows.push(("Parent", parent));
    }
    if let Some(p) = proc {
        rows.push(("User", p.user.clone()));
        rows.push(("CPU", model::percent(p.cpu)));
        rows.push(("CPU Time", cpu_time(p.cpu_ticks)));
        rows.push(("Threads", p.threads.to_string()));
    }
    rows.push(("Context Switches", format!("{} voluntary, {} involuntary", data.voluntary_switches, data.involuntary_switches)));
    rows.push(("Folder", data.cwd.clone()));
    rows.push(("Executable", data.exe.clone()));
    rows.push(("Command", data.command.clone()));
    rows
}

/// The lines of the Open Files and Ports view: a heading and the descriptors, then a heading and the sockets with their
/// endpoints. Each line is (whether it is a heading, its text).
pub fn file_and_port_lines(data: &Inspection) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    if data.denied {
        out.push((false, "Open files are not readable for this process".to_string()));
    }
    out.push((true, format!("Open Files ({})", data.files_total)));
    for f in &data.files {
        out.push((false, f.clone()));
    }
    out.push((true, format!("Ports ({})", data.ports.len())));
    for p in &data.ports {
        out.push((false, format!("{}   {}   {}   {}", p.protocol, p.local, p.remote, p.state)));
    }
    if data.sockets > data.ports.len() {
        out.push((false, format!("{} other sockets (Unix, netlink and raw) are not listed", data.sockets - data.ports.len())));
    }
    out
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
            let mut socket_inodes = Vec::new();
            for (n, target) in fds {
                match describe_fd(&target) {
                    None => {
                        out.sockets += 1;
                        socket_inodes.extend(socket_inode(&target));
                    }
                    Some(line) => {
                        out.files_total += 1;
                        if out.files.len() < MAX_FILES {
                            out.files.push(format!("{n}  {line}"));
                        }
                    }
                }
            }
            if !socket_inodes.is_empty() {
                let mut tables = read_sockets(proc, pid);
                out.ports = socket_inodes.iter().filter_map(|i| tables.remove(i)).collect();
                out.ports.sort_by(|a, b| a.protocol.cmp(b.protocol).then_with(|| a.local.cmp(&b.local)));
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
    fn endpoints_decode_ipv4_and_ipv6_words() {
        assert_eq!(endpoint("0100007F:0035", false).as_deref(), Some("127.0.0.1:53"));
        assert_eq!(endpoint("00000000:0000", false).as_deref(), Some("*"));
        assert_eq!(endpoint("00000000000000000000000001000000:01BB", true).as_deref(), Some("[::1]:443"));
        assert_eq!(endpoint("0100007F:ZZZZ", false), None);
        assert_eq!(endpoint("0100007F", false), None);
    }

    #[test]
    fn socket_tables_list_each_socket_by_inode() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
                    0: 0100007F:0035 00000000:0000 0A 00000000:00000000 00:00000000 00000000   101        0 4321 1 0000 100 0 0 10 0\n\
                    1: 0100007F:9C40 0100007F:D2F4 01 00000000:00000000 00:00000000 00000000  1000        0 999 1 0000 100 0 0 10 0\n";
        let ports = parse_socket_table(text, "tcp");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0], Port { protocol: "tcp", local: "127.0.0.1:53".into(), remote: "*".into(), state: "LISTEN".into(), inode: 4321 });
        assert_eq!(ports[1].remote, "127.0.0.1:54004");
        assert_eq!(ports[1].state, "ESTABLISHED");
        let udp = parse_socket_table("h\n  0: 00000000:0044 00000000:0000 07 0 0 0 0 0 0 0 0 0 55 1 0 0 0 0\n", "udp");
        assert_eq!(udp[0].state, "UNCONN");
        assert_eq!(socket_state("0D", false), "state 0D", "an unknown code is shown as it is");
    }

    #[test]
    fn socket_inodes_come_from_the_descriptor_target() {
        assert_eq!(socket_inode("socket:[1234]"), Some(1234));
        assert_eq!(socket_inode("socket:1234"), None);
        assert_eq!(socket_inode("/tmp/x"), None);
    }

    #[test]
    fn detail_rows_follow_the_segment_and_the_snapshot() {
        let data = Inspection { minor_faults: 5, major_faults: 1, cwd: "/home/a".into(), parent: "bash".into(), ppid: 9, ..Inspection::default() };
        let p = ProcSample { rss: 2048, vsize: 4096, cpu: 12.5, cpu_ticks: 250, threads: 3, user: "olibuijr".into(), ..ProcSample::default() };
        let memory = detail_rows(0, &data, Some(&p));
        assert_eq!(memory[0], ("Real Memory", "2.0 KB".to_string()));
        assert_eq!(memory.last(), Some(&("Page Faults", "5 minor, 1 major".to_string())));
        let stats = detail_rows(1, &data, Some(&p));
        assert!(stats.contains(&("Parent", "bash (9)".to_string())));
        assert!(stats.contains(&("CPU Time", "0:02.50".to_string())));
        assert!(stats.contains(&("Folder", "/home/a".to_string())));
        // without a snapshot only what the inspection read is there
        assert_eq!(detail_rows(0, &data, None).len(), 1);
    }

    #[test]
    fn the_port_view_lists_descriptors_then_sockets() {
        let mut data = Inspection { files_total: 1, files: vec!["3  /dev/null".into()], sockets: 2, ..Inspection::default() };
        data.ports = vec![Port { protocol: "tcp", local: "127.0.0.1:53".into(), remote: "*".into(), state: "LISTEN".into(), inode: 1 }];
        let lines = file_and_port_lines(&data);
        assert_eq!(lines[0], (true, "Open Files (1)".to_string()));
        assert_eq!(lines[1], (false, "3  /dev/null".to_string()));
        assert_eq!(lines[2], (true, "Ports (1)".to_string()));
        assert_eq!(lines[3].1, "tcp   127.0.0.1:53   *   LISTEN");
        assert_eq!(lines[4].1, "1 other sockets (Unix, netlink and raw) are not listed");
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
