//! Gamepad input in web content: the snapshot a page reads from a connected controller, laid out as the web gamepad
//! "standard" mapping. Seventeen buttons and four axes in fixed positions, so a page binds the same control on any pad.
//!
//! Pure: it reads a [`Gamepad`] state from `crate::gamepad` and returns a value. The browser engine decides when to
//! take a snapshot, and routing the snapshot into a page's script is not wired yet (see `docs/parity/game-controller.md`).

use crate::gamepad::{Axis, Button, Gamepad};

/// Buttons in the standard mapping.
pub const STANDARD_BUTTONS: usize = 17;
/// Axes in the standard mapping: left X, left Y, right X, right Y.
pub const STANDARD_AXES: usize = 4;
/// Pads a page can see at once. Slots are numbered from zero.
pub const MAX_SLOTS: u32 = 4;
/// An analogue trigger counts as pressed from this value up, when its digital click is not reported.
pub const TRIGGER_PRESS: f32 = 0.5;

/// The name of the mapping every snapshot carries.
pub const MAPPING: &str = "standard";

/// One button of the snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebButton {
    pub pressed: bool,
    /// Pressure from 0 to 1. A digital button reports 0 or 1.
    pub value: f64,
}

/// A connected controller as a page sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct WebGamepad {
    pub index: u32,
    pub id: String,
    pub mapping: &'static str,
    pub axes: [f64; STANDARD_AXES],
    pub buttons: [WebButton; STANDARD_BUTTONS],
}

/// Why a snapshot could not be taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebGamepadError {
    /// The slot is not one a page can see.
    SlotOutOfRange { index: u32, max: u32 },
    /// A controller needs a name for the page to show.
    EmptyId,
}

impl std::fmt::Display for WebGamepadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebGamepadError::SlotOutOfRange { index, max } => {
                write!(f, "gamepad slot {index} is out of range 0 to {}", max - 1)
            }
            WebGamepadError::EmptyId => write!(f, "gamepad id is empty"),
        }
    }
}

/// The standard-mapping button index of a digital element. The direction pad is not a button here; see [`snapshot`].
pub fn standard_button(button: Button) -> usize {
    match button {
        Button::South => 0,
        Button::East => 1,
        Button::West => 2,
        Button::North => 3,
        Button::LeftShoulder => 4,
        Button::RightShoulder => 5,
        Button::LeftTrigger => 6,
        Button::RightTrigger => 7,
        Button::Select => 8,
        Button::Start => 9,
        Button::LeftStick => 10,
        Button::RightStick => 11,
        Button::Home => 16,
    }
}

/// The standard-mapping axis index of an analogue element. The triggers are buttons in this mapping, not axes.
fn standard_axis(axis: Axis) -> Option<usize> {
    match axis {
        Axis::LeftX => Some(0),
        Axis::LeftY => Some(1),
        Axis::RightX => Some(2),
        Axis::RightY => Some(3),
        Axis::LeftTrigger | Axis::RightTrigger => None,
    }
}

/// Takes the snapshot of the pad in `slot`, named `id`, from its current state.
pub fn snapshot(slot: u32, id: &str, pad: &Gamepad) -> Result<WebGamepad, WebGamepadError> {
    if slot >= MAX_SLOTS {
        return Err(WebGamepadError::SlotOutOfRange { index: slot, max: MAX_SLOTS });
    }
    if id.trim().is_empty() {
        return Err(WebGamepadError::EmptyId);
    }
    let mut axes = [0.0f64; STANDARD_AXES];
    for axis in Axis::ALL {
        if let Some(i) = standard_axis(axis) {
            axes[i] = f64::from(pad.axis(axis));
        }
    }
    let mut buttons = [WebButton { pressed: false, value: 0.0 }; STANDARD_BUTTONS];
    for button in Button::ALL {
        let digital = pad.is_pressed(button);
        let analogue = match button {
            Button::LeftTrigger => pad.axis(Axis::LeftTrigger),
            Button::RightTrigger => pad.axis(Axis::RightTrigger),
            _ => 0.0,
        };
        let is_trigger = matches!(button, Button::LeftTrigger | Button::RightTrigger);
        let value = if is_trigger { analogue.max(if digital { 1.0 } else { 0.0 }) } else if digital { 1.0 } else { 0.0 };
        let pressed = digital || (is_trigger && analogue >= TRIGGER_PRESS);
        buttons[standard_button(button)] = WebButton { pressed, value: f64::from(value) };
    }
    let (x, y) = pad.dpad();
    for (index, active) in [(12usize, y < 0), (13usize, y > 0), (14usize, x < 0), (15usize, x > 0)] {
        buttons[index] = WebButton { pressed: active, value: if active { 1.0 } else { 0.0 } };
    }
    Ok(WebGamepad { index: slot, id: id.to_string(), mapping: MAPPING, axes, buttons })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_button_indices_follow_the_web_layout() {
        assert_eq!(standard_button(Button::South), 0);
        assert_eq!(standard_button(Button::Start), 9);
        assert_eq!(standard_button(Button::Home), 16);
    }

    #[test]
    fn a_slot_beyond_the_page_limit_is_refused() {
        let pad = Gamepad::new();
        assert_eq!(snapshot(MAX_SLOTS, "pad", &pad), Err(WebGamepadError::SlotOutOfRange { index: 4, max: 4 }));
    }

    #[test]
    fn an_empty_name_is_refused() {
        let pad = Gamepad::new();
        assert_eq!(snapshot(0, "  ", &pad), Err(WebGamepadError::EmptyId));
    }
}
