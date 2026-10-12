//! The single-producer single-consumer frame ring between the ALSA application thread and the IO procedure,
//! plus the sample conversions. Real-time safe: no allocation, locks or syscalls after `new`.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};

/// Interleaved f32 frames. One side pushes, the other pops; positions count frames since the last reset.
pub struct PcmRing {
    data: Box<[UnsafeCell<f32>]>,
    frames: u32,
    channels: u32,
    write: AtomicU64,
    read: AtomicU64,
    /// Frames missed (playback underrun) or dropped (capture overrun) since creation.
    pub xruns: AtomicU64,
}

// SAFETY: the producer only writes slots between `read` and `read + frames` that the consumer has released, the
// consumer only reads slots the producer has published with a release store of `write`.
unsafe impl Sync for PcmRing {}
unsafe impl Send for PcmRing {}

impl PcmRing {
    pub fn new(frames: u32, channels: u32) -> PcmRing {
        let n = frames as usize * channels as usize;
        PcmRing {
            data: (0..n).map(|_| UnsafeCell::new(0.0)).collect(),
            frames,
            channels,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            xruns: AtomicU64::new(0),
        }
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }
    pub fn channels(&self) -> u32 {
        self.channels
    }
    pub fn written(&self) -> u64 {
        self.write.load(Ordering::Acquire)
    }
    pub fn consumed(&self) -> u64 {
        self.read.load(Ordering::Acquire)
    }
    /// Frames queued.
    pub fn fill(&self) -> u32 {
        let r = self.read.load(Ordering::Acquire);
        let w = self.write.load(Ordering::Acquire);
        w.saturating_sub(r) as u32
    }
    /// Frames the producer may still push.
    pub fn writable(&self) -> u32 {
        self.frames - self.fill().min(self.frames)
    }

    /// Pushes up to `frames` frames; `src(i, frame)` fills the `channels` samples of frame `i`. Returns how many
    /// were pushed (fewer when the ring is full).
    pub fn push(&self, frames: u32, mut src: impl FnMut(u32, &mut [f32])) -> u32 {
        let n = frames.min(self.writable());
        let w = self.write.load(Ordering::Relaxed);
        let ch = self.channels as usize;
        for i in 0..n {
            let slot = ((w + i as u64) % self.frames as u64) as usize * ch;
            // SAFETY: slots [w, w+n) are free (n <= writable) and only this producer writes them; the range
            // stays inside `data` because slot + ch <= frames * ch.
            let frame = unsafe { core::slice::from_raw_parts_mut(self.data[slot].get(), ch) };
            src(i, frame);
        }
        self.write.store(w + n as u64, Ordering::Release);
        n
    }

    /// Pops up to `frames` frames; `sink(i, frame)` receives each. Returns how many were popped.
    pub fn pop(&self, frames: u32, mut sink: impl FnMut(u32, &[f32])) -> u32 {
        let n = frames.min(self.fill());
        let r = self.read.load(Ordering::Relaxed);
        let ch = self.channels as usize;
        for i in 0..n {
            let slot = ((r + i as u64) % self.frames as u64) as usize * ch;
            // SAFETY: slots [r, r+n) were published by the producer (n <= fill) and are not rewritten until
            // `read` advances below.
            let frame = unsafe { core::slice::from_raw_parts(self.data[slot].get(), ch) };
            sink(i, frame);
        }
        self.read.store(r + n as u64, Ordering::Release);
        n
    }

    /// Empties the ring (call only while neither side is running).
    pub fn reset(&self) {
        self.read.store(0, Ordering::Release);
        self.write.store(0, Ordering::Release);
    }
}

/// The sample formats the plug-in offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    S16,
    /// 24 bits in the low three bytes of a 32-bit word.
    S24,
    S32,
    F32,
}

impl Fmt {
    pub fn from_alsa(f: u32) -> Option<Fmt> {
        match f {
            crate::ffi::SND_PCM_FORMAT_S16_LE => Some(Fmt::S16),
            crate::ffi::SND_PCM_FORMAT_S24_LE => Some(Fmt::S24),
            crate::ffi::SND_PCM_FORMAT_S32_LE => Some(Fmt::S32),
            crate::ffi::SND_PCM_FORMAT_FLOAT_LE => Some(Fmt::F32),
            _ => None,
        }
    }
    pub fn bytes(self) -> usize {
        match self {
            Fmt::S16 => 2,
            _ => 4,
        }
    }

    /// Decodes one little-endian sample.
    pub fn decode(self, b: &[u8]) -> f32 {
        match self {
            Fmt::S16 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            Fmt::S24 => ((i32::from_le_bytes([b[0], b[1], b[2], b[3]]) << 8) >> 8) as f32 / 8_388_608.0,
            Fmt::S32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            Fmt::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    }

    /// Encodes one sample (clamped to full scale for the integer formats).
    pub fn encode(self, v: f32, out: &mut [u8]) {
        let v = if v.is_nan() { 0.0 } else { v };
        match self {
            Fmt::S16 => out[..2].copy_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()),
            Fmt::S24 => out[..4].copy_from_slice(&((v.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32).to_le_bytes()),
            Fmt::S32 => out[..4].copy_from_slice(&((v.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0).round() as i32).to_le_bytes()),
            Fmt::F32 => out[..4].copy_from_slice(&v.to_le_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_wraps() {
        let r = PcmRing::new(4, 2);
        assert_eq!(r.writable(), 4);
        assert_eq!(
            r.push(3, |i, f| {
                f[0] = i as f32;
                f[1] = -(i as f32);
            }),
            3
        );
        assert_eq!(r.fill(), 3);
        let mut got = vec![];
        assert_eq!(r.pop(2, |_, f| got.push((f[0], f[1]))), 2);
        assert_eq!(got, vec![(0.0, 0.0), (1.0, -1.0)]);
        // 1 queued, 3 free; pushing 3 wraps the end of the buffer
        assert_eq!(
            r.push(5, |i, f| {
                f[0] = 10.0 + i as f32;
                f[1] = 0.0;
            }),
            3
        );
        assert_eq!(r.fill(), 4);
        assert_eq!(r.writable(), 0);
        got.clear();
        assert_eq!(r.pop(9, |_, f| got.push((f[0], f[1]))), 4);
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![2.0, 10.0, 11.0, 12.0]);
        assert_eq!((r.written(), r.consumed()), (6, 6));
    }

    #[test]
    fn underrun_pops_less() {
        let r = PcmRing::new(8, 1);
        r.push(2, |_, f| f[0] = 0.5);
        assert_eq!(r.pop(5, |_, _| {}), 2);
        assert_eq!(r.fill(), 0);
        r.push(1, |_, f| f[0] = 1.0);
        r.reset();
        assert_eq!((r.written(), r.fill()), (0, 0));
    }

    #[test]
    fn decode_known_values() {
        assert_eq!(Fmt::S16.decode(&[0x00, 0x40]), 0.5);
        assert_eq!(Fmt::S16.decode(&[0x00, 0x80]), -1.0);
        assert_eq!(Fmt::S32.decode(&[0, 0, 0, 0x40]), 0.5);
        // 0xC00000 as 24 bit signed is -0x400000, which is -0.5
        assert_eq!(Fmt::S24.decode(&[0x00, 0x00, 0xC0, 0x00]), -0.5);
        assert_eq!(Fmt::F32.decode(&0.25f32.to_le_bytes()), 0.25);
    }

    #[test]
    fn encode_clamps_and_round_trips() {
        let mut b = [0u8; 4];
        Fmt::S16.encode(2.0, &mut b);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), 32767);
        Fmt::S16.encode(-2.0, &mut b);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), -32767);
        Fmt::S16.encode(f32::NAN, &mut b);
        assert_eq!(i16::from_le_bytes([b[0], b[1]]), 0);
        Fmt::S32.encode(1.0, &mut b);
        assert_eq!(i32::from_le_bytes(b), i32::MAX);
        Fmt::S24.encode(1.0, &mut b);
        assert_eq!(i32::from_le_bytes(b), 8_388_607);
        Fmt::F32.encode(0.125, &mut b);
        assert_eq!(Fmt::F32.decode(&b), 0.125);
    }

    #[test]
    fn alsa_format_codes() {
        assert_eq!(Fmt::from_alsa(2), Some(Fmt::S16));
        assert_eq!(Fmt::from_alsa(14), Some(Fmt::F32));
        assert_eq!(Fmt::from_alsa(0), None);
    }
}
