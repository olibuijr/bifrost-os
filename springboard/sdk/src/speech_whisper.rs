//! On-device live recognition for apps, through the shell's `speech_recognizer` service (see
//! `springboard/src/shell/services/recognizer.rs`). The shell captures the microphone and runs the whisper element, so
//! no audio leaves the machine. The recognition types an app uses are the same as for the reference recognizer:
//! [`SFSpeechAudioBufferRecognitionRequest`] and [`SFSpeechRecognitionResult`] from [`speech_recognizer`].
//!
//! ```ignore
//! speech_whisper::start(ctx, &request);          // the reply is {id}; read it with App::service_reply
//! speech_whisper::poll(ctx);                     // the reply holds the transcripts since the last poll
//! let reply = speech_whisper::poll_from_reply(&result)?;
//! speech_whisper::stop(ctx);
//! ```
//!
//! Consent comes first: the model is downloaded only after the person grants it in Settings, and [`model`] reports
//! whether it is there. Without consent or the model the shell refuses to start. [`WhisperBackend`] plugs the same
//! transcripts into `SFSpeechRecognizer` for code that already takes a backend.
//!
//! Gaps: the whisper element reports no per-word timing or confidence, so the segments carry zero timestamps and
//! confidence; other languages need the multilingual model; file recognition is not served; and the reply format of the element's
//! messages is checked on the lab machine.

use crate::runtime::Ctx;
use crate::service;
use crate::speech_recognizer::{
    RecognitionBackend, SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFTranscription,
    SFTranscriptionSegment,
};
use bifrost_core::json::Json;
use bifrost_ui::analyzer_input::AnalyzerInput;
use bifrost_ui::whisper::{locale_supported, PollReply, Transcript};

/// The service name, as the shell registers it.
pub const SERVICE: &str = "speech_recognizer";

fn call(ctx: &Ctx, method: &str, params: &Json) -> u32 {
    service::call(ctx, SERVICE, method, params)
}

/// The `start` parameters for a request: the locale and whether partial results are wanted.
pub fn start_params(locale: &str, report_partial: bool) -> Json {
    Json::Obj(vec![
        ("locale".to_string(), Json::Str(locale.to_string())),
        ("partial".to_string(), Json::Bool(report_partial)),
    ])
}

/// Starts live recognition for this app with the request's options. The reply is `{id}`, or an error when the
/// person has not granted consent, the model is not installed, or no model serves the locale.
pub fn start(ctx: &Ctx, locale: &str, request: &SFSpeechAudioBufferRecognitionRequest) -> u32 {
    call(ctx, "start", &start_params(locale, request.should_report_partial_results))
}

/// Asks for the transcripts gathered since the last poll. The reply is read with [`poll_from_reply`].
pub fn poll(ctx: &Ctx) -> u32 {
    call(ctx, "poll", &Json::Null)
}

/// Ends this app's recognition. The shell keeps the final transcripts for one more poll.
pub fn stop(ctx: &Ctx) -> u32 {
    call(ctx, "stop", &Json::Null)
}

/// Asks for the model state. The reply is read with [`model_from_reply`].
pub fn model(ctx: &Ctx) -> u32 {
    call(ctx, "model", &Json::Null)
}

/// Records the person's answer in Settings. Granting starts nothing by itself; [`download`] fetches the model.
pub fn set_consent(ctx: &Ctx, granted: bool) -> u32 {
    call(ctx, "consent", &Json::Obj(vec![("granted".to_string(), Json::Bool(granted))]))
}

/// Downloads the model. Refused without consent. The reply is `{started}`; check [`model`] for progress.
pub fn download(ctx: &Ctx) -> u32 {
    call(ctx, "download", &Json::Null)
}

/// The model state: whether consent is granted, whether the model is installed and downloading, and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelStatus {
    pub granted: bool,
    pub installed: bool,
    pub downloading: bool,
    pub size_bytes: u64,
}

fn body(result: &Result<Json, String>) -> Result<&Json, String> {
    result.as_ref().map_err(|e| e.clone())
}

fn transcript_of(v: &Json) -> Option<Transcript> {
    let text = v.str_field("text").filter(|t| !t.is_empty())?;
    let is_final = matches!(v.get("final"), Some(Json::Bool(true)));
    Some(Transcript { text, is_final })
}

/// Reads the transcripts and the running flag from a `poll` reply. Entries without text are skipped.
pub fn poll_from_reply(result: &Result<Json, String>) -> Result<PollReply, String> {
    let b = body(result)?;
    let list = b.get("transcripts").and_then(Json::as_array).ok_or("speech recognizer: the reply has no transcripts")?;
    let running = matches!(b.get("running"), Some(Json::Bool(true)));
    Ok(PollReply { transcripts: list.iter().filter_map(transcript_of).collect(), running })
}

/// Reads the model state from a `model` reply.
pub fn model_from_reply(result: &Result<Json, String>) -> Result<ModelStatus, String> {
    let b = body(result)?;
    let flag = |key: &str| matches!(b.get(key), Some(Json::Bool(true)));
    let size_bytes = b.get("size_bytes").and_then(Json::as_u64).unwrap_or(0);
    if b.get("granted").is_none() {
        return Err("speech recognizer: the reply has no consent flag".to_string());
    }
    Ok(ModelStatus { granted: flag("granted"), installed: flag("installed"), downloading: flag("downloading"), size_bytes })
}

/// The transcription of one hypothesis: its words as segments. Timing and confidence are unknown, so they are zero.
pub fn transcription(text: &str) -> SFTranscription {
    SFTranscription {
        segments: text
            .split_whitespace()
            .map(|w| SFTranscriptionSegment { substring: w.to_string(), timestamp: 0.0, duration: 0.0, confidence: 0.0 })
            .collect(),
    }
}

/// The recognition result for one transcript, the shape `SFSpeechRecognizer` hands to a result handler.
pub fn result(t: &Transcript) -> SFSpeechRecognitionResult {
    let best = transcription(&t.text);
    SFSpeechRecognitionResult { best_transcription: best.clone(), transcriptions: vec![best], is_final: t.is_final }
}

/// Where the transcripts come from. The SDK client implements it over the service; tests implement it over a list.
pub trait TranscriptSource {
    /// The transcripts gathered since the last call.
    fn poll(&mut self) -> PollReply;
    /// Ends the recognition and returns what was still pending.
    fn stop(&mut self) -> Vec<Transcript>;
}

/// A `RecognitionBackend` over a [`TranscriptSource`]. The audio is captured by the shell, so `accept` ignores the
/// buffers it is given and only reads the transcripts that have arrived.
pub struct WhisperBackend {
    source: Box<dyn TranscriptSource>,
    committed: Vec<String>,
    pending: Option<String>,
}

impl WhisperBackend {
    pub fn new(source: Box<dyn TranscriptSource>) -> Self {
        Self { source, committed: Vec::new(), pending: None }
    }

    fn apply(&mut self, t: Transcript) {
        if t.is_final {
            self.committed.push(t.text);
            self.pending = None;
        } else {
            self.pending = Some(t.text);
        }
    }

    fn text(&self) -> String {
        self.committed.iter().map(String::as_str).chain(self.pending.as_deref()).collect::<Vec<_>>().join(" ")
    }
}

impl RecognitionBackend for WhisperBackend {
    fn is_available(&self, locale: &str) -> bool {
        locale_supported(locale)
    }

    fn accept(&mut self, _input: &AnalyzerInput) -> Option<SFTranscription> {
        let reply = self.source.poll();
        if reply.transcripts.is_empty() {
            return None;
        }
        for t in reply.transcripts {
            self.apply(t);
        }
        Some(transcription(&self.text()))
    }

    fn finish(&mut self) -> Result<SFTranscription, String> {
        for t in self.source.stop() {
            self.apply(t);
        }
        if let Some(p) = self.pending.take() {
            self.committed.push(p);
        }
        if self.committed.is_empty() {
            return Err("no speech was recognized".to_string());
        }
        Ok(transcription(&self.text()))
    }

    fn transcribe_file(&mut self, _path: &str) -> Result<SFTranscription, String> {
        Err("the whisper pipeline does not recognize recorded files yet".to_string())
    }
}

#[cfg(test)]
mod tests {
    use bifrost_ui::analyzer_input::AudioFormat;
    use super::*;

    fn t(text: &str, is_final: bool) -> Transcript {
        Transcript { text: text.to_string(), is_final }
    }

    #[test]
    fn the_start_request_maps_locale_and_partial_results() {
        let p = start_params("en_US", false);
        assert_eq!(p.get("locale").and_then(Json::as_str), Some("en_US"));
        assert_eq!(p.get("partial"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_poll_reply_maps_to_transcripts_and_the_running_flag() {
        let reply = Ok(Json::Obj(vec![
            (
                "transcripts".to_string(),
                Json::Arr(vec![
                    Json::Obj(vec![("text".to_string(), Json::Str("hello".into())), ("final".to_string(), Json::Bool(true))]),
                    Json::Obj(vec![("text".to_string(), Json::Str("".into()))]),
                    Json::Obj(vec![("text".to_string(), Json::Str("wor".into()))]),
                ]),
            ),
            ("running".to_string(), Json::Bool(true)),
        ]));
        let got = poll_from_reply(&reply).unwrap();
        assert!(got.running);
        assert_eq!(got.transcripts, vec![t("hello", true), t("wor", false)]);
    }

    #[test]
    fn a_poll_error_is_passed_on_and_a_reply_without_transcripts_is_refused() {
        assert_eq!(poll_from_reply(&Err("refused".to_string())), Err("refused".to_string()));
        assert!(poll_from_reply(&Ok(Json::Obj(vec![]))).is_err());
    }

    #[test]
    fn a_model_reply_maps_consent_install_and_size() {
        let reply = Ok(Json::Obj(vec![
            ("granted".to_string(), Json::Bool(true)),
            ("installed".to_string(), Json::Bool(false)),
            ("downloading".to_string(), Json::Bool(true)),
            ("size_bytes".to_string(), Json::Num(147_964_211.0)),
        ]));
        assert_eq!(
            model_from_reply(&reply),
            Ok(ModelStatus { granted: true, installed: false, downloading: true, size_bytes: 147_964_211 })
        );
        assert!(model_from_reply(&Ok(Json::Obj(vec![]))).is_err());
    }

    #[test]
    fn a_result_marks_its_finality_and_splits_words_into_segments() {
        let r = result(&t("good morning", true));
        assert!(r.is_final);
        assert_eq!(r.best_transcription.formatted_string(), "good morning");
        assert_eq!(r.best_transcription.segments.len(), 2);
        assert!(!result(&t("good", false)).is_final);
    }

    struct Fake {
        polls: Vec<PollReply>,
        rest: Vec<Transcript>,
    }

    impl TranscriptSource for Fake {
        fn poll(&mut self) -> PollReply {
            if self.polls.is_empty() {
                PollReply { transcripts: vec![], running: true }
            } else {
                self.polls.remove(0)
            }
        }
        fn stop(&mut self) -> Vec<Transcript> {
            std::mem::take(&mut self.rest)
        }
    }

    fn backend(polls: Vec<PollReply>, rest: Vec<Transcript>) -> WhisperBackend {
        WhisperBackend::new(Box::new(Fake { polls, rest }))
    }

    fn buffer() -> AnalyzerInput {
        AnalyzerInput::new(0.0, AudioFormat::new(16_000, 1), &[0.0; 16])
    }

    #[test]
    fn partial_text_is_reported_and_finals_are_joined_on_finish() {
        let mut b = backend(
            vec![
                PollReply { transcripts: vec![t("turn on", true)], running: true },
                PollReply { transcripts: vec![t("the lamp", false)], running: true },
            ],
            vec![t("the lamp please", true)],
        );
        assert_eq!(b.accept(&buffer()).unwrap().formatted_string(), "turn on");
        assert_eq!(b.accept(&buffer()).unwrap().formatted_string(), "turn on the lamp");
        assert!(b.accept(&buffer()).is_none(), "an empty poll reports nothing new");
        assert_eq!(b.finish().unwrap().formatted_string(), "turn on the lamp please");
    }

    #[test]
    fn finishing_with_nothing_recognized_is_an_error() {
        let mut b = backend(vec![], vec![]);
        assert_eq!(b.finish(), Err("no speech was recognized".to_string()));
    }

    #[test]
    fn whisper_languages_are_available_and_files_are_not_served() {
        let mut b = backend(vec![], vec![]);
        assert!(b.is_available("en_GB"));
        assert!(b.is_available("is_IS"));
        assert!(!b.is_available("tlh"));
        assert!(b.transcribe_file("/tmp/a.wav").is_err());
    }

    #[test]
    fn a_buffer_request_reaches_the_recognizer_with_its_partial_setting() {
        let req = SFSpeechAudioBufferRecognitionRequest::new();
        assert!(req.should_report_partial_results);
        assert_eq!(start_params("en", req.should_report_partial_results).get("partial"), Some(&Json::Bool(true)));
    }
}
