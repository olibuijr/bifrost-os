//! Buffer lists: the audio an IOProc reads and writes. The server's internal format is f32 non-interleaved, so a
//! list holds one buffer per channel. Owner: task 01.

use alloc::vec::Vec;

/// One channel's (or one interleaved group's) samples.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Buffer {
    pub channels: u32,
    pub data: Vec<f32>,
}

/// The buffers of one cycle.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BufferList {
    pub buffers: Vec<Buffer>,
}

impl BufferList {
    /// `channels` non-interleaved buffers of `frames` zeros.
    pub fn silent(channels: u32, frames: u32) -> BufferList {
        let buffers = (0..channels)
            .map(|_| Buffer { channels: 1, data: alloc::vec![0.0; frames as usize] })
            .collect();
        BufferList { buffers }
    }

    /// Frames per buffer (all buffers have the same length).
    pub fn frames(&self) -> u32 {
        self.buffers.first().map_or(0, |b| (b.data.len() as u32) / b.channels.max(1))
    }

    /// Writes the list into an interleaved slice (`frames * channels` long).
    pub fn interleave_into(&self, out: &mut [f32]) {
        let n = self.buffers.len();
        let frames = (self.frames() as usize).min(out.len() / n.max(1));
        for (c, b) in self.buffers.iter().enumerate() {
            for (f, s) in b.data.iter().take(frames).enumerate() {
                out[f * n + c] = *s;
            }
        }
    }

    /// Fills the list from an interleaved slice.
    pub fn deinterleave_from(&mut self, src: &[f32]) {
        let n = self.buffers.len();
        if n == 0 {
            return;
        }
        for (c, b) in self.buffers.iter_mut().enumerate() {
            for (f, s) in b.data.iter_mut().enumerate() {
                if let Some(v) = src.get(f * n + c) {
                    *s = *v;
                }
            }
        }
    }

    pub fn clear(&mut self) {
        for b in &mut self.buffers {
            b.data.iter_mut().for_each(|s| *s = 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_has_shape() {
        let l = BufferList::silent(2, 4);
        assert_eq!(l.buffers.len(), 2);
        assert_eq!(l.frames(), 4);
        assert!(l.buffers.iter().all(|b| b.data.iter().all(|s| *s == 0.0)));
        assert_eq!(BufferList::default().frames(), 0);
    }

    #[test]
    fn interleave_round_trip() {
        let mut l = BufferList::silent(2, 3);
        l.deinterleave_from(&[1.0, 10.0, 2.0, 20.0, 3.0, 30.0]);
        assert_eq!(l.buffers[0].data, [1.0, 2.0, 3.0]);
        assert_eq!(l.buffers[1].data, [10.0, 20.0, 30.0]);
        let mut out = [0.0; 6];
        l.interleave_into(&mut out);
        assert_eq!(out, [1.0, 10.0, 2.0, 20.0, 3.0, 30.0]);
        l.clear();
        assert!(l.buffers[1].data.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn short_slices_do_not_panic() {
        let mut l = BufferList::silent(2, 3);
        l.deinterleave_from(&[1.0, 2.0, 3.0]);
        assert_eq!(l.buffers[0].data, [1.0, 3.0, 0.0]);
        assert_eq!(l.buffers[1].data, [2.0, 0.0, 0.0]);
        let mut out = [9.0; 4];
        l.interleave_into(&mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, 0.0]);
    }
}
