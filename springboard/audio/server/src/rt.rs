//! Lowest-latency plumbing of the server (task 21): wake-up margin from measured jitter, timer slack, memory lock,
//! the PM QoS request on `/dev/cpu_dma_latency`, CPU affinity away from the sound card IRQ, the threaded IRQ
//! handler raised above the driver thread, and the sustained-xrun quantum fallback.
//!
//! The pure parts (`wake_margin_ns`, `JitterTracker`, `parse_cpu_list`, `parse_irq_thread`, `driver_cpus`,
//! `FallbackPolicy`) have host tests; the syscall wrappers are thin and each `unsafe` block has a SAFETY comment.

use crate::io::{log, set_realtime, IO_PRIORITY};
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

// ---------------------------------------------------------------------------------------------------------------
// Wake-up margin

/// The margin before any jitter has been measured.
pub const DEFAULT_MARGIN_NS: u64 = 100_000;
/// The smallest margin: a timer on an idle RT thread wakes about 20 us late at worst.
pub const MIN_MARGIN_NS: u64 = 30_000;
/// Cycles kept for the jitter percentile.
pub const JITTER_WINDOW: usize = 256;

/// The 99th percentile (nearest rank) of `samples`, 0 when empty.
pub fn p99(samples: &[u64]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut v = samples.to_vec();
    v.sort_unstable();
    let rank = (v.len() * 99).div_ceil(100);
    v[rank.max(1) - 1]
}

/// How long before the cycle start the timer is armed: the measured wake-up lateness (p99) plus 10 us, within
/// `MIN_MARGIN_NS` and half a quantum. The thread then spins from the wake-up to the exact start.
pub fn wake_margin_ns(late_samples: &[u64], quantum_ns: u64) -> u64 {
    let base = if late_samples.is_empty() { DEFAULT_MARGIN_NS } else { p99(late_samples) + 10_000 };
    base.max(MIN_MARGIN_NS).min((quantum_ns / 2).max(MIN_MARGIN_NS))
}

/// Records how late each timer wake-up was and derives the margin.
pub struct JitterTracker {
    late: Vec<u64>,
    next: usize,
}

impl JitterTracker {
    pub fn new() -> JitterTracker {
        JitterTracker { late: Vec::with_capacity(JITTER_WINDOW), next: 0 }
    }

    /// One wake-up: `late_ns` after the time the timer was armed for.
    pub fn record(&mut self, late_ns: u64) {
        if self.late.len() < JITTER_WINDOW {
            self.late.push(late_ns);
        } else {
            self.late[self.next] = late_ns;
        }
        self.next = (self.next + 1) % JITTER_WINDOW;
    }

    pub fn margin_ns(&self, quantum_ns: u64) -> u64 {
        wake_margin_ns(&self.late, quantum_ns)
    }
}

impl Default for JitterTracker {
    fn default() -> Self {
        JitterTracker::new()
    }
}

/// Busy-waits until `target_raw` (`CLOCK_MONOTONIC_RAW` ns) is reached; the spin is at most the margin.
pub fn spin_until(target_raw: u64) {
    while crate::io::now_raw_ns() < target_raw {
        std::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Process and thread settings

/// Timer slack of the calling thread to 1 ns (the default 50 us would be added to every timer wake-up).
pub fn set_timer_slack_1ns() {
    // SAFETY: PR_SET_TIMERSLACK takes an integer and affects only the calling thread.
    if unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, 1 as libc::c_ulong, 0, 0, 0) } != 0 {
        log("io", &format!("timer slack: {}", std::io::Error::last_os_error()));
    }
}

/// Locks all present and future pages. Call after setup, so what setup allocated is resident too.
pub fn lock_memory() {
    // SAFETY: mlockall takes flags only. Failure is logged: the daemon still works, without the guarantee.
    if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } != 0 {
        log("io", &format!("mlockall failed: {}", std::io::Error::last_os_error()));
    }
}

/// Pins the calling thread to `cpus`; an empty list leaves the affinity alone.
pub fn set_thread_affinity(cpus: &[usize]) -> std::io::Result<()> {
    if cpus.is_empty() {
        return Ok(());
    }
    // SAFETY: `set` is zero-initialised and only filled through CPU_SET within its bit count.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        for &c in cpus {
            if c >= 8 * std::mem::size_of::<libc::cpu_set_t>() {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
            libc::CPU_SET(c, &mut set);
        }
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The CPUs this process may run on.
pub fn allowed_cpus() -> Vec<usize> {
    // SAFETY: `set` is a valid out-buffer of the size passed.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set) < 0 {
            return Vec::new();
        }
        (0..8 * std::mem::size_of::<libc::cpu_set_t>()).filter(|&c| libc::CPU_ISSET(c, &set)).collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// IRQ affinity and threads

/// Parses a kernel CPU list such as `0-3,6`. Malformed items are skipped; the result is sorted and unique.
pub fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for item in s.trim().split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        match item.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    if a <= b && b - a < 4096 {
                        out.extend(a..=b);
                    }
                }
            }
            None => {
                if let Ok(n) = item.parse::<usize>() {
                    out.push(n);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// `irq/NN-name` (a threaded IRQ handler's comm) to `(NN, name)`.
pub fn parse_irq_thread(comm: &str) -> Option<(u32, &str)> {
    let rest = comm.trim().strip_prefix("irq/")?;
    let (n, name) = rest.split_once('-')?;
    Some((n.parse().ok()?, name))
}

/// True for the IRQ names of sound hardware (HDA, SOF; USB audio has none: it rides the USB controller).
pub fn is_sound_irq_name(name: &str) -> bool {
    name.contains("snd") || name.contains("sof") || name.contains("hda")
}

/// The CPUs for the driver thread: every allowed CPU except those that take the sound IRQ, so the two do not
/// preempt each other. With no other CPU left the full set is kept (affinity cannot help on one core).
pub fn driver_cpus(allowed: &[usize], irq_cpus: &[usize]) -> Vec<usize> {
    let rest: Vec<usize> = allowed.iter().copied().filter(|c| !irq_cpus.contains(c)).collect();
    if rest.is_empty() || irq_cpus.is_empty() {
        allowed.to_vec()
    } else {
        rest
    }
}

/// One threaded sound IRQ handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrqThread {
    pub irq: u32,
    pub tid: i32,
    pub cpus: Vec<usize>,
}

/// Finds the sound IRQ threads by reading `/proc/*/comm` (kernel threads: tid == pid) and
/// `/proc/irq/N/smp_affinity_list`. Empty when the kernel does not run threaded IRQs (no `threadirqs`) or no
/// sound IRQ is threaded.
pub fn scan_sound_irq_threads() -> Vec<IrqThread> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else { return out };
    for e in dir.flatten() {
        let Some(tid) = e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{tid}/comm")) else { continue };
        let Some((irq, name)) = parse_irq_thread(&comm) else { continue };
        if !is_sound_irq_name(name) {
            continue;
        }
        let cpus = std::fs::read_to_string(format!("/proc/irq/{irq}/smp_affinity_list")).map(|s| parse_cpu_list(&s)).unwrap_or_default();
        out.push(IrqThread { irq, tid, cpus });
    }
    out
}

/// The kernel's default priority of a threaded IRQ handler (`MAX_USER_RT_PRIO / 2`).
pub const IRQ_DEFAULT_PRIORITY: u32 = 50;
/// The sound IRQ thread runs one above the driver thread, so the position update is never held up by it.
pub const IRQ_PRIORITY: u32 = IO_PRIORITY + 1;

// ---------------------------------------------------------------------------------------------------------------
// PM QoS and the process-wide hold

/// What is held while any device runs.
struct Held {
    users: u32,
    /// `/dev/cpu_dma_latency`, open: the request lasts as long as the fd.
    _dma: Option<File>,
    irq: Vec<IrqThread>,
    driver_cpus: Vec<usize>,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// Opens `/dev/cpu_dma_latency` and writes a 4-byte 0: CPUs stay out of deep C-states while the fd is open. The
/// power cost: an idle package no longer sleeps below C1, a few watts on a laptop, which is why it is held only
/// while a device runs.
fn open_pm_qos() -> std::io::Result<File> {
    let mut f = std::fs::OpenOptions::new().write(true).open("/dev/cpu_dma_latency")?;
    f.write_all(&0i32.to_ne_bytes())?;
    Ok(f)
}

/// What the IO thread needs from the hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tuning {
    /// CPUs for the driver thread (empty: leave the affinity).
    pub cpus: Vec<usize>,
}

/// A device starts: the first one takes the PM QoS request and raises the sound IRQ thread.
pub fn device_started() -> Tuning {
    let mut g = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = g.as_mut() {
        h.users += 1;
        return Tuning { cpus: h.driver_cpus.clone() };
    }
    let dma = match open_pm_qos() {
        Ok(f) => Some(f),
        Err(e) => {
            log("io", &format!("cpu_dma_latency: {e} (CPUs may sleep deeply between cycles)"));
            None
        }
    };
    let irq = scan_sound_irq_threads();
    let mut irq_cpus = Vec::new();
    for t in &irq {
        irq_cpus.extend_from_slice(&t.cpus);
        match set_realtime(t.tid, IRQ_PRIORITY, 0) {
            Ok(()) => log("io", &format!("irq {} thread {} raised to SCHED_FIFO {IRQ_PRIORITY}", t.irq, t.tid)),
            Err(e) => log("io", &format!("irq {} thread {}: cannot raise priority ({e})", t.irq, t.tid)),
        }
    }
    irq_cpus.sort_unstable();
    irq_cpus.dedup();
    let cpus = driver_cpus(&allowed_cpus(), &irq_cpus);
    *g = Some(Held { users: 1, _dma: dma, irq, driver_cpus: cpus.clone() });
    Tuning { cpus }
}

/// A device stops: the last one releases the PM QoS request and puts the IRQ thread back.
pub fn device_stopped() {
    let mut g = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let Some(h) = g.as_mut() else { return };
    h.users = h.users.saturating_sub(1);
    if h.users > 0 {
        return;
    }
    for t in &h.irq {
        let _ = set_realtime(t.tid, IRQ_DEFAULT_PRIORITY, 0);
    }
    *g = None; // drops the fd: the request ends
}

// ---------------------------------------------------------------------------------------------------------------
// Quantum fallback

/// The quantum above which the fallback does not go.
pub const FALLBACK_CEILING: u32 = 512;
/// Bad cycles (device xrun or late client) in one window that count as sustained.
pub const FALLBACK_LIMIT: u32 = 8;
/// The window, in milliseconds.
pub const FALLBACK_WINDOW_MS: u64 = 1000;

/// Cycles in the fallback window at this quantum and rate.
pub fn window_cycles(quantum: u32, rate: u32) -> u32 {
    let q = quantum.max(1) as u64;
    (FALLBACK_WINDOW_MS * rate as u64 / 1000).div_ceil(q).max(1) as u32
}

/// The next quantum after a sustained overload: doubled, up to `FALLBACK_CEILING`; `None` when already there.
pub fn doubled(quantum: u32) -> Option<u32> {
    (quantum < FALLBACK_CEILING).then(|| (quantum * 2).min(FALLBACK_CEILING))
}

/// Counts bad cycles per window; asks for a doubled quantum when `FALLBACK_LIMIT` fall in one window.
#[derive(Debug, Default, Clone)]
pub struct FallbackPolicy {
    cycles: u32,
    bad: u32,
}

impl FallbackPolicy {
    pub fn new() -> FallbackPolicy {
        FallbackPolicy::default()
    }

    /// One cycle. `window` is `window_cycles` for the present quantum. Returns the new quantum when the policy
    /// fires (the counters then restart).
    pub fn record(&mut self, bad: bool, quantum: u32, window: u32) -> Option<u32> {
        self.cycles += 1;
        if bad {
            self.bad += 1;
        }
        if self.bad >= FALLBACK_LIMIT {
            *self = FallbackPolicy::default();
            return doubled(quantum);
        }
        if self.cycles >= window {
            *self = FallbackPolicy::default();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p99_uses_the_nearest_rank() {
        assert_eq!(p99(&[]), 0);
        assert_eq!(p99(&[7]), 7);
        let mut v = vec![10u64; 99];
        v.push(200);
        // rank = ceil(100 * 0.99) = 99: the 99th smallest; the outlier is the 100th
        assert_eq!(p99(&v), 10);
        let mut w = vec![10u64; 98];
        w.extend([200, 300]);
        assert_eq!(p99(&w), 200);
    }

    #[test]
    fn the_margin_follows_jitter_within_bounds() {
        let q = 5_333_333;
        assert_eq!(wake_margin_ns(&[], q), DEFAULT_MARGIN_NS);
        assert_eq!(wake_margin_ns(&[5_000; 100], q), MIN_MARGIN_NS); // 15 us, raised to the floor
        assert_eq!(wake_margin_ns(&[400_000; 100], q), 410_000);
        assert_eq!(wake_margin_ns(&[9_000_000; 100], q), 2_666_666); // half a quantum
        assert_eq!(wake_margin_ns(&[400_000; 100], 333_333), 166_666); // 16 frames: half is 166 us
    }

    #[test]
    fn the_tracker_keeps_the_last_window() {
        let mut t = JitterTracker::new();
        assert_eq!(t.margin_ns(5_333_333), DEFAULT_MARGIN_NS);
        for _ in 0..JITTER_WINDOW {
            t.record(300_000);
        }
        assert_eq!(t.margin_ns(5_333_333), 310_000);
        for _ in 0..JITTER_WINDOW {
            t.record(50_000);
        }
        assert_eq!(t.margin_ns(5_333_333), 60_000);
    }

    #[test]
    fn cpu_lists_parse() {
        assert_eq!(parse_cpu_list("0-3,6\n"), vec![0, 1, 2, 3, 6]);
        assert_eq!(parse_cpu_list("2"), vec![2]);
        assert_eq!(parse_cpu_list("3,1,1-2"), vec![1, 2, 3]);
        assert_eq!(parse_cpu_list("x,4-,5-4,7"), vec![7]);
        assert!(parse_cpu_list("").is_empty());
    }

    #[test]
    fn irq_thread_names_parse() {
        assert_eq!(parse_irq_thread("irq/33-snd_hda_intel\n"), Some((33, "snd_hda_intel")));
        assert_eq!(parse_irq_thread("irq/9-acpi"), Some((9, "acpi")));
        assert_eq!(parse_irq_thread("kworker/0:1"), None);
        assert_eq!(parse_irq_thread("irq/x-snd"), None);
        assert!(is_sound_irq_name("snd_hda_intel"));
        assert!(is_sound_irq_name("snd_hda_intel:card0"));
        assert!(!is_sound_irq_name("xhci_hcd"));
    }

    #[test]
    fn the_driver_leaves_the_irq_cpus() {
        assert_eq!(driver_cpus(&[0, 1, 2, 3], &[1]), vec![0, 2, 3]);
        assert_eq!(driver_cpus(&[0, 1, 2, 3], &[0, 1, 2, 3]), vec![0, 1, 2, 3]);
        assert_eq!(driver_cpus(&[0, 1], &[]), vec![0, 1]);
        assert_eq!(driver_cpus(&[0], &[0]), vec![0]);
        assert_eq!(driver_cpus(&[2, 3], &[0]), vec![2, 3]);
    }

    #[test]
    fn priorities_order_irq_over_driver_over_clients() {
        assert!(IRQ_PRIORITY > IO_PRIORITY);
        assert!(IO_PRIORITY > crate::io::CLIENT_PRIORITY);
    }

    #[test]
    fn window_and_doubling() {
        assert_eq!(window_cycles(256, 48_000), 188); // 48000 / 256 = 187.5
        assert_eq!(window_cycles(16, 48_000), 3000);
        assert_eq!(doubled(16), Some(32));
        assert_eq!(doubled(256), Some(512));
        assert_eq!(doubled(300), Some(512));
        assert_eq!(doubled(512), None);
    }

    #[test]
    fn the_fallback_fires_on_sustained_xruns_only() {
        let mut p = FallbackPolicy::new();
        for _ in 0..7 {
            assert_eq!(p.record(true, 32, 1500), None);
        }
        assert_eq!(p.record(true, 32, 1500), Some(64));
        // counters restarted: a lone glitch does not fire
        assert_eq!(p.record(true, 64, 750), None);
        // spread thin over several windows: never fires
        let mut q = FallbackPolicy::new();
        for i in 0..100 {
            assert_eq!(q.record(i % 10 == 0, 64, 20), None);
        }
        // at the ceiling it fires but there is nowhere to go
        let mut r = FallbackPolicy::new();
        for _ in 0..7 {
            assert_eq!(r.record(true, 512, 100), None);
        }
        assert_eq!(r.record(true, 512, 100), None);
    }
}
