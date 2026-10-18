//! The `speech_recognizer` service: on-device live recognition for apps. The SDK client is `bifrost_app::speech_whisper`.
//!
//! The shell captures the microphone with a `gst-launch-1.0` pipeline that feeds the whisper element with a local model
//! (`bifrost_ui::whisper`). No audio leaves the machine. Each app has at most one recognition at a time; a new `start`
//! replaces it, and the shell ends it when the app quits. Consent comes first: the model is downloaded only after the
//! person grants it in Settings, and recognition starts only with consent and the model installed.
//!
//! Calls (JSON params):
//! - `start` `{locale, partial?}` starts recognition; replies `{id}`. The locale must be English. `partial` (default
//!   false) asks for hypotheses as well as final transcripts.
//! - `poll` `{}` replies `{transcripts: [{text, final}], running}` with the transcripts since the last poll. After the
//!   pipeline has ended and its last transcripts were returned, `running` is false and the recognition is gone.
//! - `stop` `{}` ends this app's recognition (the pipeline is interrupted so it flushes its last transcript); replies
//!   `{stopped}`.
//! - `model` `{}` replies `{granted, installed, downloading, size_bytes}`.
//! - `captions_start` `{}`, `captions_poll` `{}` and `captions_stop` `{}` are the shell's own, for Live Captions
//!   (`shell/live_captions.rs`): they capture the sound the computer plays (the default output's monitor) instead of the
//!   microphone and run beside an app's recognition. An app calling them is refused. `captions_poll` replies
//!   `{transcripts: [{text, final}], running}` with hypotheses as well as final transcripts.
//! - `consent` `{granted}` records the person's answer; revoking ends a running recognition. Replies `{granted}`.
//! - `download` `{}` fetches the model in a thread once consent is granted; replies `{started}`. Progress is in `model`.
//!
//! Refusals are errors: no consent, no model, a non-English locale, or no pipeline program installed.
//!
//! Voice Control: while the `voice_control` preference is `1` the shell holds a recognition of its own (the app id
//! [`SYSTEM_APP`]) when no app has one, runs every final transcript through `Shell::voice_press` ("press Save") and
//! hands the resulting effects to the next shell tick. It needs the same consent and model as dictation, never takes
//! the microphone from an app, and retries a refused start only every [`VOICE_RETRY_S`] seconds.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::whisper::{
    consent_path, data_subdir, locale_supported, model_path, parse_transcript_line, pipeline_argv, pipeline_argv_for, Capture,
    Consent, PipelineError, Refusal, Source, Transcript, MODEL_SIZE_BYTES, MODEL_URL,
};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The name the service is registered under.
pub const SERVICE: &str = "speech_recognizer";
/// The app id of the shell's own Voice Control recognition. It is not an installed app, so it is never reaped as gone.
pub const SYSTEM_APP: &str = "system.voice_control";
/// The preference (`1` or `0`) that turns Voice Control on.
pub const VOICE_CONTROL_PREF: &str = "voice_control";
/// Seconds to wait before starting Voice Control again after a refusal or an ended pipeline.
pub const VOICE_RETRY_S: f64 = 30.0;
/// Clients of the shell's own (Voice Control) are named with this prefix; no installed app has such a name.
pub const SHELL_CLIENT_PREFIX: &str = "shell:";

/// Whether a session whose client is `app` has lost it: the app is not running. A client of the shell itself (Voice Control)
/// is no app and stays.
fn orphaned(app: &str, running: impl Fn(&str) -> bool) -> bool {
    !app.starts_with(SHELL_CLIENT_PREFIX) && !running(app)
}
/// The caller name of the shell itself. Only it may capture the sound the computer plays.
pub const SHELL_APP: &str = "bifrost-shell";

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
/// SIGINT, which gst-launch turns into an end-of-stream so the whisper element flushes its last transcript.
const SIGINT: i32 = 2;

/// One running pipeline, as the service drives it.
pub trait Pipeline {
    /// The lines the pipeline printed since the last call.
    fn take_lines(&mut self) -> Vec<String>;
    /// Asks the pipeline to end, so it flushes what it has.
    fn interrupt(&mut self);
    /// True once the pipeline has exited (this reaps it).
    fn finished(&mut self) -> bool;
    fn kill(&mut self);
}

/// Starts a pipeline from its argv (the first entry is the program).
pub trait PipelineSpawner {
    fn start(&mut self, argv: &[String]) -> Result<Box<dyn Pipeline>, String>;
}

/// Fetches the model file to a path. The real one runs curl; the shell runs it in a thread.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str, to: &Path) -> Result<(), String>;
}

/// The real spawner: gst-launch-1.0 with its standard output read line by line in a thread.
struct GstSpawner;

struct GstPipeline {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
}

impl PipelineSpawner for GstSpawner {
    fn start(&mut self, argv: &[String]) -> Result<Box<dyn Pipeline>, String> {
        let (program, args) = argv.split_first().ok_or("speech recognizer: the pipeline has no program")?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "speech recognizer: gst-launch-1.0 is not installed".to_string()
                } else {
                    format!("speech recognizer: {e}")
                }
            })?;
        let lines = Arc::new(Mutex::new(Vec::new()));
        if let Some(out) = child.stdout.take() {
            let shared = Arc::clone(&lines);
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Ok(mut l) = shared.lock() {
                        l.push(line);
                    }
                }
            });
        }
        Ok(Box::new(GstPipeline { child, lines }))
    }
}

impl Pipeline for GstPipeline {
    fn take_lines(&mut self) -> Vec<String> {
        self.lines.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default()
    }

    fn interrupt(&mut self) {
        // SAFETY: the pid is a child this service started and has not reaped.
        unsafe {
            kill(self.child.id() as i32, SIGINT);
        }
    }

    fn finished(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The real fetcher: curl into a partial file, renamed into place only when the download completed.
struct CurlFetcher;

impl Fetcher for CurlFetcher {
    fn fetch(&self, url: &str, to: &Path) -> Result<(), String> {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("speech model: {e}"))?;
        }
        let part = to.with_extension("part");
        let status = Command::new("curl")
            .args(["-fsSL", "--output"])
            .arg(&part)
            .arg(url)
            .status()
            .map_err(|e| format!("speech model: curl: {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&part);
            return Err("speech model: the download failed".to_string());
        }
        std::fs::rename(&part, to).map_err(|e| format!("speech model: {e}"))
    }
}

/// The download state, shared with its thread.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Download {
    Idle,
    Running,
    Done,
    Failed(String),
}

/// An app's recognition: its pipeline and whether it reports hypotheses.
struct Session {
    app: String,
    pipeline: Box<dyn Pipeline>,
    partial: bool,
}

/// The model download, shared by the service and Settings. Settings starts it and the service reports it.
static DOWNLOAD: Mutex<Download> = Mutex::new(Download::Idle);

pub struct Recognizer {
    data_dir: PathBuf,
    spawner: Box<dyn PipelineSpawner>,
    fetcher: Arc<dyn Fetcher>,
    session: Option<Session>,
    /// the Live Captions pipeline, which captures the computer's own sound; it runs beside an app's recognition
    captions: Option<Box<dyn Pipeline>>,
    next: u32,
    /// the earliest time Voice Control may try to start again
    voice_retry_at: f64,
}

/// What Settings shows for the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelView {
    pub granted: bool,
    pub installed: bool,
    pub downloading: bool,
    /// the reason the last download failed, until the next one starts
    pub failed: Option<String>,
}

/// The consent record under `data_dir`. A missing or unreadable record counts as no consent.
pub fn consent_of(data_dir: &Path) -> Consent {
    Consent::from_record(&std::fs::read_to_string(consent_path(data_dir)).unwrap_or_default())
}

fn write_consent(data_dir: &Path, consent: Consent) -> Result<(), String> {
    std::fs::create_dir_all(data_subdir(data_dir)).map_err(|e| format!("speech recognizer: {e}"))?;
    std::fs::write(consent_path(data_dir), consent.to_record()).map_err(|e| format!("speech recognizer: {e}"))
}

/// The model state for Settings.
pub fn model_state(data_dir: &Path) -> ModelView {
    let download = DOWNLOAD.lock().map(|d| d.clone()).unwrap_or(Download::Idle);
    ModelView {
        granted: consent_of(data_dir).is_granted(),
        installed: model_path(data_dir).is_file(),
        downloading: download == Download::Running,
        failed: match download {
            Download::Failed(e) => Some(e),
            _ => None,
        },
    }
}

/// Settings: the person agreed to the download. Records the consent and starts the download in a thread.
pub fn grant_and_download(data_dir: &Path) -> Result<(), String> {
    write_consent(data_dir, Consent::from_record("granted"))?;
    start_download(data_dir, Arc::new(CurlFetcher)).map(|_| ())
}

/// Settings: removes the model file and withdraws consent. A running recognition ends on its next poll.
pub fn remove_model(data_dir: &Path) -> Result<(), String> {
    write_consent(data_dir, Consent::default())?;
    match std::fs::remove_file(model_path(data_dir)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("speech model: {e}")),
    }
    if let Ok(mut d) = DOWNLOAD.lock() {
        *d = Download::Idle;
    }
    Ok(())
}

/// Starts the model download when consent is granted and the model is missing. Returns false when nothing started.
fn start_download(data_dir: &Path, fetcher: Arc<dyn Fetcher>) -> Result<bool, String> {
    consent_of(data_dir).may_download().map_err(refusal_text)?;
    if model_path(data_dir).is_file() {
        return Ok(false);
    }
    {
        let mut d = DOWNLOAD.lock().map_err(|_| "speech model: state unavailable".to_string())?;
        if *d == Download::Running {
            return Ok(false);
        }
        *d = Download::Running;
    }
    let to = model_path(data_dir);
    std::thread::spawn(move || {
        let result = fetcher.fetch(MODEL_URL, &to);
        if let Ok(mut d) = DOWNLOAD.lock() {
            *d = match result {
                Ok(()) => Download::Done,
                Err(e) => Download::Failed(e),
            };
        }
    });
    Ok(true)
}

/// The user data directory the model lives in: `$XDG_DATA_HOME/springboard`, else `~/.local/share/springboard`.
pub fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("springboard")
}

/// The audio backend: PipeWire when its socket is up, else PulseAudio.
fn capture() -> Capture {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/run/user/0"));
    if runtime.join("pipewire-0").exists() { Capture::PipeWire } else { Capture::PulseAudio }
}

fn refusal_text(r: Refusal) -> String {
    match r {
        Refusal::NoConsent => "speech recognizer: grant consent in Settings before recognition starts".to_string(),
        Refusal::ModelMissing => "speech recognizer: the speech model is not installed; download it in Settings".to_string(),
    }
}

fn pipeline_error(e: PipelineError) -> String {
    format!("speech recognizer: {e}")
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn poll_json(transcripts: &[Transcript], running: bool) -> Json {
    let list = transcripts
        .iter()
        .map(|t| obj(vec![("text", Json::Str(t.text.clone())), ("final", Json::Bool(t.is_final))]))
        .collect();
    obj(vec![("transcripts", Json::Arr(list)), ("running", Json::Bool(running))])
}

impl Recognizer {
    /// The service for the user data directory `data_dir`, with the real pipeline and curl.
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_parts(data_dir, Box::new(GstSpawner), Arc::new(CurlFetcher))
    }

    fn with_parts(data_dir: PathBuf, spawner: Box<dyn PipelineSpawner>, fetcher: Arc<dyn Fetcher>) -> Self {
        Recognizer { data_dir, spawner, fetcher, session: None, next: 1, voice_retry_at: f64::NEG_INFINITY }
    }

    /// One step of the shell's Voice Control recognition at time `t`: starts it when `wanted` and nothing else holds
    /// the microphone, stops it when no longer wanted or when consent was withdrawn, and returns the final
    /// transcripts heard since the last step. Hypotheses are never returned.
    fn voice_step(&mut self, wanted: bool, t: f64) -> Vec<Transcript> {
        let mine = self.session.as_ref().is_some_and(|s| s.app == SYSTEM_APP);
        if !wanted || (mine && !consent_of(&self.data_dir).is_granted()) {
            if mine {
                self.stop_session();
            }
            return Vec::new();
        }
        if !mine {
            if self.session.is_some() || t < self.voice_retry_at {
                return Vec::new();
            }
            if self.handle(SYSTEM_APP, "start", &Json::Null).is_err() {
                self.voice_retry_at = t + VOICE_RETRY_S;
                return Vec::new();
            }
        }
        let Some(session) = self.session.as_mut().filter(|s| s.app == SYSTEM_APP) else { return Vec::new() };
        let heard: Vec<Transcript> = session.pipeline.take_lines().iter().filter_map(|l| parse_transcript_line(l)).filter(|t| t.is_final).collect();
        if session.pipeline.finished() {
            self.session = None;
            self.voice_retry_at = t + VOICE_RETRY_S;
        }
        heard
    pub(crate) fn with_parts(data_dir: PathBuf, spawner: Box<dyn PipelineSpawner>, fetcher: Arc<dyn Fetcher>) -> Self {
        Recognizer { data_dir, spawner, fetcher, session: None, captions: None, next: 1 }
    }

    fn stop_captions(&mut self) -> bool {
        match self.captions.take() {
            Some(mut p) => {
                p.kill();
                true
            }
            None => false,
        }
    }

    fn installed(&self) -> bool {
        model_path(&self.data_dir).is_file()
    }

    fn stop_session(&mut self) -> bool {
        match self.session.take() {
            Some(mut s) => {
                s.pipeline.kill();
                true
            }
            None => false,
        }
    }

    /// Answers one call from `app`. The shell's [`SystemService::call`] delegates here, so the rules can be tested
    /// without a shell.
    fn handle(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "start" => {
                let locale = params.str_field("locale").unwrap_or_else(|| "en_US".to_string());
                if !locale_supported(&locale) {
                    return Err("speech recognizer: only English is served".to_string());
                }
                let partial = matches!(params.get("partial"), Some(Json::Bool(true)));
                consent_of(&self.data_dir).may_recognize(self.installed()).map_err(refusal_text)?;
                let argv = pipeline_argv(capture(), &model_path(&self.data_dir)).map_err(pipeline_error)?;
                // start first: when the pipeline cannot start the app keeps what it had
                let pipeline = self.spawner.start(&argv)?;
                self.stop_session();
                self.session = Some(Session { app: app.to_string(), pipeline, partial });
                let id = self.next;
                self.next = self.next.checked_add(1).unwrap_or(1);
                Ok(obj(vec![("id", Json::Num(id as f64))]))
            }
            "poll" => {
                // a withdrawn consent ends the recognition at once
                if !consent_of(&self.data_dir).is_granted() {
                    self.stop_session();
                    return Ok(poll_json(&[], false));
                }
                let Some(session) = self.session.as_mut().filter(|s| s.app == app) else {
                    return Ok(poll_json(&[], false));
                };
                let partial = session.partial;
                let transcripts: Vec<Transcript> = session
                    .pipeline
                    .take_lines()
                    .iter()
                    .filter_map(|l| parse_transcript_line(l))
                    .filter(|t| partial || t.is_final)
                    .collect();
                let finished = session.pipeline.finished();
                if finished {
                    self.session = None;
                }
                Ok(poll_json(&transcripts, !finished))
            }
            "captions_start" | "captions_poll" | "captions_stop" if app != SHELL_APP => {
                Err("speech recognizer: only the shell captures the sound the computer plays".to_string())
            }
            "captions_start" => {
                consent_of(&self.data_dir).may_recognize(self.installed()).map_err(refusal_text)?;
                let argv = pipeline_argv_for(capture(), Source::SystemAudio, &model_path(&self.data_dir)).map_err(pipeline_error)?;
                let pipeline = self.spawner.start(&argv)?;
                self.stop_captions();
                self.captions = Some(pipeline);
                Ok(obj(vec![("started", Json::Bool(true))]))
            }
            "captions_poll" => {
                if !consent_of(&self.data_dir).is_granted() {
                    self.stop_captions();
                    return Ok(poll_json(&[], false));
                }
                let Some(pipeline) = self.captions.as_mut() else { return Ok(poll_json(&[], false)) };
                let transcripts: Vec<Transcript> = pipeline.take_lines().iter().filter_map(|l| parse_transcript_line(l)).collect();
                let finished = pipeline.finished();
                if finished {
                    self.captions = None;
                }
                Ok(poll_json(&transcripts, !finished))
            }
            "captions_stop" => Ok(obj(vec![("stopped", Json::Bool(self.stop_captions()))])),
            "stop" => {
                let stopped = match self.session.as_mut().filter(|s| s.app == app) {
                    Some(s) => {
                        s.pipeline.interrupt();
                        true
                    }
                    None => false,
                };
                Ok(obj(vec![("stopped", Json::Bool(stopped))]))
            }
            "model" => {
                let view = model_state(&self.data_dir);
                Ok(obj(vec![
                    ("granted", Json::Bool(view.granted)),
                    ("installed", Json::Bool(view.installed)),
                    ("downloading", Json::Bool(view.downloading)),
                    ("size_bytes", Json::Num(MODEL_SIZE_BYTES as f64)),
                ]))
            }
            "consent" => {
                let granted = match params.get("granted") {
                    Some(Json::Bool(g)) => *g,
                    _ => return Err("speech recognizer: consent needs a granted flag".to_string()),
                };
                if !granted {
                    self.stop_session();
                    self.stop_captions();
                }
                write_consent(&self.data_dir, Consent::from_record(if granted { "granted" } else { "revoked" }))?;
                Ok(obj(vec![("granted", Json::Bool(granted))]))
            }
            "download" => {
                let started = start_download(&self.data_dir, Arc::clone(&self.fetcher))?;
                Ok(obj(vec![("started", Json::Bool(started))]))
            }
            other => Err(format!("speech recognizer: no method {other}")),
        }
    }
}

impl SystemService for Recognizer {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let running = |app: &str| {
            app == SYSTEM_APP || crate::shell::apphost::slot_of(app).is_some_and(|slot| shell.ext.get(slot).is_some_and(|h| h.is_some()))
        };
        let gone = self.session.as_ref().is_some_and(|s| orphaned(&s.app, &running));
        if gone {
            self.stop_session();
        }
        let wanted = shell.prefs.get(VOICE_CONTROL_PREF) == Some("1");
        for heard in self.voice_step(wanted, t) {
            match shell.voice_press(&heard, t) {
                Some(Ok((_, fx))) => shell.voice_fx.extend(fx),
                // a command that could not run says why on the announcement banner
                Some(Err(why)) => {
                    shell.announce.post(&why, bifrost_proto::AnnouncePriority::High);
                    shell.dirty = true;
                }
                None => {}
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Log {
        argvs: Vec<Vec<String>>,
        kills: usize,
        interrupts: usize,
        fetches: Vec<String>,
    }

    struct FakeSpawner {
        log: Arc<Mutex<Log>>,
        lines: Arc<Mutex<Vec<String>>>,
        finished: Arc<AtomicBool>,
    }

    struct FakePipeline {
        log: Arc<Mutex<Log>>,
        lines: Arc<Mutex<Vec<String>>>,
        finished: Arc<AtomicBool>,
    }

    impl PipelineSpawner for FakeSpawner {
        fn start(&mut self, argv: &[String]) -> Result<Box<dyn Pipeline>, String> {
            self.log.lock().unwrap().argvs.push(argv.to_vec());
            Ok(Box::new(FakePipeline {
                log: Arc::clone(&self.log),
                lines: Arc::clone(&self.lines),
                finished: Arc::clone(&self.finished),
            }))
        }
    }

    impl Pipeline for FakePipeline {
        fn take_lines(&mut self) -> Vec<String> {
            std::mem::take(&mut *self.lines.lock().unwrap())
        }
        fn interrupt(&mut self) {
            self.log.lock().unwrap().interrupts += 1;
        }
        fn finished(&mut self) -> bool {
            self.finished.load(Ordering::SeqCst)
        }
        fn kill(&mut self) {
            self.log.lock().unwrap().kills += 1;
        }
    }

    struct FakeFetcher(Arc<Mutex<Log>>);

    impl Fetcher for FakeFetcher {
        fn fetch(&self, url: &str, _to: &Path) -> Result<(), String> {
            self.0.lock().unwrap().fetches.push(url.to_string());
            Ok(())
        }
    }

    struct Rig {
        rec: Recognizer,
        log: Arc<Mutex<Log>>,
        lines: Arc<Mutex<Vec<String>>>,
        finished: Arc<AtomicBool>,
        root: PathBuf,
    }

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A data dir with the model installed or not, and a recognizer with fake pipeline and fetcher.
    fn rig(model: bool, consent: bool) -> Rig {
        let root = std::env::temp_dir().join(format!(
            "bifrost-recognizer-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(data_subdir(&root)).unwrap();
        if model {
            std::fs::write(model_path(&root), b"model").unwrap();
        }
        let log = Arc::new(Mutex::new(Log::default()));
        let lines = Arc::new(Mutex::new(Vec::new()));
        let finished = Arc::new(AtomicBool::new(false));
        if consent {
            write_consent(&root, Consent::from_record("granted")).unwrap();
        }
        let spawner = FakeSpawner { log: Arc::clone(&log), lines: Arc::clone(&lines), finished: Arc::clone(&finished) };
        let rec = Recognizer::with_parts(root.clone(), Box::new(spawner), Arc::new(FakeFetcher(Arc::clone(&log))));
        Rig { rec, log, lines, finished, root }
    }

    fn params(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    fn flag(reply: &Result<Json, String>, key: &str) -> bool {
        match reply.as_ref().unwrap().get(key) {
            Some(Json::Bool(b)) => *b,
            other => panic!("no flag {key}: {other:?}"),
        }
    }

    fn text_of(reply: &Result<Json, String>, index: usize) -> String {
        let list = reply.as_ref().unwrap().get("transcripts").and_then(Json::as_array).unwrap();
        list[index].str_field("text").unwrap()
    }

    #[test]
    fn a_session_of_the_shell_is_not_ended_for_want_of_an_app() {
        assert!(orphaned("is.example.app", |_| false), "the app quit");
        assert!(!orphaned("is.example.app", |_| true), "the app runs");
        assert!(!orphaned("shell:voice-control", |_| false), "Voice Control is the shell's own client");
    }

    #[test]
    fn a_start_without_consent_is_refused_even_with_the_model() {
        let mut r = rig(true, false);
        let err = r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap_err();
        assert!(err.contains("consent"), "{err}");
        assert!(r.log.lock().unwrap().argvs.is_empty());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_start_with_consent_but_no_model_is_refused() {
        let mut r = rig(false, true);
        let err = r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap_err();
        assert!(err.contains("not installed"), "{err}");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_start_runs_the_pipeline_with_the_model_and_a_second_start_replaces_it() {
        let mut r = rig(true, true);
        let first = r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#));
        assert_eq!(first.unwrap().get("id").and_then(Json::number), Some(1.0));
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        let log = r.log.lock().unwrap();
        assert_eq!(log.argvs.len(), 2);
        assert_eq!(log.argvs[0][0], "gst-launch-1.0");
        assert!(log.argvs[0].iter().any(|a| a.starts_with("model=")));
        assert_eq!(log.kills, 1, "the first recognition ends when the second starts");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn non_english_locales_are_refused() {
        let mut r = rig(true, true);
        assert!(r.rec.handle("app.a", "start", &params(r#"{"locale":"is_IS"}"#)).is_err());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_poll_returns_the_final_transcripts_unless_partial_results_were_asked_for() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        *r.lines.lock().unwrap() = vec![
            "whisper0: {text=\"turn on\", is-final=(boolean)false}".to_string(),
            "Setting pipeline to PLAYING".to_string(),
            "whisper0: {text=\"turn on the lamp\", is-final=(boolean)true}".to_string(),
        ];
        let reply = r.rec.handle("app.a", "poll", &Json::Null);
        assert!(flag(&reply, "running"));
        assert_eq!(text_of(&reply, 0), "turn on the lamp");
        assert_eq!(reply.as_ref().unwrap().get("transcripts").and_then(Json::as_array).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn partial_results_are_returned_when_asked_for() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US","partial":true}"#)).unwrap();
        *r.lines.lock().unwrap() = vec!["whisper0: {text=\"turn\", is-final=(boolean)false}".to_string()];
        let reply = r.rec.handle("app.a", "poll", &Json::Null);
        assert_eq!(text_of(&reply, 0), "turn");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_stop_interrupts_and_the_session_ends_once_the_pipeline_has_exited() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        assert!(flag(&r.rec.handle("app.a", "stop", &Json::Null), "stopped"));
        assert_eq!(r.log.lock().unwrap().interrupts, 1);
        r.finished.store(true, Ordering::SeqCst);
        let reply = r.rec.handle("app.a", "poll", &Json::Null);
        assert!(!flag(&reply, "running"));
        assert!(r.rec.session.is_none());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn another_app_cannot_poll_or_stop_a_recognition() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        assert!(!flag(&r.rec.handle("app.b", "stop", &Json::Null), "stopped"));
        assert!(!flag(&r.rec.handle("app.b", "poll", &Json::Null), "running"));
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn revoking_consent_ends_a_running_recognition_and_is_recorded() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        let reply = r.rec.handle("app.a", "consent", &params(r#"{"granted":false}"#));
        assert!(!flag(&reply, "granted"));
        assert_eq!(r.log.lock().unwrap().kills, 1);
        let record = std::fs::read_to_string(consent_path(&r.root)).unwrap();
        assert!(!Consent::from_record(&record).is_granted());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn captions_capture_the_computers_sound_only_for_the_shell_and_only_with_consent() {
        let mut r = rig(true, false);
        let err = r.rec.handle(SHELL_APP, "captions_start", &Json::Null).unwrap_err();
        assert!(err.contains("consent"), "{err}");
        let mut r = rig(true, true);
        let err = r.rec.handle("app.a", "captions_start", &Json::Null).unwrap_err();
        assert!(err.contains("only the shell"), "{err}");
        assert!(r.rec.handle("app.a", "captions_poll", &Json::Null).is_err());
        assert!(r.rec.handle("app.a", "captions_stop", &Json::Null).is_err());
        assert!(r.log.lock().unwrap().argvs.is_empty(), "nothing was captured");
        assert!(flag(&r.rec.handle(SHELL_APP, "captions_start", &Json::Null), "started"));
        let argv = r.log.lock().unwrap().argvs[0].clone();
        assert!(argv.iter().any(|a| a == "device=@DEFAULT_MONITOR@" || a.starts_with("stream-properties=")), "{argv:?}");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn captions_run_beside_an_apps_recognition_and_report_hypotheses() {
        let mut r = rig(true, true);
        r.rec.handle("app.a", "start", &params(r#"{"locale":"en_US"}"#)).unwrap();
        r.rec.handle(SHELL_APP, "captions_start", &Json::Null).unwrap();
        assert_eq!(r.log.lock().unwrap().kills, 0, "the app keeps its recognition");
        r.lines.lock().unwrap().push(r#"whisper0: {text="hello", is-final=(boolean)false}"#.to_string());
        let reply = r.rec.handle(SHELL_APP, "captions_poll", &Json::Null);
        assert_eq!(text_of(&reply, 0), "hello", "a hypothesis is reported");
        assert!(flag(&reply, "running"));
        // a withdrawn consent ends both
        r.rec.handle(SHELL_APP, "consent", &params(r#"{"granted":false}"#)).unwrap();
        assert_eq!(r.log.lock().unwrap().kills, 2);
        assert!(!flag(&r.rec.handle(SHELL_APP, "captions_poll", &Json::Null), "running"));
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn captions_stop_ends_the_pipeline() {
        let mut r = rig(true, true);
        r.rec.handle(SHELL_APP, "captions_start", &Json::Null).unwrap();
        assert!(flag(&r.rec.handle(SHELL_APP, "captions_stop", &Json::Null), "stopped"));
        assert_eq!(r.log.lock().unwrap().kills, 1);
        assert!(!flag(&r.rec.handle(SHELL_APP, "captions_stop", &Json::Null), "stopped"));
        r.rec.handle(SHELL_APP, "captions_start", &Json::Null).unwrap();
        r.finished.store(true, Ordering::SeqCst);
        assert!(!flag(&r.rec.handle(SHELL_APP, "captions_poll", &Json::Null), "running"), "a pipeline that exited is gone");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_consent_call_needs_a_flag() {
        let mut r = rig(false, false);
        assert!(r.rec.handle("app.a", "consent", &Json::Null).is_err());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn the_model_reply_shows_consent_install_and_size() {
        let mut r = rig(true, true);
        let reply = r.rec.handle("app.a", "model", &Json::Null);
        assert!(flag(&reply, "granted"));
        assert!(flag(&reply, "installed"));
        assert!(!flag(&reply, "downloading"));
        assert_eq!(reply.unwrap().get("size_bytes").and_then(Json::number), Some(MODEL_SIZE_BYTES as f64));
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_download_is_refused_without_consent() {
        let mut r = rig(false, false);
        let err = r.rec.handle("app.a", "download", &Json::Null).unwrap_err();
        assert!(err.contains("consent"), "{err}");
        assert!(r.log.lock().unwrap().fetches.is_empty());
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn a_granted_download_fetches_the_model_once_it_is_missing() {
        let mut r = rig(false, true);
        assert!(flag(&r.rec.handle("app.a", "download", &Json::Null), "started"));
        // give the thread a moment: it records the fetch and the state
        for _ in 0..200 {
            if !r.log.lock().unwrap().fetches.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(r.log.lock().unwrap().fetches, vec![MODEL_URL.to_string()]);
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn settings_sees_consent_and_the_installed_model() {
        let root = std::env::temp_dir().join(format!("bifrost-recognizer-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(data_subdir(&root)).unwrap();
        assert_eq!(model_state(&root).granted, false);
        assert_eq!(model_state(&root).installed, false);
        write_consent(&root, Consent::from_record("granted")).unwrap();
        std::fs::write(model_path(&root), b"model").unwrap();
        let view = model_state(&root);
        assert!(view.granted && view.installed);
        remove_model(&root).unwrap();
        assert!(!model_state(&root).installed);
        assert!(!model_state(&root).granted, "removing the model withdraws consent");
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn voice_control_starts_its_own_recognition_and_returns_final_transcripts_only() {
        let mut r = rig(true, true);
        assert!(r.rec.voice_step(false, 1.0).is_empty(), "off means nothing starts");
        assert!(r.log.lock().unwrap().argvs.is_empty());
        *r.lines.lock().unwrap() = vec![
            "whisper0: {text=\"press sa\", is-final=(boolean)false}".to_string(),
            "whisper0: {text=\"press Save\", is-final=(boolean)true}".to_string(),
        ];
        let heard = r.rec.voice_step(true, 2.0);
        assert_eq!(r.log.lock().unwrap().argvs.len(), 1, "one pipeline was started");
        assert_eq!(heard, vec![Transcript { text: "press Save".into(), is_final: true }]);
        assert!(r.rec.voice_step(true, 3.0).is_empty(), "the running recognition is kept, not restarted");
        assert_eq!(r.log.lock().unwrap().argvs.len(), 1);
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn voice_control_stops_when_switched_off_or_consent_is_withdrawn() {
        let mut r = rig(true, true);
        r.rec.voice_step(true, 1.0);
        assert!(r.rec.session.is_some());
        r.rec.voice_step(false, 2.0);
        assert!(r.rec.session.is_none());
        assert_eq!(r.log.lock().unwrap().kills, 1);
        r.rec.voice_step(true, 100.0);
        write_consent(&r.root, Consent::default()).unwrap();
        r.rec.voice_step(true, 101.0);
        assert!(r.rec.session.is_none(), "withdrawn consent ends it on the next step");
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn voice_control_never_takes_the_microphone_from_an_app_and_waits_after_a_refusal() {
        let mut r = rig(true, true);
        r.rec.handle("is.x.notes", "start", &Json::Null).unwrap();
        assert!(r.rec.voice_step(true, 1.0).is_empty());
        assert_eq!(r.log.lock().unwrap().argvs.len(), 1, "the app keeps its pipeline");
        assert_eq!(r.rec.session.as_ref().map(|s| s.app.as_str()), Some("is.x.notes"));
        let _ = std::fs::remove_dir_all(&r.root);

        let mut refused = rig(false, true);
        assert!(refused.rec.voice_step(true, 10.0).is_empty(), "no model: the start is refused");
        assert!(refused.log.lock().unwrap().argvs.is_empty());
        std::fs::write(model_path(&refused.root), b"model").unwrap();
        refused.rec.voice_step(true, 20.0);
        assert!(refused.rec.session.is_none(), "inside the retry window nothing is tried");
        refused.rec.voice_step(true, 10.0 + VOICE_RETRY_S);
        assert!(refused.rec.session.is_some(), "after the window it starts");
        let _ = std::fs::remove_dir_all(&refused.root);
    }

    #[test]
    fn a_spoken_overlay_command_reaches_the_shell_through_the_service_tick() {
        let mut r = rig(true, true);
        let mut sh = crate::shell::tests::desktop_shell();
        sh.prefs.set(VOICE_CONTROL_PREF, "1").unwrap();
        *r.lines.lock().unwrap() = vec![
            "whisper0: {text=\"show num\", is-final=(boolean)false}".to_string(),
            "whisper0: {text=\"show numbers\", is-final=(boolean)true}".to_string(),
        ];
        r.rec.tick(&mut sh, 1.0);
        assert_eq!(sh.voice_overlay_shown(), Some(bifrost_ui::voice_commands::Overlay::Numbers), "the final transcript ran, the hypothesis did not");
        *r.lines.lock().unwrap() = vec!["whisper0: {text=\"press 99\", is-final=(boolean)true}".to_string()];
        r.rec.tick(&mut sh, 2.0);
        sh.announce.tick(2.0);
        assert!(sh.announce.shown().is_some_and(|t| t.contains("99")), "a command that cannot run says why");
        sh.prefs.set(VOICE_CONTROL_PREF, "0").unwrap();
        r.rec.tick(&mut sh, 3.0);
        assert!(r.rec.session.is_none(), "switching Voice Control off in Settings ends the recognition");
        assert_eq!(sh.voice_overlay_shown(), None);
        let _ = std::fs::remove_dir_all(&r.root);
    }

    #[test]
    fn an_ended_voice_control_pipeline_is_restarted_after_the_wait() {
        let mut r = rig(true, true);
        r.rec.voice_step(true, 1.0);
        r.finished.store(true, Ordering::SeqCst);
        r.rec.voice_step(true, 2.0);
        assert!(r.rec.session.is_none());
        r.finished.store(false, Ordering::SeqCst);
        r.rec.voice_step(true, 3.0);
        assert!(r.rec.session.is_none(), "not before the wait is over");
        r.rec.voice_step(true, 2.0 + VOICE_RETRY_S);
        assert!(r.rec.session.is_some());
        let _ = std::fs::remove_dir_all(&r.root);
    }
}
