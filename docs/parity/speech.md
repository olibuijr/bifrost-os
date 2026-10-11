# Speech: recognition models (wave 2)

Status: partial. Pure Rust models for the recognition side of the Speech framework. The speech-to-text engine,
the microphone stream to apps and the on-device model install are not built.

## What was built

- `springboard/ui/src/recognition.rs`: `Alternative` (text plus a clamped 0 to 1 confidence), `Transcription`
  (time range in seconds, final flag, ranked alternatives, blank readings dropped), `ContextualStrings` (phrases
  grouped by tag, whole-word case-insensitive matching that adds `CONTEXT_BOOST` to a reading and re-ranks it),
  and `Session` (a partial result replaces the partial before it; a final result is kept; `final_text()` and
  `text()`).
- `springboard/ui/src/voice_activity.rs`: `SpeechDetector` over mono f32 frames. Sensitivity levels Low, Medium
  and High set the RMS threshold (-35, -45, -55 dBFS). Speech opens after `ONSET_FRAMES` (2) loud frames and
  closes after `HANGOVER_FRAMES` (5) quiet frames. Events report the first frame of each run, in seconds.
- `springboard/ui/src/analyzer_input.rs`: `AnalyzerInput` (start time, format, interleaved samples, whole frames
  only), `AudioFormat`, `ANALYSIS_FORMAT` (16 kHz mono) and `AnalyzerInputConverter` (channel averaging, then
  linear resampling; the start time is kept).

`springboard/ui/src/speech.rs` is unchanged; it is the synthesis model from wave 1.

## Official API names mirrored

- `SpeechDetector`, `SpeechDetector.SensitivityLevel`, `SpeechDetector.DetectionOptions`
  (https://developer.apple.com/documentation/speech/speechdetector): `voice_activity.rs` covers voice activity
  detection. The VAD runs on sample frames the caller supplies, not through an analyzer module protocol.
- `AnalyzerInput` (https://developer.apple.com/documentation/speech/analyzerinput): `bufferStartTime`,
  `bufferDuration` and `buffer` map to `start()`, `duration()` and `samples()`.
- `AnalyzerInputConverter` (https://developer.apple.com/documentation/speech): the target format is fixed to mono.
- `SpeechModuleResult` (https://developer.apple.com/documentation/speech/speechmoduleresult): `range` and `isFinal`
  map to `Transcription::range` and `Transcription::is_final`. `resultsFinalizationTime` is not mirrored.
- `AnalysisContext.contextualStrings` (https://developer.apple.com/documentation/speech/analysiscontext):
  `ContextualStrings::add` and `phrases()`. `userData` is not mirrored.

## REA evidence

- `ev_8f86d9c19f7bf478511e031971a400f47fbda75b676cb2eb5ad3c80b4623021e`: rendered page
  https://developer.apple.com/documentation/speech (run in this wave with `tools/parity/rea-page.sh`).
- `ev_83d419f1903e2491c4d2f87ecace3e8a120a051be00d72c9640c0513b97e4722`: the matrix row's evidence from wave 1.

## Matrix rows closed (for the lead to update centrally)

| framework | feature (as in data/speech.json) | new status | BifrOSt path |
|---|---|---|---|
| Speech | Analyzer modules and voice activity detection | partial | springboard/ui/src/voice_activity.rs |
| Speech | Audio input for analysis (file and microphone sequence providers, converter) | partial | springboard/ui/src/analyzer_input.rs |
| Speech | Recognition results (alternatives, confidence, time ranges) | partial | springboard/ui/src/recognition.rs |
| Speech | Custom vocabulary and language models | partial | springboard/ui/src/recognition.rs (ContextualStrings) |

## Still missing

- No recognizer engine. Nothing turns audio into the readings that `Session` stores.
- No SDK API streams time-coded microphone audio to an app. Capture stays WAV to a path through the audio service.
- No module protocol, no file or microphone input sequence providers, and no custom language model data.
- No asset inventory or install path for speech models.
- The keyboard Mic key remains inert (`springboard/apps/keyboard/src/keyboard.rs`).
