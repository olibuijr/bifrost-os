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
        let _ = (channels, frames);
        todo!("audio-01")
    }

    /// Frames per buffer (all buffers have the same length).
    pub fn frames(&self) -> u32 {
        let _ = self;
        todo!("audio-01")
    }

    /// Writes the list into an interleaved slice (`frames * channels` long).
    pub fn interleave_into(&self, out: &mut [f32]) {
        let _ = out;
        todo!("audio-01")
    }

    /// Fills the list from an interleaved slice.
    pub fn deinterleave_from(&mut self, src: &[f32]) {
        let _ = src;
        todo!("audio-01")
    }

    pub fn clear(&mut self) {
        for b in &mut self.buffers {
            b.data.iter_mut().for_each(|s| *s = 0.0);
        }
    }
}
