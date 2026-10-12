//! Audio input picker as pure state: the list of microphones an app may offer, the choice the person makes from it, and
//! the presentation of the picker itself. The audio service owns the devices; the owner passes the currently available
//! inputs in and shows the picker it gets back. Presenting and dismissing report `Will` and `Did` events, so the app
//! can dim its own interface around the picker. An input list is empty unless the app is recording, because a session
//! that does not record has no inputs to offer. Nothing here opens a device or draws anything.

/// A microphone the audio service reports as available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub id: String,
    pub name: String,
}

/// Delegate events. `Will*` is sent before the picker changes, `Did*` after it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    WillPresent,
    DidPresent,
    WillDismiss,
    DidDismiss,
}

/// Why a choice was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectError {
    /// The id names no input in the list passed in.
    UnknownInput,
}

/// The picker state of one app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picker {
    presented: bool,
    selected: Option<String>,
    events: Vec<Event>,
}

/// The inputs an app may offer. Empty unless the session records: a session in any other mode has no inputs.
pub fn offered_inputs(recording: bool, available: &[Input]) -> Vec<Input> {
    if recording {
        available.to_vec()
    } else {
        Vec::new()
    }
}

impl Picker {
    pub fn new() -> Self {
        Picker::default()
    }

    pub fn is_presented(&self) -> bool {
        self.presented
    }

    /// Shows the picker. Returns false, with no events, when it is already on screen.
    pub fn present(&mut self) -> bool {
        if self.presented {
            return false;
        }
        self.events.push(Event::WillPresent);
        self.presented = true;
        self.events.push(Event::DidPresent);
        true
    }

    /// Takes the picker away. Returns false, with no events, when it is not on screen.
    pub fn dismiss(&mut self) -> bool {
        if !self.presented {
            return false;
        }
        self.events.push(Event::WillDismiss);
        self.presented = false;
        self.events.push(Event::DidDismiss);
        true
    }

    /// Drains the delegate events queued since the last call, in the order they happened.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Chooses the input with this id. The id must be one of `offered`.
    pub fn select(&mut self, offered: &[Input], id: &str) -> Result<(), SelectError> {
        if offered.iter().any(|i| i.id == id) {
            self.selected = Some(id.to_string());
            Ok(())
        } else {
            Err(SelectError::UnknownInput)
        }
    }

    /// The chosen input, if it is still in the list. A choice that has since disappeared yields `None`, so the
    /// caller falls back to the system default.
    pub fn selected_in<'a>(&self, offered: &'a [Input]) -> Option<&'a Input> {
        let id = self.selected.as_deref()?;
        offered.iter().find(|i| i.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str, name: &str) -> Input {
        Input { id: id.to_string(), name: name.to_string() }
    }

    #[test]
    fn present_then_dismiss_reports_will_and_did_in_order() {
        let mut p = Picker::new();
        assert!(p.present());
        assert!(p.is_presented());
        assert!(p.dismiss());
        assert!(!p.is_presented());
        assert_eq!(
            p.take_events(),
            vec![Event::WillPresent, Event::DidPresent, Event::WillDismiss, Event::DidDismiss]
        );
        assert!(p.take_events().is_empty());
    }

    #[test]
    fn presenting_twice_or_dismissing_when_hidden_does_nothing() {
        let mut p = Picker::new();
        assert!(!p.dismiss());
        assert!(p.present());
        assert!(!p.present());
        assert_eq!(p.take_events(), vec![Event::WillPresent, Event::DidPresent]);
    }

    #[test]
    fn inputs_are_offered_only_while_recording() {
        let mics = vec![input("usb", "USB microphone"), input("builtin", "Built-in microphone")];
        assert!(offered_inputs(false, &mics).is_empty());
        let offered = offered_inputs(true, &mics);
        assert_eq!(offered.len(), 2);
        assert_eq!(offered[1].name, "Built-in microphone");
    }

    #[test]
    fn select_accepts_only_an_offered_id() {
        let mics = vec![input("usb", "USB microphone")];
        let mut p = Picker::new();
        assert_eq!(p.select(&mics, "hdmi"), Err(SelectError::UnknownInput));
        assert_eq!(p.selected_in(&mics), None);
        assert_eq!(p.select(&mics, "usb"), Ok(()));
        assert_eq!(p.selected_in(&mics).map(|i| i.name.as_str()), Some("USB microphone"));
    }

    #[test]
    fn a_vanished_choice_falls_back_to_none() {
        let mics = vec![input("usb", "USB microphone"), input("builtin", "Built-in microphone")];
        let mut p = Picker::new();
        assert_eq!(p.select(&mics, "usb"), Ok(()));
        let unplugged = vec![input("builtin", "Built-in microphone")];
        assert_eq!(p.selected_in(&unplugged), None);
        assert_eq!(p.selected_in(&mics).map(|i| i.id.as_str()), Some("usb"));
    }
}
