//! Capture events from the hardware capture buttons of a device, as pure state. An app that is capturing registers an
//! `Interaction` with either one shared handler or two handlers (primary and secondary). The model decides which
//! handler receives a press, or whether the press falls back to the system behaviour: the interaction is disabled,
//! the app is not capturing, or the app is in the background. Each press is a `Event` with a phase (began, ended or
//! cancelled), and only an ended press counts as a completed press, such as the moment a photo is taken. Nothing here
//! reads a button or plays a sound: the input layer feeds events in, and the owner acts on what `dispatch` returns.

/// Which physical button was pressed. A device with one capture button reports `Primary`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Primary,
    Secondary,
}

/// Where a press is in its life cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Began,
    Ended,
    Cancelled,
}

/// One press of a capture button, with the time in seconds when the input layer saw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub button: Button,
    pub phase: Phase,
    pub time: f64,
}

/// The handler slot that should receive a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The single handler of an interaction created with one handler.
    Shared,
    /// The primary handler of a split interaction.
    Primary,
    /// The secondary handler of a split interaction.
    Secondary,
}

/// What happens to a press. `System` means the device keeps its own button behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Handler(Slot),
    System,
}

/// The capture button registration of one app.
#[derive(Clone, Debug, PartialEq)]
pub struct Interaction {
    split: bool,
    enabled: bool,
    /// When set, the device plays no sound for a capture press.
    pub default_sound_disabled: bool,
}

impl Interaction {
    /// An interaction with one handler that receives both buttons. Enabled, with the default sound.
    pub fn single() -> Self {
        Interaction { split: false, enabled: true, default_sound_disabled: false }
    }

    /// An interaction with separate primary and secondary handlers. Enabled, with the default sound.
    pub fn split() -> Self {
        Interaction { split: true, enabled: true, default_sound_disabled: false }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turns the interaction on or off. While off, presses return to the system behaviour.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Decides who receives a press. `capturing` is true while the app runs a capture session, and `foreground` is
    /// true while the app is in the foreground. The system sends capture events only to a capturing app in the
    /// foreground, so any other press returns `System`.
    pub fn dispatch(&self, event: &Event, capturing: bool, foreground: bool) -> Dispatch {
        if !self.enabled || !capturing || !foreground {
            return Dispatch::System;
        }
        if !self.split {
            return Dispatch::Handler(Slot::Shared);
        }
        match event.button {
            Button::Primary => Dispatch::Handler(Slot::Primary),
            Button::Secondary => Dispatch::Handler(Slot::Secondary),
        }
    }
}

/// True for a press that finished normally: the moment a capture action, such as taking a photo, should run.
pub fn is_completed_press(event: &Event) -> bool {
    event.phase == Phase::Ended
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(button: Button, phase: Phase) -> Event {
        Event { button, phase, time: 1.0 }
    }

    #[test]
    fn single_handler_receives_both_buttons() {
        let i = Interaction::single();
        let p = press(Button::Primary, Phase::Began);
        let s = press(Button::Secondary, Phase::Began);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::Handler(Slot::Shared));
        assert_eq!(i.dispatch(&s, true, true), Dispatch::Handler(Slot::Shared));
    }

    #[test]
    fn split_interaction_routes_each_button_to_its_own_slot() {
        let i = Interaction::split();
        let p = press(Button::Primary, Phase::Ended);
        let s = press(Button::Secondary, Phase::Ended);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::Handler(Slot::Primary));
        assert_eq!(i.dispatch(&s, true, true), Dispatch::Handler(Slot::Secondary));
    }

    #[test]
    fn disabled_interaction_returns_presses_to_the_system() {
        let mut i = Interaction::single();
        i.set_enabled(false);
        assert!(!i.is_enabled());
        let p = press(Button::Primary, Phase::Began);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::System);
        i.set_enabled(true);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::Handler(Slot::Shared));
    }

    #[test]
    fn presses_go_to_the_system_unless_the_app_is_capturing_in_the_foreground() {
        let i = Interaction::single();
        let p = press(Button::Primary, Phase::Began);
        // Not capturing: no session runs, so the system keeps the button.
        assert_eq!(i.dispatch(&p, false, true), Dispatch::System);
        // Capturing but in the background: events are not delivered.
        assert_eq!(i.dispatch(&p, true, false), Dispatch::System);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::Handler(Slot::Shared));
    }

    #[test]
    fn only_an_ended_press_is_a_completed_press() {
        assert!(is_completed_press(&press(Button::Primary, Phase::Ended)));
        assert!(!is_completed_press(&press(Button::Primary, Phase::Began)));
        assert!(!is_completed_press(&press(Button::Primary, Phase::Cancelled)));
    }

    #[test]
    fn default_sound_flag_starts_off_and_stays_separate_from_dispatch() {
        let mut i = Interaction::single();
        assert!(!i.default_sound_disabled);
        i.default_sound_disabled = true;
        // Disabling the default sound does not change where a press goes.
        let p = press(Button::Primary, Phase::Ended);
        assert_eq!(i.dispatch(&p, true, true), Dispatch::Handler(Slot::Shared));
    }
}
