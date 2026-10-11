//! App audio session model: the category an app declares (what it does with sound), the mode that refines it,
//! the options, and the activation state. An interruption, such as an incoming call, deactivates the session and
//! records whether the system recommends resuming when it ends. This is a pure model. It does not open a device;
//! the audio service applies the session to the streams it plays and records.

/// What the app does with sound. Input is possible only in the record and play-and-record categories, and output
/// is possible in every category except record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    /// Mixes with other audio and stops when the device is locked or silenced.
    Ambient,
    /// Stops other audio while active.
    #[default]
    SoloAmbient,
    /// Output only, with background playback allowed.
    Playback,
    /// Input only.
    Record,
    /// Input and output.
    PlayAndRecord,
    /// Output on several routes at once.
    MultiRoute,
}

impl Category {
    pub fn can_output(self) -> bool {
        self != Category::Record
    }
    pub fn can_input(self) -> bool {
        matches!(self, Category::Record | Category::PlayAndRecord)
    }
}

/// How the app uses the category. The mode does not change what is allowed, only the processing the service applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Default,
    VoiceChat,
    GameChat,
    VideoRecording,
    Measurement,
    MoviePlayback,
    SpokenAudio,
}

/// Optional behaviours that refine a category.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub mix_with_others: bool,
    pub duck_others: bool,
    pub allow_bluetooth: bool,
    pub default_to_speaker: bool,
}

/// Why the session stopped being active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deactivation {
    /// The app deactivated the session.
    App,
    /// The system interrupted the session.
    Interruption,
}

/// A change the app can read with `take_events`, in the order it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    BecameActive,
    BecameInactive(Deactivation),
    InterruptionBegan,
    /// The interruption ended. `should_resume` is the system's recommendation to resume playback.
    InterruptionEnded { should_resume: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The session cannot be activated while an interruption is in progress.
    Interrupted,
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    category: Category,
    mode: Mode,
    options: Options,
    active: bool,
    interrupted: bool,
    resume_recommended: bool,
    events: Vec<Event>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn category(&self) -> Category {
        self.category
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn options(&self) -> Options {
        self.options
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }
    /// The system's recommendation from the last interruption. Cleared by the next activation.
    pub fn resume_recommended(&self) -> bool {
        self.resume_recommended
    }

    /// Sets the category, mode and options together. Changing the category never changes activation.
    pub fn set_category(&mut self, category: Category, mode: Mode, options: Options) {
        self.category = category;
        self.mode = mode;
        self.options = options;
    }

    /// Activates the session. Fails during an interruption. Activating an active session does nothing and reports
    /// success.
    pub fn activate(&mut self) -> Result<(), SessionError> {
        if self.interrupted {
            return Err(SessionError::Interrupted);
        }
        if self.active {
            return Ok(());
        }
        self.active = true;
        self.resume_recommended = false;
        self.events.push(Event::BecameActive);
        Ok(())
    }

    /// Deactivates the session. Returns false when it was not active.
    pub fn deactivate(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.active = false;
        self.events.push(Event::BecameInactive(Deactivation::App));
        true
    }

    /// The system interrupts the session. Returns false when it was already interrupted.
    pub fn begin_interruption(&mut self) -> bool {
        if self.interrupted {
            return false;
        }
        self.interrupted = true;
        if self.active {
            self.active = false;
            self.events.push(Event::BecameInactive(Deactivation::Interruption));
        }
        self.events.push(Event::InterruptionBegan);
        true
    }

    /// The interruption ends with the system's resume recommendation. Returns false when none was in progress.
    /// The session stays inactive until the app activates it again.
    pub fn end_interruption(&mut self, should_resume: bool) -> bool {
        if !self.interrupted {
            return false;
        }
        self.interrupted = false;
        self.resume_recommended = should_resume;
        self.events.push(Event::InterruptionEnded { should_resume });
        true
    }

    /// Drains the events recorded since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_decides_which_directions_are_allowed() {
        assert!(Category::Playback.can_output() && !Category::Playback.can_input());
        assert!(!Category::Record.can_output() && Category::Record.can_input());
        assert!(Category::PlayAndRecord.can_output() && Category::PlayAndRecord.can_input());
        assert_eq!(Session::new().category(), Category::SoloAmbient);
    }

    #[test]
    fn activate_and_deactivate_report_changes_once() {
        let mut s = Session::new();
        assert_eq!(s.activate(), Ok(()));
        assert_eq!(s.activate(), Ok(()));
        assert!(s.deactivate());
        assert!(!s.deactivate());
        assert_eq!(s.take_events(), vec![Event::BecameActive, Event::BecameInactive(Deactivation::App)]);
        assert!(s.take_events().is_empty());
    }

    #[test]
    fn interruption_deactivates_and_blocks_activation_until_it_ends() {
        let mut s = Session::new();
        s.activate().unwrap();
        assert!(s.begin_interruption());
        assert!(!s.is_active() && s.is_interrupted());
        assert_eq!(s.activate(), Err(SessionError::Interrupted));
        assert!(!s.begin_interruption());
        assert!(s.end_interruption(true));
        assert!(!s.is_interrupted() && !s.is_active());
        assert!(s.resume_recommended());
        assert_eq!(
            s.take_events(),
            vec![
                Event::BecameActive,
                Event::BecameInactive(Deactivation::Interruption),
                Event::InterruptionBegan,
                Event::InterruptionEnded { should_resume: true },
            ]
        );
    }

    #[test]
    fn interruption_while_idle_only_records_the_interruption() {
        let mut s = Session::new();
        assert!(s.begin_interruption());
        assert_eq!(s.take_events(), vec![Event::InterruptionBegan]);
        assert!(s.end_interruption(false));
        assert!(!s.resume_recommended());
        assert!(!s.end_interruption(true), "no interruption is in progress");
    }

    #[test]
    fn a_new_activation_clears_the_resume_recommendation() {
        let mut s = Session::new();
        s.begin_interruption();
        s.end_interruption(true);
        assert!(s.resume_recommended());
        s.activate().unwrap();
        assert!(!s.resume_recommended());
        assert!(s.is_active());
    }

    #[test]
    fn category_change_keeps_activation_and_stores_mode_and_options() {
        let mut s = Session::new();
        s.activate().unwrap();
        let opts = Options { mix_with_others: true, allow_bluetooth: true, ..Options::default() };
        s.set_category(Category::PlayAndRecord, Mode::VoiceChat, opts);
        assert!(s.is_active());
        assert_eq!((s.category(), s.mode()), (Category::PlayAndRecord, Mode::VoiceChat));
        assert!(s.options().mix_with_others && s.options().allow_bluetooth && !s.options().duck_others);
    }
}
