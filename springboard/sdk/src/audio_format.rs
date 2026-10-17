//! Audio stream formats for apps: the format description the audio server uses (`StreamFormat`, the stream basic
//! description of the reference platform, from `bifrost-audio-types`), the format of a voice take (`pcm16`, the 16-bit
//! PCM that the recorder writes) and the duration of a buffer in a format (`duration_ms`). An app reads a take's
//! format from the audio service and sizes its buffers and its progress from it, instead of passing a rate and a
//! channel count around as two loose numbers.

pub use bifrost_audio_types::{FormatFlags, FormatId, StreamFormat};
pub use springboard::media::capture::pcm16;

/// Whole milliseconds of `bytes` of interleaved PCM in `format`, rounded down. A partial frame at the end is not
/// counted. `None` when the format is not interleaved (its byte count is per channel buffer), has no frame size, or
/// has no sample rate.
pub fn duration_ms(format: &StreamFormat, bytes: u64) -> Option<u64> {
    if !format.is_interleaved() || format.bytes_per_frame == 0 || !(format.sample_rate > 0.0) {
        return None;
    }
    let frames = bytes / format.bytes_per_frame as u64;
    Some((frames as f64 * 1000.0 / format.sample_rate) as u64)
}

/// Why a buffer list could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferListError {
    /// No channels were given.
    NoChannels,
    /// Interleaved samples do not divide evenly into frames of the channel count.
    Misaligned,
    /// Planar channels do not all have the same number of frames.
    UnevenChannels,
}

impl std::fmt::Display for BufferListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BufferListError::NoChannels => write!(f, "a buffer needs at least one channel"),
            BufferListError::Misaligned => write!(f, "interleaved samples must fill whole frames"),
            BufferListError::UnevenChannels => write!(f, "every planar channel needs the same number of frames"),
        }
    }
}

/// One buffer of decoded or captured samples: `channels` interleaved channels in `data` (a planar buffer has one
/// channel and one buffer per channel).
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    pub channels: u32,
    pub data: Vec<f32>,
}

/// The buffers that make up one block of audio (`AudioBufferList`). A block is either one interleaved buffer or one
/// mono buffer per channel. Decoders, the speaker path and the capture path can share it, and it converts to the
/// interleaved 16-bit samples the speaker takes.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBufferList {
    buffers: Vec<AudioBuffer>,
}

impl AudioBufferList {
    /// One interleaved buffer of `channels` channels. The sample count must be a whole number of frames.
    pub fn interleaved(channels: u32, data: Vec<f32>) -> Result<AudioBufferList, BufferListError> {
        if channels == 0 {
            return Err(BufferListError::NoChannels);
        }
        if data.len() % channels as usize != 0 {
            return Err(BufferListError::Misaligned);
        }
        Ok(AudioBufferList { buffers: vec![AudioBuffer { channels, data }] })
    }

    /// One mono buffer per channel. Every channel must have the same number of frames.
    pub fn planar(planes: Vec<Vec<f32>>) -> Result<AudioBufferList, BufferListError> {
        let frames = match planes.first() {
            Some(first) => first.len(),
            None => return Err(BufferListError::NoChannels),
        };
        if planes.iter().any(|p| p.len() != frames) {
            return Err(BufferListError::UnevenChannels);
        }
        Ok(AudioBufferList { buffers: planes.into_iter().map(|data| AudioBuffer { channels: 1, data }).collect() })
    }

    /// The buffers, in order.
    pub fn buffers(&self) -> &[AudioBuffer] {
        &self.buffers
    }

    /// Channels in the block: the interleaved count, or the number of planar buffers.
    pub fn channel_count(&self) -> usize {
        match self.buffers.as_slice() {
            [only] => only.channels as usize,
            many => many.len(),
        }
    }

    /// Frames in the block: one frame is one sample on every channel.
    pub fn frames(&self) -> usize {
        match self.buffers.as_slice() {
            [only] => only.data.len() / only.channels.max(1) as usize,
            many => many.first().map_or(0, |b| b.data.len()),
        }
    }

    /// The samples in interleaved order (frame by frame, channel by channel).
    pub fn to_interleaved(&self) -> Vec<f32> {
        match self.buffers.as_slice() {
            [only] => only.data.clone(),
            many => {
                let mut out = Vec::with_capacity(self.frames() * many.len());
                for frame in 0..self.frames() {
                    for plane in many {
                        out.push(plane.data[frame]);
                    }
                }
                out
            }
        }
    }

    /// The interleaved samples as 16-bit PCM. Samples are clamped to plus or minus full scale first.
    pub fn to_pcm16(&self) -> Vec<i16> {
        self.to_interleaved().iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_is_16_bit_interleaved_pcm_with_a_two_byte_mono_frame() {
        let take = pcm16(8_000, 1);
        assert!(take.is_interleaved() && !take.is_float());
        assert_eq!(take.bits_per_channel, 16);
        assert_eq!(take.bytes_per_frame, 2);
        assert_eq!(take.channels, 1);
        assert_eq!(take.sample_rate, 8_000.0);
    }

    #[test]
    fn duration_counts_whole_frames_of_the_format() {
        // One second of 8 kHz mono is 16000 bytes; stereo 48 kHz frames are 4 bytes each.
        assert_eq!(duration_ms(&pcm16(8_000, 1), 16_000), Some(1_000));
        assert_eq!(duration_ms(&pcm16(48_000, 2), 4 * 48_000), Some(1_000));
        // Three bytes of stereo 16-bit is less than one frame, so it counts as no time at all.
        assert_eq!(duration_ms(&pcm16(48_000, 2), 3), Some(0));
    }

    #[test]
    fn duration_is_none_for_planar_or_unrated_formats() {
        // The float32 server format is non-interleaved, so its byte count is per channel buffer.
        assert_eq!(duration_ms(&StreamFormat::float32(48_000.0, 2), 4_800), None);
        assert_eq!(duration_ms(&pcm16(0, 1), 100), None);
    }

    #[test]
    fn planar_channels_interleave_frame_by_frame() {
        let list = AudioBufferList::planar(vec![vec![1.0, 2.0], vec![3.0, 4.0]]).unwrap();
        assert_eq!(list.channel_count(), 2);
        assert_eq!(list.frames(), 2);
        assert_eq!(list.to_interleaved(), vec![1.0, 3.0, 2.0, 4.0]);
    }

    #[test]
    fn an_interleaved_buffer_counts_frames_not_samples() {
        let list = AudioBufferList::interleaved(2, vec![0.0, 0.1, 0.2, 0.3]).unwrap();
        assert_eq!((list.channel_count(), list.frames()), (2, 2));
        assert_eq!(list.buffers().len(), 1);
        assert_eq!(list.to_interleaved(), vec![0.0, 0.1, 0.2, 0.3]);
    }

    #[test]
    fn malformed_lists_are_refused() {
        assert_eq!(AudioBufferList::interleaved(2, vec![0.0; 3]), Err(BufferListError::Misaligned));
        assert_eq!(AudioBufferList::interleaved(0, vec![]), Err(BufferListError::NoChannels));
        assert_eq!(AudioBufferList::planar(vec![]), Err(BufferListError::NoChannels));
        assert_eq!(AudioBufferList::planar(vec![vec![0.0; 2], vec![0.0; 3]]), Err(BufferListError::UnevenChannels));
        assert_eq!(BufferListError::Misaligned.to_string(), "interleaved samples must fill whole frames");
    }

    #[test]
    fn pcm16_clamps_to_full_scale_and_rounds() {
        let list = AudioBufferList::interleaved(1, vec![1.5, -1.5, 0.5]).unwrap();
        assert_eq!(list.to_pcm16(), vec![32767, -32767, 16384]);
    }

    #[test]
    fn an_empty_interleaved_block_has_no_frames() {
        let list = AudioBufferList::interleaved(2, vec![]).unwrap();
        assert_eq!(list.frames(), 0);
        assert!(list.to_pcm16().is_empty());
    }
}
