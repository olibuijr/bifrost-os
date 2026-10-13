//! `SFSpeechRecognizer` and its requests: speech to text over a pluggable [`RecognitionBackend`].
//!
//! BifrOSt has no on-device recognizer yet. This module owns the Apple-shaped surface: authorization, availability,
//! requests, tasks and results with segments. A backend turns time-coded audio ([`AnalyzerInput`]) into hypotheses.
//! The task is driven by the caller: [`SFSpeechRecognitionTask::pump`] drains the audio appended so far, so there is
//! no thread and no hidden clock. Recognition never starts before the person allowed it.
//!
//! ```ignore
//! let mut r = SFSpeechRecognizer::new("en-US", Box::new(backend));
//! r.request_authorization(true);
//! let req = SFSpeechAudioBufferRecognitionRequest::new();
//! let mut task = r.recognition_task(&req, Box::new(|res, err| { /* ... */ }))?;
//! req.append(input); task.pump(); req.end_audio(); task.pump();
//! ```

use bifrost_ui::analyzer_input::AnalyzerInput;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// `SFSpeechRecognizerAuthorizationStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SFSpeechRecognizerAuthorizationStatus {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
}

/// Why recognition did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechError {
    NotAuthorized,
    Unavailable,
    Canceled,
    Backend(String),
}

/// One recognised word with its place in the audio, in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct SFTranscriptionSegment {
    pub substring: String,
    pub timestamp: f64,
    pub duration: f64,
    pub confidence: f32,
}

/// A whole hypothesis: the text and its segments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SFTranscription {
    pub segments: Vec<SFTranscriptionSegment>,
}

impl SFTranscription {
    /// `formattedString`: the segments joined by single spaces.
    pub fn formatted_string(&self) -> String {
        self.segments.iter().map(|s| s.substring.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// `SFSpeechRecognitionResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct SFSpeechRecognitionResult {
    pub best_transcription: SFTranscription,
    pub transcriptions: Vec<SFTranscription>,
    pub is_final: bool,
}

/// Turns audio into hypotheses. Implement it over whatever engine the system provides.
pub trait RecognitionBackend {
    /// Whether the engine can serve `locale` now.
    fn is_available(&self, locale: &str) -> bool;
    /// Feeds the next audio and returns the best partial transcription so far, if it changed.
    fn accept(&mut self, input: &AnalyzerInput) -> Option<SFTranscription>;
    /// Ends the audio and returns the final transcription.
    fn finish(&mut self) -> Result<SFTranscription, String>;
    /// Transcribes a whole file in one go.
    fn transcribe_file(&mut self, path: &str) -> Result<SFTranscription, String>;
}

type Handler = Box<dyn FnMut(Option<&SFSpeechRecognitionResult>, Option<&SpeechError>)>;

#[derive(Default)]
struct Queue {
    inputs: VecDeque<AnalyzerInput>,
    ended: bool,
}

/// `SFSpeechAudioBufferRecognitionRequest`: audio appended as it arrives.
#[derive(Clone, Default)]
pub struct SFSpeechAudioBufferRecognitionRequest {
    queue: Rc<RefCell<Queue>>,
    pub should_report_partial_results: bool,
}

impl SFSpeechAudioBufferRecognitionRequest {
    /// A request that reports partial results, as Apple's default does.
    pub fn new() -> Self {
        Self { queue: Rc::default(), should_report_partial_results: true }
    }
    pub fn append(&self, input: AnalyzerInput) {
        let mut q = self.queue.borrow_mut();
        if !q.ended {
            q.inputs.push_back(input);
        }
    }
    pub fn end_audio(&self) {
        self.queue.borrow_mut().ended = true;
    }
}

/// `SFSpeechURLRecognitionRequest`: a recorded file.
#[derive(Clone, Debug, PartialEq)]
pub struct SFSpeechURLRecognitionRequest {
    pub url: String,
    pub should_report_partial_results: bool,
}

impl SFSpeechURLRecognitionRequest {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_string(), should_report_partial_results: false }
    }
}

/// `SFSpeechRecognitionTaskState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SFSpeechRecognitionTaskState {
    Starting,
    Running,
    Finishing,
    Canceling,
    Completed,
}

enum Source {
    Buffer(SFSpeechAudioBufferRecognitionRequest),
    File(String),
}

/// `SFSpeechRecognitionTask`.
pub struct SFSpeechRecognitionTask {
    backend: Rc<RefCell<Box<dyn RecognitionBackend>>>,
    source: Source,
    handler: Handler,
    state: SFSpeechRecognitionTaskState,
}

impl SFSpeechRecognitionTask {
    pub fn state(&self) -> SFSpeechRecognitionTaskState {
        self.state
    }
    pub fn is_finishing(&self) -> bool {
        self.state == SFSpeechRecognitionTaskState::Finishing
    }
    pub fn is_cancelled(&self) -> bool {
        self.state == SFSpeechRecognitionTaskState::Canceling
    }

    /// Stops the task. The handler gets `Canceled` once and nothing after.
    pub fn cancel(&mut self) {
        if self.state == SFSpeechRecognitionTaskState::Completed || self.is_cancelled() {
            return;
        }
        self.state = SFSpeechRecognitionTaskState::Canceling;
        (self.handler)(None, Some(&SpeechError::Canceled));
    }

    /// Asks for the final result of the audio so far; the audio ends.
    pub fn finish(&mut self) {
        if let Source::Buffer(r) = &self.source {
            r.end_audio();
        }
    }

    /// Drains the appended audio into the backend and delivers results to the handler.
    pub fn pump(&mut self) {
        if matches!(self.state, SFSpeechRecognitionTaskState::Completed | SFSpeechRecognitionTaskState::Canceling) {
            return;
        }
        self.state = SFSpeechRecognitionTaskState::Running;
        match &self.source {
            Source::File(path) => {
                let out = self.backend.borrow_mut().transcribe_file(path);
                self.complete(out);
            }
            Source::Buffer(req) => {
                let (inputs, ended): (Vec<_>, bool) = {
                    let mut q = req.queue.borrow_mut();
                    (q.inputs.drain(..).collect(), q.ended)
                };
                let report = req.should_report_partial_results;
                for input in inputs {
                    let partial = self.backend.borrow_mut().accept(&input);
                    if let (Some(t), true) = (partial, report) {
                        let result = SFSpeechRecognitionResult {
                            best_transcription: t.clone(),
                            transcriptions: vec![t],
                            is_final: false,
                        };
                        (self.handler)(Some(&result), None);
                    }
                }
                if ended {
                    self.state = SFSpeechRecognitionTaskState::Finishing;
                    let out = self.backend.borrow_mut().finish();
                    self.complete(out);
                }
            }
        }
    }

    fn complete(&mut self, out: Result<SFTranscription, String>) {
        self.state = SFSpeechRecognitionTaskState::Completed;
        match out {
            Ok(t) => {
                let result =
                    SFSpeechRecognitionResult { best_transcription: t.clone(), transcriptions: vec![t], is_final: true };
                (self.handler)(Some(&result), None);
            }
            Err(e) => (self.handler)(None, Some(&SpeechError::Backend(e))),
        }
    }
}

/// `SFSpeechRecognizer`.
pub struct SFSpeechRecognizer {
    locale: String,
    status: SFSpeechRecognizerAuthorizationStatus,
    backend: Rc<RefCell<Box<dyn RecognitionBackend>>>,
}

impl SFSpeechRecognizer {
    pub fn new(locale: &str, backend: Box<dyn RecognitionBackend>) -> Self {
        Self {
            locale: locale.to_string(),
            status: SFSpeechRecognizerAuthorizationStatus::NotDetermined,
            backend: Rc::new(RefCell::new(backend)),
        }
    }
    pub fn locale(&self) -> &str {
        &self.locale
    }
    pub fn authorization_status(&self) -> SFSpeechRecognizerAuthorizationStatus {
        self.status
    }
    /// `requestAuthorization`: records the person's answer once; later calls keep it.
    pub fn request_authorization(&mut self, person_allows: bool) -> SFSpeechRecognizerAuthorizationStatus {
        if self.status == SFSpeechRecognizerAuthorizationStatus::NotDetermined {
            self.status = if person_allows {
                SFSpeechRecognizerAuthorizationStatus::Authorized
            } else {
                SFSpeechRecognizerAuthorizationStatus::Denied
            };
        }
        self.status
    }
    pub fn is_available(&self) -> bool {
        self.backend.borrow().is_available(&self.locale)
    }

    fn start(&self, source: Source, handler: Handler) -> Result<SFSpeechRecognitionTask, SpeechError> {
        if self.status != SFSpeechRecognizerAuthorizationStatus::Authorized {
            return Err(SpeechError::NotAuthorized);
        }
        if !self.is_available() {
            return Err(SpeechError::Unavailable);
        }
        Ok(SFSpeechRecognitionTask {
            backend: self.backend.clone(),
            source,
            handler,
            state: SFSpeechRecognitionTaskState::Starting,
        })
    }

    /// `recognitionTask(with:resultHandler:)` for live audio.
    pub fn recognition_task(
        &self,
        request: &SFSpeechAudioBufferRecognitionRequest,
        handler: Handler,
    ) -> Result<SFSpeechRecognitionTask, SpeechError> {
        self.start(Source::Buffer(request.clone()), handler)
    }

    /// `recognitionTask(with:resultHandler:)` for a file.
    pub fn recognition_task_url(
        &self,
        request: &SFSpeechURLRecognitionRequest,
        handler: Handler,
    ) -> Result<SFSpeechRecognitionTask, SpeechError> {
        self.start(Source::File(request.url.clone()), handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::analyzer_input::ANALYSIS_FORMAT;

    struct Fake {
        available: bool,
        words: Vec<String>,
    }
    fn seg(w: &str, i: usize) -> SFTranscriptionSegment {
        SFTranscriptionSegment { substring: w.to_string(), timestamp: i as f64 * 0.5, duration: 0.5, confidence: 0.9 }
    }
    fn tr(words: &[String]) -> SFTranscription {
        SFTranscription { segments: words.iter().enumerate().map(|(i, w)| seg(w, i)).collect() }
    }
    impl RecognitionBackend for Fake {
        fn is_available(&self, _: &str) -> bool {
            self.available
        }
        fn accept(&mut self, _: &AnalyzerInput) -> Option<SFTranscription> {
            self.words.push(format!("w{}", self.words.len()));
            Some(tr(&self.words))
        }
        fn finish(&mut self) -> Result<SFTranscription, String> {
            Ok(tr(&self.words))
        }
        fn transcribe_file(&mut self, path: &str) -> Result<SFTranscription, String> {
            if path.ends_with(".wav") {
                Ok(tr(&["hello".to_string(), "world".to_string()]))
            } else {
                Err("bad file".into())
            }
        }
    }

    fn recognizer(available: bool) -> SFSpeechRecognizer {
        SFSpeechRecognizer::new("en-US", Box::new(Fake { available, words: vec![] }))
    }
    fn input() -> AnalyzerInput {
        AnalyzerInput::new(0.0, ANALYSIS_FORMAT, &[0.0; 160])
    }
    type Log = Rc<RefCell<Vec<(Option<SFSpeechRecognitionResult>, Option<SpeechError>)>>>;
    fn logger(log: &Log) -> Handler {
        let log = log.clone();
        Box::new(move |r, e| log.borrow_mut().push((r.cloned(), e.cloned())))
    }

    #[test]
    fn authorization_starts_undetermined_and_keeps_the_first_answer() {
        let mut r = recognizer(true);
        assert_eq!(r.authorization_status(), SFSpeechRecognizerAuthorizationStatus::NotDetermined);
        assert_eq!(r.request_authorization(false), SFSpeechRecognizerAuthorizationStatus::Denied);
        assert_eq!(r.request_authorization(true), SFSpeechRecognizerAuthorizationStatus::Denied);
    }

    #[test]
    fn a_task_needs_authorization_and_availability() {
        let log = Log::default();
        let req = SFSpeechAudioBufferRecognitionRequest::new();
        let mut r = recognizer(false);
        assert!(matches!(r.recognition_task(&req, logger(&log)), Err(SpeechError::NotAuthorized)));
        r.request_authorization(true);
        assert!(!r.is_available());
        assert!(matches!(r.recognition_task(&req, logger(&log)), Err(SpeechError::Unavailable)));
    }

    #[test]
    fn partials_then_one_final_result() {
        let mut r = recognizer(true);
        r.request_authorization(true);
        let log = Log::default();
        let req = SFSpeechAudioBufferRecognitionRequest::new();
        let mut task = r.recognition_task(&req, logger(&log)).unwrap();
        req.append(input());
        req.append(input());
        task.pump();
        assert_eq!(task.state(), SFSpeechRecognitionTaskState::Running);
        req.end_audio();
        task.pump();
        let log = log.borrow();
        let finals: Vec<bool> = log.iter().map(|(r, _)| r.as_ref().unwrap().is_final).collect();
        assert_eq!(finals, vec![false, false, true]);
        assert_eq!(log[2].0.as_ref().unwrap().best_transcription.formatted_string(), "w0 w1");
        assert_eq!(task.state(), SFSpeechRecognitionTaskState::Completed);
    }

    #[test]
    fn partial_results_can_be_switched_off() {
        let mut r = recognizer(true);
        r.request_authorization(true);
        let log = Log::default();
        let mut req = SFSpeechAudioBufferRecognitionRequest::new();
        req.should_report_partial_results = false;
        let mut task = r.recognition_task(&req, logger(&log)).unwrap();
        req.append(input());
        req.end_audio();
        task.pump();
        assert_eq!(log.borrow().len(), 1);
        assert!(log.borrow()[0].0.as_ref().unwrap().is_final);
    }

    #[test]
    fn segments_carry_timing() {
        let t = tr(&["a".to_string(), "b".to_string()]);
        assert_eq!(t.segments[1].timestamp, 0.5);
        assert_eq!(t.segments[1].duration, 0.5);
        assert_eq!(t.formatted_string(), "a b");
    }

    #[test]
    fn cancel_reports_once_and_silences_the_task() {
        let mut r = recognizer(true);
        r.request_authorization(true);
        let log = Log::default();
        let req = SFSpeechAudioBufferRecognitionRequest::new();
        let mut task = r.recognition_task(&req, logger(&log)).unwrap();
        task.cancel();
        task.cancel();
        req.append(input());
        req.end_audio();
        task.pump();
        assert!(task.is_cancelled());
        assert_eq!(log.borrow().len(), 1);
        assert_eq!(log.borrow()[0].1, Some(SpeechError::Canceled));
    }

    #[test]
    fn a_file_request_gives_one_final_result_or_an_error() {
        let mut r = recognizer(true);
        r.request_authorization(true);
        let log = Log::default();
        let mut ok = r.recognition_task_url(&SFSpeechURLRecognitionRequest::new("a.wav"), logger(&log)).unwrap();
        ok.pump();
        let mut bad = r.recognition_task_url(&SFSpeechURLRecognitionRequest::new("a.txt"), logger(&log)).unwrap();
        bad.pump();
        let log = log.borrow();
        assert_eq!(log[0].0.as_ref().unwrap().best_transcription.formatted_string(), "hello world");
        assert_eq!(log[1].1, Some(SpeechError::Backend("bad file".into())));
    }
}
