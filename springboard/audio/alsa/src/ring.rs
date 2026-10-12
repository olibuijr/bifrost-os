//! The DMA ring arithmetic, free of the kernel so it is testable on the host: pointer wrap-around against the
//! stream's `boundary`, the layout of interleaved and non-interleaved rings, and the plans for one write (playback)
//! or one read (capture) including the recovery from an under- or overrun.

use crate::convert::{pack_channel, silence_ring, unpack_channel, SampleFormat};
use bifrost_audio_types::Buffer;

/// Signed distance `a - b` of two pointers that wrap at `boundary`, in `[-boundary/2, boundary/2)`.
pub fn ptr_diff(a: u64, b: u64, boundary: u64) -> i64 {
    let bd = boundary as i128;
    let d = (a as i128 - b as i128).rem_euclid(bd);
    (if d >= bd / 2 { d - bd } else { d }) as i64
}

/// Adds `frames` to a pointer, wrapping at `boundary`.
pub fn ptr_add(p: u64, frames: u64, boundary: u64) -> u64 {
    ((p as u128 + frames as u128) % boundary as u128) as u64
}

/// Unwraps a hardware pointer reading into a running frame count: the forward distance from the last reading,
/// added to the total.
pub fn unwrap_ptr(last_raw: u64, total: u64, raw: u64, boundary: u64) -> u64 {
    let fwd = (raw as i128 - last_raw as i128).rem_euclid(boundary as i128) as u64;
    total + fwd
}

/// How the samples sit in the mmapped ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingLayout {
    pub interleaved: bool,
    pub channels: usize,
    pub format: SampleFormat,
    pub buffer_frames: usize,
}

impl RingLayout {
    pub fn bytes_per_frame(&self) -> usize {
        self.channels * self.format.bytes()
    }

    pub fn ring_bytes(&self) -> usize {
        self.bytes_per_frame() * self.buffer_frames
    }

    /// Byte offset and stride of channel `c` starting at ring frame `idx`.
    pub fn channel_span(&self, c: usize, idx: usize) -> (usize, usize) {
        let w = self.format.bytes();
        if self.interleaved {
            ((idx * self.channels + c) * w, self.channels * w)
        } else {
            ((c * self.buffer_frames + idx) * w, w)
        }
    }

    /// Converts `frames` frames of `bufs` (one non-interleaved buffer per channel; missing channels are silent)
    /// into the ring at frame `at` (any value; reduced modulo the ring), wrapping.
    pub fn pack(&self, mem: &mut [u8], at: u64, bufs: &[Buffer], frames: usize) {
        let start = (at % self.buffer_frames as u64) as usize;
        let first = frames.min(self.buffer_frames - start);
        for c in 0..self.channels {
            let src: &[f32] = bufs.get(c).map(|b| b.data.as_slice()).unwrap_or(&[]);
            let n = src.len().min(frames);
            let (o, s) = self.channel_span(c, start);
            pack_channel(self.format, mem, o, s, &src[..n.min(first)]);
            if n > first {
                let (o2, s2) = self.channel_span(c, 0);
                pack_channel(self.format, mem, o2, s2, &src[first..n]);
            }
            // a short or missing buffer leaves the rest of the cycle silent
            if n < frames {
                self.silence(mem, at + n as u64, frames - n, Some(c));
            }
        }
    }

    /// Converts `frames` frames from the ring at frame `at` into `dst` (one buffer per channel).
    pub fn unpack(&self, mem: &[u8], at: u64, dst: &mut [Buffer], frames: usize) {
        let start = (at % self.buffer_frames as u64) as usize;
        let first = frames.min(self.buffer_frames - start);
        for c in 0..self.channels.min(dst.len()) {
            let n = dst[c].data.len().min(frames);
            let (o, s) = self.channel_span(c, start);
            unpack_channel(self.format, mem, o, s, &mut dst[c].data[..n.min(first)]);
            if n > first {
                let (o2, s2) = self.channel_span(c, 0);
                unpack_channel(self.format, mem, o2, s2, &mut dst[c].data[first..n]);
            }
        }
    }

    /// Zeroes `frames` frames at ring frame `at`: all channels, or only `only`.
    pub fn silence(&self, mem: &mut [u8], at: u64, frames: usize, only: Option<usize>) {
        let start = (at % self.buffer_frames as u64) as usize;
        match only {
            None => silence_ring(mem, self.bytes_per_frame(), self.buffer_frames, start, frames, self.interleaved, self.channels, self.format.bytes()),
            Some(c) => {
                for f in 0..frames {
                    let idx = (start + f) % self.buffer_frames;
                    let (o, _) = self.channel_span(c, idx);
                    if let Some(s) = mem.get_mut(o..o + self.format.bytes()) {
                        s.fill(0);
                    }
                }
            }
        }
    }
}

/// What to do with one playback cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPlan {
    /// Write the cycle at `write_at`.
    Write { write_at: u64 },
    /// The hardware ran past the application pointer by `lost` frames. Silence `silence_frames` frames from
    /// `silence_from`, then write the cycle at `write_at`.
    Underrun { lost: u32, silence_from: u64, silence_frames: u32, write_at: u64 },
    /// The ring has no room for the cycle (the server is ahead of the hardware): drop the cycle.
    Overflow,
}

/// Plans a playback write of `frames` frames given the application pointer `appl`, the hardware pointer `hw`
/// (both modulo `boundary`), the ring size and the fill the driver keeps after a recovery.
pub fn plan_playback(appl: u64, hw: u64, buffer_frames: u32, boundary: u64, frames: u32, prefill: u32) -> PlaybackPlan {
    let fill = ptr_diff(appl, hw, boundary);
    if fill < 0 {
        return PlaybackPlan::Underrun { lost: (-fill) as u32, silence_from: hw, silence_frames: prefill, write_at: ptr_add(hw, prefill as u64, boundary) };
    }
    if fill as u64 + frames as u64 > buffer_frames as u64 {
        return PlaybackPlan::Overflow;
    }
    PlaybackPlan::Write { write_at: appl }
}

/// What to do with one capture cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePlan {
    /// Read the cycle from `read_at`.
    Read { read_at: u64 },
    /// Fewer than `frames` frames are captured yet (`avail`): hand out silence, keep the data.
    NotReady { avail: u32 },
    /// The hardware overwrote unread data; `lost` frames are gone. Read the newest cycle from `read_at`.
    Overrun { lost: u32, read_at: u64 },
}

pub fn plan_capture(appl: u64, hw: u64, buffer_frames: u32, boundary: u64, frames: u32) -> CapturePlan {
    let avail = ptr_diff(hw, appl, boundary);
    if avail < 0 {
        return CapturePlan::NotReady { avail: 0 };
    }
    let avail = avail as u64;
    if avail > buffer_frames as u64 {
        let read_at = (hw as i128 - frames as i128).rem_euclid(boundary as i128) as u64;
        return CapturePlan::Overrun { lost: (avail - frames as u64) as u32, read_at };
    }
    if avail >= frames as u64 {
        CapturePlan::Read { read_at: appl }
    } else {
        CapturePlan::NotReady { avail: avail as u32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BD: u64 = 1 << 20; // a boundary of 1 Mi frames

    #[test]
    fn ptr_diff_wraps_at_the_boundary() {
        assert_eq!(ptr_diff(100, 40, BD), 60);
        assert_eq!(ptr_diff(40, 100, BD), -60);
        // 10 just after the wrap vs 1 Mi - 10 just before it: 20 frames apart
        assert_eq!(ptr_diff(10, BD - 10, BD), 20);
        assert_eq!(ptr_diff(BD - 10, 10, BD), -20);
        assert_eq!(ptr_diff(7, 7, BD), 0);
    }

    #[test]
    fn ptr_add_wraps() {
        assert_eq!(ptr_add(BD - 5, 8, BD), 3);
        assert_eq!(ptr_add(5, 8, BD), 13);
    }

    #[test]
    fn hardware_pointer_unwraps_into_a_running_total() {
        assert_eq!(unwrap_ptr(0, 0, 64, BD), 64);
        assert_eq!(unwrap_ptr(64, 64, 128, BD), 128);
        // wrap: from 1 Mi - 16 to 48 is 64 frames forward
        assert_eq!(unwrap_ptr(BD - 16, 5_000_000, 48, BD), 5_000_064);
        // no movement
        assert_eq!(unwrap_ptr(300, 900, 300, BD), 900);
    }

    fn layout(interleaved: bool) -> RingLayout {
        RingLayout { interleaved, channels: 2, format: SampleFormat::S16, buffer_frames: 4 }
    }

    fn bufs(l: &[f32], r: &[f32]) -> Vec<Buffer> {
        vec![Buffer { channels: 1, data: l.to_vec() }, Buffer { channels: 1, data: r.to_vec() }]
    }

    fn s16(v: i16) -> f32 {
        v as f32 / 32768.0
    }

    #[test]
    fn layout_spans() {
        let i = layout(true);
        assert_eq!(i.bytes_per_frame(), 4);
        assert_eq!(i.ring_bytes(), 16);
        assert_eq!(i.channel_span(1, 2), (6, 4));
        let n = layout(false);
        assert_eq!(n.channel_span(1, 2), (12, 2));
    }

    #[test]
    fn pack_wraps_interleaved() {
        let l = layout(true);
        let mut mem = vec![0u8; 16];
        // 3 frames starting at ring frame 3: frame 3, then 0 and 1
        l.pack(&mut mem, 3, &bufs(&[s16(1), s16(2), s16(3)], &[s16(-1), s16(-2), s16(-3)]), 3);
        // frame 0 = (2, -2), frame 1 = (3, -3), frame 2 untouched, frame 3 = (1, -1)
        assert_eq!(mem, [2, 0, 0xfe, 0xff, 3, 0, 0xfd, 0xff, 0, 0, 0, 0, 1, 0, 0xff, 0xff]);
    }

    #[test]
    fn pack_wraps_non_interleaved() {
        let l = layout(false);
        let mut mem = vec![0u8; 16];
        l.pack(&mut mem, 7, &bufs(&[s16(1), s16(2)], &[s16(5), s16(6)]), 2);
        // ring frame 3 then 0: channel 0 in bytes 0..8, channel 1 in bytes 8..16
        assert_eq!(&mem[..8], &[2, 0, 0, 0, 0, 0, 1, 0]);
        assert_eq!(&mem[8..], &[6, 0, 0, 0, 0, 0, 5, 0]);
    }

    #[test]
    fn a_short_buffer_leaves_the_rest_of_the_cycle_silent() {
        let l = layout(true);
        let mut mem = vec![0xffu8; 16];
        l.pack(&mut mem, 0, &bufs(&[s16(1)], &[s16(2)]), 2);
        assert_eq!(&mem[..8], &[1, 0, 2, 0, 0, 0, 0, 0]);
        // a missing channel is silent for the whole cycle
        let mut mem = vec![0xffu8; 16];
        l.pack(&mut mem, 0, &bufs(&[s16(1), s16(1)], &[]).into_iter().take(1).collect::<Vec<_>>(), 2);
        assert_eq!(&mem[..8], &[1, 0, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn unpack_is_the_inverse_of_pack() {
        for interleaved in [true, false] {
            let l = layout(interleaved);
            let mut mem = vec![0u8; 16];
            let src = bufs(&[s16(100), s16(200), s16(300)], &[s16(-100), s16(-200), s16(-300)]);
            l.pack(&mut mem, 2, &src, 3);
            let mut dst = bufs(&[0.0; 3], &[0.0; 3]);
            l.unpack(&mem, 2, &mut dst, 3);
            assert_eq!(dst, src, "interleaved={interleaved}");
        }
    }

    #[test]
    fn playback_normal_writes_at_the_application_pointer() {
        // 128 queued of a 1024 ring: room for 64 more
        assert_eq!(plan_playback(1128, 1000, 1024, BD, 64, 128), PlaybackPlan::Write { write_at: 1128 });
    }

    #[test]
    fn playback_underrun_resumes_prefill_frames_after_the_hardware() {
        // hardware at 1200, application stopped at 1128: 72 frames lost
        assert_eq!(
            plan_playback(1128, 1200, 1024, BD, 64, 128),
            PlaybackPlan::Underrun { lost: 72, silence_from: 1200, silence_frames: 128, write_at: 1328 }
        );
    }

    #[test]
    fn playback_underrun_across_the_wrap() {
        let p = plan_playback(BD - 8, 40, 1024, BD, 64, 128);
        assert_eq!(p, PlaybackPlan::Underrun { lost: 48, silence_from: 40, silence_frames: 128, write_at: 168 });
    }

    #[test]
    fn playback_overflow_drops_the_cycle() {
        // 1000 queued of 1024, 64 do not fit
        assert_eq!(plan_playback(2000, 1000, 1024, BD, 64, 128), PlaybackPlan::Overflow);
        // exactly full is fine
        assert_eq!(plan_playback(1960, 1000, 1024, BD, 64, 128), PlaybackPlan::Write { write_at: 1960 });
    }

    #[test]
    fn capture_reads_when_a_cycle_is_available() {
        assert_eq!(plan_capture(1000, 1128, 1024, BD, 64), CapturePlan::Read { read_at: 1000 });
        assert_eq!(plan_capture(1000, 1064, 1024, BD, 64), CapturePlan::Read { read_at: 1000 });
    }

    #[test]
    fn capture_waits_when_the_cycle_is_incomplete() {
        assert_eq!(plan_capture(1000, 1040, 1024, BD, 64), CapturePlan::NotReady { avail: 40 });
        assert_eq!(plan_capture(1000, 1000, 1024, BD, 64), CapturePlan::NotReady { avail: 0 });
    }

    #[test]
    fn capture_overrun_reads_the_newest_cycle() {
        // 1500 frames unread in a 1024 ring: the newest 64 end at the hardware pointer 2500
        assert_eq!(plan_capture(1000, 2500, 1024, BD, 64), CapturePlan::Overrun { lost: 1436, read_at: 2436 });
    }
}
