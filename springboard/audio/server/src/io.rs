//! The IO cycle (docs/audio-architecture.md, "IO cycle"): one `DeviceRunner` per running device, driven either by
//! the IO thread (timerfd, SCHED_FIFO) or, in tests, step by step with `cycle(now_ns)` on a `NullDriver`.
//!
//! Per cycle: (1) apply a pending quantum change, (2) read the hardware position and publish the zero timestamp,
//! (3) ReadInput into the clients' input rings, (4) trigger the clients through their activation records and wait
//! for them until the budget is spent, (5) MixOutput from the clients' output rings (a client without a full
//! quantum is skipped, counted and told; the device never waits for it), (6) WriteMix to the driver, (7) when the
//! driver restarted the stream on its own (xrun or suspend recovery), re-anchor the timeline and republish the zero
//! timestamp with a new seed.
//!
//! The hot path (`cycle`) never allocates, locks (the client list is read with `try_lock` and only when its
//! generation changed) or makes a syscall other than the futex. `unsafe` is confined to the shared-memory accessors
//! of `ClientSlot` and the scheduling syscalls; each block has a SAFETY comment. Owner: task 05.

use crate::mixer::{self, VolumeRamp, RAMP_FRAMES};
use bifrost_audio_driver::{DeviceIo, DriverError, HwPosition, ZeroTimeStamp};
use bifrost_audio_shm::{self as shm, Activation, Region, Ring, RingHeader, ZeroStamp, ZeroStampValue};
use bifrost_audio_types::control::ShmLayout;
use bifrost_audio_types::{Buffer, BufferList};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The most clients one server runs.
pub const MAX_CLIENTS: usize = 64;
/// The most channels one client stream or device has.
pub const MAX_CH: usize = bifrost_audio_types::control::MAX_CLIENT_CHANNELS;
pub const MIN_QUANTUM: u32 = 16;
pub const MAX_QUANTUM: u32 = 4096;
pub const DEFAULT_QUANTUM: u32 = 256;
/// SCHED_FIFO priority of the device IO threads (Decisions, 2).
pub const IO_PRIORITY: u32 = 88;
/// SCHED_FIFO priority granted to client IO threads.
pub const CLIENT_PRIORITY: u32 = 80;

/// Unified Logging, subsystem `is.olibuijr.audio` (categories `io`, `driver`, `client`, `policy`, `xrun`).
/// The shared logger is not a dependency of this crate, so this is the stderr form with the category prefix.
pub fn log(category: &str, msg: &str) {
    eprintln!("[is.olibuijr.audio/{category}] {msg}");
}

/// Host time: `CLOCK_MONOTONIC_RAW` in nanoseconds.
pub fn now_raw_ns() -> u64 {
    shm::now_ns()
}

// ---------------------------------------------------------------------------------------------------------------
// Real-time scheduling

#[repr(C)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;

/// Puts thread `tid` (0 = the caller) on SCHED_FIFO at `priority` with `sched_util_min` = `util_min` (the CPU
/// frequency hint, 0..=1024). Falls back to plain SCHED_FIFO when the clamp is refused. Needs CAP_SYS_NICE.
pub fn set_realtime(tid: i32, priority: u32, util_min: u32) -> std::io::Result<()> {
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_policy: libc::SCHED_FIFO as u32,
        sched_flags: SCHED_FLAG_UTIL_CLAMP_MIN,
        sched_nice: 0,
        sched_priority: priority,
        sched_runtime: 0,
        sched_deadline: 0,
        sched_period: 0,
        sched_util_min: util_min,
        sched_util_max: 1024,
    };
    // SAFETY: `attr` is a fully initialised `struct sched_attr` of the size it declares; the kernel only reads it.
    let r = unsafe { libc::syscall(libc::SYS_sched_setattr, tid as libc::c_long, &attr as *const SchedAttr, 0 as libc::c_long) };
    if r == 0 {
        return Ok(());
    }
    let sp = libc::sched_param { sched_priority: priority as i32 };
    // SAFETY: `sp` is a valid `sched_param` for the call.
    let r = unsafe { libc::sched_setscheduler(tid, libc::SCHED_FIFO, &sp) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Client slots

/// One client IO procedure on one device: its shared memory and the server's view of its state. Shared between the
/// control thread (which creates and removes it) and the device's IO thread (which runs it).
pub struct ClientSlot {
    pub id: u32,
    /// The connection that owns it.
    pub owner: u32,
    pub input_channels: u32,
    pub output_channels: u32,
    pub buffer_frames: u32,
    pub ring_frames: u32,
    /// Layout of the client's memfd as sent to the client (`total` includes the done record).
    pub layout: ShmLayout,
    /// Offset of the "done" activation (`layout.done`, after the rings): the client's `finish` targets it.
    pub done_offset: u32,
    pub region: Region,
    pub wake_fd: OwnedFd,
    pub running: AtomicBool,
    /// `f32::to_bits` of the stream volume (0..=1 scalar).
    pub volume: AtomicU32,
    pub muted: AtomicBool,
    /// Deadline misses not yet reported to the client.
    pub overloads: AtomicU32,
    pub missed_total: AtomicU64,
}

impl ClientSlot {
    pub fn new(
        id: u32,
        owner: u32,
        input_channels: u32,
        output_channels: u32,
        buffer_frames: u32,
        ring_frames: u32,
    ) -> std::io::Result<ClientSlot> {
        let layout = shm::layout(input_channels, output_channels, ring_frames);
        let done_offset = layout.done;
        let region = Region::create("bifrost-audio-client", layout.total as usize)?;
        // SAFETY: eventfd takes no pointers; the result is checked before it is owned.
        let efd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if efd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `efd` is a fresh descriptor that nothing else owns.
        let wake_fd = unsafe { OwnedFd::from_raw_fd(efd) };
        let slot = ClientSlot {
            id,
            owner,
            input_channels,
            output_channels,
            buffer_frames,
            ring_frames,
            layout,
            done_offset,
            region,
            wake_fd,
            running: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            overloads: AtomicU32::new(0),
            missed_total: AtomicU64::new(0),
        };
        // SAFETY: the region was just created, is zeroed, no other process has it yet, and the offsets come from
        // `shm::layout` (64-byte aligned, in bounds). Plain fields are written through raw pointers before any
        // reference to them exists.
        unsafe {
            let base = slot.region.addr;
            (*((base + layout.activation as usize) as *mut Activation)).required = 1;
            (*((base + done_offset as usize) as *mut Activation)).required = 1;
            if input_channels > 0 {
                let h = (base + layout.input_ring as usize) as *mut RingHeader;
                (*h).frames = ring_frames;
                (*h).channels = input_channels;
            }
            if output_channels > 0 {
                let h = (base + layout.output_ring as usize) as *mut RingHeader;
                (*h).frames = ring_frames;
                (*h).channels = output_channels;
            }
        }
        Ok(slot)
    }

    pub fn activation(&self) -> &Activation {
        self.region.get(self.layout.activation)
    }

    pub fn done(&self) -> &Activation {
        self.region.get(self.done_offset)
    }

    pub fn zero(&self) -> &ZeroStamp {
        self.region.get(self.layout.zero_stamp)
    }

    /// A ring view sized from the server's own `channels` and `ring_frames`; the header's size fields (in memory the
    /// client can rewrite) are never read.
    fn ring_at(&self, offset: u32, channels: u32) -> Option<Ring<'_>> {
        self.region.ring_trusted(offset, channels, self.ring_frames)
    }

    pub fn input_ring(&self) -> Option<Ring<'_>> {
        if self.input_channels == 0 {
            return None;
        }
        self.ring_at(self.layout.input_ring, self.input_channels)
    }

    pub fn output_ring(&self) -> Option<Ring<'_>> {
        if self.output_channels == 0 {
            return None;
        }
        self.ring_at(self.layout.output_ring, self.output_channels)
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn raw_wake_fd(&self) -> i32 {
        self.wake_fd.as_raw_fd()
    }

    fn miss(&self) {
        self.overloads.fetch_add(1, Ordering::Relaxed);
        self.missed_total.fetch_add(1, Ordering::Relaxed);
    }
}

/// The client list of a device, published by the control thread and snapshotted by the IO thread. Old snapshots are
/// kept in `retired` until the IO thread has dropped them, so the IO thread never frees one.
#[derive(Default)]
struct ClientList {
    current: Arc<Vec<Arc<ClientSlot>>>,
    retired: Vec<Arc<Vec<Arc<ClientSlot>>>>,
}

/// State shared between the control thread and a device's IO thread.
pub struct Shared {
    list: Mutex<ClientList>,
    generation: AtomicU32,
    /// The graph quantum the IO thread should use from its next cycle.
    pub quantum: AtomicU32,
    pub stop: AtomicBool,
    pub cycles: AtomicU64,
    pub xruns: AtomicU64,
    pub late_total: AtomicU64,
    /// Stream restarts the driver did on its own (xrun or suspend recovery), each a new timeline.
    pub restarts: AtomicU64,
    /// The quantum the sustained-xrun fallback raised to (0 = none); the control thread never goes below it.
    pub fallback_floor: AtomicU32,
    /// Event counters the real-time thread bumps instead of logging (a log line allocates and writes to a pipe);
    /// a normal-priority thread drains them once a second (`spawn_event_logger`).
    pub ev_device_xruns: AtomicU64,
    pub ev_late_cycles: AtomicU64,
    pub ev_restarts: AtomicU64,
    /// Number of sustained-xrun fallbacks, and the quantum the last one raised to.
    pub ev_fallbacks: AtomicU64,
    pub ev_fallback_quantum: AtomicU32,
}

impl Shared {
    pub fn new(quantum: u32) -> Arc<Shared> {
        Arc::new(Shared {
            list: Mutex::new(ClientList::default()),
            generation: AtomicU32::new(0),
            quantum: AtomicU32::new(quantum),
            stop: AtomicBool::new(false),
            cycles: AtomicU64::new(0),
            xruns: AtomicU64::new(0),
            late_total: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            fallback_floor: AtomicU32::new(0),
            ev_device_xruns: AtomicU64::new(0),
            ev_late_cycles: AtomicU64::new(0),
            ev_restarts: AtomicU64::new(0),
            ev_fallbacks: AtomicU64::new(0),
            ev_fallback_quantum: AtomicU32::new(0),
        })
    }

    /// Replaces the client list (control thread).
    pub fn publish(&self, slots: Vec<Arc<ClientSlot>>) {
        let mut l = self.list.lock().unwrap();
        let old = std::mem::replace(&mut l.current, Arc::new(slots));
        l.retired.push(old);
        l.retired.retain(|a| Arc::strong_count(a) > 1);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn slots(&self) -> Vec<Arc<ClientSlot>> {
        self.list.lock().unwrap().current.as_ref().clone()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The device runner

/// How step 4 waits for clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitPolicy {
    /// Do not wait (deterministic tests: the clients' output is already in the rings).
    None,
    /// Sleep on each triggered client's done word until its output is ready or `budget_ns` after the trigger.
    Futex { budget_ns: u64 },
}

#[derive(Debug, Clone, Copy)]
pub struct RunnerConfig {
    pub rate: u32,
    pub in_channels: u32,
    pub out_channels: u32,
    pub wait: WaitPolicy,
}

#[derive(Clone, Copy)]
struct SlotState {
    id: u32,
    ramp: VolumeRamp,
    active: bool,
    start_cycle: u64,
    every: u64,
    trigger: bool,
}

/// What one cycle did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CycleReport {
    /// Frames of this cycle (the quantum used).
    pub frames: u32,
    pub quantum_changed: bool,
    /// Clients skipped because they had no full quantum ready.
    pub late: u32,
    /// Clients triggered this cycle.
    pub triggered: u32,
    pub device_xrun: bool,
    /// The driver restarted the stream during this cycle: the timeline was re-anchored on its new zero timestamp.
    pub restarted: bool,
    /// The driver-clock time at which the next cycle starts.
    pub next_ns: u64,
}

pub struct DeviceRunner {
    io: Box<dyn DeviceIo>,
    shared: Arc<Shared>,
    cfg: RunnerConfig,
    quantum: u32,
    cycle: u64,
    timeline_frames: u64,
    start_host: u64,
    seed: u32,
    drv: Box<Activation>,
    snapshot: Arc<Vec<Arc<ClientSlot>>>,
    gen_seen: u32,
    states: Vec<SlotState>,
    spare: Vec<SlotState>,
    master: BufferList,
    in_buf: BufferList,
    scratch: Vec<Vec<f32>>,
    zeros: Vec<f32>,
}

fn planes(n: usize) -> BufferList {
    BufferList {
        buffers: (0..n).map(|_| Buffer { channels: 1, data: vec![0.0; MAX_QUANTUM as usize] }).collect(),
    }
}

impl DeviceRunner {
    pub fn new(io: Box<dyn DeviceIo>, shared: Arc<Shared>, cfg: RunnerConfig) -> DeviceRunner {
        let quantum = shared.quantum.load(Ordering::Acquire).clamp(MIN_QUANTUM, MAX_QUANTUM);
        DeviceRunner {
            io,
            shared,
            cfg,
            quantum,
            cycle: 0,
            timeline_frames: 0,
            start_host: 0,
            seed: 1,
            drv: Box::default(),
            snapshot: Arc::default(),
            gen_seen: 0,
            states: Vec::with_capacity(MAX_CLIENTS),
            spare: Vec::with_capacity(MAX_CLIENTS),
            master: planes(cfg.out_channels as usize),
            in_buf: planes(cfg.in_channels as usize),
            scratch: (0..MAX_CH).map(|_| vec![0.0; MAX_QUANTUM as usize]).collect(),
            zeros: vec![0.0; MAX_QUANTUM as usize],
        }
    }

    /// Starts the hardware; returns the driver-clock time of cycle 0.
    pub fn start(&mut self) -> Result<u64, DriverError> {
        let z = self.io.start()?;
        self.start_host = z.host_ns;
        self.timeline_frames = 0;
        self.cycle = 0;
        Ok(z.host_ns)
    }

    pub fn stop(&mut self) {
        self.io.stop();
    }

    pub fn quantum(&self) -> u32 {
        self.quantum
    }

    pub fn cycles(&self) -> u64 {
        self.cycle
    }

    /// The zero-timestamp seed: it changes whenever the timeline starts over, so clients drop their extrapolation.
    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn latency(&self) -> bifrost_audio_driver::Latency {
        self.io.latency()
    }

    /// The host time of the next cycle: the driver's DLL prediction once it has locked, else the nominal timeline.
    pub fn next_cycle_ns(&self) -> u64 {
        self.io.host_time_of(self.timeline_frames).unwrap_or_else(|| self.next_ns())
    }

    /// Tells every client of the device that the server overloaded (delivered as `Reply::Overload`).
    pub fn notify_overload(&self) {
        for s in self.snapshot.iter() {
            s.overloads.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn next_ns(&self) -> u64 {
        self.start_host + (self.timeline_frames as u128 * 1_000_000_000 / self.cfg.rate as u128) as u64
    }

    /// Picks up a new client list when the control thread published one (never blocks).
    fn refresh_snapshot(&mut self) {
        let g = self.shared.generation.load(Ordering::Acquire);
        if g == self.gen_seen {
            return;
        }
        let Ok(l) = self.shared.list.try_lock() else { return };
        self.snapshot = Arc::clone(&l.current);
        drop(l);
        self.gen_seen = g;
        self.spare.clear();
        for s in self.snapshot.iter().take(MAX_CLIENTS) {
            let st = self.states.iter().find(|st| st.id == s.id).copied().unwrap_or(SlotState {
                id: s.id,
                ramp: VolumeRamp::new(if s.muted.load(Ordering::Relaxed) { 0.0 } else { s.volume() }),
                active: false,
                start_cycle: 0,
                every: 1,
                trigger: false,
            });
            self.spare.push(st);
        }
        std::mem::swap(&mut self.states, &mut self.spare);
    }

    fn publish_zero(&self, pos: &HwPosition) {
        let span = pos.host_ns.saturating_sub(self.start_host);
        let rate_scalar = if span >= 100_000_000 {
            pos.hw_frames as f64 / (self.cfg.rate as f64 * span as f64 / 1e9)
        } else {
            1.0
        };
        let v = ZeroStampValue { sample_time: pos.hw_frames, host_ns: pos.host_ns, rate_scalar, seed: self.seed };
        for s in self.snapshot.iter() {
            s.zero().publish(v);
        }
    }

    /// Re-anchors the timeline on the zero timestamp of a stream restart the driver did on its own: the next cycle
    /// is the stamp's frame (cycles follow at the quantum from there), and every client gets the new zero timestamp
    /// at once, with a new seed and a rate scalar of 1 (the DLL starts over too).
    fn reanchor(&mut self, z: ZeroTimeStamp) {
        let rate = self.cfg.rate.max(1) as u128;
        self.start_host = z.host_ns.saturating_sub((z.sample_time as u128 * 1_000_000_000 / rate) as u64);
        self.timeline_frames = z.sample_time;
        self.seed = self.seed.wrapping_add(1);
        self.shared.restarts.fetch_add(1, Ordering::Relaxed);
        let v = ZeroStampValue { sample_time: z.sample_time, host_ns: z.host_ns, rate_scalar: 1.0, seed: self.seed };
        for s in self.snapshot.iter() {
            s.zero().publish(v);
        }
    }

    /// Runs one cycle at host time `now_ns`. See the module comment for the steps.
    pub fn cycle(&mut self, now_ns: u64) -> CycleReport {
        let want = self.shared.quantum.load(Ordering::Acquire).clamp(MIN_QUANTUM, MAX_QUANTUM);
        let quantum_changed = want != self.quantum;
        self.quantum = want;
        let q = want as usize;
        let rate = self.cfg.rate;
        let mut report = CycleReport { frames: want, quantum_changed, ..Default::default() };

        self.refresh_snapshot();

        // 2. position and zero timestamp
        let pos = self.io.position();
        self.publish_zero(&pos);

        // 3. ReadInput: one read for the device, copied into every running client's input ring.
        let dev_in = self.cfg.in_channels as usize;
        if dev_in > 0
            && self.snapshot.iter().any(|s| s.input_channels > 0 && s.running.load(Ordering::Acquire))
        {
            if self.io.read_input(want, &mut self.in_buf).is_err() {
                self.shared.xruns.fetch_add(1, Ordering::Relaxed);
                report.device_xrun = true;
            }
            for s in self.snapshot.iter() {
                if s.input_channels == 0 || !s.running.load(Ordering::Acquire) {
                    continue;
                }
                if let Some(ring) = s.input_ring() {
                    if ring.writable() as usize >= q {
                        let bufs = &self.in_buf.buffers;
                        let src: [&[f32]; MAX_CH] = std::array::from_fn(|i| &bufs[i % dev_in].data[..q]);
                        ring.write(&src[..s.input_channels as usize], want);
                    }
                }
            }
        }

        // 4. trigger the clients
        let cycle = self.cycle;
        for (s, st) in self.snapshot.iter().zip(self.states.iter_mut()) {
            if !s.running.load(Ordering::Acquire) {
                st.active = false;
                continue;
            }
            let every = ((s.buffer_frames as u64 + q as u64 - 1) / q as u64).max(1);
            if !st.active {
                st.active = true;
                st.start_cycle = cycle;
                if every > 1 {
                    if let Some(ring) = s.output_ring() {
                        // Pre-roll so the first cycles of a large-buffer client find data.
                        let mut left = (every as usize - 1) * q;
                        while left > 0 && ring.writable() as usize >= q {
                            let n = left.min(q);
                            let z: [&[f32]; MAX_CH] = std::array::from_fn(|_| &self.zeros[..n]);
                            ring.write(&z[..s.output_channels as usize], n as u32);
                            left -= n;
                        }
                    }
                }
            }
            st.every = every;
            st.trigger = (cycle - st.start_cycle) % every == 0;
            if st.trigger {
                shm::reset(s.activation(), want, rate);
                shm::reset(s.done(), want, rate);
                shm::finish(&self.drv, &[s.activation()], now_ns);
                report.triggered += 1;
            }
        }
        if let WaitPolicy::Futex { budget_ns } = self.cfg.wait {
            // The budget runs on the host clock, whatever clock the driver's `now_ns` is on.
            let deadline = now_raw_ns() + budget_ns;
            for (s, st) in self.snapshot.iter().zip(self.states.iter()) {
                if !(st.active && st.trigger && st.every == 1) {
                    continue;
                }
                loop {
                    let seen = s.done().wake.load(Ordering::Acquire);
                    // The client's `finish` decrements the done record's pending to 0 and wakes `wake`; its output
                    // is in the ring by then. A ring that is already full needs no wait either.
                    let finished = s.done().pending.load(Ordering::Acquire) == 0;
                    let ready = finished || s.output_ring().is_some_and(|r| r.readable() as usize >= q);
                    let t = now_raw_ns();
                    if ready || t >= deadline {
                        break;
                    }
                    shm::futex::wait(&s.done().wake, seen, deadline - t);
                }
            }
        }

        // 5. MixOutput
        mixer::clear_master(&mut self.master, q);
        for (s, st) in self.snapshot.iter().zip(self.states.iter_mut()) {
            if !st.active || s.output_channels == 0 {
                continue;
            }
            let ring = s.output_ring().unwrap();
            let ch = s.output_channels as usize;
            if st.every == 1 {
                // A client that was late left a stale quantum behind: drop it to stay in step.
                while ring.readable() as usize >= 2 * q {
                    let mut it = self.scratch.iter_mut();
                    let mut d: [&mut [f32]; MAX_CH] = std::array::from_fn(|_| &mut it.next().unwrap()[..q]);
                    ring.read(&mut d[..ch], want);
                }
            }
            if (ring.readable() as usize) < q {
                s.miss();
                self.shared.late_total.fetch_add(1, Ordering::Relaxed);
                report.late += 1;
                continue;
            }
            let mut it = self.scratch.iter_mut();
            let mut d: [&mut [f32]; MAX_CH] = std::array::from_fn(|_| &mut it.next().unwrap()[..q]);
            ring.read(&mut d[..ch], want);
            let target = if s.muted.load(Ordering::Relaxed) { 0.0 } else { s.volume() };
            if st.ramp.target() != target {
                st.ramp.set_target(target, RAMP_FRAMES);
            }
            mixer::mix_stream(&mut self.master, &d[..ch], &mut st.ramp, q);
        }

        // 6. ConvertMix and WriteMix (the driver converts to its physical format)
        if self.cfg.out_channels > 0 && self.io.write_mix(want, &self.master).is_err() {
            self.shared.xruns.fetch_add(1, Ordering::Relaxed);
            report.device_xrun = true;
        }

        self.cycle += 1;
        self.shared.cycles.store(self.cycle, Ordering::Relaxed);
        // 7. A restart the driver did on its own (in `read_input` or `write_mix`) starts a new timeline.
        match self.io.take_restart() {
            Some(z) => {
                self.reanchor(z);
                report.restarted = true;
            }
            None => self.timeline_frames += q as u64,
        }
        report.next_ns = self.next_ns();
        report
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The IO thread

/// What `drain_events` saw last time, to report only the change.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventMark {
    device_xruns: u64,
    late_cycles: u64,
    restarts: u64,
    fallbacks: u64,
}

/// The log lines for the events since `mark` (empty when nothing happened); updates `mark`. Called off the real-time
/// thread, so it may allocate.
pub fn drain_events(name: &str, shared: &Shared, mark: &mut EventMark) -> Vec<(&'static str, String)> {
    let now = EventMark {
        device_xruns: shared.ev_device_xruns.load(Ordering::Relaxed),
        late_cycles: shared.ev_late_cycles.load(Ordering::Relaxed),
        restarts: shared.ev_restarts.load(Ordering::Relaxed),
        fallbacks: shared.ev_fallbacks.load(Ordering::Relaxed),
    };
    let mut out = Vec::new();
    if now.device_xruns > mark.device_xruns {
        out.push(("xrun", format!("{name}: {} device xrun(s)", now.device_xruns - mark.device_xruns)));
    }
    if now.restarts > mark.restarts {
        out.push(("driver", format!("{name}: {} stream restart(s), timeline re-anchored", now.restarts - mark.restarts)));
    }
    if now.late_cycles > mark.late_cycles {
        out.push(("xrun", format!("{name}: {} cycle(s) with late clients", now.late_cycles - mark.late_cycles)));
    }
    if now.fallbacks > mark.fallbacks {
        let q = shared.ev_fallback_quantum.load(Ordering::Relaxed);
        out.push(("xrun", format!("{name}: sustained xruns, quantum now {q}")));
    }
    *mark = now;
    out
}

/// A normal-priority thread that logs the real-time thread's events at most once a second, until `shared.stop`.
fn spawn_event_logger(name: String, shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name(format!("{name}-log")).spawn(move || {
        let mut mark = EventMark::default();
        let mut ticks = 0u32;
        while !shared.stop.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            ticks += 1;
            if ticks % 10 == 0 {
                for (cat, line) in drain_events(&name, &shared, &mut mark) {
                    log(cat, &line);
                }
            }
        }
        for (cat, line) in drain_events(&name, &shared, &mut mark) {
            log(cat, &line);
        }
    });
}

/// A cycle target more than this many periods ahead of now is not believed (see `resync_target`).
pub const MAX_AHEAD_PERIODS: u64 = 8;

/// The IO thread never sleeps for more than a few periods on one target: a target beyond `MAX_AHEAD_PERIODS`
/// periods from `now` is replaced by `now`, so the loop runs a cycle, re-reads the driver and can see a stop.
pub fn resync_target(target: u64, now: u64, period_ns: u64) -> u64 {
    if target > now.saturating_add(MAX_AHEAD_PERIODS * period_ns.max(1)) {
        now
    } else {
        target
    }
}

/// Waits until `arm` (host ns) in slices of at most `slice_ns`, checking `stop` between slices, so a stop request
/// is seen within one slice whatever the timer or the sleep would otherwise do.
pub fn wait_until(timer: Option<&bifrost_audio_alsa::CycleTimer>, arm: u64, stop: &AtomicBool, slice_ns: u64) {
    loop {
        let now = now_raw_ns();
        if now >= arm || stop.load(Ordering::Acquire) {
            return;
        }
        let slice = (arm - now).min(slice_ns);
        let fired = timer.map(|t| t.arm_raw(now + slice).and_then(|_| t.wait_timeout(slice + 1_000_000)));
        if !matches!(fired, Some(Ok(_))) {
            std::thread::sleep(std::time::Duration::from_nanos(slice));
        }
    }
}

/// Runs `runner` on its own SCHED_FIFO thread until `shared.stop` is set; then stops the hardware.
pub fn spawn_io_thread(runner: DeviceRunner, name: String) -> std::io::Result<std::thread::JoinHandle<()>> {
    spawn_io_thread_tuned(runner, name, crate::rt::Tuning::default())
}

/// As `spawn_io_thread`, with the driver thread's CPU set (away from the sound IRQ) from `rt::device_started`.
///
/// Per cycle the thread sleeps on an absolute timer armed `margin` before the cycle start (the start comes from
/// the driver's DLL when it has locked), then spins to the exact start. The margin is the measured p99 wake-up
/// lateness (`rt::JitterTracker`). Sustained xruns double the quantum and post an overload notice
/// (`rt::FallbackPolicy`).
pub fn spawn_io_thread_tuned(mut runner: DeviceRunner, name: String, tuning: crate::rt::Tuning) -> std::io::Result<std::thread::JoinHandle<()>> {
    use crate::rt;
    std::thread::Builder::new().name(name.clone()).spawn(move || {
        rt::set_timer_slack_1ns();
        if let Err(e) = rt::set_thread_affinity(&tuning.cpus) {
            log("io", &format!("{name}: affinity: {e}"));
        }
        if let Err(e) = set_realtime(0, IO_PRIORITY, 1024) {
            log("io", &format!("{name}: no real-time priority ({e}); running at normal priority"));
        }
        let start = match runner.start() {
            Ok(t) => t,
            Err(e) => {
                log("driver", &format!("{name}: start failed: {e:?}"));
                return;
            }
        };
        let timer = bifrost_audio_alsa::CycleTimer::new().ok();
        // The driver's clock is CLOCK_MONOTONIC_RAW for the ALSA driver; the offset covers drivers whose clock
        // has another origin.
        let offset = now_raw_ns() as i64 - start as i64;
        let mut next = start;
        let mut jitter = rt::JitterTracker::new();
        let mut policy = rt::FallbackPolicy::new();
        let mut margin = rt::DEFAULT_MARGIN_NS;
        log("io", &format!("{name}: started at quantum {}", runner.quantum()));
        spawn_event_logger(name.clone(), runner.shared.clone());
        while !runner.shared.stop.load(Ordering::Acquire) {
            let period_ns = runner.quantum() as u64 * 1_000_000_000 / runner.cfg.rate.max(1) as u64;
            // A target far ahead (a bad DLL extrapolation, a quantum change) never parks the thread: resync to now.
            let target = resync_target((next as i64 + offset).max(0) as u64, now_raw_ns(), period_ns);
            let arm = target.saturating_sub(margin);
            let slice = (2 * period_ns).clamp(1_000_000, 10_000_000);
            let now = now_raw_ns();
            if arm > now {
                wait_until(timer.as_ref(), arm, &runner.shared.stop, slice);
                jitter.record(now_raw_ns().saturating_sub(arm));
            }
            if runner.shared.stop.load(Ordering::Acquire) {
                break;
            }
            while now_raw_ns() < target && !runner.shared.stop.load(Ordering::Relaxed) {
                std::hint::spin_loop();
            }
            let rep = runner.cycle(now_raw_ns());
            // Counters only: the real-time thread must not allocate or write to stderr.
            if rep.device_xrun {
                runner.shared.ev_device_xruns.fetch_add(1, Ordering::Relaxed);
            }
            if rep.restarted {
                runner.shared.ev_restarts.fetch_add(1, Ordering::Relaxed);
            }
            if rep.late > 0 {
                runner.shared.ev_late_cycles.fetch_add(1, Ordering::Relaxed);
            }
            let rate = runner.cfg.rate;
            if let Some(q) = policy.record(rep.device_xrun || rep.late > 0, rep.frames, rt::window_cycles(rep.frames, rate)) {
                runner.shared.fallback_floor.store(q, Ordering::Release);
                runner.shared.quantum.store(q, Ordering::Release);
                runner.notify_overload();
                runner.shared.ev_fallback_quantum.store(q, Ordering::Relaxed);
                runner.shared.ev_fallbacks.fetch_add(1, Ordering::Relaxed);
            }
            if runner.cycles() % 64 == 0 {
                margin = jitter.margin_ns(rep.frames as u64 * 1_000_000_000 / rate.max(1) as u64);
            }
            next = runner.next_cycle_ns();
        }
        runner.stop();
        log("io", &format!("{name}: stopped"));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_driver::null::NullDriver;
    use bifrost_audio_driver::{Driver, IoConfig};

    #[test]
    fn a_far_target_is_resynced_and_a_stop_ends_the_wait() {
        assert_eq!(resync_target(1_000, 0, 1_000_000), 1_000);
        assert_eq!(resync_target(1_000_000_000, 50, 1_000_000), 50);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let t = std::thread::spawn(move || {
            let timer = bifrost_audio_alsa::CycleTimer::new().ok();
            let t0 = std::time::Instant::now();
            wait_until(timer.as_ref(), now_raw_ns() + 60_000_000_000, &s2, 2_000_000);
            t0.elapsed()
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        stop.store(true, Ordering::Release);
        assert!(t.join().unwrap() < std::time::Duration::from_millis(500));
    }

    const UID: &str = "n0";
    const NS_PER_256: u64 = 5_333_333;

    struct Rig {
        h: bifrost_audio_driver::null::NullHandle,
        shared: Arc<Shared>,
        runner: DeviceRunner,
        slots: Vec<Arc<ClientSlot>>,
    }

    fn rig(quantum: u32) -> Rig {
        let mut d = NullDriver::new(vec![NullDriver::stereo(UID, 48_000.0)]);
        let h = d.handle();
        let io = d.open(UID, &IoConfig { rate: 48_000.0, quantum, input: true, output: true }).unwrap();
        let shared = Shared::new(quantum);
        let cfg = RunnerConfig { rate: 48_000, in_channels: 2, out_channels: 2, wait: WaitPolicy::None };
        let mut runner = DeviceRunner::new(io, shared.clone(), cfg);
        runner.start().unwrap();
        Rig { h, shared, runner, slots: Vec::new() }
    }

    impl Rig {
        fn add(&mut self, id: u32, in_ch: u32, out_ch: u32, buffer: u32) -> Arc<ClientSlot> {
            let s = Arc::new(ClientSlot::new(id, 1, in_ch, out_ch, buffer, 4096).unwrap());
            s.running.store(true, Ordering::Release);
            self.slots.push(s.clone());
            self.shared.publish(self.slots.clone());
            s
        }
        /// A client producing `frames` frames of a constant per channel.
        fn play(slot: &ClientSlot, frames: usize, values: &[f32]) {
            let planes: Vec<Vec<f32>> = values.iter().map(|v| vec![*v; frames]).collect();
            let refs: Vec<&[f32]> = planes.iter().map(|p| &p[..]).collect();
            assert_eq!(slot.output_ring().unwrap().write(&refs, frames as u32), frames as u32);
        }
        fn step(&mut self) -> CycleReport {
            self.h.advance(NS_PER_256);
            let t = self.h.clock().now();
            self.runner.cycle(t)
        }
    }

    #[test]
    fn done_record_follows_the_layout() {
        let s = ClientSlot::new(1, 1, 2, 2, 256, 1024).unwrap();
        assert_eq!(s.done_offset, s.layout.done);
        assert_eq!(s.done_offset + std::mem::size_of::<Activation>() as u32, s.layout.total);
        assert!(s.done_offset >= s.layout.output_ring);
        assert_eq!(s.done_offset % 64, 0);
    }

    #[test]
    fn a_client_that_finishes_late_does_not_burn_the_budget() {
        let mut r = rig(256);
        r.runner.cfg.wait = WaitPolicy::Futex { budget_ns: 2_500_000 };
        let slot = r.add(1, 0, 2, 256);
        let client = slot.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = std::thread::spawn(move || {
            let act: &Activation = client.region.get(client.layout.activation);
            let done: &Activation = client.region.get(client.layout.done);
            while !stop2.load(Ordering::Acquire) {
                if act.wait_triggered(20_000_000) {
                    std::thread::sleep(std::time::Duration::from_micros(300));
                    Rig::play(&client, 256, &[0.5, 0.5]);
                    shm::finish(act, &[done], now_raw_ns());
                }
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut worst = 0u64;
        let mut late = 0;
        for _ in 0..100 {
            let t0 = now_raw_ns();
            late += r.runner.cycle(t0).late;
            worst = worst.max(now_raw_ns() - t0);
        }
        stop.store(true, Ordering::Release);
        shm::futex::wake(&slot.activation().wake, 1);
        t.join().unwrap();
        assert_eq!(late, 0, "the client always finished inside the cycle");
        assert!(worst < 2_000_000, "a cycle waited out the 2.5 ms budget: {worst} ns");
    }

    #[test]
    fn rt_thread_events_are_counted_and_drained_once() {
        let shared = Shared::new(256);
        shared.ev_late_cycles.fetch_add(3, Ordering::Relaxed);
        shared.ev_fallbacks.fetch_add(1, Ordering::Relaxed);
        shared.ev_fallback_quantum.store(512, Ordering::Relaxed);
        let mut mark = EventMark::default();
        let lines = drain_events("dev", &shared, &mut mark);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].1.contains("3 cycle(s)"));
        assert!(lines[1].1.contains("512"));
        assert!(drain_events("dev", &shared, &mut mark).is_empty());
    }

    #[test]
    fn a_hostile_header_cannot_widen_the_server_ring() {
        let slot = ClientSlot::new(1, 1, 0, 2, 256, 64).unwrap();
        let h = (slot.region.addr + slot.layout.output_ring as usize) as *mut RingHeader;
        // SAFETY: test-only write of the non-atomic size field of a live header, standing in for the client process.
        unsafe { std::ptr::addr_of_mut!((*h).frames).write(u32::MAX) };
        let ring = slot.output_ring().unwrap();
        let (a, b) = (vec![1.0f32; 100], vec![1.0f32; 100]);
        assert_eq!(ring.write(&[&a, &b], 100), 64, "capped by the server's own ring_frames");
        assert_eq!(ring.writable(), 0);
    }

    #[test]
    fn sched_attr_layout_is_the_kernels() {
        assert_eq!(std::mem::size_of::<SchedAttr>(), 56);
    }

    #[test]
    fn two_clients_are_mixed_sample_exactly() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        let b = r.add(2, 0, 2, 256);
        Rig::play(&a, 256, &[0.25, -0.5]);
        Rig::play(&b, 256, &[0.125, 0.5]);
        let rep = r.step();
        assert_eq!((rep.frames, rep.late, rep.triggered), (256, 0, 2));
        let w = r.h.written(UID);
        assert_eq!(w[0], vec![0.375; 256]);
        assert_eq!(w[1], vec![0.0; 256]);
    }

    #[test]
    fn a_late_client_is_skipped_counted_and_the_device_keeps_its_cycle() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        let b = r.add(2, 0, 2, 256);
        // cycle 0: both on time
        Rig::play(&a, 256, &[0.5, 0.5]);
        Rig::play(&b, 256, &[0.25, 0.25]);
        assert_eq!(r.step().late, 0);
        // cycle 1: b is late
        Rig::play(&a, 256, &[0.5, 0.5]);
        let rep = r.step();
        assert_eq!(rep.late, 1);
        assert_eq!(b.missed_total.load(Ordering::Relaxed), 1);
        assert_eq!(b.overloads.load(Ordering::Relaxed), 1);
        assert_eq!(a.missed_total.load(Ordering::Relaxed), 0);
        // The device wrote this cycle anyway, with only a.
        let w = r.h.written(UID);
        assert_eq!(w[0].len(), 512);
        assert_eq!(w[0][256..], vec![0.5; 256][..]);
        // cycle 2: b delivers the stale cycle-1 buffer (late) and the cycle-2 buffer; the stale one is dropped.
        Rig::play(&b, 256, &[0.9, 0.9]);
        Rig::play(&b, 256, &[0.125, 0.125]);
        Rig::play(&a, 256, &[0.5, 0.5]);
        let rep = r.step();
        assert_eq!(rep.late, 0);
        assert_eq!(r.h.written(UID)[0][512..], vec![0.625; 256][..]);
        assert_eq!(r.shared.late_total.load(Ordering::Relaxed), 1);
        assert_eq!(r.shared.cycles.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn the_quantum_changes_at_a_cycle_boundary() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 64);
        Rig::play(&a, 256, &[0.5, 0.5]);
        let rep = r.step();
        assert_eq!((rep.frames, rep.quantum_changed), (256, false));
        r.shared.quantum.store(64, Ordering::Release);
        Rig::play(&a, 64, &[0.25, 0.25]);
        let rep = r.step();
        assert_eq!((rep.frames, rep.quantum_changed), (64, true));
        let w = r.h.written(UID);
        assert_eq!(w[0].len(), 256 + 64);
        assert_eq!(w[0][256..], vec![0.25; 64][..]);
        // next_ns advances by 64 frames = 1.333 ms from the 320 frames played: 320 * 1e9 / 48000.
        assert_eq!(rep.next_ns, 6_666_666);
    }

    #[test]
    fn a_large_buffer_client_is_triggered_every_n_cycles() {
        let mut r = rig(64);
        let a = r.add(1, 0, 2, 256);
        // every = 4; pre-roll of 3 quanta is written at the first cycle, then the client delivers 256 frames.
        let mut triggers = Vec::new();
        for _ in 0..8 {
            let rep = r.step_q(64);
            triggers.push(rep.triggered);
            if rep.triggered == 1 {
                Rig::play(&a, 256, &[0.5, 0.5]);
            }
        }
        assert_eq!(triggers, vec![1, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(a.missed_total.load(Ordering::Relaxed), 0);
    }

    impl Rig {
        fn step_q(&mut self, q: u32) -> CycleReport {
            self.h.advance(q as u64 * 1_000_000_000 / 48_000);
            let t = self.h.clock().now();
            self.runner.cycle(t)
        }
    }

    #[test]
    fn stream_volume_ramps_without_a_step() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        Rig::play(&a, 256, &[1.0, 1.0]);
        r.step();
        a.set_volume(0.5);
        Rig::play(&a, 256, &[1.0, 1.0]);
        r.step();
        let w = &r.h.written(UID)[0];
        // 480-frame ramp from 1.0 to 0.5: frame i of the second cycle has gain 1 - 0.5 * (i + 1) / 480.
        let g = |i: usize| 1.0 - 0.5 * (i as f32 + 1.0) / 480.0;
        assert_eq!(w[255], 1.0);
        assert!((w[256] - g(0)).abs() < 1e-6);
        assert!((w[511] - g(255)).abs() < 1e-5);
        assert!(w[511] > 0.5);
    }

    #[test]
    fn input_reaches_the_clients_input_ring() {
        let mut r = rig(256);
        let a = r.add(1, 2, 0, 256);
        r.h.push_input(UID, &[vec![0.5; 256], vec![-0.5; 256]]);
        r.step();
        let ring = a.input_ring().unwrap();
        assert_eq!(ring.readable(), 256);
        let (mut l, mut rr) = (vec![0.0; 256], vec![0.0; 256]);
        ring.read(&mut [&mut l[..], &mut rr[..]], 256);
        assert_eq!((l[0], rr[255]), (0.5, -0.5));
    }

    #[test]
    fn a_device_xrun_is_counted_and_the_cycle_continues() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        Rig::play(&a, 256, &[0.5, 0.5]);
        r.h.inject_xrun(UID, 128);
        let rep = r.step();
        assert!(rep.device_xrun);
        assert_eq!(r.shared.xruns.load(Ordering::Relaxed), 1);
        Rig::play(&a, 256, &[0.5, 0.5]);
        assert!(!r.step().device_xrun);
    }

    #[test]
    fn the_zero_timestamp_is_published_into_every_clients_memfd() {
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        Rig::play(&a, 256, &[0.0, 0.0]);
        r.step();
        Rig::play(&a, 256, &[0.0, 0.0]);
        r.step();
        let z = a.zero().read();
        // The second cycle starts at 10_666_666 ns: 10_666_666 * 48000 / 1e9 = 511.99 frames, rounded down.
        assert_eq!(z.sample_time, 2 * 5_333_333 * 48_000 / 1_000_000_000);
        assert_eq!(z.host_ns, 2 * NS_PER_256);
    }

    #[test]
    fn a_driver_restart_republishes_the_zero_timestamp_and_reanchors_the_timeline() {
        const RESTART_NS: u64 = 2 * NS_PER_256 + 1_000_000;
        let mut r = rig(256);
        let a = r.add(1, 0, 2, 256);
        Rig::play(&a, 256, &[0.0, 0.0]);
        let rep = r.step();
        // 256 frames after the start at 0: 256e9 / 48000 = 5_333_333.3 ns.
        assert_eq!((rep.restarted, rep.next_ns), (false, 5_333_333));
        assert_eq!(r.runner.seed(), 1);
        // The second cycle runs 1 ms late, at 11_666_666 ns, and the driver restarts the stream (xrun recovery).
        r.h.advance(NS_PER_256 + 1_000_000);
        r.h.force_restart(UID);
        Rig::play(&a, 256, &[0.0, 0.0]);
        let rep = r.runner.cycle(r.h.clock().now());
        // Re-anchored: the next cycle is frame 0 of the new stream, at the restart (the old timeline said 512
        // frames after 0 = 10_666_666), and the clients already have the new stamp with a new seed.
        assert_eq!((rep.restarted, rep.next_ns), (true, RESTART_NS));
        assert_eq!(r.shared.restarts.load(Ordering::Relaxed), 1);
        assert_eq!(r.runner.seed(), 2);
        assert_eq!(a.zero().read(), ZeroStampValue { sample_time: 0, host_ns: RESTART_NS, rate_scalar: 1.0, seed: 2 });
        // The next cycle follows the new anchor: 5_333_333 ns after the restart the hardware has played
        // 5_333_333 * 48000 / 1e9 = 255.99 -> 255 frames of the new stream.
        Rig::play(&a, 256, &[0.0, 0.0]);
        let rep = r.step();
        assert!(!rep.restarted);
        let z = a.zero().read();
        assert_eq!((z.sample_time, z.host_ns, z.seed), (255, RESTART_NS + NS_PER_256, 2));
        // 256 frames after the restart: 11_666_666 + 5_333_333 (the old timeline said 768 frames = 16_000_000).
        assert_eq!(rep.next_ns, 16_999_999);
        assert_eq!(r.runner.next_cycle_ns(), 16_999_999);
        assert_eq!(r.shared.restarts.load(Ordering::Relaxed), 1);
    }
}
