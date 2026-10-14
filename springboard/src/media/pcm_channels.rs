//! Planar PCM for a multi-channel buffer: one sample buffer per channel, the layout the output and the sample
//! conversions work with. The decoder produces interleaved S16 frames; [`ChannelBuffers::from_interleaved`] splits
//! them into one buffer per channel and [`ChannelBuffers::to_interleaved`] joins them back. Every channel has the same
//! number of frames, because the type builds the buffers together and gives no way to change one alone.

/// The most channels one buffer holds.
pub const MAX_CHANNELS: usize = 8;

/// Sample buffers for each channel of one block of audio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelBuffers {
    channels: Vec<Vec<i16>>,
}

impl ChannelBuffers {
    /// Splits interleaved samples into one buffer per channel. `None` when the channel count is zero or above
    /// [`MAX_CHANNELS`], or when the samples do not fill whole frames.
    pub fn from_interleaved(samples: &[i16], channels: usize) -> Option<Self> {
        if channels == 0 || channels > MAX_CHANNELS || samples.len() % channels != 0 {
            return None;
        }
        let frames = samples.len() / channels;
        let mut out = vec![Vec::with_capacity(frames); channels];
        for frame in samples.chunks_exact(channels) {
            for (buffer, &sample) in out.iter_mut().zip(frame) {
                buffer.push(sample);
            }
        }
        Some(ChannelBuffers { channels: out })
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// The number of frames, one sample per channel each.
    pub fn frame_count(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// The samples of one channel, `None` when there is no such channel.
    pub fn channel(&self, index: usize) -> Option<&[i16]> {
        self.channels.get(index).map(Vec::as_slice)
    }

    /// The samples in interleaved order, frame by frame.
    pub fn to_interleaved(&self) -> Vec<i16> {
        let mut out = Vec::with_capacity(self.frame_count() * self.channel_count());
        for frame in 0..self.frame_count() {
            for channel in &self.channels {
                out.push(channel[frame]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_samples_split_into_a_left_and_a_right_buffer() {
        let buffers = ChannelBuffers::from_interleaved(&[1, -1, 2, -2, 3, -3], 2).unwrap();
        assert_eq!(buffers.channel_count(), 2);
        assert_eq!(buffers.frame_count(), 3);
        assert_eq!(buffers.channel(0), Some(&[1, 2, 3][..]));
        assert_eq!(buffers.channel(1), Some(&[-1, -2, -3][..]));
        assert_eq!(buffers.channel(2), None);
    }

    #[test]
    fn joining_the_channels_restores_the_interleaved_order() {
        let samples = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let buffers = ChannelBuffers::from_interleaved(&samples, 4).unwrap();
        assert_eq!(buffers.frame_count(), 2);
        assert_eq!(buffers.to_interleaved(), samples);
    }

    #[test]
    fn a_mono_block_is_one_buffer_of_its_samples() {
        let buffers = ChannelBuffers::from_interleaved(&[7, 8, 9], 1).unwrap();
        assert_eq!(buffers.channel(0), Some(&[7, 8, 9][..]));
    }

    #[test]
    fn samples_that_do_not_fill_whole_frames_are_refused() {
        assert!(ChannelBuffers::from_interleaved(&[1, 2, 3], 2).is_none());
        assert!(ChannelBuffers::from_interleaved(&[1, 2], 0).is_none());
        assert!(ChannelBuffers::from_interleaved(&[0; 9], MAX_CHANNELS + 1).is_none());
    }

    #[test]
    fn an_empty_block_has_no_frames() {
        let buffers = ChannelBuffers::from_interleaved(&[], 2).unwrap();
        assert_eq!(buffers.frame_count(), 0);
        assert!(buffers.to_interleaved().is_empty());
        assert_eq!(buffers.channel(1), Some(&[][..]));
    }
}
