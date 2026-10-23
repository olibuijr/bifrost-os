//! Readings of the machine from `/proc`: the CPU, memory, disk and network totals, one sample per process, and the
//! user names from `/etc/passwd`. The parsers are pure functions over the file text, so they are tested without
//! touching `/proc`. Rates come from the difference between two samples and the time between them.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

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
    pub buffers: u64,
    pub shmem: u64,
    /// Kernel memory that cannot be reclaimed: slab, kernel stacks and page tables.
    pub sunreclaim: u64,
    pub kernel_stack: u64,
    pub page_tables: u64,
    /// Compressed swap pool (`Zswap`), the memory the compressed pages occupy.
    pub zswap: u64,
}

impl Memory {
    /// Memory the kernel holds for itself and cannot give back.
    pub fn wired(&self) -> u64 {
        self.sunreclaim + self.kernel_stack + self.page_tables
    }

    /// Shared and tmpfs memory.
    pub fn shared(&self) -> u64 {
        self.shmem
    }

    /// File cache the kernel can drop: page cache without shared memory, plus buffers.
    pub fn file_cache(&self) -> u64 {
        self.cached.saturating_sub(self.shmem) + self.buffers
    }

    /// Memory used by programs: what is in use, less the kernel's and the shared part.
    pub fn app(&self) -> u64 {
        self.used().saturating_sub(self.wired() + self.shared())
    }

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
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// An interface with its rates over the last interval (bytes per second).
#[derive(Clone, Debug, PartialEq)]
pub struct IfaceRate {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// One process, as the table shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcSample {
    pub pid: u32,
    pub ppid: u32,
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
    /// Virtual size in bytes.
    pub vsize: u64,
    /// User plus system CPU time so far, in ticks.
    pub cpu_ticks: u64,
    /// Bytes read and written so far (the cumulative counters behind the rates).
    pub read_total: u64,
    pub write_total: u64,
    /// Share of the GPU's engines over the last interval, in percent. Zero unless the sampler scans the GPU.
    pub gpu: f32,
    /// GPU engine time used so far, in nanoseconds.
    pub gpu_ns: u64,
    /// Whether the process holds a blocking sleep or idle inhibitor (only read while the sampler scans for them).
    pub prevents_sleep: bool,
    /// Mean energy score of the processes of this name since the monitor started, weighted towards the last 12 hours.
    pub energy_avg: f64,
    /// Open sockets (TCP, UDP, Unix and others) of the process; None when its descriptors are not readable or the
    /// sampler is not counting them.
    pub ports: Option<u32>,
    /// Private memory in bytes (private clean and dirty pages); None when not readable or not being read.
    pub private: Option<u64>,
    /// Voluntary context switches of its threads per second: the wake-ups of an idle process. Zero unless counted.
    pub wakeups: f64,
    /// Seconds since the process started; None when the boot time is not known.
    pub age_secs: Option<u64>,
    /// Real memory in bytes: the proportional set size, which splits every shared page between the processes that map
    /// it. None when not readable or not being read.
    pub real: Option<u64>,
    /// Memory the kernel may take back without writing it anywhere (pages freed with `MADV_FREE`) in bytes; None when
    /// not readable or not being read.
    pub purgeable: Option<u64>,
    /// The machine the program runs on, read from its executable's ELF header ("x86_64", "ARM64"); None when the
    /// executable is not readable (another user's process, a kernel thread) or not being read.
    pub kind: Option<&'static str>,
    /// Bytes received and sent per second over the process's TCP sockets. Zero unless the sampler reads sockets.
    pub net_rx_bps: f64,
    pub net_tx_bps: f64,
    /// Bytes received and sent so far over the TCP sockets the process holds open now (a closed socket takes its count
    /// with it). Zero unless the sampler reads sockets.
    pub net_rx_total: u64,
    pub net_tx_total: u64,
    /// The TCP sockets the process holds; None when it owns none the monitor can see, or the sampler is not reading
    /// sockets.
    pub net_sockets: Option<u32>,
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
    /// Busy share of each core over the interval, in percent, in core order.
    pub cores: Vec<f32>,
    /// All processes' GPU shares together, capped at 100; None while the sampler is not scanning the GPU.
    pub gpu_busy: Option<f32>,
}

/// The fields of `/proc/<pid>/stat` the monitor uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcStat {
    pub name: String,
    pub ppid: u32,
    /// User plus system time, in ticks.
    pub ticks: u64,
    pub threads: u32,
    /// When the process started, in ticks since boot (field 22 of the stat line).
    pub start_ticks: u64,
}

/// The boot time in seconds since the epoch: the `btime` line of `/proc/stat`.
pub fn parse_btime(text: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix("btime "))?.trim().parse().ok()
}

/// Parses the aggregate `cpu` line of `/proc/stat`.
pub fn parse_proc_stat(text: &str) -> Option<CpuTotals> {
    totals_of(text.lines().find(|l| l.starts_with("cpu "))?)
}

/// The per-core `cpuN` lines of `/proc/stat`, in core order.
pub fn parse_core_stats(text: &str) -> Vec<CpuTotals> {
    text.lines()
        .filter(|l| l.strip_prefix("cpu").and_then(|r| r.chars().next()).map_or(false, |c| c.is_ascii_digit()))
        .filter_map(totals_of)
        .collect()
}

/// The totals on one `cpu` or `cpuN` line.
fn totals_of(line: &str) -> Option<CpuTotals> {
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
            "Buffers" => m.buffers = bytes,
            "Shmem" => m.shmem = bytes,
            "SUnreclaim" => m.sunreclaim = bytes,
            "KernelStack" => m.kernel_stack = bytes,
            "PageTables" => m.page_tables = bytes,
            "Zswap" => m.zswap = bytes,
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

/// Parses `/proc/net/dev`: receive bytes and packets are the first two counters after the colon, transmit bytes and
/// packets the ninth and tenth.
pub fn parse_netdev(text: &str) -> Vec<Iface> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let n: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if n.len() < 10 {
                return None;
            }
            Some(Iface { name: name.trim().to_string(), rx: n[0], tx: n[8], rx_packets: n[1], tx_packets: n[9] })
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
    let ppid: u32 = rest.get(1)?.parse().ok()?;
    let start_ticks: u64 = rest.get(19).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(ProcStat { name, ppid, ticks: utime + stime, threads, start_ticks })
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

/// Virtual size in pages: the first field of `/proc/<pid>/statm`.
pub fn parse_vsize_pages(statm: &str) -> Option<u64> {
    statm.split_whitespace().next()?.parse().ok()
}

/// What `/proc/<pid>/fdinfo/<fd>` of a DRM descriptor says about GPU use (the kernel's drm-usage-stats format): the
/// client it belongs to, as `pdev/client-id`, and the engine time of all its engines in nanoseconds. None when the
/// descriptor is not a DRM client. Several descriptors of one process can be the same client; count each client once.
pub fn parse_drm_fdinfo(text: &str) -> Option<(String, u64)> {
    let (mut driver, mut client, mut pdev, mut ns) = (false, String::new(), String::new(), 0u64);
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key {
            "drm-driver" => driver = true,
            "drm-client-id" => client = value.to_string(),
            "drm-pdev" => pdev = value.to_string(),
            _ => {
                let engine = key.strip_prefix("drm-engine-");
                if engine.map_or(false, |e| !e.starts_with("capacity-")) {
                    if let Some(n) = value.strip_suffix("ns").and_then(|n| n.trim().parse::<u64>().ok()) {
                        ns += n;
                    }
                }
            }
        }
    }
    driver.then(|| (format!("{pdev}/{client}"), ns))
}

/// A logind inhibitor lock (one row of `ListInhibitors`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inhibitor {
    pub what: String,
    pub who: String,
    pub why: String,
    pub mode: String,
    pub uid: u32,
    pub pid: u32,
}

/// Splits `busctl` output into words. Quoted strings come back without the quotes and with their backslash escapes
/// resolved.
fn busctl_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut word = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => word.extend(chars.next()),
                    '"' => break,
                    _ => word.push(c),
                }
            }
            words.push(word);
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
            words.push(word);
        }
    }
    words
}

/// Parses `busctl call ... ListInhibitors`: the signature, a count, then six fields per lock (what, who, why, mode,
/// uid, pid). A malformed reply gives no locks.
pub fn parse_inhibitors(text: &str) -> Vec<Inhibitor> {
    let words = busctl_words(text);
    let Some(count) = words.get(1).and_then(|c| c.parse::<usize>().ok()) else { return Vec::new() };
    let fields = &words[2..];
    if fields.len() < count * 6 {
        return Vec::new();
    }
    fields
        .chunks_exact(6)
        .take(count)
        .filter_map(|f| Some(Inhibitor { what: f[0].clone(), who: f[1].clone(), why: f[2].clone(), mode: f[3].clone(), uid: f[4].parse().ok()?, pid: f[5].parse().ok()? }))
        .collect()
}

/// The processes holding a lock that keeps the machine awake: a blocking sleep or idle inhibitor.
pub fn sleep_blockers(locks: &[Inhibitor]) -> HashSet<u32> {
    locks.iter().filter(|l| l.mode == "block" && l.what.split(':').any(|w| w == "sleep" || w == "idle")).map(|l| l.pid).collect()
}

/// Asks logind for its inhibitor locks. Empty when `busctl` or logind is not there.
fn read_inhibitors() -> HashSet<u32> {
    let out = Command::new("busctl")
        .args(["--system", "call", "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager", "ListInhibitors"])
        .output();
    match out {
        Ok(o) if o.status.success() => sleep_blockers(&parse_inhibitors(&String::from_utf8_lossy(&o.stdout))),
        _ => HashSet::new(),
    }
}

/// What a process's `smaps_rollup` says about its memory, in bytes. A field is None when its lines are not there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rollup {
    /// Private clean and dirty pages.
    pub private: Option<u64>,
    /// The proportional set size: each shared page counted as its size over the processes that map it.
    pub pss: Option<u64>,
    /// Pages freed with `MADV_FREE`, which the kernel may drop at any moment (`LazyFree`).
    pub lazy_free: Option<u64>,
}

/// The fields of a process's `smaps_rollup` the monitor uses. A line that does not read is skipped.
pub fn parse_rollup(text: &str) -> Rollup {
    let mut out = Rollup::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let Some(kb) = rest.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()) else { continue };
        let bytes = kb * 1024;
        if key.starts_with("Private_") {
            out.private = Some(out.private.unwrap_or(0) + bytes);
        } else if key == "Pss" {
            out.pss = Some(bytes);
        } else if key == "LazyFree" {
            out.lazy_free = Some(bytes);
        }
    }
    out
}

/// Private memory of a process in bytes: the private clean and dirty pages of its `smaps_rollup`. None when the file
/// is not there or has no private lines.
pub fn parse_private(text: &str) -> Option<u64> {
    parse_rollup(text).private
}

/// The rollup of a process, or None when its `smaps_rollup` is not readable (another user's process).
fn read_rollup(pid: u32) -> Option<Rollup> {
    read_file(&format!("/proc/{pid}/smaps_rollup")).map(|t| parse_rollup(&t))
}

/// The machine an ELF executable is built for, from the first 20 bytes of the file: the byte order tells how to read
/// `e_machine` (offset 18). None when the bytes are not an ELF header.
pub fn parse_elf_kind(header: &[u8]) -> Option<&'static str> {
    if header.len() < 20 || header[..4] != [0x7f, b'E', b'L', b'F'] {
        return None;
    }
    let machine = match header[5] {
        1 => u16::from_le_bytes([header[18], header[19]]),
        2 => u16::from_be_bytes([header[18], header[19]]),
        _ => return None,
    };
    Some(match machine {
        0x3E => "x86_64",
        0x03 => "x86",
        0xB7 => "ARM64",
        0x28 => "ARM",
        0xF3 => "RISC-V",
        _ => "Other",
    })
}

/// The kind of a process's program, from `/proc/<pid>/exe`. None when the link cannot be opened (another user's
/// process, a kernel thread) or is not an ELF file.
fn read_kind(pid: u32) -> Option<&'static str> {
    use std::io::Read;
    let mut header = [0u8; 20];
    fs::File::open(format!("/proc/{pid}/exe")).ok()?.read_exact(&mut header).ok()?;
    parse_elf_kind(&header)
}

/// The sockets a process has open, counted from its descriptors. None when its descriptor table is not readable.
fn count_sockets(pid: u32) -> Option<u32> {
    let fds = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let sockets = fds.flatten().filter(|fd| fs::read_link(fd.path()).map_or(false, |t| t.to_string_lossy().starts_with("socket:"))).count();
    Some(sockets as u32)
}

/// Voluntary context switches summed over a process's threads (each thread's status file). None when the thread list
/// is not readable.
fn read_voluntary(pid: u32) -> Option<u64> {
    let tasks = fs::read_dir(format!("/proc/{pid}/task")).ok()?;
    let mut total = 0;
    for task in tasks.flatten() {
        if let Some(text) = read_file(&format!("/proc/{pid}/task/{}/status", task.file_name().to_string_lossy())) {
            total += crate::inspect::parse_switches(&text).0;
        }
    }
    Some(total)
}

/// The command of the vendor's GPU management tool. It reports each process's engine share when the driver publishes
/// no DRM counters, as the proprietary drivers do.
const GPU_TOOL: &str = "nvidia-smi";

/// The per-process engine share from the vendor GPU tool's process table (its `sm` column, percent), by pid. The header
/// lines start with `#`; a process with no figure shows `-`. A process on several GPUs adds up.
pub fn parse_pmon(text: &str) -> HashMap<u32, f32> {
    let mut shares = HashMap::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(pid), Some(sm)) = (f.get(1).and_then(|p| p.parse::<u32>().ok()), f.get(3)) else { continue };
        let share: f32 = sm.parse().unwrap_or(0.0);
        *shares.entry(pid).or_insert(0.0) += share;
    }
    shares
}

/// Reads the per-process engine shares once from the vendor GPU tool. None when the tool cannot be run (no such driver).
fn read_gpu_tool_shares() -> Option<HashMap<u32, f32>> {
    let out = Command::new(GPU_TOOL).args(["pmon", "-c", "1", "-s", "u"]).output().ok()?;
    out.status.success().then(|| parse_pmon(&String::from_utf8_lossy(&out.stdout)))
}

/// Engine time of the DRM clients a process holds open, in nanoseconds. Descriptors of other users are unreadable
/// and count as nothing.
fn read_gpu_ns(pid: u32) -> u64 {
    let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else { return 0 };
    let mut seen = HashSet::new();
    let mut total = 0;
    for fd in fds.flatten() {
        if !fs::read_link(fd.path()).map_or(false, |t| t.starts_with("/dev/dri/")) {
            continue;
        }
        let Some(text) = read_file(&format!("/proc/{pid}/fdinfo/{}", fd.file_name().to_string_lossy())) else { continue };
        if let Some((client, ns)) = parse_drm_fdinfo(&text) {
            if seen.insert(client) {
                total += ns;
            }
        }
    }
    total
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

/// The uid a user namespace shows for an owner it does not map (`/proc/sys/kernel/overflowuid`). Activity Monitor runs in
/// one, so this is what it reads for every process that is not its own account's.
pub const OVERFLOW_UID: u32 = 65534;

/// The owners the shell's `process-owners` service answered: pid -> (real uid, account name).
pub type Owners = HashMap<u32, (u32, String)>;

/// Reads the answer of `process-owners.owners`: `{users: [{uid, user, pids: [..]}]}`.
pub fn parse_owners(answer: &bifrost_core::json::Json) -> Owners {
    let mut out = Owners::new();
    for user in answer.get("users").and_then(|u| u.array()).unwrap_or(&[]) {
        let (Some(uid), Some(name)) = (user.get("uid").and_then(|u| u.as_u64()), user.str_field("user")) else { continue };
        for pid in user.get("pids").and_then(|p| p.array()).unwrap_or(&[]).iter().filter_map(|p| p.as_u64()) {
            out.insert(pid as u32, (uid as u32, name.clone()));
        }
    }
    out
}

/// The uid and account name of process `pid`, which this sandbox reads as `uid`. The overflow uid means the namespace
/// could not map the real owner, so the shell's answer (when it has one for the pid) takes its place.
fn owner_of(pid: u32, uid: u32, users: &HashMap<u32, String>, owners: &Owners) -> (u32, String) {
    if uid == OVERFLOW_UID {
        if let Some((real, name)) = owners.get(&pid) {
            return (*real, name.clone());
        }
    }
    (uid, users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()))
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
    cores: Vec<CpuTotals>,
    net: HashMap<String, (u64, u64)>,
    procs: HashMap<u32, Prev>,
    users: HashMap<u32, String>,
    /// Real owners from the shell, for the processes this sandbox cannot map (see `owner_of`).
    owners: Owners,
    sleep_blockers: HashSet<u32>,
    blockers_read_at: Option<f64>,
    /// The user id of this process; its processes are the ones the monitor may quit.
    pub own_uid: u32,
    /// Read the DRM descriptors of every process for GPU time. Costly on a busy machine, so only while it is shown.
    pub scan_gpu: bool,
    /// Ask logind which processes keep the machine awake (every `BLOCKERS_EVERY_MS`). Only while it is shown.
    pub scan_sleep: bool,
    /// Count each process's open sockets (the Ports column). Only while it is shown.
    pub scan_ports: bool,
    /// Read each process's private memory (the Private Memory column). Only while it is shown.
    pub scan_private: bool,
    /// Count each process's voluntary context switches over its threads (the Idle Wake Ups column). Only while shown.
    pub scan_wakeups: bool,
    /// Read each process's real memory (the Real Mem column). Only while it is shown.
    pub scan_real: bool,
    /// Read each process's purgeable memory (the Purgeable column). Only while it is shown.
    pub scan_purgeable: bool,
    /// Read each program's kind from its ELF header (the Kind column). Only while it is shown.
    pub scan_kind: bool,
    /// Read the TCP sockets with their byte counters and sum them per process (the Network tab). Runs `ss` once per
    /// reading, so only while the tab is shown.
    pub scan_net: bool,
    /// Each process's socket byte counters at the previous reading, as (received, sent).
    net_prev: HashMap<u32, (u64, u64)>,
    /// The kind of each program already read, by pid; a process keeps its program, so it is read once.
    kinds: HashMap<u32, Option<&'static str>>,
    /// The boot time in seconds since the epoch (zero when unknown), to age the processes.
    boot: u64,
    /// The vendor tool's engine share of each process, read while the GPU is scanned (see `parse_pmon`).
    tool_shares: HashMap<u32, f32>,
    /// The vendor tool is not on this machine: it is not asked again.
    no_gpu_tool: bool,
}

/// How long the list of sleep blockers is trusted before logind is asked again (milliseconds).
const BLOCKERS_EVERY_MS: f64 = 10_000.0;

/// A process's counters at the previous reading.
#[derive(Clone, Copy)]
struct Prev {
    ticks: u64,
    read: u64,
    written: u64,
    /// None when the GPU was not scanned then, so no rate can be taken.
    gpu_ns: Option<u64>,
    /// None when the wake-ups were not counted then.
    voluntary: Option<u64>,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            users: read_file("/etc/passwd").map(|t| parse_passwd(&t)).unwrap_or_default(),
            own_uid: read_file("/proc/self/status").and_then(|t| parse_uid(&t)).unwrap_or(u32::MAX),
            boot: read_file("/proc/stat").and_then(|t| parse_btime(&t)).unwrap_or(0),
            ..Sampler::default()
        }
    }

    /// Takes the owners the shell answered; the next sample names the processes with them.
    pub fn set_owners(&mut self, owners: Owners) {
        self.owners = owners;
    }

    /// Reads the machine now. `now_ms` is a monotonic time in milliseconds; the first sample has no rates.
    pub fn sample(&mut self, now_ms: f64) -> Snapshot {
        if bifrost_app::testing::pinned() {
            // Under the compliance harness the readings are a fixed fixture, never the machine that runs the test:
            // its CPU load, memory and process list change from one run to the next and would move the golden.
            return fixture();
        }
        let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
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
        let cores = read_file("/proc/stat").map(|t| parse_core_stats(&t)).unwrap_or_default();
        let core_busy: Vec<f32> = cores
            .iter()
            .enumerate()
            .map(|(i, now)| self.cores.get(i).map_or(0.0, |prev| cpu_share(*prev, *now).busy))
            .collect();
        self.cores = cores;
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
                rx_packets: iface.rx_packets,
                tx_packets: iface.tx_packets,
                name: iface.name,
            });
        }

        if self.scan_sleep && self.blockers_read_at.map_or(true, |at| now_ms - at >= BLOCKERS_EVERY_MS) {
            self.sleep_blockers = read_inhibitors();
            self.blockers_read_at = Some(now_ms);
        }
        if !self.scan_sleep {
            self.sleep_blockers.clear();
            self.blockers_read_at = None;
        }
        // a proprietary driver publishes no DRM counters, so its per-process shares come from the vendor tool
        if !self.scan_gpu {
            self.tool_shares.clear();
        } else if !self.no_gpu_tool {
            match read_gpu_tool_shares() {
                Some(shares) => self.tool_shares = shares,
                None => {
                    self.no_gpu_tool = true;
                    self.tool_shares.clear();
                }
            }
        }
        // the sockets are read once for all processes: their counters are summed per owner
        let net_now: HashMap<u32, crate::sample_net::ProcNet> =
            if self.scan_net { crate::sample_net::per_process(&crate::sample_net::read_sockets()).into_iter().map(|n| (n.pid, n)).collect() } else { HashMap::new() };
        let net_ms = dt.map_or(0, |d| (d * 1000.0) as u64);
        let net_prev = std::mem::take(&mut self.net_prev);
        let mut net_seen = HashMap::new();
        let mut kinds_seen = HashMap::new();
        let mut procs = Vec::new();
        let mut seen = HashMap::new();
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
                let base = format!("/proc/{pid}");
                let Some(stat) = read_file(&format!("{base}/stat")).and_then(|t| parse_stat(&t)) else { continue };
                let uid = read_file(&format!("{base}/status")).and_then(|t| parse_uid(&t)).unwrap_or(u32::MAX);
                let ports = self.scan_ports.then(|| count_sockets(pid)).flatten();
                // one read of smaps_rollup serves Private Memory, Real Mem and Purgeable
                let rollup = (self.scan_private || self.scan_real || self.scan_purgeable).then(|| read_rollup(pid)).flatten();
                let private = rollup.filter(|_| self.scan_private).and_then(|r| r.private);
                let real = rollup.filter(|_| self.scan_real).and_then(|r| r.pss);
                let purgeable = rollup.filter(|_| self.scan_purgeable).and_then(|r| r.lazy_free);
                let kind = if self.scan_kind {
                    let known = self.kinds.get(&pid).copied();
                    let kind = known.unwrap_or_else(|| read_kind(pid));
                    kinds_seen.insert(pid, kind);
                    kind
                } else {
                    None
                };
                let net = net_now.get(&pid);
                let (net_rx_bps, net_tx_bps) = match (net, net_prev.get(&pid)) {
                    (Some(n), Some((rx, tx))) => (crate::sample_net::rate(*rx, n.bytes_received, net_ms) as f64, crate::sample_net::rate(*tx, n.bytes_sent, net_ms) as f64),
                    _ => (0.0, 0.0),
                };
                if let Some(n) = net {
                    net_seen.insert(pid, (n.bytes_received, n.bytes_sent));
                }
                let voluntary = self.scan_wakeups.then(|| read_voluntary(pid)).flatten();
                let statm = read_file(&format!("{base}/statm"));
                let rss = statm.as_deref().and_then(parse_rss_pages).unwrap_or(0) * PAGE_BYTES;
                let vsize = statm.as_deref().and_then(parse_vsize_pages).unwrap_or(0) * PAGE_BYTES;
                let (read, written) = read_file(&format!("{base}/io")).map(|t| parse_io(&t)).unwrap_or((0, 0));
                let gpu_ns = self.scan_gpu.then(|| read_gpu_ns(pid));
                let before = self.procs.get(&pid).copied();
                let (cpu, read_bps, write_bps) = match before {
                    // ticks / TICKS_PER_SEC seconds of CPU over dt seconds, as a percentage
                    Some(prev) => (
                        dt.map_or(0.0, |d| (stat.ticks.saturating_sub(prev.ticks) as f64 / TICKS_PER_SEC / d * 100.0) as f32),
                        rate(read.saturating_sub(prev.read)),
                        rate(written.saturating_sub(prev.written)),
                    ),
                    None => (0.0, 0.0, 0.0),
                };
                // GPU time over dt seconds is nanoseconds / 1e9 / dt of engine time, as a percentage
                let gpu = match (before.and_then(|b| b.gpu_ns), gpu_ns, dt) {
                    (Some(prev), Some(now), Some(d)) => (now.saturating_sub(prev) as f64 / 1e9 / d * 100.0) as f32,
                    _ => 0.0,
                };
                let gpu = if gpu == 0.0 { self.tool_shares.get(&pid).copied().unwrap_or(0.0) } else { gpu };
                // wake-ups per second, from the switches counted at the last reading
                let wakeups = match (before.and_then(|b| b.voluntary), voluntary, dt) {
                    (Some(prev), Some(now), Some(d)) => now.saturating_sub(prev) as f64 / d,
                    _ => 0.0,
                };
                seen.insert(pid, Prev { ticks: stat.ticks, read, written, gpu_ns, voluntary });
                let (uid, user) = owner_of(pid, uid, &self.users, &self.owners);
                procs.push(ProcSample {
                    pid,
                    ppid: stat.ppid,
                    name: stat.name,
                    user,
                    uid,
                    cpu,
                    rss,
                    threads: stat.threads,
                    read_bps,
                    write_bps,
                    vsize,
                    cpu_ticks: stat.ticks,
                    read_total: read,
                    write_total: written,
                    gpu,
                    gpu_ns: gpu_ns.unwrap_or(0),
                    prevents_sleep: self.sleep_blockers.contains(&pid),
                    energy_avg: 0.0,
                    ports,
                    private,
                    wakeups,
                    age_secs: (self.boot > 0).then(|| now_unix.saturating_sub(self.boot + (stat.start_ticks as f64 / TICKS_PER_SEC) as u64)),
                    real,
                    purgeable,
                    kind,
                    net_rx_bps,
                    net_tx_bps,
                    net_rx_total: net.map_or(0, |n| n.bytes_received),
                    net_tx_total: net.map_or(0, |n| n.bytes_sent),
                    net_sockets: net.map(|n| n.sockets),
                });
            }
        }
        // processes that ended drop out of the previous readings
        self.procs = seen;
        self.net_prev = net_seen;
        self.kinds = kinds_seen;

        let gpu_busy = self.scan_gpu.then(|| procs.iter().map(|p| p.gpu).sum::<f32>().min(100.0));
        Snapshot { cpu, memory, disk_read_bps, disk_write_bps, ifaces, procs, cores: core_busy, gpu_busy }
    }
}

/// The readings under the test pin (`bifrost_app::testing::pinned`): a quiet eight-core machine with a short fixed process list.
fn fixture() -> Snapshot {
    const GIB: u64 = 1 << 30;
    let proc = |pid: u32, ppid: u32, name: &str, user: &str, uid: u32, cpu: f32, rss_mib: u64, threads: u32| ProcSample {
        pid,
        ppid,
        name: name.into(),
        user: user.into(),
        uid,
        cpu,
        rss: rss_mib << 20,
        vsize: (rss_mib * 3) << 20,
        threads,
        cpu_ticks: (cpu * 100.0) as u64,
        ..Default::default()
    };
    let procs = vec![
        proc(1, 0, "systemd", "root", 0, 0.1, 14, 1),
        proc(412, 1, "springboard", "me", 1000, 6.5, 310, 12),
        proc(520, 412, "bifrost-activity", "me", 1000, 2.4, 96, 6),
        proc(533, 412, "bifrost-files", "me", 1000, 0.8, 74, 5),
        proc(601, 1, "pipewire", "me", 1000, 0.6, 28, 3),
        proc(655, 1, "NetworkManager", "root", 0, 0.3, 22, 4),
        proc(702, 1, "dbus-daemon", "me", 1000, 0.1, 6, 1),
        proc(844, 1, "bluetoothd", "root", 0, 0.0, 9, 1),
        proc(910, 1, "sshd", "root", 0, 0.0, 8, 1),
        proc(977, 1, "cron", "root", 0, 0.0, 4, 1),
    ];
    Snapshot {
        cpu: CpuShare { busy: 12.0, user: 8.0, system: 4.0, idle: 88.0 },
        memory: Memory { total: 16 * GIB, available: 11 * GIB, cached: 3 * GIB, swap_total: 4 * GIB, swap_free: 4 * GIB, buffers: GIB / 8, shmem: GIB / 4, sunreclaim: GIB / 4, kernel_stack: 16 << 20, page_tables: 48 << 20, zswap: 0 },
        disk_read_bps: 120_000.0,
        disk_write_bps: 80_000.0,
        ifaces: vec![IfaceRate { name: "eth0".into(), rx_bps: 24_000.0, tx_bps: 6_000.0, rx_total: 480 << 20, tx_total: 96 << 20, rx_packets: 410_000, tx_packets: 190_000 }],
        procs,
        cores: vec![14.0, 9.0, 21.0, 6.0, 11.0, 8.0, 17.0, 10.0],
        gpu_busy: None,
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
        assert_eq!(ifaces[1], Iface { name: "eth0".into(), rx: 1000, tx: 500, rx_packets: 3, tx_packets: 4 });
    }

    #[test]
    fn stat_name_may_hold_spaces_and_parens() {
        let line = "1234 (Web (Content) x) S 1 1234 1234 0 -1 4194304 1 0 0 0 42 8 0 0 20 0 7 0 99 1 1 1";
        let s = parse_stat(line).unwrap();
        assert_eq!(s.name, "Web (Content) x");
        assert_eq!(s.ticks, 50);
        assert_eq!(s.threads, 7);
        assert_eq!(s.ppid, 1);
    }

    #[test]
    fn per_core_lines_are_read_in_order_and_the_total_line_is_left_out() {
        let text = "cpu  10 0 10 80 0 0 0 0\ncpu0 5 0 5 40 0 0 0 0\ncpu1 5 0 5 40 0 0 0 0\nintr 1\ncpuidle 3\n";
        let cores = parse_core_stats(text);
        assert_eq!(cores.len(), 2);
        assert_eq!(cores[1].total, 50);
        assert!(parse_core_stats("cpu  1 2 3 4\n").is_empty());
    }

    #[test]
    fn meminfo_breakdown_splits_programs_kernel_shared_and_cache() {
        let m = parse_meminfo(
            "MemTotal: 1000 kB\nMemAvailable: 400 kB\nCached: 300 kB\nBuffers: 20 kB\nShmem: 50 kB\nSUnreclaim: 30 kB\nKernelStack: 5 kB\nPageTables: 15 kB\nZswap: 7 kB\n",
        );
        assert_eq!(m.wired(), 50 * 1024);
        assert_eq!(m.shared(), 50 * 1024);
        assert_eq!(m.file_cache(), (300 - 50 + 20) * 1024);
        assert_eq!(m.app(), (600 - 50 - 50) * 1024);
        assert_eq!(m.zswap, 7 * 1024);
    }

    #[test]
    fn virtual_size_is_the_first_statm_field() {
        assert_eq!(parse_vsize_pages("2000 300 10 1 0 50 0"), Some(2000));
        assert_eq!(parse_vsize_pages(""), None);
    }

    #[test]
    fn drm_fdinfo_sums_the_engines_and_names_the_client() {
        let text = "pos:\t0\ndrm-driver:\tamdgpu\ndrm-client-id:\t12\ndrm-pdev:\t0000:03:00.0\ndrm-engine-gfx:\t1500 ns\ndrm-engine-compute:\t500 ns\ndrm-engine-capacity-gfx:\t1\ndrm-memory-vram:\t8 KiB\n";
        assert_eq!(parse_drm_fdinfo(text), Some(("0000:03:00.0/12".to_string(), 2000)));
        assert_eq!(parse_drm_fdinfo("pos:\t0\nflags:\t02\n"), None);
    }

    #[test]
    fn inhibitor_replies_parse_with_quoted_text_and_pick_the_blockers() {
        let reply = "a(ssssuu) 3 \"sleep\" \"NetworkManager\" \"needs \\\"a\\\" pause\" \"delay\" 0 932 \"idle:sleep\" \"Firefox\" \"Playing video\" \"block\" 1000 4242 \"shutdown\" \"Updater\" \"Updating\" \"block\" 0 77\n";
        let locks = parse_inhibitors(reply);
        assert_eq!(locks.len(), 3);
        assert_eq!(locks[0].why, "needs \"a\" pause");
        assert_eq!(locks[1].pid, 4242);
        let blockers = sleep_blockers(&locks);
        assert!(blockers.contains(&4242));
        assert!(!blockers.contains(&932));
        assert!(!blockers.contains(&77));
    }

    #[test]
    fn a_truncated_inhibitor_reply_gives_no_locks() {
        assert!(parse_inhibitors("a(ssssuu) 2 \"sleep\" \"x\"").is_empty());
        assert!(parse_inhibitors("").is_empty());
        assert!(parse_inhibitors("garbage").is_empty());
    }

    #[test]
    fn gpu_tool_lines_give_each_process_its_engine_share() {
        let text = "# gpu         pid   type     sm    mem    enc    dec    jpg    ofa    command\n\
                    # Idx           #    C/G     %      %      %      %      %      %    name\n\
                    \x20   0       1234     G     12      3      0      0      0      0    Xorg\n\
                    \x20   0       1234     G      8      1      0      0      0      0    Xorg\n\
                    \x20   1       4321     C      -      -      -      -      -      -    python\n\
                    \x20   0          -     -      -      -      -      -      -      -    -\n";
        let shares = parse_pmon(text);
        assert_eq!(shares.get(&1234).copied(), Some(20.0), "several GPUs and lines add up");
        assert_eq!(shares.get(&4321).copied(), Some(0.0), "a dash reads as nothing");
        assert_eq!(shares.len(), 2);
        assert!(parse_pmon("# header only\n").is_empty());
    }

    #[test]
    fn the_boot_time_and_a_process_start_time_give_its_age() {
        assert_eq!(parse_btime("cpu 1 2 3\nbtime 1700000000\n"), Some(1_700_000_000));
        assert_eq!(parse_btime("cpu 1\n"), None);
        let s = parse_stat("1234 (sh) S 1 1234 1234 0 -1 4194304 1 0 0 0 42 8 0 0 20 0 7 0 5000 1 1 1").unwrap();
        assert_eq!(s.start_ticks, 5000);
    }

    #[test]
    fn private_memory_adds_the_private_clean_and_dirty_pages() {
        let rollup = "Rss:  900 kB\nPss:  400 kB\nShared_Clean:  100 kB\nPrivate_Clean:  200 kB\nPrivate_Dirty:  300 kB\nAnonymous:  50 kB\n";
        assert_eq!(parse_private(rollup), Some(500 * 1024));
        assert_eq!(parse_private("Rss: 1 kB\n"), None, "no private lines, nothing to say");
    }

    #[test]
    fn the_rollup_gives_real_private_and_purgeable_memory() {
        let text = "Rss:  900 kB\nPss:  400 kB\nPss_Anon:  10 kB\nPrivate_Clean:  200 kB\nPrivate_Dirty:  300 kB\nLazyFree:  64 kB\n";
        let r = parse_rollup(text);
        assert_eq!((r.pss, r.private, r.lazy_free), (Some(400 * 1024), Some(500 * 1024), Some(64 * 1024)), "Pss_Anon is not the Pss line");
        assert_eq!(parse_rollup("Rss: 1 kB\n"), Rollup::default());
        assert_eq!(parse_rollup("Pss: lots\nLazyFree: 8 kB\n").pss, None, "a line that does not read is skipped");
    }

    #[test]
    fn an_elf_header_names_the_machine() {
        let header = |class: u8, data: u8, machine: [u8; 2]| {
            let mut h = vec![0x7f, b'E', b'L', b'F', class, data, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0];
            h.extend(machine);
            h
        };
        assert_eq!(parse_elf_kind(&header(2, 1, [0x3E, 0])), Some("x86_64"));
        assert_eq!(parse_elf_kind(&header(2, 1, [0xB7, 0])), Some("ARM64"));
        assert_eq!(parse_elf_kind(&header(1, 1, [0x03, 0])), Some("x86"));
        assert_eq!(parse_elf_kind(&header(2, 2, [0, 0x3E])), Some("x86_64"), "big-endian files read their machine the other way round");
        assert_eq!(parse_elf_kind(&header(2, 1, [0x99, 0])), Some("Other"));
        assert_eq!(parse_elf_kind(b"#!/bin/sh\nexit 0\n padding"), None);
        assert_eq!(parse_elf_kind(&[0x7f, b'E']), None);
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

    #[test]
    fn unmapped_owners_take_the_shells_answer() {
        use bifrost_core::json::parse;
        let answer = parse(r#"{"users":[{"uid":0,"user":"root","pids":[1,20]},{"uid":1000,"user":"olibuijr","pids":[300]}]}"#).unwrap();
        let owners = parse_owners(&answer);
        let users = parse_passwd("app:x:1500:1500::/:/bin/false\n");
        assert_eq!(owner_of(300, OVERFLOW_UID, &users, &owners), (1000, "olibuijr".to_string()), "nobody becomes the real owner");
        assert_eq!(owner_of(1, OVERFLOW_UID, &users, &owners), (0, "root".to_string()));
        assert_eq!(owner_of(99, OVERFLOW_UID, &users, &owners), (OVERFLOW_UID, "65534".to_string()), "no answer: unchanged");
        assert_eq!(owner_of(5, 1500, &users, &owners), (1500, "app".to_string()), "a mapped uid is trusted");
    }
}
