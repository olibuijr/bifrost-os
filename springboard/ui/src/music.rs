//! Music understanding: a session runs analyses over a mono audio buffer and returns time-stamped results
//! (`MusicUnderstandingSession`, `SessionResult`, `RhythmResult`). Implemented: rhythm (beats, bars, BPM) from an
//! onset-energy envelope; key over time (`KeyResult`) from 12-bin chroma with a Krumhansl-Kessler profile match; and
//! structure (`StructureResult`) as chroma-novelty boundaries. Pace is available from a beat grid through
//! `analyze_pace`; it is not part of `analyze` yet. Loudness and instrument activity are not implemented; asking for
//! them fails with `Unsupported`. Times are seconds from the start of the buffer. Pure Rust, no decoding: the caller
//! hands over samples.

use std::f64::consts::PI;
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
    pub const SUPPORTED: [AnalysisType; 3] = [AnalysisType::Rhythm, AnalysisType::Key, AnalysisType::Structure];
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
    pub key: Option<KeyResult>,
    pub structure: Option<StructureResult>,
}

/// Seconds per chroma frame in a session: non-overlapping frames of this many samples.
pub const CHROMA_FRAME: usize = 4096;
/// The window over which a session reports one key (`KeyResult.ranges`), in seconds.
pub const KEY_WINDOW_SECONDS: f64 = 10.0;
/// Structure: frames on each side of a candidate boundary (about 1.5 s at 44.1 kHz with `CHROMA_FRAME`).
pub const STRUCTURE_WINDOW_FRAMES: usize = 16;
/// Structure: the smallest chroma-novelty value that counts as a boundary (chroma vectors are scaled to sum 1, so the
/// largest possible distance between two of them is sqrt(2)).
pub const STRUCTURE_MIN_NOVELTY: f64 = 0.3;

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
        let frame_seconds = CHROMA_FRAME as f64 / self.sample_rate as f64;
        let needs_chroma = types.iter().any(|t| matches!(t, AnalysisType::Key | AnalysisType::Structure));
        let chroma = if needs_chroma { chroma_frames(&self.samples, self.sample_rate, CHROMA_FRAME) } else { Vec::new() };
        for kind in types {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(MusicUnderstandingError::Cancelled);
            }
            match kind {
                AnalysisType::Rhythm => out.rhythm = Some(analyze_rhythm(&self.samples, self.sample_rate)),
                AnalysisType::Key => {
                    let window = ((KEY_WINDOW_SECONDS / frame_seconds).round() as usize).max(1);
                    out.key = Some(analyze_key(&chroma, frame_seconds, window));
                }
                AnalysisType::Structure => out.structure = Some(structure_from_chroma(&chroma, frame_seconds)),
                _ => {}
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

/// `KeyResult.Mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Major,
    Minor,
}

/// A musical key: `tonic` is the pitch class of the root, C = 0 through B = 11, with no enharmonic spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub tonic: u8,
    pub mode: Mode,
}

/// `KeyResult`: the key of each time range of the song, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyResult {
    pub ranges: Vec<RangedValue<Key>>,
}

/// `StructureResult.segments`: consecutive time ranges that the boundaries split the buffer into, in seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructureResult {
    pub segments: Vec<Range<f64>>,
}

/// Krumhansl and Kessler's (1982) probe-tone profiles for a major and a minor key, indexed by scale degree from the
/// tonic (degree 0 is the tonic). Each profile is matched against chroma at every tonic.
pub const MAJOR_PROFILE: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
pub const MINOR_PROFILE: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// The key whose profile correlates best (Pearson) with a 12-bin chroma vector, over the 12 major and 12 minor tonics.
/// A tie goes to the earlier key in that order. Returns None when the chroma has no spread (silence or a flat
/// spectrum), since no key can be preferred.
pub fn key_from_chroma(chroma: &[f64; 12]) -> Option<Key> {
    let mean = chroma.iter().sum::<f64>() / 12.0;
    let dev: Vec<f64> = chroma.iter().map(|c| c - mean).collect();
    let spread: f64 = dev.iter().map(|d| d * d).sum();
    if spread <= 0.0 {
        return None;
    }
    let mut best: Option<(f64, Key)> = None;
    for (mode, profile) in [(Mode::Major, &MAJOR_PROFILE), (Mode::Minor, &MINOR_PROFILE)] {
        let profile_mean = profile.iter().sum::<f64>() / 12.0;
        for tonic in 0..12usize {
            let mut cross = 0.0;
            let mut profile_spread = 0.0;
            for (i, d) in dev.iter().enumerate() {
                let p = profile[(i + 12 - tonic) % 12] - profile_mean;
                cross += d * p;
                profile_spread += p * p;
            }
            let r = cross / (spread * profile_spread).sqrt();
            if best.map_or(true, |(b, _)| r > b) {
                best = Some((r, Key { tonic: tonic as u8, mode }));
            }
        }
    }
    best.map(|(_, key)| key)
}

/// `KeyResult` over time: the chroma frames are cut into windows of `window_frames` (the last one may be shorter), each
/// window's chroma is summed, and its key is given over the window's time range. A window with no key (silence) gives
/// no range. `frame_seconds` is the duration of one chroma frame.
pub fn analyze_key(chroma: &[[f64; 12]], frame_seconds: f64, window_frames: usize) -> KeyResult {
    let mut ranges = Vec::new();
    if window_frames == 0 {
        return KeyResult::default();
    }
    let mut start = 0;
    while start < chroma.len() {
        let end = (start + window_frames).min(chroma.len());
        let mut sum = [0.0f64; 12];
        for frame in &chroma[start..end] {
            for (s, c) in sum.iter_mut().zip(frame) {
                *s += c;
            }
        }
        if let Some(key) = key_from_chroma(&sum) {
            ranges.push(RangedValue { range: start as f64 * frame_seconds..end as f64 * frame_seconds, value: key });
        }
        start = end;
    }
    KeyResult { ranges }
}

/// Chroma from samples: each non-overlapping frame of `frame_len` samples (Hann-windowed) is measured at the equal
/// tempered tones MIDI 48 (C3) to 95 (B6), each with the Goertzel algorithm, and the power of every tone is added to
/// its pitch class. A tone at or above half the sample rate is skipped. A trailing part shorter than a frame is
/// dropped. Frequency resolution is about one bin per `sample_rate / frame_len` Hz, so low tones blur into their
/// neighbours; this is an approximation, not a transcription. Silence gives all-zero frames.
pub fn chroma_frames(samples: &[f32], sample_rate: u32, frame_len: usize) -> Vec<[f64; 12]> {
    if sample_rate == 0 || frame_len < 2 {
        return Vec::new();
    }
    let rate = sample_rate as f64;
    let window: Vec<f64> =
        (0..frame_len).map(|n| 0.5 - 0.5 * (2.0 * PI * n as f64 / (frame_len - 1) as f64).cos()).collect();
    let tones: Vec<(usize, f64)> = (48u32..=95)
        .map(|m| ((m % 12) as usize, 440.0 * 2f64.powf((m as f64 - 69.0) / 12.0)))
        .filter(|&(_, f)| f < rate / 2.0)
        .collect();
    samples
        .chunks_exact(frame_len)
        .map(|frame| {
            let windowed: Vec<f64> = frame.iter().zip(&window).map(|(s, w)| *s as f64 * w).collect();
            let mut chroma = [0.0f64; 12];
            for &(pc, freq) in &tones {
                chroma[pc] += goertzel_power(&windowed, freq, rate);
            }
            chroma
        })
        .collect()
}

/// The power of one tone in `samples`, from the Goertzel recurrence.
fn goertzel_power(samples: &[f64], freq: f64, sample_rate: f64) -> f64 {
    let coeff = 2.0 * (2.0 * PI * freq / sample_rate).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Structure boundaries from per-frame feature vectors (all the same length). The novelty at frame `b` is the
/// Euclidean distance between the mean feature of the `window` frames before `b` and the mean of the `window` frames
/// from `b`. A boundary is a novelty peak (not lower than its left neighbour, higher than its right) of at least
/// `min_novelty`; peaks closer than `window` frames to an earlier boundary are dropped. The buffer is cut at the
/// boundaries. Fewer than `2 * window` frames gives one segment; no frames gives none.
pub fn segment_frames(features: &[Vec<f64>], window: usize, min_novelty: f64) -> Vec<Range<usize>> {
    let n = features.len();
    if n == 0 {
        return Vec::new();
    }
    if window == 0 || n < 2 * window {
        return vec![0..n];
    }
    let novelty = |b: usize| -> f64 {
        let before = mean_of(&features[b - window..b]);
        let after = mean_of(&features[b..b + window]);
        before.iter().zip(&after).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt()
    };
    let last = n - window;
    let mut boundaries: Vec<usize> = Vec::new();
    for b in window..=last {
        let v = novelty(b);
        if v < min_novelty {
            continue;
        }
        let left = if b > window { novelty(b - 1) } else { 0.0 };
        let right = if b < last { novelty(b + 1) } else { 0.0 };
        if v < left || v <= right {
            continue;
        }
        if boundaries.last().map_or(false, |&p| b - p < window) {
            continue;
        }
        boundaries.push(b);
    }
    let mut segments = Vec::with_capacity(boundaries.len() + 1);
    let mut from = 0;
    for b in boundaries {
        segments.push(from..b);
        from = b;
    }
    segments.push(from..n);
    segments
}

/// The element-wise mean of non-empty feature vectors.
fn mean_of(frames: &[Vec<f64>]) -> Vec<f64> {
    let mut out = vec![0.0f64; frames[0].len()];
    for frame in frames {
        for (o, x) in out.iter_mut().zip(frame) {
            *o += x;
        }
    }
    for o in &mut out {
        *o /= frames.len() as f64;
    }
    out
}

/// `StructureResult` from chroma frames: each frame is scaled to sum 1 (silence stays zero), then `segment_frames`
/// with the session's window and threshold, and frame ranges become seconds.
pub fn structure_from_chroma(chroma: &[[f64; 12]], frame_seconds: f64) -> StructureResult {
    let features: Vec<Vec<f64>> = chroma
        .iter()
        .map(|frame| {
            let total: f64 = frame.iter().sum();
            if total > 0.0 {
                frame.iter().map(|c| c / total).collect::<Vec<f64>>()
            } else {
                vec![0.0; 12]
            }
        })
        .collect();
    let segments = segment_frames(&features, STRUCTURE_WINDOW_FRAMES, STRUCTURE_MIN_NOVELTY)
        .into_iter()
        .map(|r| r.start as f64 * frame_seconds..r.end as f64 * frame_seconds)
        .collect();
    StructureResult { segments }
}

#[cfg(test)]
mod key_structure_tests {
    use super::*;

    /// A 12-bin profile rotated so that `profile[(i - tonic) mod 12]` sits at pitch class `i`.
    fn rotated(profile: &[f64; 12], tonic: usize) -> [f64; 12] {
        let mut out = [0.0; 12];
        for (i, o) in out.iter_mut().enumerate() {
            *o = profile[(i + 12 - tonic) % 12];
        }
        out
    }

    #[test]
    fn the_major_profile_at_c_is_c_major() {
        assert_eq!(key_from_chroma(&MAJOR_PROFILE), Some(Key { tonic: 0, mode: Mode::Major }));
    }

    #[test]
    fn a_rotated_major_profile_gives_its_tonic() {
        // Chroma built on G (pitch class 7) is the major profile shifted up seven semitones.
        assert_eq!(key_from_chroma(&rotated(&MAJOR_PROFILE, 7)), Some(Key { tonic: 7, mode: Mode::Major }));
    }

    #[test]
    fn a_rotated_minor_profile_gives_a_minor_key() {
        // A minor is pitch class 9 with the minor profile.
        assert_eq!(key_from_chroma(&rotated(&MINOR_PROFILE, 9)), Some(Key { tonic: 9, mode: Mode::Minor }));
    }

    #[test]
    fn flat_chroma_and_silence_have_no_key() {
        assert_eq!(key_from_chroma(&[1.0; 12]), None);
        assert_eq!(key_from_chroma(&[0.0; 12]), None);
    }

    #[test]
    fn key_over_time_gives_one_range_per_window() {
        // Four frames of C major then four of G major, windows of four frames at 0.5 s each.
        let mut chroma = vec![MAJOR_PROFILE; 4];
        chroma.extend(vec![rotated(&MAJOR_PROFILE, 7); 4]);
        let result = analyze_key(&chroma, 0.5, 4);
        assert_eq!(result.ranges.len(), 2);
        assert_eq!(result.ranges[0].range, 0.0..2.0);
        assert_eq!(result.ranges[0].value, Key { tonic: 0, mode: Mode::Major });
        assert_eq!(result.ranges[1].range, 2.0..4.0);
        assert_eq!(result.ranges[1].value, Key { tonic: 7, mode: Mode::Major });
    }

    #[test]
    fn a_pure_tone_puts_its_energy_in_its_pitch_class() {
        // 440 Hz is A (pitch class 9); two frames of 4096 samples at 44.1 kHz.
        let samples: Vec<f32> =
            (0..8192).map(|n| (2.0 * PI * 440.0 * n as f64 / 44100.0).sin() as f32).collect();
        let frames = chroma_frames(&samples, 44100, 4096);
        assert_eq!(frames.len(), 2);
        for frame in &frames {
            let best = (0..12).max_by(|&a, &b| frame[a].partial_cmp(&frame[b]).unwrap()).unwrap();
            assert_eq!(best, 9);
        }
        assert!(chroma_frames(&vec![0.0f32; 8192], 44100, 4096).iter().all(|f| f.iter().all(|c| *c == 0.0)));
    }

    #[test]
    fn a_change_of_texture_is_a_boundary_at_the_change() {
        // Ten frames of one feature then ten of another, with a window of three frames: the only novelty peak is at
        // frame 10, where both sides are pure (distance sqrt(2)). Frames 9 and 11 score sqrt(8)/3, below the peak.
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let mut features = vec![a.clone(); 10];
        features.extend(vec![b.clone(); 10]);
        assert_eq!(segment_frames(&features, 3, 0.5), vec![0..10, 10..20]);
    }

    #[test]
    fn a_threshold_above_the_peak_gives_one_segment() {
        let mut features = vec![vec![1.0, 0.0]; 10];
        features.extend(vec![vec![0.0, 1.0]; 10]);
        assert_eq!(segment_frames(&features, 3, 1.5), vec![0..20]);
        assert!(segment_frames(&[], 3, 0.5).is_empty());
        assert_eq!(segment_frames(&features[..5], 3, 0.5), vec![0..5]);
    }
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
            s.analyze_for(&[AnalysisType::Rhythm, AnalysisType::Loudness]).unwrap_err(),
            MusicUnderstandingError::Unsupported(AnalysisType::Loudness)
        );
        assert_eq!(
            s.analyze_for(&[AnalysisType::Structure, AnalysisType::InstrumentActivity]).unwrap_err(),
            MusicUnderstandingError::Unsupported(AnalysisType::InstrumentActivity)
        );
    }

    #[test]
    fn analyze_fills_rhythm_key_and_structure_and_nothing_else() {
        let s = MusicUnderstandingSession::new(click_track(120.0, 10.0, 44100), 44100).unwrap();
        let result = s.analyze().unwrap();
        let rhythm = result.rhythm.clone().expect("rhythm result");
        assert!((rhythm.beats_per_minute - 120.0).abs() < 0.5);
        assert!(result.key.is_some());
        assert!(result.structure.is_some());
        assert_eq!(result.rhythm.as_ref(), Some(&rhythm));
        assert!((s.duration() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rhythm_only_leaves_key_and_structure_empty() {
        let s = MusicUnderstandingSession::new(click_track(120.0, 10.0, 44100), 44100).unwrap();
        let result = s.analyze_for(&[AnalysisType::Rhythm]).unwrap();
        assert!(result.rhythm.is_some());
        assert_eq!(result.key, None);
        assert_eq!(result.structure, None);
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
