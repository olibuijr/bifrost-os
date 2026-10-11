//! `MusicUnderstanding` for apps (see `docs/parity/music-understanding.md`): run analyses over audio the app already
//! holds and get time-stamped results. The model and the analyses live in `bifrost_ui::music`; decoding files is the
//! app's job (`springboard::media` decodes FLAC, Ogg Vorbis, WAV and MP3 for the DAW).

pub use bifrost_ui::music::{
    analyze_rhythm, AnalysisType, MusicUnderstandingError, MusicUnderstandingSession, RangedValue, RhythmResult,
    SessionResult, TimedValue,
};

/// One call for interleaved PCM: mix down to mono, then run every supported analysis.
pub fn analyze_interleaved(samples: &[f32], channels: usize, sample_rate: u32) -> Result<SessionResult, MusicUnderstandingError> {
    MusicUnderstandingSession::from_interleaved(samples, channels, sample_rate)?.analyze()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interleaved stereo with a 10 ms burst every beat at `bpm` on both channels.
    fn stereo_clicks(bpm: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
        let frames = (seconds * sample_rate as f64) as usize;
        let mut mono = vec![0.0f32; frames];
        let period = 60.0 / bpm * sample_rate as f64;
        let mut at = 0.0f64;
        while (at as usize) < frames {
            let start = at as usize;
            for k in 0..(sample_rate as usize / 100).min(frames - start) {
                mono[start + k] += 0.9 * (-(k as f32) / 30.0).exp();
            }
            at += period;
        }
        mono.iter().flat_map(|s| [*s, *s]).collect()
    }

    #[test]
    fn interleaved_stereo_clicks_give_the_tempo() {
        let result = analyze_interleaved(&stereo_clicks(120.0, 10.0, 44100), 2, 44100).unwrap();
        let rhythm = result.rhythm.expect("rhythm result");
        assert!((rhythm.beats_per_minute - 120.0).abs() < 0.5, "bpm {}", rhythm.beats_per_minute);
        assert!(rhythm.bars.len() >= 4);
    }

    #[test]
    fn silence_gives_an_empty_rhythm_result() {
        let result = analyze_interleaved(&vec![0.0f32; 44100 * 2 * 3], 2, 44100).unwrap();
        let rhythm = result.rhythm.expect("rhythm result");
        assert!(rhythm.beats.is_empty());
        assert_eq!(rhythm.beats_per_minute, 0.0);
    }

    #[test]
    fn zero_channels_is_an_invalid_asset() {
        assert_eq!(analyze_interleaved(&[0.0], 0, 44100).unwrap_err(), MusicUnderstandingError::InvalidAsset);
    }

    #[test]
    fn empty_analysis_set_is_refused() {
        let session = MusicUnderstandingSession::new(vec![0.0f32; 100], 44100).unwrap();
        assert_eq!(session.analyze_for(&[]).unwrap_err(), MusicUnderstandingError::EmptyAnalysisSet);
    }

    #[test]
    fn loudness_is_named_as_unsupported_until_it_exists() {
        let session = MusicUnderstandingSession::new(vec![0.0f32; 100], 44100).unwrap();
        assert_eq!(
            session.analyze_for(&[AnalysisType::Loudness]).unwrap_err(),
            MusicUnderstandingError::Unsupported(AnalysisType::Loudness)
        );
    }
}
