//! Sample conversion between the server's f32 and the device's physical formats (S32_LE, S24_LE in a 32-bit
//! container, S16_LE), on strided byte areas so one routine serves interleaved and non-interleaved mmap layouts.
//! No allocation.

use crate::uapi::{FORMAT_S16_LE, FORMAT_S24_LE, FORMAT_S32_LE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    S32,
    S24,
    S16,
}

impl SampleFormat {
    /// The kernel's `snd_pcm_format_t` value.
    pub fn alsa(self) -> u32 {
        match self {
            SampleFormat::S32 => FORMAT_S32_LE,
            SampleFormat::S24 => FORMAT_S24_LE,
            SampleFormat::S16 => FORMAT_S16_LE,
        }
    }

    pub fn from_alsa(v: u32) -> Option<SampleFormat> {
        match v {
            FORMAT_S32_LE => Some(SampleFormat::S32),
            FORMAT_S24_LE => Some(SampleFormat::S24),
            FORMAT_S16_LE => Some(SampleFormat::S16),
            _ => None,
        }
    }

    /// Bytes per sample in memory.
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::S16 => 2,
            _ => 4,
        }
    }

    /// Significant bits per sample.
    pub fn bits(self) -> u32 {
        match self {
            SampleFormat::S32 => 32,
            SampleFormat::S24 => 24,
            SampleFormat::S16 => 16,
        }
    }

    /// Preference order of the driver: the widest first.
    pub const PREFERENCE: [SampleFormat; 3] = [SampleFormat::S32, SampleFormat::S24, SampleFormat::S16];
}

/// f32 in [-1, 1] to a signed integer of `scale` full-scale steps, saturating (NaN becomes 0).
fn quantize(x: f32, scale: f64, lo: f64, hi: f64) -> i32 {
    ((x as f64) * scale).clamp(lo, hi) as i32
}

pub fn f32_to_s32(x: f32) -> i32 {
    quantize(x, 2_147_483_648.0, -2_147_483_648.0, 2_147_483_647.0)
}
pub fn f32_to_s24(x: f32) -> i32 {
    quantize(x, 8_388_608.0, -8_388_608.0, 8_388_607.0)
}
pub fn f32_to_s16(x: f32) -> i16 {
    quantize(x, 32_768.0, -32_768.0, 32_767.0) as i16
}
pub fn s32_to_f32(v: i32) -> f32 {
    (v as f64 / 2_147_483_648.0) as f32
}
/// S24_LE keeps the sample in the low three bytes; the top byte is ignored (sign-extended from bit 23).
pub fn s24_to_f32(v: i32) -> f32 {
    (((v << 8) >> 8) as f64 / 8_388_608.0) as f32
}
pub fn s16_to_f32(v: i16) -> f32 {
    (v as f64 / 32_768.0) as f32
}

/// Writes `src` (one channel) into `dst` starting at byte `offset` with `stride` bytes between samples.
pub fn pack_channel(fmt: SampleFormat, dst: &mut [u8], offset: usize, stride: usize, src: &[f32]) {
    let w = fmt.bytes();
    for (i, &x) in src.iter().enumerate() {
        let at = offset + i * stride;
        let Some(slot) = dst.get_mut(at..at + w) else { return };
        match fmt {
            SampleFormat::S32 => slot.copy_from_slice(&f32_to_s32(x).to_le_bytes()),
            SampleFormat::S24 => slot.copy_from_slice(&(f32_to_s24(x) & 0x00ff_ffff).to_le_bytes()),
            SampleFormat::S16 => slot.copy_from_slice(&f32_to_s16(x).to_le_bytes()),
        }
    }
}

/// Reads samples of one channel from `src` into `dst`.
pub fn unpack_channel(fmt: SampleFormat, src: &[u8], offset: usize, stride: usize, dst: &mut [f32]) {
    let w = fmt.bytes();
    for (i, out) in dst.iter_mut().enumerate() {
        let at = offset + i * stride;
        let Some(slot) = src.get(at..at + w) else { return };
        *out = match fmt {
            SampleFormat::S32 => s32_to_f32(i32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]])),
            SampleFormat::S24 => s24_to_f32(i32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]])),
            SampleFormat::S16 => s16_to_f32(i16::from_le_bytes([slot[0], slot[1]])),
        };
    }
}

/// Zeroes `frames` frames of silence at frame `at` of a ring of `ring_frames` frames, all channels (wraps).
pub fn silence_ring(dst: &mut [u8], frame_bytes_total: usize, ring_frames: usize, at: usize, frames: usize, interleaved: bool, channels: usize, sample_bytes: usize) {
    for f in 0..frames {
        let idx = (at + f) % ring_frames;
        if interleaved {
            let o = idx * frame_bytes_total;
            if let Some(s) = dst.get_mut(o..o + frame_bytes_total) {
                s.fill(0);
            }
        } else {
            for c in 0..channels {
                let o = (c * ring_frames + idx) * sample_bytes;
                if let Some(s) = dst.get_mut(o..o + sample_bytes) {
                    s.fill(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_to_integers_at_known_points() {
        assert_eq!(f32_to_s32(0.5), 1_073_741_824);
        assert_eq!(f32_to_s32(-0.5), -1_073_741_824);
        assert_eq!(f32_to_s32(1.0), i32::MAX, "full scale saturates");
        assert_eq!(f32_to_s32(-1.0), i32::MIN);
        assert_eq!(f32_to_s32(2.0), i32::MAX);
        assert_eq!(f32_to_s32(f32::NAN), 0);
        assert_eq!(f32_to_s24(0.5), 4_194_304);
        assert_eq!(f32_to_s24(1.0), 8_388_607);
        assert_eq!(f32_to_s24(-1.0), -8_388_608);
        assert_eq!(f32_to_s16(0.5), 16_384);
        assert_eq!(f32_to_s16(1.0), 32_767);
        assert_eq!(f32_to_s16(-1.0), -32_768);
        assert_eq!(f32_to_s16(0.0), 0);
    }

    #[test]
    fn integers_to_f32() {
        assert_eq!(s32_to_f32(i32::MIN), -1.0);
        assert_eq!(s32_to_f32(1 << 30), 0.5);
        assert_eq!(s24_to_f32(4_194_304), 0.5);
        assert_eq!(s24_to_f32(0x00ff_ffff), -1.0 / 8_388_608.0, "bit 23 set is negative; the top byte is ignored");
        assert_eq!(s24_to_f32(0x7f80_0000u32 as i32), -1.0, "garbage in the top byte is ignored");
        assert_eq!(s16_to_f32(-32_768), -1.0);
        assert_eq!(s16_to_f32(16_384), 0.5);
    }

    #[test]
    fn exactly_representable_samples_round_trip() {
        for fmt in SampleFormat::PREFERENCE {
            let src = [0.0f32, 0.5, -0.5, 0.25, -1.0];
            let mut bytes = vec![0u8; 5 * fmt.bytes()];
            pack_channel(fmt, &mut bytes, 0, fmt.bytes(), &src);
            let mut back = [9.0f32; 5];
            unpack_channel(fmt, &bytes, 0, fmt.bytes(), &mut back);
            assert_eq!(src, back, "{fmt:?}");
        }
    }

    #[test]
    fn interleaved_stride_places_channels_side_by_side() {
        // two channels of S16, 3 frames: L = [1, 2, 3] / 32768, R = [-1, -2, -3] / 32768
        let l = [1.0f32 / 32768.0, 2.0 / 32768.0, 3.0 / 32768.0];
        let r = [-1.0f32 / 32768.0, -2.0 / 32768.0, -3.0 / 32768.0];
        let mut bytes = vec![0u8; 12];
        pack_channel(SampleFormat::S16, &mut bytes, 0, 4, &l);
        pack_channel(SampleFormat::S16, &mut bytes, 2, 4, &r);
        assert_eq!(bytes, [1, 0, 0xff, 0xff, 2, 0, 0xfe, 0xff, 3, 0, 0xfd, 0xff]);
    }

    #[test]
    fn s24_is_stored_in_the_low_three_bytes() {
        let mut bytes = vec![0xaau8; 4];
        pack_channel(SampleFormat::S24, &mut bytes, 0, 4, &[-0.5]);
        // -4194304 = 0xffc00000; masked to 24 bits = 0xc00000, little endian 00 00 c0 00
        assert_eq!(bytes, [0x00, 0x00, 0xc0, 0x00]);
        let mut back = [0.0f32];
        unpack_channel(SampleFormat::S24, &bytes, 0, 4, &mut back);
        assert_eq!(back[0], -0.5);
    }

    #[test]
    fn non_interleaved_uses_per_channel_areas() {
        // ring of 4 frames, 2 channels, S16: channel 1 starts at byte 8
        let mut bytes = vec![0u8; 16];
        pack_channel(SampleFormat::S16, &mut bytes, 8, 2, &[0.5, 0.5]);
        assert_eq!(&bytes[..8], &[0; 8]);
        assert_eq!(&bytes[8..12], &[0, 0x40, 0, 0x40]);
    }

    #[test]
    fn silence_wraps_around_the_ring() {
        let mut bytes = vec![0xffu8; 16];
        // interleaved 2 ch S16 (4 bytes a frame), ring of 4 frames: frames 3 and 0 are cleared
        silence_ring(&mut bytes, 4, 4, 3, 2, true, 2, 2);
        assert_eq!(&bytes[..4], &[0; 4]);
        assert_eq!(&bytes[4..12], &[0xff; 8]);
        assert_eq!(&bytes[12..], &[0; 4]);
        let mut bytes = vec![0xffu8; 16];
        // non-interleaved: frame 3 of channel 0 (bytes 6..8) and of channel 1 (bytes 14..16), frame 0 likewise
        silence_ring(&mut bytes, 4, 4, 3, 2, false, 2, 2);
        assert_eq!(&bytes[0..2], &[0; 2]);
        assert_eq!(&bytes[6..8], &[0; 2]);
        assert_eq!(&bytes[8..10], &[0; 2]);
        assert_eq!(&bytes[14..16], &[0; 2]);
        assert_eq!(&bytes[2..6], &[0xff; 4]);
    }

    #[test]
    fn alsa_format_numbers() {
        assert_eq!(SampleFormat::S32.alsa(), 10);
        assert_eq!(SampleFormat::from_alsa(6), Some(SampleFormat::S24));
        assert_eq!(SampleFormat::from_alsa(3), None);
    }
}
