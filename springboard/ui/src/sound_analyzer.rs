//! Sound classification requests and the analyzer that runs them over audio input. A `SoundRequest` names the
//! classifier, the length of the window it reads, how much successive windows overlap and an optional list of labels
//! to report. A `SoundAnalyzer` (stream form: the input arrives in blocks) mixes its input to mono, cuts it into those
//! windows, runs the caller's `SoundClassifier` on each window and passes every result to the request's
//! `SoundResultsObserving` with a time range. Also here: `SoundAnalysisError`, the failure codes an observer receives.
//!
//! Pure Rust: no classifier model ships in this module, so the caller supplies the classifier. No audio device or file
//! is opened. The analyzer keeps its timeline in frames: a block's start time is converted to a frame index, and
//! blocks that follow each other are joined without a gap check. A trailing window shorter than the request's window
//! is not classified; `complete_analysis` ends each request.

use crate::analyzer_input::{AnalyzerInput, AudioFormat};
use crate::sound_result::{SoundClassification, SoundClassificationResult, TimeRange};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Window length, in seconds, used when a request is given no valid window duration.
pub const DEFAULT_WINDOW_SECONDS: f64 = 3.0;

/// Largest overlap factor a request accepts. A factor of 1 would make every window start at the same frame.
pub const MAX_OVERLAP_FACTOR: f64 = 0.99;

/// The failure codes an observer can receive. The names follow the official error codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundAnalysisError {
    /// `invalidModel`: the classifier's model is not a valid model type.
    InvalidModel,
    /// `invalidFormat`: the audio data's format does not match the analyzer's format.
    InvalidFormat,
    /// `invalidFile`: an audio file is invalid.
    InvalidFile,
    /// `operationFailed`: the classifier failed to analyse a window.
    OperationFailed,
    /// `unknownError`: a failure that no other code covers.
    UnknownError,
}

impl SoundAnalysisError {
    /// The official case name of this code.
    pub fn code_name(self) -> &'static str {
        match self {
            Self::InvalidModel => "invalidModel",
            Self::InvalidFormat => "invalidFormat",
            Self::InvalidFile => "invalidFile",
            Self::OperationFailed => "operationFailed",
            Self::UnknownError => "unknownError",
        }
    }
}

impl fmt::Display for SoundAnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::InvalidModel => "the sound classifier's model is not a valid model type",
            Self::InvalidFormat => "the audio data's format is not valid",
            Self::InvalidFile => "the audio file is invalid",
            Self::OperationFailed => "the analysis failed",
            Self::UnknownError => "an unknown error occurred",
        };
        f.write_str(text)
    }
}

impl std::error::Error for SoundAnalysisError {}

/// Classifies one window of mono samples (values from -1 to 1) at `sample_rate` and returns (label, confidence) pairs.
/// Implement this to plug a model into the analyzer. A returned error ends the request that asked for the window.
pub trait SoundClassifier {
    fn classify(
        &mut self,
        window: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<(String, f64)>, SoundAnalysisError>;
}

/// Receives the results of one request. Each request has its own observer, so the callbacks carry no request argument.
pub trait SoundResultsObserving {
    fn did_produce(&mut self, result: &SoundClassificationResult);
    fn did_fail(&mut self, error: SoundAnalysisError);
    fn request_did_complete(&mut self);
}

/// What a [`ResultLog`] has recorded: the results in arrival order, the errors and whether the request completed.
#[derive(Debug, Default)]
struct LogState {
    results: Vec<SoundClassificationResult>,
    errors: Vec<SoundAnalysisError>,
    completed: bool,
}

/// A results observer whose records stay readable by the caller after the analyzer takes ownership of the observer.
/// Every clone shares the same records: pass [`ResultLog::observer`] to [`SoundAnalyzer::add`] and keep the log to read
/// what the analyzer delivered.
#[derive(Clone, Debug, Default)]
pub struct ResultLog {
    state: Rc<RefCell<LogState>>,
}

impl ResultLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// An observer that records into this log.
    pub fn observer(&self) -> Box<dyn SoundResultsObserving> {
        Box::new(self.clone())
    }

    /// The results delivered so far, oldest first.
    pub fn results(&self) -> Vec<SoundClassificationResult> {
        self.state.borrow().results.clone()
    }

    /// The failure codes delivered so far, oldest first.
    pub fn errors(&self) -> Vec<SoundAnalysisError> {
        self.state.borrow().errors.clone()
    }

    /// Whether the request reported completion.
    pub fn completed(&self) -> bool {
        self.state.borrow().completed
    }
}

impl SoundResultsObserving for ResultLog {
    fn did_produce(&mut self, result: &SoundClassificationResult) {
        self.state.borrow_mut().results.push(result.clone());
    }

    fn did_fail(&mut self, error: SoundAnalysisError) {
        self.state.borrow_mut().errors.push(error);
    }

    fn request_did_complete(&mut self) {
        self.state.borrow_mut().completed = true;
    }
}

/// Handle for a request added to an analyzer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(u64);

/// A sound classification request: the classifier's identifier, the window it reads, the overlap between windows and
/// an optional label filter. Invalid numbers are replaced by the defaults rather than rejected.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundRequest {
    classifier_identifier: String,
    window_duration: f64,
    overlap_factor: f64,
    known_classifications: Option<Vec<String>>,
}

impl SoundRequest {
    /// A request with a window of `DEFAULT_WINDOW_SECONDS`, no overlap and no label filter.
    pub fn new(classifier_identifier: &str) -> Self {
        SoundRequest {
            classifier_identifier: classifier_identifier.to_string(),
            window_duration: DEFAULT_WINDOW_SECONDS,
            overlap_factor: 0.0,
            known_classifications: None,
        }
    }

    /// Window length in seconds. A zero, negative or non-finite value restores the default.
    pub fn with_window_duration(mut self, seconds: f64) -> Self {
        self.window_duration = if seconds.is_finite() && seconds > 0.0 {
            seconds
        } else {
            DEFAULT_WINDOW_SECONDS
        };
        self
    }

    /// Fraction of each window that the next window shares, from 0 to `MAX_OVERLAP_FACTOR`. Values outside that range
    /// are clamped; a non-finite value becomes 0.
    pub fn with_overlap_factor(mut self, factor: f64) -> Self {
        self.overlap_factor = if factor.is_finite() {
            factor.clamp(0.0, MAX_OVERLAP_FACTOR)
        } else {
            0.0
        };
        self
    }

    /// Report only these labels. Labels the classifier returns outside this list are dropped from the results.
    pub fn with_known_classifications(mut self, labels: &[&str]) -> Self {
        self.known_classifications = Some(labels.iter().map(|l| l.to_string()).collect());
        self
    }

    pub fn classifier_identifier(&self) -> &str {
        &self.classifier_identifier
    }

    pub fn window_duration(&self) -> f64 {
        self.window_duration
    }

    pub fn overlap_factor(&self) -> f64 {
        self.overlap_factor
    }

    pub fn known_classifications(&self) -> Option<&[String]> {
        self.known_classifications.as_deref()
    }

    /// Window length in frames at `sample_rate`, at least one.
    pub fn window_frames(&self, sample_rate: u32) -> usize {
        ((self.window_duration * sample_rate as f64).round() as usize).max(1)
    }

    /// Frames from the start of one window to the start of the next, at least one. With no overlap this equals the
    /// window length.
    pub fn hop_frames(&self, sample_rate: u32) -> usize {
        let window = self.window_frames(sample_rate) as f64;
        ((window * (1.0 - self.overlap_factor)).round() as usize).max(1)
    }

    fn accepts(&self, label: &str) -> bool {
        match &self.known_classifications {
            Some(list) => list.iter().any(|k| k == label),
            None => true,
        }
    }
}

/// Mixes interleaved frames to mono by averaging the channels of each frame.
fn mono_mix(samples: &[f32], channels: usize) -> Vec<f32> {
    let width = channels.max(1);
    samples
        .chunks_exact(width)
        .map(|frame| frame.iter().sum::<f32>() / width as f32)
        .collect()
}

/// Per-request state: the buffered mono samples and the next window to classify.
struct Active {
    id: RequestId,
    request: SoundRequest,
    classifier: Box<dyn SoundClassifier>,
    observer: Box<dyn SoundResultsObserving>,
    /// Mono samples not yet dropped. `buffer[0]` is frame `origin`.
    buffer: Vec<f32>,
    started: bool,
    origin: u64,
    /// Frame where the next window starts. Never less than `origin`.
    next_start: u64,
    /// Set after a classifier error or `complete_analysis`; a stopped request ignores further input.
    stopped: bool,
}

impl Active {
    fn push(&mut self, mono: &[f32], frame: u64, rate: u32) {
        if !self.started {
            self.started = true;
            self.origin = frame;
            self.next_start = frame;
        }
        self.buffer.extend_from_slice(mono);

        let window = self.request.window_frames(rate) as u64;
        let hop = self.request.hop_frames(rate) as u64;
        loop {
            let buffer_end = self.origin + self.buffer.len() as u64;
            if self.next_start + window > buffer_end {
                break;
            }
            let from = (self.next_start - self.origin) as usize;
            let to = from + window as usize;
            let outcome = self.classifier.classify(&self.buffer[from..to], rate);
            match outcome {
                Ok(labels) => {
                    let range = TimeRange::new(
                        self.next_start as f64 / rate as f64,
                        window as f64 / rate as f64,
                    );
                    let classes: Vec<SoundClassification> = labels
                        .iter()
                        .filter(|(label, _)| self.request.accepts(label))
                        .map(|(label, confidence)| SoundClassification::new(label, *confidence))
                        .collect();
                    let result = SoundClassificationResult::new(range, classes);
                    self.observer.did_produce(&result);
                }
                Err(error) => {
                    self.observer.did_fail(error);
                    self.stopped = true;
                    return;
                }
            }
            self.next_start += hop;
        }

        // Samples before the next window can never be used again, so drop them.
        let drop = (self.next_start - self.origin).min(self.buffer.len() as u64) as usize;
        self.buffer.drain(..drop);
        self.origin += drop as u64;
    }
}

/// Runs sound classification requests over blocks of audio in one fixed format.
pub struct SoundAnalyzer {
    format: AudioFormat,
    requests: Vec<Active>,
    next_id: u64,
}

impl SoundAnalyzer {
    /// An analyzer for blocks in `format`. A zero rate or channel count is raised to 1.
    pub fn new(format: AudioFormat) -> Self {
        SoundAnalyzer {
            format: AudioFormat::new(format.sample_rate, format.channels),
            requests: Vec::new(),
            next_id: 0,
        }
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Adds a request. Its windows are classified by `classifier` and reported to `observer`.
    pub fn add(
        &mut self,
        request: SoundRequest,
        classifier: Box<dyn SoundClassifier>,
        observer: Box<dyn SoundResultsObserving>,
    ) -> RequestId {
        self.next_id += 1;
        let id = RequestId(self.next_id);
        self.requests.push(Active {
            id,
            request,
            classifier,
            observer,
            buffer: Vec::new(),
            started: false,
            origin: 0,
            next_start: 0,
            stopped: false,
        });
        id
    }

    /// Removes one request. Returns false when no request has that id.
    pub fn remove(&mut self, id: RequestId) -> bool {
        let before = self.requests.len();
        self.requests.retain(|a| a.id != id);
        self.requests.len() != before
    }

    pub fn remove_all_requests(&mut self) {
        self.requests.clear();
    }

    pub fn request_count(&self) -> usize {
        self.requests.len()
    }

    /// Adds one block of audio. A block in a different format than the analyzer's is refused: every active request
    /// receives `InvalidFormat` and the block is dropped.
    pub fn analyze(&mut self, input: &AnalyzerInput) {
        if input.format() != self.format {
            for active in self.requests.iter_mut().filter(|a| !a.stopped) {
                active.observer.did_fail(SoundAnalysisError::InvalidFormat);
            }
            return;
        }
        let rate = self.format.sample_rate;
        let mono = mono_mix(input.samples(), self.format.channels as usize);
        let frame = (input.start() * rate as f64).round() as u64;
        for active in self.requests.iter_mut().filter(|a| !a.stopped) {
            active.push(&mono, frame, rate);
        }
    }

    /// Ends the input. Each running request receives `request_did_complete` once and stops; a trailing window shorter
    /// than the request's window is not classified.
    pub fn complete_analysis(&mut self) {
        for active in self.requests.iter_mut().filter(|a| !a.stopped) {
            active.observer.request_did_complete();
            active.stopped = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// What the test observer saw: (start, duration, labels) per result, plus errors and completions.
    #[derive(Default)]
    struct Log {
        results: Vec<(f64, f64, Vec<(String, f64)>)>,
        errors: Vec<SoundAnalysisError>,
        completed: usize,
    }

    struct Observer(Rc<RefCell<Log>>);

    impl SoundResultsObserving for Observer {
        fn did_produce(&mut self, result: &SoundClassificationResult) {
            let labels = result
                .classifications()
                .iter()
                .map(|c| (c.identifier().to_string(), c.confidence()))
                .collect();
            let range = result.time_range();
            self.0
                .borrow_mut()
                .results
                .push((range.start(), range.duration(), labels));
        }

        fn did_fail(&mut self, error: SoundAnalysisError) {
            self.0.borrow_mut().errors.push(error);
        }

        fn request_did_complete(&mut self) {
            self.0.borrow_mut().completed += 1;
        }
    }

    /// Returns the same labels for every window and records the length of each window it is given.
    struct Scripted {
        labels: Vec<(&'static str, f64)>,
        fail: bool,
        windows: Rc<RefCell<Vec<usize>>>,
    }

    impl SoundClassifier for Scripted {
        fn classify(
            &mut self,
            window: &[f32],
            _sample_rate: u32,
        ) -> Result<Vec<(String, f64)>, SoundAnalysisError> {
            self.windows.borrow_mut().push(window.len());
            if self.fail {
                return Err(SoundAnalysisError::OperationFailed);
            }
            Ok(self
                .labels
                .iter()
                .map(|(l, c)| (l.to_string(), *c))
                .collect())
        }
    }

    fn scripted(labels: Vec<(&'static str, f64)>, windows: &Rc<RefCell<Vec<usize>>>) -> Box<Scripted> {
        Box::new(Scripted {
            labels,
            fail: false,
            windows: windows.clone(),
        })
    }

    fn block(start: f64, frames: usize) -> AnalyzerInput {
        AnalyzerInput::new(start, AudioFormat::new(10, 1), &vec![0.0f32; frames])
    }

    #[test]
    fn window_and_hop_frames_follow_overlap() {
        // 3 s at 10 Hz is 30 frames.
        let plain = SoundRequest::new("builtin").with_window_duration(3.0);
        assert_eq!(plain.window_frames(10), 30);
        assert_eq!(plain.hop_frames(10), 30);
        let half = plain.clone().with_overlap_factor(0.5);
        assert_eq!(half.hop_frames(10), 15);
        // 0.99 overlap leaves 0.3 frames between starts, which rounds to 0 and is raised to 1.
        let most = plain.with_overlap_factor(2.0);
        assert_eq!(most.overlap_factor(), 0.99);
        assert_eq!(most.hop_frames(10), 1);
    }

    #[test]
    fn invalid_request_numbers_fall_back() {
        let r = SoundRequest::new("builtin")
            .with_window_duration(-1.0)
            .with_overlap_factor(f64::NAN);
        assert_eq!(r.window_duration(), DEFAULT_WINDOW_SECONDS);
        assert_eq!(r.overlap_factor(), 0.0);
        assert_eq!(r.classifier_identifier(), "builtin");
        assert!(r.known_classifications().is_none());
    }

    #[test]
    fn stereo_blocks_are_mixed_to_mono() {
        let mono = mono_mix(&[1.0, 0.0, 0.5, 0.5], 2);
        assert_eq!(mono, vec![0.5, 0.5]);
    }

    #[test]
    fn windows_without_overlap_report_time_ranges() {
        // Window 1 s (10 frames), no overlap, 25 frames of input: windows start at frames 0 and 10. Frames 20-24 are
        // a trailing partial window and are not classified.
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin").with_window_duration(1.0),
            scripted(vec![("bell", 0.8)], &windows),
            Box::new(Observer(log.clone())),
        );
        analyzer.analyze(&block(0.0, 25));
        analyzer.complete_analysis();

        let log = log.borrow();
        assert_eq!(log.results.len(), 2);
        assert_eq!(log.results[0].0, 0.0);
        assert_eq!(log.results[0].1, 1.0);
        assert_eq!(log.results[1].0, 1.0);
        assert_eq!(log.results[1].1, 1.0);
        assert_eq!(log.results[0].2, vec![("bell".to_string(), 0.8)]);
        assert_eq!(*windows.borrow(), vec![10, 10]);
        assert_eq!(log.completed, 1);
    }

    #[test]
    fn overlap_places_windows_every_half_second() {
        // Window 1 s, overlap 0.5 gives a hop of 5 frames. 20 frames of input hold windows at frames 0, 5 and 10;
        // the window at frame 15 would end at frame 25 and is not complete.
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin")
                .with_window_duration(1.0)
                .with_overlap_factor(0.5),
            scripted(vec![("dog", 0.4)], &windows),
            Box::new(Observer(log.clone())),
        );
        analyzer.analyze(&block(0.0, 20));
        let starts: Vec<f64> = log.borrow().results.iter().map(|r| r.0).collect();
        assert_eq!(starts, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn known_classifications_filter_the_labels() {
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin")
                .with_window_duration(1.0)
                .with_known_classifications(&["bell"]),
            scripted(vec![("bell", 0.8), ("dog", 0.6)], &windows),
            Box::new(Observer(log.clone())),
        );
        analyzer.analyze(&block(0.0, 10));
        let log = log.borrow();
        assert_eq!(log.results.len(), 1);
        assert_eq!(log.results[0].2, vec![("bell".to_string(), 0.8)]);
    }

    #[test]
    fn blocks_join_into_one_window() {
        // Window 1 s (10 frames). The first block of 6 frames is too short; the second block, starting at 0.6 s
        // (frame 6), completes the window at frames 0 to 9.
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin").with_window_duration(1.0),
            scripted(vec![("bell", 0.8)], &windows),
            Box::new(Observer(log.clone())),
        );
        analyzer.analyze(&block(0.0, 6));
        assert!(log.borrow().results.is_empty());
        analyzer.analyze(&block(0.6, 6));
        let log = log.borrow();
        assert_eq!(log.results.len(), 1);
        assert_eq!(log.results[0].0, 0.0);
        assert_eq!(*windows.borrow(), vec![10]);
    }

    #[test]
    fn mismatched_format_is_refused_with_invalid_format() {
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin").with_window_duration(1.0),
            scripted(vec![("bell", 0.8)], &windows),
            Box::new(Observer(log.clone())),
        );
        let wrong = AnalyzerInput::new(0.0, AudioFormat::new(16_000, 1), &[0.0f32; 20]);
        analyzer.analyze(&wrong);
        analyzer.complete_analysis();
        let log = log.borrow();
        assert_eq!(log.errors, vec![SoundAnalysisError::InvalidFormat]);
        assert!(log.results.is_empty());
        assert!(windows.borrow().is_empty());
    }

    #[test]
    fn classifier_error_stops_the_request() {
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        analyzer.add(
            SoundRequest::new("builtin").with_window_duration(1.0),
            Box::new(Scripted {
                labels: vec![],
                fail: true,
                windows: windows.clone(),
            }),
            Box::new(Observer(log.clone())),
        );
        analyzer.analyze(&block(0.0, 20));
        analyzer.complete_analysis();
        let log = log.borrow();
        assert_eq!(log.errors, vec![SoundAnalysisError::OperationFailed]);
        assert_eq!(*windows.borrow(), vec![10]);
        assert_eq!(log.completed, 0);
    }

    #[test]
    fn removed_request_gets_nothing() {
        let log = Rc::new(RefCell::new(Log::default()));
        let windows = Rc::new(RefCell::new(Vec::new()));
        let mut analyzer = SoundAnalyzer::new(AudioFormat::new(10, 1));
        let id = analyzer.add(
            SoundRequest::new("builtin").with_window_duration(1.0),
            scripted(vec![("bell", 0.8)], &windows),
            Box::new(Observer(log.clone())),
        );
        assert_eq!(analyzer.request_count(), 1);
        assert!(analyzer.remove(id));
        assert!(!analyzer.remove(id));
        analyzer.analyze(&block(0.0, 20));
        assert!(log.borrow().results.is_empty());
        assert_eq!(analyzer.request_count(), 0);
    }

    #[test]
    fn error_codes_have_official_names() {
        assert_eq!(SoundAnalysisError::InvalidFile.code_name(), "invalidFile");
        assert_eq!(SoundAnalysisError::UnknownError.code_name(), "unknownError");
        assert_eq!(
            SoundAnalysisError::InvalidFormat.to_string(),
            "the audio data's format is not valid"
        );
    }
}
