//! Guest audio stream configuration: the PCM format, rate, channels and buffer geometry of one guest stream.
//!
//! Pure model. A guest sound device (the VIRTIO sound device, id 25 in [`crate::virtio`]) describes each stream with a
//! direction, a sample format, a rate, a channel count and a period and buffer size. [`StreamConfig::new`] refuses
//! values the device cannot carry and reports the latency a buffer adds. Nothing is played here: the host side hands an
//! accepted configuration to the audio daemon. The official Virtualization documentation lists "Audio" as a device
//! topic that lets the guest play and capture through the host's audio devices. The rates and sample sizes below
//! follow the VIRTIO sound specification; the buffer limit is BifrOSt's own choice.

/// BifrOSt limit on one stream's buffer. This is a project choice, not a documented framework limit.
pub const MAX_BUFFER_BYTES: u32 = 1024 * 1024;

/// Sample rates a VIRTIO sound stream may use, in Hz.
pub const RATES: [u32; 8] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000];

pub const MIN_CHANNELS: u8 = 1;
pub const MAX_CHANNELS: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Guest playback, host output.
    Output,
    /// Guest capture, host input.
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    S16,
    S24,
    S32,
    F32,
}

impl SampleFormat {
    /// Bytes per sample.
    pub fn width(self) -> u32 {
        match self {
            SampleFormat::S16 => 2,
            SampleFormat::S24 => 3,
            SampleFormat::S32 | SampleFormat::F32 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The rate is not one of [`RATES`].
    BadRate(u32),
    /// The channel count is outside [`MIN_CHANNELS`]..=[`MAX_CHANNELS`].
    BadChannels(u8),
    /// The buffer or period is not a whole number of frames.
    Misaligned { frame_bytes: u32 },
    /// A period is longer than the whole buffer.
    PeriodExceedsBuffer,
    /// The buffer is larger than [`MAX_BUFFER_BYTES`].
    BufferTooLarge(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    direction: Direction,
    format: SampleFormat,
    rate: u32,
    channels: u8,
    period_bytes: u32,
    buffer_bytes: u32,
}

impl StreamConfig {
    pub fn new(
        direction: Direction,
        format: SampleFormat,
        rate: u32,
        channels: u8,
        period_bytes: u32,
        buffer_bytes: u32,
    ) -> Result<Self, Error> {
        if !RATES.contains(&rate) {
            return Err(Error::BadRate(rate));
        }
        if !(MIN_CHANNELS..=MAX_CHANNELS).contains(&channels) {
            return Err(Error::BadChannels(channels));
        }
        let frame = format.width() * channels as u32;
        if period_bytes % frame != 0 || buffer_bytes % frame != 0 {
            return Err(Error::Misaligned { frame_bytes: frame });
        }
        if period_bytes > buffer_bytes {
            return Err(Error::PeriodExceedsBuffer);
        }
        if buffer_bytes > MAX_BUFFER_BYTES {
            return Err(Error::BufferTooLarge(buffer_bytes));
        }
        Ok(StreamConfig { direction, format, rate, channels, period_bytes, buffer_bytes })
    }

    pub fn direction(&self) -> Direction {
        self.direction
    }

    pub fn format(&self) -> SampleFormat {
        self.format
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Bytes in one frame: one sample per channel.
    pub fn frame_bytes(&self) -> u32 {
        self.format.width() * self.channels as u32
    }

    /// Data rate in bytes per second.
    pub fn bytes_per_second(&self) -> u32 {
        self.rate * self.frame_bytes()
    }

    /// Whole milliseconds of audio one full buffer holds (rounded down).
    pub fn buffer_latency_ms(&self) -> u32 {
        let frames = self.buffer_bytes / self.frame_bytes();
        (frames as u64 * 1000 / self.rate as u64) as u32
    }

    /// Periods per buffer, rounded down.
    pub fn periods_per_buffer(&self) -> u32 {
        self.buffer_bytes / self.period_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_s16_one_second_buffer_has_one_second_latency() {
        // 2 channels x 2 bytes = 4-byte frames; 48000 frames per second.
        let c = StreamConfig::new(Direction::Output, SampleFormat::S16, 48000, 2, 4800, 192000).unwrap();
        assert_eq!(c.frame_bytes(), 4);
        assert_eq!(c.bytes_per_second(), 192000);
        assert_eq!(c.buffer_latency_ms(), 1000);
        assert_eq!(c.periods_per_buffer(), 40);
    }

    #[test]
    fn unlisted_rate_and_bad_channel_counts_are_refused() {
        assert_eq!(
            StreamConfig::new(Direction::Input, SampleFormat::S16, 44101, 2, 4, 4),
            Err(Error::BadRate(44101))
        );
        assert_eq!(
            StreamConfig::new(Direction::Input, SampleFormat::S16, 44100, 0, 4, 4),
            Err(Error::BadChannels(0))
        );
        assert_eq!(
            StreamConfig::new(Direction::Input, SampleFormat::S16, 44100, 9, 4, 4),
            Err(Error::BadChannels(9))
        );
    }

    #[test]
    fn buffer_must_be_whole_frames() {
        // 24-bit mono: 3-byte frames, so 1000 bytes is not a whole number of frames.
        assert_eq!(
            StreamConfig::new(Direction::Output, SampleFormat::S24, 48000, 1, 300, 1000),
            Err(Error::Misaligned { frame_bytes: 3 })
        );
    }

    #[test]
    fn period_longer_than_buffer_is_refused() {
        assert_eq!(
            StreamConfig::new(Direction::Output, SampleFormat::F32, 48000, 1, 8, 4),
            Err(Error::PeriodExceedsBuffer)
        );
    }

    #[test]
    fn oversized_buffer_is_refused() {
        let big = MAX_BUFFER_BYTES + 4;
        assert_eq!(
            StreamConfig::new(Direction::Output, SampleFormat::S32, 48000, 1, 4, big),
            Err(Error::BufferTooLarge(big))
        );
    }

    #[test]
    fn mono_float_at_16k_latency_is_exact() {
        // 4-byte frames at 16000 Hz; 8000 frames is half a second.
        let c = StreamConfig::new(Direction::Input, SampleFormat::F32, 16000, 1, 3200, 32000).unwrap();
        assert_eq!(c.buffer_latency_ms(), 500);
        assert_eq!(c.direction(), Direction::Input);
        assert_eq!(c.format().width(), 4);
    }
}
