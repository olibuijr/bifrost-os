//! Edit > Speak Serial Number: reads the serial aloud through the system speech engine (`espeak-ng`, else `espeak`).
//! The text goes to the engine on its standard input. Speech runs whether or not the screen reader is on, because the
//! user chose the menu item; a new utterance ends the one in progress, and speech stops when the app quits.

use std::cell::RefCell;
use std::io::Write;
use std::process::{Child, Command, Stdio};

/// The engines tried in order: the packaged one first, then the older name.
pub const ENGINES: &[&str] = &["espeak-ng", "espeak"];

#[derive(Debug, PartialEq, Eq)]
pub enum SpeakError {
    /// There is nothing but blank space to read.
    NoText,
    /// None of the engines is installed.
    NoEngine,
}

/// Control characters other than line breaks become spaces, and the ends are trimmed.
pub fn clean(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

/// The running engine process, if any.
#[derive(Default)]
pub struct Speaker {
    child: RefCell<Option<Child>>,
}

impl Speaker {
    /// Speaks `text`, ending any speech in progress first.
    pub fn start(&self, text: &str) -> Result<(), SpeakError> {
        self.start_with(ENGINES, text)
    }

    /// Like [`Speaker::start`] with the engines to try.
    pub fn start_with(&self, engines: &[&str], text: &str) -> Result<(), SpeakError> {
        let text = clean(text);
        if text.is_empty() {
            return Err(SpeakError::NoText);
        }
        self.stop();
        for engine in engines {
            let spawned = Command::new(engine).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            let Ok(mut child) = spawned else { continue };
            if let Some(mut stdin) = child.stdin.take() {
                // a thread writes, so a long text cannot block the app; dropping the pipe ends the engine's input
                let bytes = text.as_bytes().to_vec();
                bifrost_app::dispatch::DispatchQueue::global(bifrost_app::dispatch::Qos::Utility).async_(move || {
                    let _ = stdin.write_all(&bytes);
                });
            }
            *self.child.borrow_mut() = Some(child);
            return Ok(());
        }
        Err(SpeakError::NoEngine)
    }

    /// Ends the speech. True when the engine was still speaking.
    pub fn stop(&self) -> bool {
        let taken = self.child.borrow_mut().take();
        let Some(mut child) = taken else { return false };
        let running = matches!(child.try_wait(), Ok(None));
        let _ = child.kill();
        let _ = child.wait();
        running
    }

    /// True while the engine process runs. A finished process is collected here.
    #[cfg(test)]
    pub fn is_speaking(&self) -> bool {
        let mut slot = self.child.borrow_mut();
        let state = slot.as_mut().map(|child| child.try_wait());
        match state {
            Some(Ok(None)) => true,
            Some(_) => {
                *slot = None;
                false
            }
            None => false,
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
    fn clean_keeps_line_breaks_and_replaces_other_control_characters() {
        assert_eq!(clean("  a\tb\u{7}c\nd \n"), "a b c\nd");
    }

    #[test]
    fn a_missing_engine_is_reported_and_nothing_runs() {
        let speaker = Speaker::default();
        assert_eq!(speaker.start_with(&["bifrost-no-such-speech-engine"], "Serial number C 0 2 X 1"), Err(SpeakError::NoEngine));
        assert!(!speaker.is_speaking());
        assert!(!speaker.stop());
    }

    #[test]
    fn blank_text_is_refused_before_any_engine_is_tried() {
        let speaker = Speaker::default();
        assert_eq!(speaker.start_with(&["bifrost-no-such-speech-engine"], " \n"), Err(SpeakError::NoText));
    }
}
