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
        StreamFormat {
            sample_rate,
            format: FormatId::LinearPcm,
            flags: FormatFlags::FLOAT | FormatFlags::PACKED | FormatFlags::NON_INTERLEAVED,
            bytes_per_packet: 4,
            frames_per_packet: 1,
            bytes_per_frame: 4,
            channels,
            bits_per_channel: 32,
        }
    }

    /// Interleaved signed integer PCM of `bits` in a `container`-byte sample (S16 = 16/2, S24 in 32 = 24/4,
    /// S32 = 32/4), little endian.
    pub fn integer(sample_rate: f64, channels: u32, bits: u32, container: u32) -> StreamFormat {
        let mut flags = FormatFlags::SIGNED_INTEGER;
        if bits == container * 8 {
            flags = flags | FormatFlags::PACKED;
        } else {
            flags = flags | FormatFlags::ALIGNED_HIGH;
        }
        let frame = container * channels;
        StreamFormat {
            sample_rate,
            format: FormatId::LinearPcm,
            flags,
            bytes_per_packet: frame,
            frames_per_packet: 1,
            bytes_per_frame: frame,
            channels,
            bits_per_channel: bits,
        }
    }

    pub fn is_float(&self) -> bool {
        self.flags.contains(FormatFlags::FLOAT)
    }

    pub fn is_interleaved(&self) -> bool {
        !self.flags.contains(FormatFlags::NON_INTERLEAVED)
    }

    /// Frames in `seconds` at this rate (rounded to the nearest frame).
    pub fn frames_for(&self, seconds: f64) -> u64 {
        let f = seconds * self.sample_rate;
        if f <= 0.0 || f != f {
            0
        } else {
            (f + 0.5) as u64
        }
    }

    /// Bytes of `frames` frames across all channels.
    pub fn bytes_for(&self, frames: u32) -> u64 {
        let per_frame = if self.is_interleaved() {
            self.bytes_per_frame as u64
        } else {
            self.bytes_per_frame as u64 * self.channels as u64
        };
        per_frame * frames as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float32_is_packed_non_interleaved() {
        let f = StreamFormat::float32(48000.0, 2);
        assert!(f.is_float());
        assert!(!f.is_interleaved());
        assert_eq!(f.bytes_per_frame, 4);
        assert_eq!(f.bits_per_channel, 32);
        assert_eq!(f.bytes_for(10), 80); // 10 frames * 2 channels * 4 bytes
    }

    #[test]
    fn integer_formats() {
        let s16 = StreamFormat::integer(44100.0, 2, 16, 2);
        assert_eq!(s16.bytes_per_frame, 4);
        assert!(s16.flags.contains(FormatFlags::PACKED));
        assert!(s16.is_interleaved() && !s16.is_float());
        assert_eq!(s16.bytes_for(100), 400);
        let s24 = StreamFormat::integer(96000.0, 6, 24, 4);
        assert_eq!(s24.bytes_per_frame, 24);
        assert!(s24.flags.contains(FormatFlags::ALIGNED_HIGH));
        assert!(!s24.flags.contains(FormatFlags::PACKED));
        assert_eq!(s24.bytes_for(1), 24);
    }

    #[test]
    fn frames_for_duration() {
        let f = StreamFormat::float32(48000.0, 2);
        assert_eq!(f.frames_for(1.0), 48000);
        assert_eq!(f.frames_for(0.0105), 504);
        assert_eq!(f.frames_for(-1.0), 0);
        assert_eq!(StreamFormat::float32(44100.0, 1).frames_for(0.5), 22050);
    }
}
