//! Linear PCM conversion: sample formats (signed 16-bit, 8-bit unsigned and 32-bit float), channel layouts, and a
//! stateful linear sample-rate converter. Samples are interleaved. Floats run from -1.0 to 1.0, so 16-bit values
//! map to the range -32767 to 32767 and 8-bit values to 0 to 255 with 128 as silence. These are pure functions and
//! a pure model: no device is opened, and the caller owns the buffers.

/// Why a conversion was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// A sample rate is zero.
    ZeroRate,
    /// A channel count is zero.
    ZeroChannels,
    /// The sample count is not a whole number of frames.
    Ragged,
}

/// A 16-bit sample as a float in -1.0 to 1.0. The value -32768 maps to exactly -1.0.
pub fn s16_to_f32(s: i16) -> f32 {
    s as f32 / 32768.0
}

/// A float as a 16-bit sample. The float is clamped to -1.0 to 1.0 first, and NaN becomes silence.
pub fn f32_to_s16(f: f32) -> i16 {
    if f.is_nan() {
        return 0;
    }
    (f.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// An 8-bit unsigned sample as a float in -1.0 to 1.0. 128 is silence.
pub fn u8_to_f32(u: u8) -> f32 {
    (u as f32 - 128.0) / 128.0
}

/// A float as an 8-bit unsigned sample. The float is clamped to -1.0 to 1.0 first, and NaN becomes silence.
pub fn f32_to_u8(f: f32) -> u8 {
    if f.is_nan() {
        return 128;
    }
    (f.clamp(-1.0, 1.0) * 128.0 + 128.0).round().min(255.0).max(0.0) as u8
}

/// A whole buffer of 16-bit samples as floats.
pub fn s16_buffer_to_f32(src: &[i16]) -> Vec<f32> {
    src.iter().map(|&s| s16_to_f32(s)).collect()
}

/// A whole buffer of floats as 16-bit samples.
pub fn f32_buffer_to_s16(src: &[f32]) -> Vec<i16> {
    src.iter().map(|&f| f32_to_s16(f)).collect()
}

/// Changes the channel count of interleaved samples. Mono to several channels copies the sample to each channel.
/// Several channels to mono averages each frame. Between two multi-channel layouts, the first `to` channels are
/// kept, and any extra output channels repeat the last input channel. Nothing is mixed in from the dropped channels.
pub fn remap_channels(input: &[f32], from: u16, to: u16) -> Result<Vec<f32>, ConvertError> {
    if from == 0 || to == 0 {
        return Err(ConvertError::ZeroChannels);
    }
    let (from, to) = (from as usize, to as usize);
    if input.len() % from != 0 {
        return Err(ConvertError::Ragged);
    }
    let mut out = Vec::with_capacity(input.len() / from * to);
    for frame in input.chunks_exact(from) {
        if from == to {
            out.extend_from_slice(frame);
        } else if to == 1 {
            out.push(frame.iter().sum::<f32>() / from as f32);
        } else if from == 1 {
            out.extend(std::iter::repeat(frame[0]).take(to));
        } else {
            out.extend((0..to).map(|c| frame[c.min(from - 1)]));
        }
    }
    Ok(out)
}

/// A stateful linear sample-rate converter for interleaved floats. The output position carries over between calls,
/// so a stream split into blocks converts the same way as one block. The converter holds back the newest input
/// frame until the next frame arrives, because interpolation needs both neighbours.
#[derive(Clone, Debug)]
pub struct Resampler {
    step: f64,
    channels: usize,
    /// Output position in input frames, relative to the first frame of the next block. Negative values refer to
    /// the last frame of the previous block, held in `prev`.
    pos: f64,
    prev: Vec<f32>,
}

impl Resampler {
    /// A converter from `from_rate` to `to_rate` for `channels` interleaved channels.
    pub fn new(from_rate: u32, to_rate: u32, channels: u16) -> Result<Self, ConvertError> {
        if from_rate == 0 || to_rate == 0 {
            return Err(ConvertError::ZeroRate);
        }
        if channels == 0 {
            return Err(ConvertError::ZeroChannels);
        }
        let channels = channels as usize;
        Ok(Resampler {
            step: from_rate as f64 / to_rate as f64,
            channels,
            pos: 0.0,
            prev: vec![0.0; channels],
        })
    }

    /// Converts one block of interleaved input. The result holds the output frames that are now ready.
    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, ConvertError> {
        let ch = self.channels;
        if input.len() % ch != 0 {
            return Err(ConvertError::Ragged);
        }
        let n = input.len() / ch;
        if n == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut pos = self.pos;
        loop {
            let i0 = pos.floor();
            let i0i = i0 as i64;
            // Both neighbours must be known: frame i0 + 1 has to be inside this block.
            if i0i + 1 >= n as i64 {
                break;
            }
            let frac = (pos - i0) as f32;
            for c in 0..ch {
                let a = frame_sample(&self.prev, input, ch, i0i, c);
                let b = frame_sample(&self.prev, input, ch, i0i + 1, c);
                out.push(a + (b - a) * frac);
            }
            pos += self.step;
        }
        self.pos = pos - n as f64;
        self.prev.copy_from_slice(&input[(n - 1) * ch..]);
        Ok(out)
    }

    /// Forgets the position and the held frame. The next block starts a new stream.
    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.prev.iter_mut().for_each(|s| *s = 0.0);
    }
}

/// The storage depth of one linear PCM sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    /// 8-bit unsigned, 128 is silence.
    U8,
    /// 16-bit signed, little-endian.
    S16,
    /// 32-bit float, little-endian, from -1.0 to 1.0.
    F32,
}

impl Depth {
    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        match self {
            Depth::U8 => 1,
            Depth::S16 => 2,
            Depth::F32 => 4,
        }
    }
}

/// A linear PCM stream format: the rate in frames per second, the interleaved channel count and the sample depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub depth: Depth,
}

impl PcmFormat {
    /// Bytes in one frame (one sample per channel).
    pub fn frame_bytes(self) -> usize {
        self.depth.bytes() * self.channels as usize
    }
}

/// Decodes little-endian interleaved bytes of `depth` into floats from -1.0 to 1.0. A byte count that is not a whole
/// number of samples is refused.
pub fn decode(bytes: &[u8], depth: Depth) -> Result<Vec<f32>, ConvertError> {
    let width = depth.bytes();
    if bytes.len() % width != 0 {
        return Err(ConvertError::Ragged);
    }
    Ok(bytes
        .chunks_exact(width)
        .map(|c| match depth {
            Depth::U8 => u8_to_f32(c[0]),
            Depth::S16 => s16_to_f32(i16::from_le_bytes([c[0], c[1]])),
            Depth::F32 => f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
        })
        .collect())
}

/// Encodes floats as little-endian interleaved bytes of `depth`. Integer depths clamp the floats to range first.
pub fn encode(samples: &[f32], depth: Depth) -> Vec<u8> {
    match depth {
        Depth::U8 => samples.iter().map(|&f| f32_to_u8(f)).collect(),
        Depth::S16 => samples.iter().flat_map(|&f| f32_to_s16(f).to_le_bytes()).collect(),
        Depth::F32 => samples.iter().flat_map(|f| f.to_le_bytes()).collect(),
    }
}

/// Converts a block of interleaved bytes from one PCM format to another: channel layout first, then sample rate, then
/// the target depth. The rate conversion is one-shot, so the output holds ceil(frames * to_rate / from_rate) frames:
/// the newest input frame is repeated once so the converter can emit its final output frames.
pub fn convert_bytes(input: &[u8], from: PcmFormat, to: PcmFormat) -> Result<Vec<u8>, ConvertError> {
    if from.sample_rate == 0 || to.sample_rate == 0 {
        return Err(ConvertError::ZeroRate);
    }
    if from.channels == 0 || to.channels == 0 {
        return Err(ConvertError::ZeroChannels);
    }
    let samples = decode(input, from.depth)?;
    let mut mixed = remap_channels(&samples, from.channels, to.channels)?;
    let channels = to.channels as usize;
    if mixed.is_empty() {
        return Ok(Vec::new());
    }
    let last_frame = mixed.len() - channels;
    mixed.extend_from_within(last_frame..);
    let mut resampler = Resampler::new(from.sample_rate, to.sample_rate, to.channels)?;
    let resampled = resampler.process(&mixed)?;
    Ok(encode(&resampled, to.depth))
}

/// One sample of frame `idx`, where -1 is the held frame of the previous block.
fn frame_sample(prev: &[f32], input: &[f32], ch: usize, idx: i64, c: usize) -> f32 {
    if idx < 0 {
        prev[c]
    } else {
        input[idx as usize * ch + c]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit_samples_map_to_unit_floats_and_back() {
        assert_eq!(s16_to_f32(16384), 0.5);
        assert_eq!(s16_to_f32(-32768), -1.0);
        assert_eq!(f32_to_s16(0.25), 8192);
        assert_eq!(f32_to_s16(2.0), 32767);
        assert_eq!(f32_to_s16(-3.0), -32767);
        assert_eq!(f32_to_s16(f32::NAN), 0);
        // 0.5 times 32767 is 16383.5, which rounds away from zero to 16384.
        assert_eq!(f32_to_s16(s16_to_f32(16384)), 16384);
    }

    #[test]
    fn eight_bit_samples_centre_on_128() {
        assert_eq!(u8_to_f32(128), 0.0);
        assert_eq!(u8_to_f32(0), -1.0);
        assert_eq!(f32_to_u8(0.0), 128);
        assert_eq!(f32_to_u8(0.5), 192);
        assert_eq!(f32_to_u8(1.0), 255);
        assert_eq!(f32_to_u8(-1.0), 0);
        assert_eq!(f32_to_u8(9.0), 255);
    }

    #[test]
    fn mono_to_stereo_copies_and_stereo_to_mono_averages() {
        assert_eq!(remap_channels(&[0.2, -0.4], 1, 2).unwrap(), vec![0.2, 0.2, -0.4, -0.4]);
        assert_eq!(remap_channels(&[1.0, 0.5, -1.0, 1.0], 2, 1).unwrap(), vec![0.75, 0.0]);
    }

    #[test]
    fn multichannel_keeps_the_first_channels_and_repeats_the_last() {
        assert_eq!(remap_channels(&[1.0, 2.0, 3.0, 4.0], 4, 2).unwrap(), vec![1.0, 2.0]);
        assert_eq!(remap_channels(&[1.0, 2.0], 2, 4).unwrap(), vec![1.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn channel_remap_refuses_zero_and_ragged_input() {
        assert_eq!(remap_channels(&[1.0], 0, 2), Err(ConvertError::ZeroChannels));
        assert_eq!(remap_channels(&[1.0, 2.0, 3.0], 2, 1), Err(ConvertError::Ragged));
    }

    #[test]
    fn buffer_helpers_convert_each_sample() {
        assert_eq!(s16_buffer_to_f32(&[0, 16384, -32768]), vec![0.0, 0.5, -1.0]);
        assert_eq!(f32_buffer_to_s16(&[0.0, 0.5, -1.0]), vec![0, 16384, -32767]);
    }

    #[test]
    fn resampler_refuses_zero_rates_and_channels() {
        assert_eq!(Resampler::new(0, 48000, 1).err(), Some(ConvertError::ZeroRate));
        assert_eq!(Resampler::new(48000, 0, 1).err(), Some(ConvertError::ZeroRate));
        assert_eq!(Resampler::new(48000, 48000, 0).err(), Some(ConvertError::ZeroChannels));
        let mut r = Resampler::new(48000, 48000, 2).unwrap();
        assert_eq!(r.process(&[1.0, 2.0, 3.0]), Err(ConvertError::Ragged));
    }

    #[test]
    fn same_rate_passes_through_across_blocks() {
        // The first block holds back its newest frame, so it gives frames 0 and 1. The next block gives frames 2 and 3.
        let mut r = Resampler::new(8000, 8000, 1).unwrap();
        let mut out = r.process(&[0.1, 0.2, 0.3]).unwrap();
        out.extend(r.process(&[0.4, 0.5]).unwrap());
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn upsampling_by_two_interpolates_midpoints() {
        // From 2 Hz to 4 Hz: the step is 0.5 input frames. Midpoints are averages of neighbours.
        let mut r = Resampler::new(2, 4, 1).unwrap();
        let out = r.process(&[0.0, 1.0, 2.0, 3.0]).unwrap();
        assert_eq!(out, vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
        // Frame 3 was held back. It is output first, at position -1 of the next block, then the midpoint to frame 4.
        let next = r.process(&[4.0]).unwrap();
        assert_eq!(next, vec![3.0, 3.5]);
    }

    #[test]
    fn downsampling_by_two_keeps_every_second_frame() {
        let mut r = Resampler::new(4, 2, 1).unwrap();
        let out = r.process(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(out, vec![0.0, 2.0, 4.0]);
    }

    #[test]
    fn stereo_channels_are_interpolated_separately() {
        // From 2 Hz to 4 Hz, stereo. Left rises from 0 to 2 and right falls from 0 to -2. Output positions 0, 0.5, 1
        // and 1.5 are all inside the block; position 2 needs a frame that has not arrived.
        let mut r = Resampler::new(2, 4, 2).unwrap();
        let out = r.process(&[0.0, 0.0, 1.0, -1.0, 2.0, -2.0]).unwrap();
        assert_eq!(out, vec![0.0, 0.0, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5]);
    }

    #[test]
    fn reset_starts_a_new_stream() {
        // After a reset the held frame is silence and the position is zero. The block of ones gives two outputs
        // (positions 0 and 0.5), and the frame at position 1 waits for a frame that has not arrived.
        let mut r = Resampler::new(2, 4, 1).unwrap();
        r.process(&[5.0, 5.0, 5.0]).unwrap();
        r.reset();
        let out = r.process(&[1.0, 1.0]).unwrap();
        assert_eq!(out, vec![1.0, 1.0]);
    }
}
