//! Readings of the machine from `/proc`: the CPU, memory, disk and network totals, one sample per process, and the
//! user names from `/etc/passwd`. The parsers are pure functions over the file text, so they are tested without
//! touching `/proc`. Rates come from the difference between two samples and the time between them.

use std::collections::HashMap;
use std::fs;

/// Clock ticks per second in `/proc/<pid>/stat` (USER_HZ, 100 on Linux).
pub const TICKS_PER_SEC: f64 = 100.0;
/// Page size of `/proc/<pid>/statm` (4 KiB on the platforms BifrOSt runs on).
const PAGE_BYTES: u64 = 4096;
/// Sector size of `/proc/diskstats`.
const SECTOR_BYTES: u64 = 512;

/// Cumulative CPU time over all cores, in ticks, from the `cpu` line of `/proc/stat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTotals {
    pub total: u64,
    pub idle: u64,
    pub user: u64,
    pub system: u64,
}

/// The share of CPU time spent in each state over one interval, in percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuShare {
    pub busy: f32,
    pub user: f32,
    pub system: f32,
    pub idle: f32,
}

impl Default for CpuShare {
    fn default() -> Self {
        CpuShare { busy: 0.0, user: 0.0, system: 0.0, idle: 100.0 }
    }
}

/// Memory and swap from `/proc/meminfo`, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl Memory {
    /// Memory in use: everything the kernel does not count as available.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }

    /// Used memory in percent of the total (0 when the total is unknown).
    pub fn used_percent(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.used() as f64 / self.total as f64 * 100.0) as f32
    }
}

/// One network interface from `/proc/net/dev`, with cumulative byte counters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub rx: u64,
    pub tx: u64,
}

/// An interface with its rates over the last interval (bytes per second).
#[derive(Clone, Debug, PartialEq)]
pub struct IfaceRate {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_total: u64,
    pub tx_total: u64,
}

/// One process, as the table shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcSample {
    pub pid: u32,
    pub name: String,
    pub user: String,
    pub uid: u32,
    /// Share of one core over the last interval, in percent (can exceed 100 with threads).
    pub cpu: f32,
    /// Resident memory in bytes.
    pub rss: u64,
    pub threads: u32,
    pub read_bps: f64,
    pub write_bps: f64,
}

/// Everything read in one pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub cpu: CpuShare,
    pub memory: Memory,
    pub disk_read_bps: f64,
    pub disk_write_bps: f64,
    pub ifaces: Vec<IfaceRate>,
    pub procs: Vec<ProcSample>,
}

/// The fields of `/proc/<pid>/stat` the monitor uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcStat {
    pub name: String,
    /// User plus system time, in ticks.
    pub ticks: u64,
    pub threads: u32,
}

/// Parses the aggregate `cpu` line of `/proc/stat`.
pub fn parse_proc_stat(text: &str) -> Option<CpuTotals> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    if f.len() < 4 {
        return None;
    }
    let get = |i: usize| f.get(i).copied().unwrap_or(0);
    // user nice system idle iowait irq softirq steal
    let total: u64 = f.iter().take(8).sum();
    Some(CpuTotals { total, idle: get(3) + get(4), user: get(0) + get(1), system: get(2) + get(5) + get(6) })
}

/// The CPU shares between two readings of `/proc/stat`.
pub fn cpu_share(prev: CpuTotals, now: CpuTotals) -> CpuShare {
    let dt = now.total.saturating_sub(prev.total) as f32;
    if dt <= 0.0 {
        return CpuShare::default();
    }
    let pct = |d: u64| (d as f32 / dt * 100.0).clamp(0.0, 100.0);
    let idle = pct(now.idle.saturating_sub(prev.idle));
    CpuShare {
        busy: (100.0 - idle).clamp(0.0, 100.0),
        user: pct(now.user.saturating_sub(prev.user)),
        system: pct(now.system.saturating_sub(prev.system)),
        idle,
    }
}

/// Parses `/proc/meminfo` (values are in kB there, bytes here).
pub fn parse_meminfo(text: &str) -> Memory {
    let mut m = Memory::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let kb: u64 = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let bytes = kb * 1024;
        match key.trim() {
            "MemTotal" => m.total = bytes,
            "MemAvailable" => m.available = bytes,
            "Cached" => m.cached = bytes,
            "SwapTotal" => m.swap_total = bytes,
            "SwapFree" => m.swap_free = bytes,
            _ => {}
        }
    }
    m
}

/// Whether a block device name is a whole disk (not a partition, loop, RAM or optical device).
pub fn is_whole_disk(name: &str) -> bool {
    const SKIP: [&str; 6] = ["loop", "ram", "zram", "dm-", "sr", "md"];
    if SKIP.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    if name.starts_with("nvme") || name.starts_with("mmcblk") {
        // nvme0n1 and mmcblk0 are disks; nvme0n1p1 and mmcblk0p1 are partitions
        return !name.contains('p');
    }
    const LETTER_DISKS: [&str; 3] = ["sd", "vd", "xvd"];
    LETTER_DISKS.iter().any(|p| name.starts_with(p)) && name.ends_with(|c: char| c.is_ascii_alphabetic())
}

/// Bytes read and written across the whole disks in `/proc/diskstats`, cumulative.
pub fn parse_diskstats(text: &str) -> (u64, u64) {
    let (mut read, mut written) = (0u64, 0u64);
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !is_whole_disk(f[2]) {
            continue;
        }
        read += f[5].parse::<u64>().unwrap_or(0) * SECTOR_BYTES;
        written += f[9].parse::<u64>().unwrap_or(0) * SECTOR_BYTES;
    }
    (read, written)
}

/// Parses `/proc/net/dev`: receive bytes are the first counter after the colon, transmit bytes the ninth.
pub fn parse_netdev(text: &str) -> Vec<Iface> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let n: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if n.len() < 9 {
                return None;
            }
            Some(Iface { name: name.trim().to_string(), rx: n[0], tx: n[8] })
        })
        .collect()
}

/// The name and CPU ticks of `/proc/<pid>/stat`. The name is in parentheses and may contain spaces, so the fields
/// are counted from the last `)`: the state is field 3, so user time (field 14) is index 11 after it.
pub fn parse_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    let threads: u32 = rest.get(17)?.parse().ok()?;
    Some(ProcStat { name, ticks: utime + stime, threads })
}

/// The real user id: the first number of the `Uid:` line of `/proc/<pid>/status`.
pub fn parse_uid(status: &str) -> Option<u32> {
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Resident pages: the second field of `/proc/<pid>/statm`.
pub fn parse_rss_pages(statm: &str) -> Option<u64> {
    statm.split_whitespace().nth(1)?.parse().ok()
}

/// Bytes read and written by a process, from `/proc/<pid>/io` (zero when the file is not readable).
pub fn parse_io(text: &str) -> (u64, u64) {
    let mut read = 0;
    let mut written = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let v: u64 = value.trim().parse().unwrap_or(0);
        match key.trim() {
            "read_bytes" => read = v,
            "write_bytes" => written = v,
            _ => {}
        }
    }
    (read, written)
}

/// User names by user id, from `/etc/passwd`.
pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            Some((f.get(2)?.parse::<u32>().ok()?, f.first()?.to_string()))
        })
        .collect()
}

fn read_file(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Keeps the previous readings and turns the counters into rates.
#[derive(Default)]
pub struct Sampler {
    last_ms: Option<f64>,
    cpu: Option<CpuTotals>,
    disk: Option<(u64, u64)>,
    net: HashMap<String, (u64, u64)>,
    procs: HashMap<u32, (u64, u64, u64)>,
    users: HashMap<u32, String>,
    /// The user id of this process; its processes are the ones the monitor may quit.
    pub own_uid: u32,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            users: read_file("/etc/passwd").map(|t| parse_passwd(&t)).unwrap_or_default(),
            own_uid: read_file("/proc/self/status").and_then(|t| parse_uid(&t)).unwrap_or(u32::MAX),
            ..Sampler::default()
        }
    }

    /// Reads the machine now. `now_ms` is a monotonic time in milliseconds; the first sample has no rates.
    pub fn sample(&mut self, now_ms: f64) -> Snapshot {
        let dt = self.last_ms.map(|t| (now_ms - t) / 1000.0).filter(|d| *d > 0.0);
        self.last_ms = Some(now_ms);
        let rate = |delta: u64| dt.map_or(0.0, |d| delta as f64 / d);

        let cpu = match (self.cpu, read_file("/proc/stat").and_then(|t| parse_proc_stat(&t))) {
            (Some(prev), Some(now)) => {
                self.cpu = Some(now);
                cpu_share(prev, now)
            }
            (None, Some(now)) => {
                self.cpu = Some(now);
                CpuShare::default()
            }
            _ => CpuShare::default(),
        };
        let memory = read_file("/proc/meminfo").map(|t| parse_meminfo(&t)).unwrap_or_default();

        let (read, written) = read_file("/proc/diskstats").map(|t| parse_diskstats(&t)).unwrap_or((0, 0));
        let (disk_read_bps, disk_write_bps) = match self.disk {
            Some((pr, pw)) => (rate(read.saturating_sub(pr)), rate(written.saturating_sub(pw))),
            None => (0.0, 0.0),
        };
        self.disk = Some((read, written));

        let mut ifaces = Vec::new();
        for iface in read_file("/proc/net/dev").map(|t| parse_netdev(&t)).unwrap_or_default() {
            if iface.name == "lo" {
                continue;
            }
            let (prx, ptx) = self.net.get(&iface.name).copied().unwrap_or((iface.rx, iface.tx));
            self.net.insert(iface.name.clone(), (iface.rx, iface.tx));
            ifaces.push(IfaceRate {
                rx_bps: rate(iface.rx.saturating_sub(prx)),
                tx_bps: rate(iface.tx.saturating_sub(ptx)),
                rx_total: iface.rx,
                tx_total: iface.tx,
                name: iface.name,
            });
        }

        let mut procs = Vec::new();
        let mut seen = HashMap::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
                let base = format!("/proc/{pid}");
                let Some(stat) = read_file(&format!("{base}/stat")).and_then(|t| parse_stat(&t)) else { continue };
                let uid = read_file(&format!("{base}/status")).and_then(|t| parse_uid(&t)).unwrap_or(u32::MAX);
                let rss = read_file(&format!("{base}/statm")).and_then(|t| parse_rss_pages(&t)).unwrap_or(0) * PAGE_BYTES;
                let (read, written) = read_file(&format!("{base}/io")).map(|t| parse_io(&t)).unwrap_or((0, 0));
                let (cpu, read_bps, write_bps) = match self.procs.get(&pid).copied() {
                    // ticks / TICKS_PER_SEC seconds of CPU over dt seconds, as a percentage
                    Some((pt, pr, pw)) => (
                        dt.map_or(0.0, |d| (stat.ticks.saturating_sub(pt) as f64 / TICKS_PER_SEC / d * 100.0) as f32),
                        rate(read.saturating_sub(pr)),
                        rate(written.saturating_sub(pw)),
                    ),
                    None => (0.0, 0.0, 0.0),
                };
                seen.insert(pid, (stat.ticks, read, written));
                let user = self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string());
                procs.push(ProcSample {
                    pid,
                    name: stat.name,
                    user,
                    uid,
                    cpu,
                    rss,
                    threads: stat.threads,
                    read_bps,
                    write_bps,
                });
            }
        }
        // processes that ended drop out of the previous readings
        self.procs = seen;

        Snapshot { cpu, memory, disk_read_bps, disk_write_bps, ifaces, procs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_total_and_idle() {
        let t = parse_proc_stat("cpu  100 10 50 800 40 5 5 0 0 0\ncpu0 1 1 1 1\n").unwrap();
        assert_eq!(t.total, 100 + 10 + 50 + 800 + 40 + 5 + 5 + 0);
        assert_eq!(t.idle, 840);
        assert_eq!(t.user, 110);
        assert_eq!(t.system, 60);
    }

    #[test]
    fn cpu_share_is_the_busy_fraction_of_the_interval() {
        let prev = CpuTotals { total: 1000, idle: 800, user: 100, system: 100 };
        let now = CpuTotals { total: 1200, idle: 900, user: 160, system: 120 };
        let s = cpu_share(prev, now);
        assert!((s.idle - 50.0).abs() < 0.01);
        assert!((s.busy - 50.0).abs() < 0.01);
        assert!((s.user - 30.0).abs() < 0.01);
        assert!((s.system - 10.0).abs() < 0.01);
    }

    #[test]
    fn cpu_share_without_time_is_idle() {
        let t = CpuTotals { total: 5, idle: 5, user: 0, system: 0 };
        assert_eq!(cpu_share(t, t), CpuShare::default());
    }

    #[test]
    fn meminfo_converts_kib_to_bytes() {
        let m = parse_meminfo("MemTotal:       16000 kB\nMemAvailable:    4000 kB\nCached: 2000 kB\nSwapTotal: 1000 kB\nSwapFree: 250 kB\n");
        assert_eq!(m.total, 16_000 * 1024);
        assert_eq!(m.used(), 12_000 * 1024);
        assert_eq!(m.swap_used(), 750 * 1024);
        assert!((m.used_percent() - 75.0).abs() < 0.01);
    }

    #[test]
    fn whole_disks_only() {
        assert!(is_whole_disk("sda"));
        assert!(is_whole_disk("nvme0n1"));
        assert!(is_whole_disk("mmcblk0"));
        assert!(!is_whole_disk("sda1"));
        assert!(!is_whole_disk("nvme0n1p1"));
        assert!(!is_whole_disk("loop0"));
        assert!(!is_whole_disk("zram0"));
    }

    #[test]
    fn diskstats_sums_sectors_of_whole_disks() {
        let text = "   8       0 sda 10 0 100 0 20 0 300 0 0 0 0\n   8       1 sda1 5 0 50 0 5 0 50 0 0 0 0\n";
        assert_eq!(parse_diskstats(text), (100 * 512, 300 * 512));
    }

    #[test]
    fn netdev_skips_headers_and_reads_both_directions() {
        let text = "Inter-|   Receive\n face |bytes packets\n    lo: 1 1 0 0 0 0 0 0 2 2 0 0 0 0 0 0\n  eth0: 1000 3 0 0 0 0 0 0 500 4 0 0 0 0 0 0\n";
        let ifaces = parse_netdev(text);
        assert_eq!(ifaces.len(), 2);
        assert_eq!(ifaces[1], Iface { name: "eth0".into(), rx: 1000, tx: 500 });
    }

    #[test]
    fn stat_name_may_hold_spaces_and_parens() {
        let line = "1234 (Web (Content) x) S 1 1234 1234 0 -1 4194304 1 0 0 0 42 8 0 0 20 0 7 0 99 1 1 1";
        let s = parse_stat(line).unwrap();
        assert_eq!(s.name, "Web (Content) x");
        assert_eq!(s.ticks, 50);
        assert_eq!(s.threads, 7);
    }

    #[test]
    fn uid_io_rss_and_passwd() {
        assert_eq!(parse_uid("Name:\tbash\nUid:\t1000\t1000\t1000\t1000\n"), Some(1000));
        assert_eq!(parse_rss_pages("2000 300 10 1 0 50 0"), Some(300));
        assert_eq!(parse_io("rchar: 9\nread_bytes: 4096\nwrite_bytes: 8192\n"), (4096, 8192));
        let users = parse_passwd("root:x:0:0:root:/root:/bin/bash\nolibuijr:x:1000:1000::/home/o:/bin/zsh\n");
        assert_eq!(users.get(&1000).map(String::as_str), Some("olibuijr"));
        assert_eq!(users.get(&0).map(String::as_str), Some("root"));
    }
}
