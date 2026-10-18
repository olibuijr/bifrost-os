//! Game controller input for apps. The controller model lives in `bifrost_ui::gamepad` (buttons, sticks, triggers and
//! the direction pad, with dead zones and a callback and polling queue); this module makes it reachable from the SDK
//! and adds the plain-text description of a change that logs and accessibility labels need.
//!
//! ```ignore
//! let mut input = bifrost_app::gamepad_input::ControllerInput::new();
//! input.on_change(|change| println!("{}", bifrost_app::gamepad_input::describe(change)));
//! input.feed(ty, code, value); // one raw evdev record from the shell
//! ```
//!
//! Reading the device and delivering its records to the focused app is the shell's gamepad service; apps receive
//! the changes through the same model.

use crate::gamepad_service::{Controller as PadController, Event as PadEvent};
use std::collections::BTreeMap;

pub use bifrost_ui::gamepad::{element, Axis, AxisRange, Button, Change, ControllerInput, Element, Gamepad, QUEUE_LIMIT};

/// The name of a button as it reads in a log or a label.
pub fn button_name(button: Button) -> &'static str {
    match button {
        Button::South => "south button",
        Button::East => "east button",
        Button::North => "north button",
        Button::West => "west button",
        Button::LeftShoulder => "left shoulder",
        Button::RightShoulder => "right shoulder",
        Button::LeftTrigger => "left trigger click",
        Button::RightTrigger => "right trigger click",
        Button::LeftStick => "left stick click",
        Button::RightStick => "right stick click",
        Button::Select => "select",
        Button::Start => "start",
        Button::Home => "home",
    }
}

/// The name of an analogue element as it reads in a log or a label.
pub fn axis_name(axis: Axis) -> &'static str {
    match axis {
        Axis::LeftX => "left stick x",
        Axis::LeftY => "left stick y",
        Axis::RightX => "right stick x",
        Axis::RightY => "right stick y",
        Axis::LeftTrigger => "left trigger",
        Axis::RightTrigger => "right trigger",
    }
}

/// A one-line description of a change: `south button pressed`, `left stick x 0.44` or `direction pad -1 0`.
/// Axis values are shown with two decimals.
pub fn describe(change: &Change) -> String {
    match *change {
        Change::Button { button, pressed } => {
            format!("{} {}", button_name(button), if pressed { "pressed" } else { "released" })
        }
        Change::Axis { axis, value } => format!("{} {:.2}", axis_name(axis), value),
        Change::Dpad { x, y } => format!("direction pad {x} {y}"),
    }
}

/// Controller discovery for apps: the registry the shell keeps and its connect and disconnect events.
pub use bifrost_core::gamepad::{
    Capabilities as ControllerCapabilities, Controller, Event as ConnectionEvent, Registry as ControllerRegistry,
};

/// A one-line description of a connection change: `Pad A connected (controller 1)` or `Pad A disconnected`.
pub fn describe_connection(event: &ConnectionEvent) -> String {
    match event {
        ConnectionEvent::Connected(c) => format!("{} connected (controller {})", c.name, c.id),
        ConnectionEvent::Disconnected(c) => format!("{} disconnected", c.name),
    }
}

/// The extended-gamepad profile an app reads: a copy of one controller's state with the named elements, so a game
/// can poll `left_stick()` or `is_pressed(Button::South)` each frame without holding the live model.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtendedProfile {
    pad: Gamepad,
}

impl ExtendedProfile {
    /// Copies the state of a controller now.
    pub fn capture(pad: &Gamepad) -> Self {
        ExtendedProfile { pad: pad.clone() }
    }

    /// Whether a digital element is held (the face buttons, shoulders, trigger and stick clicks, and the menu buttons).
    pub fn is_pressed(&self, button: Button) -> bool {
        self.pad.is_pressed(button)
    }

    /// The left stick as (x, y), each in -1.0..=1.0 after its dead zone; y is positive downwards.
    pub fn left_stick(&self) -> (f32, f32) {
        (self.pad.axis(Axis::LeftX), self.pad.axis(Axis::LeftY))
    }

    /// The right stick as (x, y), with the same range and sign as the left stick.
    pub fn right_stick(&self) -> (f32, f32) {
        (self.pad.axis(Axis::RightX), self.pad.axis(Axis::RightY))
    }

    /// The left and right triggers as (left, right), each in 0.0..=1.0.
    pub fn triggers(&self) -> (f32, f32) {
        (self.pad.axis(Axis::LeftTrigger), self.pad.axis(Axis::RightTrigger))
    }

    /// The direction pad as (x, y), each -1, 0 or 1.
    pub fn dpad(&self) -> (i8, i8) {
        self.pad.dpad()
    }
}

/// One app's view of the controllers the shell reports. The shell sends `connected`, `disconnected` and `input` service
/// events to the focused app (connections to every app); feed each one to [`Session::apply`] from the app's
/// `service_event` hook, then read the pads and their input queues from the session.
#[derive(Default)]
pub struct Session {
    pads: BTreeMap<u32, (PadController, ControllerInput)>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one decoded service event: a connect adds the pad with an empty input stream, a disconnect removes it
    /// and its queue, and an input is pushed onto its controller's stream (ignored when that controller is unknown).
    pub fn apply(&mut self, event: PadEvent) {
        match event {
            PadEvent::Connected(c) => {
                self.pads.insert(c.id, (c, ControllerInput::new()));
            }
            PadEvent::Disconnected(c) => {
                self.pads.remove(&c.id);
            }
            PadEvent::Input { controller, change } => {
                if let Some((_, input)) = self.pads.get_mut(&controller) {
                    input.push(change);
                }
            }
        }
    }

    /// The connected controllers, in id order.
    pub fn controllers(&self) -> impl Iterator<Item = &PadController> {
        self.pads.values().map(|(c, _)| c)
    }

    /// The input stream of controller `id`, for polling or callbacks.
    pub fn input_mut(&mut self, id: u32) -> Option<&mut ControllerInput> {
        self.pads.get_mut(&id).map(|(_, input)| input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap with the given event codes set, sized like a `KEY_MAX` capability read.
    fn bitmap(codes: &[u16]) -> Vec<u8> {
        let mut bits = vec![0u8; 96];
        for &code in codes {
            bits[usize::from(code / 8)] |= 1u8 << (code % 8);
        }
        bits
    }

    #[test]
    fn session_tracks_connects_inputs_and_disconnects() {
        let mut session = Session::new();
        let pad = PadController { id: 3, name: "Pad A".into(), path: "/dev/input/event5".into() };
        session.apply(PadEvent::Connected(pad.clone()));
        assert_eq!(session.controllers().collect::<Vec<_>>(), vec![&pad]);
        session.apply(PadEvent::Input { controller: 3, change: Change::Button { button: Button::South, pressed: true } });
        session.apply(PadEvent::Input { controller: 9, change: Change::Dpad { x: 1, y: 0 } });
        let input = session.input_mut(3).expect("pad 3 is connected");
        assert!(input.gamepad().is_pressed(Button::South));
        assert_eq!(input.poll(), Some(Change::Button { button: Button::South, pressed: true }));
        assert_eq!(input.poll(), None);
        session.apply(PadEvent::Disconnected(pad));
        assert_eq!(session.controllers().count(), 0);
        assert!(session.input_mut(3).is_none());
    }

    #[test]
    fn connection_events_read_as_one_line() {
        let caps = ControllerCapabilities { keys: bitmap(&[0x130]), abs: bitmap(&[0, 1]) };
        let mut registry = ControllerRegistry::default();
        let connected = registry.connected("/dev/input/event5", " Pad A ", &caps).expect("a pad connects");
        assert_eq!(describe_connection(&connected), "Pad A connected (controller 1)");
        let removed = registry.disconnected("/dev/input/event5").expect("the pad leaves");
        assert_eq!(describe_connection(&removed), "Pad A disconnected");
    }

    #[test]
    fn profile_copies_the_state_of_the_pad() {
        let mut pad = Gamepad::new();
        // BTN_SOUTH pressed, left stick x at full deflection, right trigger half pressed, direction pad up
        pad.feed(0x01, 0x130, 1);
        pad.feed(0x03, 0x00, 32767);
        pad.feed(0x03, 0x05, 128);
        pad.feed(0x03, 0x11, -1);
        let profile = ExtendedProfile::capture(&pad);
        assert!(profile.is_pressed(Button::South));
        assert!(!profile.is_pressed(Button::East));
        assert!(profile.left_stick().0 > 0.9);
        let (_, right) = profile.triggers();
        assert!(right > 0.4 && right < 0.6);
        assert_eq!(profile.dpad(), (0, -1));
        // a later change to the live pad does not reach the copy
        pad.feed(0x01, 0x130, 0);
        assert!(profile.is_pressed(Button::South));
    }
}
