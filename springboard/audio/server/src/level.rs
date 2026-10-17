//! A level tap: the newest samples of what a device plays or captures, copied out of the IO cycle for the analysis the
//! control thread runs (`bifrost_audio_engine::analysis`). The IO thread writes with plain atomic stores (no allocation,
//! no locks, no syscalls); the control thread reads a window of the newest frames and discards it when the writer lapped
//! the part it was reading. Single writer, any number of readers. At most two channels are kept (a stereo device's
//! left and right; a mono device's only channel).

use bifrost_audio_types::Buffer;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

/// Frames kept per channel (a power of two).
pub const LEVEL_FRAMES: usize = 8192;
/// Frames one analysis looks at (about 85 ms at 48 kHz).
pub const WINDOW: usize = 4096;
/// Channels kept.
pub const LEVEL_CH: usize = 2;

const MASK: usize = LEVEL_FRAMES - 1;

pub struct LevelTap {
    samples: Vec<AtomicU32>,
    /// Frames written since the start, per channel.
    written: AtomicU64,
    channels: AtomicU32,
    rate: AtomicU32,
}

impl Default for LevelTap {
    fn default() -> Self {
        LevelTap::new()
    }
}

impl LevelTap {
    pub fn new() -> LevelTap {
        LevelTap {
            samples: (0..LEVEL_CH * LEVEL_FRAMES).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicU64::new(0),
            channels: AtomicU32::new(0),
            rate: AtomicU32::new(48_000),
        }
    }

    /// The sample rate the analysis should assume (set when the device starts).
    pub fn set_rate(&self, rate: u32) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Frames written so far; a reader that remembers it knows whether anything new arrived.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// IO thread: appends the first `frames` frames of the first two planes of `bufs`.
    pub fn write(&self, bufs: &[Buffer], frames: usize) {
        let nch = bufs.len().min(LEVEL_CH);
        if nch == 0 || frames == 0 {
            return;
        }
        self.channels.store(nch as u32, Ordering::Relaxed);
        let w = self.written.load(Ordering::Relaxed);
        for f in frames.saturating_sub(LEVEL_FRAMES)..frames {
            let idx = ((w + f as u64) as usize) & MASK;
            for (c, b) in bufs.iter().take(nch).enumerate() {
                self.samples[c * LEVEL_FRAMES + idx].store(b.data[f].to_bits(), Ordering::Relaxed);
            }
        }
        self.written.store(w + frames as u64, Ordering::Release);
    }

    /// Control thread: the newest `WINDOW` frames (fewer at the start) as one `Vec` per channel, or `None` when
    /// nothing was written yet or the writer overwrote part of the window while it was read.
    pub fn window(&self) -> Option<Vec<Vec<f32>>> {
        let w = self.written.load(Ordering::Acquire);
        let nch = self.channels.load(Ordering::Relaxed) as usize;
        if w == 0 || nch == 0 {
            return None;
        }
        let n = (w as usize).min(WINDOW);
        let first = w - n as u64;
        let mut out = vec![vec![0.0f32; n]; nch];
        for (c, plane) in out.iter_mut().enumerate() {
            for (i, x) in plane.iter_mut().enumerate() {
                let idx = ((first + i as u64) as usize) & MASK;
                *x = f32::from_bits(self.samples[c * LEVEL_FRAMES + idx].load(Ordering::Relaxed));
            }
        }
        fence(Ordering::Acquire);
        let after = self.written.load(Ordering::Relaxed);
        if after - first > LEVEL_FRAMES as u64 {
            return None;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(v: &[f32]) -> Buffer {
        Buffer { channels: 1, data: v.to_vec() }
    }

    #[test]
    fn nothing_is_read_before_anything_is_written() {
        let t = LevelTap::new();
        assert!(t.window().is_none());
        t.write(&[], 16);
        t.write(&[plane(&[1.0])], 0);
        assert_eq!(t.written(), 0);
    }

    #[test]
    fn the_newest_frames_come_back_in_order_for_each_channel() {
        let t = LevelTap::new();
        t.write(&[plane(&[1.0, 2.0, 3.0]), plane(&[-1.0, -2.0, -3.0]), plane(&[9.0, 9.0, 9.0])], 3);
        t.write(&[plane(&[4.0, 5.0, 6.0]), plane(&[-4.0, -5.0, -6.0])], 2);
        let w = t.window().unwrap();
        assert_eq!(w.len(), 2, "two channels are kept");
        assert_eq!(w[0], vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(w[1], vec![-1.0, -2.0, -3.0, -4.0, -5.0]);
        assert_eq!(t.written(), 5);
    }

    #[test]
    fn a_mono_device_keeps_its_only_channel() {
        let t = LevelTap::new();
        t.write(&[plane(&[0.5, 0.25])], 2);
        assert_eq!(t.window().unwrap(), vec![vec![0.5, 0.25]]);
    }

    #[test]
    fn the_window_is_the_newest_frames_after_the_ring_wrapped() {
        let t = LevelTap::new();
        let block: Vec<f32> = (0..2048).map(|i| i as f32).collect();
        for round in 0..6 {
            let v: Vec<f32> = block.iter().map(|x| x + (round * 2048) as f32).collect();
            t.write(&[plane(&v)], 2048);
        }
        let w = t.window().unwrap();
        assert_eq!(w[0].len(), WINDOW);
        let last = (6 * 2048 - 1) as f32;
        assert_eq!(*w[0].last().unwrap(), last);
        assert_eq!(w[0][0], last - (WINDOW - 1) as f32);
    }

    #[test]
    fn a_block_longer_than_the_ring_keeps_its_tail() {
        let t = LevelTap::new();
        let v: Vec<f32> = (0..LEVEL_FRAMES + 100).map(|i| i as f32).collect();
        t.write(&[plane(&v)], v.len());
        let w = t.window().unwrap();
        assert_eq!(*w[0].last().unwrap(), (LEVEL_FRAMES + 99) as f32);
        assert_eq!(t.written(), (LEVEL_FRAMES + 100) as u64);
    }

    #[test]
    fn the_rate_is_what_the_device_set() {
        let t = LevelTap::new();
        assert_eq!(t.rate(), 48_000);
        t.set_rate(44_100);
        assert_eq!(t.rate(), 44_100);
    }
}
