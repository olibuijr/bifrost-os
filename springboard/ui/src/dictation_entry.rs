//! Dictation entry point: the state of a system dictation request, as a text control's dictation key drives it. The model
//! starts listening only when the environment reports dictation as available, collects the recognized text while it
//! listens, and hands the text to the control on stop. The speech recognizer itself is not in this module; the
//! environment reports whether one is present and feeds it the recognized phrases. Mirrors the reference dictation
//! entry, which starts and stops on the system dictation key and delivers text to the focused control.

/// Why dictation cannot start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// no speech recognizer is installed
    NoRecognizer,
    /// the user has turned dictation off in preferences
    Disabled,
}

/// What the environment reports about dictation before a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(Reason),
}

/// The state of one dictation session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Listening,
}

/// One dictation session attached to a text control.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictation {
    state: State,
    text: String,
}

impl Dictation {
    pub fn new() -> Self {
        Dictation::default()
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Starts listening when `availability` allows it. A request while already listening keeps the session as it is.
    pub fn start(&mut self, availability: Availability) -> Result<(), Reason> {
        match availability {
            Availability::Unavailable(reason) => Err(reason),
            Availability::Available => {
                self.state = State::Listening;
                Ok(())
            }
        }
    }

    /// Stops listening. The text recognized so far stays until it is taken.
    pub fn stop(&mut self) {
        self.state = State::Idle;
    }

    /// Adds a recognized phrase while listening, separated from the previous one by one space. Phrases that arrive while
    /// idle are dropped and the call answers false.
    pub fn deliver(&mut self, phrase: &str) -> bool {
        if self.state != State::Listening {
            return false;
        }
        let phrase = phrase.trim();
        if phrase.is_empty() {
            return true;
        }
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        self.text.push_str(phrase);
        true
    }

    /// The recognized text so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the recognized text and clears it, as the control does when it inserts the text.
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }
}
