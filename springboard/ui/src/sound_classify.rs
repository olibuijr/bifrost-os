//! Sound classification: a request slices mono samples into overlapping windows, a classifier labels each window, and
//! the results carry the top candidates with their confidences and the time span they cover. `SoundFileAnalyzer` runs
//! one request over a whole buffer and can be cancelled from another thread.
//!
//! There is no built-in label set and no model runtime here. The caller supplies the classifier as a function from one
//! window of samples to labelled confidences, so a model loaded elsewhere plugs in without a dependency. Pure Rust, no
//! decoding: the caller hands over samples.

use std::cmp::Ordering;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

/// Candidates kept per window. A result lists at most this many classifications, highest confidence first.
pub const TOP_CANDIDATES: usize = 3;

/// A classifier: one window of mono samples in, labelled confidences (0.0..=1.0) out.
pub type Classifier<'a> = dyn Fn(&[f32]) -> Vec<(String, f64)> + 'a;

/// One label and its confidence (`SNClassification`).
#[derive(Clone, Debug, PartialEq)]
pub struct Classification {
    pub identifier: String,
    pub confidence: f64,
}

/// The candidates for one time span (`SNClassificationResult`). `classifications` is sorted by confidence, highest
/// first; ties sort by identifier so the order is stable.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassificationResult {
    /// Seconds from the start of the buffer.
    pub time_range: Range<f64>,
    pub classifications: Vec<Classification>,
}

impl ClassificationResult {
    /// The classification for one identifier, if it is among the kept candidates.
    pub fn classification_for(&self, identifier: &str) -> Option<&Classification> {
        self.classifications.iter().find(|c| c.identifier == identifier)
    }
}

/// The error codes this module produces (`SNError.Code`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundError {
    /// `invalidModel`: the request has no labels, so there is nothing to classify with.
    InvalidModel,
    /// `invalidFormat`: a window duration, overlap or sample rate that cannot be used.
    InvalidFormat,
    /// `invalidFile`: the buffer holds no samples.
    InvalidFile,
    /// `operationFailed`: the classifier returned a confidence outside 0.0..=1.0 (or not a number).
    OperationFailed,
    /// `unknownError`: reserved for failures no other code covers. This module never produces it.
    UnknownError,
    /// `cancel()` was called while the analysis was running. This is not an error code of the reference framework;
    /// the reference API reports cancellation as a completion instead.
    Cancelled,
}

impl std::fmt::Display for SoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidModel => f.write_str("the classification request has no labels"),
            Self::InvalidFormat => f.write_str("the window duration, overlap or sample rate is not usable"),
            Self::InvalidFile => f.write_str("the audio buffer holds no samples"),
            Self::OperationFailed => f.write_str("the classifier returned an invalid confidence"),
            Self::UnknownError => f.write_str("an unknown sound analysis error"),
            Self::Cancelled => f.write_str("the analysis was cancelled"),
        }
    }
}

impl std::error::Error for SoundError {}

/// A classification request: window length, overlap between successive windows, and the labels the classifier
/// knows. Build it with `new`, which checks the values.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassifyRequest {
    /// Seconds of audio per prediction (`windowDuration`).
    pub window_duration: f64,
    /// Fraction of each window that repeats in the next one, 0.0..1.0 (`overlapFactor`). 0.0 means no overlap.
    pub overlap_factor: f64,
    /// Every label the classifier can return (`knownClassifications`). Never empty.
    pub known_classifications: Vec<String>,
}

impl ClassifyRequest {
    /// Checks the request. A window must last a positive, finite time, the overlap must be in `[0, 1)`, and at least
    /// one label must be known.
    pub fn new(window_duration: f64, overlap_factor: f64, known_classifications: Vec<String>) -> Result<Self, SoundError> {
        if !(window_duration.is_finite() && window_duration > 0.0) || !(0.0..1.0).contains(&overlap_factor) {
            return Err(SoundError::InvalidFormat);
        }
        if known_classifications.is_empty() {
            return Err(SoundError::InvalidModel);
        }
        Ok(Self { window_duration, overlap_factor, known_classifications })
    }

    /// Window length in samples at `sample_rate` (at least one sample).
    pub fn window_frames(&self, sample_rate: u32) -> usize {
        ((self.window_duration * sample_rate as f64).round() as usize).max(1)
    }

    /// Samples between the starts of successive windows (at least one sample).
    pub fn hop_frames(&self, sample_rate: u32) -> usize {
        ((self.window_frames(sample_rate) as f64 * (1.0 - self.overlap_factor)).round() as usize).max(1)
    }
}

/// Sorts a classifier's output into the kept candidates. A confidence outside 0.0..=1.0 (or NaN) fails the window.
pub(crate) fn top_candidates(mut raw: Vec<(String, f64)>) -> Result<Vec<Classification>, SoundError> {
    if raw.iter().any(|(_, c)| !(0.0..=1.0).contains(c)) {
        return Err(SoundError::OperationFailed);
    }
    raw.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
    raw.truncate(TOP_CANDIDATES);
    Ok(raw.into_iter().map(|(identifier, confidence)| Classification { identifier, confidence }).collect())
}

/// A buffer of mono samples to analyze. `analyze` runs synchronously; `cancel` may be called from another thread
/// through a shared reference, for example while an observer callback runs.
pub struct SoundFileAnalyzer {
    samples: Vec<f32>,
    sample_rate: u32,
    cancelled: Arc<AtomicBool>,
}

impl SoundFileAnalyzer {
    /// Mono samples in -1.0..=1.0 at `sample_rate` Hz. The buffer must hold samples and the rate must be positive.
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Result<Self, SoundError> {
        if sample_rate == 0 {
            return Err(SoundError::InvalidFormat);
        }
        if samples.is_empty() {
            return Err(SoundError::InvalidFile);
        }
        Ok(Self { samples, sample_rate, cancelled: Arc::new(AtomicBool::new(false)) })
    }

    /// Runs `request` over every full window, in order, with the given classifier. `observer` receives each result
    /// as soon as it is made. Windows that would run past the end of the buffer are not analyzed, so a buffer
    /// shorter than one window gives no results. Cancelling stops the run at the next window with `Cancelled`.
    pub fn analyze(
        &self,
        request: &ClassifyRequest,
        classifier: &Classifier<'_>,
        observer: &mut dyn FnMut(&ClassificationResult),
    ) -> Result<Vec<ClassificationResult>, SoundError> {
        self.cancelled.store(false, AtomicOrdering::Relaxed);
        let window = request.window_frames(self.sample_rate);
        let hop = request.hop_frames(self.sample_rate);
        let sr = self.sample_rate as f64;
        let mut out = Vec::new();
        let mut start = 0usize;
        while start + window <= self.samples.len() {
            if self.cancelled.load(AtomicOrdering::Relaxed) {
                return Err(SoundError::Cancelled);
            }
            let raw = classifier(&self.samples[start..start + window]);
            let result = ClassificationResult {
                time_range: start as f64 / sr..(start + window) as f64 / sr,
                classifications: top_candidates(raw)?,
            };
            observer(&result);
            out.push(result);
            start += hop;
        }
        if self.cancelled.load(AtomicOrdering::Relaxed) {
            return Err(SoundError::Cancelled);
        }
        Ok(out)
    }

    /// Asks a running analysis to stop at its next window.
    pub fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn request_rejects_unusable_values() {
        assert_eq!(ClassifyRequest::new(0.0, 0.5, labels(&["a"])).unwrap_err(), SoundError::InvalidFormat);
        assert_eq!(ClassifyRequest::new(f64::NAN, 0.0, labels(&["a"])).unwrap_err(), SoundError::InvalidFormat);
        assert_eq!(ClassifyRequest::new(1.0, 1.0, labels(&["a"])).unwrap_err(), SoundError::InvalidFormat);
        assert_eq!(ClassifyRequest::new(1.0, -0.1, labels(&["a"])).unwrap_err(), SoundError::InvalidFormat);
        assert_eq!(ClassifyRequest::new(1.0, 0.0, Vec::new()).unwrap_err(), SoundError::InvalidModel);
        assert!(ClassifyRequest::new(1.0, 0.0, labels(&["a"])).is_ok());
    }

    #[test]
    fn window_and_hop_are_whole_samples() {
        // 1.0 s at 10 Hz is 10 samples; 50% overlap steps 5 samples.
        let r = ClassifyRequest::new(1.0, 0.5, labels(&["a"])).unwrap();
        assert_eq!(r.window_frames(10), 10);
        assert_eq!(r.hop_frames(10), 5);
        // A tiny window still has one sample, and full overlap-free hop equals the window.
        let tiny = ClassifyRequest::new(0.01, 0.0, labels(&["a"])).unwrap();
        assert_eq!(tiny.window_frames(10), 1);
        assert_eq!(tiny.hop_frames(10), 1);
    }

    #[test]
    fn empty_buffer_and_zero_rate_are_refused() {
        assert_eq!(SoundFileAnalyzer::new(Vec::new(), 10).err(), Some(SoundError::InvalidFile));
        assert_eq!(SoundFileAnalyzer::new(vec![0.0; 4], 0).err(), Some(SoundError::InvalidFormat));
    }

    #[test]
    fn overlapping_windows_are_classified_with_their_time_spans() {
        // Samples 5..10 are loud. Windows start at 0, 5, 10 (10 samples each, hop 5, sample rate 10 Hz).
        let mut samples = vec![0.0f32; 20];
        for s in &mut samples[5..10] {
            *s = 1.0;
        }
        let analyzer = SoundFileAnalyzer::new(samples, 10).unwrap();
        let request = ClassifyRequest::new(1.0, 0.5, labels(&["silence", "speech"])).unwrap();
        // The classifier looks at the first sample of each window.
        let classifier = |w: &[f32]| {
            if w[0] > 0.5 {
                vec![("speech".to_string(), 0.9), ("silence".to_string(), 0.2)]
            } else {
                vec![("silence".to_string(), 0.8)]
            }
        };
        let mut seen = 0;
        let results = analyzer.analyze(&request, &classifier, &mut |_| seen += 1).unwrap();
        assert_eq!(seen, 3);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].time_range, 0.0..1.0);
        assert_eq!(results[1].time_range, 0.5..1.5);
        assert_eq!(results[2].time_range, 1.0..2.0);
        assert_eq!(results[1].classification_for("speech").unwrap().confidence, 0.9);
        assert!(results[0].classification_for("speech").is_none());
        assert_eq!(results[2].classifications, vec![Classification { identifier: "silence".into(), confidence: 0.8 }]);
    }

    #[test]
    fn candidates_are_sorted_by_confidence_then_name_and_truncated() {
        let raw = vec![
            ("c".to_string(), 0.7),
            ("a".to_string(), 0.2),
            ("b".to_string(), 0.7),
            ("d".to_string(), 0.1),
        ];
        let kept = top_candidates(raw).unwrap();
        let names: Vec<&str> = kept.iter().map(|c| c.identifier.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"], "ties sort by name, and the fourth candidate is dropped");
        assert_eq!(TOP_CANDIDATES, 3);
    }

    #[test]
    fn invalid_confidence_fails_the_run() {
        let analyzer = SoundFileAnalyzer::new(vec![0.0; 10], 10).unwrap();
        let request = ClassifyRequest::new(1.0, 0.0, labels(&["x"])).unwrap();
        let too_big = |_: &[f32]| vec![("x".to_string(), 1.5)];
        assert_eq!(analyzer.analyze(&request, &too_big, &mut |_| {}).unwrap_err(), SoundError::OperationFailed);
        let nan = |_: &[f32]| vec![("x".to_string(), f64::NAN)];
        assert_eq!(analyzer.analyze(&request, &nan, &mut |_| {}).unwrap_err(), SoundError::OperationFailed);
    }

    #[test]
    fn cancel_from_the_observer_stops_at_the_next_window() {
        let analyzer = SoundFileAnalyzer::new(vec![0.0; 30], 10).unwrap();
        // Window 1 s, no overlap: windows at 0, 1, 2 s.
        let request = ClassifyRequest::new(1.0, 0.0, labels(&["x"])).unwrap();
        let classifier = |_: &[f32]| vec![("x".to_string(), 0.5)];
        let mut seen = 0;
        let outcome = analyzer.analyze(&request, &classifier, &mut |_| {
            seen += 1;
            analyzer.cancel();
        });
        assert_eq!(outcome.unwrap_err(), SoundError::Cancelled);
        assert_eq!(seen, 1, "the first window finishes, the second is never classified");
    }

    #[test]
    fn a_buffer_shorter_than_one_window_gives_no_results() {
        let analyzer = SoundFileAnalyzer::new(vec![0.0; 5], 10).unwrap();
        let request = ClassifyRequest::new(1.0, 0.0, labels(&["x"])).unwrap();
        let classifier = |_: &[f32]| vec![("x".to_string(), 0.5)];
        assert!(analyzer.analyze(&request, &classifier, &mut |_| {}).unwrap().is_empty());
    }
}
