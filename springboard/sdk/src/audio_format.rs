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
}
