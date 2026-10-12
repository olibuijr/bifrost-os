//! The client's IO thread: sleeps on the activation record's futex, reads the cycle's input from the input ring,
//! runs the IOProc, writes the output ring and marks the node finished. The loop never allocates, locks or makes a
//! syscall other than the futex (and the clock, which is vDSO).

use crate::workgroup::{self, Promote, DEFAULT_UTIL_MIN};
use crate::{IoContext, IoProc};
use bifrost_audio_shm::{futex, Activation, Region, Ring, RingHeader, ZeroStamp, STATUS_AWAKE};
use bifrost_audio_types::control::ShmLayout;
use bifrost_audio_types::{Buffer, BufferList, TimeStamp, TimeStampFlags};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

/// The most channels one IO procedure handles (scratch arrays are on the stack).
pub const MAX_CHANNELS: usize = bifrost_audio_types::control::MAX_CLIENT_CHANNELS;
/// The rate assumed when the activation record has none yet.
const DEFAULT_RATE: f64 = 48_000.0;

/// Host time: `CLOCK_MONOTONIC_RAW` nanoseconds (the contract's host clock).
pub fn now_ns() -> u64 {
    bifrost_audio_shm::now_ns()
}

thread_local! {
    /// 0: outside a scope, 1: inside (count violations), 2: inside and abort on a violation.
    static IN_RT: Cell<u8> = const { Cell::new(0) };
}

/// Allocations seen inside an [`RtScope`] since the process started.
static VIOLATIONS: AtomicU64 = AtomicU64::new(0);
/// `BIFROST_RT_STRICT=1`, read once.
static STRICT: OnceLock<bool> = OnceLock::new();

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn strict() -> bool {
    *STRICT.get_or_init(|| std::env::var("BIFROST_RT_STRICT").map(|v| v == "1").unwrap_or(false))
}

/// While alive (debug builds only), an allocation on this thread is a violation when the process installed
/// [`RtGuardAllocator`] as its global allocator: it is counted ([`RtScope::violations`]) and reported on stderr, and
/// the process aborts when `BIFROST_RT_STRICT=1` was set (read once, at the first scope).
pub struct RtScope(());

impl RtScope {
    pub fn enter() -> RtScope {
        #[cfg(debug_assertions)]
        {
            // read (and cache) the variable before the flag is set: the first read allocates
            let level = if strict() { 2 } else { 1 };
            IN_RT.with(|c| c.set(level));
        }
        RtScope(())
    }

    /// How many allocations happened inside scopes so far (always 0 without the guard allocator or in release builds).
    pub fn violations() -> u64 {
        VIOLATIONS.load(Ordering::Relaxed)
    }
}

impl Drop for RtScope {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        let _ = IN_RT.try_with(|c| c.set(0));
    }
}

/// A global allocator that reports allocation inside an [`RtScope`] and otherwise forwards to the system one. A
/// violation increments the counter, writes a fixed line to stderr with `write(2)` (no allocation) and aborts only
/// under `BIFROST_RT_STRICT=1`; it never unwinds, which a global allocator must not do.
/// Install with `#[global_allocator] static A: bifrost_audio_hal::RtGuardAllocator = bifrost_audio_hal::RtGuardAllocator;`
/// in debug builds of an app (or a test binary) to enforce the real-time rules.
pub struct RtGuardAllocator;

const VIOLATION_MSG: &[u8] = b"[is.olibuijr.audio/hal] allocation inside a real-time IOProc\n";

/// Called when the allocator sees an allocation: counts, reports, maybe aborts. Allocation-free.
fn check() {
    let level = IN_RT.try_with(|c| c.get()).unwrap_or(0);
    if level == 0 {
        return;
    }
    VIOLATIONS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: write(2) of a static buffer to fd 2; no allocation, no Rust state touched.
    unsafe { libc::write(2, VIOLATION_MSG.as_ptr() as *const libc::c_void, VIOLATION_MSG.len()) };
    if level == 2 {
        std::process::abort();
    }
}

// SAFETY: every method forwards to the system allocator with the same arguments; the only extra behaviour is
// `check`, which never allocates or unwinds and never touches the memory contract.
unsafe impl GlobalAlloc for RtGuardAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        check();
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        check();
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        check();
        System.realloc(p, l, n)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
}

/// A ring view over the ring at `offset` in `region`, or None when the header does not fit the mapping or does
/// not have `channels` planes.
pub(crate) fn ring_view(region: &Region, offset: u32, channels: u32) -> Option<Ring<'_>> {
    let hdr_size = std::mem::size_of::<RingHeader>() as u64;
    let end = offset as u64 + hdr_size;
    if end > region.len as u64 || channels == 0 {
        return None;
    }
    let header: &RingHeader = region.get(offset);
    if header.channels != channels || header.frames == 0 {
        return None;
    }
    let samples = header.channels as u64 * header.frames as u64;
    if end + samples * 4 > region.len as u64 {
        return None;
    }
    // SAFETY: the planes lie inside the mapping (checked above), start 64-byte aligned after the header, and the
    // mapping lives as long as `region`; f32 has no invalid bit patterns and UnsafeCell<f32> has f32's layout.
    let planes = unsafe { std::slice::from_raw_parts((region.addr + end as usize) as *const UnsafeCell<f32>, samples as usize) };
    // The size is copied now, from the header just validated against the mapping; later rewrites are ignored.
    Ring::new(header, planes, header.channels, header.frames)
}

/// The done record of `layout`, when it lies 64-aligned inside the mapping and is not the activation itself.
pub(crate) fn done_record<'a>(region: &'a Region, layout: &ShmLayout) -> Option<&'a Activation> {
    let size = std::mem::size_of::<Activation>() as u64;
    let ok = layout.done != layout.activation && layout.done % 64 == 0 && layout.done as u64 + size <= region.len as u64;
    ok.then(|| region.get(layout.done))
}

fn flag_all() -> TimeStampFlags {
    TimeStampFlags(TimeStampFlags::SAMPLE_TIME.0 | TimeStampFlags::HOST_TIME.0 | TimeStampFlags::RATE_SCALAR.0)
}

fn planes(channels: u32, frames: u32) -> BufferList {
    BufferList { buffers: (0..channels).map(|_| Buffer { channels: 1, data: vec![0.0; frames as usize] }).collect() }
}

/// What the thread needs. `ready` gets one message once the thread has joined the workgroup.
pub(crate) struct IoArgs {
    pub region: Arc<Region>,
    pub layout: ShmLayout,
    pub input_channels: u32,
    pub output_channels: u32,
    pub frames: u32,
    pub stop: Arc<AtomicBool>,
    pub promote: Arc<dyn Promote + Send + Sync>,
    pub f: IoProc,
    pub ready: Sender<()>,
}

/// Runs until `stop`; returns the IOProc so the owner can start again.
pub(crate) fn run(a: IoArgs) -> IoProc {
    let IoArgs { region, layout, input_channels, output_channels, frames, stop, promote, mut f, ready } = a;
    let member = match workgroup::join(&*promote, DEFAULT_UTIL_MIN) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("[is.olibuijr.audio/hal] real-time priority refused ({e:?}); running at normal priority");
            Some(workgroup::join_unpromoted(DEFAULT_UTIL_MIN))
        }
    };
    let _ = ready.send(());

    let act: &Activation = region.get(layout.activation);
    let zs: &ZeroStamp = region.get(layout.zero_stamp);
    // The server's "done" record: finishing it ends the server's wait for this client. An implausible offset
    // (old or hostile server) means no target, never a write outside the mapping.
    let done: Option<&Activation> = done_record(&region, &layout);
    let in_ring = if input_channels > 0 { ring_view(&region, layout.input_ring, input_channels) } else { None };
    let out_ring = if output_channels > 0 { ring_view(&region, layout.output_ring, output_channels) } else { None };
    if (input_channels > 0 && in_ring.is_none()) || (output_channels > 0 && out_ring.is_none()) {
        eprintln!("[is.olibuijr.audio/hal] ring header does not match the request; the missing side stays silent");
    }

    // all scratch is allocated here, before the loop
    let mut in_list = planes(input_channels, frames);
    let mut out_list = planes(output_channels, frames);
    let mut cycle: u64 = 0;
    let mut last = act.wake.load(Ordering::Acquire);

    while !stop.load(Ordering::Acquire) {
        // Sleep until triggered; `stop` bumps the word and wakes it, so no periodic wakeup is needed (timeout 0).
        futex::wait(&act.wake, last, 0);
        if stop.load(Ordering::Acquire) {
            break;
        }
        let cur = act.wake.load(Ordering::Acquire);
        if cur == last {
            continue;
        }
        last = cur;
        let _rt = RtScope::enter();
        let t0 = now_ns();
        act.status.store(STATUS_AWAKE, Ordering::Release);
        act.awake_ns.store(t0, Ordering::Relaxed);
        cycle_once(act, zs, in_ring.as_ref(), out_ring.as_ref(), &mut in_list, &mut out_list, frames, cycle, t0, &mut f);
        cycle += 1;
        match done {
            Some(d) => bifrost_audio_shm::finish(act, &[d], now_ns()),
            None => bifrost_audio_shm::finish(act, &[], now_ns()),
        }
    }
    drop(member);
    f
}

/// One cycle's work (separate so tests can drive it without the futex).
#[allow(clippy::too_many_arguments)]
pub(crate) fn cycle_once(
    act: &Activation,
    zs: &ZeroStamp,
    in_ring: Option<&Ring<'_>>,
    out_ring: Option<&Ring<'_>>,
    in_list: &mut BufferList,
    out_list: &mut BufferList,
    frames: u32,
    cycle: u64,
    now: u64,
    f: &mut IoProc,
) {
    // input: what the ring has, zeros for the rest
    if let Some(r) = in_ring {
        let mut s: [&mut [f32]; MAX_CHANNELS] = std::array::from_fn(|_| <&mut [f32]>::default());
        let n = in_list.buffers.len().min(MAX_CHANNELS);
        for (slot, b) in s.iter_mut().zip(in_list.buffers.iter_mut()) {
            *slot = &mut b.data[..];
        }
        let got = r.read(&mut s[..n], frames) as usize;
        for slot in s[..n].iter_mut() {
            slot[got..].iter_mut().for_each(|x| *x = 0.0);
        }
    }
    out_list.clear();

    // timestamps: the zero stamp is the anchor; this cycle's input was captured one buffer before `now`, and the
    // output plays one buffer after it
    let z = zs.read();
    let rate = match act.rate.load(Ordering::Acquire) {
        0 => DEFAULT_RATE,
        r => r as f64,
    };
    let zero = TimeStamp { sample_time: z.sample_time as f64, host_ns: z.host_ns, rate_scalar: z.rate_scalar, word_clock: 0, flags: flag_all() };
    let now_st = zero.sample_time_at(now, rate);
    let in_st = now_st - frames as f64;
    let out_st = now_st + frames as f64;
    let now_ts = TimeStamp { sample_time: now_st, host_ns: now, ..zero };
    let in_ts = TimeStamp { sample_time: in_st, host_ns: zero.host_ns_of(in_st, rate), ..zero };
    let out_ts = TimeStamp { sample_time: out_st, host_ns: zero.host_ns_of(out_st, rate), ..zero };

    let ctx = IoContext { cycle, frames };
    f(&ctx, &now_ts, in_list, &in_ts, out_list, &out_ts);

    if let Some(r) = out_ring {
        let mut s: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        let n = out_list.buffers.len().min(MAX_CHANNELS);
        for (slot, b) in s.iter_mut().zip(out_list.buffers.iter()) {
            *slot = &b.data[..];
        }
        let _ = r.write(&s[..n], frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_shm::ZeroStampValue;

    #[test]
    fn done_record_is_checked() {
        let layout = bifrost_audio_shm::layout(1, 1, 8);
        let region = Region::create("done-test", layout.total as usize).unwrap();
        assert!(done_record(&region, &layout).is_some());
        let mut bad = layout;
        bad.done = 0; // the activation itself
        assert!(done_record(&region, &bad).is_none());
        bad.done = layout.total; // past the end
        assert!(done_record(&region, &bad).is_none());
        bad.done = 70; // misaligned
        assert!(done_record(&region, &bad).is_none());
    }

    #[test]
    fn now_ns_is_monotonic() {
        let a = now_ns();
        let b = now_ns();
        assert!(b >= a);
        assert!(a > 0);
    }

    #[test]
    fn planes_has_one_buffer_per_channel() {
        let l = planes(3, 4);
        assert_eq!(l.buffers.len(), 3);
        assert!(l.buffers.iter().all(|b| b.channels == 1 && b.data == vec![0.0; 4]));
    }

    #[test]
    fn flags_cover_sample_host_and_rate() {
        let f = flag_all();
        assert_eq!(f.0, 1 | 2 | 4);
        assert!(!f.contains(TimeStampFlags::WORD_CLOCK));
    }

    #[cfg(debug_assertions)]
    #[global_allocator]
    static GUARD: RtGuardAllocator = RtGuardAllocator;

    #[cfg(debug_assertions)]
    #[test]
    fn allocation_inside_the_scope_is_counted_and_outside_is_not() {
        let before = RtScope::violations();
        let v = vec![1u8; 8];
        assert_eq!(v.len(), 8);
        {
            let _s = RtScope::enter();
            let w: Vec<u8> = Vec::with_capacity(64);
            assert_eq!(w.capacity(), 64);
        }
        // other tests' IO threads may add to the counter, never remove from it
        assert!(RtScope::violations() > before);
        // the scope is gone: this thread is no longer inside one
        assert_eq!(IN_RT.with(|c| c.get()), 0);
    }

    fn rig(channels: u32, ring_frames: u32, rate: u32) -> (Arc<Region>, ShmLayout) {
        let layout = bifrost_audio_shm::layout(channels, channels, ring_frames);
        let region = Arc::new(Region::create("iothread-test", layout.total as usize).unwrap());
        // SAFETY: the test owns the fresh mapping; the headers are plain fields nobody else touches yet, and the
        // offsets come from `layout` so they are inside the mapping.
        unsafe {
            for off in [layout.input_ring, layout.output_ring] {
                let h = (region.addr + off as usize) as *mut RingHeader;
                (*h).frames = ring_frames;
                (*h).channels = channels;
            }
            let act = (region.addr + layout.activation as usize) as *mut Activation;
            (*act).rate.store(rate, Ordering::Relaxed);
        }
        (region, layout)
    }

    /// (cycle, input plane 0, in sample time, in host ns, out sample time, out host ns)
    type Seen = Arc<std::sync::Mutex<Vec<(u64, Vec<f32>, f64, u64, f64, u64)>>>;

    /// An IOProc that records what it was given and fills output plane 0 with `fill`.
    fn recorder(seen: &Seen, fill: f32) -> IoProc {
        let seen = seen.clone();
        Box::new(move |ctx, _now, input, it, output, ot| {
            seen.lock().unwrap().push((ctx.cycle, input.buffers[0].data.clone(), it.sample_time, it.host_ns, ot.sample_time, ot.host_ns));
            output.buffers[0].data.iter_mut().for_each(|x| *x = fill);
        })
    }

    #[test]
    fn short_input_is_zero_filled_and_output_reaches_the_ring() {
        let (region, layout) = rig(1, 8, 48_000);
        let act: &Activation = region.get(layout.activation);
        let zs: &ZeroStamp = region.get(layout.zero_stamp);
        zs.publish(ZeroStampValue { sample_time: 0, host_ns: 1_000, rate_scalar: 1.0, seed: 1 });
        let in_ring = ring_view(&region, layout.input_ring, 1).unwrap();
        let out_ring = ring_view(&region, layout.output_ring, 1).unwrap();
        assert_eq!(in_ring.write(&[&[1.0, 2.0]], 2), 2);
        // stale data in the scratch list must not leak through
        let mut in_list = planes(1, 4);
        in_list.buffers[0].data.iter_mut().for_each(|x| *x = 9.0);
        let mut out_list = planes(1, 4);
        let seen: Seen = Arc::default();
        let mut f = recorder(&seen, 0.5);
        cycle_once(act, zs, Some(&in_ring), Some(&out_ring), &mut in_list, &mut out_list, 4, 0, 1_000, &mut f);
        assert_eq!(seen.lock().unwrap()[0].1, vec![1.0, 2.0, 0.0, 0.0]);
        let mut got = [0f32; 4];
        assert_eq!(out_ring.read(&mut [&mut got[..]], 4), 4);
        assert_eq!(got, [0.5; 4]);
    }

    #[test]
    fn timestamps_are_one_buffer_before_and_after_now() {
        let (region, layout) = rig(1, 8, 44_100);
        let act: &Activation = region.get(layout.activation);
        let zs: &ZeroStamp = region.get(layout.zero_stamp);
        zs.publish(ZeroStampValue { sample_time: 1000, host_ns: 1_000_000_000, rate_scalar: 1.0, seed: 1 });
        let mut in_list = planes(1, 4);
        let mut out_list = planes(1, 4);
        let seen: Seen = Arc::default();
        let mut f = recorder(&seen, 0.0);
        // one second after the zero stamp at 44.1 kHz: sample 1000 + 44100 = 45100
        cycle_once(act, zs, None, None, &mut in_list, &mut out_list, 4, 0, 2_000_000_000, &mut f);
        let s = seen.lock().unwrap()[0].clone();
        assert_eq!(s.2, 45_096.0);
        assert_eq!(s.4, 45_104.0);
        // host time of sample 45096 is 1e9 + 44096 / 44100 * 1e9 = 1_999_909_297.05; of 45104 it is 2_000_090_702.5
        assert!(s.3.abs_diff(1_999_909_297) <= 1, "{}", s.3);
        assert!(s.5.abs_diff(2_000_090_703) <= 1, "{}", s.5);
    }

    #[test]
    fn a_zero_rate_means_48_khz() {
        let (region, layout) = rig(1, 8, 0);
        let act: &Activation = region.get(layout.activation);
        let zs: &ZeroStamp = region.get(layout.zero_stamp);
        zs.publish(ZeroStampValue { sample_time: 1000, host_ns: 1_000_000_000, rate_scalar: 1.0, seed: 1 });
        let mut in_list = planes(1, 4);
        let mut out_list = planes(1, 4);
        let seen: Seen = Arc::default();
        let mut f = recorder(&seen, 0.0);
        // one second later: sample 1000 + 48000 = 49000
        cycle_once(act, zs, None, None, &mut in_list, &mut out_list, 4, 0, 2_000_000_000, &mut f);
        let s = seen.lock().unwrap()[0].clone();
        assert_eq!(s.2, 48_996.0);
        assert_eq!(s.4, 49_004.0);
        // 1e9 + 47996 / 48000 * 1e9 = 1_999_916_666.67; 1e9 + 48004 / 48000 * 1e9 = 2_000_083_333.33
        assert!(s.3.abs_diff(1_999_916_667) <= 1, "{}", s.3);
        assert!(s.5.abs_diff(2_000_083_333) <= 1, "{}", s.5);
    }

    #[test]
    fn the_io_proc_sees_cycle_numbers_in_order() {
        let (region, layout) = rig(1, 8, 48_000);
        let act: &Activation = region.get(layout.activation);
        let zs: &ZeroStamp = region.get(layout.zero_stamp);
        zs.publish(ZeroStampValue { sample_time: 0, host_ns: 0, rate_scalar: 1.0, seed: 1 });
        let mut in_list = planes(1, 4);
        let mut out_list = planes(1, 4);
        let seen: Seen = Arc::default();
        let mut f = recorder(&seen, 0.0);
        for c in 0..3u64 {
            cycle_once(act, zs, None, None, &mut in_list, &mut out_list, 4, c, c * 83_333, &mut f);
        }
        let cycles: Vec<u64> = seen.lock().unwrap().iter().map(|s| s.0).collect();
        assert_eq!(cycles, vec![0, 1, 2]);
    }
}
