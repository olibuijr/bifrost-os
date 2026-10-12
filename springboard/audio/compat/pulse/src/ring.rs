//! The lock-free single-producer single-consumer ring between the protocol thread and a real-time IO procedure,
//! and the linear resampler the consumer uses. Nothing here allocates, locks or makes a syscall after construction.
//! Samples are stored as f32 bit patterns in `AtomicU32`s, so the ring needs no `unsafe`.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// An SPSC ring of f32 samples (interleaved by the caller).
pub struct Ring {
    data: Vec<AtomicU32>,
    /// Total samples ever consumed / produced (monotonic; the difference is the fill).
    head: AtomicUsize,
    tail: AtomicUsize,
    /// A producer-requested discard: the consumer skips to this position.
    flush_mark: AtomicUsize,
}

impl Ring {
    pub fn new(capacity: usize) -> Ring {
        Ring { data: (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect(), head: AtomicUsize::new(0), tail: AtomicUsize::new(0), flush_mark: AtomicUsize::new(0) }
    }
    pub fn capacity(&self) -> usize {
        self.data.len()
    }
    fn effective_head(&self) -> usize {
        self.head.load(Ordering::Acquire).max(self.flush_mark.load(Ordering::Acquire))
    }
    /// Samples buffered.
    pub fn available(&self) -> usize {
        self.tail.load(Ordering::Acquire).saturating_sub(self.effective_head())
    }
    /// Samples that fit.
    pub fn space(&self) -> usize {
        self.capacity() - self.available()
    }
    /// Producer: appends as many samples as fit; returns the count.
    pub fn write(&self, src: &[f32]) -> usize {
        let n = src.len().min(self.space());
        let tail = self.tail.load(Ordering::Relaxed);
        for (i, s) in src[..n].iter().enumerate() {
            self.data[(tail + i) % self.data.len()].store(s.to_bits(), Ordering::Relaxed);
        }
        self.tail.store(tail + n, Ordering::Release);
        n
    }
    /// Consumer: reads up to `dst.len()` samples; returns the count.
    pub fn read(&self, dst: &mut [f32]) -> usize {
        let head = self.effective_head();
        let n = dst.len().min(self.tail.load(Ordering::Acquire).saturating_sub(head));
        for (i, d) in dst[..n].iter_mut().enumerate() {
            *d = f32::from_bits(self.data[(head + i) % self.data.len()].load(Ordering::Relaxed));
        }
        self.head.store(head + n, Ordering::Release);
        n
    }
    /// Producer: asks the consumer to drop everything written so far.
    pub fn request_flush(&self) {
        self.flush_mark.store(self.tail.load(Ordering::Acquire), Ordering::Release);
    }
}

/// Linear-interpolation sample rate converter pulling interleaved frames from a [`Ring`].
pub struct Resampler {
    channels: usize,
    /// Input frames consumed per output frame.
    ratio: f64,
    frac: f64,
    have: u8,
    prev: [f32; MAX_CHANNELS],
    next: [f32; MAX_CHANNELS],
}

pub const MAX_CHANNELS: usize = 8;

impl Resampler {
    pub fn new(channels: usize, in_rate: u32, out_rate: u32) -> Resampler {
        Resampler { channels: channels.clamp(1, MAX_CHANNELS), ratio: in_rate as f64 / out_rate.max(1) as f64, frac: 0.0, have: 0, prev: [0.0; MAX_CHANNELS], next: [0.0; MAX_CHANNELS] }
    }
    pub fn set_rates(&mut self, in_rate: u32, out_rate: u32) {
        self.ratio = in_rate as f64 / out_rate.max(1) as f64;
    }
    fn read_frame(&self, ring: &Ring, f: &mut [f32; MAX_CHANNELS]) -> bool {
        ring.read(&mut f[..self.channels]) == self.channels
    }
    /// Fills `out` (whole frames) and returns the number of frames produced; fewer than asked means the ring ran dry.
    pub fn pull(&mut self, ring: &Ring, out: &mut [f32]) -> usize {
        let ch = self.channels;
        if self.ratio == 1.0 && self.have == 0 {
            let n = out.len() / ch * ch;
            return ring.read(&mut out[..n]) / ch;
        }
        let mut tmp = [0.0f32; MAX_CHANNELS];
        while self.have < 2 {
            if !self.read_frame(ring, &mut tmp) {
                return 0;
            }
            self.prev = self.next;
            self.next = tmp;
            self.have += 1;
            if self.have == 1 {
                // the first frame is both neighbours until the second arrives
                self.prev = tmp;
            }
        }
        let mut produced = 0;
        for frame in out.chunks_exact_mut(ch) {
            while self.frac >= 1.0 {
                if !self.read_frame(ring, &mut tmp) {
                    return produced;
                }
                self.prev = self.next;
                self.next = tmp;
                self.frac -= 1.0;
            }
            let f = self.frac as f32;
            for c in 0..ch {
                frame[c] = self.prev[c] + (self.next[c] - self.prev[c]) * f;
            }
            self.frac += self.ratio;
            produced += 1;
        }
        produced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_wraps() {
        let r = Ring::new(4);
        assert_eq!(r.write(&[1.0, 2.0, 3.0]), 3);
        let mut b = [0.0; 2];
        assert_eq!(r.read(&mut b), 2);
        assert_eq!(b, [1.0, 2.0]);
        // one left; room for three more (capacity 4)
        assert_eq!(r.space(), 3);
        assert_eq!(r.write(&[4.0, 5.0, 6.0, 7.0]), 3);
        let mut c = [0.0; 8];
        assert_eq!(r.read(&mut c), 4);
        assert_eq!(&c[..4], &[3.0, 4.0, 5.0, 6.0]);
        assert_eq!(r.available(), 0);
    }

    #[test]
    fn full_ring_accepts_nothing() {
        let r = Ring::new(2);
        assert_eq!(r.write(&[1.0, 1.0, 1.0]), 2);
        assert_eq!(r.write(&[1.0]), 0);
    }

    #[test]
    fn flush_drops_what_was_written_before_but_not_after() {
        let r = Ring::new(8);
        r.write(&[1.0, 2.0, 3.0]);
        r.request_flush();
        assert_eq!(r.available(), 0);
        r.write(&[9.0]);
        let mut b = [0.0; 4];
        assert_eq!(r.read(&mut b), 1);
        assert_eq!(b[0], 9.0);
    }

    #[test]
    fn same_rate_is_a_copy() {
        let r = Ring::new(16);
        r.write(&[1.0, 2.0, 3.0, 4.0]);
        let mut rs = Resampler::new(2, 48000, 48000);
        let mut out = [0.0; 8];
        assert_eq!(rs.pull(&r, &mut out), 2);
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn upsample_by_two_interpolates() {
        let r = Ring::new(32);
        let src: Vec<f32> = (0..8).map(|i| i as f32).collect();
        r.write(&src);
        let mut rs = Resampler::new(1, 24000, 48000);
        let mut out = [0.0; 8];
        assert_eq!(rs.pull(&r, &mut out), 8);
        assert_eq!(out, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]);
    }

    #[test]
    fn downsample_by_two_decimates() {
        let r = Ring::new(32);
        let src: Vec<f32> = (0..10).map(|i| i as f32).collect();
        r.write(&src);
        let mut rs = Resampler::new(1, 48000, 24000);
        let mut out = [0.0; 4];
        assert_eq!(rs.pull(&r, &mut out), 4);
        assert_eq!(out, [0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn dry_ring_returns_fewer_frames_and_resumes() {
        let r = Ring::new(32);
        r.write(&[0.0, 1.0, 2.0]);
        let mut rs = Resampler::new(1, 24000, 48000);
        let mut out = [0.0; 8];
        // outputs 0, 0.5, 1, 1.5 need frames 0..2; the fifth (2.0) needs frame 3
        assert_eq!(rs.pull(&r, &mut out), 4);
        assert_eq!(&out[..4], &[0.0, 0.5, 1.0, 1.5]);
        r.write(&[3.0]);
        let mut more = [0.0; 2];
        assert_eq!(rs.pull(&r, &mut more), 2);
        assert_eq!(more, [2.0, 2.5]);
    }
}
