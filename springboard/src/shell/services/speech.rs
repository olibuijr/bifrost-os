//! The `speech` service: text to speech for apps, through the system engine (`espeak-ng`, else `espeak`, see
//! `bifrost_core::speech_engine`). The SDK client is `bifrost_app::speech`.
//!
//! Each app has at most one utterance at a time: a new `speak` replaces its current one. The engine runs as a child
//! process of the shell; the text goes to its standard input, written off the shell thread. Stopping, or the app
//! quitting, kills the child.
//!
//! Calls (JSON params):
//! - `speak` `{text, voice?, rate?}` starts an utterance; replies `{id}`. `text` must not be blank and is limited to
//!   [`MAX_TEXT`] bytes. `voice` is an engine voice name (see `voices`). `rate` is words per minute, a whole number from
//!   [`MIN_RATE`] to [`MAX_RATE`].
//! - `stop` `{}` ends the app's utterance; replies `{stopped}`.
//! - `pause` `{}` stops the engine with SIGSTOP; replies `{paused}`. `continue` `{}` resumes it with SIGCONT; replies `{paused}`.
//! - `is_speaking` `{}` replies `{speaking, paused}`.
//! - `voices` `{}` replies `{voices: [{name, language}], loading}`. The listing is read once, in a thread, on the first
//!   call: until it is ready the reply has no voices and `loading` true, so apps ask again later.
//!
//! An engine that is not installed fails `speak` and `voices` with "no speech engine is installed". No event is sent.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::speech_engine::{engine_args, parse_voices, voice_name_ok, Voice, ENGINES, MAX_RATE, MIN_RATE, NO_ENGINE};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The name the service is registered under.
pub const SERVICE: &str = "speech";
/// Longest text one utterance may carry, in bytes.
pub const MAX_TEXT: usize = 16 * 1024;

unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
/// The Linux signal numbers, the same on x86_64 and aarch64.
const SIGCONT: i32 = 18;
const SIGSTOP: i32 = 19;

/// One engine process, as the service drives it. Pause and resume return whether the engine is paused (`pause`) or
/// running (`resume`) afterwards.
pub trait Process {
    fn pause(&mut self) -> bool;
    fn resume(&mut self) -> bool;
    /// True once the engine has exited (this reaps it).
    fn finished(&mut self) -> bool;
    fn kill(&mut self);
}

/// Starts an engine for `args`, speaking `text`. Fails with [`NO_ENGINE`] when no engine program is installed.
pub trait Spawner {
    fn start(&mut self, args: &[String], text: Vec<u8>) -> Result<Box<dyn Process>, String>;
}

/// The real spawner: the first engine on the path.
struct EngineSpawner;

impl Spawner for EngineSpawner {
    fn start(&mut self, args: &[String], text: Vec<u8>) -> Result<Box<dyn Process>, String> {
        for engine in ENGINES {
            let spawned = Command::new(engine).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            let mut child = match spawned {
                Ok(child) => child,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{engine}: {e}")),
            };
            if let Some(mut stdin) = child.stdin.take() {
                // a long text fills the pipe while the engine speaks, so the write waits in its own thread
                std::thread::spawn(move || {
                    let _ = stdin.write_all(&text);
                });
            }
            return Ok(Box::new(Engine { child, paused: false }));
        }
        Err(NO_ENGINE.to_string())
    }
}

/// An engine child. Its pid is still held (as a zombie at worst) until `finished` reaps it, so signals never reach a
/// reused pid.
struct Engine {
    child: Child,
    paused: bool,
}

impl Process for Engine {
    fn pause(&mut self) -> bool {
        if !self.paused && signal(self.child.id(), SIGSTOP) {
            self.paused = true;
        }
        self.paused
    }

    fn resume(&mut self) -> bool {
        if self.paused && signal(self.child.id(), SIGCONT) {
            self.paused = false;
        }
        !self.paused
    }

    fn finished(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn signal(pid: u32, sig: i32) -> bool {
    // SAFETY: `kill` only sends a signal to a child this service started and has not reaped.
    unsafe { kill(pid as i32, sig) == 0 }
}

/// The voice listing, read once by a thread.
#[derive(Clone, Debug, PartialEq)]
enum Voices {
    Idle,
    Loading,
    Ready(Vec<Voice>),
    Failed(String),
}

/// The app's utterance: its id, its engine and whether the engine is paused.
struct Speaking {
    #[allow(dead_code)]
    id: u32,
    process: Box<dyn Process>,
    paused: bool,
}

pub struct Speech {
    spawner: Box<dyn Spawner>,
    voices: Arc<Mutex<Voices>>,
    speaking: BTreeMap<String, Speaking>,
    next: u32,
}

impl Default for Speech {
    fn default() -> Self {
        Self::with_spawner(Box::new(EngineSpawner))
    }
}

impl Speech {
    fn with_spawner(spawner: Box<dyn Spawner>) -> Self {
        Speech { spawner, voices: Arc::new(Mutex::new(Voices::Idle)), speaking: BTreeMap::new(), next: 1 }
    }

    /// Ends `app`'s utterance. True when there was one.
    fn stop_app(&mut self, app: &str) -> bool {
        match self.speaking.remove(app) {
            Some(mut s) => {
                s.process.kill();
                true
            }
            None => false,
        }
    }

    /// Drops the utterances whose engine has exited.
    fn reap(&mut self) {
        self.speaking.retain(|_, s| !s.process.finished());
    }

    /// Ends the utterances of every app that `running` does not name: an app that quits takes its speech with it.
    fn prune(&mut self, running: impl Fn(&str) -> bool) {
        let gone: Vec<String> = self.speaking.keys().filter(|app| !running(app.as_str())).cloned().collect();
        for app in gone {
            self.stop_app(&app);
        }
    }

    /// Answers one call from `app`. The shell's [`SystemService::call`] delegates here, so the rules can be tested
    /// without a shell.
    fn handle(&mut self, app: &str, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "speak" => {
                let text = params.str_field("text").filter(|t| !t.trim().is_empty()).ok_or("speech: speak needs text")?;
                if text.len() > MAX_TEXT {
                    return Err(format!("speech: text is longer than {MAX_TEXT} bytes"));
                }
                let voice = match params.str_field("voice") {
                    None => None,
                    Some(v) if voice_name_ok(&v) => Some(v),
                    Some(v) => return Err(format!("speech: {v:?} is not a voice name")),
                };
                let rate = match params.get("rate") {
                    None | Some(Json::Null) => None,
                    Some(j) => {
                        let r = j
                            .number()
                            .filter(|n| n.fract() == 0.0 && *n >= MIN_RATE as f64 && *n <= MAX_RATE as f64)
                            .ok_or(format!("speech: rate must be a whole number from {MIN_RATE} to {MAX_RATE}"))?;
                        Some(r as u32)
                    }
                };
                let args = engine_args(voice.as_deref(), rate, None);
                // start first: when the engine is missing the app keeps what it had
                let process = self.spawner.start(&args, text.into_bytes())?;
                self.stop_app(app);
                let id = self.next;
                self.next = self.next.checked_add(1).unwrap_or(1);
                self.speaking.insert(app.to_string(), Speaking { id, process, paused: false });
                Ok(obj(vec![("id", Json::Num(id as f64))]))
            }
            "stop" => Ok(obj(vec![("stopped", Json::Bool(self.stop_app(app)))])),
            "pause" => {
                let paused = match self.speaking.get_mut(app) {
                    Some(s) => {
                        s.paused = s.process.pause();
                        s.paused
                    }
                    None => false,
                };
                Ok(obj(vec![("paused", Json::Bool(paused))]))
            }
            "continue" => {
                let paused = match self.speaking.get_mut(app) {
                    Some(s) => {
                        s.paused = !s.process.resume();
                        s.paused
                    }
                    None => false,
                };
                Ok(obj(vec![("paused", Json::Bool(paused))]))
            }
            "is_speaking" => {
                self.reap();
                let (speaking, paused) = match self.speaking.get(app) {
                    Some(s) => (true, s.paused),
                    None => (false, false),
                };
                Ok(obj(vec![("speaking", Json::Bool(speaking)), ("paused", Json::Bool(paused))]))
            }
            "voices" => {
                self.start_voices();
                let state = self.voices.lock().map_err(|_| "speech: the voice listing is unavailable".to_string())?;
                match &*state {
                    Voices::Idle | Voices::Loading => Ok(obj(vec![("voices", Json::Arr(Vec::new())), ("loading", Json::Bool(true))])),
                    Voices::Ready(list) => Ok(obj(vec![("voices", voices_json(list)), ("loading", Json::Bool(false))])),
                    Voices::Failed(e) => Err(e.clone()),
                }
            }
            other => Err(format!("speech: no method {other}")),
        }
    }

    /// Starts the voice listing the first time it is asked for.
    fn start_voices(&mut self) {
        let started = match self.voices.lock() {
            Ok(mut state) if *state == Voices::Idle => {
                *state = Voices::Loading;
                true
            }
            _ => false,
        };
        if !started {
            return;
        }
        let shared = Arc::clone(&self.voices);
        std::thread::spawn(move || {
            let result = read_voices();
            if let Ok(mut state) = shared.lock() {
                *state = match result {
                    Ok(list) => Voices::Ready(list),
                    Err(e) => Voices::Failed(e),
                };
            }
        });
    }
}

/// Runs the engine's voice listing and parses it.
fn read_voices() -> Result<Vec<Voice>, String> {
    for engine in ENGINES {
        match Command::new(engine).arg("--voices").stdin(Stdio::null()).stderr(Stdio::null()).output() {
            Ok(out) => return Ok(parse_voices(&String::from_utf8_lossy(&out.stdout))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{engine}: {e}")),
        }
    }
    Err(NO_ENGINE.to_string())
}

fn voices_json(list: &[Voice]) -> Json {
    Json::Arr(list.iter().map(|v| obj(vec![("name", Json::Str(v.name.clone())), ("language", Json::Str(v.language.clone()))])).collect())
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

impl SystemService for Speech {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.handle(app_id, method, params)
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        let running = |app: &str| crate::shell::apphost::slot_of(app).is_some_and(|slot| shell.ext.get(slot).is_some_and(|h| h.is_some()));
        self.prune(running);
        self.reap();
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// What the fake engines did, shared with the test.
    #[derive(Default)]
    struct Log {
        starts: Vec<(Vec<String>, Vec<u8>)>,
        kills: usize,
        pauses: usize,
        resumes: usize,
        /// One flag per started engine: set it to make that engine look exited.
        exited: Vec<Arc<AtomicBool>>,
        missing: bool,
    }

    struct Fake(Arc<Mutex<Log>>);

    struct FakeEngine {
        log: Arc<Mutex<Log>>,
        exited: Arc<AtomicBool>,
        paused: bool,
    }

    impl Spawner for Fake {
        fn start(&mut self, args: &[String], text: Vec<u8>) -> Result<Box<dyn Process>, String> {
            let mut log = self.0.lock().unwrap();
            if log.missing {
                return Err(NO_ENGINE.to_string());
            }
            log.starts.push((args.to_vec(), text));
            let exited = Arc::new(AtomicBool::new(false));
            log.exited.push(Arc::clone(&exited));
            Ok(Box::new(FakeEngine { log: Arc::clone(&self.0), exited, paused: false }))
        }
    }

    impl Process for FakeEngine {
        fn pause(&mut self) -> bool {
            if !self.paused {
                self.paused = true;
                self.log.lock().unwrap().pauses += 1;
            }
            self.paused
        }
        fn resume(&mut self) -> bool {
            if self.paused {
                self.paused = false;
                self.log.lock().unwrap().resumes += 1;
            }
            !self.paused
        }
        fn finished(&mut self) -> bool {
            self.exited.load(Ordering::SeqCst)
        }
        fn kill(&mut self) {
            self.log.lock().unwrap().kills += 1;
        }
    }

    fn service(missing: bool) -> (Speech, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log { missing, ..Log::default() }));
        (Speech::with_spawner(Box::new(Fake(Arc::clone(&log)))), log)
    }

    fn params(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    fn number(reply: &Result<Json, String>, key: &str) -> f64 {
        reply.as_ref().unwrap().get(key).and_then(Json::number).unwrap()
    }

    fn flag(reply: &Result<Json, String>, key: &str) -> bool {
        match reply.as_ref().unwrap().get(key) {
            Some(Json::Bool(b)) => *b,
            other => panic!("no flag {key}: {other:?}"),
        }
    }

    #[test]
    fn speak_passes_the_engine_arguments_and_the_text() {
        let (mut s, log) = service(false);
        let r = s.handle("app.a", "speak", &params(r#"{"text":"hello there","voice":"is","rate":160}"#));
        assert_eq!(number(&r, "id"), 1.0);
        let log = log.lock().unwrap();
        assert_eq!(log.starts[0].0, vec!["-v", "is", "-s", "160"]);
        assert_eq!(log.starts[0].1, b"hello there".to_vec());
    }

    #[test]
    fn speak_refuses_bad_input_before_starting_an_engine() {
        let (mut s, log) = service(false);
        assert!(s.handle("app.a", "speak", &params(r#"{"text":"   "}"#)).is_err());
        assert!(s.handle("app.a", "speak", &params(r#"{}"#)).is_err());
        assert!(s.handle("app.a", "speak", &params(r#"{"text":"x","rate":5000}"#)).is_err());
        assert!(s.handle("app.a", "speak", &params(r#"{"text":"x","rate":120.5}"#)).is_err());
        assert!(s.handle("app.a", "speak", &params(r#"{"text":"x","voice":"-v"}"#)).is_err());
        let long = format!(r#"{{"text":"{}"}}"#, "a".repeat(MAX_TEXT + 1));
        assert!(s.handle("app.a", "speak", &params(&long)).is_err());
        assert!(log.lock().unwrap().starts.is_empty());
    }

    #[test]
    fn a_missing_engine_is_reported_and_nothing_is_recorded() {
        let (mut s, _) = service(true);
        let r = s.handle("app.a", "speak", &params(r#"{"text":"hi"}"#));
        assert_eq!(r, Err(NO_ENGINE.to_string()));
        assert!(!flag(&s.handle("app.a", "is_speaking", &Json::Null), "speaking"));
    }

    #[test]
    fn a_new_utterance_replaces_the_apps_current_one() {
        let (mut s, log) = service(false);
        let first = s.handle("app.a", "speak", &params(r#"{"text":"one"}"#));
        let second = s.handle("app.a", "speak", &params(r#"{"text":"two"}"#));
        assert_eq!(number(&second, "id"), number(&first, "id") + 1.0);
        let log = log.lock().unwrap();
        assert_eq!(log.kills, 1, "the first engine is killed");
        assert_eq!(log.starts.len(), 2);
    }

    #[test]
    fn stop_kills_the_engine_once() {
        let (mut s, log) = service(false);
        assert!(!flag(&s.handle("app.a", "stop", &Json::Null), "stopped"));
        s.handle("app.a", "speak", &params(r#"{"text":"x"}"#)).unwrap();
        assert!(flag(&s.handle("app.a", "stop", &Json::Null), "stopped"));
        assert!(!flag(&s.handle("app.a", "stop", &Json::Null), "stopped"));
        assert_eq!(log.lock().unwrap().kills, 1);
    }

    #[test]
    fn pause_and_continue_track_the_engine_state() {
        let (mut s, log) = service(false);
        s.handle("app.a", "speak", &params(r#"{"text":"x"}"#)).unwrap();
        assert!(flag(&s.handle("app.a", "pause", &Json::Null), "paused"));
        let speaking = s.handle("app.a", "is_speaking", &Json::Null);
        assert!(flag(&speaking, "speaking") && flag(&speaking, "paused"));
        assert!(!flag(&s.handle("app.a", "continue", &Json::Null), "paused"));
        let log = log.lock().unwrap();
        assert_eq!((log.pauses, log.resumes), (1, 1));
    }

    #[test]
    fn an_engine_that_exits_is_no_longer_speaking() {
        let (mut s, log) = service(false);
        s.handle("app.a", "speak", &params(r#"{"text":"x"}"#)).unwrap();
        log.lock().unwrap().exited[0].store(true, Ordering::SeqCst);
        assert!(!flag(&s.handle("app.a", "is_speaking", &Json::Null), "speaking"));
    }

    #[test]
    fn quitting_an_app_stops_its_speech_and_keeps_the_others() {
        let (mut s, log) = service(false);
        s.handle("app.a", "speak", &params(r#"{"text":"x"}"#)).unwrap();
        s.handle("app.b", "speak", &params(r#"{"text":"y"}"#)).unwrap();
        s.prune(|app| app == "app.b");
        assert_eq!(log.lock().unwrap().kills, 1);
        assert!(!flag(&s.handle("app.a", "is_speaking", &Json::Null), "speaking"));
        assert!(flag(&s.handle("app.b", "is_speaking", &Json::Null), "speaking"));
    }

    #[test]
    fn voices_are_listed_from_the_cache_and_a_missing_engine_is_an_error() {
        let (mut s, _) = service(false);
        *s.voices.lock().unwrap() = Voices::Ready(vec![Voice { name: "is".into(), language: "is".into() }]);
        let r = s.handle("app.a", "voices", &Json::Null);
        let list = r.as_ref().unwrap().get("voices").and_then(Json::as_array).unwrap();
        assert_eq!(list[0].str_field("name").as_deref(), Some("is"));
        assert!(!flag(&r, "loading"));

        *s.voices.lock().unwrap() = Voices::Failed(NO_ENGINE.to_string());
        assert_eq!(s.handle("app.a", "voices", &Json::Null), Err(NO_ENGINE.to_string()));
    }

    #[test]
    fn voices_say_they_are_loading_until_the_listing_arrives() {
        let (mut s, _) = service(false);
        *s.voices.lock().unwrap() = Voices::Loading;
        let r = s.handle("app.a", "voices", &Json::Null);
        assert!(flag(&r, "loading"));
    }

    #[test]
    fn an_unknown_method_is_an_error() {
        let (mut s, _) = service(false);
        assert!(s.handle("app.a", "sing", &Json::Null).unwrap_err().contains("no method sing"));
    }
}
