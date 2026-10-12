//! `bifrost-audio-shm`: the shared-memory ABI between `bifrost-audiod` and its clients (docs/audio-architecture.md,
//! "IO cycle", "Client IO" and the PipeWire scheduling ideas): activation records, the zero-timestamp seqlock, the
//! single-producer single-consumer rings, the futex and eventfd wakers, and the graph trigger.
//!
//! Every shared struct is `#[repr(C)]` with its size and field offsets pinned by tests. This and the drivers are the
//! only crates with `unsafe`; each block carries a SAFETY comment. Owner: task 03.

use core::sync::atomic::{AtomicU32, AtomicU64};

/// A node's activation record, one 64-byte cache line per node.
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct Activation {
    /// `STATUS_*`.
    pub status: AtomicU32,
    /// Inputs still to finish this cycle; the node runs when it reaches 0.
    pub pending: AtomicU32,
    /// How many inputs the node has (`pending` is reset to this at cycle start).
    pub required: u32,
    pub xruns: AtomicU32,
    /// When the node was signalled, woke and finished this cycle (host ns), for the profiler.
    pub signal_ns: AtomicU64,
    pub awake_ns: AtomicU64,
    pub finish_ns: AtomicU64,
    /// The graph's frames per cycle and rate for this cycle.
    pub quantum: u32,
    pub rate: u32,
    /// The futex word the node sleeps on (incremented to wake it).
    pub wake: AtomicU32,
    pub _pad: u32,
}

pub const STATUS_IDLE: u32 = 0;
pub const STATUS_TRIGGERED: u32 = 1;
pub const STATUS_AWAKE: u32 = 2;
pub const STATUS_FINISHED: u32 = 3;

/// The device's zero timestamp, published by the IO thread and read by clients without a syscall.
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct ZeroStamp {
    /// Odd while a write is in progress (seqlock).
    pub seq: AtomicU32,
    pub seed: AtomicU32,
    pub sample_time: AtomicU64,
    pub host_ns: AtomicU64,
    /// `f64::to_bits` of the rate scalar.
    pub rate_scalar: AtomicU64,
}

/// A consistent copy of a zero timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ZeroStampValue {
    pub sample_time: u64,
    pub host_ns: u64,
    pub rate_scalar: f64,
    pub seed: u32,
}

impl ZeroStamp {
    /// Publishes a new value (single writer: the device's IO thread).
    pub fn publish(&self, v: ZeroStampValue) {
        let _ = v;
        todo!("audio-03")
    }

    /// Reads a consistent value, retrying on a torn read (wait-free for the reader in the absence of a writer).
    pub fn read(&self) -> ZeroStampValue {
        todo!("audio-03")
    }
}

/// The header of a single-producer single-consumer ring of f32 frames (non-interleaved, `channels` planes of
/// `frames` samples each, following the header in the memfd).
#[repr(C, align(64))]
#[derive(Debug, Default)]
pub struct RingHeader {
    /// Frames written so far (producer), wrapping at u64.
    pub write: AtomicU64,
    /// Frames read so far (consumer).
    pub read: AtomicU64,
    pub frames: u32,
    pub channels: u32,
}

/// A ring view over a header and its planes in a mapping. Producer and consumer each hold one.
pub struct Ring<'a> {
    pub header: &'a RingHeader,
    pub planes: &'a [core::cell::UnsafeCell<f32>],
}

impl Ring<'_> {
    /// Frames available to read.
    pub fn readable(&self) -> u32 {
        todo!("audio-03")
    }

    /// Frames of free space to write.
    pub fn writable(&self) -> u32 {
        todo!("audio-03")
    }

    /// Writes up to `frames` frames from per-channel slices; returns the frames written.
    pub fn write(&self, src: &[&[f32]], frames: u32) -> u32 {
        let _ = (src, frames);
        todo!("audio-03")
    }

    /// Reads up to `frames` frames into per-channel slices; returns the frames read.
    pub fn read(&self, dst: &mut [&mut [f32]], frames: u32) -> u32 {
        let _ = (dst, frames);
        todo!("audio-03")
    }
}

/// Byte offsets of an IO procedure's memfd (see `bifrost_audio_types::control::ShmLayout`), computed from the ring
/// sizes, each part 64-byte aligned.
pub fn layout(input_channels: u32, output_channels: u32, ring_frames: u32) -> bifrost_audio_types::control::ShmLayout {
    let _ = (input_channels, output_channels, ring_frames);
    todo!("audio-03")
}

/// A shared memory region: a sealed memfd mapped read-write.
pub struct Region {
    pub fd: std::os::fd::OwnedFd,
    pub len: usize,
    /// Mapping address (as usize so the region is `Send`).
    pub addr: usize,
}

impl Region {
    /// Creates a sealed (`F_SEAL_SHRINK | F_SEAL_GROW`) memfd of `len` bytes, zeroed and mapped.
    pub fn create(name: &str, len: usize) -> std::io::Result<Region> {
        let _ = (name, len);
        todo!("audio-03")
    }

    /// Maps a memfd received from the other side.
    pub fn map(fd: std::os::fd::OwnedFd, len: usize) -> std::io::Result<Region> {
        let _ = (fd, len);
        todo!("audio-03")
    }

    /// A shared struct at `offset` (aligned, in bounds, checked).
    pub fn get<T>(&self, offset: u32) -> &T {
        let _ = offset;
        todo!("audio-03")
    }
}

/// Futex wait and wake on a word in shared memory (`FUTEX_WAIT`/`FUTEX_WAKE`, not private: cross-process).
pub mod futex {
    use core::sync::atomic::AtomicU32;

    /// Sleeps while `*word == expected`, at most `timeout_ns` (0 = forever). Returns false on timeout.
    pub fn wait(word: &AtomicU32, expected: u32, timeout_ns: u64) -> bool {
        let _ = (word, expected, timeout_ns);
        todo!("audio-03")
    }

    /// Wakes up to `n` sleepers.
    pub fn wake(word: &AtomicU32, n: u32) {
        let _ = (word, n);
        todo!("audio-03")
    }
}

/// The graph trigger (PipeWire's scheme): called by a node when it finished. Decrements each target's `pending` and
/// wakes the targets that reach 0 (status TRIGGERED, signal time stamped, futex woken). Lock-free; a target is
/// woken exactly once per cycle.
pub fn finish(me: &Activation, targets: &[&Activation], now_ns: u64) {
    let _ = (me, targets, now_ns);
    todo!("audio-03")
}

/// Resets a node for a new cycle: `pending = required`, status IDLE.
pub fn reset(node: &Activation, quantum: u32, rate: u32) {
    let _ = (node, quantum, rate);
    todo!("audio-03")
}
