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
/// Speech rate in words per minute (the engine's `-s` option).
pub const RATE_WPM: u32 = 180;
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

/// The engine's arguments for `text`: the rate, then `--` and the text, so text that starts with a dash is not an option.
pub fn args(text: &str) -> Vec<String> {
    vec!["-s".to_string(), RATE_WPM.to_string(), "--".to_string(), clean(text)]
}

/// Speaks announcements through the detected engine. A new utterance stops the one still playing.
#[derive(Debug, Default)]
pub struct Speaker {
    engine: Option<PathBuf>,
    child: Option<Child>,
}

impl Speaker {
    /// A speaker for the engine found on the current `PATH`; silent when there is none.
    pub fn detect() -> Self {
        Speaker { engine: std::env::var("PATH").ok().and_then(|p| find_engine(&p)), child: None }
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
        match Command::new(&engine).args(args(&spoken)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
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
        let a = args("-rf now");
        assert_eq!(a, vec!["-s", "180", "--", "-rf now"]);
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
        let mut s = Speaker { engine: Some(PathBuf::from("/nonexistent/engine")), child: None };
        assert!(!s.say(" \t "));
        assert!(s.child.is_none());
    }
}
