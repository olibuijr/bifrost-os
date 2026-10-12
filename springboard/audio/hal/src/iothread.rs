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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// The most channels one IO procedure handles (scratch arrays are on the stack).
pub const MAX_CHANNELS: usize = 32;
/// The loop wakes at least this often to look at the stop flag (a backstop; `stop` also wakes the futex).
const BACKSTOP_NS: u64 = 250_000_000;
/// The rate assumed when the activation record has none yet.
const DEFAULT_RATE: f64 = 48_000.0;

/// Host time: `CLOCK_MONOTONIC_RAW` nanoseconds (the contract's host clock).
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out-pointer; the clock id is constant.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

thread_local! {
    static IN_RT: Cell<bool> = const { Cell::new(false) };
}

/// While alive (debug builds only), allocation on this thread panics when the process installed
/// [`RtGuardAllocator`] as its global allocator.
pub struct RtScope(());

impl RtScope {
    pub fn enter() -> RtScope {
        #[cfg(debug_assertions)]
        IN_RT.with(|c| c.set(true));
        RtScope(())
    }
}

impl Drop for RtScope {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        let _ = IN_RT.try_with(|c| c.set(false));
    }
}

/// A global allocator that panics on allocation inside an [`RtScope`] and otherwise forwards to the system one.
/// Install with `#[global_allocator] static A: bifrost_audio_hal::RtGuardAllocator = bifrost_audio_hal::RtGuardAllocator;`
/// in debug builds of an app (or a test binary) to enforce the real-time rules.
pub struct RtGuardAllocator;

fn trip() {
    // clear the flag first: the panic machinery itself allocates
    let _ = IN_RT.try_with(|c| c.set(false));
    panic!("allocation inside a real-time IOProc");
}

// SAFETY: every method forwards to the system allocator with the same arguments; the only extra behaviour is a
// panic before forwarding, which never touches the memory contract.
unsafe impl GlobalAlloc for RtGuardAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            trip();
        }
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            trip();
        }
        System.alloc_zeroed(l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            trip();
        }
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
    Some(Ring { header, planes })
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
        futex::wait(&act.wake, last, BACKSTOP_NS);
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
        bifrost_audio_shm::finish(act, &[], now_ns());
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
    let rate = if act.rate == 0 { DEFAULT_RATE } else { act.rate as f64 };
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
    fn allocation_inside_the_scope_panics_and_outside_does_not() {
        let ok = std::panic::catch_unwind(|| vec![1u8; 8]);
        assert!(ok.is_ok());
        let r = std::panic::catch_unwind(|| {
            let _s = RtScope::enter();
            let v: Vec<u8> = Vec::with_capacity(64);
            v.len()
        });
        assert!(r.is_err());
        // the scope is gone after the panic: allocation works again
        let again = std::panic::catch_unwind(|| vec![2u8; 8]);
        assert!(again.is_ok());
    }
}
