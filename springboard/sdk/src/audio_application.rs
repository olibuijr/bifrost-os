//! Application-level audio state (`AVAudioApplication` parity): the microphone permission the app holds, and whether
//! the user has muted this app's input.
//!
//! Rules of this model:
//! - The permission starts `Undetermined`. `request_record_permission` asks the user only while it is undetermined;
//!   afterwards it returns the stored answer without asking again.
//! - A denied permission can only change through `set_permission_from_settings`, which stands in for the user
//!   changing the choice in system settings.
//! - Input mute is a flag the user sets for this app. Each real change queues one event for `take_mute_events`.
//! - `may_capture` is true only when the permission is granted and input is not muted.
//!
//! This is the pure model. The shell owns the prompt and the mute switch and does not feed this model yet, so the
//! matrix row that uses it is partial.

/// The microphone permission state of an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordPermission {
    Undetermined,
    Denied,
    Granted,
}

/// Permission and input-mute state of one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioApplication {
    permission: RecordPermission,
    input_muted: bool,
    mute_events: Vec<bool>,
}

impl AudioApplication {
    pub fn new() -> Self {
        Self { permission: RecordPermission::Undetermined, input_muted: false, mute_events: Vec::new() }
    }

    /// The current permission, without asking.
    pub fn record_permission(&self) -> RecordPermission {
        self.permission
    }

    /// Asks for the microphone permission. `ask` runs only while the permission is undetermined and answers the
    /// prompt (`true` grants). Returns the permission after the call.
    pub fn request_record_permission(&mut self, ask: impl FnOnce() -> bool) -> RecordPermission {
        if self.permission == RecordPermission::Undetermined {
            self.permission = if ask() { RecordPermission::Granted } else { RecordPermission::Denied };
        }
        self.permission
    }

    /// The user changes the permission in system settings.
    pub fn set_permission_from_settings(&mut self, permission: RecordPermission) {
        self.permission = permission;
    }

    pub fn is_input_muted(&self) -> bool {
        self.input_muted
    }

    /// Sets the input mute state. Returns `true` when the state changed; a change queues one event.
    pub fn set_input_muted(&mut self, muted: bool) -> bool {
        if self.input_muted == muted {
            return false;
        }
        self.input_muted = muted;
        self.mute_events.push(muted);
        true
    }

    /// The mute changes since the last call, oldest first (`true` means the input was muted).
    pub fn take_mute_events(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.mute_events)
    }

    /// Whether the app may capture audio now: permission granted and input not muted.
    pub fn may_capture(&self) -> bool {
        self.permission == RecordPermission::Granted && !self.input_muted
    }
}

impl Default for AudioApplication {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_is_asked_once_and_the_answer_sticks() {
        let mut app = AudioApplication::new();
        let mut asked = 0;
        let got = app.request_record_permission(|| {
            asked += 1;
            true
        });
        assert_eq!(got, RecordPermission::Granted);
        let again = app.request_record_permission(|| {
            asked += 1;
            false
        });
        assert_eq!(again, RecordPermission::Granted);
        assert_eq!(asked, 1);
    }

    #[test]
    fn a_refused_prompt_denies_and_does_not_ask_again() {
        let mut app = AudioApplication::new();
        assert_eq!(app.request_record_permission(|| false), RecordPermission::Denied);
        let got = app.request_record_permission(|| panic!("must not prompt again"));
        assert_eq!(got, RecordPermission::Denied);
        assert!(!app.may_capture());
    }

    #[test]
    fn denied_permission_changes_only_through_settings() {
        let mut app = AudioApplication::new();
        app.request_record_permission(|| false);
        app.set_permission_from_settings(RecordPermission::Granted);
        assert_eq!(app.record_permission(), RecordPermission::Granted);
        assert!(app.may_capture());
    }

    #[test]
    fn mute_reports_real_changes_only() {
        let mut app = AudioApplication::new();
        assert!(!app.set_input_muted(false), "no change from the initial state");
        assert!(app.set_input_muted(true));
        assert!(!app.set_input_muted(true));
        assert!(app.set_input_muted(false));
        assert_eq!(app.take_mute_events(), vec![true, false]);
        assert!(app.take_mute_events().is_empty());
    }

    #[test]
    fn capture_needs_permission_and_an_unmuted_input() {
        let mut app = AudioApplication::new();
        assert!(!app.may_capture(), "undetermined permission cannot capture");
        app.request_record_permission(|| true);
        assert!(app.may_capture());
        app.set_input_muted(true);
        assert!(!app.may_capture());
        assert!(app.is_input_muted());
        app.set_input_muted(false);
        assert!(app.may_capture());
    }
}
