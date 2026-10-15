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
