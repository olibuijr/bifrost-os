//! Music understanding: a session runs analyses over a mono audio buffer and returns time-stamped results
//! (`MusicUnderstandingSession`, `SessionResult`, `RhythmResult`). Implemented: rhythm (beats, bars, BPM) from an
//! onset-energy envelope. Pace is available from a beat grid through `analyze_pace`; it is not part of `analyze`
//! yet. Key, loudness, structure and instrument activity are not implemented; a `SessionResult` never carries them,
//! and asking for them fails with `Unsupported`. Times are seconds from the start of the buffer. Pure Rust, no
//! decoding: the caller hands over samples.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// `MusicUnderstandingSession.AnalysisType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnalysisType {
    Key,
    InstrumentActivity,
    Loudness,
    Pace,
    Rhythm,
    Structure,
}

impl AnalysisType {
    /// The analyses this module runs. `analyze()` performs exactly these.
    pub const SUPPORTED: [AnalysisType; 1] = [AnalysisType::Rhythm];
}

/// `MusicUnderstandingError`: the cases this module can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicUnderstandingError {
    /// `emptyAnalysisSet`: `analyze(for:)` was given no analysis types.
    EmptyAnalysisSet,
    /// `invalidAsset`: no samples, no channels or a zero sample rate.
    InvalidAsset,
    /// The analysis is named by the official API but not implemented here yet.
    Unsupported(AnalysisType),
    /// `cancel()` was called while the analysis was running.
    Cancelled,
}

impl std::fmt::Display for MusicUnderstandingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyAnalysisSet => f.write_str("the analysis set is empty"),
            Self::InvalidAsset => f.write_str("the audio input is empty or has no channels or sample rate"),
            Self::Unsupported(kind) => write!(f, "{kind:?} analysis is not implemented"),
            Self::Cancelled => f.write_str("the analysis was cancelled"),
        }
    }
}

impl std::error::Error for MusicUnderstandingError {}

/// `MusicUnderstandingSession.TimedValue`: a value at one time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedValue<T> {
    pub time: f64,
    pub value: T,
}

/// `MusicUnderstandingSession.RangedValue`: a value over a time range (seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct RangedValue<T> {
    pub range: Range<f64>,
    pub value: T,
}

/// `RhythmResult`: beat timestamps, bar start times and tempo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RhythmResult {
    /// Timestamp of each detected beat, in seconds.
    pub beats: Vec<f64>,
    /// Start of each bar: every fourth beat, counted from the first beat.
    pub bars: Vec<f64>,
    /// Tempo; 0 when the input has no rhythmic onsets (silence).
    pub beats_per_minute: f64,
}

/// `PaceResult`: energy and momentum over time, as events per minute for each range of the song. A higher value means a
/// busier or more energetic stretch, independent of the tempo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaceResult {
    pub ranges: Vec<RangedValue<f64>>,
}

/// Pace from a beat grid (as `RhythmResult` gives). The beats are cut into non-overlapping windows of
/// `beats_per_window` beat intervals; each window's value is `beats_per_window * 60 / span`, the events per minute over
/// the seconds it covers. A trailing part shorter than a window is dropped, and so is a window that spans no time.
/// This counts beats, not the reference framework's perceptual energy measure. A window of zero beats gives no ranges.
pub fn analyze_pace(beats: &[f64], beats_per_window: usize) -> PaceResult {
    let mut ranges = Vec::new();
    if beats_per_window == 0 {
        return PaceResult::default();
    }
    let mut i = 0;
    while i + beats_per_window < beats.len() {
        let (start, end) = (beats[i], beats[i + beats_per_window]);
        let span = end - start;
        if span > 0.0 {
            ranges.push(RangedValue { range: start..end, value: beats_per_window as f64 * 60.0 / span });
        }
        i += beats_per_window;
    }
    PaceResult { ranges }
}

/// `MusicUnderstandingSession.SessionResult`. Only the fields for implemented analyses exist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionResult {
    pub rhythm: Option<RhythmResult>,
}

/// A session over one mono buffer (`init(audioProvider:)` for a whole buffer). `analyze` and `analyze_for` run
/// synchronously; `cancel` may be called from another thread through an `Arc` of the session.
pub struct MusicUnderstandingSession {
    samples: Vec<f32>,
    sample_rate: u32,
    cancelled: Arc<AtomicBool>,
}

impl MusicUnderstandingSession {
    /// Mono samples in -1.0..=1.0 at `sample_rate` Hz.
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Result<Self, MusicUnderstandingError> {
        if samples.is_empty() || sample_rate == 0 {
            return Err(MusicUnderstandingError::InvalidAsset);
        }
        Ok(Self { samples, sample_rate, cancelled: Arc::new(AtomicBool::new(false)) })
    }

    /// Interleaved frames (`channels` samples per frame), mixed down to mono by averaging the channels.
    pub fn from_interleaved(interleaved: &[f32], channels: usize, sample_rate: u32) -> Result<Self, MusicUnderstandingError> {
        if channels == 0 {
            return Err(MusicUnderstandingError::InvalidAsset);
        }
        let mono: Vec<f32> = interleaved.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
        Self::new(mono, sample_rate)
    }

    /// Length of the buffer in seconds.
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }

    /// `analyze()`: every analysis this module supports.
    pub fn analyze(&self) -> Result<SessionResult, MusicUnderstandingError> {
        self.analyze_for(&AnalysisType::SUPPORTED)
    }

    /// `analyze(for:)`. An empty set and any unsupported type fail the whole call, before any work is done.
    pub fn analyze_for(&self, types: &[AnalysisType]) -> Result<SessionResult, MusicUnderstandingError> {
        if types.is_empty() {
            return Err(MusicUnderstandingError::EmptyAnalysisSet);
        }
        if let Some(kind) = types.iter().find(|t| !t.is_supported()) {
            return Err(MusicUnderstandingError::Unsupported(*kind));
        }
        self.cancelled.store(false, Ordering::Relaxed);
        let mut out = SessionResult::default();
        for kind in types {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(MusicUnderstandingError::Cancelled);
            }
            if *kind == AnalysisType::Rhythm {
                out.rhythm = Some(analyze_rhythm(&self.samples, self.sample_rate));
            }
        }
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(MusicUnderstandingError::Cancelled);
        }
        Ok(out)
    }

    /// `cancel()`: asks a running analysis to stop; it returns `Cancelled` at its next check.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl AnalysisType {
    fn is_supported(self) -> bool {
        Self::SUPPORTED.contains(&self)
    }
}

/// Rhythm analysis: an onset-energy envelope (10 ms frames), a tempo from its autocorrelation over 60 to 200 BPM
/// with a mild prior centred on 120 BPM (so a double or half tempo loses ties), then a beat grid whose phase lines
/// up with the strongest onsets. Silence gives no beats and 0 BPM.
pub fn analyze_rhythm(samples: &[f32], sample_rate: u32) -> RhythmResult {
    if sample_rate == 0 {
        return RhythmResult::default();
    }
    let hop = ((sample_rate as f64 / 100.0).round() as usize).max(1);
    let frame_rate = sample_rate as f64 / hop as f64;
    let flux = onset_envelope(samples, hop);
    let period = match tempo_period(&flux, frame_rate) {
        Some(period) => period,
        None => return RhythmResult::default(),
    };
    let phase = best_phase(&flux, period);
    let mut beats = Vec::new();
    let mut pos = phase;
    while pos < flux.len() as f64 {
        beats.push(pos * hop as f64 / sample_rate as f64);
        pos += period;
    }
    let bars: Vec<f64> = beats.iter().step_by(4).copied().collect();
    RhythmResult { beats, bars, beats_per_minute: 60.0 * frame_rate / period }
}

/// Positive changes in log-compressed frame energy. Decays and silence add nothing.
fn onset_envelope(samples: &[f32], hop: usize) -> Vec<f64> {
    let mut prev = 0.0f64;
    let mut out = Vec::with_capacity(samples.len() / hop + 1);
    for frame in samples.chunks(hop) {
        let energy = frame
            .iter()
            .map(|s| {
                let v = *s as f64;
                v * v
            })
            .sum::<f64>()
            / frame.len() as f64;
        let level = (1.0 + 1000.0 * energy).ln();
        out.push((level - prev).max(0.0));
        prev = level;
    }
    out
}

/// The beat period in envelope frames for the strongest autocorrelation lag in 60 to 200 BPM, or None when the
/// envelope has no energy to correlate.
fn tempo_period(flux: &[f64], frame_rate: f64) -> Option<f64> {
    let min_lag = (frame_rate * 60.0 / 200.0).floor().max(1.0) as usize;
    let max_lag = (frame_rate * 60.0 / 60.0).ceil() as usize;
    let mut best: Option<(f64, f64)> = None; // (score, lag in frames)
    for lag in min_lag..=max_lag {
        if lag >= flux.len() {
            break;
        }
        let n = flux.len() - lag;
        let corr: f64 = (0..n).map(|i| flux[i] * flux[i + lag]).sum::<f64>() / n as f64;
        let bpm = 60.0 * frame_rate / lag as f64;
        let prior = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let score = corr * prior;
        if score > 0.0 && best.map_or(true, |(s, _)| score > s) {
            best = Some((score, lag as f64));
        }
    }
    best.map(|(_, lag)| lag)
}

/// The grid offset, in frames (half-frame steps), whose beats collect the most onset energy.
fn best_phase(flux: &[f64], period: f64) -> f64 {
    let mut best = (f64::NEG_INFINITY, 0.0);
    let steps = (period * 2.0).ceil() as usize;
    for step in 0..steps {
        let start = step as f64 / 2.0;
        let mut score = 0.0;
        let mut pos = start;
        while pos < flux.len() as f64 {
            score += flux[pos as usize];
            pos += period;
        }
        if score > best.0 {
            best = (score, start);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short decaying burst (10 ms) every beat at `bpm`, mono.
    fn click_track(bpm: f64, seconds: f64, sample_rate: u32) -> Vec<f32> {
        let n = (seconds * sample_rate as f64) as usize;
        let mut out = vec![0.0f32; n];
        let period = 60.0 / bpm * sample_rate as f64;
        let burst = sample_rate as usize / 100;
        let mut at = 0.0f64;
        while (at as usize) < n {
            let start = at as usize;
            for k in 0..burst.min(n - start) {
                out[start + k] += 0.9 * (-(k as f32) / 30.0).exp();
            }
            at += period;
        }
        out
    }

    #[test]
    fn click_track_tempo_is_found() {
        let r = analyze_rhythm(&click_track(120.0, 10.0, 44100), 44100);
        assert!((r.beats_per_minute - 120.0).abs() < 0.5, "bpm {}", r.beats_per_minute);
    }

    #[test]
    fn beats_fall_on_the_click_grid() {
        let r = analyze_rhythm(&click_track(120.0, 10.0, 44100), 44100);
        assert!(r.beats.len() >= 18, "beats {}", r.beats.len());
        for t in &r.beats {
            let nearest = (t / 0.5).round() * 0.5;
            assert!((t - nearest).abs() < 0.02, "beat at {t} is off the 0.5 s grid");
        }
    }

    #[test]
    fn bars_start_on_every_fourth_beat() {
        let r = analyze_rhythm(&click_track(120.0, 10.0, 44100), 44100);
        assert!(r.bars.len() >= 4);
        for (k, bar) in r.bars.iter().enumerate() {
            assert_eq!(*bar, r.beats[k * 4]);
        }
        assert!((r.bars[1] - r.bars[0] - 2.0).abs() < 0.02);
    }

    #[test]
    fn silence_has_no_beats_and_no_tempo() {
        let r = analyze_rhythm(&vec![0.0f32; 44100 * 5], 44100);
        assert!(r.beats.is_empty());
        assert!(r.bars.is_empty());
        assert_eq!(r.beats_per_minute, 0.0);
    }

    #[test]
    fn tempo_is_found_at_a_lower_sample_rate() {
        let r = analyze_rhythm(&click_track(120.0, 10.0, 22050), 22050);
        assert!((r.beats_per_minute - 120.0).abs() < 1.0, "bpm {}", r.beats_per_minute);
    }

    #[test]
    fn session_rejects_empty_and_unsupported_sets() {
        let s = MusicUnderstandingSession::new(click_track(120.0, 2.0, 44100), 44100).unwrap();
        assert_eq!(s.analyze_for(&[]).unwrap_err(), MusicUnderstandingError::EmptyAnalysisSet);
        assert_eq!(
            s.analyze_for(&[AnalysisType::Rhythm, AnalysisType::Key]).unwrap_err(),
            MusicUnderstandingError::Unsupported(AnalysisType::Key)
        );
    }

    #[test]
    fn analyze_fills_only_the_rhythm_result() {
        let s = MusicUnderstandingSession::new(click_track(120.0, 10.0, 44100), 44100).unwrap();
        let result = s.analyze().unwrap();
        let rhythm = result.rhythm.clone().expect("rhythm result");
        assert!((rhythm.beats_per_minute - 120.0).abs() < 0.5);
        assert_eq!(result, SessionResult { rhythm: Some(rhythm) });
        assert!((s.duration() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn stereo_input_is_mixed_to_mono_and_bad_input_is_refused() {
        let s = MusicUnderstandingSession::from_interleaved(&[1.0, 0.0, 0.0, 0.0], 2, 44100).unwrap();
        assert_eq!(s.samples, vec![0.5, 0.0]);
        assert_eq!(MusicUnderstandingSession::new(Vec::new(), 44100).err(), Some(MusicUnderstandingError::InvalidAsset));
        assert_eq!(MusicUnderstandingSession::from_interleaved(&[0.0], 0, 44100).err(), Some(MusicUnderstandingError::InvalidAsset));
    }
}

#[cfg(test)]
mod pace_tests {
    use super::*;

    #[test]
    fn steady_beats_give_the_same_rate_in_every_window() {
        // Beats every 0.5 s (120 beats a minute). Windows of 4 intervals span 2 s: 4 * 60 / 2 = 120.
        let beats: Vec<f64> = (0..9).map(|i| i as f64 * 0.5).collect();
        let pace = analyze_pace(&beats, 4);
        assert_eq!(pace.ranges.len(), 2);
        assert_eq!(pace.ranges[0].range, 0.0..2.0);
        assert_eq!(pace.ranges[1].range, 2.0..4.0);
        assert!(pace.ranges.iter().all(|r| (r.value - 120.0).abs() < 1e-9));
    }

    #[test]
    fn a_slower_stretch_has_a_lower_rate() {
        // 0.5 s intervals up to 2.0 s, then 1.0 s intervals. The second window spans 2.0..6.0 s: 4 * 60 / 4 = 60.
        let beats = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0];
        let pace = analyze_pace(&beats, 4);
        assert_eq!(pace.ranges.len(), 2);
        assert!((pace.ranges[0].value - 120.0).abs() < 1e-9);
        assert!((pace.ranges[1].value - 60.0).abs() < 1e-9);
    }

    #[test]
    fn a_trailing_part_shorter_than_a_window_is_dropped() {
        // Five beats with windows of 4 intervals: the window needs beat 4 to exist, which it does; a second window would
        // need beat 8 and is dropped.
        let beats = [0.0, 1.0, 2.0, 3.0, 4.0];
        let pace = analyze_pace(&beats, 4);
        assert_eq!(pace.ranges.len(), 1);
        assert_eq!(pace.ranges[0].range, 0.0..4.0);
        assert!((pace.ranges[0].value - 60.0).abs() < 1e-9);
    }

    #[test]
    fn too_few_beats_zero_windows_and_zero_spans_give_nothing() {
        assert!(analyze_pace(&[0.0, 1.0], 4).ranges.is_empty());
        assert!(analyze_pace(&[0.0, 1.0, 2.0, 3.0, 4.0], 0).ranges.is_empty());
        // Repeated timestamps span no time, so no rate can be given.
        assert!(analyze_pace(&[1.0, 1.0, 1.0, 1.0, 1.0], 4).ranges.is_empty());
        assert_eq!(analyze_pace(&[], 4), PaceResult::default());
    }
}
