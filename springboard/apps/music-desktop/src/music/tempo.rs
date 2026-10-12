//! Tempo of the songs: the beats per minute of each song, from its audio. The analysis is the rhythm analysis of the
//! music understanding model (`bifrost_app::music_understanding::analyze_rhythm`). Music reads only the start of a
//! song (the first `MAX_SECONDS`), which is enough for a steady tempo, and runs it on a background job. A song with
//! no rhythmic onsets (silence, speech) or a file that cannot be decoded is 0 and is not tried again. MIDI files have
//! no sound to analyse and are always 0.

use bifrost_app::music_understanding::analyze_rhythm;
use springboard::media::audiofile;
use std::path::Path;

/// The most audio read from the start of a song for its tempo, in seconds.
pub const MAX_SECONDS: f64 = 60.0;

/// Mixes interleaved S16 frames down to mono floats in -1 to 1 (the mean of the channels of each frame). A channel
/// count of 0 gives no samples.
pub fn mix_to_mono(interleaved: &[i16], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32)
        .collect()
}

/// The tempo of a song file in beats per minute; 0 when it has none or cannot be read.
pub fn bpm_of(path: &Path) -> f64 {
    if audiofile::is_midi(path) {
        return 0.0;
    }
    let Ok(mut decoder) = audiofile::open(path) else {
        return 0.0;
    };
    let rate = decoder.rate();
    let channels = decoder.channels().max(1);
    let limit = (rate as f64 * MAX_SECONDS) as usize * channels;
    let mut pcm: Vec<i16> = Vec::new();
    let mut packet: Vec<i16> = Vec::new();
    while pcm.len() < limit {
        match decoder.next(&mut packet) {
            Ok(true) => pcm.extend_from_slice(&packet),
            _ => break,
        }
    }
    pcm.truncate(limit);
    let mono = mix_to_mono(&pcm, channels);
    analyze_rhythm(&mono, rate).beats_per_minute
}

/// The text shown for a tempo: the nearest whole beat per minute, or None when the tempo is unknown (0).
pub fn label(bpm: f64) -> Option<String> {
    if bpm > 0.0 {
        Some(format!("{:.0}", bpm))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_passes_through_and_stereo_is_averaged() {
        assert_eq!(mix_to_mono(&[8192], 1), vec![0.25]);
        assert_eq!(mix_to_mono(&[16384, 16384, -16384, -16384], 2), vec![0.5, -0.5]);
    }

    #[test]
    fn no_channels_gives_no_samples() {
        assert!(mix_to_mono(&[1, 2, 3], 0).is_empty());
    }

    #[test]
    fn a_trailing_partial_frame_is_dropped() {
        assert_eq!(mix_to_mono(&[16384, 16384, 8192], 2), vec![0.5]);
    }

    #[test]
    fn the_label_is_the_whole_bpm_and_unknown_is_none() {
        assert_eq!(label(119.6), Some("120".to_string()));
        assert_eq!(label(92.2), Some("92".to_string()));
        assert_eq!(label(0.0), None);
        assert_eq!(label(-3.0), None);
    }

    #[test]
    fn a_missing_file_has_no_tempo() {
        assert_eq!(bpm_of(Path::new("/nonexistent/bifrost-test-song.mp3")), 0.0);
    }
}
