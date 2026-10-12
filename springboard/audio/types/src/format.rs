//! Stream formats (the reference platform's stream basic description). Owner: task 01.

/// The sample encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatId {
    /// Linear PCM; the flags say float or integer, endianness, packing and interleaving.
    LinearPcm,
}

/// Linear PCM flags (combine with `|`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FormatFlags(pub u32);

impl FormatFlags {
    pub const FLOAT: FormatFlags = FormatFlags(1);
    pub const BIG_ENDIAN: FormatFlags = FormatFlags(2);
    pub const SIGNED_INTEGER: FormatFlags = FormatFlags(4);
    pub const PACKED: FormatFlags = FormatFlags(8);
    /// High-aligned samples in a wider container (24 bits in 32).
    pub const ALIGNED_HIGH: FormatFlags = FormatFlags(16);
    /// One buffer per channel.
    pub const NON_INTERLEAVED: FormatFlags = FormatFlags(32);

    pub const fn contains(self, other: FormatFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for FormatFlags {
    type Output = FormatFlags;
    fn bitor(self, rhs: FormatFlags) -> FormatFlags {
        FormatFlags(self.0 | rhs.0)
    }
}

/// A stream's format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamFormat {
    pub sample_rate: f64,
    pub format: FormatId,
    pub flags: FormatFlags,
    /// Bytes per packet and frames per packet (1 for PCM); bytes per frame is per buffer (per channel when
    /// non-interleaved).
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels: u32,
    pub bits_per_channel: u32,
}

impl StreamFormat {
    /// The server's internal (virtual) format: f32, native endian, packed, non-interleaved.
    pub fn float32(sample_rate: f64, channels: u32) -> StreamFormat {
        let _ = (sample_rate, channels);
        todo!("audio-01")
    }

    /// Interleaved signed integer PCM of `bits` in a `container`-byte sample (S16 = 16/2, S24 in 32 = 24/4,
    /// S32 = 32/4), little endian.
    pub fn integer(sample_rate: f64, channels: u32, bits: u32, container: u32) -> StreamFormat {
        let _ = (sample_rate, channels, bits, container);
        todo!("audio-01")
    }

    pub fn is_float(&self) -> bool {
        self.flags.contains(FormatFlags::FLOAT)
    }

    pub fn is_interleaved(&self) -> bool {
        !self.flags.contains(FormatFlags::NON_INTERLEAVED)
    }

    /// Frames in `seconds` at this rate (rounded to the nearest frame).
    pub fn frames_for(&self, seconds: f64) -> u64 {
        let _ = seconds;
        todo!("audio-01")
    }

    /// Bytes of `frames` frames across all channels.
    pub fn bytes_for(&self, frames: u32) -> u64 {
        let _ = frames;
        todo!("audio-01")
    }
}
