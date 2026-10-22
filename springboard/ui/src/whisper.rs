//! On-device speech recognition through the GStreamer `whisper` element (`gst-plugin-whisper`, part of gst-plugins-rs).
//!
//! Audio never leaves the machine: the shell runs a `gst-launch-1.0` pipeline that captures from PulseAudio or PipeWire,
//! converts it to 16 kHz mono float, and feeds the `whisper` element with a local model file. The element's transcript
//! arrives as bus messages that gst-launch prints on its standard output; [`parse_transcript_line`] reads them.
//!
//! The model is downloaded only after the person grants consent in Settings. [`Consent`] is that gate: no download and
//! no recording start without a grant. This module is pure: it builds argv, parses lines and decides the gate. The
//! shell service spawns the process and the download; the SDK maps the transcripts to recognition results.

use std::path::{Path, PathBuf};

/// The model file the whisper element loads. English-only base model, a ggml file from the whisper.cpp project.
pub const MODEL_FILE: &str = "ggml-base.en.bin";
/// Where the model is fetched from when the person has granted consent.
pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
/// The size of [`MODEL_FILE`] in bytes, shown to the person before consent.
pub const MODEL_SIZE_BYTES: u64 = 147_964_211;
/// The multilingual base model: the same size class as [`MODEL_FILE`], but it recognises the languages in
/// [`WHISPER_LANGUAGES`]. It is fetched only when a locale other than English is asked for.
pub const MULTILINGUAL_MODEL_FILE: &str = "ggml-base.bin";
/// Where [`MULTILINGUAL_MODEL_FILE`] is fetched from when the person has granted consent.
pub const MULTILINGUAL_MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
/// The size of [`MULTILINGUAL_MODEL_FILE`] in bytes, shown to the person before consent.
pub const MULTILINGUAL_MODEL_SIZE_BYTES: u64 = 147_951_465;
/// The program that runs the pipeline.
pub const GST_LAUNCH: &str = "gst-launch-1.0";
/// The sample rate the whisper element expects.
pub const SAMPLE_RATE: u32 = 16_000;

/// The directory under the user's data dir that holds the model and the consent record.
pub fn data_subdir(data_dir: &Path) -> PathBuf {
    data_dir.join("speech")
}

/// Where the model file lives for a given user data directory.
pub fn model_path(data_dir: &Path) -> PathBuf {
    data_subdir(data_dir).join(MODEL_FILE)
}

/// A speech model the shell can fetch: its file name under the speech directory, where it comes from and how big it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub file: &'static str,
    pub url: &'static str,
    pub size_bytes: u64,
}

/// The English-only base model.
pub const ENGLISH_MODEL: Model = Model { file: MODEL_FILE, url: MODEL_URL, size_bytes: MODEL_SIZE_BYTES };
/// The multilingual base model.
pub const MULTILINGUAL_MODEL: Model =
    Model { file: MULTILINGUAL_MODEL_FILE, url: MULTILINGUAL_MODEL_URL, size_bytes: MULTILINGUAL_MODEL_SIZE_BYTES };

/// The languages the multilingual whisper models recognise, as two or three letter ISO 639 codes.
pub const WHISPER_LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he",
    "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa",
    "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa",
    "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn",
    "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su",
];

/// The whisper language code for a locale such as `is_IS` or `en-GB`, or `None` when the models do not recognise it.
pub fn whisper_language(locale: &str) -> Option<&'static str> {
    let code = crate::translation::language_code(locale);
    WHISPER_LANGUAGES.iter().copied().find(|l| *l == code)
}

/// The model that serves `locale`: the English model for English, the multilingual one for any other language the
/// whisper models know, and `None` for a language they do not.
pub fn model_for_locale(locale: &str) -> Option<Model> {
    match whisper_language(locale)? {
        "en" => Some(ENGLISH_MODEL),
        _ => Some(MULTILINGUAL_MODEL),
    }
}

/// Where `model` lives for a given user data directory.
pub fn model_file_path(data_dir: &Path, model: Model) -> PathBuf {
    data_subdir(data_dir).join(model.file)
}

/// Where the consent record lives: a file that holds the single line `granted` once the person agrees.
pub fn consent_path(data_dir: &Path) -> PathBuf {
    data_subdir(data_dir).join("consent")
}

/// The audio source element the pipeline captures from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    PulseAudio,
    PipeWire,
}

impl Capture {
    /// The GStreamer source element for this capture backend.
    pub fn element(self) -> &'static str {
        match self {
            Capture::PulseAudio => "pulsesrc",
            Capture::PipeWire => "pipewiresrc",
        }
    }
}

/// What the pipeline listens to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// the default microphone: dictation and voice control
    #[default]
    Microphone,
    /// the sound the computer itself plays, taken from the default output's monitor: Live Captions
    SystemAudio,
}

impl Capture {
    /// The source element and its properties for `source`, one argv entry each. The microphone is the element alone.
    /// System audio is the monitor of the default output: PulseAudio names it `@DEFAULT_MONITOR@`; PipeWire's source
    /// element takes the output's stream when it is asked to capture a sink (`stream.capture.sink`).
    pub fn source_args(self, source: Source) -> Vec<&'static str> {
        match (self, source) {
            (c, Source::Microphone) => vec![c.element()],
            (Capture::PulseAudio, Source::SystemAudio) => vec!["pulsesrc", "device=@DEFAULT_MONITOR@"],
            (Capture::PipeWire, Source::SystemAudio) => vec!["pipewiresrc", "stream-properties=props,stream.capture.sink=true"],
        }
    }
}

/// Why a pipeline could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineError {
    /// The model path contains a quote, a space-separated `!` or a line break, which gst-launch would misread.
    UnsafeModelPath(String),
    /// The language code is not two or three lowercase letters.
    BadLanguage(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::UnsafeModelPath(p) => write!(f, "the model path cannot be used in a pipeline: {p}"),
            PipelineError::BadLanguage(l) => write!(f, "the language code cannot be used in a pipeline: {l}"),
        }
    }
}

impl std::error::Error for PipelineError {}

/// The argv of the recognition pipeline:
/// `gst-launch-1.0 -m <capture> ! audioconvert ! audioresample ! audio/x-raw,format=F32LE,channels=1,rate=16000 !
/// whisper model=<path> ! fakesink`. `-m` prints the element messages that carry the transcript. The first element of
/// the returned vector is the program name.
pub fn pipeline_argv(capture: Capture, model: &Path) -> Result<Vec<String>, PipelineError> {
    pipeline_argv_for(capture, Source::Microphone, model)
}

/// [`pipeline_argv`] for a chosen [`Source`]: the same chain with the capture element for the microphone or for the
/// sound the computer plays.
pub fn pipeline_argv_for(capture: Capture, source: Source, model: &Path) -> Result<Vec<String>, PipelineError> {
    pipeline_argv_lang(capture, source, model, None)
}

/// [`pipeline_argv_for`] with the language the multilingual model should listen for (`language=<code>` on the whisper
/// element). `None` and English leave the element at its default, so an English pipeline is unchanged. A code that is
/// not two or three lowercase letters is refused, since it would otherwise reach the gst-launch command line.
pub fn pipeline_argv_lang(capture: Capture, source: Source, model: &Path, language: Option<&str>) -> Result<Vec<String>, PipelineError> {
    if let Some(code) = language {
        if !(2..=3).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(PipelineError::BadLanguage(code.to_string()));
        }
    }
    let model_text = model.to_string_lossy().into_owned();
    if model_text.contains(['"', '!', '\n', '\r', '\0']) {
        return Err(PipelineError::UnsafeModelPath(model_text));
    }
    let caps = format!("audio/x-raw,format=F32LE,channels=1,rate={SAMPLE_RATE}");
    let model_arg = if model_text.contains(' ') { format!("model=\"{model_text}\"") } else { format!("model={model_text}") };
    let mut argv: Vec<&str> = vec![GST_LAUNCH, "-m"];
    argv.extend(capture.source_args(source));
    let language_arg = language.filter(|l| *l != "en").map(|l| format!("language={l}"));
    argv.extend(["!", "audioconvert", "!", "audioresample", "!", caps.as_str(), "!", "whisper", model_arg.as_str()]);
    argv.extend(language_arg.as_deref());
    argv.extend(["!", "fakesink"]);
    Ok(argv.iter().map(|s| s.to_string()).collect())
}

/// One transcript the whisper element reported. `is_final` is false for a hypothesis the element may still revise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
    pub is_final: bool,
}

/// Reads a transcript from one line of gst-launch output. The element message names the whisper element and carries
/// the words in a `text="..."` field and the state in `is-final=(boolean)true|false`. Other lines return `None`, and so
/// do lines whose text is blank.
pub fn parse_transcript_line(line: &str) -> Option<Transcript> {
    if !line.contains("whisper") {
        return None;
    }
    let start = line.find("text=\"")? + "text=\"".len();
    let text = unescape(&line[start..])?;
    if text.trim().is_empty() {
        return None;
    }
    let is_final = line.contains("is-final=(boolean)true") || line.contains("is-final=true");
    Some(Transcript { text: text.trim().to_string(), is_final })
}

/// Reads the quoted value that starts at the beginning of `rest`, undoing `\"` and `\\`. `None` when it never closes.
fn unescape(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

/// The consent gate. The person grants consent in Settings after seeing the model size. Without a grant the shell
/// neither downloads the model nor starts a recording.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Consent {
    granted: bool,
}

/// Why a step was refused by the consent gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NoConsent,
    ModelMissing,
}

impl Consent {
    /// The state read from the consent record: granted only when the record says exactly `granted`.
    pub fn from_record(text: &str) -> Self {
        Consent { granted: text.trim() == "granted" }
    }

    /// The text to write to the consent record.
    pub fn to_record(self) -> &'static str {
        if self.granted { "granted\n" } else { "revoked\n" }
    }

    pub fn is_granted(self) -> bool {
        self.granted
    }

    /// The person agreed to the download and to on-device recognition.
    pub fn grant(&mut self) {
        self.granted = true;
    }

    /// The person withdrew consent. Recognition stops being allowed; the model file is kept until it is removed.
    pub fn revoke(&mut self) {
        self.granted = false;
    }

    /// A download may start only with consent.
    pub fn may_download(self) -> Result<(), Refusal> {
        if self.granted { Ok(()) } else { Err(Refusal::NoConsent) }
    }

    /// A recording may start with consent and an installed model.
    pub fn may_recognize(self, model_installed: bool) -> Result<(), Refusal> {
        if !self.granted {
            Err(Refusal::NoConsent)
        } else if !model_installed {
            Err(Refusal::ModelMissing)
        } else {
            Ok(())
        }
    }
}

/// The request the SDK sends to start live recognition, and the reply the shell gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub locale: String,
    pub report_partial: bool,
}

/// The reply to a poll: the transcripts gathered since the last poll, and whether recognition still runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollReply {
    pub transcripts: Vec<Transcript>,
    pub running: bool,
}

/// Whether a locale can be served: English with the English model, and every other language the multilingual whisper
/// models know once that model is installed ([`model_for_locale`]).
pub fn locale_supported(locale: &str) -> bool {
    whisper_language(locale).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_and_consent_live_under_the_user_data_dir() {
        let dir = Path::new("/home/someone/.local/share/bifrost");
        assert_eq!(model_path(dir), PathBuf::from("/home/someone/.local/share/bifrost/speech/ggml-base.en.bin"));
        assert_eq!(consent_path(dir), PathBuf::from("/home/someone/.local/share/bifrost/speech/consent"));
    }

    #[test]
    fn the_pipeline_captures_converts_and_feeds_the_whisper_element() {
        let argv = pipeline_argv(Capture::PipeWire, Path::new("/data/speech/ggml-base.en.bin")).unwrap();
        assert_eq!(argv[0], "gst-launch-1.0");
        assert!(argv.contains(&"pipewiresrc".to_string()));
        assert!(argv.contains(&"whisper".to_string()));
        assert!(argv.contains(&"model=/data/speech/ggml-base.en.bin".to_string()));
        assert!(argv.contains(&"audio/x-raw,format=F32LE,channels=1,rate=16000".to_string()));
        assert_eq!(argv.last().map(String::as_str), Some("fakesink"));
        let bang_count = argv.iter().filter(|a| a.as_str() == "!").count();
        assert_eq!(bang_count, 5, "six elements are joined by five links");
    }

    #[test]
    fn system_audio_is_the_monitor_of_the_default_output() {
        let model = Path::new("/data/speech/ggml-base.en.bin");
        let pulse = pipeline_argv_for(Capture::PulseAudio, Source::SystemAudio, model).unwrap();
        assert!(pulse.contains(&"pulsesrc".to_string()) && pulse.contains(&"device=@DEFAULT_MONITOR@".to_string()));
        let pipe = pipeline_argv_for(Capture::PipeWire, Source::SystemAudio, model).unwrap();
        assert!(pipe.contains(&"pipewiresrc".to_string()) && pipe.contains(&"stream-properties=props,stream.capture.sink=true".to_string()));
        for argv in [&pulse, &pipe] {
            assert_eq!(argv.last().map(String::as_str), Some("fakesink"));
            assert_eq!(argv.iter().filter(|a| a.as_str() == "!").count(), 5, "the same chain after the source");
        }
        let mic = pipeline_argv_for(Capture::PulseAudio, Source::Microphone, model).unwrap();
        assert!(!mic.iter().any(|a| a.contains("MONITOR")), "the microphone has no monitor");
        assert_eq!(mic, pipeline_argv(Capture::PulseAudio, model).unwrap());
    }

    #[test]
    fn pulseaudio_uses_pulsesrc_and_a_path_with_spaces_is_quoted() {
        let argv = pipeline_argv(Capture::PulseAudio, Path::new("/my data/model.bin")).unwrap();
        assert!(argv.contains(&"pulsesrc".to_string()));
        assert!(argv.contains(&"model=\"/my data/model.bin\"".to_string()));
    }

    #[test]
    fn a_model_path_that_would_break_the_pipeline_is_refused() {
        assert!(pipeline_argv(Capture::PulseAudio, Path::new("/x\"y/model.bin")).is_err());
        assert!(pipeline_argv(Capture::PulseAudio, Path::new("/x ! fakesink/model.bin")).is_err());
    }

    #[test]
    fn a_partial_transcript_line_is_parsed() {
        let line = r#"/GstPipeline:pipeline0/whisper:whisper0: element message {text="hello there", is-final=(boolean)false}"#;
        assert_eq!(parse_transcript_line(line), Some(Transcript { text: "hello there".into(), is_final: false }));
    }

    #[test]
    fn a_final_transcript_line_is_marked_final_and_escapes_are_undone() {
        let line = r#"whisper0: {text="say \"hi\"", is-final=(boolean)true}"#;
        assert_eq!(parse_transcript_line(line), Some(Transcript { text: "say \"hi\"".into(), is_final: true }));
    }

    #[test]
    fn other_lines_and_blank_text_are_ignored() {
        assert_eq!(parse_transcript_line("Setting pipeline to PLAYING"), None);
        assert_eq!(parse_transcript_line(r#"whisper0: {text="   ", is-final=(boolean)true}"#), None);
        assert_eq!(parse_transcript_line(r#"whisper0: {text="never closes"#), None);
    }

    #[test]
    fn consent_is_needed_before_any_download_or_recording() {
        let mut c = Consent::default();
        assert_eq!(c.may_download(), Err(Refusal::NoConsent));
        assert_eq!(c.may_recognize(true), Err(Refusal::NoConsent));
        c.grant();
        assert_eq!(c.may_download(), Ok(()));
        assert_eq!(c.may_recognize(false), Err(Refusal::ModelMissing));
        assert_eq!(c.may_recognize(true), Ok(()));
        c.revoke();
        assert_eq!(c.may_recognize(true), Err(Refusal::NoConsent));
    }

    #[test]
    fn the_consent_record_round_trips_and_only_granted_counts() {
        assert!(Consent::from_record("granted\n").is_granted());
        assert!(!Consent::from_record("revoked\n").is_granted());
        assert!(!Consent::from_record("").is_granted());
        assert!(!Consent::from_record("yes").is_granted());
        let mut c = Consent::default();
        c.grant();
        assert_eq!(Consent::from_record(c.to_record()), c);
    }

    #[test]
    fn locales_the_whisper_models_know_are_served() {
        assert!(locale_supported("en_US"));
        assert!(locale_supported("en-GB"));
        assert!(locale_supported("is_IS"));
        assert!(locale_supported("zh-Hans"));
        assert!(!locale_supported("tlh"));
        assert!(!locale_supported(""));
    }

    #[test]
    fn english_uses_the_english_model_and_other_languages_the_multilingual_one() {
        assert_eq!(model_for_locale("en_GB"), Some(ENGLISH_MODEL));
        assert_eq!(model_for_locale("is_IS"), Some(MULTILINGUAL_MODEL));
        assert_eq!(model_for_locale("de"), Some(MULTILINGUAL_MODEL));
        assert_eq!(model_for_locale("tlh"), None);
        assert_ne!(ENGLISH_MODEL.file, MULTILINGUAL_MODEL.file);
        assert!(MULTILINGUAL_MODEL.url.ends_with(MULTILINGUAL_MODEL.file));
        let dir = Path::new("/home/someone/data");
        assert_eq!(model_file_path(dir, MULTILINGUAL_MODEL), PathBuf::from("/home/someone/data/speech/ggml-base.bin"));
        assert_eq!(model_file_path(dir, ENGLISH_MODEL), model_path(dir));
    }

    #[test]
    fn whisper_language_codes_are_unique_lowercase_letters() {
        let mut seen = std::collections::HashSet::new();
        for l in WHISPER_LANGUAGES {
            assert!((2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase()), "{l}");
            assert!(seen.insert(*l), "{l} listed twice");
        }
        assert_eq!(whisper_language("Is_is"), Some("is"));
        assert_eq!(whisper_language("haw_US"), Some("haw"));
    }

    #[test]
    fn a_non_english_pipeline_names_its_language_after_the_model() {
        let model = Path::new("/home/someone/data/speech/ggml-base.bin");
        let is = pipeline_argv_lang(Capture::PulseAudio, Source::Microphone, model, Some("is")).unwrap();
        let at = is.iter().position(|a| a == "language=is").expect("language property");
        assert_eq!(is[at - 1], "model=/home/someone/data/speech/ggml-base.bin");
        assert_eq!(is[at + 1], "!");
        assert_eq!(is.last().map(String::as_str), Some("fakesink"));
        assert_eq!(is.iter().filter(|a| a.as_str() == "!").count(), 5);
        let en = pipeline_argv_lang(Capture::PulseAudio, Source::Microphone, model, Some("en")).unwrap();
        assert_eq!(en, pipeline_argv(Capture::PulseAudio, model).unwrap(), "English keeps the element's default");
        assert!(pipeline_argv_lang(Capture::PulseAudio, Source::Microphone, model, Some("is ! x")).is_err());
        assert!(pipeline_argv_lang(Capture::PulseAudio, Source::Microphone, model, Some("IS")).is_err());
    }
}
