//! Mach-style host statistics for Linux (`host_statistics64`, `host_processor_info`, `task_info`, `os_proc_available_memory`
//! and the `ProcessInfo` memory/CPU counters), read from `/proc`. One code path for the shell and the SDK.
//!
//! Every call is one `open` + one `read` of a small procfs file. A [`HostSampler`] owns the read buffer and the
//! per-CPU vector, so a steady-state sample allocates nothing. There is no sampler thread: callers read at their
//! own deadline (the Golden Gate way: the counters are cheap, the caller picks the cadence).
//!
//! Mapping choices are documented in `docs/bifrost-ui.md`, "Host statistics and ProcessInfo".
use std::{fs::File, io::Read};

/// `host_cpu_load_info.cpu_ticks` (`CPU_STATE_USER`, `SYSTEM`, `IDLE`, `NICE`), in clock ticks (USER_HZ).
/// `system` folds in irq, softirq and steal; `idle` folds in iowait, as Mach counts waiting as idle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostCpuLoadInfo {
    pub user: u64,
    pub system: u64,
    pub idle: u64,
    pub nice: u64,
}

impl HostCpuLoadInfo {
    pub fn total(&self) -> u64 {
        self.user + self.system + self.idle + self.nice
    }
    /// Activity Monitor's "CPU load": busy share of the ticks between two samples, 0..=100. 0 when no tick passed.
    pub fn usage_percent(previous: &HostCpuLoadInfo, now: &HostCpuLoadInfo) -> f32 {
        let total = now.total().saturating_sub(previous.total());
        if total == 0 {
            return 0.0;
        }
        let idle = now.idle.saturating_sub(previous.idle).min(total);
        (100.0 * (total - idle) as f32 / total as f32).clamp(0.0, 100.0)
    }
}

/// `vm_statistics64`, in pages of [`vm_kernel_page_size`] (`pageins` and `pageouts` count since boot).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VmStatistics64 {
    /// MemFree
    pub free_count: u64,
    /// Active
    pub active_count: u64,
    /// Inactive
    pub inactive_count: u64,
    /// Kernel memory that cannot be reclaimed: Unevictable + KernelStack + PageTables + SUnreclaim.
    pub wire_count: u64,
    /// Compressed memory held by zswap (meminfo `Zswap`) or zram (`mm_stat` mem_used_total), as pages of RAM it occupies.
    pub compressor_page_count: u64,
    /// Linux has no speculative list: always 0.
    pub speculative_count: u64,
    /// File-backed cache (meminfo `Cached`); Mach's `external_page_count`.
    pub external_page_count: u64,
    /// vmstat `pgpgin` (KiB read from block devices) converted to pages.
    pub pageins: u64,
    /// vmstat `pgpgout`, as pages.
    pub pageouts: u64,
}

impl VmStatistics64 {
    /// Pages "in use" the way Golden Gate's Memory Used counts: app memory (active) + wired + compressed.
    pub fn used_count(&self) -> u64 {
        self.active_count + self.wire_count + self.compressor_page_count
    }
}

/// One meminfo read: the Mach counters plus the values Activity Monitor shows beside them, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemorySample {
    pub vm: VmStatistics64,
    /// `ProcessInfo.physicalMemory`
    pub physical: u64,
    /// MemAvailable
    pub available: u64,
    /// Cached
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl MemorySample {
    /// Golden Gate "Memory Used": (active + wired + compressed) bytes.
    pub fn used_bytes(&self) -> u64 {
        self.vm.used_count().saturating_mul(vm_kernel_page_size())
    }
    /// Memory Used as a share of physical memory, 0..=100.
    pub fn used_percent(&self) -> f32 {
        if self.physical == 0 {
            return 0.0;
        }
        (100.0 * self.used_bytes() as f64 / self.physical as f64).clamp(0.0, 100.0) as f32
    }
}

/// `task_vm_info`: `phys_footprint` is the memory the process is charged for, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskVmInfo {
    pub phys_footprint: u64,
}

extern "C" {
    fn sysconf(name: i32) -> i64;
    fn getloadavg(avg: *mut f64, n: i32) -> i32;
    #[cfg(target_env = "gnu")]
    fn malloc_trim(pad: usize) -> i32;
    #[cfg(target_env = "gnu")]
    fn mallopt(param: i32, value: i32) -> i32;
}

/// `malloc_zone_pressure_relief(NULL, 0)`: hands freed heap pages back to the kernel and returns how many
/// resident bytes that released. glibc keeps freed memory mapped for reuse, so a process that drew a
/// large frame or decoded a picture once stays that big until it trims; call this when going idle.
/// Pins glibc's mmap threshold at 256 KiB (a fixed value turns the dynamic threshold off), so a full-screen
/// buffer is always its own mapping and goes back to the kernel on free instead of growing `[heap]` (glibc raises
/// the threshold to the largest block freed, up to 32 MiB, and then serves 14 MiB frames from brk). Also trims
/// eagerly and keeps one arena. Call once at startup before the first big allocation.
pub fn tune_allocator() {
    #[cfg(target_env = "gnu")]
    // SAFETY: mallopt only sets allocator tunables; M_TRIM_THRESHOLD -1, M_MMAP_THRESHOLD -3, M_ARENA_MAX -8.
    unsafe {
        mallopt(-3, 256 * 1024);
        mallopt(-1, 1024 * 1024);
        mallopt(-8, 2);
    }
}

pub fn pressure_relief() -> u64 {
    let resident = || std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
    let before = resident();
    #[cfg(target_env = "gnu")]
    // SAFETY: malloc_trim only walks and releases the allocator's own free lists.
    unsafe {
        malloc_trim(0);
    }
    before.saturating_sub(resident()) * vm_kernel_page_size()
}
const SC_NPROCESSORS_CONF: i32 = 83;
const SC_NPROCESSORS_ONLN: i32 = 84;
const SC_PAGESIZE: i32 = 30;

/// `vm_kernel_page_size` / `vm_page_size`.
pub fn vm_kernel_page_size() -> u64 {
    static SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    // SAFETY: sysconf with a constant name has no memory effects.
    *SIZE.get_or_init(|| unsafe { sysconf(SC_PAGESIZE) }.max(4096) as u64)
}
/// `ProcessInfo.processorCount`.
pub fn processor_count() -> usize {
    // SAFETY: see above.
    (unsafe { sysconf(SC_NPROCESSORS_CONF) }).max(1) as usize
}
/// `ProcessInfo.activeProcessorCount`.
pub fn active_processor_count() -> usize {
    // SAFETY: see above.
    (unsafe { sysconf(SC_NPROCESSORS_ONLN) }).max(1) as usize
}
/// `getloadavg`: 1, 5 and 15 minute averages.
pub fn load_average() -> [f32; 3] {
    let mut a = [0f64; 3];
    // SAFETY: writes at most three doubles into `a`.
    let n = unsafe { getloadavg(a.as_mut_ptr(), 3) };
    if n < 3 {
        return [0.0; 3];
    }
    [a[0] as f32, a[1] as f32, a[2] as f32]
}

fn kb(line_value: &str) -> u64 {
    line_value.trim().trim_end_matches("kB").trim().parse::<u64>().unwrap_or(0).saturating_mul(1024)
}

/// Parses the `cpu` and `cpuN` lines of `/proc/stat`. Fills `per_cpu` (cleared first) and returns the aggregate.
pub fn parse_proc_stat(text: &str, per_cpu: &mut Vec<HostCpuLoadInfo>) -> Option<HostCpuLoadInfo> {
    per_cpu.clear();
    let mut aggregate = None;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            if aggregate.is_some() {
                break; // the cpu lines are contiguous at the top
            }
            continue;
        };
        let (name, nums) = rest.split_once(' ').unwrap_or((rest, ""));
        let mut v = [0u64; 8]; // user nice system idle iowait irq softirq steal
        for (slot, tok) in v.iter_mut().zip(nums.split_ascii_whitespace()) {
            *slot = tok.parse().ok()?;
        }
        let info = HostCpuLoadInfo { user: v[0], nice: v[1], system: v[2] + v[5] + v[6] + v[7], idle: v[3] + v[4] };
        if name.is_empty() {
            aggregate = Some(info);
        } else if name.bytes().all(|b| b.is_ascii_digit()) {
            per_cpu.push(info);
        }
    }
    aggregate
}

/// Parses `/proc/meminfo` into a [`MemorySample`] (paging counters stay 0; see [`parse_vmstat`]).
pub fn parse_meminfo(text: &str, page: u64) -> MemorySample {
    let (mut free, mut active, mut inactive, mut cached, mut total, mut avail) = (0, 0, 0, 0, 0, 0);
    let (mut unevict, mut kstack, mut ptables, mut sunreclaim, mut zswap, mut st, mut sf) = (0, 0, 0, 0, 0, 0, 0);
    for line in text.lines() {
        let Some((key, val)) = line.split_once(':') else { continue };
        let slot = match key {
            "MemTotal" => &mut total,
            "MemFree" => &mut free,
            "MemAvailable" => &mut avail,
            "Active" => &mut active,
            "Inactive" => &mut inactive,
            "Cached" => &mut cached,
            "Unevictable" => &mut unevict,
            "KernelStack" => &mut kstack,
            "PageTables" => &mut ptables,
            "SUnreclaim" => &mut sunreclaim,
            "Zswap" => &mut zswap,
            "SwapTotal" => &mut st,
            "SwapFree" => &mut sf,
            _ => continue,
        };
        *slot = kb(val);
    }
    let pages = |bytes: u64| bytes / page.max(1);
    MemorySample {
        vm: VmStatistics64 {
            free_count: pages(free),
            active_count: pages(active),
            inactive_count: pages(inactive),
            wire_count: pages(unevict + kstack + ptables + sunreclaim),
            compressor_page_count: pages(zswap),
            speculative_count: 0,
            external_page_count: pages(cached),
            pageins: 0,
            pageouts: 0,
        },
        physical: total,
        available: avail,
        cached,
        swap_total: st,
        swap_free: sf,
    }
}

/// Fills `pageins` and `pageouts` from `/proc/vmstat` text (`pgpgin` and `pgpgout` are KiB).
pub fn parse_vmstat(text: &str, page: u64, vm: &mut VmStatistics64) {
    let per_page = (page / 1024).max(1);
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("pgpgin", n)) => vm.pageins = n.trim().parse::<u64>().unwrap_or(0) / per_page,
            Some(("pgpgout", n)) => {
                vm.pageouts = n.trim().parse::<u64>().unwrap_or(0) / per_page;
                break; // pgpgout follows pgpgin
            }
            _ => {}
        }
    }
}

/// `task_vm_info.phys_footprint` from `smaps_rollup` text: `Pss_Anon + Pss_Shmem` (anonymous and shared-memory pages
/// the process is charged for, without file-backed code and cache, as Golden Gate's footprint excludes clean mapped files).
/// Kernels without those fields fall back to `Private_Dirty`.
pub fn parse_phys_footprint(text: &str) -> u64 {
    let (mut anon, mut shmem, mut dirty, mut seen) = (0, 0, 0, false);
    for line in text.lines() {
        let Some((key, val)) = line.split_once(':') else { continue };
        match key {
            "Pss_Anon" => {
                anon = kb(val);
                seen = true;
            }
            "Pss_Shmem" => shmem = kb(val),
            "Private_Dirty" => dirty = kb(val),
            _ => {}
        }
    }
    if seen {
        anon + shmem
    } else {
        dirty
    }
}

/// Reads host counters with a reusable buffer. Keep one per caller.
pub struct HostSampler {
    buf: Vec<u8>,
    per_cpu: Vec<HostCpuLoadInfo>,
    zram: Option<bool>,
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostSampler {
    pub fn new() -> Self {
        HostSampler { buf: Vec::with_capacity(8192), per_cpu: Vec::new(), zram: None }
    }

    /// One open and one read into the reused buffer.
    fn read(&mut self, path: &str) -> Option<&str> {
        self.buf.clear();
        File::open(path).ok()?.read_to_end(&mut self.buf).ok()?;
        std::str::from_utf8(&self.buf).ok()
    }

    /// `host_statistics64(HOST_CPU_LOAD_INFO)`. The per-CPU ticks of the same read are in [`Self::processor_info`].
    pub fn host_cpu_load_info(&mut self) -> Option<HostCpuLoadInfo> {
        let mut per_cpu = std::mem::take(&mut self.per_cpu);
        let r = self.read("/proc/stat").and_then(|t| parse_proc_stat(t, &mut per_cpu));
        self.per_cpu = per_cpu;
        r
    }
    /// `host_processor_info(PROCESSOR_CPU_LOAD_INFO)`: per-CPU ticks from the last `host_cpu_load_info` read.
    pub fn processor_info(&self) -> &[HostCpuLoadInfo] {
        &self.per_cpu
    }
    /// `host_statistics64(HOST_VM_INFO64)` plus the memory totals. Reads meminfo and vmstat (and zram once, if present).
    pub fn memory(&mut self) -> Option<MemorySample> {
        let page = vm_kernel_page_size();
        let mut m = self.read("/proc/meminfo").map(|t| parse_meminfo(t, page))?;
        let mut vm = m.vm;
        if let Some(t) = self.read("/proc/vmstat") {
            parse_vmstat(t, page, &mut vm);
        }
        if vm.compressor_page_count == 0 && self.zram != Some(false) {
            let used = self.read("/sys/block/zram0/mm_stat").and_then(|t| t.split_ascii_whitespace().nth(2)?.parse::<u64>().ok());
            self.zram = Some(used.is_some());
            vm.compressor_page_count = used.unwrap_or(0) / page;
        }
        m.vm = vm;
        Some(m)
    }
    /// `task_info(TASK_VM_INFO)` for the calling process.
    pub fn task_vm_info(&mut self) -> TaskVmInfo {
        TaskVmInfo { phys_footprint: self.read("/proc/self/smaps_rollup").map_or(0, parse_phys_footprint) }
    }
    /// `os_proc_available_memory`: bytes this process can still allocate. The cgroup limit minus its current use when a
    /// limit is set (capped by MemAvailable), otherwise MemAvailable.
    pub fn os_proc_available_memory(&mut self) -> u64 {
        let avail = self.read("/proc/meminfo").map_or(0, |t| parse_meminfo(t, 1).available);
        match self.cgroup_headroom() {
            Some(h) if avail > 0 => h.min(avail),
            Some(h) => h,
            None => avail,
        }
    }
    fn cgroup_headroom(&mut self) -> Option<u64> {
        let rel = self.read("/proc/self/cgroup")?.lines().find_map(|l| l.strip_prefix("0::")).map(str::to_owned)?;
        let dir = format!("/sys/fs/cgroup{}", rel.trim_end());
        let max = self.read(&format!("{dir}/memory.max"))?.trim().parse::<u64>().ok()?; // "max" means unlimited
        let cur = self.read(&format!("{dir}/memory.current"))?.trim().parse::<u64>().unwrap_or(0);
        Some(max.saturating_sub(cur))
    }
    /// `ProcessInfo.systemUptime`, seconds.
    pub fn system_uptime(&mut self) -> f64 {
        self.read("/proc/uptime").and_then(|t| t.split_ascii_whitespace().next()?.parse().ok()).unwrap_or(0.0)
    }
}

/// `ProcessInfo.physicalMemory`, bytes (one meminfo read).
pub fn physical_memory() -> u64 {
    HostSampler::new().memory().map_or(0, |m| m.physical)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 10 50 800 40 5 5 0 8 0\ncpu0 60 5 20 400 20 2 3 0 0 0\ncpu1 40 5 30 400 20 3 2 0 0 0\nintr 1\n";
    const MEMINFO: &str = "MemTotal:       16384000 kB\nMemFree:         1024000 kB\nMemAvailable:    8192000 kB\nCached:          4096000 kB\nActive:          5120000 kB\nInactive:        3072000 kB\nUnevictable:       10240 kB\nSwapTotal:       2048000 kB\nSwapFree:        1024000 kB\nKernelStack:        4096 kB\nPageTables:         8192 kB\nSUnreclaim:       102400 kB\nZswap:             40960 kB\n";

    #[test]
    fn proc_stat_gives_aggregate_and_per_cpu_ticks() {
        let mut cpus = Vec::new();
        let a = parse_proc_stat(STAT, &mut cpus).unwrap();
        assert_eq!(a, HostCpuLoadInfo { user: 100, nice: 10, system: 60, idle: 840 });
        assert_eq!(cpus.len(), 2);
        assert_eq!(cpus[1], HostCpuLoadInfo { user: 40, nice: 5, system: 35, idle: 420 });
        assert!(parse_proc_stat("intr 1\n", &mut cpus).is_none());
    }

    #[test]
    fn usage_is_the_busy_share_of_the_delta() {
        let a = HostCpuLoadInfo { user: 10, system: 10, idle: 80, nice: 0 };
        let b = HostCpuLoadInfo { user: 30, system: 20, idle: 150, nice: 0 };
        assert_eq!(HostCpuLoadInfo::usage_percent(&a, &b), 30.0);
        assert_eq!(HostCpuLoadInfo::usage_percent(&b, &b), 0.0);
        assert_eq!(HostCpuLoadInfo::usage_percent(&b, &a), 0.0, "a counter going backwards reads as idle");
    }

    #[test]
    fn meminfo_maps_to_vm_statistics_in_pages() {
        let m = parse_meminfo(MEMINFO, 4096);
        assert_eq!(m.physical, 16384000 * 1024);
        assert_eq!(m.vm.free_count, 1024000 / 4);
        assert_eq!(m.vm.active_count, 5120000 / 4);
        assert_eq!(m.vm.inactive_count, 3072000 / 4);
        assert_eq!(m.vm.wire_count, (10240 + 4096 + 8192 + 102400) / 4);
        assert_eq!(m.vm.compressor_page_count, 40960 / 4);
        assert_eq!((m.available, m.cached), (8192000 * 1024, 4096000 * 1024));
        assert_eq!((m.swap_total, m.swap_free), (2048000 * 1024, 1024000 * 1024));
    }

    #[test]
    fn memory_used_counts_active_wired_and_compressed() {
        let mut m = parse_meminfo(MEMINFO, vm_kernel_page_size());
        let want = (5120000 + 10240 + 4096 + 8192 + 102400 + 40960) as f64 / 16384000.0 * 100.0;
        assert!((m.used_percent() as f64 - want).abs() < 0.1);
        m.physical = 0;
        assert_eq!(m.used_percent(), 0.0);
    }

    #[test]
    fn pressure_relief_returns_freed_heap_to_the_kernel() {
        // 64 MiB of touched small blocks, freed: glibc keeps them mapped until a trim
        // the result is a whole-process RSS delta: parallel tests allocate meanwhile, so only its unit is checked
        // here; the release itself is measured on Midget (docs/snow-leopard.md, shell PSS)
        let blocks: Vec<Vec<u8>> = (0..16_384).map(|_| vec![1u8; 4096]).collect();
        drop(blocks);
        let released = pressure_relief();
        assert_eq!(released % vm_kernel_page_size(), 0);
    }

    #[test]
    fn vmstat_paging_counters_are_pages() {
        let mut vm = VmStatistics64::default();
        parse_vmstat("nr_free_pages 5\npgpgin 4000\npgpgout 8000\npgfault 9\n", 4096, &mut vm);
        assert_eq!((vm.pageins, vm.pageouts), (1000, 2000));
    }

    #[test]
    fn footprint_prefers_pss_anon_plus_shmem_then_private_dirty() {
        assert_eq!(parse_phys_footprint("Rss: 900 kB\nPss_Anon: 100 kB\nPss_Shmem: 20 kB\nPrivate_Dirty: 500 kB\n"), 120 * 1024);
        assert_eq!(parse_phys_footprint("Rss: 900 kB\nPrivate_Dirty: 500 kB\n"), 500 * 1024);
    }

    #[test]
    fn sampler_reads_this_host() {
        let mut s = HostSampler::new();
        assert!(s.host_cpu_load_info().unwrap().total() > 0);
        assert!(!s.processor_info().is_empty());
        assert!(s.memory().unwrap().physical > 0);
        assert!(processor_count() >= 1 && active_processor_count() >= 1);
    }
}
