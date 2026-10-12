//! The mixer: f32 sums into the device's master buffer, the per-stream volume ramp (no zipper noise) and the
//! conversion from the internal f32 format to the physical integer formats (docs/audio-architecture.md, "Formats",
//! IO cycle steps MixOutput and ConvertMix). Pure functions: no allocation, no locks, so they are safe on the IO
//! thread. Owner: task 05.

use bifrost_audio_types::BufferList;

/// Frames over which a volume change ramps (10 ms at 48 kHz), so a change never steps.
pub const RAMP_FRAMES: u32 = 480;

/// A linear gain ramp: `current` moves to `target` by `step` per frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeRamp {
    current: f32,
    target: f32,
    step: f32,
}

impl VolumeRamp {
    pub const fn new(gain: f32) -> VolumeRamp {
        VolumeRamp { current: gain, target: gain, step: 0.0 }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn settled(&self) -> bool {
        self.step == 0.0
    }

    /// Ramps to `target` over `ramp_frames` frames (0 = jump).
    pub fn set_target(&mut self, target: f32, ramp_frames: u32) {
        self.target = target;
        if ramp_frames == 0 || target == self.current {
            self.current = target;
            self.step = 0.0;
        } else {
            self.step = (target - self.current) / ramp_frames as f32;
        }
    }

    /// The gain for the next frame.
    #[inline]
    pub fn next_gain(&mut self) -> f32 {
        if self.step != 0.0 {
            self.current += self.step;
            if (self.step > 0.0 && self.current >= self.target) || (self.step < 0.0 && self.current <= self.target) {
                self.current = self.target;
                self.step = 0.0;
            }
        }
        self.current
    }
}

/// Adds `frames` frames of a stream (`src`, one plane per client channel) into `master` (one buffer per device
/// channel) with the stream's ramped gain. Device channel `d` takes client channel `d % src.len()`, so a mono
/// client plays on every channel and extra client channels are dropped.
pub fn mix_stream(master: &mut BufferList, src: &[&mut [f32]], ramp: &mut VolumeRamp, frames: usize) {
    if src.is_empty() {
        return;
    }
    for f in 0..frames {
        let g = ramp.next_gain();
        for (d, mb) in master.buffers.iter_mut().enumerate() {
            mb.data[f] += src[d % src.len()][f] * g;
        }
    }
}

/// Zeroes the first `frames` frames of every master buffer.
pub fn clear_master(master: &mut BufferList, frames: usize) {
    for b in &mut master.buffers {
        b.data[..frames].iter_mut().for_each(|s| *s = 0.0);
    }
}

/// Converts the master to interleaved signed integers of `bits` (16, 24 or 32), right-aligned in an `i32` (so the
/// 32-bit case uses the whole word). Values are clamped, so a sum above full scale clips instead of wrapping.
/// `out` is `frames * channels` long.
pub fn pack_interleaved(src: &BufferList, frames: usize, bits: u32, out: &mut [i32]) {
    let ch = src.buffers.len();
    let scale = (1u64 << (bits - 1)) as f64;
    let (lo, hi) = (-scale, scale - 1.0);
    for f in 0..frames {
        for (c, b) in src.buffers.iter().enumerate() {
            out[f * ch + c] = (b.data[f] as f64 * scale).round().clamp(lo, hi) as i32;
        }
    }
}

/// The inverse of `pack_interleaved` for capture: interleaved integers to planar f32.
pub fn unpack_interleaved(src: &[i32], frames: usize, bits: u32, dst: &mut BufferList) {
    let ch = dst.buffers.len();
    let scale = (1u64 << (bits - 1)) as f64;
    for f in 0..frames {
        for (c, b) in dst.buffers.iter_mut().enumerate() {
            b.data[f] = (src[f * ch + c] as f64 / scale) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;

    fn bl(ch: usize, frames: usize, v: f32) -> BufferList {
        BufferList { buffers: (0..ch).map(|_| Buffer { channels: 1, data: vec![v; frames] }).collect() }
    }

    #[test]
    fn ramp_is_linear_and_lands_exactly() {
        let mut r = VolumeRamp::new(0.0);
        r.set_target(1.0, 4);
        let g: Vec<f32> = (0..6).map(|_| r.next_gain()).collect();
        assert_eq!(g, vec![0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
        assert!(r.settled());
    }

    #[test]
    fn ramp_down_and_jump() {
        let mut r = VolumeRamp::new(1.0);
        r.set_target(0.5, 2);
        assert_eq!(r.next_gain(), 0.75);
        assert_eq!(r.next_gain(), 0.5);
        r.set_target(0.0, 0);
        assert_eq!(r.next_gain(), 0.0);
    }

    #[test]
    fn two_streams_sum_sample_exactly() {
        let mut master = bl(2, 4, 0.0);
        let mut a0 = vec![0.25f32, 0.5, -0.25, 0.0];
        let mut a1 = vec![0.0f32; 4];
        let mut b0 = vec![0.125f32; 4];
        let mut b1 = vec![1.0f32; 4];
        let sa: [&mut [f32]; 2] = [&mut a0[..], &mut a1[..]];
        let sb: [&mut [f32]; 2] = [&mut b0[..], &mut b1[..]];
        mix_stream(&mut master, &sa, &mut VolumeRamp::new(1.0), 4);
        mix_stream(&mut master, &sb, &mut VolumeRamp::new(1.0), 4);
        assert_eq!(master.buffers[0].data, vec![0.375, 0.625, -0.125, 0.125]);
        assert_eq!(master.buffers[1].data, vec![1.0; 4]);
    }

    #[test]
    fn mono_client_plays_on_both_channels_and_gain_applies() {
        let mut master = bl(2, 2, 0.0);
        let mut m = vec![0.5f32, -0.5];
        let s: [&mut [f32]; 1] = [&mut m[..]];
        mix_stream(&mut master, &s, &mut VolumeRamp::new(0.5), 2);
        assert_eq!(master.buffers[0].data, vec![0.25, -0.25]);
        assert_eq!(master.buffers[1].data, vec![0.25, -0.25]);
    }

    #[test]
    fn clear_master_zeroes_prefix() {
        let mut master = bl(1, 4, 1.0);
        clear_master(&mut master, 2);
        assert_eq!(master.buffers[0].data, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn pack_s16_s24_s32_and_clipping() {
        let mut m = bl(2, 2, 0.0);
        m.buffers[0].data = vec![0.5, 1.0];
        m.buffers[1].data = vec![-1.0, 2.0];
        let mut o = [0i32; 4];
        pack_interleaved(&m, 2, 16, &mut o);
        assert_eq!(o, [16384, -32768, 32767, 32767]);
        pack_interleaved(&m, 2, 24, &mut o);
        assert_eq!(o, [4194304, -8388608, 8388607, 8388607]);
        pack_interleaved(&m, 2, 32, &mut o);
        assert_eq!(o, [1073741824, i32::MIN, i32::MAX, i32::MAX]);
    }

    #[test]
    fn unpack_inverts_pack() {
        let mut d = bl(2, 2, 0.0);
        unpack_interleaved(&[16384, -32768, 0, 8192], 2, 16, &mut d);
        assert_eq!(d.buffers[0].data, vec![0.5, 0.0]);
        assert_eq!(d.buffers[1].data, vec![-1.0, 0.25]);
    }
}
