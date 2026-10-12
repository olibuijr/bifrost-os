//! Microphone consent and ownership: whether the person has allowed recording, whether input is muted, and which app
//! holds the one recording the audio service runs at a time. Pure state with no I/O. The audio service is the only
//! process that opens the microphone; this model decides whether an app may ask it to start. The shell shows the
//! prompt and reports the answer with `answer`. Consent is asked once: after an answer, the stored value stands.

/// What the person has decided about recording. `Undetermined` until the prompt is answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Consent {
    #[default]
    Undetermined,
    Granted,
    Denied,
}

/// Why a recording could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The person has not answered the prompt yet; the shell should ask first.
    NeedsPrompt,
    /// The person refused recording.
    Denied,
    /// Input is muted, so a recording would hold silence.
    Muted,
    /// Another app holds the recording; the name is the holder.
    Busy(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// The prompt was already answered; the stored answer is returned.
    AlreadyAnswered(Consent),
}

/// The microphone as one app sees it: consent, the mute switch and the current owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Microphone {
    consent: Consent,
    muted: bool,
    owner: Option<String>,
}

impl Microphone {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn consent(&self) -> Consent {
        self.consent
    }

    /// Records the person's answer to the prompt. Returns the stored answer, or an error when one is already stored.
    pub fn answer(&mut self, granted: bool) -> Result<Consent, AnswerError> {
        if self.consent != Consent::Undetermined {
            return Err(AnswerError::AlreadyAnswered(self.consent));
        }
        self.consent = if granted { Consent::Granted } else { Consent::Denied };
        Ok(self.consent)
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Sets the input mute switch. A recording that is already running keeps its owner and records silence.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The app that holds the recording, if any.
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// True when a recording could start now: consent is granted and input is not muted.
    pub fn can_record(&self) -> bool {
        self.consent == Consent::Granted && !self.muted
    }

    /// Asks to start a recording for `app`. Checks run in this order: consent, mute, then ownership. An app that
    /// already holds the recording gets `Ok` again without a change.
    pub fn begin(&mut self, app: &str) -> Result<(), StartError> {
        match self.consent {
            Consent::Undetermined => return Err(StartError::NeedsPrompt),
            Consent::Denied => return Err(StartError::Denied),
            Consent::Granted => {}
        }
        if self.muted {
            return Err(StartError::Muted);
        }
        if let Some(holder) = &self.owner {
            if holder == app {
                return Ok(());
            }
            return Err(StartError::Busy(holder.clone()));
        }
        self.owner = Some(app.to_string());
        Ok(())
    }

    /// Releases the recording. Only the holder can end it; the result is false for any other app.
    pub fn end(&mut self, app: &str) -> bool {
        if self.owner.as_deref() == Some(app) {
            self.owner = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_microphone_asks_first() {
        let mut mic = Microphone::new();
        assert_eq!(mic.consent(), Consent::Undetermined);
        assert!(!mic.is_muted());
        assert!(!mic.can_record());
        assert_eq!(mic.begin("notes"), Err(StartError::NeedsPrompt));
        assert_eq!(mic.owner(), None);
    }

    #[test]
    fn the_answer_is_stored_once() {
        let mut mic = Microphone::new();
        assert_eq!(mic.answer(true), Ok(Consent::Granted));
        assert_eq!(mic.answer(false), Err(AnswerError::AlreadyAnswered(Consent::Granted)));
        assert_eq!(mic.consent(), Consent::Granted);
    }

    #[test]
    fn a_refused_microphone_never_starts() {
        let mut mic = Microphone::new();
        assert_eq!(mic.answer(false), Ok(Consent::Denied));
        assert_eq!(mic.begin("voice-memos"), Err(StartError::Denied));
        assert!(!mic.can_record());
        assert_eq!(mic.owner(), None);
    }

    #[test]
    fn a_granted_app_owns_the_recording_and_can_start_again() {
        let mut mic = Microphone::new();
        mic.answer(true).unwrap();
        assert!(mic.can_record());
        assert_eq!(mic.begin("notes"), Ok(()));
        assert_eq!(mic.owner(), Some("notes"));
        assert_eq!(mic.begin("notes"), Ok(()));
        assert_eq!(mic.owner(), Some("notes"));
    }

    #[test]
    fn a_second_app_is_refused_while_the_first_records() {
        let mut mic = Microphone::new();
        mic.answer(true).unwrap();
        mic.begin("notes").unwrap();
        assert_eq!(mic.begin("music"), Err(StartError::Busy("notes".to_string())));
        assert!(!mic.end("music"));
        assert_eq!(mic.owner(), Some("notes"));
    }

    #[test]
    fn only_the_holder_ends_the_recording() {
        let mut mic = Microphone::new();
        mic.answer(true).unwrap();
        mic.begin("notes").unwrap();
        assert!(!mic.end("music"));
        assert!(mic.end("notes"));
        assert_eq!(mic.owner(), None);
        assert_eq!(mic.begin("music"), Ok(()));
    }

    #[test]
    fn a_muted_input_blocks_the_start_until_unmuted() {
        let mut mic = Microphone::new();
        mic.answer(true).unwrap();
        mic.set_muted(true);
        assert!(!mic.can_record());
        assert_eq!(mic.begin("notes"), Err(StartError::Muted));
        mic.set_muted(false);
        assert!(mic.can_record());
        assert_eq!(mic.begin("notes"), Ok(()));
    }
}
