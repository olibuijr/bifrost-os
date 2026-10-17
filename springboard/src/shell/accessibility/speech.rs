//! Spoken output for the screen reader. Announcements (the reader's moves and the apps' `announce` calls) are spoken
//! through an installed speech engine, `espeak-ng` or `espeak`, found on `PATH`. With no engine the screen reader stays
//! silent and the banner still shows the text, so nothing breaks on a machine without one.
//!
//! One utterance plays at a time: a new announcement stops the one still speaking, as a focus move does on the reference
//! platform. The engine runs without a shell (the text is one argument after `--`), with its output discarded.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The engines tried in order.
pub const ENGINES: [&str; 2] = ["espeak-ng", "espeak"];
/// The slowest and fastest rate the engine is asked for, in words per minute (the engine's `-s` option).
pub const MIN_RATE: u32 = 80;
pub const MAX_RATE: u32 = 450;
/// The highest pitch the engine takes (its `-p` option; 0 is the lowest).
pub const MAX_PITCH: u32 = 99;
/// The longest text spoken, in characters; a longer announcement is cut.
pub const MAX_CHARS: usize = 400;

/// The first engine found in `path` (a `:`-separated list of directories), trying the names in `ENGINES` order.
pub fn find_engine(path: &str) -> Option<PathBuf> {
    find_in(path, &ENGINES)
}

fn find_in(path: &str, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        for dir in path.split(':').filter(|d| !d.is_empty()) {
            let candidate = Path::new(dir).join(name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The text as it is spoken: control characters become spaces, runs of blanks collapse to one, and the result is cut to
/// `MAX_CHARS` characters. Empty when nothing speakable is left.
pub fn clean(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(MAX_CHARS).collect()
}

/// How much punctuation the engine reads aloud (Settings > Accessibility > Screen Reader > Speech > Punctuation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Punctuation {
    /// Punctuation marks are not named; the engine only pauses at them.
    #[default]
    Off,
    /// The symbols that change what a text means are named (@ # $ % & * + = < > / \ | ^ ~ _ and brackets), not the marks of
    /// ordinary sentences.
    Symbols,
    /// Every punctuation mark is named.
    All,
}

/// The characters `Punctuation::Symbols` names (the engine's `--punct=` list).
pub const SYMBOLS: &str = "@#$%&*+=<>/\\|^~_[]{}";

/// The voice the screen reader speaks with, from Settings > Accessibility > Screen Reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// words per minute, clamped to `MIN_RATE..=MAX_RATE` when spoken
    pub rate: u32,
    /// 0 to `MAX_PITCH`
    pub pitch: u32,
    /// an engine voice name; empty for the engine's default voice
    pub voice: String,
    /// how much punctuation is named
    pub punctuation: Punctuation,
}

impl Default for Config {
    fn default() -> Self {
        Config { rate: 180, pitch: 50, voice: String::new(), punctuation: Punctuation::Off }
    }
}

/// The engine's arguments for `text` under `config`: the rate, the pitch, the voice when one is set, the punctuation to name
/// when any is, then `--` and the text, so text that starts with a dash is not an option. Out-of-range values are clamped; a voice name that is not a plain word
/// is left out.
pub fn args(text: &str, config: &Config) -> Vec<String> {
    let mut out = vec!["-s".to_string(), config.rate.clamp(MIN_RATE, MAX_RATE).to_string(), "-p".to_string(), config.pitch.min(MAX_PITCH).to_string()];
    if bifrost_core::speech_engine::voice_name_ok(&config.voice) {
        out.push("-v".to_string());
        out.push(config.voice.clone());
    }
    match config.punctuation {
        Punctuation::Off => {}
        Punctuation::Symbols => out.push(format!("--punct={SYMBOLS}")),
        Punctuation::All => out.push("--punct".to_string()),
    }
    out.push("--".to_string());
    out.push(clean(text));
    out
}

/// Speaks announcements through the detected engine. A new utterance stops the one still playing.
#[derive(Debug, Default)]
pub struct Speaker {
    engine: Option<PathBuf>,
    child: Option<Child>,
    config: Config,
}

impl Speaker {
    /// A speaker for the engine found on the current `PATH`; silent when there is none.
    pub fn detect() -> Self {
        Speaker { engine: std::env::var("PATH").ok().and_then(|p| find_engine(&p)), child: None, config: Config::default() }
    }

    /// Sets the rate, pitch and voice the next utterances use. An utterance in progress keeps its own.
    pub fn configure(&mut self, config: Config) {
        self.config = config;
    }

    /// The voice the next utterance uses.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Whether an engine was found.
    pub fn available(&self) -> bool {
        self.engine.is_some()
    }

    /// Speaks `text`, stopping the utterance in progress. Returns false when there is no engine, the text is blank, or
    /// the engine could not start.
    pub fn say(&mut self, text: &str) -> bool {
        let spoken = clean(text);
        let Some(engine) = self.engine.clone() else { return false };
        if spoken.is_empty() {
            return false;
        }
        self.stop();
        match Command::new(&engine).args(args(&spoken, &self.config)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(child) => {
                self.child = Some(child);
                true
            }
            Err(e) => {
                eprintln!("springboard: speech engine {}: {e}", engine.display());
                false
            }
        }
    }

    /// Stops the utterance in progress, if any, and reaps the engine.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_collapses_blanks_and_drops_control_characters() {
        assert_eq!(clean("  button,\tClose\n\x07 "), "button, Close");
        assert_eq!(clean(" \n\t "), "");
    }

    #[test]
    fn clean_cuts_long_text_on_a_character_boundary() {
        let long: String = "\u{00e9}".repeat(MAX_CHARS + 50);
        let out = clean(&long);
        assert_eq!(out.chars().count(), MAX_CHARS);
        assert!(out.chars().all(|c| c == '\u{00e9}'));
    }

    #[test]
    fn args_put_the_text_after_a_separator() {
        let a = args("-rf now", &Config::default());
        assert_eq!(a, vec!["-s", "180", "-p", "50", "--", "-rf now"]);
    }

    #[test]
    fn args_carry_the_chosen_rate_pitch_and_voice() {
        let c = Config { rate: 240, pitch: 70, voice: "is".into(), ..Config::default() };
        assert_eq!(args("Halló", &c), vec!["-s", "240", "-p", "70", "-v", "is", "--", "Halló"]);
    }

    #[test]
    fn punctuation_is_named_only_when_asked_for() {
        let off = args("a, b", &Config::default());
        assert!(!off.iter().any(|a| a.starts_with("--punct")), "off: the engine's own behaviour");
        let symbols = args("a@b", &Config { punctuation: Punctuation::Symbols, ..Config::default() });
        assert_eq!(symbols, vec!["-s".to_string(), "180".into(), "-p".into(), "50".into(), format!("--punct={SYMBOLS}"), "--".into(), "a@b".into()]);
        let all = args("a, b", &Config { punctuation: Punctuation::All, ..Config::default() });
        assert_eq!(all, vec!["-s", "180", "-p", "50", "--punct", "--", "a, b"], "all: the bare option names every mark");
        // the option comes before the separator, so text that looks like one cannot turn it on
        let text = args("--punct", &Config::default());
        assert_eq!(text, vec!["-s", "180", "-p", "50", "--", "--punct"]);
    }

    #[test]
    fn the_symbols_list_has_no_ordinary_sentence_marks() {
        for mark in ['.', ',', '?', '!', ':', ';', '\'', '"'] {
            assert!(!SYMBOLS.contains(mark), "{mark}");
        }
        for mark in ['@', '#', '$', '%', '/', '\\'] {
            assert!(SYMBOLS.contains(mark), "{mark}");
        }
    }

    #[test]
    fn args_clamp_the_rate_and_pitch_and_drop_a_bad_voice() {
        let c = Config { rate: 10, pitch: 500, voice: "-x y".into(), ..Config::default() };
        assert_eq!(args("hi", &c), vec!["-s", "80", "-p", "99", "--", "hi"]);
        let fast = Config { rate: 9000, ..Config::default() };
        assert_eq!(args("hi", &fast)[1], "450");
    }

    #[test]
    fn configure_replaces_the_voice_for_the_next_utterance() {
        let mut s = Speaker::default();
        assert_eq!(s.config(), &Config::default());
        s.configure(Config { rate: 140, pitch: 30, voice: "en-gb".into(), ..Config::default() });
        assert_eq!(s.config().rate, 140);
        assert_eq!(s.config().voice, "en-gb");
    }

    #[test]
    fn find_in_returns_the_first_executable_on_the_path() {
        let dir = std::env::temp_dir().join(format!("bifrost-speech-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = dir.join("espeak");
        std::fs::write(&engine, b"#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        let not_exec = dir.join("espeak-ng");
        std::fs::write(&not_exec, b"x").unwrap();
        std::fs::set_permissions(&not_exec, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = format!(":{}", dir.display());
        // espeak-ng is listed first but is not executable, so the search falls through to espeak
        assert_eq!(find_in(&path, &["espeak-ng", "espeak"]), Some(engine.clone()));
        assert_eq!(find_in("", &ENGINES), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_silent_speaker_says_nothing() {
        let mut s = Speaker::default();
        assert!(!s.available());
        assert!(!s.say("button, Close"));
        s.stop();
    }

    #[test]
    fn blank_text_is_never_spoken() {
        let mut s = Speaker { engine: Some(PathBuf::from("/nonexistent/engine")), child: None, config: Config::default() };
        assert!(!s.say(" \t "));
        assert!(s.child.is_none());
    }
}
