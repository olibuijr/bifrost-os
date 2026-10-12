//! Lock-free pieces shared by the control side and the render thread. Everything here is atomics over
//! pre-allocated storage: no allocation, no locks, in `write`, `read`, `send` and `drain`.

use bifrost_audio_units::MidiEvent;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

// ----------------------------------------------------------------------------------------------- node control

/// Per-node output gain and mute (the render thread ramps to the target across each block).
pub(crate) struct NodeControl {
    pub gain: AtomicU32,
    pub mute: AtomicBool,
}

impl NodeControl {
    pub fn new() -> NodeControl {
        NodeControl { gain: AtomicU32::new(1.0f32.to_bits()), mute: AtomicBool::new(false) }
    }

    pub fn target(&self) -> f32 {
        if self.mute.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.gain.load(Ordering::Relaxed))
        }
    }
}

/// Control of a node's gain and mute.
#[derive(Clone)]
pub struct NodeHandle(pub(crate) Arc<NodeControl>);

impl NodeHandle {
    /// Linear gain, 0 to 8 (non-finite values are ignored).
    pub fn set_gain(&self, g: f32) {
        if g.is_finite() {
            self.0.gain.store(g.clamp(0.0, 8.0).to_bits(), Ordering::Relaxed);
        }
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.0.gain.load(Ordering::Relaxed))
    }
    pub fn set_mute(&self, m: bool) {
        self.0.mute.store(m, Ordering::Relaxed);
    }
    pub fn is_muted(&self) -> bool {
        self.0.mute.load(Ordering::Relaxed)
    }
}

/// The gain of one connection.
#[derive(Clone)]
pub struct EdgeHandle(pub(crate) Arc<AtomicU32>);

impl EdgeHandle {
    pub fn set_gain(&self, g: f32) {
        if g.is_finite() {
            self.0.store(g.clamp(0.0, 8.0).to_bits(), Ordering::Relaxed);
        }
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------------------------- player control

const NO_SEEK: u64 = u64::MAX;

pub(crate) struct PlayerControl {
    pub playing: AtomicBool,
    pub looping: AtomicBool,
    pub position: AtomicU64,
    pub seek: AtomicU64,
    pub finished: AtomicBool,
    pub total: u64,
}

/// Control of a player node: play, pause, loop, seek, position.
#[derive(Clone)]
pub struct PlayerHandle(pub(crate) Arc<PlayerControl>);

impl PlayerHandle {
    pub(crate) fn new(total: u64) -> PlayerHandle {
        PlayerHandle(Arc::new(PlayerControl {
            playing: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            position: AtomicU64::new(0),
            seek: AtomicU64::new(NO_SEEK),
            finished: AtomicBool::new(false),
            total,
        }))
    }
    pub fn play(&self) {
        self.0.finished.store(false, Ordering::Relaxed);
        self.0.playing.store(true, Ordering::Relaxed);
    }
    pub fn pause(&self) {
        self.0.playing.store(false, Ordering::Relaxed);
    }
    pub fn set_looping(&self, l: bool) {
        self.0.looping.store(l, Ordering::Relaxed);
    }
    /// Moves the play position to `frame` at the start of the next block (clamped to the length).
    pub fn seek(&self, frame: u64) {
        self.0.finished.store(false, Ordering::Relaxed);
        self.0.seek.store(frame.min(self.0.total), Ordering::Release);
    }
    pub fn position(&self) -> u64 {
        self.0.position.load(Ordering::Relaxed)
    }
    pub fn is_playing(&self) -> bool {
        self.0.playing.load(Ordering::Relaxed)
    }
    /// True once a non-looping player ran off its end.
    pub fn is_finished(&self) -> bool {
        self.0.finished.load(Ordering::Relaxed)
    }
    pub fn length(&self) -> u64 {
        self.0.total
    }
    pub(crate) fn take_seek(&self) -> Option<u64> {
        let s = self.0.seek.swap(NO_SEEK, Ordering::Acquire);
        (s != NO_SEEK).then_some(s)
    }
}

// ----------------------------------------------------------------------------------------------------- taps

/// A single-producer single-consumer ring of planar f32 frames. The render thread writes; any one thread reads.
/// A block that does not fit is dropped whole and counted.
pub struct TapRing {
    planes: Vec<Vec<AtomicU32>>,
    cap: usize,
    write: AtomicU64,
    read: AtomicU64,
    dropped: AtomicU64,
}

impl TapRing {
    pub(crate) fn new(channels: usize, cap: usize) -> TapRing {
        TapRing {
            planes: (0..channels).map(|_| (0..cap).map(|_| AtomicU32::new(0)).collect()).collect(),
            cap,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    pub(crate) fn write(&self, src: &[Vec<f32>], n: usize) {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        if (w - r) as usize + n > self.cap {
            self.dropped.fetch_add(n as u64, Ordering::Relaxed);
            return;
        }
        for (plane, s) in self.planes.iter().zip(src.iter()) {
            for i in 0..n {
                plane[(w as usize + i) % self.cap].store(s[i].to_bits(), Ordering::Relaxed);
            }
        }
        self.write.store(w + n as u64, Ordering::Release);
    }
}

/// The reading end of a tap: a copy of what a node produced, delivered to a non-real-time thread.
#[derive(Clone)]
pub struct Tap(pub(crate) Arc<TapRing>);

impl Tap {
    pub fn channels(&self) -> usize {
        self.0.planes.len()
    }
    pub fn available(&self) -> usize {
        (self.0.write.load(Ordering::Acquire) - self.0.read.load(Ordering::Relaxed)) as usize
    }
    /// Frames dropped because the reader was too slow.
    pub fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }
    /// Reads up to `max` frames per channel into `dst` (one vector per channel, appended); returns the frames.
    pub fn read(&self, dst: &mut [Vec<f32>], max: usize) -> usize {
        let r = self.0.read.load(Ordering::Relaxed);
        let n = self.available().min(max);
        for (plane, d) in self.0.planes.iter().zip(dst.iter_mut()) {
            for i in 0..n {
                d.push(f32::from_bits(plane[(r as usize + i) % self.0.cap].load(Ordering::Relaxed)));
            }
        }
        self.0.read.store(r + n as u64, Ordering::Release);
        n
    }
}

// ----------------------------------------------------------------------------------------------------- MIDI

const MIDI_CAP: usize = 256;

/// A queue of MIDI messages from any one control thread to the render thread. Messages reach the unit at the
/// start of the next block (offset 0, in order).
pub(crate) struct MidiQueue {
    slots: Vec<AtomicU32>,
    write: AtomicU64,
    read: AtomicU64,
}

impl MidiQueue {
    pub fn new() -> MidiQueue {
        MidiQueue { slots: (0..MIDI_CAP).map(|_| AtomicU32::new(0)).collect(), write: AtomicU64::new(0), read: AtomicU64::new(0) }
    }

    pub fn send(&self, e: MidiEvent) -> bool {
        let w = self.write.load(Ordering::Relaxed);
        if (w - self.read.load(Ordering::Acquire)) as usize >= MIDI_CAP {
            return false;
        }
        self.slots[w as usize % MIDI_CAP].store(e.pack(), Ordering::Relaxed);
        self.write.store(w + 1, Ordering::Release);
        true
    }

    /// Moves queued messages into `out` (capacity is fixed by the caller); returns the count.
    pub fn drain(&self, out: &mut [MidiEvent]) -> usize {
        let mut r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        let mut n = 0;
        while r < w && n < out.len() {
            if let Some(e) = MidiEvent::unpack(self.slots[r as usize % MIDI_CAP].load(Ordering::Relaxed), 0) {
                out[n] = e;
                n += 1;
            }
            r += 1;
        }
        self.read.store(r, Ordering::Release);
        n
    }
}

/// Sends MIDI to an instrument node.
#[derive(Clone)]
pub struct MidiHandle(pub(crate) Arc<MidiQueue>);

impl MidiHandle {
    /// False when the queue (256 messages) is full.
    pub fn send(&self, e: MidiEvent) -> bool {
        self.0.send(e)
    }
}

macro_rules! opaque_debug {
    ($($t:ty),*) => {$(
        impl std::fmt::Debug for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(stringify!($t))
            }
        }
    )*};
}
opaque_debug!(NodeHandle, EdgeHandle, PlayerHandle, Tap, MidiHandle);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_wins_over_gain() {
        let c = Arc::new(NodeControl::new());
        let h = NodeHandle(c.clone());
        assert_eq!(c.target(), 1.0);
        h.set_gain(0.5);
        assert_eq!(c.target(), 0.5);
        h.set_mute(true);
        assert_eq!(c.target(), 0.0);
        assert!(h.is_muted());
        h.set_gain(f32::NAN);
        assert_eq!(h.gain(), 0.5);
        h.set_gain(100.0);
        assert_eq!(h.gain(), 8.0);
    }

    #[test]
    fn player_seek_clamps_and_is_taken_once() {
        let p = PlayerHandle::new(10);
        p.seek(99);
        assert_eq!(p.take_seek(), Some(10));
        assert_eq!(p.take_seek(), None);
        p.play();
        assert!(p.is_playing() && !p.is_finished());
    }

    #[test]
    fn tap_ring_delivers_in_order_and_wraps() {
        let ring = Arc::new(TapRing::new(1, 4));
        let tap = Tap(ring.clone());
        ring.write(&[vec![1.0, 2.0, 3.0]], 3);
        let mut d = vec![vec![]];
        assert_eq!(tap.read(&mut d, 2), 2);
        assert_eq!(d[0], vec![1.0, 2.0]);
        ring.write(&[vec![4.0, 5.0, 6.0]], 3); // 1 pending + 3 = 4 fits; wraps
        assert_eq!(tap.read(&mut d, 10), 4);
        assert_eq!(d[0], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(tap.dropped(), 0);
    }

    #[test]
    fn tap_ring_drops_whole_blocks_when_full() {
        let ring = Arc::new(TapRing::new(1, 4));
        let tap = Tap(ring.clone());
        ring.write(&[vec![1.0, 2.0, 3.0]], 3);
        ring.write(&[vec![9.0, 9.0]], 2); // 3 + 2 > 4
        assert_eq!(tap.dropped(), 2);
        assert_eq!(tap.available(), 3);
    }

    #[test]
    fn midi_queue_is_fifo_and_bounded() {
        let q = Arc::new(MidiQueue::new());
        let h = MidiHandle(q.clone());
        for n in 0..256 {
            assert!(h.send(MidiEvent::note_on(0, 0, (n % 100) as u8, 1)));
        }
        assert!(!h.send(MidiEvent::note_on(0, 0, 1, 1)));
        let mut out = [MidiEvent::default(); 4];
        assert_eq!(q.drain(&mut out), 4);
        assert_eq!(out.map(|e| e.data1), [0, 1, 2, 3]);
        assert!(h.send(MidiEvent::note_on(0, 0, 5, 1)));
    }
}
